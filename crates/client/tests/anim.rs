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
/// The rig `anim.rs` actually loads. **A `.glb` is binary, so this is bytes** —
/// its JSON chunk is UTF-8 at the head of the file, which is all a name search
/// needs. It pointed at `mannequin.gltf` until 2026-08-18 and passed the whole
/// time by coincidence: every name this client asks for came *from* the
/// mannequin by retarget, so the old check could never have caught the one
/// thing it existed to catch — a clip missing from the file the game opens.
const RIG_GLB: &[u8] = include_bytes!("../../../assets/models/stumpy.glb");
/// The two files that carry the flinch from the wire to the body. Text,
/// because both are behind `--features render` and this suite is not — the
/// same hop `the_swing_clip_fits_between_two_swings` makes for its
/// constants.
const BODIES: &str = include_str!("../src/render/bodies.rs");
const FEED: &str = include_str!("../src/render/feed.rs");

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
fn every_clip_name_ships_in_the_rig() {
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
    let json = String::from_utf8_lossy(&RIG_GLB[..RIG_GLB.len().min(400_000)]);
    for n in &names {
        assert!(
            json.contains(&format!("\"name\":\"{n}\""))
                || json.contains(&format!("\"name\": \"{n}\"")),
            "anim.rs asks for clip {n:?}, which models/stumpy.glb does not \
             have — the rig was re-imported and a clip renamed. The symptom \
             is a body that stops animating and an error! line nobody reads."
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

/// **The one-shot has to fit inside the cadence, and that is arithmetic.**
///
/// The sim lets a player swing every `SWING_INTERVAL_TICKS / TICK_HZ`
/// seconds, and a clip longer than that (plus the blend it costs to get
/// into) means every arc is cut off by the next one and no body ever
/// finishes a stroke. This is the check that the numbers still hold.
///
/// ⚠ **The clip is `Sword_Attack`, not `Punch_Cross`, and the reasoning this
/// gate was written under has been inverted.** `Punch_Cross` was picked here
/// as the only candidate short enough to fit unmodified — then the operator
/// saw it on the real body: an oversized head that a punch drives a hand
/// 15 cm inside (0.147 m against a 0.295 m head radius, measured). So the
/// long clip was cut to fit instead (`ci/retarget_anim.py --retime`) rather
/// than a short one being accepted. The inequality below is unchanged and is
/// now satisfied by construction — `SWING_CLIP_S` is derived from it — which
/// is why this stayed a strict `<`: the derivation leaves exactly one
/// resample frame, and a gate that allowed equality would permit a stroke
/// that rounds past the cadence.
///
/// It is deliberately NOT a check that `SWING_CLIP_S` equals the shipped
/// clip's length — that would need the glTF's accessor maxima and would be
/// a hand-kept mirror either way. It is the *inequality the decision rests
/// on*, so switching to a 1.5 s clip fails here rather than in play.
#[test]
fn the_swing_clip_fits_between_two_swings() {
    let cadence = sim_core::gather::SWING_INTERVAL_TICKS as f32 / sim_core::limits::TICK_HZ as f32;
    // The constants are read out of the source rather than linked, because
    // this suite is not behind the `render` feature and `anim.rs` is.
    let read = |name: &str| -> f32 {
        ANIM.split(&format!("{name}: f32 = "))
            .nth(1)
            .and_then(|s| s.split(';').next())
            .and_then(|s| s.trim().parse::<f32>().ok())
            .unwrap_or_else(|| panic!("{name} is not declared as a plain f32 literal any more"))
    };
    let clip = read("pub const SWING_CLIP_S");
    let blend = read("pub const ANIM_BLEND_S");
    assert!(
        clip + blend < cadence,
        "the swing clip is {clip} s and the blend {blend} s, but the sim \
         allows a swing every {cadence:.3} s — every arc would be cut off by \
         the next one and no body would finish a stroke. The clip is cut to \
         fit at import; re-cut it with ci/retarget_anim.py --retime."
    );
}

/// **The flinch is three call sites and every one of them is a deletion
/// away from a silently correct-looking client.**
///
/// `client-core` proves the victim survives the wire and `anim.rs`'s units
/// prove the ranking, but between them sit two lines in `bodies.rs` and one
/// filter in `feed.rs` — and dropping any of them leaves a build that
/// compiles, boots, plays every other animation, and simply never flinches.
/// That is the shape `CLAUDE.md`'s single-drain trap warns about: a defect
/// that is a missing call site rather than a wrong value.
///
/// A text scrape because both files are behind `--features render`. The
/// anti-vacuity assertions are the point of the shape — a scrape that
/// stopped matching would otherwise report success.
#[test]
fn the_flinch_is_wired_from_the_feed_to_the_body() {
    // Both arms of `bodies::stream`: the body already drawn, and the body
    // entering AOI on the same frame the blow lands on it. One is easy to
    // forget and it is the one that matters in a fight, because a raider
    // who has just come over the wall is exactly a body that entered AOI.
    let calls = BODIES.matches("feed.hit_victims()").count();
    assert_eq!(
        calls, 2,
        "render/bodies.rs asks the feed for hit victims {calls} time(s) and \
         `stream` has two arms - the missing one is a body that flinches only \
         if it was already on screen"
    );
    assert_eq!(
        BODIES.matches("anim.flinch()").count(),
        2,
        "a hit victim is looked up and then not flinched"
    );

    // And the sentinel stops at the drain. `EV_STRUCT_HIT` shares the
    // hitmarker ring and carries `NO_VICTIM`; handing it on would put an id
    // no body can hold into a list `bodies::stream` searches every frame —
    // harmless today and a trap the moment anything else reads it.
    assert!(
        FEED.contains("NO_VICTIM"),
        "render/feed.rs no longer filters the struct-hit sentinel out of the \
         victim list - a wall is not a body"
    );
}
