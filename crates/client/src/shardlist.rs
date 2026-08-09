//! `scry-shardlist-v1` — the document a title serves so its shards can be
//! found, and the one the scry launcher's Servers window renders.
//!
//! **The shard list is the game's to serve.** That is scry's rule, not ours
//! (`docs/LAUNCHER.md` §6 in `AnthonE/scry`): the launcher reads
//! `manifest.servers.url`, fetches it, and renders the rows — it "does not
//! invent, cache or rank them", and its broker refuses to proxy the fetch
//! (`fetch it yourself; the launcher does not proxy it`). So this shape is a
//! contract with a consumer that already exists and is already dark waiting
//! for it. The window's own empty-state names this game by name.
//!
//! The shape, verbatim from that section:
//!
//! ```json
//! {"servers": [{"id": "eu-1", "name": "…", "addr": "host:port",
//!               "players": 47, "max_players": 100, "map": "…",
//!               "ping_ms": 31}]}
//! ```
//!
//! Two things about it are load-bearing here and neither is obvious:
//!
//! - **`addr` is `host:port`, not an address.** The public shard's cert is
//!   for `game.moreright.xyz` and a WebTransport client needs that name for
//!   SNI, so a row carrying `65.108.x.x:61234` would connect and then be
//!   unable to say who it is talking to. `wtransport` resolves the name
//!   itself and uses it as the server name, which is why `Shard::url` hands
//!   it the domain rather than resolving first.
//! - **Every count is optional and none of them is invented.** A row whose
//!   shard has no way to report a live player count leaves `players` out,
//!   and the launcher already draws `?` for an absent one. Writing a zero
//!   there — or worse, a plausible number — would be the "card advertising a
//!   reward the rule cannot pay" defect that both repos' `CLAUDE.md` warn
//!   about, in its smallest form.
//!
//! ## `status_url`, and why the count is not baked into the row
//!
//! A row may carry `status_url`, naming where *that shard* answers
//! `GET /status.json` (`crates/server/src/status.rs`). It is an additive,
//! optional field — the kind stays `scry-shardlist-v1` and every reader that
//! predates it ignores it — and it exists because the alternative is worse.
//!
//! **A baked count is stale the moment the file is written.** `ci/shardlist.py`
//! generates this document and an operator copies it to an origin; a `players`
//! written at generation time describes a shard as it was whenever someone
//! last ran the script, which on a served file is hours or weeks. A count that
//! is confidently wrong is worse than the `?` it replaced — a busy shard
//! reading as empty is the exact failure the paragraph above refuses, arriving
//! by a slower road. So the generator stays a pure generator, the row names
//! where the live number lives, and **each reader polls that endpoint itself.**
//!
//! That also keeps scry's rule intact rather than bending it: the launcher
//! "does not invent, cache or rank" the list and its broker refuses to proxy
//! the fetch, so a launcher reading a shard's own status endpoint directly is
//! the same posture one layer down. Nothing proxies; everyone measures.
//!
//! A `players` baked into the row is still honoured when present — a title
//! serving this document from something that *can* measure live is a shape
//! this parser must not refuse. `status_url` wins over it when both are there,
//! because a poll that just succeeded is newer than a field in a file.
//!
//! **Wall 4, and where its caps live.** This parses bytes off the network,
//! which is the most client-driven path in the client. The caps are here
//! rather than in `sim_core::limits` on purpose: `limits.rs` bounds the sim's
//! queues and per-tick work, and a shard list is neither — it never reaches
//! the sim thread. The policy is stated per cap below and is always *refuse
//! the document*, never *silently truncate it*: a list that quietly lost its
//! last eight shards is a list nobody can tell is wrong.

use serde::Deserialize;

/// Most shards one document may list. A refusal, not a truncation — see the
/// module docs. Sized well above any plausible list for this game so that
/// hitting it means the document is hostile or broken, not popular.
pub const MAX_SHARDS: usize = 64;

/// Longest `id`, `name` and `map` a row may carry, in bytes. The launcher
/// renders these into a fixed-width row and this client draws them into a
/// menu; a megabyte of name is a display bug at best.
pub const MAX_FIELD_BYTES: usize = 96;

/// Longest `status_url` a row may carry, in bytes. Its own cap because a url
/// is legitimately longer than a name — `MAX_FIELD_BYTES` would refuse an
/// honest one — and shorter than the document cap, because this string is
/// handed to a fetcher rather than drawn.
pub const MAX_URL_BYTES: usize = 256;

/// Largest `/status.json` document accepted from a shard, in bytes. The real
/// one is under 60; the cap is three orders of magnitude of headroom and
/// still bounds a shard that answers forever.
pub const MAX_STATUS_BYTES: usize = 4 * 1024;

/// Largest document accepted off the wire, in bytes. `MAX_SHARDS` rows of
/// `MAX_FIELD_BYTES` fields plus JSON overhead, rounded up — a cap on the
/// *read*, so a server that streams forever cannot hold the menu open.
pub const MAX_DOC_BYTES: usize = 64 * 1024;

/// The `kind` a manifest declares for this document. Carried so a future
/// `-v2` can be told apart from a truncated `-v1`.
pub const KIND: &str = "scry-shardlist-v1";

/// One row of the list — one shard a player may join.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Shard {
    /// Stable key for the row. The launcher builds its row key from this,
    /// falling back to `addr`.
    pub id: String,
    pub name: String,
    /// `host:port`. Validated for shape only; never resolved here, because
    /// resolving is I/O and this parser must stay testable offline.
    pub addr: String,
    #[serde(default)]
    pub players: Option<u32>,
    #[serde(default)]
    pub max_players: Option<u32>,
    #[serde(default)]
    pub map: Option<String>,
    #[serde(default)]
    pub ping_ms: Option<u32>,
    /// Where this shard answers `GET /status.json`. Optional; see the module
    /// docs for why the live count is polled rather than baked.
    #[serde(default)]
    pub status_url: Option<String>,
}

impl Shard {
    /// The WebTransport url for this row. Not a `SocketAddr`: the name is
    /// the TLS server name and `wtransport` wants it unresolved.
    pub fn url(&self) -> String {
        format!("https://{}", self.addr)
    }

    /// `47/100`, `47/?`, or `?` — whatever the row actually states. Never
    /// fills a missing count with a zero.
    pub fn population(&self) -> String {
        match (self.players, self.max_players) {
            (Some(p), Some(m)) => format!("{p}/{m}"),
            (Some(p), None) => format!("{p}"),
            (None, Some(m)) => format!("?/{m}"),
            (None, None) => "?".to_string(),
        }
    }

    /// Fold a freshly polled status into this row.
    ///
    /// **The poll wins over the file.** Both numbers describe the same shard
    /// and one of them was measured a second ago; see the module docs. This
    /// is the only way a count becomes live, and a *failed* poll never calls
    /// it — a row keeps whatever it had rather than being zeroed by a
    /// timeout, which would turn a brief network blip into "everyone left".
    pub fn apply_status(&mut self, s: &Status) {
        self.players = Some(s.players);
        self.max_players = Some(s.max_players);
    }
}

/// A shard's `GET /status.json`, as `crates/server/src/status.rs` writes it.
///
/// Integers only, which is that endpoint's own rule (`stats.rs` L5:
/// diagnostics are numbers, not strings). `tick` is carried because it is the
/// liveness half — a number that stops moving is a shard that stopped
/// ticking — and defaulted rather than required, so a shard that trims the
/// document to the two counts a list needs still parses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct Status {
    pub players: u32,
    pub max_players: u32,
    #[serde(default)]
    pub tick: u64,
}

/// Parse a shard's `/status.json`.
///
/// Same refuse-don't-guess policy as [`parse`], and the same reason: this is
/// bytes off the network, from a host named in a document fetched from
/// another host. A shard that answers junk must leave the row's count
/// *absent*, never zero — so every failure here is an `Err` the caller drops,
/// and none of them is a `Status` with plausible-looking fields.
pub fn parse_status(bytes: &[u8]) -> Result<Status, String> {
    if bytes.len() > MAX_STATUS_BYTES {
        return Err(format!(
            "status is {} bytes, over the {MAX_STATUS_BYTES}-byte cap",
            bytes.len()
        ));
    }
    let v: serde_json::Value = serde_json::from_slice(bytes).map_err(|e| format!("status: {e}"))?;
    if !v.is_object() {
        return Err("status: top level is not an object".into());
    }
    let s: Status = serde_json::from_value(v).map_err(|e| format!("status: {e}"))?;
    // A shard reporting more players than it can hold is broken or hostile,
    // and the row it would draw (`412/100`) is nonsense either way. Refuse it
    // rather than render it — the row keeps its previous, honest count.
    if s.max_players == 0 {
        return Err("status: max_players is 0".into());
    }
    if s.players > s.max_players {
        return Err(format!(
            "status: {} players over a cap of {}",
            s.players, s.max_players
        ));
    }
    Ok(s)
}

/// Validate a `status_url` for shape. `http(s)` only, capped.
///
/// Refused rather than carried, exactly as `args.rs` refuses a bad
/// `--servers`: a url the reader cannot fetch becomes a row that blames the
/// network for a typo in a file.
pub fn check_status_url(url: &str) -> Result<(), String> {
    let u = url.trim();
    if u.is_empty() {
        return Err("status_url is empty".into());
    }
    if u.len() > MAX_URL_BYTES {
        return Err(format!(
            "status_url is {} bytes, over the {MAX_URL_BYTES}-byte cap",
            u.len()
        ));
    }
    if u.chars().any(char::is_whitespace) {
        return Err(format!("status_url {u:?} contains whitespace"));
    }
    if !u.starts_with("https://") && !u.starts_with("http://") {
        return Err(format!("status_url {u:?} is not an http(s) url"));
    }
    // `https://` and nothing after it names no host.
    let rest = u.split_once("://").map(|(_, r)| r).unwrap_or("");
    if rest.is_empty() || rest.starts_with('/') {
        return Err(format!("status_url {u:?} has no host"));
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
struct Doc {
    #[serde(default)]
    servers: Vec<Shard>,
}

/// Parse a `scry-shardlist-v1` document.
///
/// Every refusal names what was wrong, because this string is drawn in the
/// menu where a player reads it — "the shard list did not parse" tells
/// nobody anything, and a dark panel that cannot say why is the one thing
/// both repos' dark-surface discipline forbids.
pub fn parse(bytes: &[u8]) -> Result<Vec<Shard>, String> {
    if bytes.len() > MAX_DOC_BYTES {
        return Err(format!(
            "shard list is {} bytes, over the {MAX_DOC_BYTES}-byte cap",
            bytes.len()
        ));
    }
    // The top level must be an OBJECT, checked before the struct is built.
    // serde will happily deserialize a struct from a JSON *array* in
    // positional form, so a bare `[]` reads as `{"servers": []}` — which is
    // indistinguishable from a healthy shard list with nothing up. Caught by
    // this module's own junk test; a wrong document must never be able to
    // present as an honest empty one.
    let v: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|e| format!("shard list: {e}"))?;
    if !v.is_object() {
        return Err(format!(
            "shard list: top level is {}, expected an object with a \"servers\" key",
            match v {
                serde_json::Value::Array(_) => "an array",
                serde_json::Value::Null => "null",
                serde_json::Value::Bool(_) => "a bool",
                serde_json::Value::Number(_) => "a number",
                serde_json::Value::String(_) => "a string",
                serde_json::Value::Object(_) => unreachable!(),
            }
        ));
    }
    let doc: Doc = serde_json::from_value(v).map_err(|e| format!("shard list: {e}"))?;

    if doc.servers.len() > MAX_SHARDS {
        return Err(format!(
            "shard list has {} rows, over the {MAX_SHARDS} cap",
            doc.servers.len()
        ));
    }

    for (n, s) in doc.servers.iter().enumerate() {
        let field = |what: &str, v: &str| -> Result<(), String> {
            if v.trim().is_empty() {
                return Err(format!("shard list row {n}: {what} is empty"));
            }
            if v.len() > MAX_FIELD_BYTES {
                return Err(format!(
                    "shard list row {n}: {what} is {} bytes, over the {MAX_FIELD_BYTES} cap",
                    v.len()
                ));
            }
            Ok(())
        };
        field("id", &s.id)?;
        field("name", &s.name)?;
        if let Some(m) = &s.map {
            field("map", m)?;
        }
        check_addr(&s.addr).map_err(|why| format!("shard list row {n} ({}): {why}", s.id))?;
        if let Some(u) = &s.status_url {
            check_status_url(u).map_err(|why| format!("shard list row {n} ({}): {why}", s.id))?;
        }
    }
    Ok(doc.servers)
}

/// Validate a `host:port` for shape, without resolving it.
///
/// Deliberately NOT `SocketAddr::from_str`. That is what the client used to
/// do for `--server`, and it refuses every hostname — including the one the
/// public shard's certificate is actually issued for. A name is the normal
/// case here, not the exception.
pub fn check_addr(addr: &str) -> Result<(), String> {
    let addr = addr.trim();
    if addr.is_empty() {
        return Err("address is empty".into());
    }
    if addr.len() > MAX_FIELD_BYTES {
        return Err(format!("address is over the {MAX_FIELD_BYTES}-byte cap"));
    }
    if addr.contains("://") || addr.contains('/') {
        return Err(format!("{addr:?} is a url, not a host:port"));
    }
    if addr.chars().any(char::is_whitespace) {
        return Err(format!("{addr:?} contains whitespace"));
    }

    // An IPv6 literal is bracketed; anything else splits on the LAST colon,
    // so a bare `::1` is refused rather than read as host `:` port `1`.
    let (host, port) = if let Some(rest) = addr.strip_prefix('[') {
        let (h, r) = rest
            .split_once(']')
            .ok_or_else(|| format!("{addr:?} opens a bracket it never closes"))?;
        let p = r
            .strip_prefix(':')
            .ok_or_else(|| format!("{addr:?} has no port"))?;
        (h, p)
    } else {
        let (h, p) = addr
            .rsplit_once(':')
            .ok_or_else(|| format!("{addr:?} has no port — expected host:port"))?;
        if h.contains(':') {
            return Err(format!(
                "{addr:?} looks like a bare IPv6 address; bracket it as [{h}]:{p}"
            ));
        }
        (h, p)
    };

    if host.is_empty() {
        return Err(format!("{addr:?} has no host"));
    }
    match port.parse::<u16>() {
        Ok(0) => Err(format!("{addr:?} has port 0")),
        Ok(_) => Ok(()),
        Err(_) => Err(format!("{addr:?} has a bad port {port:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape as `docs/LAUNCHER.md` §6 writes it, verbatim. If scry
    /// changes the document, this is the test that should go red first.
    const SPEC: &str = r#"{"servers": [{"id": "eu-1", "name": "Gates EU 1",
        "addr": "game.moreright.xyz:61234", "players": 47, "max_players": 100,
        "map": "island", "ping_ms": 31}]}"#;

    /// What `ci/shardlist.py` actually writes, verbatim, for a two-shard
    /// `shards.toml` — one publishing a status endpoint and one not.
    ///
    /// **The generator and this parser are in two languages and nothing else
    /// compares them on a whole document.** `--self-test` reads this file's
    /// CAPS out of the Rust source, which catches a cap that drifts and would
    /// not catch a field that was renamed, reordered or dropped. Regenerate:
    ///
    /// ```text
    /// ./ci/shardlist.py --shards shards.toml --out -
    /// ```
    const GENERATED: &str = r#"{
  "servers": [
    {
      "addr": "game.moreright.xyz:61234",
      "id": "eu-1",
      "map": "island 20260731",
      "max_players": 100,
      "name": "Gates EU 1",
      "status_url": "https://game.moreright.xyz:8080/status.json"
    },
    {
      "addr": "127.0.0.1:4433",
      "id": "dev",
      "max_players": 100,
      "name": "Dev shard"
    }
  ]
}"#;

    #[test]
    fn what_the_generator_writes_is_what_this_reads() {
        let rows = parse(GENERATED.as_bytes()).expect("our own generator's output");
        assert_eq!(rows.len(), 2);

        // The pair that is the whole design: the generator says WHERE the
        // count lives and never what it would have found.
        assert_eq!(rows[0].id, "eu-1");
        assert_eq!(rows[0].players, None, "the generator must not bake a count");
        assert_eq!(rows[0].population(), "?/100");
        assert_eq!(
            rows[0].status_url.as_deref(),
            Some("https://game.moreright.xyz:8080/status.json")
        );
        // The name survives to the transport unresolved — the SNI property.
        assert_eq!(rows[0].url(), "https://game.moreright.xyz:61234");

        // A shard with no endpoint stays `?`, which is correct and permanent.
        assert_eq!(rows[1].status_url, None);
        assert_eq!(rows[1].population(), "?/100");
    }

    #[test]
    fn the_documented_shape_parses() {
        let rows = parse(SPEC.as_bytes()).expect("spec shape");
        assert_eq!(rows.len(), 1);
        let s = &rows[0];
        assert_eq!(s.id, "eu-1");
        assert_eq!(s.addr, "game.moreright.xyz:61234");
        assert_eq!(s.players, Some(47));
        assert_eq!(s.population(), "47/100");
        // The name survives to the transport unresolved — this is the SNI
        // property the module docs explain, pinned.
        assert_eq!(s.url(), "https://game.moreright.xyz:61234");
    }

    #[test]
    fn a_row_may_state_no_counts_at_all() {
        // The honest shape for a shard with no status endpoint. `players`
        // absent must never render as 0 — the launcher draws `?` and so do
        // we, and this pins that a missing count stays missing.
        let rows = parse(br#"{"servers":[{"id":"a","name":"A","addr":"host:1"}]}"#)
            .expect("countless row");
        assert_eq!(rows[0].players, None);
        assert_eq!(rows[0].max_players, None);
        assert_eq!(rows[0].population(), "?");
        assert_eq!(rows[0].map, None);
    }

    #[test]
    fn an_empty_list_is_a_list_and_not_an_error() {
        // "no shards up" is a state the launcher already renders. It is not
        // a parse failure and must not be reported as one.
        assert_eq!(parse(br#"{"servers":[]}"#).unwrap().len(), 0);
        assert_eq!(parse(br#"{}"#).unwrap().len(), 0);
    }

    #[test]
    fn the_caps_refuse_rather_than_truncate() {
        let rows: Vec<String> = (0..MAX_SHARDS + 1)
            .map(|i| format!(r#"{{"id":"s{i}","name":"n","addr":"h:1"}}"#))
            .collect();
        let doc = format!(r#"{{"servers":[{}]}}"#, rows.join(","));
        let why = parse(doc.as_bytes()).expect_err("over the row cap");
        assert!(why.contains("over the"), "{why}");
        // The failure must NOT be a silent short list.
        assert!(why.contains(&(MAX_SHARDS + 1).to_string()), "{why}");

        let long = "x".repeat(MAX_FIELD_BYTES + 1);
        let doc = format!(r#"{{"servers":[{{"id":"a","name":"{long}","addr":"h:1"}}]}}"#);
        assert!(parse(doc.as_bytes()).is_err());

        let huge = vec![b' '; MAX_DOC_BYTES + 1];
        assert!(parse(&huge).is_err());
    }

    #[test]
    fn a_row_with_a_bad_address_is_refused_and_says_which() {
        let doc = br#"{"servers":[{"id":"eu-1","name":"n","addr":"nonsense"}]}"#;
        let why = parse(doc).expect_err("no port");
        // Naming the row is the point: this string is drawn in the menu.
        assert!(why.contains("eu-1"), "{why}");
        assert!(why.contains("host:port"), "{why}");
    }

    #[test]
    fn addresses_are_shapes_not_socket_addrs() {
        // The regression this whole module exists around: a hostname is the
        // NORMAL case, and `SocketAddr::from_str` refuses every one of them.
        check_addr("game.moreright.xyz:61234").expect("a hostname is an address");
        check_addr("127.0.0.1:4433").expect("so is a literal");
        check_addr("[::1]:4433").expect("so is a bracketed v6");

        for bad in [
            "",
            "host",
            "host:",
            "host:0",
            "host:99999",
            "host:port",
            ":4433",
            "https://host:4433",
            "host:4433/join",
            "host :4433",
            "::1:4433",
            "[::1:4433",
        ] {
            assert!(check_addr(bad).is_err(), "{bad:?} should be refused");
        }
    }

    #[test]
    fn a_bare_ipv6_names_its_own_fix() {
        // Refusing is not enough when the correct form is one bracket away.
        let why = check_addr("::1:4433").expect_err("bare v6");
        assert!(why.contains("[::1]:4433"), "{why}");
    }

    #[test]
    fn a_row_may_name_where_its_live_count_lives() {
        // The additive field. An old reader ignores it; this one carries it
        // so the menu can poll the shard rather than trust a baked number.
        let rows = parse(
            br#"{"servers":[{"id":"eu-1","name":"A","addr":"h:1",
                 "status_url":"https://h:8080/status.json"}]}"#,
        )
        .expect("status_url row");
        assert_eq!(
            rows[0].status_url.as_deref(),
            Some("https://h:8080/status.json")
        );
        // ...and it is still absent when nobody states one.
        let rows = parse(br#"{"servers":[{"id":"a","name":"A","addr":"h:1"}]}"#).unwrap();
        assert_eq!(rows[0].status_url, None);
    }

    #[test]
    fn a_bad_status_url_is_refused_and_names_its_row() {
        for bad in [
            "h:8080/status.json",     // no scheme
            "ftp://h/status.json",    // wrong scheme
            "https://",               // no host
            "https:///status.json",   // no host
            "https://h/ status.json", // whitespace
        ] {
            let doc = format!(
                r#"{{"servers":[{{"id":"eu-1","name":"n","addr":"h:1","status_url":"{bad}"}}]}}"#
            );
            let why = parse(doc.as_bytes()).expect_err(bad);
            assert!(why.contains("eu-1"), "{bad}: {why}");
        }
        let long = format!("https://h/{}", "x".repeat(MAX_URL_BYTES));
        assert!(check_status_url(&long).is_err());
    }

    #[test]
    fn a_polled_status_is_what_lights_the_count() {
        // The whole point of the field: `?` becomes `3/100` without the
        // document having claimed a number it could not know.
        let mut s =
            parse(br#"{"servers":[{"id":"a","name":"A","addr":"h:1"}]}"#).unwrap()[0].clone();
        assert_eq!(s.population(), "?");
        let st = parse_status(br#"{"players":3,"max_players":100,"tick":123456}"#)
            .expect("the shard's own shape");
        assert_eq!(st.players, 3);
        assert_eq!(st.tick, 123_456);
        s.apply_status(&st);
        assert_eq!(s.population(), "3/100");
    }

    #[test]
    fn a_poll_beats_a_baked_count() {
        // Both describe the same shard and one of them was measured a second
        // ago. Pinned because the opposite order reads as reasonable and is
        // the bug that would make the field pointless.
        let mut s = parse(
            br#"{"servers":[{"id":"a","name":"A","addr":"h:1","players":99,"max_players":100}]}"#,
        )
        .unwrap()[0]
            .clone();
        assert_eq!(s.population(), "99/100");
        s.apply_status(&parse_status(br#"{"players":4,"max_players":100}"#).unwrap());
        assert_eq!(s.population(), "4/100");
    }

    #[test]
    fn a_shard_answering_junk_leaves_the_count_alone() {
        // Every one of these must be an Err the caller drops, never a
        // `Status` full of zeroes — a busy shard reading as empty is the
        // defect this module's docs are mostly about.
        for junk in [
            &b""[..],
            b"not json",
            b"[]",
            b"null",
            b"{}",
            br#"{"players":1}"#,
            br#"{"players":-1,"max_players":100}"#,
            br#"{"players":"3","max_players":"100"}"#,
            br#"{"players":5,"max_players":0}"#,
            // More players than the shard can hold: broken or hostile, and
            // the row it would draw is nonsense either way.
            br#"{"players":412,"max_players":100}"#,
            b"\xff\xfe\x00",
        ] {
            assert!(parse_status(junk).is_err(), "{junk:?} should be refused");
        }
        let huge = vec![b' '; MAX_STATUS_BYTES + 1];
        assert!(parse_status(&huge).is_err());

        // A failed poll must not touch the row. This is the shape the caller
        // is obliged to keep and the reason `apply_status` takes a `Status`
        // rather than a `Result`: there is no way to spell "apply a failure".
        let mut s = parse(
            br#"{"servers":[{"id":"a","name":"A","addr":"h:1","players":7,"max_players":100}]}"#,
        )
        .unwrap()[0]
            .clone();
        if let Ok(st) = parse_status(b"garbage") {
            s.apply_status(&st);
        }
        assert_eq!(s.population(), "7/100", "a failed poll zeroed the row");
    }

    #[test]
    fn an_empty_shard_is_a_zero_and_not_a_missing_count() {
        // The other half of the honesty rule, and it is easy to get backwards:
        // a shard that ANSWERED "nobody is on" states `0/100`, which is a
        // measurement. Only an unmeasured count is `?`.
        let mut s =
            parse(br#"{"servers":[{"id":"a","name":"A","addr":"h:1"}]}"#).unwrap()[0].clone();
        s.apply_status(&parse_status(br#"{"players":0,"max_players":100}"#).unwrap());
        assert_eq!(s.population(), "0/100");
    }

    #[test]
    fn junk_is_refused_without_panicking() {
        // This is network input. Every one of these must be an Err, never a
        // panic and never an empty list that reads as "no shards up".
        for junk in [
            &b""[..],
            b"not json",
            b"[]",
            b"null",
            br#"{"servers":{}}"#,
            br#"{"servers":[{"id":"a"}]}"#,
            br#"{"servers":[{"id":"","name":"n","addr":"h:1"}]}"#,
            b"\xff\xfe\x00",
        ] {
            assert!(parse(junk).is_err(), "{:?} should be refused", junk);
        }
    }
}
