//! The grid's edge names are axes, and the axes are the ones they claim.
//!
//! `build.rs`'s `LOC_EDGE_*` and `collide.rs`'s `ColMasks` edge masks were
//! named after compass bearings (`_W`/`_N`) until 2026-08-15. The compass
//! then moved (`DECISIONS.md` 2026-08-15) and every one of those names came
//! to say the opposite of what it addressed, while **every gate in the tree
//! stayed green**: the values did not change (2 and 3), so
//! `test_protocol_golden` saw nothing; the addressing did not change, so
//! `test_replay` saw nothing; the types did not change, so clippy saw
//! nothing. The defect was a name, and a byte-golden is blind to what a
//! field means (`CLAUDE.md` §traps, the positional-payload entry) — blinder
//! still to what one is *called*.
//!
//! So this gate reads the source. Three checks, each red under its own
//! inversion:
//!
//! - **§A** no edge location constant or column mask is named after a
//!   bearing, and neither doc block explains itself in bearings either —
//!   renaming the constant and leaving the sentence that defines it is
//!   `CLAUDE.md`'s half-a-switch shape, and it is how this stayed invisible
//!   the first time.
//! - **§B** the name is not merely stable, it is *true*: `anchor` puts
//!   `LOC_EDGE_XLO` on the cell's low-x plane and `LOC_EDGE_ZLO` on its
//!   low-z plane, and the two are the pair the re-address rule needs.
//! - **§C** the 2026-08-15 rename moved no value, which is the claim that
//!   let it land without a wire bump.
//!
//! §A is a source check by `include_str!` — compile-time, so sim-core's
//! no-I/O wall is untouched (wall 1 governs the crate, and nothing here
//! runs a syscall at all).

use sim_core::build::{anchor, BUILD_CELL_M, LOC_EDGE_XLO, LOC_EDGE_ZLO, LOC_PLANE, LOC_RISER};

const BUILD_RS: &str = include_str!("../src/build.rs");
const COLLIDE_RS: &str = include_str!("../src/collide.rs");

/// The four bearings, as whole words. `LOC_EDGE_XLO` is not a hit — a single
/// letter inside an identifier is not a claim; "the cell's west boundary"
/// is. The suffix check below is what catches the letter form.
const BEARINGS: [&str; 8] = [
    "west",
    "east",
    "north",
    "south",
    "westward",
    "eastward",
    "northward",
    "southward",
];

/// Whole-word, case-insensitive bearing search — byte-wise, because wall 3
/// keeps `String` out of this crate and `to_ascii_lowercase` allocates one.
fn names_a_bearing(text: &str) -> Option<&'static str> {
    let b = text.as_bytes();
    for w in BEARINGS {
        let wb = w.as_bytes();
        if wb.len() > b.len() {
            continue;
        }
        for i in 0..=(b.len() - wb.len()) {
            if (0..wb.len()).any(|k| b[i + k].to_ascii_lowercase() != wb[k]) {
                continue;
            }
            let end = i + wb.len();
            let before_ok = i == 0 || !is_word_byte(b[i - 1]);
            let after_ok = end == b.len() || !is_word_byte(b[end]);
            if before_ok && after_ok {
                return Some(w);
            }
        }
    }
    None
}

fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Within a tenth of a millimetre, written without `f32::abs`.
///
/// Wall 1 restricts sim-core's floats to `+ − × ÷ sqrt min max clamp
/// floor-by-cast` and `clippy.toml` disallows `abs` by name — in the tests
/// too, which is correct: a test is the crate. `max` of the two differences
/// is the same predicate out of the allowed set.
fn near(a: f32, b: f32) -> bool {
    (a - b).max(b - a) < 1e-4
}

/// The contiguous run of `///` lines immediately above the line containing
/// `needle` — the doc block that defines it. Derive attributes sit between
/// the two (`ColMasks` carries four), so they are stepped over rather than
/// treated as the end of the block. Returns a slice of `src`: wall 3 keeps
/// `String` out of the crate, and a borrow is what this wants anyway.
fn doc_block_above<'a>(src: &'a str, needle: &str) -> &'a str {
    let mut lines: Vec<(usize, &str)> = Vec::new();
    let mut off = 0usize;
    for l in src.lines() {
        lines.push((off, l));
        off += l.len() + 1;
    }
    let at = lines
        .iter()
        .position(|(_, l)| l.contains(needle))
        .unwrap_or_else(|| panic!("`{needle}` is not in the source any more"));

    let mut end = at;
    while end > 0 && lines[end - 1].1.trim_start().starts_with("#[") {
        end -= 1;
    }
    let mut start = end;
    while start > 0 {
        let prev = lines[start - 1].1.trim_start();
        if prev.starts_with("///") || prev.starts_with("//!") {
            start -= 1;
        } else {
            break;
        }
    }
    &src[lines[start].0..lines[end].0]
}

// ---------------------------------------------------------------- §A names

/// Every `LOC_EDGE_*` the grid declares is named for an axis, not a bearing.
///
/// Red if the pre-2026-08-15 names come back, or if a *new* edge location
/// lands carrying one: the suffix after `LOC_EDGE_` must not be a compass
/// letter or word. A future `LOC_EDGE_XHI` passes; `LOC_EDGE_S` does not.
#[test]
fn no_edge_location_is_named_after_a_bearing() {
    let mut seen = 0;
    for line in BUILD_RS.lines() {
        let t = line.trim_start();
        let Some(rest) = t.strip_prefix("pub const LOC_EDGE_") else {
            continue;
        };
        let Some(suffix) = rest.split(':').next() else {
            continue;
        };
        let suffix = suffix.trim();
        seen += 1;
        let bad = matches!(
            suffix.to_ascii_uppercase().as_str(),
            "W" | "N" | "E" | "S" | "WEST" | "NORTH" | "EAST" | "SOUTH"
        );
        assert!(
            !bad,
            "LOC_EDGE_{suffix} is named after a compass bearing. The compass \
             moved once already (DECISIONS.md 2026-08-15) and took every one \
             of these names with it; an axis cannot be re-decided out from \
             under the constant."
        );
    }
    assert!(
        seen >= 2,
        "expected at least the two edge locations, found {seen} — this gate \
         reads `pub const LOC_EDGE_`, so a change to that spelling silently \
         empties it"
    );
}

/// The `ColMasks` edge masks are axes too — the same lie, one crate over.
///
/// `walls_xlo`/`walls_zlo`/`doors_xlo`/`doors_zlo`/`shut_xlo`/`shut_zlo` were the
/// original spelling and the judge of pass `20260815-042118-01` found them
/// still standing after the constants were first scoped for rename. Red if
/// any mask field ends in a bearing letter.
#[test]
fn no_column_mask_is_named_after_a_bearing() {
    let body = COLLIDE_RS
        .split_once("pub struct ColMasks {")
        .expect("ColMasks struct is not in collide.rs any more")
        .1
        .split_once("\n}")
        .expect("ColMasks struct has no close brace")
        .0;

    let mut fields = 0;
    for line in body.lines() {
        let t = line.trim_start();
        let Some(rest) = t.strip_prefix("pub ") else {
            continue;
        };
        let Some(name) = rest.split(':').next() else {
            continue;
        };
        let name = name.trim();
        fields += 1;
        let suffix = name.rsplit('_').next().unwrap_or("");
        assert!(
            !matches!(suffix, "w" | "n" | "e" | "s"),
            "ColMasks::{name} is named after a compass bearing; the masks \
             address a cell's low-x / low-z planes and always did"
        );
    }
    assert!(
        fields >= 8,
        "expected the plane/stair/wall/door/shut/solid masks, found {fields} \
         — the struct was reshaped and this gate stopped reading it"
    );
}

/// The half-a-switch check: the doc that *defines* the edges says so in
/// axes, not bearings.
///
/// This is the one that would have caught the 2026-08-15 drift on the day.
/// A rename that leaves the defining sentence behind reads as covered while
/// the prose still lies — `CLAUDE.md` names the shape twice (the mingw
/// linker entry, and the dead-citation warning in its header).
#[test]
fn neither_doc_block_defines_an_edge_in_bearings() {
    for (what, src, needle) in [
        ("build.rs LOC_EDGE", BUILD_RS, "pub const LOC_PLANE"),
        ("collide.rs ColMasks", COLLIDE_RS, "pub struct ColMasks {"),
    ] {
        let doc = doc_block_above(src, needle);
        assert!(
            !doc.is_empty(),
            "{what}'s doc block is empty — the sentence that defines the \
             convention is what this gate exists to hold"
        );
        if let Some(word) = names_a_bearing(doc) {
            panic!(
                "{what}'s doc block explains the edges with the bearing \
                 \"{word}\". The constants are axes; the sentence that \
                 defines them has to be one too, or the rename was half a \
                 switch.\n--- block ---\n{doc}"
            );
        }
    }
}

// ------------------------------------------------------------ §B semantics

/// `LOC_EDGE_XLO` is the low-x plane and `LOC_EDGE_ZLO` the low-z one —
/// the names are true, not merely stable.
///
/// `anchor` is the sim's single source for an address's world point (it is
/// public precisely so the client cannot write a second copy), so pinning it
/// pins what the constants mean everywhere. Red if the two are swapped, or
/// if either stops sitting on its plane.
#[test]
fn each_edge_constant_sits_on_the_plane_its_name_claims() {
    let (cx, cz) = (341u16, 682u16);
    let (x0, z0) = (cx as f32 * BUILD_CELL_M, cz as f32 * BUILD_CELL_M);
    let half = BUILD_CELL_M * 0.5;

    let (ax, az) = anchor(cx, cz, LOC_EDGE_XLO);
    assert!(
        near(ax, x0),
        "LOC_EDGE_XLO anchored at x={ax}, not the low-x plane x={x0}"
    );
    assert!(
        near(az, z0 + half),
        "LOC_EDGE_XLO is not centred along its edge in z"
    );

    let (bx, bz) = anchor(cx, cz, LOC_EDGE_ZLO);
    assert!(
        near(bz, z0),
        "LOC_EDGE_ZLO anchored at z={bz}, not the low-z plane z={z0}"
    );
    assert!(
        near(bx, x0 + half),
        "LOC_EDGE_ZLO is not centred along its edge in x"
    );

    // And the body locations are the cell centre, so the edges are the only
    // two that name a boundary at all.
    for loc in [LOC_PLANE, LOC_RISER] {
        let (px, pz) = anchor(cx, cz, loc);
        assert!(
            near(px, x0 + half) && near(pz, z0 + half),
            "loc {loc} left the cell body"
        );
    }
}

/// The re-address rule, in the sim: a cell's high-x boundary IS the next
/// cell's `LOC_EDGE_XLO`, and only one of the two names it.
///
/// `ui/place.rs` tests this on the client (`a_plus_x_edge_is_addressed_as_
/// the_next_cells_low_x_edge`). It belongs here too — the client's `cx += 1`
/// arm is only correct because the sim's anchor grid is exactly one cell
/// apart, and a client-only gate leaves the sim free to move under it.
#[test]
fn the_next_cells_low_edge_is_this_cells_high_boundary() {
    let (cx, cz) = (341u16, 682u16);

    let here = anchor(cx, cz, LOC_EDGE_XLO).0;
    let next = anchor(cx + 1, cz, LOC_EDGE_XLO).0;
    assert!(
        near(next - here, BUILD_CELL_M),
        "x edges are {} apart, not one cell",
        next - here
    );
    assert!(
        near(next, (cx + 1) as f32 * BUILD_CELL_M),
        "cell {cx}'s high-x boundary is not cell {}'s low-x edge",
        cx + 1
    );

    let here = anchor(cx, cz, LOC_EDGE_ZLO).1;
    let next = anchor(cx, cz + 1, LOC_EDGE_ZLO).1;
    assert!(
        near(next - here, BUILD_CELL_M),
        "z edges are {} apart, not one cell",
        next - here
    );
}

// ---------------------------------------------------------------- §C values

/// The 2026-08-15 rename moved no value.
///
/// This is the whole reason it could land without a wire bump: `loc` rides
/// the place/deploy/use requests as a raw `u8` and `protocol/src/goldens.rs`
/// pins those bytes. If either number moves, `test_protocol_golden` reddens
/// too — this states the claim in the crate that owns it, so the next reader
/// does not have to infer it from a golden staying quiet.
#[test]
fn the_rename_moved_no_value() {
    assert_eq!(LOC_PLANE, 0);
    assert_eq!(LOC_RISER, 1);
    assert_eq!(LOC_EDGE_XLO, 2, "the wire's edge code, unchanged since v0");
    assert_eq!(LOC_EDGE_ZLO, 3, "the wire's edge code, unchanged since v0");
}
