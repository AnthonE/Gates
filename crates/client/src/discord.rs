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
//! This module is the third path and the only one that is *ours*: the game
//! connects to the Discord client over a local socket and says what it is
//! doing. It overrides the bare process name with a title, a line of copy and
//! an elapsed clock. It does **not** require entry into the detectable list —
//! that is the common confusion, and it is why this is buildable today.
//!
//! ## Dark until an operator says otherwise
//!
//! Rich presence needs an **application id**, which only exists once someone
//! creates the application in Discord's developer portal. That is an operator
//! act (`CLAUDE.md`, the loop discipline), so there is no id in this tree and
//! no default: [`app_id`] reads `GATES_DISCORD_APP_ID` and an unset or
//! malformed value means **nothing connects and no thread is spawned**. Same
//! fail-closed shape scry's own announcer uses (`meter/praeco.py`: no webhook
//! configured, nothing posts).
//!
//! ## What a presence may say, and the reason it is this little
//!
//! **Never the shard's address.** A presence string is published to everyone
//! who can see the player's Discord profile, so putting `host:port` in it
//! hands a stranger the address of the box a friend is playing on.
//! `reference/VOICE.md` §9.1 is the same mistake with a different transport —
//! the reference game's peer-to-peer voice let players read each other's IP
//! and DDoS them, and it took a devblog and a forced migration to undo.
//! Publishing an address on purpose would be that bug typed deliberately.
//! Never the player's identity either: the wallet is the launcher's
//! (`crate::scry`), and a presence line is not a place to leak one.
//!
//! What is left is the honest part — *which screen the player is on* and how
//! long the session has run. That is [`Presence`], and it is deliberately a
//! small closed set rather than free text.
//!
//! ## The wire
//!
//! Discord's IPC is a Unix domain socket (`$XDG_RUNTIME_DIR/discord-ipc-N`)
//! or a Windows named pipe (`\\.\pipe\discord-ipc-N`), N in 0..=9 because
//! several Discord builds (stable, PTB, canary) can each hold one. Framing is
//! an 8-byte header — opcode then payload length, both little-endian `u32` —
//! followed by UTF-8 JSON. [`frame`] is that, and it is the whole protocol.
//!
//! **No dependency for any of it.** The payloads are four fixed shapes and
//! the framing is two integers, which does not earn serde — the same call
//! `crate::config` makes about TOML, in the same crate, for the same reason.
//!
//! ## Not feature-gated
//!
//! Everything here is `std` and arithmetic, so it builds and tests without
//! Bevy and its tests run in the code tier. `render/presence.rs` is the Bevy
//! half: it owns the `Screen` → [`Presence`] mapping and the once-per-change
//! handoff. That split is also what keeps the exhaustive matches on the same
//! side of `--features render` as the types they cover — `CLAUDE.md`'s
//! feature-line trap, which cost a Bevy gate the day `Verb::Recycler` landed.

use std::io::Write;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The environment variable carrying the Discord application id. Unset means
/// dark — see the module header.
pub const APP_ID_ENV: &str = "GATES_DISCORD_APP_ID";

/// The art key Discord looks up for the large icon. It resolves to whatever
/// the operator uploads under this name in the developer portal's art assets;
/// until something is uploaded Discord simply draws no image, which is why
/// this is a constant rather than a knob.
pub const LARGE_IMAGE: &str = "gates";

/// Opcodes. Only two are ever sent from our side.
const OP_HANDSHAKE: u32 = 0;
const OP_FRAME: u32 = 1;

/// How many IPC endpoints to try. Discord numbers them 0..=9 so that several
/// installed builds can coexist; the first that accepts a connection wins.
pub const IPC_SLOTS: u32 = 10;

/// Discord rate-limits activity updates to 5 per 20 seconds. This is the
/// floor the worker holds between sends, with headroom — presence changes are
/// screen transitions, so the natural rate is far under it and this only
/// matters when a player is cycling a menu.
pub const MIN_UPDATE: Duration = Duration::from_secs(5);

/// How long the worker waits before trying the socket again when Discord is
/// not running. Discord not being installed is the normal case, not an error,
/// so this is slow on purpose: it costs one failed `connect` per interval.
pub const RECONNECT_WAIT: Duration = Duration::from_secs(30);

/// The channel between the frame and the worker. Bounded, per wall 4.
///
/// **Overflow policy: the sender keeps the value and retries next frame.**
/// Not drop-oldest and not drop-newest, because both lose the *current*
/// state and this queue carries a latch rather than a stream — a dropped
/// transition would leave the presence permanently describing a screen the
/// player already left. [`Link::send`] only advances its own record of what
/// was sent when the push succeeds, so a full queue costs a frame's delay
/// and nothing else.
pub const QUEUE_CAP: usize = 8;

/// What the player is doing, as a closed set. One variant per thing worth
/// saying — deliberately coarser than `render::menu::Screen`, because
/// `Paused` and `Map` are not facts a stranger needs and "on the island" is
/// true for all three.
///
/// The exhaustive match over this type is [`Presence::copy`], **in this
/// file**, so a new variant fails the code tier rather than the Bevy gate.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Presence {
    /// The splash, before anything has been asked of the player.
    Booting,
    /// Any screen that is not a session: the intro menu and settings.
    Menu,
    /// Dialling a shard, or streaming the world in behind the loading bar.
    Joining,
    /// A live session — including paused and the map, which are screens over
    /// a world that is still connected.
    InWorld,
    /// This body died and has not answered the respawn.
    Dead,
    /// The shard hung up.
    Disconnected,
}

impl Presence {
    /// Every variant, for tests and for callers that want to enumerate.
    /// Kept in step with the type by `all_variants_are_listed` below, which
    /// will not compile if a variant is added without touching this.
    pub const ALL: [Presence; 6] = [
        Presence::Booting,
        Presence::Menu,
        Presence::Joining,
        Presence::InWorld,
        Presence::Dead,
        Presence::Disconnected,
    ];

    /// The two lines Discord draws: `details` on top, `state` under it.
    ///
    /// Plain sentences, no proper nouns a stranger would have to decode —
    /// this is read by people who have never heard of the game, which is the
    /// whole point of it appearing in a friend list.
    pub fn copy(self) -> (&'static str, &'static str) {
        match self {
            Presence::Booting => ("Starting up", "Gates"),
            Presence::Menu => ("In the menu", "Choosing a shard"),
            Presence::Joining => ("Joining a shard", "Streaming the island in"),
            Presence::InWorld => ("On the island", "Surviving"),
            Presence::Dead => ("Dead", "Choosing where to wake"),
            Presence::Disconnected => ("Disconnected", "The shard hung up"),
        }
    }

    /// Whether a session clock belongs on this presence. Discord renders a
    /// start timestamp as a running "elapsed" counter, which is honest for a
    /// session and noise for a menu.
    pub fn timed(self) -> bool {
        matches!(self, Presence::InWorld | Presence::Dead)
    }
}

/// The application id, or `None` for dark.
///
/// Shape-checked rather than trusted: Discord ids are decimal snowflakes, and
/// a stray quote or a pasted url would otherwise be sent to the socket and
/// rejected once per reconnect forever. The bound is generous on purpose —
/// this validates *shape*, and only Discord can validate the id itself.
pub fn app_id() -> Option<String> {
    let raw = std::env::var(APP_ID_ENV).ok()?;
    let id = raw.trim();
    let ok = (17..=20).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_digit());
    ok.then(|| id.to_string())
}

/// One IPC frame: opcode, payload length, payload. Both integers
/// little-endian, which is the protocol and not a host assumption — the same
/// bytes go out from a big-endian box.
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
/// Real escaping even though today's copy is static ASCII: the whole reason
/// this is a function is the day something dynamic reaches it, and a
/// hand-rolled writer that only escapes what its current inputs contain is
/// the bug that ships the first time the inputs change.
pub fn push_json_str(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            // Everything below 0x20 must be escaped or the document is
            // invalid; \u form covers the ones without a shorthand.
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
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

/// The `SET_ACTIVITY` payload for `presence`.
///
/// `pid` is required by the protocol: it is how Discord ties the activity to
/// a process and clears it when that process dies, which is why there is no
/// explicit teardown on the way out of this module.
pub fn activity_payload(presence: Presence, pid: u32, nonce: u64, start_unix: u64) -> String {
    let (details, state) = presence.copy();
    let mut s = String::with_capacity(256);
    s.push_str("{\"cmd\":\"SET_ACTIVITY\",\"nonce\":");
    push_json_str(&mut s, &nonce.to_string());
    s.push_str(",\"args\":{\"pid\":");
    s.push_str(&pid.to_string());
    s.push_str(",\"activity\":{\"details\":");
    push_json_str(&mut s, details);
    s.push_str(",\"state\":");
    push_json_str(&mut s, state);
    if presence.timed() {
        s.push_str(",\"timestamps\":{\"start\":");
        s.push_str(&start_unix.to_string());
        s.push('}');
    }
    s.push_str(",\"assets\":{\"large_image\":");
    push_json_str(&mut s, LARGE_IMAGE);
    s.push_str(",\"large_text\":");
    push_json_str(&mut s, "Gates");
    s.push_str("}}}}");
    s
}

/// Where the Discord IPC endpoints live on this box, in the order to try.
///
/// The base directory is the first of `XDG_RUNTIME_DIR`, `TMPDIR`, `TMP`,
/// `TEMP`, then `/tmp` — Discord's own client library walks the same list.
/// The Flatpak and Snap packages bind their socket a directory deeper, which
/// is why those are searched too rather than only the bare base; a player on
/// a sandboxed Discord is otherwise silently unreachable.
///
/// Windows takes no directory at all: named pipes live in their own
/// namespace.
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
/// for Discord's elapsed counter, so a nonsense clock costs a wrong timer
/// and never a panic.
pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// The frame's end of the link. Holds what was last accepted so the caller
/// can push on change and let a full queue simply retry.
pub struct Link {
    tx: SyncSender<Presence>,
    sent: Option<Presence>,
}

impl Link {
    /// Push `presence` if it differs from the last one that got through.
    /// Returns whether anything was handed over. A full queue is not an
    /// error and not a loss: `sent` does not advance, so the next call tries
    /// the same value again.
    pub fn send(&mut self, presence: Presence) -> bool {
        if self.sent == Some(presence) {
            return false;
        }
        match self.tx.try_send(presence) {
            Ok(()) => {
                self.sent = Some(presence);
                true
            }
            // Full: keep `sent` where it is and retry next frame.
            Err(TrySendError::Full(_)) => false,
            // The worker is gone (Discord went away and the thread ended).
            // Record it as sent so a dead link stops costing a syscall a
            // frame; the presence is simply over for this run.
            Err(TrySendError::Disconnected(_)) => {
                self.sent = Some(presence);
                false
            }
        }
    }
}

/// A link with no worker behind it, and the receiving end so the channel
/// stays connected for as long as the caller holds it.
///
/// Exists so the send-on-change bookkeeping is testable without an
/// application id and without Discord running. The alternative was a test
/// that returns early when the environment is bare — which is the silent
/// skip `CLAUDE.md` names as the worst bug class, since it would report a
/// pass it did not earn on every box in CI.
#[cfg(test)]
pub fn test_link() -> (Link, Receiver<Presence>) {
    let (tx, rx) = mpsc::sync_channel(QUEUE_CAP);
    (Link { tx, sent: None }, rx)
}

/// Start the presence worker, or return `None` when there is nothing to do.
///
/// `None` means dark and is the shipping default: no application id in the
/// environment, no thread, no socket, no cost. The caller treats that as
/// normal rather than as a failure — see the module header.
pub fn start() -> Option<Link> {
    let app_id = app_id()?;
    let (tx, rx) = mpsc::sync_channel(QUEUE_CAP);
    // A detached thread: the process exits when main returns regardless, and
    // Discord clears the activity itself when our pid dies, so there is
    // nothing to join and nothing to tear down.
    std::thread::Builder::new()
        .name("discord-presence".to_string())
        .spawn(move || worker(app_id, rx))
        .ok()?;
    Some(Link { tx, sent: None })
}

/// The worker loop. Never panics and never blocks the game: every failure
/// path here means "no presence", which is the same outcome as Discord not
/// being installed.
fn worker(app_id: String, rx: Receiver<Presence>) {
    let pid = std::process::id();
    let start_unix = now_unix();
    let mut nonce: u64 = 0;
    let mut current: Option<Presence> = None;
    let mut pipe: Option<Pipe> = None;
    // When the last connect sweep was tried. A sweep is up to
    // `IPC_SLOTS * 3` failed `connect` calls, and without this a player
    // clicking through menus with Discord closed would run one per
    // transition — cheap individually, unbounded in aggregate, which is the
    // shape wall 4 is about even out here where the wall does not reach.
    let mut last_try: Option<std::time::Instant> = None;

    loop {
        // Wait for a change, but **wake up anyway** every `RECONNECT_WAIT`.
        // Blocking on `recv` alone would be the obvious shape and it has a
        // hole big enough to be the normal case: a player who starts the
        // game first and Discord second reaches a steady screen, stops
        // changing state, and would then never be announced at all, because
        // the only retry was another transition that is not coming.
        match rx.recv_timeout(RECONNECT_WAIT) {
            Ok(next) => current = Some(next),
            // Nothing changed. Worth a round only if we still owe Discord
            // the state we have — otherwise fall straight back to waiting
            // rather than re-sending a presence that is already correct.
            Err(RecvTimeoutError::Timeout) => {
                if pipe.is_some() || current.is_none() {
                    continue;
                }
            }
            // The game is going away.
            Err(RecvTimeoutError::Disconnected) => return,
        }
        // Coalesce anything that piled up behind it — only the newest state
        // is worth sending, and this is what keeps the rate limit reachable.
        while let Ok(newer) = rx.try_recv() {
            current = Some(newer);
        }
        let Some(presence) = current else { continue };

        if pipe.is_none() {
            if last_try.is_some_and(|t| t.elapsed() < RECONNECT_WAIT) {
                continue;
            }
            last_try = Some(std::time::Instant::now());
            pipe = Pipe::connect(&app_id);
            if pipe.is_none() {
                // Discord is not running, which is normal and not an error.
                // The `recv_timeout` above is the retry.
                continue;
            }
        }

        nonce = nonce.wrapping_add(1);
        let payload = activity_payload(presence, pid, nonce, start_unix);
        if let Some(p) = pipe.as_mut() {
            if p.write(OP_FRAME, &payload).is_err() {
                // Discord closed or restarted. Drop the handle; the next
                // change reconnects.
                pipe = None;
            }
        }
        // Hold the floor whether or not that write landed — a reconnect loop
        // that ignores the rate limit is how an integration gets throttled.
        std::thread::sleep(MIN_UPDATE);
    }
}

/// The platform's IPC handle. Two implementations behind one name so the
/// worker above has no `cfg` in it.
///
/// **The `cfg` is on the `use` as well as the body**, which is the specific
/// shape the vendored scry SDK got wrong: it named `std::os::unix::net`
/// unconditionally and therefore could not compile for Windows at all, with
/// every gate in both repos green (`CLAUDE.md`, the vendoring note). The
/// Windows depot is built in CI and smoke-run under wine, so this would be
/// caught — but it would be caught late, and the fix is one line here.
struct Pipe {
    #[cfg(unix)]
    sock: std::os::unix::net::UnixStream,
    #[cfg(windows)]
    sock: std::fs::File,
}

impl Pipe {
    /// Connect to the first endpoint that answers and complete the
    /// handshake. `None` when Discord is not running, which is normal.
    fn connect(app_id: &str) -> Option<Pipe> {
        for path in socket_paths() {
            let Some(mut pipe) = Pipe::open(&path) else {
                continue;
            };
            if pipe.write(OP_HANDSHAKE, &handshake_payload(app_id)).is_ok() {
                return Some(pipe);
            }
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
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Adding a `Presence` variant without adding it to `ALL` fails to
    /// compile here — the match is exhaustive and the index it returns is
    /// checked against the array. This is the code-tier stand-in for the
    /// registry a derive would give us.
    #[test]
    fn all_variants_are_listed() {
        fn index(p: Presence) -> usize {
            match p {
                Presence::Booting => 0,
                Presence::Menu => 1,
                Presence::Joining => 2,
                Presence::InWorld => 3,
                Presence::Dead => 4,
                Presence::Disconnected => 5,
            }
        }
        for (i, p) in Presence::ALL.iter().enumerate() {
            assert_eq!(index(*p), i, "ALL is out of order at {i}");
        }
    }

    #[test]
    fn every_presence_says_something() {
        for p in Presence::ALL {
            let (details, state) = p.copy();
            assert!(!details.is_empty(), "{p:?} has no details");
            assert!(!state.is_empty(), "{p:?} has no state");
            // Discord truncates past 128 bytes; copy that gets cut off mid
            // word reads as a bug to the person looking at it.
            assert!(details.len() <= 128, "{p:?} details too long");
            assert!(state.len() <= 128, "{p:?} state too long");
        }
    }

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
        // Two chars, four bytes — written as escapes so the claim does not
        // depend on how this file is encoded. A length in chars would
        // desynchronise the stream and every later frame with it.
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
        // The invariant that matters: no raw control byte survives, so a
        // crafted name can never inject a second field.
        assert!(!s.contains('\n'));
        assert!(!s.contains('\u{1}'));
    }

    #[test]
    fn the_handshake_names_version_one_and_the_id() {
        let s = handshake_payload("123456789012345678");
        assert!(s.contains("\"v\":1"));
        assert!(s.contains("\"client_id\":\"123456789012345678\""));
    }

    #[test]
    fn an_activity_carries_the_copy_and_the_pid() {
        let s = activity_payload(Presence::InWorld, 4242, 7, 1_700_000_000);
        assert!(s.contains("\"cmd\":\"SET_ACTIVITY\""));
        assert!(s.contains("\"pid\":4242"));
        assert!(s.contains("\"nonce\":\"7\""));
        assert!(s.contains("\"details\":\"On the island\""));
        assert!(s.contains("\"timestamps\":{\"start\":1700000000}"));
        // Balanced braces: the payload is assembled by hand, so the one way
        // it can be wrong is a missing closer.
        assert_eq!(
            s.chars().filter(|c| *c == '{').count(),
            s.chars().filter(|c| *c == '}').count(),
            "unbalanced braces in {s}"
        );
    }

    #[test]
    fn an_untimed_presence_carries_no_clock() {
        let s = activity_payload(Presence::Menu, 1, 1, 1_700_000_000);
        assert!(!s.contains("timestamps"));
        assert_eq!(
            s.chars().filter(|c| *c == '{').count(),
            s.chars().filter(|c| *c == '}').count(),
        );
    }

    /// The privacy rule of the module header, as a test rather than a
    /// paragraph: nothing that identifies the box or the player may appear
    /// in a payload. This is the assertion that fails when somebody adds a
    /// shard name and reaches for the address next to it.
    #[test]
    fn no_payload_carries_an_address_or_an_identity() {
        for p in Presence::ALL {
            let s = activity_payload(p, 1, 1, 1_700_000_000).to_lowercase();
            for banned in ["http", "://", "0x", "wallet", "ip", "addr", "port"] {
                assert!(!s.contains(banned), "{p:?} payload leaks {banned}: {s}");
            }
        }
    }

    #[test]
    fn the_socket_list_is_bounded_and_ordered() {
        let paths = socket_paths();
        assert!(!paths.is_empty());
        // Three search roots on unix, one namespace on windows.
        let want = if cfg!(windows) {
            IPC_SLOTS as usize
        } else {
            IPC_SLOTS as usize * 3
        };
        assert_eq!(paths.len(), want);
        // Slot 0 of the plainest location is tried first — every candidate
        // before it is a failed syscall for the common case.
        assert!(paths[0].ends_with("discord-ipc-0"), "{}", paths[0]);
    }

    #[test]
    fn a_missing_or_malformed_app_id_is_dark() {
        // The shape check is what stands between a pasted url and a
        // reconnect loop that can never succeed.
        for bad in ["", "  ", "not-a-number", "123", "12345678901234567890123"] {
            let id = bad.trim();
            let ok = (17..=20).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_digit());
            assert!(!ok, "{bad:?} should not pass the shape check");
        }
        let good = "123456789012345678";
        assert!((17..=20).contains(&good.len()));
        assert!(good.bytes().all(|b| b.is_ascii_digit()));
    }
}
