//! scry overlay — the Rust reference client. One file, `std` only.
//!
//! What a native game (Gates) uses to reach a running scry launcher: who is
//! playing, a signature, the catalog, the shard list. **It holds no key and
//! has no code path that could.** See `sdk/PROTOCOL.md` and `docs/client/SDK.md`.
//!
//! `ignore`, not `no_run`: `submit`, `show` and `log` below are the GAME's own
//! functions, so this sketch never compiled — and nothing noticed until Gates
//! vendored the file into a cargo crate, where rustdoc finally tried. A doc
//! example that cannot build is a small lie in the one file a game reads first.
//!
//! ```ignore
//! let mut scry = scry_overlay::Overlay::connect("gates", "0.1.0");
//! match scry {
//!     Ok(mut ov) => {
//!         let who = ov.address();                     // Option<String>
//!         let msg = scry_overlay::play_message("duel", "vow_x", "ETH up 5", None);
//!         match ov.sign(&msg, "settling round 41") {
//!             Ok(sig)                          => submit(&sig.signature),
//!             Err(scry_overlay::SignError::Handoff(url)) => show_link(&url),
//!             Err(e)                           => show(&e.to_string()),
//!         }
//!     }
//!     // No launcher is a NORMAL state. Play anyway. Never fall back to
//!     // asking the player for a private key — there is no third option and
//!     // this file deliberately does not offer one.
//!     Err(why) => log(&format!("no scry launcher: {why}")),
//! }
//! ```
//!
//! Vendor it: no crates, no build script, no features. A project that already
//! has serde can delete the `json` module and swap in `serde_json` — the
//! client logic does not care which produced the map.

#![allow(dead_code)]

use std::env;
use std::fmt;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::time::Duration;

// ── the transport, and the one thing that differs by platform ───────────────
//
// On unix the door is a UNIX stream socket. On Windows there is no `AF_UNIX`
// in `std`, so it is a **named pipe** — which opens as an ordinary file, and
// `File` already has the `try_clone` + `Read` + `Write` shape `UnixStream`
// has. That is why this is a type alias and not an abstraction: the client
// logic below is byte-identical on both platforms.

#[cfg(unix)]
use std::os::unix::net::UnixStream;

#[cfg(unix)]
type Stream = UnixStream;

#[cfg(windows)]
type Stream = std::fs::File;

pub const PROTOCOL: i64 = 1;
pub const SOCKET_ENV: &str = "SCRY_LAUNCHER_SOCKET";
/// A consent prompt waits for a person, so the read timeout is generous.
/// A short one here turns "the player was reading the message" into a bug.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(300);

pub const FAMILIES: [&str; 13] = [
    "braid", "card", "covenant", "doc", "familiar", "hive", "holder", "meter", "pact", "play",
    "review", "store", "vow",
];

// ── a very small JSON, so this file needs no crates ─────────────────────────
pub mod json {
    use std::collections::BTreeMap;
    use std::fmt::Write as _;

    #[derive(Debug, Clone, PartialEq)]
    pub enum Value {
        Null,
        Bool(bool),
        Num(f64),
        Str(String),
        Arr(Vec<Value>),
        Obj(BTreeMap<String, Value>),
    }

    impl Value {
        pub fn get(&self, key: &str) -> Option<&Value> {
            match self {
                Value::Obj(m) => m.get(key),
                _ => None,
            }
        }
        pub fn as_str(&self) -> Option<&str> {
            match self {
                Value::Str(s) => Some(s),
                _ => None,
            }
        }
        pub fn as_bool(&self) -> bool {
            matches!(self, Value::Bool(true))
        }
        pub fn as_i64(&self) -> Option<i64> {
            match self {
                Value::Num(n) => Some(*n as i64),
                _ => None,
            }
        }
        /// `""` for anything missing or non-string. Callers here always want a
        /// displayable string and never need to tell absent from empty.
        pub fn str_or_empty(&self, key: &str) -> String {
            self.get(key)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string()
        }
        pub fn opt_str(&self, key: &str) -> Option<String> {
            match self.get(key) {
                Some(Value::Str(s)) if !s.is_empty() => Some(s.clone()),
                _ => None,
            }
        }
    }

    pub fn escape(s: &str, out: &mut String) {
        out.push('"');
        for c in s.chars() {
            match c {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                c if (c as u32) < 0x20 => {
                    let _ = write!(out, "\\u{:04x}", c as u32);
                }
                c => out.push(c),
            }
        }
        out.push('"');
    }

    /// Only what this client emits: an object of strings and integers.
    pub fn object(fields: &[(&str, Field)]) -> String {
        let mut out = String::from("{");
        for (i, (k, v)) in fields.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            escape(k, &mut out);
            out.push(':');
            match v {
                Field::S(s) => escape(s, &mut out),
                Field::I(n) => {
                    let _ = write!(out, "{n}");
                }
            }
        }
        out.push('}');
        out
    }

    pub enum Field<'a> {
        S(&'a str),
        I(i64),
    }

    pub fn parse(src: &str) -> Result<Value, String> {
        let b: Vec<char> = src.chars().collect();
        let mut i = 0usize;
        let v = parse_value(&b, &mut i)?;
        skip_ws(&b, &mut i);
        if i < b.len() {
            return Err(format!("trailing input at byte {i}"));
        }
        Ok(v)
    }

    fn skip_ws(b: &[char], i: &mut usize) {
        while *i < b.len() && b[*i].is_whitespace() {
            *i += 1;
        }
    }

    fn parse_value(b: &[char], i: &mut usize) -> Result<Value, String> {
        skip_ws(b, i);
        match b.get(*i) {
            None => Err("unexpected end of input".into()),
            Some('{') => parse_obj(b, i),
            Some('[') => parse_arr(b, i),
            Some('"') => Ok(Value::Str(parse_str(b, i)?)),
            Some('t') => lit(b, i, "true", Value::Bool(true)),
            Some('f') => lit(b, i, "false", Value::Bool(false)),
            Some('n') => lit(b, i, "null", Value::Null),
            Some(_) => parse_num(b, i),
        }
    }

    fn lit(b: &[char], i: &mut usize, word: &str, v: Value) -> Result<Value, String> {
        if b[*i..].iter().take(word.len()).collect::<String>() == word {
            *i += word.len();
            Ok(v)
        } else {
            Err(format!("expected {word}"))
        }
    }

    fn parse_num(b: &[char], i: &mut usize) -> Result<Value, String> {
        let start = *i;
        while *i < b.len() && (b[*i].is_ascii_digit() || "+-.eE".contains(b[*i])) {
            *i += 1;
        }
        b[start..*i]
            .iter()
            .collect::<String>()
            .parse::<f64>()
            .map(Value::Num)
            .map_err(|e| e.to_string())
    }

    fn parse_str(b: &[char], i: &mut usize) -> Result<String, String> {
        *i += 1; // opening quote
        let mut out = String::new();
        while *i < b.len() {
            match b[*i] {
                '"' => {
                    *i += 1;
                    return Ok(out);
                }
                '\\' => {
                    *i += 1;
                    let c = *b.get(*i).ok_or("dangling escape")?;
                    match c {
                        'n' => out.push('\n'),
                        'r' => out.push('\r'),
                        't' => out.push('\t'),
                        'b' => out.push('\u{8}'),
                        'f' => out.push('\u{c}'),
                        'u' => {
                            let hex: String = b[*i + 1..(*i + 5).min(b.len())].iter().collect();
                            let n = u32::from_str_radix(&hex, 16).map_err(|e| e.to_string())?;
                            // Surrogate pairs: a lone half is replaced rather
                            // than dropped, so a mangled string never silently
                            // shortens a message a player is about to sign.
                            out.push(char::from_u32(n).unwrap_or('\u{fffd}'));
                            *i += 4;
                        }
                        c => out.push(c),
                    }
                    *i += 1;
                }
                c => {
                    out.push(c);
                    *i += 1;
                }
            }
        }
        Err("unterminated string".into())
    }

    fn parse_arr(b: &[char], i: &mut usize) -> Result<Value, String> {
        *i += 1;
        let mut items = Vec::new();
        loop {
            skip_ws(b, i);
            if b.get(*i) == Some(&']') {
                *i += 1;
                return Ok(Value::Arr(items));
            }
            items.push(parse_value(b, i)?);
            skip_ws(b, i);
            match b.get(*i) {
                Some(',') => *i += 1,
                Some(']') => {}
                _ => return Err("expected , or ] in array".into()),
            }
        }
    }

    fn parse_obj(b: &[char], i: &mut usize) -> Result<Value, String> {
        *i += 1;
        let mut map = BTreeMap::new();
        loop {
            skip_ws(b, i);
            if b.get(*i) == Some(&'}') {
                *i += 1;
                return Ok(Value::Obj(map));
            }
            skip_ws(b, i);
            if b.get(*i) != Some(&'"') {
                return Err("expected a key".into());
            }
            let k = parse_str(b, i)?;
            skip_ws(b, i);
            if b.get(*i) != Some(&':') {
                return Err("expected :".into());
            }
            *i += 1;
            map.insert(k, parse_value(b, i)?);
            skip_ws(b, i);
            match b.get(*i) {
                Some(',') => *i += 1,
                Some('}') => {}
                _ => return Err("expected , or } in object".into()),
            }
        }
    }
}

use json::{Field, Value};

// ── messages, built offline ─────────────────────────────────────────────────

/// The exact text a wallet signs for one game action.
///
/// Rebuilt here rather than fetched, on purpose: a client that asks a server
/// what to sign has handed that server the ability to change what it is
/// signing. The format is deterministic (`meter/playauth.py`).
///
/// `day` is UTC `YYYY-MM-DD`; pass `None` for today. It is a parameter because
/// a round that straddles midnight should sign the day it started, and a
/// client that cannot express that would silently sign the wrong one.
/// The exact text a wallet signs for one game action.
///
/// ⚠ The subject is the WALLET, and it was the vow_id until 2026-08-12: the
/// signature recovers the signer, so a player does not swear a vow to act. The
/// address is LOWERCASED here — a checksummed one is different bytes and would
/// verify differently, which is the detail that bites a hand-rolled string.
pub fn play_message(action: &str, wallet: &str, detail: &str, day: Option<&str>) -> String {
    let today;
    let day = match day {
        Some(d) => d,
        None => {
            today = utc_day();
            &today
        }
    };
    let wallet = wallet.to_ascii_lowercase();
    format!("scry play\naction: {action}\nwallet: {wallet}\nday: {day}\ndetail: {detail}")
}

/// UTC `YYYY-MM-DD` from the clock, with no `chrono`. Civil-from-days is
/// Howard Hinnant's algorithm; it is exact for every date this will ever see.
pub fn utc_day() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let z = secs.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

// ── results ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Signature {
    pub signature: String,
    pub address: Option<String>,
    pub family: String,
    pub backend: String,
}

/// An identity proof — see [`Overlay::prove`].
///
/// ⚠ **`message` is an echo, not evidence.** It is here so a game can log or
/// display what was signed. A server MUST recompute the message from what it
/// already knows and verify against that; checking a signature against a
/// message the client supplied proves only that the client can do arithmetic.
#[derive(Debug, Clone)]
pub struct Proof {
    pub signature: String,
    pub address: Option<String>,
    pub message: String,
}

/// What the town says about one address — see [`Overlay::profile`].
///
/// Every field is a claim about a stranger, and `sworn` is the only thing
/// separating a checked name from a typed one.
///
/// `found == false` is a real answer: *we looked, there is no sworn identity*.
/// It is not the same as [`Overlay::profile`] returning `None`, which means we
/// could not look at all.
#[derive(Debug, Clone)]
pub struct Profile {
    pub raw: Value,
    pub reachable: bool,
    pub found: bool,
}

impl Profile {
    /// The SWORN name — in the locked index, and it cost SCRY to change.
    pub fn handle(&self) -> Option<String> {
        self.raw.opt_str("handle")
    }

    /// SELF-DECLARED. Whatever its owner typed. See [`Profile::label`].
    pub fn display_name(&self) -> Option<String> {
        self.raw.opt_str("display_name")
    }

    pub fn sworn(&self) -> bool {
        self.raw.get("sworn").map(Value::as_bool).unwrap_or(false)
    }

    pub fn address(&self) -> Option<String> {
        self.raw.opt_str("address")
    }

    /// A path on the origin — join it to the host before fetching.
    pub fn avatar(&self) -> Option<String> {
        self.raw.opt_str("avatar")
    }

    /// How many vows this wallet holds. One party, several identities.
    pub fn identities(&self) -> i64 {
        self.raw
            .get("identities")
            .and_then(Value::as_i64)
            .unwrap_or(1)
    }

    /// A path you fetch yourself — `achievements`, `holdings`, `reputation`,
    /// `vow`, `tip`. One verb is one origin read, the way Steam splits
    /// summaries from achievements.
    pub fn link(&self, what: &str) -> Option<String> {
        self.raw.get("links")?.opt_str(what)
    }

    /// The one string safe to draw without thinking about it.
    ///
    /// A sworn name renders bare; anything self-declared gets `~`, the town's
    /// marker for *this party said so and nobody checked* (`ACHIEVEMENTS.md`).
    /// A surface that shows a claimed name in the clothes of a checked one has
    /// lied for free, and it is the cheapest mistake to make and the hardest to
    /// notice.
    pub fn label(&self) -> String {
        if self.sworn() {
            if let Some(h) = self.handle() {
                return h;
            }
        }
        if let Some(n) = self.display_name() {
            if !n.is_empty() {
                return format!("~{n}");
            }
        }
        let addr = self.address().unwrap_or_default();
        if addr.len() >= 10 {
            format!("{}…{}", &addr[..6], &addr[addr.len() - 4..])
        } else {
            "anonymous".to_string()
        }
    }
}

/// Why a signature did not come back. **`Handoff` is not a failure** — it
/// means the player's signer is their browser and the act finishes there. A
/// game that retries on it will spin forever; show the link instead.
#[derive(Debug, Clone)]
pub enum SignError {
    /// The player said no at the consent prompt. Do not retry.
    RefusedByPlayer(String),
    /// Finish in a browser at this URL. Not an error.
    Handoff(String),
    /// The signer refused: unknown family, budget spent, unarmed, unreachable.
    Refused(String),
    /// No launcher, or the connection died.
    NoLauncher(String),
}

impl fmt::Display for SignError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SignError::RefusedByPlayer(s) => write!(f, "{s}"),
            SignError::Handoff(u) => write!(f, "finish this in your browser: {u}"),
            SignError::Refused(s) => write!(f, "{s}"),
            SignError::NoLauncher(s) => write!(f, "{s}"),
        }
    }
}

impl std::error::Error for SignError {}

// ── the client ──────────────────────────────────────────────────────────────

/// Where the door is when nobody named one.
///
/// ⚠ **This must agree with the launcher's own default exactly**
/// (`scry-broker/src/transport.rs`). A mismatch is the worst shape of bug
/// here: a game finds no launcher on a machine that is running one, reports
/// the normal "playing anonymously", and nothing anywhere is red.
/// `$SCRY_LAUNCHER_SOCKET` is set by the launcher on every native build it
/// starts and wins over both defaults, which is why that is the path a game
/// should actually end up using.
pub fn default_socket() -> PathBuf {
    if let Ok(p) = env::var(SOCKET_ENV) {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    platform_default()
}

#[cfg(unix)]
fn platform_default() -> PathBuf {
    if let Ok(rt) = env::var("XDG_RUNTIME_DIR") {
        if !rt.is_empty() {
            return PathBuf::from(rt).join("scry").join("launcher.sock");
        }
    }
    PathBuf::from(env::var("HOME").unwrap_or_default()).join(".cache/scry/launcher/launcher.sock")
}

/// One pipe per user, because the Windows pipe namespace is machine-wide. Two
/// people signed into one box must not land on the same door — a game would
/// otherwise ask the wrong session's launcher to sign.
#[cfg(windows)]
fn platform_default() -> PathBuf {
    let user = env::var("USERNAME").unwrap_or_default();
    let user: String = user
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    let user = if user.is_empty() {
        "default".to_string()
    } else {
        user
    };
    PathBuf::from(format!(r"\\.\pipe\scry-launcher-{user}"))
}

/// Open the door, or say there is none. **`Err` is a normal state** — it means
/// no launcher is running, and a game plays fine without one.
#[cfg(unix)]
fn open_stream(path: &std::path::Path, timeout: Duration) -> Result<Stream, String> {
    let stream =
        UnixStream::connect(path).map_err(|e| format!("no launcher at {}: {e}", path.display()))?;
    // A consent prompt waits for a person, so these are generous. A short read
    // timeout turns "the player was reading the message" into a bug.
    stream.set_read_timeout(Some(timeout)).ok();
    stream.set_write_timeout(Some(timeout)).ok();
    Ok(stream)
}

/// The Windows half — a named pipe, which opens as a file.
///
/// ⚠ **The timeout bounds the CONNECT, not the reads, and that difference is
/// real.** `WaitNamedPipeW` waits for a free instance for `timeout` and then
/// gives up, so a launcher that is not running still fails fast. Once open,
/// reads block: bounding them wants overlapped I/O, which is a great deal of
/// machinery for a file whose whole property is that it vendors with no crates
/// and no build script.
///
/// What that costs, said plainly rather than discovered: a launcher that
/// accepts a connection and then never answers will hang the calling game
/// instead of erroring after `timeout`. On unix the same launcher returns an
/// error. That is a launcher bug in both cases; on Windows the game feels it as
/// a freeze. A game that cares should call the SDK off its main thread — which
/// is good practice anyway, because a consent prompt waits for a human.
#[cfg(windows)]
fn open_stream(path: &std::path::Path, timeout: Duration) -> Result<Stream, String> {
    use std::os::windows::ffi::OsStrExt;

    extern "system" {
        fn WaitNamedPipeW(name: *const u16, timeout_ms: u32) -> i32;
    }

    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let ms = timeout.as_millis().min(u32::MAX as u128) as u32;
    // Not fatal on its own: the pipe may already have a free instance, in
    // which case the open below simply succeeds. This only makes a busy
    // launcher wait rather than fail instantly.
    // SAFETY: `wide` is a NUL-terminated UTF-16 buffer that outlives the call.
    unsafe { WaitNamedPipeW(wide.as_ptr(), ms) };

    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(|e| format!("no launcher at {}: {e}", path.display()))
}

pub struct Overlay {
    stream: Stream,
    reader: BufReader<Stream>,
    pub hello: Value,
}

impl Overlay {
    /// Connect and say hello. `Err` is a **normal** state — it means no
    /// launcher is running, and a game must play fine without one.
    pub fn connect(game: &str, version: &str) -> Result<Overlay, String> {
        Overlay::connect_at(&default_socket(), game, version, DEFAULT_TIMEOUT)
    }

    pub fn connect_at(
        path: &std::path::Path,
        game: &str,
        version: &str,
        timeout: Duration,
    ) -> Result<Overlay, String> {
        let stream = open_stream(path, timeout)?;
        let reader = BufReader::new(stream.try_clone().map_err(|e| e.to_string())?);
        let mut ov = Overlay {
            stream,
            reader,
            hello: Value::Null,
        };
        let reply = ov.call(&json::object(&[
            ("op", Field::S("hello")),
            ("game", Field::S(game)),
            ("version", Field::S(version)),
            ("protocol", Field::I(PROTOCOL)),
        ]))?;
        if !reply.as_bool_key("ok") {
            return Err(reply.str_or_empty("reason"));
        }
        ov.hello = reply;
        Ok(ov)
    }

    /// Which backend holds the key: `local` · `browser` · `arca` · `external`
    /// · `none`. Worth reading before asking — with `none` there is nothing to
    /// ask, and with `browser` expect a handoff rather than a signature and
    /// shape the UI for it.
    ///
    /// ⚠ **`local` is what the shipped client reports** once the player has
    /// made an account in it, and it is the only backend `prove` works on. The
    /// others are reachable by anyone building on the launcher crates; a
    /// downloaded launcher answers `local` or `none` and nothing else.
    pub fn signer(&self) -> String {
        self.hello.str_or_empty("signer")
    }

    pub fn protocol(&self) -> i64 {
        self.hello
            .get("protocol")
            .and_then(|v| v.as_i64())
            .unwrap_or(0)
    }

    /// The address the player asked their launcher to watch.
    ///
    /// A CLAIM, not authentication — anything can say a number. If it matters,
    /// ask for a signature over something you chose and recover the signer.
    pub fn address(&mut self) -> Option<String> {
        let reply = self
            .call(&json::object(&[("op", Field::S("identity"))]))
            .ok()?;
        if reply.as_bool_key("ok") {
            reply.opt_str("address")
        } else {
            None
        }
    }

    pub fn sign(&mut self, text: &str, why: &str) -> Result<Signature, SignError> {
        let req = json::object(&[
            ("op", Field::S("sign")),
            ("text", Field::S(text)),
            ("why", Field::S(why)),
        ]);
        let reply = self.call(&req).map_err(SignError::NoLauncher)?;
        if reply.as_bool_key("ok") {
            return Ok(Signature {
                signature: reply.str_or_empty("signature"),
                address: reply.opt_str("address"),
                family: reply.str_or_empty("family"),
                backend: reply.str_or_empty("backend"),
            });
        }
        let reason = reply.str_or_empty("reason");
        if let Some(url) = reply.opt_str("handoff") {
            return Err(SignError::Handoff(url));
        }
        if reply.str_or_empty("refused_by") == "the player" {
            return Err(SignError::RefusedByPlayer(reason));
        }
        Err(SignError::Refused(reason))
    }

    /// **Prove to your own server who is playing.** This is the verb behind a
    /// ticket check, and it is the one to reach for — not [`Overlay::address`],
    /// which is a claim anything can make.
    ///
    /// The reply is a **SIWE (EIP-4361) message and signature**, so your
    /// backend needs no scry-specific code — hand both to any `siwe` library
    /// (JS, Python, Rust, Go; built into viem and ethers):
    ///
    /// ```text
    /// let m = siwe::Message::from_str(&body.message)?;
    /// m.verify(&sig, &VerificationOpts {
    ///     domain: Some("shard-3.gates.example".parse()?),
    ///     nonce:  Some(the_nonce_you_issued),
    ///     ..Default::default()
    /// })?;
    /// // m.address is now proven. Look it up and admit or kick.
    /// ```
    ///
    /// `server` is your **domain** — `shard-3.gates.example`, optionally with
    /// `:port`. No scheme, no path: EIP-4361 binds the domain, and the launcher
    /// writes the `URI:` line itself.
    ///
    /// ⚠ **The nonce must be one you issued and have not seen before**, and
    /// EIP-4361 requires **at least 8 alphanumeric characters**. A dashed uuid
    /// is refused — strip the dashes. A signature over a message with no fresh
    /// nonce is valid forever to anyone who captures it.
    ///
    /// ⚠ **Do not recompute the message string and compare.** It carries
    /// `Issued At` from the launcher's clock, so you cannot rebuild it. Parse
    /// it and check the fields you care about — which is what `verify` does,
    /// and why passing your own domain and nonce to it is the whole check.
    ///
    /// **No consent prompt fires for this**, and that is by construction rather
    /// than by permission: the launcher composes every word of the message, so
    /// a game cannot smuggle a sentence into an unprompted signature. `sign` —
    /// where the game writes the text — still asks the player.
    pub fn prove(&mut self, server: &str, nonce: &str) -> Result<Proof, SignError> {
        let req = json::object(&[
            ("op", Field::S("prove")),
            ("nonce", Field::S(nonce)),
            ("server", Field::S(server)),
        ]);
        let reply = self.call(&req).map_err(SignError::NoLauncher)?;
        if reply.as_bool_key("ok") {
            return Ok(Proof {
                signature: reply.str_or_empty("signature"),
                address: reply.opt_str("address"),
                message: reply.str_or_empty("message"),
            });
        }
        Err(SignError::Refused(reply.str_or_empty("reason")))
    }

    /// The URL of a title's shard list, which YOU fetch. The launcher does not
    /// proxy it — a launcher between a game and its own server list is a cache
    /// nobody asked for and a ranking nobody can see.
    pub fn servers_url(&mut self, slug: &str) -> Option<String> {
        let reply = self
            .call(&json::object(&[
                ("op", Field::S("servers")),
                ("slug", Field::S(slug)),
            ]))
            .ok()?;
        if reply.as_bool_key("ok") {
            reply.opt_str("url")
        } else {
            None
        }
    }

    /// The URL of this title's **manifest**, which YOU fetch.
    ///
    /// ⚠ It returns a url, not a title object — the launcher does not proxy a
    /// game's own documents, the same rule [`Overlay::servers_url`] states.
    /// This read `reply.get("title")` until 2026-08-07 and therefore answered
    /// `None` for every slug against the real broker, which has always replied
    /// with `url`. It went unnoticed because this client was only ever driven
    /// against a second broker that no longer exists.
    pub fn title(&mut self, slug: &str) -> Option<String> {
        let reply = self
            .call(&json::object(&[
                ("op", Field::S("title")),
                ("slug", Field::S(slug)),
            ]))
            .ok()?;
        if reply.as_bool_key("ok") {
            reply.opt_str("url")
        } else {
            None
        }
    }

    /// One snapshot a game can DRAW — the overlay's read.
    ///
    /// Operator, 2026-08-05: *"and what about an overlay?"* Steam injects a
    /// library and hooks the graphics API because Steam does not own the
    /// games. This platform does, so the overlay is **the game rendering its
    /// own HUD** from this call — which is why this file has carried the name
    /// `scry_overlay` since before the feature existed. No injection, nothing
    /// to break on a driver update, nothing that reads as malware to an
    /// anti-cheat.
    ///
    /// ⚠ **Display freely; never approve.** A game controls its own pixels, so
    /// it can draw a convincing fake of any dialog. This reply carries no
    /// consent token, no pending-request queue and no already-allowed flag,
    /// and the launcher will not accept one back. Asking the player is the
    /// launcher's job, in the launcher's own window, per game and per family.
    /// A HUD that draws its own *"allow this signature?"* is drawing a forgery
    /// — call [`Overlay::sign`] and let the launcher ask.
    ///
    /// Cheap enough to poll each frame: it is one round trip over a local
    /// socket, and every field in it could already be asked for one at a time.
    pub fn overlay(&mut self, slug: &str) -> Option<Value> {
        let reply = self
            .call(&json::object(&[
                ("op", Field::S("overlay")),
                ("slug", Field::S(slug)),
            ]))
            .ok()?;
        if reply.as_bool_key("ok") {
            reply.get("overlay").cloned()
        } else {
            None
        }
    }

    /// **A name and a face to draw** — Steam's `GetPlayerSummaries`.
    ///
    /// Pass `""` for the address this launcher watches, or an address to look
    /// somebody else up — the one you recovered from [`Overlay::prove`], say,
    /// or another player on your shard.
    ///
    /// ⚠ **Three states, and a game that collapses them tells a lie.** `None`
    /// means *we could not look*: the origin was unreachable, or this launcher
    /// has no reader. [`Profile::found`] `== false` means *we looked and there
    /// is no sworn identity*, which is a normal way to play. Drawing "no name"
    /// for the first invents a fact about a stranger, and it is the cheapest
    /// mistake in this file to make.
    ///
    /// ⚠ **This is not authentication.** It describes an address; it does not
    /// establish that the player holds its key. Call [`Overlay::prove`] for
    /// that, then look up the address you recovered.
    pub fn profile(&mut self, address: &str) -> Option<Profile> {
        let mut fields = vec![("op", Field::S("profile"))];
        if !address.is_empty() {
            fields.push(("address", Field::S(address)));
        }
        let reply = self.call(&json::object(&fields)).ok()?;
        if !reply.as_bool_key("ok") {
            return None;
        }
        let reachable = reply.get("reachable").map(Value::as_bool).unwrap_or(true);
        match reply.get("profile") {
            Some(Value::Null) | None => Some(Profile {
                raw: Value::Null,
                reachable,
                found: false,
            }),
            Some(v) => Some(Profile {
                raw: v.clone(),
                reachable,
                found: true,
            }),
        }
    }

    pub fn open_url(&mut self, url: &str) -> bool {
        self.call(&json::object(&[
            ("op", Field::S("open")),
            ("url", Field::S(url)),
        ]))
        .map(|r| r.as_bool_key("ok"))
        .unwrap_or(false)
    }

    fn call(&mut self, line: &str) -> Result<Value, String> {
        self.stream
            .write_all(line.as_bytes())
            .and_then(|_| self.stream.write_all(b"\n"))
            .and_then(|_| self.stream.flush())
            .map_err(|e| format!("lost the launcher: {e}"))?;
        let mut buf = String::new();
        let n = self
            .reader
            .read_line(&mut buf)
            .map_err(|e| format!("lost the launcher: {e}"))?;
        if n == 0 {
            return Err("the launcher closed the connection".into());
        }
        json::parse(buf.trim()).map_err(|e| format!("the launcher replied with non-JSON: {e}"))
    }
}

trait BoolKey {
    fn as_bool_key(&self, key: &str) -> bool;
}

impl BoolKey for Value {
    fn as_bool_key(&self, key: &str) -> bool {
        self.get(key).map(|v| v.as_bool()).unwrap_or(false)
    }
}

// ── tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_roundtrips_the_shapes_the_broker_sends() {
        let v = json::parse(
            r#"{"ok":true,"signature":"0xab","n":3,"nested":{"a":[1,"two",null]},"esc":"a\nb"}"#,
        )
        .unwrap();
        assert!(v.as_bool_key("ok"));
        assert_eq!(v.str_or_empty("signature"), "0xab");
        assert_eq!(v.get("n").unwrap().as_i64(), Some(3));
        assert_eq!(v.str_or_empty("esc"), "a\nb");
        assert!(matches!(
            v.get("nested").unwrap().get("a"),
            Some(Value::Arr(_))
        ));
    }

    #[test]
    fn a_message_with_newlines_survives_the_encoder() {
        // The whole protocol carries multi-line messages; an encoder that
        // ate a newline would change what the player signs.
        let msg = play_message("duel", "vow_x", "ETH up 5", Some("2026-08-04"));
        let line = json::object(&[("op", Field::S("sign")), ("text", Field::S(&msg))]);
        let back = json::parse(&line).unwrap();
        assert_eq!(back.str_or_empty("text"), msg);
        assert_eq!(msg.lines().count(), 5);
    }

    #[test]
    fn play_message_matches_the_servers_format() {
        assert_eq!(
            play_message("answer", "0xAbC1", "abc", Some("2026-08-04")),
            "scry play\naction: answer\nwallet: 0xabc1\nday: 2026-08-04\ndetail: abc"
        );
    }

    #[test]
    fn utc_day_is_a_real_date() {
        let d = utc_day();
        assert_eq!(d.len(), 10);
        assert_eq!(d.as_bytes()[4], b'-');
        assert_eq!(d.as_bytes()[7], b'-');
    }

    #[test]
    fn missing_launcher_is_an_error_not_a_panic() {
        let e = Overlay::connect_at(
            std::path::Path::new("/nonexistent/scry.sock"),
            "t",
            "1",
            Duration::from_millis(50),
        );
        assert!(e.is_err());
    }
}
