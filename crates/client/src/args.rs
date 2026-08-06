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

use crate::shardlist::check_addr;
use std::path::PathBuf;

pub const DEFAULT_SERVER: &str = "127.0.0.1:4433";

pub const USAGE: &str = "\
gates — the Gates desktop client

  gates [ADDR] [options]

  ADDR                 shard address, host:port (default 127.0.0.1:4433).
                       A NAME is normal — the public shard's certificate is
                       issued for one, and the transport resolves it
  --server ADDR        the same, named. Wins over the positional form.
                       Given, the client joins it straight away; absent, it
                       opens the server menu instead
  --servers URL        where to fetch the scry-shardlist-v1 document the menu
                       lists. Absent, the menu offers the default shard only
                       and says why the rest of it is empty
  --identity ADDR      the wallet address to play as. UNVERIFIED — the shard
                       does not check it yet and this client does not claim it
                       does. Empty is the same as absent
  --capture DIR        run the probe harness instead of a player: settle, warm
                       the pipelines, shoot the vantage list, exit (RENDER.md)
  --no-launcher        do not look for a scry launcher, even if one is running
  --help               this

The scry launcher fills --server and --identity from a depot's launch block;
running without one is a normal, supported state. A player who joined from the
launcher's own Servers window arrives with --server already chosen, which is
why that flag suppresses the menu rather than pre-selecting a row in it.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args {
    /// `host:port`, shape-checked and NOT resolved — see `shardlist::
    /// check_addr`. A `SocketAddr` here used to refuse every hostname,
    /// including `game.moreright.xyz`, which is the name the public shard's
    /// certificate is issued for and the name the transport needs for SNI.
    pub server: String,
    /// Whether `server` was asked for or is just the default. The menu turns
    /// on this: an address the launcher chose must not be second-guessed by
    /// a screen asking the player to choose again.
    pub server_given: bool,
    /// `--servers URL`: the `scry-shardlist-v1` document. `None` is normal
    /// and is a *stated* empty menu, never a silent one.
    pub servers_url: Option<String>,
    /// `None` when absent OR empty — see the module docs; the launcher's
    /// empty substitution is how "no wallet set" arrives.
    pub identity: Option<String>,
    /// `--capture DIR`: run the probe harness instead of a player
    /// (`RENDER.md`). Only the windowed binary honours it; the headless one
    /// parses it so a shared parser cannot silently mean two things.
    pub capture: Option<PathBuf>,
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
    let mut servers_url: Option<String> = None;
    let mut identity: Option<String> = None;
    let mut capture: Option<PathBuf> = None;
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
            // Same empty-placeholder rule as the two below: a launcher that
            // has no shard list substitutes "" rather than dropping the flag.
            "--servers" => match it.next() {
                Some(v) => servers_url = Some(v),
                None => return Parsed::Bad("--servers needs a url".into()),
            },
            "--identity" => match it.next() {
                Some(v) => identity = Some(v),
                None => return Parsed::Bad("--identity needs an address".into()),
            },
            "--capture" => match it.next() {
                // Refused rather than defaulted. A capture run that shot into
                // the wrong directory and exited 0 is the failure shape
                // `RENDER.md` already paid for once.
                Some(v) if !v.trim().is_empty() => capture = Some(PathBuf::from(v)),
                _ => return Parsed::Bad("--capture needs a directory".into()),
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
    let given = server_flag
        .filter(|s| !s.trim().is_empty())
        .or(server_pos.filter(|s| !s.trim().is_empty()));
    let server_given = given.is_some();
    let raw = given.unwrap_or_else(|| DEFAULT_SERVER.to_string());

    // Shape only. Resolving here would put a DNS lookup inside argument
    // parsing, which is I/O in the one part of startup that has to stay
    // testable offline — and `wtransport` resolves the name itself anyway,
    // because it needs the unresolved name for SNI.
    if let Err(why) = check_addr(&raw) {
        return Parsed::Bad(format!("bad address: {why}"));
    }

    let servers_url = match servers_url.map(|s| s.trim().to_string()) {
        // An empty `--servers` is an unfilled placeholder, not a bad url —
        // the same rule `--identity ""` follows.
        None => None,
        Some(u) if u.is_empty() => None,
        // Refused rather than carried: a url the menu cannot fetch would
        // become a dark panel blaming the network for a typo.
        Some(u) if !u.starts_with("https://") && !u.starts_with("http://") => {
            return Parsed::Bad(format!("--servers {u:?} is not an http(s) url"));
        }
        Some(u) => Some(u),
    };

    Parsed::Run(Args {
        server: raw.trim().to_string(),
        server_given,
        servers_url,
        identity: identity
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        capture,
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
        assert_eq!(x.server, DEFAULT_SERVER);
        assert_eq!(x.identity, None);
        assert!(!x.no_launcher);
    }

    #[test]
    fn the_positional_form_still_works() {
        assert_eq!(run(&["10.0.0.4:4433"]).server, "10.0.0.4:4433");
    }

    #[test]
    fn the_launch_block_shape_parses() {
        // Verbatim the argv a depot's launch args produce once filled.
        let x = run(&["--server", "10.0.0.4:4433", "--identity", "0xAbC"]);
        assert_eq!(x.server, "10.0.0.4:4433");
        assert_eq!(x.identity.as_deref(), Some("0xAbC"));
    }

    #[test]
    fn an_unfilled_placeholder_is_absence_not_an_empty_address() {
        // What `{wallet}` becomes when the player has no address set. This is
        // the normal anonymous launch and it must not read as an identity.
        let x = run(&["--server", "10.0.0.4:4433", "--identity", ""]);
        assert_eq!(x.identity, None);
        assert_eq!(x.server, "10.0.0.4:4433");

        // ...and the same for the server, which falls back rather than failing.
        let x = run(&["--server", "", "--identity", "0x1"]);
        assert_eq!(x.server, DEFAULT_SERVER);
        assert_eq!(x.identity.as_deref(), Some("0x1"));
    }

    #[test]
    fn the_named_form_beats_the_positional_one() {
        let x = run(&["1.2.3.4:1", "--server", "5.6.7.8:2"]);
        assert_eq!(x.server, "5.6.7.8:2");
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
    fn capture_survives_the_move_into_this_parser() {
        // `--capture` was hand-parsed in gates.rs before the launcher's flags
        // arrived. The merge folded it in here, and this pins that it still
        // works alongside them rather than being eaten as a positional.
        let x = run(&["--capture", "/tmp/shots", "--server", "1.2.3.4:1"]);
        assert_eq!(
            x.capture.as_deref(),
            Some(std::path::Path::new("/tmp/shots"))
        );
        assert_eq!(x.server, "1.2.3.4:1");
        assert!(run(&[]).capture.is_none());
        // An empty directory is refused, not defaulted: a capture that shot
        // into the wrong place and exited 0 is a gate that passed for nothing.
        assert!(matches!(a(&["--capture", ""]), Parsed::Bad(_)));
        assert!(matches!(a(&["--capture"]), Parsed::Bad(_)));
    }

    #[test]
    fn a_name_is_an_address() {
        // The regression that made the server menu possible at all: the
        // public shard is reached by the name its certificate is issued for,
        // and `SocketAddr::from_str` refused every one of them.
        assert_eq!(
            run(&["--server", "game.moreright.xyz:61234"]).server,
            "game.moreright.xyz:61234"
        );
        assert_eq!(run(&["[::1]:4433"]).server, "[::1]:4433");
        // Still shape-checked, though — a typo must not become a DNS lookup
        // for a hostname with a colon in it.
        assert!(matches!(a(&["host:port"]), Parsed::Bad(_)));
        assert!(matches!(a(&["host:0"]), Parsed::Bad(_)));
    }

    #[test]
    fn whether_the_server_was_chosen_is_a_separate_fact_from_what_it_is() {
        // The menu turns on this bit. A player who came through the
        // launcher's Servers window has already chosen, and must not be
        // asked again; a player who ran the binary bare has not.
        assert!(!run(&[]).server_given);
        assert!(run(&["--server", "1.2.3.4:1"]).server_given);
        assert!(run(&["1.2.3.4:1"]).server_given);
        // An unfilled placeholder is NOT a choice — this is the case that
        // decides whether a launcher with no shard picked opens the menu.
        assert!(!run(&["--server", ""]).server_given);
        assert_eq!(run(&["--server", ""]).server, DEFAULT_SERVER);
    }

    #[test]
    fn the_shard_list_url_is_optional_and_checked() {
        assert_eq!(run(&[]).servers_url, None);
        let x = run(&["--servers", "https://example.test/servers.json"]);
        assert_eq!(
            x.servers_url.as_deref(),
            Some("https://example.test/servers.json")
        );
        // Same unfilled-placeholder rule as every other flag here.
        assert_eq!(run(&["--servers", ""]).servers_url, None);
        // A typo is refused at parse rather than becoming a dark menu that
        // blames the network.
        assert!(matches!(a(&["--servers", "example.test"]), Parsed::Bad(_)));
        assert!(matches!(a(&["--servers"]), Parsed::Bad(_)));
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
