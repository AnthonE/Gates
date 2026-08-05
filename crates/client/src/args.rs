//! The client's command line — shared by the headless `client` binary and the
//! windowed `gates` one, so a flag can never mean two things.
//!
//! This exists because of the depot. A scry depot's launch block names an argv
//! with placeholders the launcher fills:
//!
//! ```json
//! "launch": { "exec": "gates",
//!             "args": ["--server", "{server}", "--identity", "{wallet}"] }
//! ```
//!
//! ...so the flags below are not a convenience, they are the interface the
//! launcher starts this game through. Two properties of that seam drive the
//! parsing and neither is obvious:
//!
//! - **An unset placeholder arrives as an empty string, not as a missing
//!   flag.** The launcher substitutes `{wallet}` with `""` when the player has
//!   no address set, so `--identity ""` is the *normal* shape of "playing
//!   anonymously" and must not be read as an address of zero length.
//! - **An unknown placeholder is refused by the launcher**, never passed
//!   through — so a literal `{token}` can never reach here, and this parser
//!   does not have to defend against one.
//!
//! `--help` matters for the same reason: before this module, `gates --help`
//! parsed the word "--help" as a socket address and exited with a stderr line
//! about syntax. A game started by a launcher is a game nobody types, which is
//! exactly when a broken argv goes unnoticed for a month.

use std::net::SocketAddr;

pub const DEFAULT_SERVER: &str = "127.0.0.1:4433";

pub const USAGE: &str = "\
gates — the Gates desktop client

  gates [ADDR] [options]

  ADDR                 shard address, host:port (default 127.0.0.1:4433)
  --server ADDR        the same, named. Wins over the positional form
  --identity ADDR      the wallet address to play as. UNVERIFIED — the shard
                       does not check it yet and this client does not claim it
                       does. Empty is the same as absent
  --no-launcher        do not look for a scry launcher, even if one is running
  --help               this

The scry launcher fills --server and --identity from a depot's launch block;
running without one is a normal, supported state.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args {
    pub server: SocketAddr,
    /// `None` when absent OR empty — see the module docs; the launcher's
    /// empty substitution is how "no wallet set" arrives.
    pub identity: Option<String>,
    pub no_launcher: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Parsed {
    Run(Args),
    /// `--help` was asked for. A separate arm rather than an exit inside the
    /// parser, so the parser stays testable and does not own the process.
    Help,
    Bad(String),
}

pub fn parse<I: IntoIterator<Item = String>>(argv: I) -> Parsed {
    let mut server_flag: Option<String> = None;
    let mut server_pos: Option<String> = None;
    let mut identity: Option<String> = None;
    let mut no_launcher = false;

    let mut it = argv.into_iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--help" | "-h" => return Parsed::Help,
            "--no-launcher" => no_launcher = true,
            "--server" => match it.next() {
                Some(v) => server_flag = Some(v),
                None => return Parsed::Bad("--server needs an address".into()),
            },
            "--identity" => match it.next() {
                Some(v) => identity = Some(v),
                None => return Parsed::Bad("--identity needs an address".into()),
            },
            other if other.starts_with('-') => {
                // Refused rather than ignored. A typo'd flag that is silently
                // dropped is a setting the player believes is on.
                return Parsed::Bad(format!("unknown option {other:?}"));
            }
            other => {
                if server_pos.is_some() {
                    return Parsed::Bad(format!("unexpected argument {other:?}"));
                }
                server_pos = Some(other.to_string());
            }
        }
    }

    // An EMPTY --server is the same case as an empty --identity: the launcher
    // substituted a placeholder it had no value for. Fall back rather than
    // failing to parse "" as an address.
    let raw = server_flag
        .filter(|s| !s.trim().is_empty())
        .or(server_pos.filter(|s| !s.trim().is_empty()))
        .unwrap_or_else(|| DEFAULT_SERVER.to_string());

    let server = match raw.parse::<SocketAddr>() {
        Ok(s) => s,
        Err(e) => return Parsed::Bad(format!("bad address {raw:?}: {e}")),
    };

    Parsed::Run(Args {
        server,
        identity: identity
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        no_launcher,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(v: &[&str]) -> Parsed {
        parse(v.iter().map(|s| s.to_string()))
    }
    fn run(v: &[&str]) -> Args {
        match a(v) {
            Parsed::Run(x) => x,
            other => panic!("expected Run, got {other:?}"),
        }
    }

    #[test]
    fn the_default_is_a_local_shard() {
        let x = run(&[]);
        assert_eq!(x.server.to_string(), DEFAULT_SERVER);
        assert_eq!(x.identity, None);
        assert!(!x.no_launcher);
    }

    #[test]
    fn the_positional_form_still_works() {
        assert_eq!(run(&["10.0.0.4:4433"]).server.to_string(), "10.0.0.4:4433");
    }

    #[test]
    fn the_launch_block_shape_parses() {
        // Verbatim the argv a depot's launch args produce once filled.
        let x = run(&["--server", "10.0.0.4:4433", "--identity", "0xAbC"]);
        assert_eq!(x.server.to_string(), "10.0.0.4:4433");
        assert_eq!(x.identity.as_deref(), Some("0xAbC"));
    }

    #[test]
    fn an_unfilled_placeholder_is_absence_not_an_empty_address() {
        // What `{wallet}` becomes when the player has no address set. This is
        // the normal anonymous launch and it must not read as an identity.
        let x = run(&["--server", "10.0.0.4:4433", "--identity", ""]);
        assert_eq!(x.identity, None);
        assert_eq!(x.server.to_string(), "10.0.0.4:4433");

        // ...and the same for the server, which falls back rather than failing.
        let x = run(&["--server", "", "--identity", "0x1"]);
        assert_eq!(x.server.to_string(), DEFAULT_SERVER);
        assert_eq!(x.identity.as_deref(), Some("0x1"));
    }

    #[test]
    fn the_named_form_beats_the_positional_one() {
        let x = run(&["1.2.3.4:1", "--server", "5.6.7.8:2"]);
        assert_eq!(x.server.to_string(), "5.6.7.8:2");
    }

    #[test]
    fn help_is_a_state_not_a_bad_address() {
        // The regression this module was written for: `gates --help` used to
        // parse "--help" as a socket address and die with a syntax error.
        assert_eq!(a(&["--help"]), Parsed::Help);
        assert_eq!(a(&["-h"]), Parsed::Help);
        assert_eq!(a(&["--server", "1.2.3.4:1", "--help"]), Parsed::Help);
    }

    #[test]
    fn a_typo_is_refused_rather_than_ignored() {
        assert!(matches!(a(&["--identtiy", "0x1"]), Parsed::Bad(_)));
        assert!(matches!(a(&["--server"]), Parsed::Bad(_)));
        assert!(matches!(a(&["--identity"]), Parsed::Bad(_)));
        assert!(matches!(a(&["1.2.3.4:1", "5.6.7.8:2"]), Parsed::Bad(_)));
        assert!(matches!(a(&["not-an-address"]), Parsed::Bad(_)));
    }

    #[test]
    fn every_flag_the_usage_names_is_a_flag_this_parses() {
        // A usage string is documentation and drifts like one. This pins it to
        // the parser so a flag cannot be described and not exist.
        for line in USAGE.lines() {
            let t = line.trim_start();
            if !t.starts_with("--") {
                continue;
            }
            let flag = t.split_whitespace().next().unwrap();
            let probe = [flag.to_string(), "1.2.3.4:1".to_string()];
            assert!(
                !matches!(parse(probe), Parsed::Bad(ref why) if why.contains("unknown option")),
                "USAGE documents {flag} and the parser does not accept it"
            );
        }
    }
}
