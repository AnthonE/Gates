//! Who may run an admin verb, and what running one does (admin v0,
//! `ALPHA.md` §3).
//!
//! ## The allowlist is wallets, and it is read once at boot
//!
//! `shard.toml`'s `admin_wallets` is the whole authority. It is a list of
//! addresses, compared against the `PlayerKey` the SIWE handshake already
//! proved (`auth.rs`) — so admin rights are the one thing on this shard
//! that cannot be spoofed by a client, because the client never asserts
//! them. There is no in-game promotion verb and there is not going to be
//! one: a shard whose admin list can grow at runtime is a shard where one
//! compromised key is every key.
//!
//! Unset means **nobody is an admin**, which is the default every test and
//! every community shard runs — the same one-build-two-populations posture
//! `entitle.rs` takes (`DECISIONS.md` 2026-08-04).
//!
//! ## Where a verb executes, and why it is split
//!
//! The sim thread owns the world and the wallets; the accept loop owns the
//! connections and the slot table. Neither can do the other's half:
//!
//! - **say / tp / give / save** are the sim thread's outright. `tp` and
//!   `give` go through `Command::AdminTeleport`/`AdminGive` rather than
//!   poking `world.players`, because the WAL is the command stream and an
//!   admin act has to be *visible in a replay* — which `ALPHA.md` §3 asks
//!   for in the same sentence as the lane.
//! - **kick / ban** need a `Connection` to close and a slot to mark, and
//!   both live in the accept loop. They cross on an [`AdminAct`] ring —
//!   sim → accept, SPSC, `SaveMsg`'s exact shape for `SaveMsg`'s reason.
//!
//! ## A ban is a wallet, kept in its own file
//!
//! The admin types an id because that is what their screen shows; the
//! server resolves it to the wallet it holds and hands *that* to the
//! accept loop, because an id is meaningless across a reconnect. With
//! `ban_file` set the list is read at boot and written whole on every new
//! ban ([`Bans::load`], [`Bans::save`]); without it a ban holds for the
//! shard's uptime. Its own file with its own header, never the player
//! store's: that header pins the seed, and a wipe would forget every ban.
//! The file is text, one wallet a line, so an operator can also lift a ban
//! by deleting its line while the shard is down.
//!
//! A ban is enforced **at the door**: a handshake that proves a banned
//! wallet is refused `REFUSE_ADMIN` before it claims a slot, so the screen
//! says what it said at the kick. `/unban <hex>` lifts one in a running
//! shard; it names the wallet's front (`/ban`'s answer prints it), because
//! the player it means is not on the shard to have an id. Only the accept
//! loop holds the list, so it is the one that knows how either verb went,
//! and it says so to the admin on an [`AdminReply`] ring back to the sim.

use protocol::admin::{AdminCmd, WalletPrefix};
use protocol::ChatText;

use crate::store::PlayerKey;

/// The most admin wallets one shard's config may name. Wall 4 on a list
/// read from a file: generous for a real crew, small enough that the
/// linear scan on join is free.
pub const MAX_ADMINS: usize = 16;

/// The most wallets one shard may ban in one uptime. Bounded like every
/// other store here; overflow **refuses the ban** and says so, rather than
/// evicting an earlier one — a griefer who bans their way back in by
/// filling the list is exactly the hole a silent eviction would open.
pub const MAX_BANS: usize = 256;

/// Verb codes for the anomaly log's `what` field. Integers because the log
/// record is integers (wall 3); the names live in one place, here, and the
/// log's reader is a person with this file open.
pub const VERB_KICK: u16 = 0;
pub const VERB_BAN: u16 = 1;
pub const VERB_SAY: u16 = 2;
pub const VERB_TP: u16 = 3;
pub const VERB_GIVE: u16 = 4;
pub const VERB_SAVE: u16 = 5;
/// Not an admin verb — the code the log uses for a refused slash line.
pub const VERB_UNKNOWN: u16 = 6;
pub const VERB_WEATHER: u16 = 7;
pub const VERB_TIME: u16 = 8;
pub const VERB_WIPE: u16 = 9;
pub const VERB_BRAIN: u16 = 10;
pub const VERB_WHO: u16 = 11;
pub const VERB_UNBAN: u16 = 12;

/// The verb code a parsed command carries into the log.
pub fn verb_of(cmd: &AdminCmd) -> u16 {
    match cmd {
        AdminCmd::Kick { .. } => VERB_KICK,
        AdminCmd::Ban { .. } => VERB_BAN,
        AdminCmd::Unban { .. } => VERB_UNBAN,
        AdminCmd::Say { .. } => VERB_SAY,
        AdminCmd::Teleport { .. } => VERB_TP,
        AdminCmd::Give { .. } => VERB_GIVE,
        AdminCmd::SaveNow => VERB_SAVE,
        AdminCmd::Weather { .. } => VERB_WEATHER,
        AdminCmd::Time { .. } => VERB_TIME,
        AdminCmd::Wipe { .. } | AdminCmd::WipeCancel | AdminCmd::WipeWhen => VERB_WIPE,
        AdminCmd::Brain => VERB_BRAIN,
        AdminCmd::Who => VERB_WHO,
        // A `/bug` is never an admin act; it has its own `Kind`.
        AdminCmd::Bug { .. } => VERB_UNKNOWN,
    }
}

/// The wallets a shard trusts, as parsed from `shard.toml`.
///
/// Comparison is on the `PlayerKey`'s bytes, which `auth::key_of` builds
/// lowercase from the recovered address — so the config is normalised the
/// same way at parse time and a checksummed address in the file matches a
/// lowercase one on the wire.
#[derive(Clone, Debug, Default)]
pub struct Admins {
    keys: Vec<PlayerKey>,
}

impl Admins {
    /// Empty: nobody is an admin. The default.
    pub fn none() -> Self {
        Self { keys: Vec::new() }
    }

    /// Parse one config value — a comma-separated list of `0x…` addresses.
    ///
    /// Refuses the whole list on any bad entry rather than admitting the
    /// good ones: a typo'd admin address is an operator expecting a
    /// privilege they do not have, and finding out at boot is cheaper than
    /// finding out during a raid.
    pub fn parse(raw: &str) -> Result<Self, String> {
        let mut keys = Vec::new();
        for entry in raw.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            if !is_wallet(entry) {
                return Err(format!(
                    "admin_wallets names `{entry}`, which is not an 0x-prefixed 40-hex address"
                ));
            }
            if keys.len() >= MAX_ADMINS {
                return Err(format!(
                    "admin_wallets names more than {MAX_ADMINS} wallets"
                ));
            }
            let lower = entry.to_ascii_lowercase();
            let key = PlayerKey::new(lower.as_bytes())
                .ok_or_else(|| format!("admin_wallets entry `{entry}` is too long for a key"))?;
            if keys.contains(&key) {
                return Err(format!("admin_wallets names `{entry}` twice"));
            }
            keys.push(key);
        }
        Ok(Self { keys })
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    pub fn len(&self) -> usize {
        self.keys.len()
    }

    /// Is this the key of an admin? A linear scan over at most
    /// [`MAX_ADMINS`], asked once per slash line and never per tick.
    pub fn allows(&self, key: &PlayerKey) -> bool {
        self.keys.iter().any(|k| k == key)
    }
}

/// An 0x-prefixed 40-hex address, the one shape `auth.rs` can produce.
/// Written here rather than borrowed from `entitle::is_wallet` so the
/// config's rule and the ticket route's rule can diverge without one
/// silently changing the other.
fn is_wallet(s: &str) -> bool {
    s.len() == 42 && s.starts_with("0x") && s[2..].bytes().all(|b| b.is_ascii_hexdigit())
}

/// What the sim thread asks the accept loop to do — the half of the admin
/// lane that needs a connection, or the ban list beside the connections.
/// `by` is the admin's own player id, which the [`AdminReply`] goes back to.
///
/// `Copy` and fixed-size, like every other record crossing an `rtrb` ring
/// in this crate.
#[derive(Clone, Copy, Debug)]
pub enum AdminAct {
    /// Close this player's connection. The body stays as a sleeper: a kick
    /// is not a wipe, and the reference's is not either.
    Kick { by: u32, id: u32 },
    /// Kick, and remember the wallet for the rest of this uptime.
    Ban { by: u32, id: u32, key: PlayerKey },
    /// Forget the one banned wallet starting with `prefix`.
    Unban { by: u32, prefix: WalletPrefix },
}

/// Accept → sim: how an [`AdminAct`] went, said to admin `to` as a
/// `[server]` line by the next tick's chat pump. A full ring drops it (the
/// act still happened; the line is counted as `chat_undelivered`).
#[derive(Clone, Copy, Debug)]
pub struct AdminReply {
    pub to: u32,
    pub text: ChatText,
}

impl AdminReply {
    /// `line` cut to a chat line. Every answer is ASCII, so the cut cannot
    /// split a character; a line that still fails the sanitize is no reply.
    pub fn new(to: u32, line: &str) -> Option<Self> {
        let b = line.as_bytes();
        let text = ChatText::sanitize(&b[..b.len().min(ChatText::CAP)])?;
        Some(Self { to, text })
    }
}

/// How many hex digits of a wallet the answers print: what an admin types
/// back into `/unban`, and short enough that every answer below fits beside
/// the `[server] ` mark.
const SHOWN_HEX: usize = 12;

/// The front of a wallet as the answers show it: `0x` and [`SHOWN_HEX`]
/// digits. A key that is not an address (a hand-edited ban file) shows as
/// much of itself as is printable.
fn shown(key: &PlayerKey) -> &str {
    let b = key.as_bytes();
    let n = b.len().min(2 + SHOWN_HEX);
    std::str::from_utf8(&b[..n]).unwrap_or("?")
}

/// `/kick`'s answer.
pub fn kicked_line(id: u32, kicked: bool) -> String {
    if kicked {
        format!("kicked {id}")
    } else {
        format!("{id} is not on")
    }
}

/// `/ban`'s answer: the wallet's front, which is what `/unban` takes.
pub fn banned_line(id: u32, key: &PlayerKey) -> String {
    format!("banned {id} ({})", shown(key))
}

/// `/ban`'s answer when [`MAX_BANS`] refused it.
pub fn ban_full_line(id: u32) -> String {
    format!("{id} not banned: {MAX_BANS} bans")
}

/// `/unban`'s answer.
pub fn unban_line(prefix: &WalletPrefix, how: &Result<PlayerKey, UnbanMiss>) -> String {
    let typed = &prefix.as_bytes()[..prefix.as_bytes().len().min(SHOWN_HEX)];
    let typed = std::str::from_utf8(typed).unwrap_or("?");
    match how {
        Ok(key) => format!("unbanned {}", shown(key)),
        Err(UnbanMiss::None) => format!("no ban starts {typed}"),
        Err(UnbanMiss::Ambiguous(n)) => format!("{n} bans start {typed}: type more"),
    }
}

/// Why `/unban` lifted nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnbanMiss {
    /// No banned wallet starts with the prefix.
    None,
    /// This many do: lifting one would be a guess.
    Ambiguous(usize),
}

/// The ban file's first line. Its own format version, bumped if a line's
/// shape ever changes; a file that does not open with it refuses the boot.
pub const BAN_FILE_HEADER: &str = "# gates bans v1";

/// The wallets refused at the door. Lives in the accept loop beside the
/// key table it is compared against; `file`, when set, is where it lasts.
#[derive(Default)]
pub struct Bans {
    keys: Vec<PlayerKey>,
    file: Option<std::path::PathBuf>,
}

impl Bans {
    pub fn new() -> Self {
        Self {
            keys: Vec::new(),
            file: None,
        }
    }

    /// The list kept in `path`: read now if the file exists, written by
    /// every [`Bans::save`]. A file that does not parse refuses the boot —
    /// a shard that came up having forgotten its bans would readmit them.
    pub fn load(path: &std::path::Path) -> Result<Self, String> {
        let mut bans = Self::new();
        match std::fs::read_to_string(path) {
            Ok(text) => bans.keys = parse_bans(&text)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
        }
        bans.file = Some(path.to_path_buf());
        Ok(bans)
    }

    /// Write the whole list to its file, through a temp file and a rename
    /// so a crash mid-write keeps the old one. A no-op with no file.
    pub fn save(&self) -> std::io::Result<()> {
        use std::io::Write;
        let Some(path) = &self.file else {
            return Ok(());
        };
        let tmp = std::path::PathBuf::from(format!("{}.tmp", path.display()));
        {
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(bans_text(&self.keys).as_bytes())?;
            f.sync_data()?;
        }
        std::fs::rename(&tmp, path)
    }

    /// Record a ban. `false` ⇒ the list is full and the ban did NOT take —
    /// the caller must say so rather than assume it did.
    pub fn insert(&mut self, key: PlayerKey) -> bool {
        if self.keys.contains(&key) {
            return true; // already banned; idempotent, not an error
        }
        if self.keys.len() >= MAX_BANS {
            return false;
        }
        self.keys.push(key);
        true
    }

    pub fn contains(&self, key: &PlayerKey) -> bool {
        self.keys.iter().any(|k| k == key)
    }

    /// Lift the one ban whose wallet's hex starts with `prefix` (case
    /// ignored, `0x` implied), and say whose it was. Two matches lift
    /// nothing: unbanning the wrong griefer is worse than asking for two
    /// more digits. The caller saves.
    pub fn remove_prefix(&mut self, prefix: &WalletPrefix) -> Result<PlayerKey, UnbanMiss> {
        let want = prefix.as_bytes();
        let hits = |k: &PlayerKey| {
            k.as_bytes()
                .strip_prefix(b"0x")
                .and_then(|hex| hex.get(..want.len()))
                .is_some_and(|front| front.eq_ignore_ascii_case(want))
        };
        let mut found = None;
        let mut n = 0;
        for (i, k) in self.keys.iter().enumerate() {
            if hits(k) {
                n += 1;
                found = Some(i);
            }
        }
        match (n, found) {
            (1, Some(i)) => Ok(self.keys.remove(i)),
            (0, _) => Err(UnbanMiss::None),
            _ => Err(UnbanMiss::Ambiguous(n)),
        }
    }

    pub fn len(&self) -> usize {
        self.keys.len()
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }
}

/// The file's text: the header, then a wallet a line — as typed when it is
/// printable ASCII (an address always is), else `hex:` and its bytes.
fn bans_text(keys: &[PlayerKey]) -> String {
    let mut text = String::from(BAN_FILE_HEADER);
    text.push('\n');
    for k in keys {
        let b = k.as_bytes();
        if b.iter().all(|c| c.is_ascii_graphic()) && !b.starts_with(b"hex:") && b[0] != b'#' {
            text.push_str(std::str::from_utf8(b).unwrap_or_default());
        } else {
            text.push_str("hex:");
            for c in b {
                text.push_str(&format!("{c:02x}"));
            }
        }
        text.push('\n');
    }
    text
}

/// [`bans_text`] read back. Blank lines and `#` comments are skipped, so
/// an operator may annotate a ban.
fn parse_bans(text: &str) -> Result<Vec<PlayerKey>, String> {
    let mut lines = text.lines();
    if lines.next().map(str::trim) != Some(BAN_FILE_HEADER) {
        return Err(format!(
            "not a ban file: the first line must be `{BAN_FILE_HEADER}`"
        ));
    }
    let mut keys = Vec::new();
    for (n, line) in lines.enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let bytes = match line.strip_prefix("hex:") {
            Some(hex) if hex.len() % 2 == 0 => (0..hex.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&hex[i..i + 2], 16))
                .collect::<Result<Vec<u8>, _>>()
                .map_err(|_| format!("line {}: bad hex", n + 2))?,
            Some(_) => return Err(format!("line {}: odd hex", n + 2)),
            None => line.as_bytes().to_vec(),
        };
        let key = PlayerKey::new(&bytes).ok_or_else(|| format!("line {}: not a wallet", n + 2))?;
        if keys.len() >= MAX_BANS {
            return Err(format!("more than {MAX_BANS} bans"));
        }
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    Ok(keys)
}

/// The line the house says when an admin broadcasts. Prefixed so a player
/// can tell a server notice from a stranger typing the same words —
/// `pump_chat` sends it with `from = 0`, which names no player (ids start
/// at 256), and this marker is what makes that legible without the client
/// learning a new message shape.
/// `/brain`'s answer, for the animal in roster slot `slot` standing `dist_m`
/// from the asker: species, slot, state, how the last think went (`.` done,
/// `!` failed), the
/// remembered target's player id (or `-`), hp, range and how long it stays
/// roused. Packed tight because a chat line is 48 bytes and the `[server] `
/// mark takes nine; the order is what a person debugging a chase reads first.
pub fn brain_line(
    slot: usize,
    m: &sim_core::mob::Mob,
    target_id: Option<u32>,
    dist_m: f32,
    tick: u64,
) -> String {
    use sim_core::brain::{AiState, FAILED, FINISHED};
    let species = match m.kind {
        sim_core::mob::MOB_PIG => "pig",
        sim_core::mob::MOB_WOLF => "wolf",
        sim_core::mob::MOB_STAG => "stag",
        sim_core::mob::MOB_HELI => "heli",
        sim_core::mob::MOB_SENTRY => "guard",
        _ => "mob",
    };
    let state = match m.state {
        AiState::Idle => "idle",
        AiState::Roam => "roam",
        AiState::Chase => "chase",
        AiState::Attack => "bite",
        AiState::Flee => "flee",
        AiState::NavigateHome => "home",
        AiState::Orbit => "orbit",
        AiState::Patrol => "patrol",
        AiState::Sleep => "sleep",
        AiState::MoveTowards => "listen",
    };
    let status = match m.status {
        FINISHED => ".",
        FAILED => "!",
        _ => "",
    };
    let target = target_id.map_or_else(|| "-".to_string(), |id| id.to_string());
    let roused_s = m.roused_until.saturating_sub(tick) / sim_core::limits::TICK_HZ as u64;
    format!(
        "{species}#{slot} {state}{status} >{target} hp{} {dist_m:.0}m r{roused_s}s",
        m.hp
    )
}

/// `/weather`'s answer to the admin: the preset it asked for, by the word
/// `/weather` takes, and when the sky gets there.
pub fn weather_line(mode: u8) -> String {
    use sim_core::weather::*;
    let word = match mode {
        MODE_AUTO => return "weather: back to the schedule".to_string(),
        CLEAR => "clear",
        OVERCAST => "overcast",
        FOG => "fog",
        RAIN_MILD => "rain",
        RAIN_HEAVY => "heavy",
        STORM => "storm",
        _ => "?",
    };
    let secs = FORCE_FADE_TICKS / sim_core::limits::TICK_HZ as u64;
    format!("weather: {word} in {secs} s")
}

/// `/time`'s answer to the admin: the day fraction the clock moved to.
pub fn time_line(frac_pm: u16) -> String {
    format!("time: {}.{:03} of the day", frac_pm / 1000, frac_pm % 1000)
}

pub fn server_line(text: &ChatText) -> ChatText {
    let mut buf = [0u8; ChatText::CAP];
    let prefix = b"[server] ";
    let n = prefix.len().min(ChatText::CAP);
    buf[..n].copy_from_slice(&prefix[..n]);
    let body = text.as_bytes();
    let room = ChatText::CAP - n;
    let m = body.len().min(room);
    buf[n..n + m].copy_from_slice(&body[..m]);
    // Sanitize rather than construct: the prefix plus a truncated tail is
    // a new line and is held to the same rule as any other. It cannot fail
    // — the prefix is printable and the tail was already sanitized — but
    // the fallback keeps this total rather than asserting.
    ChatText::sanitize(&buf[..n + m]).unwrap_or(*text)
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "0x00112233445566778899aabbccddeeff00112233";
    const B: &str = "0xffeeddccbbaa99887766554433221100ffeeddcc";

    fn key(s: &str) -> PlayerKey {
        PlayerKey::new(s.to_ascii_lowercase().as_bytes()).unwrap()
    }

    #[test]
    fn the_default_is_nobody() {
        let a = Admins::none();
        assert!(a.is_empty());
        assert!(!a.allows(&key(A)));
        assert!(Admins::parse("").unwrap().is_empty());
        assert!(Admins::parse("  ,  ").unwrap().is_empty());
    }

    #[test]
    fn a_listed_wallet_is_allowed_and_others_are_not() {
        let a = Admins::parse(&format!("{A}, {B}")).unwrap();
        assert_eq!(a.len(), 2);
        assert!(a.allows(&key(A)));
        assert!(a.allows(&key(B)));
        assert!(!a.allows(&key("0x1111111111111111111111111111111111111111")));
    }

    /// A checksummed address in the file matches the lowercase key the
    /// handshake produces — the case-folding that makes the config usable
    /// by a person copying from a block explorer.
    #[test]
    fn the_comparison_ignores_case() {
        let mixed = "0x00112233445566778899AABBCCDDEEFF00112233";
        let a = Admins::parse(mixed).unwrap();
        assert!(a.allows(&key(A)), "a mixed-case config entry must match");
    }

    /// Every refusal, and the property: a bad list is refused whole, so an
    /// operator never gets a partial privilege silently.
    #[test]
    fn a_bad_list_is_refused_whole() {
        for bad in [
            "nonsense",
            "0x123",                                      // too short
            "00112233445566778899aabbccddeeff00112233",   // no 0x
            "0x00112233445566778899aabbccddeeff0011223g", // not hex
            &format!("{A},nonsense"),                     // one good, one bad
            &format!("{A},{A}"),                          // duplicate
        ] {
            assert!(Admins::parse(bad).is_err(), "{bad:?} should be refused");
        }
    }

    #[test]
    fn the_admin_list_is_capped() {
        let many: Vec<String> = (0..MAX_ADMINS + 1)
            .map(|i| format!("0x{:040x}", i + 1))
            .collect();
        assert!(Admins::parse(&many.join(",")).is_err());
        let exact: Vec<String> = (0..MAX_ADMINS)
            .map(|i| format!("0x{:040x}", i + 1))
            .collect();
        assert_eq!(Admins::parse(&exact.join(",")).unwrap().len(), MAX_ADMINS);
    }

    /// A ban is idempotent, and a full list refuses rather than evicting —
    /// the overflow policy the type's doc states.
    #[test]
    fn bans_are_idempotent_and_bounded() {
        let mut b = Bans::new();
        assert!(b.insert(key(A)));
        assert!(b.insert(key(A)), "re-banning is not an error");
        assert_eq!(b.len(), 1);
        assert!(b.contains(&key(A)));
        assert!(!b.contains(&key(B)));
        for i in 0..MAX_BANS {
            b.insert(PlayerKey::new(format!("0x{:040x}", i + 100).as_bytes()).unwrap());
        }
        assert_eq!(b.len(), MAX_BANS);
        assert!(
            !b.insert(PlayerKey::new(b"0xdeadbeef").unwrap()),
            "a full ban list must refuse rather than evict"
        );
    }

    /// `/unban` lifts exactly one ban: a unique prefix (any case) removes it,
    /// a prefix two wallets share and a prefix nobody has both lift nothing,
    /// and the file written after the lift no longer has the wallet.
    #[test]
    fn unban_lifts_one_ban_by_its_prefix_and_never_guesses() {
        let p = |s: &str| WalletPrefix::parse(s).unwrap();
        let twin = "0x001122ffffffffffffffffffffffffffffffffff";
        let mut b = Bans::new();
        for w in [A, B, twin] {
            b.insert(key(w));
        }
        assert_eq!(b.remove_prefix(&p("001122")), Err(UnbanMiss::Ambiguous(2)));
        assert_eq!(b.remove_prefix(&p("abcdef")), Err(UnbanMiss::None));
        assert_eq!(b.len(), 3, "a miss lifts nothing");
        assert_eq!(b.remove_prefix(&p("0x0011223344")), Ok(key(A)));
        assert!(!b.contains(&key(A)));
        assert_eq!(b.remove_prefix(&p("FFEEDD")), Ok(key(B)), "case ignored");
        assert_eq!(b.remove_prefix(&p("001122")), Ok(key(twin)), "unique now");
        assert!(b.is_empty());

        let path = std::env::temp_dir().join(format!("gates-unban-{}.txt", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut f = Bans::load(&path).unwrap();
        f.insert(key(A));
        f.insert(key(B));
        f.save().unwrap();
        f.remove_prefix(&p("00112233")).unwrap();
        f.save().unwrap();
        let back = Bans::load(&path).unwrap();
        assert_eq!(back.len(), 1);
        assert!(back.contains(&key(B)) && !back.contains(&key(A)));
        let _ = std::fs::remove_file(&path);
    }

    /// Every answer the accept loop gives an admin reads whole after the
    /// `[server] ` mark at its longest: a ten-digit id, a full list.
    #[test]
    fn the_admin_answers_fit_a_chat_line() {
        let room = ChatText::CAP - "[server] ".len();
        let pre = WalletPrefix::parse(&"a".repeat(40)).unwrap();
        for line in [
            kicked_line(u32::MAX, true),
            kicked_line(u32::MAX, false),
            banned_line(u32::MAX, &key(A)),
            ban_full_line(u32::MAX),
            unban_line(&pre, &Ok(key(A))),
            unban_line(&pre, &Err(UnbanMiss::None)),
            unban_line(&pre, &Err(UnbanMiss::Ambiguous(MAX_BANS))),
        ] {
            assert!(line.len() <= room, "{line:?} is {} bytes", line.len());
            assert!(AdminReply::new(1, &line).is_some(), "{line:?}");
        }
        assert_eq!(banned_line(257, &key(A)), "banned 257 (0x001122334455)");
    }

    /// A ban outlives the shard in its own file, read back as written —
    /// annotations skipped, an unprintable key carried as hex — and a file
    /// that is not a ban file refuses rather than loading as empty.
    #[test]
    fn bans_survive_a_restart_in_their_own_file() {
        let path = std::env::temp_dir().join(format!("gates-bans-{}.txt", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut b = Bans::load(&path).unwrap();
        assert!(b.is_empty(), "no file yet is no bans");
        b.insert(key(A));
        b.insert(PlayerKey::new(&[0x00, 0xff, b' ']).unwrap());
        b.save().unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, format!("{text}\n# lifted by hand:\n")).unwrap();
        let back = Bans::load(&path).unwrap();
        assert_eq!(back.len(), 2);
        assert!(back.contains(&key(A)));
        assert!(back.contains(&PlayerKey::new(&[0x00, 0xff, b' ']).unwrap()));
        assert!(text.contains("hex:00ff20"), "{text}");

        std::fs::write(&path, "0xabc\n").unwrap();
        assert!(
            Bans::load(&path).is_err(),
            "a headerless file is not a ban file"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn env_answers_name_what_changed() {
        assert_eq!(
            weather_line(sim_core::weather::STORM),
            "weather: storm in 20 s"
        );
        assert_eq!(
            weather_line(sim_core::weather::MODE_AUTO),
            "weather: back to the schedule"
        );
        assert_eq!(time_line(250), "time: 0.250 of the day");
    }

    /// The house's line is marked, and a long one truncates rather than
    /// refusing — a truncated notice still says something.
    #[test]
    fn a_server_line_is_marked_and_bounded() {
        let t = ChatText::sanitize(b"wipe in 5").unwrap();
        assert_eq!(server_line(&t).as_bytes(), b"[server] wipe in 5");
        let long = ChatText::sanitize(&[b'x'; ChatText::CAP]).unwrap();
        let out = server_line(&long);
        assert_eq!(out.len(), ChatText::CAP);
        assert!(out.as_bytes().starts_with(b"[server] "));
    }

    /// A worst-case `/brain` answer still reads whole after the `[server] `
    /// mark: a chase with a five-digit target, full hp and a long rouse.
    #[test]
    fn a_brain_line_fits_a_chat_line() {
        let m = sim_core::mob::Mob {
            kind: sim_core::mob::MOB_WOLF,
            state: sim_core::brain::AiState::Chase,
            status: sim_core::brain::FAILED,
            hp: 100,
            roused_until: 30 * 99,
            ..Default::default()
        };
        let line = brain_line(255, &m, Some(65_535), 99.0, 0);
        assert!(
            line.len() + "[server] ".len() <= ChatText::CAP,
            "{line:?} is {} bytes",
            line.len()
        );
    }
}
