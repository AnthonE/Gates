//! Gate: the clip table and the graph's array width cannot drift apart, and
//! every clip name still exists in the shipped rig.
//!
//! # Why a source scrape and not a runtime check
//!
//! Two failures live here and **neither is a compile error**, which is the
//! whole reason this file exists.
//!
//! The first is arithmetic. `Clip::slot()` indexes `Rig::nodes`, whose
//! width is a literal written in three places — the field's type and two
//! `[AnimationNodeIndex::default(); N]` constructors — while the number of
//! clips is a fourth literal on `Clip::ALL`. Adding a variant and moving
//! three of the four compiles perfectly and panics the first time that
//! clip is played. For a locomotion loop that would be instant and
//! obvious; for the one-shot swing it is the first time *somebody else*
//! swings near you, which is a state no headless test reaches and no
//! capture vantage stands in.
//!
//! The second is a rename. Clips are resolved by NAME through
//! `Gltf::named_animations`, and a name the file does not have is reported
//! at runtime with `error!` and then falls back to nothing forever — so a
//! re-vendored mannequin with a renamed clip is a body that stops
//! animating, in a log line nobody is reading, with every gate green.
//!
//! Both are checked against the actual shipped `.gltf` rather than against
//! a list kept here, for `CLAUDE.md`'s reason: a hand-kept mirror of
//! another file's surface goes stale, so read the surface.

const ANIM: &str = include_str!("../src/render/anim.rs");
const GLTF: &str = include_str!("../../../assets/models/mannequin.gltf");

/// Every `Clip` name in the match arms, scraped as text.
fn clip_names() -> Vec<String> {
    let mut out = Vec::new();
    for line in ANIM.lines() {
        let line = line.trim();
        // `Clip::Idle => "Idle_Loop",` — the arm shape, and only in
        // `name()`, because it is the only match that maps to a string.
        let Some(rest) = line.strip_prefix("Clip::") else {
            continue;
        };
        let Some((_, tail)) = rest.split_once(" => \"") else {
            continue;
        };
        let Some((name, _)) = tail.split_once('"') else {
            continue;
        };
        out.push(name.to_string());
    }
    out
}

/// Every clip the code asks for is a clip the shipped rig has.
#[test]
fn every_clip_name_ships_in_the_mannequin() {
    let names = clip_names();
    // Anti-vacuity: a scrape that stopped matching would otherwise pass by
    // finding nothing at all, which is the failure mode this repo names as
    // its worst — a pass it did not earn.
    assert!(
        names.len() >= 6,
        "scraped only {} clip name(s) from anim.rs — the arm shape moved \
         and this gate is looking at nothing",
        names.len()
    );
    for n in &names {
        assert!(
            GLTF.contains(&format!("\"name\":\"{n}\""))
                || GLTF.contains(&format!("\"name\": \"{n}\"")),
            "anim.rs asks for clip {n:?}, which models/mannequin.gltf does \
             not have — the library was re-vendored and a clip renamed. \
             The symptom is a body that stops animating and an error! line \
             nobody reads."
        );
    }
}

/// `Clip::ALL`'s length and every `Rig::nodes` width are the same number.
///
/// Red-proof: change one of the four literals and this fails naming both
/// values; that is the exact edit that otherwise ships a runtime panic.
#[test]
fn the_clip_table_and_the_graph_are_the_same_width() {
    let all = ANIM
        .split("const ALL: [Clip; ")
        .nth(1)
        .and_then(|s| s.split(']').next())
        .and_then(|s| s.trim().parse::<usize>().ok())
        .expect("Clip::ALL's declared width moved — this gate reads it by shape");

    let mut widths = Vec::new();
    for (pat, tail) in [
        ("nodes: [AnimationNodeIndex; ", "]"),
        ("[AnimationNodeIndex::default(); ", "]"),
    ] {
        for chunk in ANIM.split(pat).skip(1) {
            if let Some(n) = chunk
                .split(tail)
                .next()
                .and_then(|s| s.trim().parse::<usize>().ok())
            {
                widths.push(n);
            }
        }
    }
    assert!(
        widths.len() >= 3,
        "found only {} node-array width(s) in anim.rs — the declaration \
         shape moved and this gate is looking at nothing",
        widths.len()
    );
    for w in &widths {
        assert_eq!(
            *w, all,
            "Clip::ALL holds {all} clips and a node array is {w} wide. \
             `Clip::slot()` indexes that array, so the narrow one panics \
             the first time the last clip is played — which for the \
             one-shot swing is the first time another body swings near you."
        );
    }
    assert_eq!(
        clip_names().len(),
        all,
        "Clip::ALL and Clip::name()'s arms disagree on how many clips there are"
    );
}
