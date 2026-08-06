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
