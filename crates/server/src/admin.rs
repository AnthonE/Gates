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
//! ## A ban is a wallet, and the file is not this module's
//!
//! The admin types an id because that is what their screen shows; the
//! server resolves it to the wallet it holds and hands *that* to the
//! accept loop, because an id is meaningless across a reconnect. Where the
//! list is persisted is `net.rs`'s business (it owns the store), and at v0
//! it is **memory only** — a ban holds for the shard's uptime and a
//! restart forgets it. That is stated rather than hidden: persisting it
//! wants its own file with its own format version, and a ban list quietly
//! sharing the player store's header would be wiped by the next seed
//! change (`store.rs` §the header pins the seed).

use protocol::admin::AdminCmd;
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

/// The verb code a parsed command carries into the log.
pub fn verb_of(cmd: &AdminCmd) -> u16 {
    match cmd {
        AdminCmd::Kick { .. } => VERB_KICK,
        AdminCmd::Ban { .. } => VERB_BAN,
        AdminCmd::Say { .. } => VERB_SAY,
        AdminCmd::Teleport { .. } => VERB_TP,
        AdminCmd::Give { .. } => VERB_GIVE,
        AdminCmd::SaveNow => VERB_SAVE,
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
/// lane that needs a connection.
///
/// `Copy` and fixed-size, like every other record crossing an `rtrb` ring
/// in this crate.
#[derive(Clone, Copy, Debug)]
pub enum AdminAct {
    /// Close this player's connection. The body stays as a sleeper: a kick
    /// is not a wipe, and the reference's is not either.
    Kick { id: u32 },
    /// Kick, and remember the wallet for the rest of this uptime.
    Ban { id: u32, key: PlayerKey },
}

/// The wallets refused at the door for this uptime. Lives in the accept
/// loop beside the key table it is compared against.
#[derive(Default)]
pub struct Bans {
    keys: Vec<PlayerKey>,
}

impl Bans {
    pub fn new() -> Self {
        Self { keys: Vec::new() }
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

    pub fn len(&self) -> usize {
        self.keys.len()
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }
}

/// The line the house says when an admin broadcasts. Prefixed so a player
/// can tell a server notice from a stranger typing the same words —
/// `pump_chat` sends it with `from = 0`, which names no player (ids start
/// at 256), and this marker is what makes that legible without the client
/// learning a new message shape.
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
}
