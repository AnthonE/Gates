//! **Does this player own a copy?** — the licence's teeth, and the only
//! thing in this repo that asks another host a question about a person.
//!
//! A copy of Gates is an ERC-721 (`ScryGameTicket`, one deployment per
//! title) and holding any token of it is the licence. scry's origin answers
//! two questions about that, both of them one `balanceOf` underneath:
//!
//! - `GET  /api/ticket/{slug}/of/{wallet}` — one wallet, asked at join.
//! - `POST /api/ticket/{slug}/check` — the whole roster in one call, asked
//!   on an interval, because a player can **sell the ticket and stay logged
//!   in**. That is the case this module exists for; a join check alone would
//!   be a door with no lock behind it.
//!
//! ## Three answers, never two — this is the whole design
//!
//! [`Verdict`] has `Owns`, `Nope` and `Unknown`, and the third one is not a
//! degenerate case of the second. *"The chain says zero"* and *"we could not
//! ask"* are opposite facts that arrive down the same wire, and collapsing
//! them is how an RPC outage boots every paying player off the shard at
//! once. scry's own route says so in its response body (`kick_rule`), the
//! platform's `CLAUDE.md` names it as a trap it has already paid for, and
//! this type is that rule made unrepresentable-wrong: there is no `bool`
//! anywhere in this file for a caller to get backwards.
//!
//! **So the two policies are deliberately different, and both fail toward
//! the player:**
//!
//! | | `Owns` | `Nope` | `Unknown` |
//! |---|---|---|---|
//! | at join | admit | refuse `REFUSE_TICKET` | **admit** |
//! | on sweep | keep | kick | **keep** |
//!
//! Admitting on `Unknown` is a choice and it is the right one here. The game
//! is open source and the depot is served openly; the licence is a posted
//! rule about official servers, not a secret to be defended by an outage.
//! A shard that refused everyone whenever scry was slow would fail loudly at
//! the players and quietly at the operator, which is backwards. The counters
//! (`stats.rs`) make the fail-open visible instead.
//!
//! ## Polling state, never watching events
//!
//! `balanceOf` at `latest`, on an interval. Watching `Transfer` would look
//! cleaner and is a machine that misses on this chain: `eth_getLogs` is
//! refused past ~100 blocks on 4663, about ten seconds at 101 ms blocks, so
//! a watcher that blinks loses the transfer forever. State cannot miss.
//! (scry's `CLAUDE.md` carries the measurement; we take the conclusion.)
//!
//! ## Where this runs
//!
//! **Never on the sim thread** (wall 3) and never on the accept loop's own
//! task. The join check runs inside the handshake task, which is already
//! its own future and is already allowed to wait — it is doing a network
//! round trip either way. The sweep is a plain `std::thread`, the shape
//! `status.rs` already uses, and it talks to the accept loop through a
//! channel carrying nothing but slot numbers.
//!
//! Bounded like everything else (wall 4): a capped response read, a global
//! timeout on every call, and a roster page that cannot exceed the cap the
//! far end publishes.
//!
//! ## What is not here
//!
//! No key material, ever — this asks a public question about a public
//! address. Nothing in this file logs a wallet: an address names a person,
//! `PlayerKey` refuses to `Debug`-print its own bytes for that reason, and
//! a refusal counted is worth more than a refusal narrated.

use std::time::Duration;

/// The far end's own page cap (`meter/tickets.py::CHECK_MAX_WALLETS`). Our
/// roster cannot exceed `MAX_PLAYERS`, which is 100, so a full shard is
/// exactly one page and this never has to paginate — but the constant is
/// named rather than assumed, because the day `MAX_PLAYERS` grows is the day
/// a silent truncation would start kicking nobody and reporting success.
pub const CHECK_MAX_WALLETS: usize = 100;

/// Wall 4 on a network read: the far end streaming forever must not grow
/// this process's heap. One page of 100 verdicts is ~6 KB of JSON; this is
/// comfortably past it and refuses rather than truncating, because a
/// truncated answer parses into *fewer wallets checked* and that reads as
/// "everyone is fine".
pub const MAX_RESPONSE_BYTES: usize = 64 * 1024;

/// How long any one call may take. Shorter than the sweep interval by
/// construction — see [`Config::sane`] — so a slow origin cannot stack
/// rounds until the shard is polling itself in a loop. `status.rs`'s poller
/// makes the same trade for the same reason.
///
/// Seconds as a plain integer, with the `Duration` derived below, because
/// `ci/knob_registry.mjs` pins a number and cannot read
/// `Duration::from_secs(6)`. A knob the registry cannot see is a knob
/// `DECISIONS.md` cannot be authoritative about, which is the whole reason
/// that gate exists.
pub const DEFAULT_TIMEOUT_SECS: u64 = 6;
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(DEFAULT_TIMEOUT_SECS);

/// Default gap between roster sweeps. **This number is the whole security
/// property**: it is how long a sold copy can keep playing, and it is a
/// posted knob rather than a hole. 120 s against a 100-player shard is one
/// request every two minutes to one host.
pub const DEFAULT_SWEEP_SECS: u64 = 120;
pub const DEFAULT_SWEEP: Duration = Duration::from_secs(DEFAULT_SWEEP_SECS);

/// What the far end said about one wallet.
///
/// The variants are ordered by how much they license a server to do, and
/// there is no `From<bool>`: constructing one has to be a decision about
/// which of THREE cases you saw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// `entitled: true` — a token in the wallet, or the title has no ticket
    /// contract at all and every wallet owns it. Both are "let them play",
    /// and they are one variant because no caller may treat them apart.
    Owns,
    /// `entitled: false` — a definite on-chain zero. **The only value that
    /// may cost a player their session.**
    Nope,
    /// `entitled: null`, a transport failure, a malformed body, a wallet the
    /// far end did not answer about, or a timeout. *We could not look.*
    Unknown,
}

impl Verdict {
    /// The join gate. `Unknown` admits — see the table in the module header.
    pub fn admits(self) -> bool {
        !matches!(self, Verdict::Nope)
    }

    /// The sweep gate, and the same predicate said the other way round so
    /// that a reader of the call site cannot mistake which way it points.
    /// A kick is a definite zero and nothing else.
    pub fn kicks(self) -> bool {
        matches!(self, Verdict::Nope)
    }
}

/// Where to ask, and how often. Absent from `shard.toml` ⇒ [`Config::off`],
/// and a shard that names no ticket authority checks nothing — the same
/// honest default `status_addr` and `require_auth` take.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    /// The origin, no trailing slash: `https://elopros.com`. `None`
    /// disables every check in this module.
    pub origin: Option<String>,
    /// The title's slug in scry's catalog. Ours, and it is not a knob a
    /// deployment should be inventing — but it is configurable because a
    /// staging origin may file this game under a different name.
    pub slug: String,
    pub timeout: Duration,
    pub sweep: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self::off()
    }
}

impl Config {
    /// Checks nothing and says so. Every read below short-circuits to
    /// [`Verdict::Owns`] — **not** `Unknown`: an unconfigured shard is not
    /// failing to look, it has been told there is nothing to look for, and a
    /// counter full of "could not look" on a dev box would be noise that
    /// hides the real thing.
    pub fn off() -> Self {
        Self {
            origin: None,
            slug: crate::ENTITLE_SLUG.to_string(),
            timeout: DEFAULT_TIMEOUT,
            sweep: DEFAULT_SWEEP,
        }
    }

    pub fn armed(&self) -> bool {
        self.origin.is_some()
    }

    /// A timeout that outlasts its own interval is a shard that polls itself
    /// in a loop the first time the origin is slow. Refused at config load
    /// rather than clamped, because clamping a number an operator typed is
    /// how a shard runs on a cadence nobody chose.
    pub fn sane(&self) -> Result<(), String> {
        if !self.armed() {
            return Ok(());
        }
        if self.timeout >= self.sweep {
            return Err(format!(
                "entitle_timeout_secs ({}) must be shorter than entitle_sweep_secs ({}) — \
                 a round has to finish before the next is due, or a slow origin stacks them",
                self.timeout.as_secs(),
                self.sweep.as_secs()
            ));
        }
        if self.slug.is_empty() {
            return Err("entitle_slug is empty — it names this title in scry's catalog".into());
        }
        Ok(())
    }

    fn url_of(&self, wallet: &str) -> Option<String> {
        Some(format!(
            "{}/api/ticket/{}/of/{}",
            self.origin.as_ref()?,
            self.slug,
            wallet
        ))
    }

    fn url_check(&self) -> Option<String> {
        Some(format!(
            "{}/api/ticket/{}/check",
            self.origin.as_ref()?,
            self.slug
        ))
    }
}

/// A wallet as the wire wants it: `0x` + 40 lowercase hex.
///
/// Checked here rather than trusted because this string goes into a URL
/// path. Everything reaching this module comes from `auth::key_of`, which
/// builds it from a recovered address — but "the only caller is careful" is
/// not a property a URL builder should rest on, and the far end refuses a
/// malformed wallet with a 422 that we would then have to read as `Unknown`.
pub fn is_wallet(s: &str) -> bool {
    s.len() == 42
        && s.starts_with("0x")
        && s[2..]
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

// ---------------------------------------------------------------------------
// The two calls
// ---------------------------------------------------------------------------

/// One wallet, at join. Blocking — the caller is a handshake task that is
/// waiting on the network anyway (`net.rs` runs it through
/// `spawn_blocking`), and never a frame or a tick.
pub fn check_one(cfg: &Config, wallet: &str) -> Verdict {
    if !cfg.armed() {
        return Verdict::Owns;
    }
    if !is_wallet(wallet) {
        // Not a transport failure and not a chain answer: a caller handed us
        // something that is not an address. `Unknown` rather than `Nope`,
        // because refusing a player over OUR bug is the failure this module
        // is shaped to avoid.
        return Verdict::Unknown;
    }
    let Some(url) = cfg.url_of(wallet) else {
        return Verdict::Owns;
    };
    let Some(body) = get(&url, cfg.timeout) else {
        return Verdict::Unknown;
    };
    entitled_field(&body)
}

/// The whole roster in one call. Returns a verdict **per input wallet, in
/// input order**, so a caller can zip it against its own slot list without
/// re-matching strings — and a wallet the far end did not mention comes back
/// `Unknown` rather than being dropped, because a missing row is exactly the
/// "we could not look" case wearing a different hat.
///
/// A roster longer than [`CHECK_MAX_WALLETS`] is refused as a page rather
/// than silently trimmed: every wallet in it gets `Unknown`, which keeps
/// everybody playing and shows up in the counters.
pub fn check_many(cfg: &Config, wallets: &[String]) -> Vec<Verdict> {
    if !cfg.armed() {
        return vec![Verdict::Owns; wallets.len()];
    }
    if wallets.is_empty() || wallets.len() > CHECK_MAX_WALLETS {
        return vec![Verdict::Unknown; wallets.len()];
    }
    let Some(url) = cfg.url_check() else {
        return vec![Verdict::Owns; wallets.len()];
    };

    let mut payload = String::from("{\"wallets\":[");
    for (i, w) in wallets.iter().enumerate() {
        if !is_wallet(w) {
            return vec![Verdict::Unknown; wallets.len()];
        }
        if i > 0 {
            payload.push(',');
        }
        payload.push('"');
        payload.push_str(w);
        payload.push('"');
    }
    payload.push_str("]}");

    let Some(body) = post(&url, &payload, cfg.timeout) else {
        return vec![Verdict::Unknown; wallets.len()];
    };
    verdicts_from_page(&body, wallets)
}

// ---------------------------------------------------------------------------
// Parsing — deliberately small, and it reads the ONE field that decides
// ---------------------------------------------------------------------------

/// `{"entitled": true|false|null, …}` → a verdict.
///
/// Hand-scanned rather than parsed into a document, and the reason is not
/// weight: the *only* thing this file may act on is one tri-state field, and
/// a scanner that can find nothing else cannot be talked into acting on
/// something else by a response that grew a key. Anything it does not
/// understand — a missing field, a number, a nested object — is `Unknown`,
/// which is the safe direction on both call sites.
fn entitled_field(body: &str) -> Verdict {
    match json_value_after(body, "\"entitled\"") {
        Some(Tri::True) => Verdict::Owns,
        Some(Tri::False) => Verdict::Nope,
        _ => Verdict::Unknown,
    }
}

#[derive(PartialEq, Eq, Clone, Copy)]
enum Tri {
    True,
    False,
    Null,
}

/// The literal following `"key":` at `from`, or `None` if it is not one of
/// the three literals this module recognises.
fn json_value_after(body: &str, key: &str) -> Option<Tri> {
    let at = body.find(key)? + key.len();
    let rest = body[at..].trim_start();
    let rest = rest.strip_prefix(':')?.trim_start();
    if rest.starts_with("true") {
        Some(Tri::True)
    } else if rest.starts_with("false") {
        Some(Tri::False)
    } else if rest.starts_with("null") {
        Some(Tri::Null)
    } else {
        None
    }
}

/// A `/check` page → one verdict per input wallet, in input order.
///
/// The free-title shortcut is read first and by name: a response carrying
/// `every_wallet_entitled: true` has no `wallets` map at all, and reading it
/// as "nobody was answered about" would kick nobody but would fill the
/// counters with a failure that is not happening.
fn verdicts_from_page(body: &str, wallets: &[String]) -> Vec<Verdict> {
    if json_value_after(body, "\"every_wallet_entitled\"") == Some(Tri::True) {
        return vec![Verdict::Owns; wallets.len()];
    }
    let Some(map_at) = body.find("\"wallets\"") else {
        return vec![Verdict::Unknown; wallets.len()];
    };
    let map = &body[map_at..];
    wallets
        .iter()
        .map(|w| {
            // The far end lowercases the keys it answers about
            // (`out[w.lower()]`), and ours are already lowercase — but the
            // needle is built from the wallet rather than assumed, so a
            // future mixed-case caller degrades to `Unknown` instead of
            // silently matching the wrong row.
            let needle = format!("\"{}\"", w.to_ascii_lowercase());
            match map.find(&needle) {
                Some(at) => match json_value_after(&map[at..], "\"entitled\"") {
                    Some(Tri::True) => Verdict::Owns,
                    Some(Tri::False) => Verdict::Nope,
                    _ => Verdict::Unknown,
                },
                None => Verdict::Unknown,
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Transport
// ---------------------------------------------------------------------------
//
// `ureq` with rustls, which the client already builds for the shard list —
// one trust store the build controls, on a box that may have no OpenSSL, and
// no new crate in the tree. Blocking on purpose and never on a hot path.
//
// A non-2xx is `None` and therefore `Unknown` at both call sites, and that
// covers the 503 scry answers when its own chain read fails — the response
// body for that case says `entitled: null` anyway, so the two roads to
// "could not look" meet.

fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .build()
        .into()
}

fn get(url: &str, timeout: Duration) -> Option<String> {
    let mut res = agent(timeout).get(url).call().ok()?;
    read_capped(&mut res)
}

fn post(url: &str, body: &str, timeout: Duration) -> Option<String> {
    let mut res = agent(timeout)
        .post(url)
        .content_type("application/json")
        .send(body)
        .ok()?;
    read_capped(&mut res)
}

fn read_capped(res: &mut ureq::http::Response<ureq::Body>) -> Option<String> {
    res.body_mut()
        .with_config()
        .limit((MAX_RESPONSE_BYTES + 1) as u64)
        .read_to_string()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "0x00000000000000000000000000000000000000a1";
    const B: &str = "0x00000000000000000000000000000000000000b2";

    fn armed() -> Config {
        Config {
            origin: Some("https://origin.test".into()),
            ..Config::off()
        }
    }

    // ── the one rule this module exists to keep ─────────────────────────────

    #[test]
    fn only_a_definite_zero_kicks() {
        assert!(Verdict::Nope.kicks());
        assert!(!Verdict::Unknown.kicks(), "a failed read must never kick");
        assert!(!Verdict::Owns.kicks());
    }

    #[test]
    fn only_a_definite_zero_is_refused_at_join() {
        assert!(!Verdict::Nope.admits());
        assert!(Verdict::Unknown.admits(), "a failed read must never refuse");
        assert!(Verdict::Owns.admits());
    }

    // ── one wallet ──────────────────────────────────────────────────────────

    #[test]
    fn a_true_owns_and_a_false_does_not() {
        assert_eq!(
            entitled_field(r#"{"entitled":true,"copies":1}"#),
            Verdict::Owns
        );
        assert_eq!(
            entitled_field(r#"{"entitled":false,"copies":0}"#),
            Verdict::Nope
        );
    }

    #[test]
    fn a_null_is_could_not_look_and_not_a_zero() {
        // scry's 503 body, verbatim in shape. The bug this pins is one
        // character wide: `entitled == false` in a language where null is
        // falsey would kick every player on the shard during an RPC blip.
        let body =
            r#"{"game":"gates","entitled":null,"reachable":false,"why":"the chain read failed"}"#;
        assert_eq!(entitled_field(body), Verdict::Unknown);
    }

    #[test]
    fn a_free_title_entitles_everyone() {
        let body =
            r#"{"game":"gates","ticketed":false,"entitled":true,"why":"no ticket contract"}"#;
        assert_eq!(entitled_field(body), Verdict::Owns);
    }

    #[test]
    fn nonsense_is_unknown_rather_than_anything() {
        for body in [
            "",
            "not json",
            r#"{"error":"wallet must be a 0x… address"}"#,
            r#"{"entitled":1}"#,
            r#"{"entitled":"true"}"#,
            r#"{"entitledish":true}"#,
        ] {
            assert_eq!(entitled_field(body), Verdict::Unknown, "body {body:?}");
        }
    }

    // ── the roster page ─────────────────────────────────────────────────────

    #[test]
    fn a_page_answers_in_input_order() {
        let body = format!(
            r#"{{"game":"gates","wallets":{{"{B}":{{"entitled":false}},"{A}":{{"entitled":true}}}}}}"#
        );
        assert_eq!(
            verdicts_from_page(&body, &[A.to_string(), B.to_string()]),
            vec![Verdict::Owns, Verdict::Nope],
            "the map is keyed, not ordered — the answer must follow the INPUT"
        );
    }

    #[test]
    fn a_wallet_the_page_skipped_is_unknown_not_absent() {
        let body = format!(r#"{{"wallets":{{"{A}":{{"entitled":true}}}}}}"#);
        let got = verdicts_from_page(&body, &[A.to_string(), B.to_string()]);
        assert_eq!(got.len(), 2, "every input wallet gets a verdict");
        assert_eq!(got[1], Verdict::Unknown, "unmentioned is 'could not look'");
    }

    #[test]
    fn a_null_row_in_a_page_does_not_kick() {
        let body = format!(r#"{{"wallets":{{"{A}":{{"entitled":null}}}}}}"#);
        assert_eq!(
            verdicts_from_page(&body, &[A.to_string()]),
            vec![Verdict::Unknown]
        );
    }

    #[test]
    fn a_free_title_page_entitles_the_whole_roster() {
        let body = r#"{"game":"gates","ticketed":false,"every_wallet_entitled":true}"#;
        assert_eq!(
            verdicts_from_page(body, &[A.to_string(), B.to_string()]),
            vec![Verdict::Owns; 2]
        );
    }

    #[test]
    fn a_body_with_no_map_kicks_nobody() {
        assert_eq!(
            verdicts_from_page("{}", &[A.to_string(), B.to_string()]),
            vec![Verdict::Unknown; 2]
        );
    }

    // ── bounds and refusals, before any socket is opened ────────────────────

    #[test]
    fn an_unconfigured_shard_checks_nothing_and_admits() {
        let off = Config::off();
        assert!(!off.armed());
        assert_eq!(check_one(&off, A), Verdict::Owns);
        assert_eq!(check_many(&off, &[A.to_string()]), vec![Verdict::Owns]);
    }

    #[test]
    fn a_malformed_wallet_never_reaches_the_url() {
        // No network in this test: `is_wallet` refuses first, and the proof
        // is that an armed config with an unroutable origin still answers.
        for bad in [
            "",
            "0x",
            "not-an-address",
            "0xZZZZ00000000000000000000000000000000000a",
            "0x00000000000000000000000000000000000000A1", // upper case
            "00000000000000000000000000000000000000a1",   // no prefix
        ] {
            assert!(!is_wallet(bad), "{bad:?} must not pass the wallet check");
            assert_eq!(check_one(&armed(), bad), Verdict::Unknown);
        }
        assert!(is_wallet(A));
    }

    #[test]
    fn a_roster_over_the_page_cap_is_refused_rather_than_trimmed() {
        let big: Vec<String> = (0..CHECK_MAX_WALLETS + 1).map(|_| A.to_string()).collect();
        let got = check_many(&armed(), &big);
        assert_eq!(got.len(), big.len(), "every wallet still gets a verdict");
        assert!(
            got.iter().all(|v| *v == Verdict::Unknown),
            "a page we refused to send is 'could not look' — and kicks nobody"
        );
    }

    #[test]
    fn an_empty_roster_asks_nothing() {
        assert_eq!(check_many(&armed(), &[]), Vec::<Verdict>::new());
    }

    #[test]
    fn a_timeout_that_outlasts_its_interval_is_refused_at_load() {
        let mut cfg = armed();
        cfg.timeout = Duration::from_secs(30);
        cfg.sweep = Duration::from_secs(10);
        assert!(cfg.sane().is_err());

        cfg.sweep = Duration::from_secs(120);
        assert!(cfg.sane().is_ok());

        // An unarmed shard is never wrong about a cadence it does not run.
        let mut off = Config::off();
        off.timeout = Duration::from_secs(999);
        assert!(off.sane().is_ok());
    }

    #[test]
    fn the_urls_are_the_routes_scry_actually_serves() {
        let cfg = armed();
        assert_eq!(
            cfg.url_of(A).unwrap(),
            format!("https://origin.test/api/ticket/gates/of/{A}")
        );
        assert_eq!(
            cfg.url_check().unwrap(),
            "https://origin.test/api/ticket/gates/check"
        );
    }
}
