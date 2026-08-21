//! The stored plate: a base's floor is chosen once and everything after
//! builds relative to it (build plate v1, `build::plate_for`).
//!
//! **What this replaces.** Before it, a column's floor was
//! `terrain_band(seed, cell)` and nothing else — so a base was whatever shape
//! the ground under it happened to be. Measured on the shipped island the day
//! this landed: of 5 726 buildable 4×4 footprints, **658 were flat** — 11.5%.
//! 45.8% stepped half a metre across themselves, 25.7% a whole metre, and the
//! worst stepped 10 m. `base_lattice.rs` is the gate on the *terrain* rule,
//! which still decides where a base STARTS; this is the gate on the rule that
//! decides where the rest of it goes.
//!
//! Every assertion drives `build::place` — the sim's own verb, with content,
//! reach, cost and support all live — rather than calling `plate_for`
//! directly. The rule is only worth anything if the verb consults it in the
//! right order, and a test that called the rule alone would pass with the
//! call site deleted.

use sim_core::build::{
    place, plate_for, terrain_band, BuildContent, Pieces, BUILD_BASE_Q_M, LOC_EDGE_XLO, LOC_PLANE,
    PLATE_RISE_MAX_BANDS, PLATE_SINK_MAX_BANDS, REFUSE_B_PLATE_HIGH, REFUSE_B_PLATE_LOW,
};
use sim_core::deploy::Deploys;
use sim_core::gather::ItemStack;
use sim_core::movement::Body;
use sim_core::terrain;
use sim_core::world::{EventQueue, Player, EV_BUILD_REFUSED, EV_PIECE_PLACED};

fn hv(seed: u64) -> &'static terrain::Haven {
    use std::cell::RefCell;
    thread_local! {
        static CACHE: RefCell<Vec<(u64, &'static terrain::Haven)>> =
            const { RefCell::new(Vec::new()) };
    }
    if let Some(h) = CACHE.with(|c| c.borrow().iter().find(|(s, _)| *s == seed).map(|&(_, h)| h)) {
        return h;
    }
    let h: &'static terrain::Haven = Box::leak(Box::new(terrain::haven(seed)));
    CACHE.with(|c| c.borrow_mut().push((seed, h)));
    h
}

const SEED: u64 = 20260731;
/// `ghost.rs`' and `deploy.rs`' own buildable cell.
const CX: u16 = 341;
const CZ: u16 = 341;
/// Row 0 of `BuildContent::probe_fixture` — a twig foundation.
const ROW_FOUNDATION: u16 = 0;
/// Row 1 — a twig wall.
const ROW_WALL: u16 = 1;

struct Rig {
    bc: BuildContent,
    pieces: Pieces,
    deploys: Deploys,
}

impl Rig {
    fn new() -> Self {
        Self {
            bc: BuildContent::probe_fixture(),
            pieces: Pieces::new(),
            deploys: Deploys::new(),
        }
    }

    /// Drive the sim's own verb from a body standing at the address, with a
    /// full purse. `Err(code)` on a refusal.
    fn place(&mut self, row: u16, cx: u16, cz: u16, level: u8, loc: u8) -> Result<(), u32> {
        let (ax, az) = sim_core::build::anchor(cx, cz, loc);
        let mut p = Player {
            id: 7,
            active: true,
            body: Body::at(SEED, hv(SEED), ax, az),
            ..Player::default()
        };
        for (i, slot) in p.inv.iter_mut().enumerate().take(2) {
            *slot = ItemStack {
                item: i as u16,
                count: 99,
                cond: 0,
            };
        }
        let mut ev = EventQueue::default();
        place(
            SEED,
            hv(SEED),
            &self.bc,
            &self.deploys,
            &mut self.pieces,
            &mut p,
            0,
            row,
            cx,
            cz,
            level,
            loc,
            &mut ev,
        );
        match ev.entries().iter().find(|e| e.code == EV_BUILD_REFUSED) {
            Some(e) => Err(e.b),
            None => {
                assert!(
                    ev.entries().iter().any(|e| e.code == EV_PIECE_PLACED),
                    "the verb neither placed nor refused"
                );
                Ok(())
            }
        }
    }

    fn plate_at(&self, cx: u16, cz: u16) -> Option<i8> {
        self.pieces
            .entries()
            .iter()
            .find(|r| r.cx == cx && r.cz == cz)
            .map(|r| r.plate)
    }

    fn floor_at(&self, cx: u16, cz: u16) -> f32 {
        sim_core::build::column_floor_y(SEED, hv(SEED), cx, cz, self.plate_at(cx, cz).unwrap_or(0))
    }
}

/// A cell whose terrain band differs from `(cx, cz)`'s by `want`, found by
/// walking out along +x — the fixture for the latch cases below.
fn neighbour_stepping(cx: u16, cz: u16, want: i32) -> Option<u16> {
    let here = terrain_band(SEED, hv(SEED), cx, cz);
    (1..600u16).find(|&k| terrain_band(SEED, hv(SEED), cx + k, cz) - here == want)
}

// ---------------------------------------------------------------------------
// The rule
// ---------------------------------------------------------------------------

/// The first foundation of a base takes its own ground, and the second takes
/// the FIRST one's floor — not its own.
///
/// This is the whole feature in one assertion, and it is stated as an
/// identity: two columns in one base are bit-equal flush, on any ground,
/// rather than flush-when-the-terrain-agrees.
#[test]
fn a_neighbour_latches_to_the_floor_that_is_already_there() {
    // A pair the terrain would have put on two different floors — so a plate
    // 0 answer would be a false pass.
    let step = neighbour_stepping(CX, CZ, 1).or_else(|| neighbour_stepping(CX, CZ, -1));
    let Some(k) = step else {
        panic!("the fixture needs a stepped neighbour on this seed")
    };
    let (nx, nz) = (CX + k, CZ);

    let mut r = Rig::new();
    r.place(ROW_FOUNDATION, CX, CZ, 0, LOC_PLANE)
        .expect("first");
    let first = r.floor_at(CX, CZ);
    assert_eq!(
        r.plate_at(CX, CZ),
        Some(0),
        "the first foundation of a base takes its own ground"
    );

    // Walk the plate out to the stepped cell one foundation at a time, which
    // is what a player does.
    for x in CX + 1..=nx {
        r.place(ROW_FOUNDATION, x, nz, 0, LOC_PLANE)
            .unwrap_or_else(|e| panic!("cell {x} refused with {e}"));
    }
    assert_eq!(
        r.floor_at(nx, nz),
        first,
        "the far end of the base is not on the floor the first foundation set"
    );
    assert_ne!(
        terrain_band(SEED, hv(SEED), nx, nz),
        terrain_band(SEED, hv(SEED), CX, CZ),
        "the fixture's two cells must disagree about terrain, or this proves nothing"
    );
    assert_ne!(
        r.plate_at(nx, nz),
        Some(0),
        "a stepped cell that latched must carry a non-zero plate"
    );
}

/// Every piece in a column shares the column's floor, whatever it is — a wall
/// on a stilted foundation stands on that foundation, not on the dirt below.
#[test]
fn a_wall_stands_on_the_floor_under_it_and_not_on_the_ground() {
    let Some(k) = neighbour_stepping(CX, CZ, -1) else {
        panic!("the fixture needs a neighbour one band lower")
    };
    let mut r = Rig::new();
    r.place(ROW_FOUNDATION, CX, CZ, 0, LOC_PLANE)
        .expect("first");
    for x in CX + 1..=CX + k {
        r.place(ROW_FOUNDATION, x, CZ, 0, LOC_PLANE)
            .unwrap_or_else(|e| panic!("cell {x} refused with {e}"));
    }
    let (nx, nz) = (CX + k, CZ);
    let floor = r.floor_at(nx, nz);
    r.place(ROW_WALL, nx, nz, 0, LOC_EDGE_XLO).expect("a wall");
    let wall = r
        .pieces
        .entries()
        .iter()
        .find(|p| p.cx == nx && p.cz == nz && p.loc == LOC_EDGE_XLO)
        .expect("the wall is in the store");
    assert_eq!(
        sim_core::build::column_floor_y(SEED, hv(SEED), nx, nz, wall.plate),
        floor,
        "the wall bases itself somewhere other than the floor it stands on"
    );
}

/// The stilt limit refuses rather than stretching, and it refuses BY NAME:
/// ground that falls away past `PLATE_RISE_MAX_BANDS` gets
/// `REFUSE_B_PLATE_HIGH`, ground that climbs past `PLATE_SINK_MAX_BANDS` gets
/// `REFUSE_B_PLATE_LOW`, and both are reached by carrying a plate across real
/// terrain rather than by constructing a step.
///
/// **Marched, not jumped, and the first draft did jump.** It walked out to a
/// cell seven bands down and read `Ok(0)` — correct, because `plate_for`
/// latches to an ORTHOGONAL NEIGHBOUR and an unbuilt gap latches to nothing.
/// Rewriting it as an adjacent pair then failed to find one: a seven-band step
/// between two 3 m cells is a 49° cliff and `foundation_terrain_ok` refuses to
/// start a base on one, so the fixture could not exist. Which is the right
/// answer to both: the limit is not reached by one cliff, it is reached by
/// carrying a floor far enough that the leg under it runs out — and that is
/// what a player does.
#[test]
fn the_ground_running_away_refuses_by_name() {
    // March a plate out from several starts in both x directions and collect
    // which refusals the terrain produces. Every start is buildable ground.
    let mut seen_high = false;
    let mut seen_low = false;
    for start in [CX, CX + 40, CX - 40, CX + 90, CX - 90] {
        for dir in [1i32, -1] {
            let (sx, sz) = (start, CZ);
            let (ax, az) = sim_core::build::anchor(sx, sz, LOC_PLANE);
            if !sim_core::build::foundation_terrain_ok(SEED, hv(SEED), ax, az) {
                continue;
            }
            let mut r = Rig::new();
            if r.place(ROW_FOUNDATION, sx, sz, 0, LOC_PLANE).is_err() {
                continue;
            }
            let mut at = sx as i32;
            loop {
                let next = at + dir;
                if !(0..1024).contains(&next) {
                    break;
                }
                let (nx, nz) = (next as u16, sz);
                // What the rule WANTS here, before the verb is asked.
                let want = plate_for(r.pieces.cols(), SEED, hv(SEED), nx, nz);
                match r.place(ROW_FOUNDATION, nx, nz, 0, LOC_PLANE) {
                    Ok(()) => {
                        assert!(want.is_ok(), "the verb took a placement the rule refused");
                        at = next;
                    }
                    Err(e) if e == REFUSE_B_PLATE_HIGH => {
                        // The rule wanted a plate past the stilt ceiling.
                        let over = terrain_band(SEED, hv(SEED), at as u16, sz)
                            + r.plate_at(at as u16, sz).unwrap_or(0) as i32
                            - terrain_band(SEED, hv(SEED), nx, nz);
                        assert!(
                            over > PLATE_RISE_MAX_BANDS,
                            "refused HIGH on a {over}-band lift, which is inside the limit"
                        );
                        seen_high = true;
                        break;
                    }
                    Err(e) if e == REFUSE_B_PLATE_LOW => {
                        let under = terrain_band(SEED, hv(SEED), nx, nz)
                            - terrain_band(SEED, hv(SEED), at as u16, sz)
                            - r.plate_at(at as u16, sz).unwrap_or(0) as i32;
                        assert!(
                            under > PLATE_SINK_MAX_BANDS,
                            "refused LOW on a {under}-band cut, which is inside the limit"
                        );
                        seen_low = true;
                        break;
                    }
                    // Anything else — bad ground, out of the grid — ends this
                    // march without a verdict, which is fine: another start
                    // supplies the case.
                    Err(_) => break,
                }
            }
        }
    }
    assert!(
        seen_high && seen_low,
        "the shipped island produced HIGH={seen_high} LOW={seen_low}; \
both refusals must be reachable by building on it"
    );
}

/// The refusal arrives through `place`, in the sim's own order — after
/// terrain and before cost. A rule the verb does not consult is not a rule.
#[test]
fn the_verb_itself_refuses_the_step() {
    // March a base toward rising ground until the verb says no; the code it
    // says no with must be a plate refusal and not something else.
    let mut r = Rig::new();
    r.place(ROW_FOUNDATION, CX, CZ, 0, LOC_PLANE)
        .expect("first");
    let mut said = None;
    for x in CX + 1..CX + 400 {
        match r.place(ROW_FOUNDATION, x, CZ, 0, LOC_PLANE) {
            Ok(()) => continue,
            Err(e) => {
                said = Some(e);
                break;
            }
        }
    }
    let said = said.expect("400 cells of one plate is not a base, it is a bug");
    assert!(
        said == REFUSE_B_PLATE_HIGH || said == REFUSE_B_PLATE_LOW,
        "the march stopped on {said}, which is not a plate refusal"
    );
}

/// A plate is a whole number of lattice quanta and nothing else — the
/// property `BUILD_BASE_Q_M` exists for, carried onto the stored height.
#[test]
fn a_plate_is_a_whole_number_of_bands() {
    let mut r = Rig::new();
    r.place(ROW_FOUNDATION, CX, CZ, 0, LOC_PLANE)
        .expect("first");
    for x in CX + 1..CX + 12 {
        if r.place(ROW_FOUNDATION, x, CZ, 0, LOC_PLANE).is_err() {
            break;
        }
    }
    let first = r.floor_at(CX, CZ);
    for rec in r.pieces.entries() {
        let y = sim_core::build::column_floor_y(SEED, hv(SEED), rec.cx, rec.cz, rec.plate);
        assert_eq!(y, first, "one base, two floors");
        assert!(
            (rec.plate as i32) <= PLATE_RISE_MAX_BANDS
                && (rec.plate as i32) >= -PLATE_SINK_MAX_BANDS,
            "a stored plate is outside the limits that produced it"
        );
        // Bit-equal, not close: the whole point of the quantum.
        let terrain = sim_core::build::band_y(terrain_band(SEED, hv(SEED), rec.cx, rec.cz));
        assert_eq!(y - terrain, rec.plate as f32 * BUILD_BASE_Q_M);
    }
}

/// A column that empties re-derives from terrain rather than inheriting a
/// plate nothing stands on any more — the property `ColIndex`'s
/// backward-shift deletion gives for free, asserted so it stays free.
#[test]
fn an_emptied_column_forgets_its_plate() {
    let Some(k) = neighbour_stepping(CX, CZ, -1) else {
        panic!("the fixture needs a lower neighbour")
    };
    let mut r = Rig::new();
    r.place(ROW_FOUNDATION, CX, CZ, 0, LOC_PLANE)
        .expect("first");
    for x in CX + 1..=CX + k {
        r.place(ROW_FOUNDATION, x, CZ, 0, LOC_PLANE)
            .unwrap_or_else(|e| panic!("cell {x} refused with {e}"));
    }
    let (nx, nz) = (CX + k, CZ);
    let stilted = r.plate_at(nx, nz).expect("the far cell is built");
    assert_ne!(stilted, 0, "the fixture's far cell must be stilted");

    // Take the whole base away and the far cell is bare ground again.
    let mut empty = Pieces::new();
    core::mem::swap(&mut r.pieces, &mut empty);
    assert_eq!(
        plate_for(r.pieces.cols(), SEED, hv(SEED), nx, nz),
        Ok(0),
        "an empty column still remembers a plate"
    );
}

/// The stilt ceiling is exactly one storey — the derivation
/// `PLATE_RISE_MAX_BANDS` states, held against the two constants it is
/// derived from rather than against the number it currently is.
#[test]
fn the_stilt_ceiling_is_one_storey() {
    assert_eq!(
        PLATE_RISE_MAX_BANDS as f32 * BUILD_BASE_Q_M,
        sim_core::build::LEVEL_H_M,
        "the stilt ceiling stopped being a storey"
    );
    // ...and a plate is measured in the same quanta the lattice steps in,
    // so a stilt lands on a band a neighbouring plate could also land on.
    assert_eq!(sim_core::build::LEVEL_H_M % BUILD_BASE_Q_M, 0.0);
}
