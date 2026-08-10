//! **The TLS posture is decided from an argument, so the gate has to be on
//! the argument.** Code tier: no `render` feature, no GPU, no socket.
//!
//! `client::client_endpoint(server, pin)` picks *anything*, *a pinned digest*
//! or *the platform root store* by asking `is_loopback_host(server)`, and
//! `crates/server/tests/tls_posture.rs` proves all three postures come out
//! right for the address it is handed. What that gate cannot see is **which
//! address it is handed.** Point any call site at a loopback constant instead
//! of the host actually being dialled and the client validates nothing on
//! every connection it makes — with `tls_posture` still 4/4 green, because
//! the function it tests still behaves perfectly on the string it is given.
//!
//! The carve-out is only sound under one condition: *the address the endpoint
//! is asked about is the address the session connects to.* That condition is
//! a property of call sites, not of values, so a source scan is the right
//! instrument — the same reasoning, and the same shape, as
//! `tests/sound.rs`'s `only_the_feed_drain_pops_the_core`, and the reason
//! `CLAUDE.md` says the gate for a call-site defect is a grep for the call
//! site.
//!
//! `Session::connect(endpoint, server, address, sign)` builds `https://{server}`
//! and dials it, so the identifier at `client_endpoint`'s argument 0 must be
//! the identifier at `connect`'s argument 1. Nothing subtler than identity is
//! checked and nothing subtler should be: two *different* expressions that
//! happen to evaluate equal today is exactly the state this refuses.

use std::path::Path;

/// Every place in the client that builds an endpoint and then dials with it.
/// Three, across the two player binaries — `client` (`main.rs`), and the
/// `gates` that `ci/depot.py` ships, which dials from two places: `--server`
/// at boot (`bin/gates.rs`) and the shard clicked out of the menu's list
/// (`render/menu.rs`).
const SITES: [&str; 3] = ["src/main.rs", "src/bin/gates.rs", "src/render/menu.rs"];

const ENDPOINT: &str = "client_endpoint(";
const CONNECT: &str = "Session::connect(";

/// The file with ordinary and doc comments removed. Comments may name these
/// calls freely — every one of them is *about* this rule — and the rule is
/// about code. Line-oriented, like `sound.rs`'s scan, which is also what
/// keeps a call spread over several lines readable to `args_of`.
fn code_of(path: &Path) -> String {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    text.lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// How many times `needle` is *called* in `code` — a definition
/// (`fn client_endpoint(`) is not a call, which is what keeps `lib.rs` out of
/// the census below without naming it as an exception.
fn call_count(code: &str, needle: &str) -> usize {
    code.match_indices(needle)
        .filter(|(i, _)| !code[..*i].trim_end().ends_with("fn"))
        .count()
}

/// The argument list of the first call to `needle`, split at top-level commas
/// and reduced to the bare identifier each argument names. `&server` and
/// `server` are the same identifier; `"127.0.0.1:4433"` is not.
fn args_of(code: &str, needle: &str) -> Vec<String> {
    let open = code.find(needle).expect("caller checked the count") + needle.len();
    let mut depth = 0usize;
    let mut args = vec![String::new()];
    for c in code[open..].chars() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' if depth == 0 => break,
            ')' | ']' => depth -= 1,
            ',' if depth == 0 => {
                args.push(String::new());
                continue;
            }
            _ => {}
        }
        args.last_mut().expect("never empty").push(c);
    }
    args.into_iter()
        .map(|a| {
            a.split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .trim_start_matches('&')
                .trim_start_matches("mut ")
                .trim()
                .to_string()
        })
        .collect()
}

/// **Every endpoint is asked about the host its own session then dials.**
///
/// Red under the one-line mutation it exists to defend against: replace the
/// address at any of the three `client_endpoint` calls with a loopback
/// literal — or with any second variable holding one — and this names the
/// site, while `tls_posture` and the rest of the client suite stay green.
#[test]
fn every_dialling_site_asks_about_the_address_it_dials() {
    let mut checked = 0;
    for site in SITES {
        let path = Path::new(site);
        assert!(
            path.is_file(),
            "{site} is gone - this gate names it by path"
        );
        let code = code_of(path);

        // Exactly one of each. A second dial in the same file is not
        // necessarily wrong, but it is unwatched by the pairing below, so it
        // is red until whoever adds it teaches this test to see it.
        assert_eq!(
            call_count(&code, ENDPOINT),
            1,
            "{site} has {} calls to {ENDPOINT} - this gate pairs the first one only",
            call_count(&code, ENDPOINT)
        );
        assert_eq!(
            call_count(&code, CONNECT),
            1,
            "{site} has {} calls to {CONNECT} - this gate pairs the first one only",
            call_count(&code, CONNECT)
        );

        let endpoint_args = args_of(&code, ENDPOINT);
        let connect_args = args_of(&code, CONNECT);
        assert!(
            !endpoint_args.is_empty(),
            "{site}: could not read the arguments of {ENDPOINT}"
        );
        assert!(
            connect_args.len() >= 2,
            "{site}: {CONNECT} has {} arguments - expected (endpoint, server, address, sign)",
            connect_args.len()
        );

        assert_eq!(
            endpoint_args[0], connect_args[1],
            "{site} builds its endpoint for `{}` and then connects to `{}`. \
             The loopback carve-out in `client_endpoint` is only safe while those \
             are the same address: asked about loopback, it validates NOTHING, \
             and SIWE has no channel binding to notice the relay that is answering.",
            endpoint_args[0], connect_args[1]
        );
        checked += 1;
    }
    assert_eq!(checked, SITES.len(), "not every listed site was checked");
}

/// **And `SITES` is all of them.** Without this the pairing above is a lint
/// that a fourth, unlisted call site walks straight past — the whole defect
/// class re-opened by adding a file rather than by editing one.
#[test]
fn the_listed_sites_are_every_caller_in_the_client() {
    fn walk(dir: &Path, out: &mut Vec<String>) {
        for e in std::fs::read_dir(dir).expect("src must exist") {
            let p = e.expect("readable entry").path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|x| x == "rs")
                && call_count(&code_of(&p), ENDPOINT) > 0
            {
                out.push(p.display().to_string());
            }
        }
    }

    let mut callers = Vec::new();
    walk(Path::new("src"), &mut callers);
    callers.sort();
    let mut expected: Vec<String> = SITES.iter().map(|s| (*s).to_string()).collect();
    expected.sort();
    assert_eq!(
        callers, expected,
        "the set of `client_endpoint` callers moved. Every caller has to pair its \
         address with the one it dials, so a new one joins `SITES` in the same commit."
    );

    // The definition is still where the census assumes it is - otherwise a
    // rename empties the walk above and both tests pass by watching nothing.
    let lib = code_of(Path::new("src/lib.rs"));
    assert!(
        lib.contains("fn client_endpoint("),
        "src/lib.rs no longer defines client_endpoint - this gate is watching a ghost"
    );
    assert_eq!(
        call_count(&lib, ENDPOINT),
        0,
        "src/lib.rs now calls client_endpoint as well as defining it"
    );
}
