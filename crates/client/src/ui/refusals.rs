//! Every "no" the server can send, as the sentence a player reads.
//!
//! Four tables, one shape: the array INDEX is the sim's own refusal code and
//! the string is what that code says out loud.
//!
//! ## Why this is gated the way it is
//!
//! `web/src/refusals.js`'s header records the same file falling behind the
//! sim **twice** — `REFUSE_B_INTACT` (9) shipped and the table stayed at
//! nine entries, so repairing an undamaged wall (the likeliest repair
//! refusal there is) answered `can't build: code 9`; then `REFUSE_B_UNPRICED`
//! (10) landed from the sim lane and did it again.
//!
//! Worse than a short table is a **transposed** one. That happened too: a
//! judge exchanged `REFUSE_D_HEARTH` (10) and `REFUSE_D_DOOR` (11) in
//! `deploy.rs`, touched no client file, and the browser's gate stayed green
//! while a player placing on a missing hearth was told "no door there" —
//! `CLAUDE.md`'s positional-payload trap, landing in exactly this file. The
//! browser's gate answers that with a keyword-per-code heuristic because JS
//! cannot see a Rust constant.
//!
//! **Rust can.** So the tests below bind each sentence to its constant BY
//! NAME (`assert_eq!(build(REFUSE_B_INTACT), "not damaged")`), which no
//! transposition survives, and then read the sim's own source to count the
//! codes it declares — the "the sim is ahead of us" half that a hand-written
//! list cannot self-check. Adding a refusal to the sim turns this red until
//! the sentence lands, on purpose.

/// `protocol`'s `REFUSE_*: u8` — why a shard turned the connection down at
/// hello, before there is a world to be in.
pub const CONNECT: [&str; 3] = [
    "protocol version mismatch",
    "shard is full",
    // `REFUSE_AUTH`. The wire deliberately does not distinguish "you sent no
    // token" from "your token was rejected" — that split is a probing oracle
    // — and the player-facing sentence is the same either way, because the
    // action is the same: sign in through the launcher and come back.
    "this shard needs a launcher sign-in",
];

/// `sim_core::craft`'s `REFUSE_*: u32`.
pub const CRAFT: [&str; 5] = [
    "no such recipe",
    "bad count",
    "needs a station",
    "queue full",
    "missing ingredients",
];

/// `sim_core::build`'s `REFUSE_B_*: u32` — a build, an upgrade or a repair
/// on a structure piece.
pub const BUILD: [&str; 11] = [
    "no such piece",
    "spot taken",
    "needs support",
    "bad ground",
    "out of reach",
    "missing materials",
    "world is full",
    "claimed by a hearth",
    "nothing to upgrade into",
    "not damaged",
    "cannot be repaired",
];

/// `sim_core::deploy`'s `REFUSE_D_*: u32` — placing a deployable.
///
/// Codes 1–7 share their names with `REFUSE_B_*` and mostly share their
/// words, but not all of them: `REFUSE_D_COST` is an item you are not
/// carrying where `REFUSE_B_COST` is materials you cannot afford. The two
/// tables are separate for that reason and no check ties them together.
pub const DEPLOY: [&str; 13] = [
    "no such deployable",
    "spot taken",
    "needs support",
    "bad ground",
    "out of reach",
    "item not in inventory",
    "world is full",
    "claimed by a hearth",
    "too close to a hearth",
    "bag limit reached",
    "no hearth there",
    "no door there",
    "not your door",
];

/// The sentence, or the bare code when the sim is ahead of the client.
///
/// The fallback keeps a wire ahead of us honest rather than mislabelled: an
/// unknown code prints as itself instead of borrowing the nearest sentence,
/// so a player who reports `code 13` is reporting something a developer can
/// find.
fn text(table: &[&str], code: u32) -> String {
    match table.get(code as usize) {
        Some(s) => (*s).to_string(),
        None => format!("code {code}"),
    }
}

/// Why the shard refused the connection.
pub fn connect(code: u8) -> String {
    text(&CONNECT, code as u32)
}

/// Why the craft was turned down.
pub fn craft(code: u8) -> String {
    text(&CRAFT, code as u32)
}

/// Why the build, upgrade or repair was turned down.
pub fn build(code: u8) -> String {
    text(&BUILD, code as u32)
}

/// Why the deployable would not go down.
pub fn deploy(code: u8) -> String {
    text(&DEPLOY, code as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Count `pub const <prefix>` declarations in a sim source file.
    ///
    /// Reading the source is the only way to ask "how many reasons does the
    /// sim have" — Rust has no reflection over constants, and a hand-kept
    /// second list is the very thing that went stale twice. The path is
    /// relative to this crate's manifest dir so it works from any cwd.
    fn sim_code_count(file: &str, prefix: &str) -> usize {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(file);
        let src = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        src.lines()
            .filter(|l| l.trim_start().starts_with(&format!("pub const {prefix}")))
            .count()
    }

    /// The half a hand-written table cannot self-check: has the sim grown a
    /// reason we have no sentence for?
    #[test]
    fn every_reason_the_sim_declares_has_a_sentence() {
        for (file, prefix, len, name) in [
            (
                "crates/sim-core/src/craft.rs",
                "REFUSE_",
                CRAFT.len(),
                "CRAFT",
            ),
            (
                "crates/sim-core/src/build.rs",
                "REFUSE_B_",
                BUILD.len(),
                "BUILD",
            ),
            (
                "crates/sim-core/src/deploy.rs",
                "REFUSE_D_",
                DEPLOY.len(),
                "DEPLOY",
            ),
            (
                "crates/protocol/src/lib.rs",
                "REFUSE_",
                CONNECT.len(),
                "CONNECT",
            ),
        ] {
            let declared = sim_code_count(file, prefix);
            assert_eq!(
                declared, len,
                "{name}: {file} declares {declared} {prefix}* codes and this table has {len}. \
                 A code with no sentence reaches the player as `code N`."
            );
        }
    }

    /// Bound to the CONSTANT, not to the index, so a transposition in the
    /// sim is red here. This is the check the browser's gate could only
    /// approximate with keywords, and the one the hearth/door swap defeated.
    #[test]
    fn each_sentence_is_bound_to_its_named_code() {
        use sim_core::build::*;
        assert_eq!(build(REFUSE_B_PIECE as u8), "no such piece");
        assert_eq!(build(REFUSE_B_SPOT as u8), "spot taken");
        assert_eq!(build(REFUSE_B_SUPPORT as u8), "needs support");
        assert_eq!(build(REFUSE_B_TERRAIN as u8), "bad ground");
        assert_eq!(build(REFUSE_B_REACH as u8), "out of reach");
        assert_eq!(build(REFUSE_B_COST as u8), "missing materials");
        assert_eq!(build(REFUSE_B_FULL as u8), "world is full");
        assert_eq!(build(REFUSE_B_CLAIM as u8), "claimed by a hearth");
        assert_eq!(build(REFUSE_B_TIER as u8), "nothing to upgrade into");
        assert_eq!(build(REFUSE_B_INTACT as u8), "not damaged");
        assert_eq!(build(REFUSE_B_UNPRICED as u8), "cannot be repaired");

        use sim_core::deploy::*;
        assert_eq!(deploy(REFUSE_D_KIND as u8), "no such deployable");
        assert_eq!(deploy(REFUSE_D_SPOT as u8), "spot taken");
        assert_eq!(deploy(REFUSE_D_SUPPORT as u8), "needs support");
        assert_eq!(deploy(REFUSE_D_TERRAIN as u8), "bad ground");
        assert_eq!(deploy(REFUSE_D_REACH as u8), "out of reach");
        assert_eq!(deploy(REFUSE_D_COST as u8), "item not in inventory");
        assert_eq!(deploy(REFUSE_D_FULL as u8), "world is full");
        assert_eq!(deploy(REFUSE_D_CLAIM as u8), "claimed by a hearth");
        assert_eq!(deploy(REFUSE_D_OVERLAP as u8), "too close to a hearth");
        assert_eq!(deploy(REFUSE_D_BAG_CAP as u8), "bag limit reached");
        assert_eq!(deploy(REFUSE_D_HEARTH as u8), "no hearth there");
        assert_eq!(deploy(REFUSE_D_DOOR as u8), "no door there");
        assert_eq!(deploy(REFUSE_D_OWNER as u8), "not your door");

        use sim_core::craft::*;
        assert_eq!(craft(REFUSE_RECIPE as u8), "no such recipe");
        assert_eq!(craft(REFUSE_COUNT as u8), "bad count");
        assert_eq!(craft(REFUSE_STATION as u8), "needs a station");
        assert_eq!(craft(REFUSE_QUEUE_FULL as u8), "queue full");
        assert_eq!(craft(REFUSE_INPUTS as u8), "missing ingredients");

        assert_eq!(
            connect(protocol::REFUSE_VERSION),
            "protocol version mismatch"
        );
        assert_eq!(connect(protocol::REFUSE_FULL), "shard is full");
        assert_eq!(
            connect(protocol::REFUSE_AUTH),
            "this shard needs a launcher sign-in"
        );
    }

    /// No two codes in a table may share a sentence, or a player cannot tell
    /// which "no" they got.
    #[test]
    fn no_two_reasons_read_the_same() {
        for (table, name) in [
            (&CRAFT[..], "CRAFT"),
            (&BUILD[..], "BUILD"),
            (&DEPLOY[..], "DEPLOY"),
            (&CONNECT[..], "CONNECT"),
        ] {
            for (i, a) in table.iter().enumerate() {
                assert!(!a.is_empty(), "{name}[{i}] is empty");
                for (j, b) in table.iter().enumerate().skip(i + 1) {
                    assert_ne!(a, b, "{name}[{i}] and {name}[{j}] read the same");
                }
            }
        }
    }

    /// Past the end it falls through to the code rather than borrowing the
    /// nearest sentence.
    #[test]
    fn an_unknown_code_prints_as_itself() {
        assert_eq!(build(BUILD.len() as u8), format!("code {}", BUILD.len()));
        assert_eq!(deploy(200), "code 200");
    }
}
