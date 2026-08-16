//! Every "no" the server can send, as the sentence a player reads.
//!
//! One shape per table: the array INDEX is the sim's own refusal code and
//! the string is what that code says out loud. (The count is deliberately
//! not written here — it drifted from "four" the first time a fifth landed,
//! and the tests below are the thing that actually counts.)
//!
//! [`CONSUME`] is the one table whose codes start at 1 rather than 0, and
//! the one serving two verbs at once; its doc says why in full.
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

// The CONNECT sentences used to live here as a fourth table, and they were a
// SECOND implementation of something `protocol` already had to say — the
// server formats the same refusals for its own connect helper, and neither
// crate can see the other's array. That is the `siwe_message` argument
// exactly: two implementations of one string is two chances to disagree, and
// the symptom is a player told the wrong reason they cannot play.
//
// So `protocol::refuse_text` is the one implementation and this delegates.
// The tables below stay local because their codes are `sim-core`'s, and
// nothing outside this client renders them.

/// `sim_core::craft`'s `REFUSE_*: u32`.
pub const CRAFT: [&str; 6] = [
    "no such recipe",
    "bad count",
    "needs a station",
    "queue full",
    "missing ingredients",
    // Not "no such recipe": the recipe exists and the player can see it,
    // which is exactly why this reason is its own code (research v0). The
    // sentence names the verb that fixes it, because a refusal a player
    // cannot act on is a refusal that reads as a bug.
    "not researched — take one to a research table",
];

/// `sim_core::research`'s `REFUSE_R_*: u32`. The last two are the tree
/// verb's (tech tree v0), and each names the act that fixes it —
/// `CRAFT`'s blueprint sentence sets the bar.
pub const RESEARCH: [&str; 7] = [
    "no research table in reach",
    "nothing in that slot",
    "that cannot be researched",
    "already known",
    "not enough obol",
    "unlock the node before it first",
    "no workbench of that level in reach",
];

/// `sim_core::build`'s `REFUSE_B_*: u32` — a build, an upgrade or a repair
/// on a structure piece.
pub const BUILD: [&str; 13] = [
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
    "too late to take that down",
    "nothing there",
];

/// `sim_core::deploy`'s `REFUSE_D_*: u32` — placing a deployable.
///
/// Codes 1–7 share their names with `REFUSE_B_*` and mostly share their
/// words, but not all of them: `REFUSE_D_COST` is an item you are not
/// carrying where `REFUSE_B_COST` is materials you cannot afford. The two
/// tables are separate for that reason and no check ties them together.
pub const DEPLOY: [&str; 20] = [
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
    // Lock v1's one sentence for both halves of "this lock does not know
    // you" — a stranger's press and a guest reaching for a full-rights
    // op. Kept as one because telling them apart would tell a raider
    // something about the lock refusing them.
    "the lock says no",
    "nothing in it to burn",
    "no lock on that door",
    "that door already has a lock",
    "wrong code",
    "keypad locked out",
    "that lock remembers too many already",
    "empty it first",
];

/// `sim_core::survival`'s `REFUSE_C_*: u32` — a consume that did nothing.
///
/// **Two things make this table unlike the four above it.**
///
/// First, its codes start at **1**, not 0. Zero is not a refusal on this
/// wire: `EventMsg::ConsumeRefused` never carries it and `ClientCore` uses
/// `last_eat_refused == 0` to mean *the consume landed*. Index 0 is filled
/// anyway so the array INDEX stays the sim's own code — shifting the table
/// by one would import the exact off-by-one class this whole file exists to
/// refuse — and its string is worded so that a reader who ever sees it on
/// screen knows a guard broke rather than reading it as a game rule.
///
/// Second, **one table serves two verbs.** `survival::consume` (eat, `G`)
/// emits `REFUSE_C_NOT_FOOD` and `REFUSE_C_FULL`; `survival::drink`
/// (`H`) emits `REFUSE_C_NO_WATER` and `REFUSE_C_FULL`, on the *same*
/// `EV_CONSUME_REFUSED`. So the sentences are worded to be true of
/// whichever verb was pressed: only code 1 may name food, only code 3 may
/// name water, and code 2 — the one both verbs reach — names neither.
pub const CONSUME: [&str; 4] = [
    // Code 0 = "no refusal". Unreachable through `render::feed::drain`,
    // which only pushes a non-zero code, and unreachable through
    // `hud::feedback`, which reads the landed half off `last_eat` instead.
    "nothing was refused",
    "that is not something you can eat",
    "already full — that would be wasted",
    "no water within reach",
];

/// `sim_core::gather`'s `REFUSE_G_*: u32` — a gather swing the node
/// refused (wire v42). [`CONSUME`]'s two oddities both apply: codes start
/// at 1 (zero is "no refusal", refused by the codec at both ends), and
/// the sentences serve a held item the caller names — each carries one
/// `{}` that [`gather`] fills with the item's own label, because the
/// point of the event's item field is saying *your torch cannot harvest
/// this* instead of "bare hands" (`NOW.md` §0kit item 2).
pub const GATHER: [&str; 3] = [
    // Code 0 = "no refusal" — unreachable, `CONSUME[0]`'s posture.
    "nothing was refused",
    "{} cannot harvest this",
    // Q3: re-craft is the repair, so the sentence names the act that
    // fixes it — a refusal a player cannot act on reads as a bug.
    "{} is broken — craft a new one",
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

/// Why the shard refused the connection — `protocol`'s table, not ours.
///
/// It answers `None` for a code this build has no word for, which is the
/// same case [`text`] handles for the sim's tables: a shard newer than this
/// client can refuse for a reason we cannot name, and naming the number is
/// honest where a guess would not be.
pub fn connect(code: u8) -> String {
    match protocol::refuse_text(code) {
        Some(s) => s.to_string(),
        None => format!("code {code}"),
    }
}

/// Why the craft was turned down.
pub fn craft(code: u8) -> String {
    text(&CRAFT, code as u32)
}

/// Why the research was turned down.
pub fn research(code: u8) -> String {
    text(&RESEARCH, code as u32)
}

/// Why the build, upgrade or repair was turned down.
pub fn build(code: u8) -> String {
    text(&BUILD, code as u32)
}

/// Why the deployable would not go down.
pub fn deploy(code: u8) -> String {
    text(&DEPLOY, code as u32)
}

/// Why the eat or the drink did nothing.
///
/// Never called with 0 — that code means the consume landed, and the landed
/// sentence is `hud::feedback`'s, because it names the item off `last_eat`
/// rather than a reason off this table.
pub fn consume(code: u8) -> String {
    text(&CONSUME, code as u32)
}

/// Why the gather swing did nothing, with the held item's label spliced
/// in — `held` is the possessive phrase ("your Torch", "bare hands"), so
/// the sentence reads as a whole. Never called with 0, [`consume`]'s rule.
pub fn gather(code: u8, held: &str) -> String {
    match GATHER.get(code as usize) {
        Some(s) if code != 0 => s.replacen("{}", held, 1),
        _ => format!("code {code}"),
    }
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
            // `…_MAX` names the ledger rather than sitting in it — it is the
            // highest live reason, restated so the wire's field can be
            // bounded against a constant instead of a literal. Counting it
            // would demand a sentence for a code no sim ever emits, which
            // is the opposite of what this gate is for. Same exemption the
            // protocol's domain table spells as `exempt: &["MAX"]`.
            .filter(|l| !l.contains(&format!("{prefix}MAX")))
            .count()
    }

    /// The half a hand-written table cannot self-check: has the sim grown a
    /// reason we have no sentence for?
    ///
    /// The fifth column is how many leading indices are **not** refusal
    /// codes. It is 0 for every table whose codes start at zero and 1 for
    /// [`CONSUME`], whose code 0 means "it landed" — see that table's doc
    /// for why the slot is filled rather than the array shifted.
    #[test]
    fn every_reason_the_sim_declares_has_a_sentence() {
        for (file, prefix, len, name, reserved) in [
            (
                "crates/sim-core/src/craft.rs",
                "REFUSE_",
                CRAFT.len(),
                "CRAFT",
                0,
            ),
            (
                "crates/sim-core/src/research.rs",
                "REFUSE_R_",
                RESEARCH.len(),
                "RESEARCH",
                0,
            ),
            (
                "crates/sim-core/src/build.rs",
                "REFUSE_B_",
                BUILD.len(),
                "BUILD",
                0,
            ),
            (
                "crates/sim-core/src/deploy.rs",
                "REFUSE_D_",
                DEPLOY.len(),
                "DEPLOY",
                0,
            ),
            (
                "crates/sim-core/src/survival.rs",
                "REFUSE_C_",
                CONSUME.len(),
                "CONSUME",
                1,
            ),
            (
                "crates/sim-core/src/gather.rs",
                "REFUSE_G_",
                GATHER.len(),
                "GATHER",
                1,
            ),
        ] {
            let declared = sim_code_count(file, prefix);
            assert_eq!(
                declared,
                len - reserved,
                "{name}: {file} declares {declared} {prefix}* codes and this table has \
                 {len} entries of which {reserved} are not codes. \
                 A code with no sentence reaches the player as `code N`."
            );
        }

        // The same claim off a SECOND source, which is the habit `CLAUDE.md`
        // wants after the relief sweep: the scrape above reads the file as
        // text and would agree with itself if a code were declared and the
        // ledger's top left behind, so the table's length is also bound to
        // the ledger constant the wire's four-bit field is checked against.
        assert_eq!(
            GATHER.len(),
            sim_core::gather::REFUSE_G_MAX as usize + 1,
            "GATHER must cover 0..=REFUSE_G_MAX exactly — the sim refuses any \
             reason above that constant at the encode boundary"
        );
        assert_eq!(
            CONSUME.len(),
            sim_core::survival::REFUSE_C_MAX as usize + 1,
            "CONSUME must cover 0..=REFUSE_C_MAX exactly — the sim refuses any \
             reason above that constant at the encode boundary, so a table \
             shorter than it has a hole and a longer one has a sentence no \
             shard can ever send"
        );
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
        assert_eq!(deploy(REFUSE_D_OWNER as u8), "the lock says no");
        assert_eq!(deploy(REFUSE_D_NO_LOCK as u8), "no lock on that door");
        assert_eq!(
            deploy(REFUSE_D_HAS_LOCK as u8),
            "that door already has a lock"
        );
        assert_eq!(deploy(REFUSE_D_CODE as u8), "wrong code");
        assert_eq!(deploy(REFUSE_D_LOCKOUT as u8), "keypad locked out");
        assert_eq!(
            deploy(REFUSE_D_AUTH_FULL as u8),
            "that lock remembers too many already"
        );
        assert_eq!(deploy(REFUSE_D_NOT_EMPTY as u8), "empty it first");

        assert_eq!(build(REFUSE_B_WINDOW as u8), "too late to take that down");
        assert_eq!(build(REFUSE_B_EMPTY as u8), "nothing there");

        // Paths rather than a glob: `survival` exports a great deal more
        // than its refusal ledger and a glob here would be three names of
        // borrowed scope for three assertions.
        assert_eq!(
            consume(sim_core::survival::REFUSE_C_NOT_FOOD as u8),
            "that is not something you can eat"
        );
        assert_eq!(
            consume(sim_core::survival::REFUSE_C_FULL as u8),
            "already full — that would be wasted"
        );
        assert_eq!(
            consume(sim_core::survival::REFUSE_C_NO_WATER as u8),
            "no water within reach"
        );

        // GATHER, bound to the constants with a fixture label so the
        // splice is checked with the binding.
        assert_eq!(
            gather(sim_core::gather::REFUSE_G_TOOL as u8, "your Torch"),
            "your Torch cannot harvest this"
        );
        assert_eq!(
            gather(sim_core::gather::REFUSE_G_BROKEN as u8, "your Rock"),
            "your Rock is broken — craft a new one"
        );

        use sim_core::craft::*;
        assert_eq!(craft(REFUSE_RECIPE as u8), "no such recipe");
        assert_eq!(craft(REFUSE_COUNT as u8), "bad count");
        assert_eq!(craft(REFUSE_STATION as u8), "needs a station");
        assert_eq!(craft(REFUSE_QUEUE_FULL as u8), "queue full");
        assert_eq!(craft(REFUSE_INPUTS as u8), "missing ingredients");

        // CONNECT is `protocol`'s table now (see the note above `connect`),
        // and protocol gates its own completeness. What is still this
        // crate's business is that the client asks IT rather than growing a
        // second copy — which is what the old assertions here would have
        // frozen in place.
        for code in [
            protocol::REFUSE_VERSION,
            protocol::REFUSE_FULL,
            protocol::REFUSE_AUTH,
            protocol::REFUSE_TICKET,
        ] {
            assert_eq!(
                connect(code),
                protocol::refuse_text(code).expect("every declared code has one"),
                "the client must not carry its own connect sentences"
            );
        }
    }

    /// No two codes in a table may share a sentence, or a player cannot tell
    /// which "no" they got.
    #[test]
    fn no_two_reasons_read_the_same() {
        for (table, name) in [
            (&CRAFT[..], "CRAFT"),
            (&BUILD[..], "BUILD"),
            (&DEPLOY[..], "DEPLOY"),
            // RESEARCH was missing from this list for no reason anyone
            // wrote down, and CONSUME needs it more than any table here:
            // its three codes are shared between two verbs, so two of them
            // reading the same would leave a player unable to tell a dry
            // shoreline from a full stomach.
            (&RESEARCH[..], "RESEARCH"),
            (&CONSUME[..], "CONSUME"),
            (&GATHER[..], "GATHER"),
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
        // The wire's `reason` is four bits, so 4..=15 is forgeable width the
        // server refuses at the encode boundary — but a shard NEWER than
        // this client can legitimately send a fifth reason, and that is the
        // case this fallback is for.
        assert_eq!(
            consume(CONSUME.len() as u8),
            format!("code {}", CONSUME.len())
        );
        assert_eq!(
            gather(GATHER.len() as u8, "your Torch"),
            format!("code {}", GATHER.len())
        );
    }
}
