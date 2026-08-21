//! The ticket door as an operator meets it: `shard.toml` keys, the pairing
//! this shard refuses to boot without, and the one constant that has to
//! agree with a string in another crate.
//!
//! The verdict arithmetic — which of three answers admits, which kicks — is
//! unit-tested next to the code it protects (`entitle.rs`). What is here is
//! everything that only shows up once a config file, two crates and a boot
//! sequence are in the same room, which is where this seam actually failed
//! for other people: not in the parse, but in an armed door hanging over an
//! open one.

use server::config::parse_shard_toml;
use server::entitle::{self, Verdict};

const BASE: &str = "bind = \"127.0.0.1:0\"\nseed = 1\n";

fn cfg(extra: &str) -> Result<server::config::ShardConfig, String> {
    parse_shard_toml(&format!("{BASE}{extra}"))
}

// ── the pairing that would silently check nobody ────────────────────────────

#[test]
fn a_ticket_door_over_an_open_door_refuses_to_boot() {
    let err = cfg("entitle_origin = \"https://elo.example\"\n")
        .expect_err("an armed door with require_auth off must not boot");
    assert!(
        err.contains("require_auth"),
        "the error has to name the key that fixes it, got {err:?}"
    );

    let ok = cfg("entitle_origin = \"https://elo.example\"\nrequire_auth = true\n")
        .expect("armed over require_auth is the shipping pairing");
    assert!(ok.entitle.armed());
}

#[test]
fn the_default_shard_checks_nothing() {
    let c = cfg("").expect("a bare config parses");
    assert!(
        !c.entitle.armed(),
        "unset must check NOTHING — every test and every community shard runs here"
    );
    // And an unarmed door does not merely skip: it admits, so no caller can
    // read "off" as "could not look" and start refusing people.
    assert_eq!(entitle::check_one(&c.entitle, "0x00"), Verdict::Owns);
}

// ── the values an operator can get wrong ────────────────────────────────────

#[test]
fn an_origin_with_no_scheme_is_refused() {
    let err = cfg("entitle_origin = \"elopros.com\"\nrequire_auth = true\n")
        .expect_err("a bare host is not a URL and would build a garbage path");
    assert!(err.contains("scheme"), "got {err:?}");
}

#[test]
fn an_empty_origin_is_refused_rather_than_read_as_off() {
    // The distinction this pins: omitting the key means "check nothing" and
    // is a fine state. Writing `entitle_origin = ""` is somebody trying to
    // arm the door and getting it wrong, and silently treating it as off
    // would hand the game away with the config file looking correct.
    let err = cfg("entitle_origin = \"\"\nrequire_auth = true\n").expect_err("empty is not off");
    assert!(err.contains("omit the key"), "got {err:?}");
}

#[test]
fn a_trailing_slash_does_not_reach_the_url_builder() {
    let c = cfg("entitle_origin = \"https://elo.example/\"\nrequire_auth = true\n").unwrap();
    assert_eq!(
        c.entitle.origin.as_deref(),
        Some("https://elo.example"),
        "a trailing slash would build …//api/… , which some origins 404"
    );
}

#[test]
fn a_timeout_longer_than_its_sweep_is_refused_at_boot() {
    let err = cfg(
        "entitle_origin = \"https://elo.example\"\nrequire_auth = true\n\
         entitle_timeout_secs = 60\nentitle_sweep_secs = 30\n",
    )
    .expect_err("a round must finish before the next is due");
    assert!(err.contains("shorter"), "got {err:?}");
}

#[test]
fn the_knobs_read_the_way_they_are_written() {
    let c = cfg(
        "entitle_origin = \"https://elo.example\"\nrequire_auth = true\n\
         entitle_slug = \"gates-staging\"\nentitle_timeout_secs = 3\n\
         entitle_sweep_secs = 45\n",
    )
    .unwrap();
    assert_eq!(c.entitle.slug, "gates-staging");
    assert_eq!(c.entitle.timeout.as_secs(), 3);
    assert_eq!(c.entitle.sweep.as_secs(), 45);
}

#[test]
fn a_typo_in_a_ticket_key_is_refused_like_every_other_key() {
    // `config.rs`'s standing rule, re-asserted at this seam because a typo
    // here is the failure mode that runs an UNARMED shard while its operator
    // reads the file and believes otherwise.
    let err = cfg("entitle_orgin = \"https://elo.example\"\n").expect_err("a typo must not run");
    assert!(err.contains("unknown key"), "got {err:?}");
}

// ── the seam between the two crates ─────────────────────────────────────────

#[test]
fn the_server_and_the_client_agree_on_this_title_s_slug() {
    // Two copies on purpose — `client` must never depend on `server` and the
    // reverse would ship the sim in the client — so the agreement is pinned
    // rather than shared. They index the same catalog: a drift means the
    // shard asks elo about one title while the launcher installs another,
    // and both halves look correct alone.
    assert_eq!(server::ENTITLE_SLUG, client::elo::SLUG);
    assert_eq!(server::ENTITLE_SLUG, "gates");
}

#[test]
fn the_default_slug_is_the_one_the_shard_uses_unasked() {
    let c = cfg("").unwrap();
    assert_eq!(c.entitle.slug, server::ENTITLE_SLUG);
}

// ── the refusal a player is shown ───────────────────────────────────────────

#[test]
fn the_ticket_refusal_is_its_own_code_and_its_own_sentence() {
    assert_ne!(
        protocol::REFUSE_TICKET,
        protocol::REFUSE_AUTH,
        "'sign in through the launcher' is useless advice to somebody who \
         signed in fine and has not bought the game"
    );
    let text = protocol::refuse_text(protocol::REFUSE_TICKET).expect("it is declared");
    assert!(
        text.contains("copy"),
        "the sentence has to name the fix: {text:?}"
    );
    assert!(
        !text.contains("code"),
        "a number is not a sentence: {text:?}"
    );

    // A code this build has no word for answers `None` rather than guessing
    // — a shard newer than this client can refuse for a new reason, and each
    // caller says so in its own voice.
    assert!(protocol::refuse_text(200).is_none());
}

#[test]
fn every_refusal_code_has_a_sentence_and_they_are_all_different() {
    let codes = [
        protocol::REFUSE_VERSION,
        protocol::REFUSE_FULL,
        protocol::REFUSE_AUTH,
        protocol::REFUSE_TICKET,
    ];
    let mut seen: Vec<&str> = Vec::new();
    for c in codes {
        let t = protocol::refuse_text(c).expect("every declared code has a sentence");
        assert!(!t.is_empty());
        assert!(
            !seen.contains(&t),
            "two codes share a sentence, so the player cannot tell them apart: {t:?}"
        );
        seen.push(t);
    }
}
