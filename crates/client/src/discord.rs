//! Discord rich presence — the local IPC socket, the frames, and the copy.
//!
//! ## What Discord already knew, and what it did not
//!
//! Discord shows "Playing <something>" by two entirely separate mechanisms,
//! and only one of them is ours to control. Measured 2026-08-16 rather than
//! assumed, because the difference decides whether this file is worth having:
//!
//! 1. **Its verified database.** The Discord desktop app polls the running
//!    process list and matches executable names against
//!    `GET /api/v9/applications/detectable` — 22,455 entries on the day this
//!    landed, and **not one of them is `gates` or `gates.exe`**. So nothing
//!    about our binary is recognised by Discord automatically, and nothing
//!    in this file changes that: getting into that list is a submission to
//!    Discord and therefore an operator act, not a code change.
//! 2. **The player's own "Add it!"** — Discord offers any unrecognised
//!    windowed process for manual registration, and thereafter displays it
//!    using the process name. That is the path that produces a bare
//!    lowercase `gates`, which is exactly what our binary is called
//!    (`ci/depot.py`'s launch contract: `gates` on Linux, `gates.exe` on
//!    Windows). It requires nothing of us and tells the viewer nothing
//!    beyond the word.
//!
//! ⚠ **That 22,455 is not a cap on who may do this**, and reading it as one
//! is the mistake this paragraph exists to stop. It is the curated list for
//! *passive detection*; rich presence needs only an **application id**, which
//! any developer creates with no review. The proof is in the same data — VS
//! Code, Spotify, Photoshop, Figma, Blender, OBS, IntelliJ and Neovim are all
//! **absent** from it, and every one of them appears in Discord statuses
//! daily, as rich presence under its own id.
//!
//! This module is that third path and the only one that is *ours*. It also
//! overrides the bare process name: **the application's NAME in the portal is
//! the word drawn after "Playing"**, so naming it `Gates` is what retires the
//! lowercase `gates`.
//!
//! ## Dark until an operator says otherwise
//!
//! [`app_id`] reads `GATES_DISCORD_APP_ID`; unset or malformed means **no
//! thread, no socket, no cost**. Same fail-closed shape scry's own announcer
//! uses (`meter/praeco.py`: no webhook configured, nothing posts).
//!
//! ## What a presence may say — and who decides
//!
//! **The player decides, and that is the operator's call** (2026-08-16:
//! *"nothing wrong with that if the player enables it? or at least a join
//! server button"*). v0 refused to carry an address at all; that refusal is
//! now a **default** rather than a wall, and the setting is the door. Two
//! settings, because they are two different disclosures:
//!
//! - `discord_presence` (default **on**) — the doing, the place, the party
//!   count. None of it locates a machine.
//! - `discord_share_server` (default **off**) — the shard's name and its
//!   address, which is what makes **Ask to Join** appear. This is opt-in
//!   because a presence line is public to everyone who can see the profile,
//!   and `reference/VOICE.md` §9.1 is what publishing an address costs when
//!   nobody chose it: the reference game's peer-to-peer voice let players
//!   read each other's IP and DDoS them.
//!
//! `an_unshared_activity_carries_no_address` is that boundary as a test, and
//! it is the one that fails when a later slice reaches for the address
//! outside the opt-in. The player's identity is never carried at all — the
//! wallet is the launcher's (`crate::scry`) and no setting opens it.
//!
//! ## No allocation on the frame path
//!
//! The Bevy side builds an [`Activity`] **every frame** to compare against
//! the last one sent, so `Activity` is `Copy` and every string in it is a
//! [`Text`] — a bounded inline buffer, not a `String`. The allocating half
//! (the JSON) happens on the worker thread, only when something changed.
//! That is `CLAUDE.md`'s no-per-frame-allocation law applied to a feature
//! that would otherwise format two sentences per frame forever.
//!
//! ## The wire
//!
//! A Unix domain socket (`$XDG_RUNTIME_DIR/discord-ipc-N`) or a Windows
//! named pipe (`\\.\pipe\discord-ipc-N`), N in 0..=9 because several Discord
//! builds can each hold one. Framing is an 8-byte header — opcode then
//! payload length, both little-endian `u32` — then UTF-8 JSON. [`frame`] is
//! that, and it is the whole protocol.
//!
//! **No dependency for any of it.** The payloads are a handful of fixed
//! shapes, which does not earn serde — the call `crate::config` makes about
//! TOML, in the same crate, for the same reason. The one thing we *read* is
//! a single string field out of one event ([`join_secret`]), and it is
//! deliberately a scanner rather than a parser: what protects us is not the
//! parse, it is that the value then goes through the same address whitelist
//! a pasted deeplink does. A mis-parse fails closed.
//!
//! ## Not feature-gated
//!
//! Everything here is `std` plus `sim_core::terrain::Biome`, so it builds and
//! tests without Bevy. `render/presence.rs` is the Bevy half. That split also
//! keeps every exhaustive match on the same side of `--features render` as
//! the type it covers — `CLAUDE.md`'s feature-line trap, which cost a Bevy
//! gate the day `Verb::Recycler` landed.

use std::fmt;
use std::io::{Read, Write};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sim_core::terrain::Biome;

/// The environment variable carrying the Discord application id. Unset means
/// dark — see the module header.
pub const APP_ID_ENV: &str = "GATES_DISCORD_APP_ID";

/// The art key Discord looks up for the large icon. It resolves to whatever
/// the operator uploads under this name in the developer portal's art
/// assets; until something is uploaded Discord draws no image, which is why
/// this is a constant rather than a knob.
pub const LARGE_IMAGE: &str = "gates";

/// Opcodes. Only two are ever sent from our side; a third arrives.
const OP_HANDSHAKE: u32 = 0;
const OP_FRAME: u32 = 1;

/// How many IPC endpoints to try. Discord numbers them 0..=9 so several
/// installed builds can coexist; the first that accepts a connection wins.
pub const IPC_SLOTS: u32 = 10;

/// Discord rate-limits activity updates to 5 per 20 seconds. This is the
/// floor the worker holds between sends, with headroom.
pub const MIN_UPDATE: Duration = Duration::from_secs(5);

/// How long the worker waits before trying the socket again when Discord is
/// not running — which is the normal case, not an error.
pub const RECONNECT_WAIT: Duration = Duration::from_secs(30);

/// The channel between the frame and the worker. Bounded, per wall 4.
///
/// **Overflow policy: the sender keeps the value and retries next frame.**
/// Not drop-oldest and not drop-newest, because both lose the *current*
/// state and this queue carries a latch rather than a stream — a dropped
/// transition would leave the presence permanently describing a screen the
/// player already left.
pub const QUEUE_CAP: usize = 8;

/// The join queue's cap. Smaller than [`QUEUE_CAP`] because a join is an act
/// a person performs, not a state that changes: more than two in flight is a
/// stuck client or a hostile one, and the overflow policy is to drop rather
/// than to grow.
pub const JOIN_CAP: usize = 2;

/// The largest incoming frame we will read. Discord's own frames are far
/// under this; the cap exists because the length is a `u32` **the other side
/// chooses**, and a reader that trusts it can be told to allocate 4 GiB.
pub const MAX_READ_BYTES: usize = 16 * 1024;

// ── a bounded inline string ──────────────────────────────────────────────────

/// A string with its bytes inline, so an [`Activity`] can be `Copy` and be
/// rebuilt every frame without touching the allocator.
///
/// `N` must be ≤ 255: the length is a `u8`, which is the bound made
/// unrepresentable rather than checked.
#[derive(Clone, Copy)]
pub struct Text<const N: usize> {
    buf: [u8; N],
    len: u8,
}

impl<const N: usize> Text<N> {
    /// Truncating, and **on a char boundary** — the reason [`Text::as_str`]
    /// can be infallible. A shard name is arbitrary text off a network
    /// document (`shardlist.rs`), so a multi-byte character landing across
    /// the cap is a real input and not a hypothetical.
    pub fn new(s: &str) -> Self {
        let mut buf = [0u8; N];
        let mut end = s.len().min(N);
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        buf[..end].copy_from_slice(&s.as_bytes()[..end]);
        Self {
            buf,
            len: end as u8,
        }
    }

    pub fn as_str(&self) -> &str {
        // Only ever filled from a `&str` cut at a char boundary, so this
        // cannot fail; the fallback keeps the panic out of the type anyway.
        std::str::from_utf8(&self.buf[..self.len as usize]).unwrap_or("")
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl<const N: usize> PartialEq for Text<N> {
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}
impl<const N: usize> Eq for Text<N> {}

impl<const N: usize> fmt::Debug for Text<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_str(), f)
    }
}

/// A shard's display name.
pub type ShardName = Text<48>;
/// A `host:port`. `shardlist::MAX_FIELD_BYTES` is the real bound on the way
/// in; this is the same order and leaves room for a long hostname.
pub type Addr = Text<80>;

// ── what the player is doing ─────────────────────────────────────────────────

/// Which screen the player is on, as a closed set. Deliberately coarser than
/// `render::menu::Screen`: `Paused` and `Map` are not facts a stranger needs,
/// and "on the island" is true for all three.
///
/// The exhaustive match over this type is [`State::doing`], **in this file**,
/// so a new variant fails the code tier rather than the Bevy gate.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum State {
    /// The default, and it is the honest one: before anything is known, the
    /// client is starting up. `Activity::default()` is also what a player
    /// who turns presence OFF publishes, which is why this variant carries
    /// no place, no party and no address.
    #[default]
    Booting,
    Menu,
    Joining,
    InWorld,
    Dead,
    Disconnected,
}

impl State {
    pub const ALL: [State; 6] = [
        State::Booting,
        State::Menu,
        State::Joining,
        State::InWorld,
        State::Dead,
        State::Disconnected,
    ];

    /// The verb phrase, without the place. Plain sentences on purpose: this
    /// is read by people who have never heard of the game.
    pub fn doing(self) -> &'static str {
        match self {
            State::Booting => "Starting up",
            State::Menu => "In the menu",
            State::Joining => "Joining a shard",
            State::InWorld => "Surviving",
            State::Dead => "Dead",
            State::Disconnected => "Disconnected",
        }
    }

    /// Whether a session clock belongs on this state. Discord renders a start
    /// timestamp as a running "elapsed" counter, honest for a session and
    /// noise for a menu.
    pub fn timed(self) -> bool {
        matches!(self, State::InWorld | State::Dead)
    }

    /// Whether the player is in a world at all — what decides if a place, a
    /// party or a join is meaningful.
    pub fn in_session(self) -> bool {
        matches!(self, State::InWorld | State::Dead)
    }
}

/// Where on the island. Mirrors [`Biome`] one-for-one, and the conversion
/// lives here so that adding a biome fails in the code tier.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Place {
    Shore,
    Meadow,
    Forest,
    Highland,
}

impl Place {
    pub const ALL: [Place; 4] = [Place::Shore, Place::Meadow, Place::Forest, Place::Highland];

    pub fn of(biome: Biome) -> Place {
        match biome {
            Biome::Beach => Place::Shore,
            Biome::Meadow => Place::Meadow,
            Biome::Forest => Place::Forest,
            Biome::Highland => Place::Highland,
        }
    }

    /// Reads after the verb: "Surviving **on the shore**".
    pub fn phrase(self) -> &'static str {
        match self {
            Place::Shore => "on the shore",
            Place::Meadow => "in the meadows",
            Place::Forest => "in the forest",
            Place::Highland => "in the highlands",
        }
    }
}

/// Everything one presence says. `Copy`, so the Bevy side rebuilds it every
/// frame and compares without allocating.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Activity {
    pub state: State,
    /// Where on the island. `None` outside a session, or before the first
    /// snapshot has placed the body.
    pub place: Option<Place>,
    /// Bodies on this shard, and the cap. Drawn as Discord's party size.
    pub party: Option<(u16, u16)>,
    /// The shard's name. `Some` **only** when the player shares the server —
    /// a name is a weaker disclosure than an address but it is still *which
    /// box*, so it rides the same setting.
    pub shard: Option<ShardName>,
    /// What a friend would dial. `Some` **only** when the player shares the
    /// server; this is what makes Ask to Join appear.
    pub join: Option<Addr>,
}

impl Activity {
    /// The first line: what the player is doing, and where.
    pub fn details(&self) -> String {
        match (self.state.in_session(), self.place) {
            (true, Some(p)) => format!("{} {}", self.state.doing(), p.phrase()),
            _ => self.state.doing().to_string(),
        }
    }

    /// The second line: which shard, and how busy. Empty when there is
    /// nothing true to say, and an empty `state` is simply omitted rather
    /// than drawn as a blank row.
    pub fn state_line(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if let Some(name) = self.shard.filter(|n| !n.is_empty()) {
            parts.push(name.as_str().to_string());
        }
        if let Some((here, cap)) = self.party {
            parts.push(if cap > 0 {
                format!("{here} of {cap} awake")
            } else {
                format!("{here} awake")
            });
        }
        parts.join(" · ")
    }

    /// A stable id for the party. Discord will not offer **Ask to Join**
    /// without one, and it must be the same string for two players on the
    /// same shard or they read as two parties — so it is derived from the
    /// address rather than from anything per-player.
    fn party_id(&self) -> Option<String> {
        let join = self.join?;
        Some(format!("gates-{:016x}", fnv1a(join.as_str().as_bytes())))
    }
}

/// FNV-1a, 64-bit. Not a security property and not on the wire — it exists to
/// turn an address into a short stable party id.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    h
}

// ── the app id ───────────────────────────────────────────────────────────────

/// The application id, or `None` for dark.
///
/// Shape-checked rather than trusted: Discord ids are decimal snowflakes, and
/// a stray quote or a pasted url would otherwise be sent to the socket and
/// rejected once per reconnect forever.
pub fn app_id() -> Option<String> {
    let raw = std::env::var(APP_ID_ENV).ok()?;
    let id = raw.trim();
    id_shaped(id).then(|| id.to_string())
}

/// The shape rule, factored out so the test checks the rule rather than a
/// copy of it.
pub fn id_shaped(id: &str) -> bool {
    (17..=20).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_digit())
}

// ── framing and payloads ─────────────────────────────────────────────────────

/// One IPC frame: opcode, payload length, payload. Both integers
/// little-endian, which is the protocol and not a host assumption.
pub fn frame(opcode: u32, payload: &str) -> Vec<u8> {
    let body = payload.as_bytes();
    let mut out = Vec::with_capacity(8 + body.len());
    out.extend_from_slice(&opcode.to_le_bytes());
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(body);
    out
}

/// Append `s` to `out` as a quoted JSON string.
///
/// Real escaping even where today's copy is static ASCII: a shard name is
/// arbitrary text off a network document, and a hand-rolled writer that only
/// escapes what its current inputs contain is the bug that ships the first
/// time the inputs change.
pub fn push_json_str(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

/// The handshake payload — RPC version 1 and the application id.
pub fn handshake_payload(app_id: &str) -> String {
    let mut s = String::with_capacity(64);
    s.push_str("{\"v\":1,\"client_id\":");
    push_json_str(&mut s, app_id);
    s.push('}');
    s
}

/// Ask Discord to tell us when someone accepts an invite. Without this
/// subscription the **Ask to Join** button still appears and simply does
/// nothing for a player whose game is already running, which is a worse
/// outcome than not offering it.
pub fn subscribe_join_payload(nonce: u64) -> String {
    let mut s = String::with_capacity(96);
    s.push_str("{\"cmd\":\"SUBSCRIBE\",\"evt\":\"ACTIVITY_JOIN\",\"nonce\":");
    push_json_str(&mut s, &nonce.to_string());
    s.push_str(",\"args\":{}}");
    s
}

/// The `SET_ACTIVITY` payload.
///
/// `pid` is required by the protocol: it is how Discord ties the activity to
/// a process and clears it when that process dies, which is why there is no
/// explicit teardown anywhere in this module.
pub fn activity_payload(a: &Activity, pid: u32, nonce: u64, start_unix: u64) -> String {
    let mut s = String::with_capacity(384);
    s.push_str("{\"cmd\":\"SET_ACTIVITY\",\"nonce\":");
    push_json_str(&mut s, &nonce.to_string());
    s.push_str(",\"args\":{\"pid\":");
    s.push_str(&pid.to_string());
    s.push_str(",\"activity\":{\"details\":");
    push_json_str(&mut s, &a.details());

    let state_line = a.state_line();
    if !state_line.is_empty() {
        s.push_str(",\"state\":");
        push_json_str(&mut s, &state_line);
    }
    if a.state.timed() {
        s.push_str(",\"timestamps\":{\"start\":");
        s.push_str(&start_unix.to_string());
        s.push('}');
    }
    // Party and join secret travel together or not at all: Discord shows no
    // Ask to Join without a party id, and a party with no secret is a size
    // counter nobody can act on.
    if let (Some(id), Some(join)) = (a.party_id(), a.join) {
        s.push_str(",\"party\":{\"id\":");
        push_json_str(&mut s, &id);
        if let Some((here, cap)) = a.party {
            s.push_str(",\"size\":[");
            s.push_str(&here.max(1).to_string());
            s.push(',');
            s.push_str(&cap.max(here).max(1).to_string());
            s.push(']');
        }
        s.push_str("},\"secrets\":{\"join\":");
        push_json_str(&mut s, join.as_str());
        s.push('}');
    }
    s.push_str(",\"assets\":{\"large_image\":");
    push_json_str(&mut s, LARGE_IMAGE);
    s.push_str(",\"large_text\":");
    push_json_str(&mut s, "Gates");
    s.push_str("}}}}");
    s
}

// ── reading one field back ───────────────────────────────────────────────────

/// The join secret out of an `ACTIVITY_JOIN` frame, or `None` for anything
/// else.
///
/// **A scanner, not a parser, and that is a deliberate bound on ambition.**
/// The only thing we ever read from Discord is one string, and what protects
/// the client is not this function — it is that the value goes through
/// `shardlist::check_addr`, the same whitelist a stranger's pasted deeplink
/// gets. So the failure mode that matters is "returns something wrong", and
/// something wrong is refused downstream; there is no shape of input here
/// that reaches the transport unchecked.
pub fn join_secret(payload: &str) -> Option<String> {
    // Both orderings appear in the wild depending on Discord build, so match
    // the event by field rather than by position.
    if !has_field(payload, "evt", "ACTIVITY_JOIN") {
        return None;
    }
    string_field(payload, "secret")
}

/// Is `"key":"value"` present, with `value` exactly this string?
fn has_field(doc: &str, key: &str, value: &str) -> bool {
    string_field(doc, key).is_some_and(|v| v == value)
}

/// The value of the first `"key": "..."` in `doc`, unescaped.
///
/// Deliberately simple: it finds the quoted key, steps over the colon, and
/// reads one JSON string. It does not track nesting, so a key repeated in
/// two objects yields the first — which for a one-event frame is the one
/// that matters, and which the whitelist downstream re-checks anyway.
fn string_field(doc: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    let mut from = 0usize;
    while let Some(at) = doc[from..].find(&needle) {
        let after = from + at + needle.len();
        let rest = doc[after..].trim_start();
        if let Some(rest) = rest.strip_prefix(':') {
            let rest = rest.trim_start();
            if let Some(body) = rest.strip_prefix('"') {
                return Some(read_json_string(body));
            }
        }
        from = after;
    }
    None
}

/// Read one JSON string body, up to the first unescaped quote.
fn read_json_string(body: &str) -> String {
    let mut out = String::new();
    let mut chars = body.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => break,
            '\\' => match chars.next() {
                Some('n') => out.push('\n'),
                Some('r') => out.push('\r'),
                Some('t') => out.push('\t'),
                Some('u') => {
                    // Four hex digits. An unreadable escape contributes
                    // nothing rather than aborting: the whitelist decides.
                    let hex: String = chars.by_ref().take(4).collect();
                    if let Some(ch) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                        out.push(ch);
                    }
                }
                Some(other) => out.push(other),
                None => break,
            },
            c => out.push(c),
        }
    }
    out
}

// ── where the socket lives ───────────────────────────────────────────────────

/// Where the Discord IPC endpoints live on this box, in the order to try.
///
/// The base directory is the first of `XDG_RUNTIME_DIR`, `TMPDIR`, `TMP`,
/// `TEMP`, then `/tmp` — Discord's own client library walks the same list.
/// The Flatpak and Snap packages bind their socket a directory deeper, so
/// those are searched too; a player on a sandboxed Discord is otherwise
/// silently unreachable. Windows takes no directory at all: named pipes live
/// in their own namespace.
pub fn socket_paths() -> Vec<String> {
    let mut out = Vec::with_capacity(IPC_SLOTS as usize);
    if cfg!(windows) {
        for n in 0..IPC_SLOTS {
            out.push(format!("\\\\.\\pipe\\discord-ipc-{n}"));
        }
        return out;
    }
    let base = ["XDG_RUNTIME_DIR", "TMPDIR", "TMP", "TEMP"]
        .iter()
        .find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty()))
        .unwrap_or_else(|| "/tmp".to_string());
    let base = base.trim_end_matches('/');
    // Bare first: an unsandboxed Discord is the common case, and every extra
    // candidate is a failed syscall on the way to it.
    for sub in ["", "app/com.discordapp.Discord", "snap.discord"] {
        for n in 0..IPC_SLOTS {
            if sub.is_empty() {
                out.push(format!("{base}/discord-ipc-{n}"));
            } else {
                out.push(format!("{base}/{sub}/discord-ipc-{n}"));
            }
        }
    }
    out
}

/// Seconds since the epoch, or 0 if the clock is before it. Only ever used
/// for Discord's elapsed counter, so a nonsense clock costs a wrong timer and
/// never a panic.
pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ── the frame's end of the link ──────────────────────────────────────────────

/// The frame's end of the link. Holds what was last accepted so the caller
/// can push on change and let a full queue simply retry.
pub struct Link {
    tx: SyncSender<Activity>,
    sent: Option<Activity>,
    joins: Receiver<Addr>,
}

impl Link {
    /// Push `activity` if it differs from the last one that got through.
    /// Returns whether anything was handed over. A full queue is not an error
    /// and not a loss: `sent` does not advance, so the next call tries the
    /// same value again.
    pub fn send(&mut self, activity: Activity) -> bool {
        if self.sent.as_ref() == Some(&activity) {
            return false;
        }
        match self.tx.try_send(activity) {
            Ok(()) => {
                self.sent = Some(activity);
                true
            }
            // Full: keep `sent` where it is and retry next frame.
            Err(TrySendError::Full(_)) => false,
            // The worker is gone. Record it as sent so a dead link stops
            // costing a syscall a frame; presence is over for this run.
            Err(TrySendError::Disconnected(_)) => {
                self.sent = Some(activity);
                false
            }
        }
    }

    /// A shard someone invited this player to, if one arrived since the last
    /// call. **Untrusted** — it has been through no address check yet, and
    /// the caller owes it `shardlist::check_addr` exactly as a pasted
    /// deeplink gets.
    pub fn poll_join(&mut self) -> Option<Addr> {
        // Empty and Disconnected are the same answer here: no invite. A
        // dead worker is not an error the frame can act on.
        self.joins.try_recv().ok()
    }
}

/// A link with no worker behind it, and both channel ends, for tests.
///
/// Exists so the send-on-change bookkeeping is testable without an
/// application id and without Discord running. The alternative was a test
/// that returns early when the environment is bare — the silent skip
/// `CLAUDE.md` names as the worst bug class.
#[cfg(test)]
pub fn test_link() -> (Link, Receiver<Activity>, SyncSender<Addr>) {
    let (tx, rx) = mpsc::sync_channel(QUEUE_CAP);
    let (jtx, jrx) = mpsc::sync_channel(JOIN_CAP);
    (
        Link {
            tx,
            sent: None,
            joins: jrx,
        },
        rx,
        jtx,
    )
}

/// Start the presence worker, or return `None` when there is nothing to do.
///
/// `None` means dark and is the shipping default: no application id in the
/// environment, no thread, no socket, no cost.
pub fn start() -> Option<Link> {
    let app_id = app_id()?;
    let (tx, rx) = mpsc::sync_channel(QUEUE_CAP);
    let (jtx, jrx) = mpsc::sync_channel(JOIN_CAP);
    // Detached: the process exits when main returns regardless, and Discord
    // clears the activity itself when our pid dies, so there is nothing to
    // join and nothing to tear down.
    std::thread::Builder::new()
        .name("discord-presence".to_string())
        .spawn(move || worker(app_id, rx, jtx))
        .ok()?;
    Some(Link {
        tx,
        sent: None,
        joins: jrx,
    })
}

/// The worker loop. Never panics and never blocks the game: every failure
/// path means "no presence", the same outcome as Discord not being installed.
fn worker(app_id: String, rx: Receiver<Activity>, joins: SyncSender<Addr>) {
    let pid = std::process::id();
    let start_unix = now_unix();
    let mut nonce: u64 = 0;
    let mut current: Option<Activity> = None;
    let mut pipe: Option<Pipe> = None;
    // When the last connect sweep was tried. A sweep is up to
    // `IPC_SLOTS * 3` failed `connect` calls, and without this a player
    // clicking through menus with Discord closed would run one per
    // transition — cheap individually, unbounded in aggregate.
    let mut last_try: Option<std::time::Instant> = None;

    loop {
        // Wait for a change, but **wake up anyway** every `RECONNECT_WAIT`.
        // Blocking on `recv` alone would be the obvious shape and it has a
        // hole big enough to be the normal case: a player who starts the
        // game first and Discord second reaches a steady screen, stops
        // changing state, and would never be announced at all.
        match rx.recv_timeout(RECONNECT_WAIT) {
            Ok(next) => current = Some(next),
            Err(RecvTimeoutError::Timeout) => {
                if pipe.is_some() || current.is_none() {
                    continue;
                }
            }
            Err(RecvTimeoutError::Disconnected) => return,
        }
        // Coalesce anything that piled up behind it — only the newest state
        // is worth sending, and this is what keeps the rate limit reachable.
        while let Ok(newer) = rx.try_recv() {
            current = Some(newer);
        }
        let Some(activity) = current else { continue };

        if pipe.is_none() {
            if last_try.is_some_and(|t| t.elapsed() < RECONNECT_WAIT) {
                continue;
            }
            last_try = Some(std::time::Instant::now());
            nonce = nonce.wrapping_add(1);
            pipe = Pipe::connect(&app_id, nonce, &joins);
            if pipe.is_none() {
                // Discord is not running, which is normal and not an error.
                // The `recv_timeout` above is the retry.
                continue;
            }
        }

        nonce = nonce.wrapping_add(1);
        let payload = activity_payload(&activity, pid, nonce, start_unix);
        if let Some(p) = pipe.as_mut() {
            if p.write(OP_FRAME, &payload).is_err() {
                // Discord closed or restarted. Drop the handle; the next
                // change reconnects, and the reader thread ends with it.
                pipe = None;
            }
        }
        // Hold the floor whether or not that write landed — a reconnect loop
        // that ignores the rate limit is how an integration gets throttled.
        std::thread::sleep(MIN_UPDATE);
    }
}

/// The platform's IPC handle. Two implementations behind one name so the
/// worker has no `cfg` in it.
///
/// **The `cfg` is on the field as well as the call**, which is the specific
/// shape the vendored scry SDK got wrong: it named `std::os::unix::net`
/// unconditionally and therefore could not compile for Windows at all, with
/// every gate in both repos green (`CLAUDE.md`, the vendoring note).
struct Pipe {
    #[cfg(unix)]
    sock: std::os::unix::net::UnixStream,
    #[cfg(windows)]
    sock: std::fs::File,
}

impl Pipe {
    /// Connect to the first endpoint that answers, handshake, subscribe to
    /// join events, and start the reader. `None` when Discord is not
    /// running.
    fn connect(app_id: &str, nonce: u64, joins: &SyncSender<Addr>) -> Option<Pipe> {
        for path in socket_paths() {
            let Some(mut pipe) = Pipe::open(&path) else {
                continue;
            };
            if pipe
                .write(OP_HANDSHAKE, &handshake_payload(app_id))
                .is_err()
            {
                continue;
            }
            // Best effort: a failed subscribe costs Ask-to-Join for an
            // already-running client and nothing else, so it is not a reason
            // to throw away a working presence connection.
            let _ = pipe.write(OP_FRAME, &subscribe_join_payload(nonce));
            pipe.spawn_reader(joins);
            return Some(pipe);
        }
        None
    }

    #[cfg(unix)]
    fn open(path: &str) -> Option<Pipe> {
        std::os::unix::net::UnixStream::connect(path)
            .ok()
            .map(|sock| Pipe { sock })
    }

    /// A Windows named pipe is opened as a file — the client half needs no
    /// pipe-specific call, which is why this needs no `windows-sys`.
    #[cfg(windows)]
    fn open(path: &str) -> Option<Pipe> {
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .ok()
            .map(|sock| Pipe { sock })
    }

    fn write(&mut self, opcode: u32, payload: &str) -> std::io::Result<()> {
        let bytes = frame(opcode, payload);
        self.sock.write_all(&bytes)?;
        self.sock.flush()
    }

    /// Read incoming frames on a cloned handle until the socket dies.
    ///
    /// **A thread rather than a read timeout**, because a Windows named pipe
    /// opened as a `File` has no `set_read_timeout` and the alternative is a
    /// platform split through the middle of the worker. It is self-cleaning:
    /// the only reason the worker drops a pipe is a failed write, which means
    /// the socket is dead, which ends the blocking read here.
    fn spawn_reader(&self, joins: &SyncSender<Addr>) {
        let Ok(mut sock) = self.sock.try_clone() else {
            return;
        };
        let joins = joins.clone();
        let _ = std::thread::Builder::new()
            .name("discord-join".to_string())
            .spawn(move || {
                let mut header = [0u8; 8];
                loop {
                    if sock.read_exact(&mut header).is_err() {
                        return;
                    }
                    let len = u32::from_le_bytes([header[4], header[5], header[6], header[7]]);
                    // The length is a number the other side chose. Wall 4
                    // applies out here too: a reader that trusts it can be
                    // told to allocate 4 GiB.
                    let len = len as usize;
                    if len > MAX_READ_BYTES {
                        return;
                    }
                    let mut body = vec![0u8; len];
                    if sock.read_exact(&mut body).is_err() {
                        return;
                    }
                    let Ok(text) = std::str::from_utf8(&body) else {
                        continue;
                    };
                    if let Some(secret) = join_secret(text) {
                        // Full means a join is already waiting unread; drop
                        // rather than grow, and the player clicks again.
                        let _ = joins.try_send(Text::new(&secret));
                    }
                }
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(state: State) -> Activity {
        Activity {
            state,
            ..Default::default()
        }
    }

    // ── the closed sets ──────────────────────────────────────────────────

    /// Adding a variant without adding it to `ALL` fails to compile here.
    #[test]
    fn all_variants_are_listed() {
        fn state_index(s: State) -> usize {
            match s {
                State::Booting => 0,
                State::Menu => 1,
                State::Joining => 2,
                State::InWorld => 3,
                State::Dead => 4,
                State::Disconnected => 5,
            }
        }
        for (i, s) in State::ALL.iter().enumerate() {
            assert_eq!(state_index(*s), i, "State::ALL is out of order at {i}");
        }
        fn place_index(p: Place) -> usize {
            match p {
                Place::Shore => 0,
                Place::Meadow => 1,
                Place::Forest => 2,
                Place::Highland => 3,
            }
        }
        for (i, p) in Place::ALL.iter().enumerate() {
            assert_eq!(place_index(*p), i, "Place::ALL is out of order at {i}");
        }
    }

    #[test]
    fn every_state_and_place_says_something() {
        for s in State::ALL {
            assert!(!s.doing().is_empty(), "{s:?} has no verb");
            assert!(s.doing().len() <= 128, "{s:?} verb too long");
        }
        for p in Place::ALL {
            assert!(!p.phrase().is_empty(), "{p:?} has no phrase");
        }
    }

    /// Every biome maps to a place. The `match` is exhaustive, so a new
    /// biome fails the CODE tier here rather than the Bevy gate.
    #[test]
    fn every_biome_has_a_place() {
        for b in [Biome::Beach, Biome::Meadow, Biome::Forest, Biome::Highland] {
            let p = Place::of(b);
            assert!(Place::ALL.contains(&p), "{b:?} mapped outside ALL");
        }
    }

    // ── the bounded string ───────────────────────────────────────────────

    #[test]
    fn text_truncates_on_a_char_boundary() {
        // 30 two-byte characters into a 48-byte buffer: 24 fit, and the cut
        // must not land inside the 24th.
        let s = "\u{e4}".repeat(30);
        let t = ShardName::new(&s);
        assert_eq!(t.as_str().chars().count(), 24);
        assert_eq!(t.as_str().len(), 48);
        // The invariant that matters: it is still valid UTF-8 to read.
        assert!(t.as_str().chars().all(|c| c == '\u{e4}'));
    }

    #[test]
    fn text_is_copy_and_compares_by_content() {
        let a = ShardName::new("us-east-1");
        let b = a; // moves nothing: Copy
        assert_eq!(a, b);
        assert_eq!(a.as_str(), "us-east-1");
        assert_ne!(a, ShardName::new("eu-1"));
        assert!(ShardName::new("").is_empty());
    }

    // ── framing ──────────────────────────────────────────────────────────

    #[test]
    fn the_header_is_two_little_endian_u32s() {
        let f = frame(OP_FRAME, "hi");
        assert_eq!(f.len(), 10);
        assert_eq!(&f[0..4], &[1, 0, 0, 0], "opcode is little-endian");
        assert_eq!(&f[4..8], &[2, 0, 0, 0], "length is little-endian");
        assert_eq!(&f[8..], b"hi");
    }

    #[test]
    fn the_length_counts_bytes_not_chars() {
        // Two chars, four bytes — as escapes, so the claim does not depend
        // on this file's encoding. A length in chars would desynchronise the
        // stream and every later frame with it.
        let s = "\u{e4}\u{e4}";
        assert_eq!(s.chars().count(), 2);
        assert_eq!(s.len(), 4);
        let f = frame(OP_FRAME, s);
        assert_eq!(&f[4..8], &[4, 0, 0, 0]);
        assert_eq!(f.len(), 12);
    }

    #[test]
    fn strings_escape_what_would_break_the_document() {
        let mut s = String::new();
        push_json_str(&mut s, "a\"b\\c\nd\te\u{1}f");
        assert_eq!(s, "\"a\\\"b\\\\c\\nd\\te\\u0001f\"");
        // No raw control byte survives, so a crafted shard name can never
        // inject a second field.
        assert!(!s.contains('\n'));
        assert!(!s.contains('\u{1}'));
    }

    #[test]
    fn the_handshake_names_version_one_and_the_id() {
        let s = handshake_payload("123456789012345678");
        assert!(s.contains("\"v\":1"));
        assert!(s.contains("\"client_id\":\"123456789012345678\""));
    }

    // ── the copy ─────────────────────────────────────────────────────────

    #[test]
    fn the_detail_line_names_the_place_only_in_a_session() {
        let mut a = plain(State::InWorld);
        a.place = Some(Place::Forest);
        assert_eq!(a.details(), "Surviving in the forest");
        // The same place while still on a menu is not a fact about the
        // player, and saying it would be a lie about where they are.
        let mut m = plain(State::Menu);
        m.place = Some(Place::Forest);
        assert_eq!(m.details(), "In the menu");
    }

    #[test]
    fn the_state_line_is_empty_when_there_is_nothing_true_to_say() {
        assert_eq!(plain(State::Menu).state_line(), "");
        let mut a = plain(State::InWorld);
        a.party = Some((7, 100));
        assert_eq!(a.state_line(), "7 of 100 awake");
        a.shard = Some(ShardName::new("us-east-1"));
        assert_eq!(a.state_line(), "us-east-1 · 7 of 100 awake");
    }

    #[test]
    fn an_empty_state_line_is_omitted_rather_than_drawn_blank() {
        let s = activity_payload(&plain(State::Menu), 1, 1, 1_700_000_000);
        assert!(!s.contains("\"state\""), "{s}");
        assert!(balanced(&s), "{s}");
    }

    #[test]
    fn an_activity_carries_the_copy_and_the_pid() {
        let mut a = plain(State::InWorld);
        a.place = Some(Place::Shore);
        let s = activity_payload(&a, 4242, 7, 1_700_000_000);
        assert!(s.contains("\"cmd\":\"SET_ACTIVITY\""));
        assert!(s.contains("\"pid\":4242"));
        assert!(s.contains("\"nonce\":\"7\""));
        assert!(s.contains("\"details\":\"Surviving on the shore\""));
        assert!(s.contains("\"timestamps\":{\"start\":1700000000}"));
        assert!(balanced(&s), "{s}");
    }

    #[test]
    fn an_untimed_state_carries_no_clock() {
        let s = activity_payload(&plain(State::Menu), 1, 1, 1_700_000_000);
        assert!(!s.contains("timestamps"));
        assert!(balanced(&s));
    }

    // ── the privacy boundary ─────────────────────────────────────────────

    /// The rule of the module header as a test rather than a paragraph: with
    /// sharing off, nothing that locates a machine may appear. This is what
    /// fails when a later slice reaches for the address outside the opt-in.
    #[test]
    fn an_unshared_activity_carries_no_address() {
        for state in State::ALL {
            let mut a = plain(state);
            a.place = Some(Place::Forest);
            a.party = Some((7, 100));
            // `shard` and `join` stay None — this is the setting being off.
            let s = activity_payload(&a, 1, 1, 1_700_000_000).to_lowercase();
            for banned in ["http", "://", "0x", "wallet", "secret", "party", "127."] {
                assert!(!s.contains(banned), "{state:?} leaks {banned}: {s}");
            }
        }
    }

    /// …and with sharing ON the address is there, deliberately, because the
    /// player asked for it. A test that only checked the negative would pass
    /// just as well over a feature that never worked.
    #[test]
    fn a_shared_activity_offers_the_join() {
        let mut a = plain(State::InWorld);
        a.shard = Some(ShardName::new("us-east-1"));
        a.join = Some(Addr::new("game.moreright.xyz:4433"));
        a.party = Some((7, 100));
        let s = activity_payload(&a, 1, 1, 1_700_000_000);
        assert!(s.contains("\"secrets\":{\"join\":\"game.moreright.xyz:4433\"}"));
        assert!(s.contains("\"party\":{\"id\":\"gates-"));
        assert!(s.contains("\"size\":[7,100]"));
        assert!(balanced(&s), "{s}");
    }

    /// Two players on one shard must read as ONE party or Discord shows two
    /// groups that cannot see each other.
    #[test]
    fn the_party_id_follows_the_shard_not_the_player() {
        let mk = |addr: &str| {
            let mut a = plain(State::InWorld);
            a.join = Some(Addr::new(addr));
            a.party_id()
        };
        assert_eq!(mk("a.example:4433"), mk("a.example:4433"));
        assert_ne!(mk("a.example:4433"), mk("b.example:4433"));
        // No party id without an address to derive one from.
        assert_eq!(plain(State::InWorld).party_id(), None);
    }

    /// A party with no secret is a size counter nobody can act on, so
    /// neither ships without the other.
    #[test]
    fn a_party_never_ships_without_its_secret() {
        let mut a = plain(State::InWorld);
        a.party = Some((3, 50)); // counted, but not shared
        let s = activity_payload(&a, 1, 1, 1_700_000_000);
        assert!(!s.contains("\"party\""), "{s}");
        assert!(!s.contains("\"secrets\""), "{s}");
    }

    // ── reading the join back ────────────────────────────────────────────

    #[test]
    fn a_join_event_yields_its_secret() {
        let doc = r#"{"cmd":"DISPATCH","evt":"ACTIVITY_JOIN","data":{"secret":"game.moreright.xyz:4433"}}"#;
        assert_eq!(join_secret(doc).as_deref(), Some("game.moreright.xyz:4433"));
    }

    #[test]
    fn another_event_yields_nothing() {
        let doc = r#"{"cmd":"DISPATCH","evt":"ACTIVITY_SPECTATE","data":{"secret":"nope:1"}}"#;
        assert_eq!(join_secret(doc), None);
        // …and a frame with no evt at all, which is every SET_ACTIVITY reply.
        assert_eq!(join_secret(r#"{"cmd":"SET_ACTIVITY","data":{}}"#), None);
    }

    #[test]
    fn a_secret_is_unescaped_and_never_trusted() {
        let doc = r#"{"evt":"ACTIVITY_JOIN","data":{"secret":"a\"b\\cA"}}"#;
        // The scanner's job is to hand over what was in the field, escapes
        // resolved. Refusing it is `shardlist::check_addr`'s job, and this
        // value is exactly the kind it refuses.
        assert_eq!(join_secret(doc).as_deref(), Some("a\"b\\cA"));
    }

    #[test]
    fn a_truncated_or_junk_frame_is_none_rather_than_a_panic() {
        for junk in [
            "",
            "{",
            r#"{"evt":"ACTIVITY_JOIN"}"#,
            r#"{"evt":"ACTIVITY_JOIN","data":{"secret":"#,
            r#"{"evt":"ACTIVITY_JOIN","data":{"secret":"unterminated"#,
            r#"{"evt":}"#,
        ] {
            let _ = join_secret(junk); // must not panic
        }
        assert_eq!(join_secret(r#"{"evt":"ACTIVITY_JOIN"}"#), None);
        // An unterminated string still yields what it had; the whitelist is
        // what turns that into a refusal.
        assert_eq!(
            join_secret(r#"{"evt":"ACTIVITY_JOIN","data":{"secret":"half"#).as_deref(),
            Some("half")
        );
    }

    // ── the rest ─────────────────────────────────────────────────────────

    #[test]
    fn the_socket_list_is_bounded_and_ordered() {
        let paths = socket_paths();
        assert!(!paths.is_empty());
        let want = if cfg!(windows) {
            IPC_SLOTS as usize
        } else {
            IPC_SLOTS as usize * 3
        };
        assert_eq!(paths.len(), want);
        assert!(paths[0].ends_with("discord-ipc-0"), "{}", paths[0]);
    }

    #[test]
    fn a_missing_or_malformed_app_id_is_dark() {
        for bad in ["", "  ", "not-a-number", "123", "12345678901234567890123"] {
            assert!(!id_shaped(bad.trim()), "{bad:?} should not pass");
        }
        assert!(id_shaped("123456789012345678"));
    }

    #[test]
    fn the_subscribe_names_the_join_event() {
        let s = subscribe_join_payload(3);
        assert!(s.contains("\"cmd\":\"SUBSCRIBE\""));
        assert!(s.contains("\"evt\":\"ACTIVITY_JOIN\""));
        assert!(balanced(&s), "{s}");
    }

    /// The payload is assembled by hand, so the one way it can be wrong in
    /// every arm at once is a missing closer.
    fn balanced(s: &str) -> bool {
        s.chars().filter(|c| *c == '{').count() == s.chars().filter(|c| *c == '}').count()
            && s.chars().filter(|c| *c == '[').count() == s.chars().filter(|c| *c == ']').count()
    }
}
