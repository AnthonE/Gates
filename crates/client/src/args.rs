//! The client's command line — shared by the headless `client` binary and the
//! windowed `gates` one, so a flag can never mean two things.
//!
//! This exists because of the depot. A elo depot's launch block names an argv
//! with placeholders the launcher fills:
//!
//! ```json
//! "launch": { "exec": "gates",
//!             "args": ["--server", "{server}", "--identity", "{wallet}",
//!                      "--servers", "{servers}"] }
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
                       issued for one, and the transport resolves it.
                       A join link (elo://join/gates/host:port) may be given
                       here too — that is how the OS hands one to this binary
  --server ADDR        the same, named. Wins over the positional form.
                       Given, the client joins it straight away; absent, it
                       opens the server menu instead
  --join LINK          a join link, named. Equivalent to --server with the
                       address the link carries — a link is a way of spelling
                       an address, never a second way to connect
  --servers URL        where to fetch the elo-shardlist-v1 document the menu
                       lists. Absent, the menu offers the default shard only
                       and says why the rest of it is empty
  --cert-hash SHA256   trust ONLY the shard certificate with this SHA-256,
                       as the shard prints it at boot (aa:bb:...). For a dev
                       shard that is not on loopback: without it such a shard
                       is refused, because a self-signed certificate off this
                       machine is indistinguishable from a relay's. Needs
                       --server — the menu joins listed shards, which serve
                       real certificates and need no pin
  --identity ADDR      the wallet address to play as. The handshake proves it
                       by signing the shard's SIWE challenge with the key the
                       launcher holds; no launcher (or a declined prompt)
                       joins as a guest. Empty is the same as absent
  --capture DIR        run the probe harness instead of a player: settle, warm
                       the pipelines, shoot the vantage list, exit (RENDER.md)
  --no-hud             with --capture: shoot the world with no HUD, no
                       viewmodel and no compass — a clean PLATE. What it is
                       for is the menu backdrop, which is footage rather than
                       a live scene, and a screenshot with a hotbar across it
                       is not footage. Refused without --capture: a HUD-less
                       client a player could walk around in is a different
                       thing nobody asked for
  --no-launcher        do not look for an elo launcher, even if one is running
  --help               this

F12 takes a screenshot, on any screen, and the game reads its own frame to do
it — so it works where a desktop screenshot key does not (a Wayland session
with no portal, a bare WM, a fullscreen surface a compositor hands back black).
They land in ~/Pictures/gates (%USERPROFILE%\\Pictures\\gates on Windows,
$XDG_PICTURES_DIR/gates where the session sets one); GATES_SHOTS_DIR overrides
that verbatim. The name is gates-YYYYMMDD-HHMMSS.png, UTC, so it sorts by time.

The elo launcher fills --server and --identity from a depot's launch block;
running without one is a normal, supported state. A player who joined from the
launcher's own Servers window arrives with --server already chosen, which is
why that flag suppresses the menu rather than pre-selecting a row in it.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args {
    /// `host:port`, shape-checked and NOT resolved — see `shardlist::
    /// check_addr`. A `SocketAddr` here used to refuse every hostname,
    /// including `game.elopros.com`, which is the name the public shard's
    /// certificate is issued for and the name the transport needs for SNI.
    pub server: String,
    /// Whether `server` was asked for or is just the default. The menu turns
    /// on this: an address the launcher chose must not be second-guessed by
    /// a screen asking the player to choose again.
    pub server_given: bool,
    /// `--servers URL`: the `elo-shardlist-v1` document. `None` is normal
    /// and is a *stated* empty menu, never a silent one.
    pub servers_url: Option<String>,
    /// `None` when absent OR empty — see the module docs; the launcher's
    /// empty substitution is how "no wallet set" arrives.
    pub identity: Option<String>,
    /// `--capture DIR`: run the probe harness instead of a player
    /// (`RENDER.md`). Only the windowed binary honours it; the headless one
    /// parses it so a shared parser cannot silently mean two things.
    pub capture: Option<PathBuf>,
    /// `--no-hud`: shoot a clean plate. Only ever true alongside `capture`,
    /// which the parser enforces rather than leaving to the caller.
    pub no_hud: bool,
    /// `--cert-hash`: the ONE shard certificate this run will trust, as the
    /// shard prints it (`client::client_endpoint`). `None` is the shipping
    /// state and means the posture is chosen from the address — permissive on
    /// loopback, the platform root store everywhere else.
    ///
    /// Shape-checked at the endpoint and not here, deliberately: the parser
    /// owns argv and `wtransport::tls::Sha256Digest` owns what a digest is,
    /// and a second opinion about hex in this file is a second thing to keep
    /// in step. What this file DOES enforce is that the flag reaches
    /// something — see the `--server` requirement below.
    pub cert_hash: Option<String>,
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
    let mut join_flag: Option<String> = None;
    let mut server_pos: Option<String> = None;
    let mut servers_url: Option<String> = None;
    let mut identity: Option<String> = None;
    let mut capture: Option<PathBuf> = None;
    let mut cert_hash: Option<String> = None;
    let mut no_hud = false;
    let mut no_launcher = false;

    let mut it = argv.into_iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--help" | "-h" => return Parsed::Help,
            "--no-launcher" => no_launcher = true,
            "--no-hud" => no_hud = true,
            "--server" => match it.next() {
                Some(v) => server_flag = Some(v),
                None => return Parsed::Bad("--server needs an address".into()),
            },
            "--join" => match it.next() {
                Some(v) => join_flag = Some(v),
                None => return Parsed::Bad("--join needs a join link".into()),
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
            // Same empty-placeholder rule as every flag above it, even though
            // no launcher substitutes this one today: a flag that behaves
            // differently from its neighbours on `""` is a flag someone will
            // wire into a launch block and be surprised by.
            "--cert-hash" => match it.next() {
                Some(v) => cert_hash = Some(v),
                None => return Parsed::Bad("--cert-hash needs a sha-256 digest".into()),
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
    //
    // `--server` beats `--join` beats the positional, which is the existing
    // precedence with the link form slotted where it belongs: an operator who
    // typed an explicit address means it, and a link is the least explicit of
    // the three because it usually arrived from someone else.
    let given = server_flag
        .filter(|s| !s.trim().is_empty())
        .or(join_flag.filter(|s| !s.trim().is_empty()))
        .or(server_pos.filter(|s| !s.trim().is_empty()));
    let server_given = given.is_some();
    let raw = given.unwrap_or_else(|| DEFAULT_SERVER.to_string());

    // A join link resolves to the address it carries and then follows exactly
    // the path a typed one does — same field, same shape check, same
    // menu-skipping `server_given`. `deeplink::parse` is what refuses a link
    // for another title or one carrying anything past the address.
    //
    // Checked BEFORE `check_addr` so a malformed link is refused as a bad
    // *link*, naming the shape a link should have; falling through would
    // report `"elo://join/gates/nonsense" is a url, not a host:port`, which
    // names the wrong mistake to someone who was handed a link by a friend.
    let raw = if crate::deeplink::is_link(&raw) {
        match crate::deeplink::parse(&raw) {
            Ok(j) => j.addr,
            Err(why) => return Parsed::Bad(why),
        }
    } else {
        raw
    };

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

    // Refused rather than ignored. A flag that silently does nothing is how
    // an operator spends an afternoon wondering why their plates still have a
    // hotbar on them — the same refuse-don't-ignore rule `--servers` above
    // applies to a url it cannot use.
    if no_hud && capture.is_none() {
        return Parsed::Bad("--no-hud only means something with --capture".into());
    }

    // Refused rather than ignored, for the same reason `--no-hud` is, and
    // with a sharper edge: only the straight-in connect reads the pin, so a
    // `--cert-hash` without a `--server` would be a player believing they had
    // pinned a certificate while the menu dialled a shard with the flag
    // nowhere in the path. That is the one failure mode a security flag may
    // not have. `render::menu` therefore passes `None` and is *correct* to,
    // because this line makes the pairing unreachable.
    let cert_hash = cert_hash
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    if cert_hash.is_some() && !server_given {
        return Parsed::Bad("--cert-hash pins one shard's certificate and needs --server".into());
    }

    Parsed::Run(Args {
        server: raw.trim().to_string(),
        server_given,
        servers_url,
        identity: identity
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        capture,
        no_hud,
        cert_hash,
        no_launcher,
    })
}

impl Args {
    /// The address this client will claim, as a wire value.
    ///
    /// **A claim until it is signed for.** `--identity` says who the player
    /// means to be; the handshake proves it by signing the shard's nonce
    /// with the key behind that address (`Session::connect`). An absent or
    /// unparseable one is [`Address::GUEST`], which is a normal, playable
    /// state on a shard that takes guests.
    ///
    /// Malformed is an **error, never a silent guest**: a typo'd address
    /// would otherwise present as "the shard forgot my base", which is the
    /// worst possible way to learn about a typo.
    pub fn address(&self) -> Result<protocol::Address, String> {
        match self.identity.as_deref() {
            None => Ok(protocol::Address::GUEST),
            Some(a) => protocol::Address::from_hex(a.as_bytes()).ok_or_else(|| {
                format!(
                    "--identity `{a}` is not an Ethereum address — it must be \
                     0x followed by 40 hex digits"
                )
            }),
        }
    }
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
        let x = run(&[
            "--server",
            "10.0.0.4:4433",
            "--identity",
            "0xAbC",
            "--servers",
            "https://elopros.com/api/launcher/servers/gates",
        ]);
        assert_eq!(x.server, "10.0.0.4:4433");
        assert_eq!(x.identity.as_deref(), Some("0xAbC"));
        assert_eq!(
            x.servers_url.as_deref(),
            Some("https://elopros.com/api/launcher/servers/gates")
        );
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
            run(&["--server", "game.elopros.com:61234"]).server,
            "game.elopros.com:61234"
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
    fn a_join_link_is_a_way_of_spelling_an_address() {
        // Both doors, and both must land in the same field as a typed one —
        // including `server_given`, which is what skips the menu. A player who
        // clicked a friend's link has chosen a shard exactly as firmly as one
        // who picked a row.
        for argv in [
            vec!["elo://join/gates/game.elopros.com:61234"],
            vec!["--join", "elo://join/gates/game.elopros.com:61234"],
            vec!["gates://game.elopros.com:61234"],
        ] {
            let x = run(&argv);
            assert_eq!(x.server, "game.elopros.com:61234", "{argv:?}");
            assert!(x.server_given, "{argv:?} must skip the menu");
        }
    }

    #[test]
    fn a_bad_link_is_refused_as_a_link_and_not_as_an_address() {
        // The message names the mistake the player actually made. Falling
        // through to `check_addr` would report "is a url, not a host:port" to
        // someone who was handed a url on purpose.
        let why = match a(&["elo://join/some-other-game/h:1"]) {
            Parsed::Bad(why) => why,
            other => panic!("expected Bad, got {other:?}"),
        };
        assert!(why.contains("some-other-game"), "{why}");
        assert!(!why.contains("bad address"), "{why}");

        assert!(matches!(a(&["elo://join/gates/nonsense"]), Parsed::Bad(_)));
        assert!(matches!(a(&["--join", "not-a-link"]), Parsed::Bad(_)));
        assert!(matches!(a(&["--join"]), Parsed::Bad(_)));
    }

    #[test]
    fn an_explicit_address_still_beats_a_link() {
        // Precedence, pinned: --server > --join > positional.
        let x = run(&["--join", "elo://join/gates/h:1", "--server", "5.6.7.8:2"]);
        assert_eq!(x.server, "5.6.7.8:2");
        let x = run(&["elo://join/gates/h:1", "--join", "elo://join/gates/h:2"]);
        assert_eq!(x.server, "h:2");
        // An unfilled --join is absence, the same rule every other flag here
        // follows — it must not become "no server" when a positional exists.
        let x = run(&["h:9", "--join", ""]);
        assert_eq!(x.server, "h:9");
        assert_eq!(run(&["--join", ""]).server, DEFAULT_SERVER);
        assert!(!run(&["--join", ""]).server_given);
    }

    #[test]
    fn a_certificate_pin_is_carried_and_cannot_be_set_without_a_shard() {
        // The dev shape: an address and the hash the shard printed for it.
        let x = run(&["--server", "10.0.0.4:4433", "--cert-hash", "aa:bb:cc"]);
        assert_eq!(x.cert_hash.as_deref(), Some("aa:bb:cc"));
        assert!(run(&[]).cert_hash.is_none());
        // Same unfilled-placeholder rule as every other flag in this parser.
        assert!(run(&["--server", "1.2.3.4:1", "--cert-hash", ""])
            .cert_hash
            .is_none());
        // **The one that matters.** A pin with no shard to pin means the menu
        // will do the dialling and the flag will never be read — a player
        // believing they are pinned while nothing is. Refused, so that
        // `render::menu` passing `None` is provably not a hole.
        assert!(matches!(a(&["--cert-hash", "aa:bb"]), Parsed::Bad(_)));
        assert!(matches!(a(&["--cert-hash"]), Parsed::Bad(_)));
        // A positional address is a chosen shard too, so it pairs.
        assert_eq!(
            run(&["10.0.0.4:4433", "--cert-hash", "aa:bb"])
                .cert_hash
                .as_deref(),
            Some("aa:bb")
        );
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
