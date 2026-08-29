//! Gate: the two run conditions that stopped a per-frame sweep are complete.
//!
//! `structures::stream` reconciles the whole piece/deploy/bag mirror and
//! `props::harvest` sweeps every fellable against a linear-scan membership
//! test. Both ran on every frame; both now run on a `Feed::applied` bit. That
//! trade is only sound while the bit set is **complete** — a bit that can
//! change what the system draws and is not in the set is a base that does not
//! redraw, or a felled tree that keeps standing, and neither produces an error.
//!
//! A hand-kept list of another crate's constants is exactly the mirror
//! `CLAUDE.md` says goes stale (`tests/sound.rs`'s drain scrape had drifted to
//! nine names against fourteen rings). So this reads `client_core`'s own
//! surface instead: every `APPLIED*` constant it publishes, classified by
//! name, with an exemption list that has to state a reason.
//!
//! **Writing it found a live defect in the change it gates.** `client_core`
//! has a SECOND applied word, and `APPLIED2_BAGS` is in it — a bit whose name
//! matches the mirror this system draws, in a word the run condition does not
//! read. It turns out to be the player's own bag list for the death screen
//! rather than the world's bag store, so the condition is right; but that is a
//! fact somebody had to go and check, and without §3 below nobody would have
//! to check it again.

use client::render::props::HARVEST_APPLIED;
use client::render::structures::STRUCT_APPLIED;

/// The applied-flag constants `client_core` publishes, as `(name, word, bit)`.
///
/// Scraped rather than listed, for the reason in the header. A malformed
/// declaration is a loud failure and never a skip — the floor has slack and a
/// silently dropped case is always the newest one.
fn applied_flags() -> Vec<(String, u8, u32)> {
    let src = include_str!("../../client-core/src/core.rs");
    let mut out = Vec::new();
    for line in src.lines() {
        let t = line.trim_start();
        let Some(rest) = t.strip_prefix("pub const APPLIED") else {
            continue;
        };
        // `APPLIED_FOO` or `APPLIED2_FOO`.
        let (word, rest) = match rest.strip_prefix('2') {
            Some(r) => (1u8, r),
            None => (0u8, rest),
        };
        let Some(rest) = rest.strip_prefix('_') else {
            continue;
        };
        let Some((name, value)) = rest.split_once(':') else {
            panic!("cannot parse an applied-flag declaration: {line}");
        };
        let bit = value
            .split("1 <<")
            .nth(1)
            .and_then(|v| v.trim().trim_end_matches(';').trim().parse::<u32>().ok())
            .unwrap_or_else(|| panic!("applied flag {name} is not declared as `1 << n`: {line}"));
        out.push((name.trim().to_string(), word, bit));
    }
    assert!(
        out.len() > 25,
        "the scrape found only {} applied flags — it is pointed at the wrong file \
         or the declaration shape changed",
        out.len()
    );
    out
}

/// Whether a flag's NAME says it is about the store `structures::stream` draws.
fn mirror_shaped(name: &str) -> bool {
    ["PIECE", "DEPLOY", "BAG", "STRUCT"]
        .iter()
        .any(|k| name.contains(k))
}

/// Mirror-shaped flags that are deliberately NOT in `STRUCT_APPLIED`, each
/// with the reason it cannot change what is drawn.
///
/// Adding a row is a claim, in the shape `tests/height_roles.rs` uses for its
/// own list: this bit's name looks like the mirror and it is not the mirror.
/// Do not add one to make the gate pass.
const NOT_THE_MIRROR: &[(&str, u8, &str)] = &[
    (
        "DEPLOY_REFUSED",
        0,
        "a refusal is feedback, not state: the server declined to place \
         something, so no deployable exists to draw. `render/panels` and the \
         HUD read it; the mirror is unchanged.",
    ),
    (
        "BAGS",
        1,
        "word 1's `APPLIED2_BAGS` is THIS CLIENT'S OWN bag list (`own_bags()`, \
         sent on a death and nowhere else) for the death screen to list \
         recoverable packs. The world's standing-backpack store is word 0's \
         `APPLIED_BAGS`, and that one is in the set.",
    ),
];

// ── 1. Every mirror-shaped word-0 bit is in the set ────────────────────────

#[test]
fn the_structure_change_gate_names_every_bit_that_moves_its_mirror() {
    let mut missing: Vec<String> = Vec::new();
    let mut covered = 0usize;
    for (name, word, bit) in applied_flags() {
        if word != 0 || !mirror_shaped(&name) {
            continue;
        }
        if NOT_THE_MIRROR
            .iter()
            .any(|(n, w, _)| *n == name && *w == word)
        {
            continue;
        }
        if STRUCT_APPLIED & (1 << bit) == 0 {
            missing.push(name);
        } else {
            covered += 1;
        }
    }
    assert!(
        missing.is_empty(),
        "these `client_core` flags name the store `structures::stream` draws and \
         are not in `STRUCT_APPLIED`:\n  {missing:?}\n\nThe system runs only on \
         frames where one of them is raised, so a bit left out is a base that \
         does not redraw — a piece upgraded, a door removed or a bag dropped and \
         the frame not told. Add it to `STRUCT_APPLIED`, or add it to \
         `NOT_THE_MIRROR` with the reason it cannot change what is drawn."
    );
    assert!(
        covered >= 9,
        "only {covered} mirror flags were checked — the scrape or the naming \
         convention moved"
    );
}

// ── 2. And the set names nothing that has stopped existing ─────────────────

#[test]
fn the_structure_change_gate_carries_no_bit_that_means_nothing() {
    let flags = applied_flags();
    let mut stale = Vec::new();
    for bit in 0..32u32 {
        if STRUCT_APPLIED & (1 << bit) == 0 {
            continue;
        }
        let named = flags.iter().find(|(_, w, b)| *w == 0 && *b == bit);
        match named {
            None => stale.push(format!("bit {bit} names no flag")),
            Some((name, _, _)) if !mirror_shaped(name) => stale.push(format!(
                "bit {bit} is APPLIED_{name}, which is not the mirror"
            )),
            Some(_) => {}
        }
    }
    assert!(
        stale.is_empty(),
        "`STRUCT_APPLIED` carries bits that no longer mean what it says:\n  \
         {stale:?}\n\nA list holding names that mean nothing is how the next \
         reader learns to distrust it."
    );
}

// ── 3. The second word is not quietly carrying the mirror ──────────────────

/// The run condition reads `Feed::applied` and nothing else. `client_core` has
/// a second word (`APPLIED2_*`, opened because word 0 filled up), and a
/// mirror-shaped flag landing in it would be invisible to the condition with
/// no compile error and no failing test anywhere else.
#[test]
fn no_second_word_flag_moves_the_mirror_unnoticed() {
    let mut unwatched: Vec<String> = Vec::new();
    for (name, word, _) in applied_flags() {
        if word != 1 || !mirror_shaped(&name) {
            continue;
        }
        if NOT_THE_MIRROR.iter().any(|(n, w, _)| *n == name && *w == 1) {
            continue;
        }
        unwatched.push(name);
    }
    assert!(
        unwatched.is_empty(),
        "these flags are in `client_core`'s SECOND applied word and name the \
         store `structures::stream` draws:\n  {unwatched:?}\n\n\
         `structures_changed` reads `Feed::applied` only, so this bit cannot \
         reach it. Either widen the condition to `applied2` — which means \
         widening `STRUCT_APPLIED` into a second constant, because the two \
         words share bit numbers — or add a row to `NOT_THE_MIRROR` saying why \
         this one cannot change what is drawn."
    );
}

// ── 4. The harvest gate ────────────────────────────────────────────────────

/// `props::harvest`'s bit is `APPLIED_SLOTS`, and the coverage claim — that
/// every write to `HarvestedSet` raises it — is gated where the writes are:
/// `client_core::core`'s own `stream_tracks_the_harvested_set` drives an
/// insert, a remove, a reset and a re-insert through `on_stream` and asserts
/// the returned word each time. This is the half that test cannot see: that
/// the client reads the same bit it raises.
#[test]
fn the_harvest_gate_watches_the_slot_bit_and_only_that() {
    assert_eq!(
        HARVEST_APPLIED,
        client_core::core::APPLIED_SLOTS,
        "`props::harvest` runs on this word; `APPLIED_SLOTS` is the only flag \
         `client_core` raises when the harvested set moves"
    );
    assert_ne!(HARVEST_APPLIED, 0, "the harvest sweep would never run");

    // And no OTHER flag claims the slot set by name, in either word — the
    // failure this would catch is a second bit being added for respawns.
    let stray: Vec<String> = applied_flags()
        .into_iter()
        .filter(|(n, _, b)| {
            (n.contains("SLOT") || n.contains("HARVEST")) && (1u32 << b) != HARVEST_APPLIED
        })
        .map(|(n, w, _)| format!("APPLIED{}_{n}", if w == 1 { "2" } else { "" }))
        .collect();
    assert!(
        stray.is_empty(),
        "these flags name the harvested set and are not the one `props::harvest` \
         watches:\n  {stray:?}"
    );
}
