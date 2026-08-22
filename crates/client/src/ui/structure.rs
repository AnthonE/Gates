//! Which built thing `L`, `U`, `R` and the raid verb are pointed at.
//!
//! Separate from [`super::interact`] because the two picks disagree about
//! every term. `E` ranks an aim radius against a nearby rank over the DEPLOY
//! records and measures to a cell centre; these four address a **structure**
//! — either store — and measure to the piece ANCHOR, which is the corner
//! `build.rs`'s own reach checks use. Folding them would have to pick one
//! metric and would then advertise a verb on the wrong terms.
//!
//! ## The store bit is the trap
//!
//! `CLAUDE.md`'s positional-payload entry, in its sharpest form: **a built
//! piece and a deployable can share one address exactly.** `place_deploy`
//! requires the doorway piece at the *identical* `(cx, cz, level, loc)`, so a
//! door and its doorway are the same four numbers and the leading store bit
//! is the only thing that tells them apart. Send the wrong bit and the
//! address is still valid, every field is still in range, the encoder is
//! untouched and no golden moves — the server just mends the other thing, or
//! answers "not damaged" for a wall the player is watching burn.
//!
//! So [`Target`] carries [`Store`] as a typed enum rather than a `bool`, and
//! the call site converts once, at the encoder, where the argument is named.

use sim_core::build::{
    anchor, shape_has_facing, soft_side, BuildContent, PieceRec, BUILD_REACH_M, MAT_METAL,
};
use sim_core::deploy::{DeployContent, DeployRec};

/// Which store an address names — `encode_action_repair`'s leading argument
/// and `encode_action_throw`'s.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Store {
    Piece,
    Deploy,
}

impl Store {
    /// The wire's bit. The ONE place this conversion happens, so a
    /// transposition has one site to be wrong at instead of five.
    pub fn is_deploy(self) -> bool {
        self == Store::Deploy
    }
}

/// A resolved structure address.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Target {
    pub store: Store,
    pub cx: u16,
    pub cz: u16,
    pub level: u8,
    pub loc: u8,
    pub row: u8,
    /// The damage band the server last stated for this address
    /// (`build::damage_band`, 0 = untouched), so a prompt can say whether
    /// repair has anything to buy.
    ///
    /// **This was `hp`/`hp_max` until wire v44, and the pair was a lie.**
    /// Piece and deploy hp were never on the wire — `write_piece_rec` wrote
    /// address and row and stopped — so the mirror's `hp` was a permanent 0
    /// and `damaged()` answered TRUE for every structure in the world. The
    /// one thing it gates, `hammer.rs`'s "not damaged" refusal, was
    /// therefore unreachable: every repair swing at an intact wall took a
    /// server round trip to come back refused, by a client-side check that
    /// had never once fired. A band is what the wire carries now, and it is
    /// all this needs — `damaged()` is a comparison against 0.
    pub dmg: u8,
    /// The row's baked maximum hp, 0 until that def row has dripped in.
    /// Kept because the repair prompt prices against it; it is content the
    /// client already holds, not a per-piece fact off the wire.
    pub hp_max: u16,
    /// Which face of a sided piece the player stands on — `Some(true)` is
    /// the SOFT side, `Some(false)` the hard one, `None` a shape with no
    /// sides (or the other store). Computed by `sim_core::build::soft_side`,
    /// the same comparison `combat::raid` prices the swing with, so the
    /// label can never disagree with the bill (hard/soft v0).
    pub side: Option<bool>,
}

impl Target {
    /// Is there damage to pay for? `build.rs` refuses a repair on an intact
    /// piece (`REFUSE_B_INTACT`), so a client that offered one would be
    /// advertising a refusal.
    pub fn damaged(&self) -> bool {
        self.dmg > 0
    }
}

/// The nearest structure within `BUILD_REACH_M` of the feet, over BOTH
/// stores.
///
/// Reach is measured to the anchor via `sim_core::build::anchor` — the sim's
/// own function, not a copy — because that is what `repair` and `upgrade`
/// gate on. Quantize both sides: the client picks inside the radius the
/// server will accept.
///
/// A tie between the two stores at one address goes to the **deployable**,
/// which is the door rather than its doorway. That is the thing a player is
/// looking at and the thing a raider means; the doorway behind it is still
/// reachable by breaking the door first.
pub fn nearest(
    at: (f32, f32),
    pieces: &[PieceRec],
    piece_defs: &BuildContent,
    piece_have: u16,
    deploys: &[DeployRec],
    deploy_defs: &DeployContent,
    deploy_have: u16,
) -> Option<Target> {
    let mut best: Option<Target> = None;
    let mut best_d2 = BUILD_REACH_M * BUILD_REACH_M;

    for rec in pieces {
        let (ax, az) = anchor(rec.cx, rec.cz, rec.loc);
        let (dx, dz) = (ax - at.0, az - at.1);
        let d2 = dx * dx + dz * dz;
        if d2 > best_d2 {
            continue;
        }
        best_d2 = d2;
        let sided = (rec.row as u16) < piece_have
            && shape_has_facing(piece_defs.pieces[rec.row as usize].shape);
        best = Some(Target {
            store: Store::Piece,
            cx: rec.cx,
            cz: rec.cz,
            level: rec.level,
            loc: rec.loc,
            row: rec.row,
            dmg: rec.dmg,
            hp_max: if (rec.row as u16) < piece_have {
                piece_defs.pieces[rec.row as usize].hp
            } else {
                0
            },
            side: sided.then(|| soft_side(rec, at.0, at.1)),
        });
    }

    for rec in deploys {
        let (ax, az) = anchor(rec.cx, rec.cz, rec.loc);
        let (dx, dz) = (ax - at.0, az - at.1);
        let d2 = dx * dx + dz * dz;
        // `<=` rather than `<`: a tie goes to the deployable. See the header.
        if d2 > best_d2 {
            continue;
        }
        best_d2 = d2;
        best = Some(Target {
            store: Store::Deploy,
            cx: rec.cx,
            cz: rec.cz,
            level: rec.level,
            loc: rec.loc,
            row: rec.row,
            dmg: rec.dmg,
            hp_max: if (rec.row as u16) < deploy_have {
                deploy_defs.defs[rec.row as usize].hp
            } else {
                0
            },
            side: None,
        });
    }

    best
}

/// Which deployable row a held item places, if any.
///
/// **This is what makes a box placeable at all.** `DeployDef::item` is the
/// crafted item placement consumes one unit of, so "the thing in my hand" is
/// a row lookup rather than a menu — which is how the reference does it, and
/// it needs no new wheel. `None` means the held item is not a deployable, or
/// the def table has not dripped far enough to say.
pub fn row_for_item(defs: &DeployContent, have: u16, item: u16) -> Option<u8> {
    if item == 0 {
        return None;
    }
    (0..have.min(defs.def_count))
        .find(|&i| defs.defs[i as usize].item == item)
        .map(|i| i as u8)
}

/// The material one rung above this piece's, or `None` at the top of the
/// ladder.
///
/// The sim refuses an upgrade that is not to the next rung
/// (`REFUSE_B_TIER`), so picking the rung client-side is what turns `U` into
/// one press instead of a guess between three.
pub fn next_material(piece_defs: &BuildContent, have: u16, row: u8) -> Option<u8> {
    if (row as u16) >= have {
        return None;
    }
    let current = piece_defs.pieces[row as usize].material;
    if current >= MAT_METAL {
        return None;
    }
    Some(current + 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim_core::build::{damage_band, PieceDef, LOC_EDGE_XLO, LOC_PLANE, MAT_STONE, MAT_WOOD};
    use sim_core::deploy::{DeployDef, ARCH_DOOR};
    use sim_core::limits::MAX_PIECE_COSTS;

    fn piece_table(materials: &[u8]) -> (BuildContent, u16) {
        let mut c = BuildContent::EMPTY;
        for (i, &material) in materials.iter().enumerate() {
            c.pieces[i] = PieceDef {
                shape: 1,
                material,
                hp: 500,
                n_costs: 0,
                costs: [(0, 0); MAX_PIECE_COSTS],
            };
        }
        c.piece_count = materials.len() as u16;
        (c, materials.len() as u16)
    }

    fn deploy_table() -> (DeployContent, u16) {
        let mut c = DeployContent::EMPTY;
        c.defs[0] = DeployDef {
            arch: ARCH_DOOR,
            hp: 200,
            ..DeployDef::INERT
        };
        c.def_count = 1;
        (c, 1)
    }

    /// A piece still stated in hp and banded the way the SERVER bands it
    /// (wire v44) — against `piece_table`'s own 500, so the fixture and the
    /// def it is measured against cannot drift apart.
    fn piece(cx: u16, cz: u16, loc: u8, hp: u16) -> PieceRec {
        PieceRec {
            cx,
            cz,
            level: 0,
            loc,
            row: 0,
            facing: 0,
            hp,
            uh: 0,
            dmg: damage_band(hp, 500),
            plate: 0,
        }
    }

    fn door(cx: u16, cz: u16, loc: u8, hp: u16) -> DeployRec {
        DeployRec {
            cx,
            cz,
            level: 0,
            loc,
            row: 0,
            owner: 1,
            hp,
            uh: 0,
            open: false,
            locked: false,
            has_lock: false,
            // `deploy_table`'s own 200 — see `piece`.
            dmg: damage_band(hp, 200),
        }
    }

    #[test]
    fn nothing_in_reach_is_no_target() {
        let (pd, ph) = piece_table(&[MAT_WOOD]);
        let (dd, dh) = deploy_table();
        let far = [piece(200, 200, LOC_PLANE, 10)];
        assert!(nearest((0.0, 0.0), &far, &pd, ph, &[], &dd, dh).is_none());
    }

    /// The trap, stated as a test: one address, two stores, and the pick must
    /// name the door rather than the doorway behind it.
    #[test]
    fn a_door_and_its_doorway_share_an_address_and_the_door_wins() {
        let (pd, ph) = piece_table(&[MAT_WOOD]);
        let (dd, dh) = deploy_table();
        let addr = (5u16, 5u16, LOC_EDGE_XLO);
        let pieces = [piece(addr.0, addr.1, addr.2, 100)];
        let deploys = [door(addr.0, addr.1, addr.2, 100)];
        let (ax, az) = anchor(addr.0, addr.1, addr.2);
        let t = nearest((ax, az), &pieces, &pd, ph, &deploys, &dd, dh).expect("in reach");
        assert_eq!(t.store, Store::Deploy);
        assert!(t.store.is_deploy(), "the wire bit must follow the store");
        // Same four numbers either way — which is exactly why the bit is the
        // only thing distinguishing them.
        assert_eq!((t.cx, t.cz, t.loc), addr);
    }

    #[test]
    fn hp_max_comes_from_the_right_table_for_each_store() {
        let (pd, ph) = piece_table(&[MAT_WOOD]);
        let (dd, dh) = deploy_table();
        let (ax, az) = anchor(5, 5, LOC_PLANE);
        let t = nearest(
            (ax, az),
            &[piece(5, 5, LOC_PLANE, 250)],
            &pd,
            ph,
            &[],
            &dd,
            dh,
        )
        .unwrap();
        assert_eq!((t.dmg, t.hp_max), (damage_band(250, 500), 500));
        assert!(t.damaged());

        let (ax, az) = anchor(6, 6, LOC_EDGE_XLO);
        let t = nearest(
            (ax, az),
            &[],
            &pd,
            ph,
            &[door(6, 6, LOC_EDGE_XLO, 200)],
            &dd,
            dh,
        )
        .unwrap();
        assert_eq!((t.dmg, t.hp_max), (0, 200));
        assert!(!t.damaged(), "an intact door has nothing to buy");
    }

    /// A row the def table has not dripped reports max 0 — and since wire
    /// v44 it still reports **damage**, which is the improvement rather than
    /// a regression.
    ///
    /// This asserted `!damaged()` until 2026-08-16, and that was a property
    /// of the arithmetic rather than of the system: `damaged` divided hp by
    /// a maximum the client did not have, so an undripped row could only
    /// answer false. The band is the SERVER's division now — it knows every
    /// maximum — so it survives a def table that has not arrived, and a
    /// player who walks into a raided base before the drip finishes sees the
    /// damage rather than a clean wall.
    ///
    /// **What has not changed is the behaviour that mattered**: `hammer`'s
    /// repair guard is `!damaged() && hp_max > 0`, so an undripped row still
    /// sends and lets the sim answer. That is gated where it belongs, on
    /// `hammer::act` itself (`tests/ui.rs` §K), not inferred from this.
    #[test]
    fn an_undripped_row_reports_no_maximum() {
        let (pd, _) = piece_table(&[MAT_WOOD]);
        let (dd, dh) = deploy_table();
        let (ax, az) = anchor(5, 5, LOC_PLANE);
        let t = nearest(
            (ax, az),
            &[piece(5, 5, LOC_PLANE, 100)],
            &pd,
            0,
            &[],
            &dd,
            dh,
        )
        .unwrap();
        assert_eq!(t.hp_max, 0, "an undripped row has no known maximum");
        // The fixture bands against `piece_table`'s own 500, which is what
        // the server would have sent: hp 100 of 500 is heavy damage, and it
        // reaches the client whole even with the def table empty.
        assert_eq!(t.dmg, damage_band(100, 500));
        assert!(t.damaged(), "the server's band survives an undripped table");
    }

    #[test]
    fn reach_is_the_sims_reach_measured_to_the_anchor() {
        let (pd, ph) = piece_table(&[MAT_WOOD]);
        let (dd, dh) = deploy_table();
        let pieces = [piece(9, 9, LOC_EDGE_XLO, 10)];
        let (ax, az) = anchor(9, 9, LOC_EDGE_XLO);
        assert!(nearest(
            (ax, az - BUILD_REACH_M + 0.2),
            &pieces,
            &pd,
            ph,
            &[],
            &dd,
            dh
        )
        .is_some());
        assert!(nearest(
            (ax, az - BUILD_REACH_M - 0.2),
            &pieces,
            &pd,
            ph,
            &[],
            &dd,
            dh
        )
        .is_none());
    }

    #[test]
    fn the_ladder_climbs_once_and_stops_at_metal() {
        let (pd, ph) = piece_table(&[MAT_WOOD, MAT_STONE, MAT_METAL]);
        assert_eq!(next_material(&pd, ph, 0), Some(MAT_STONE));
        assert_eq!(next_material(&pd, ph, 1), Some(MAT_METAL));
        assert_eq!(next_material(&pd, ph, 2), None);
        // An undripped row has no known material, so no rung either.
        assert_eq!(next_material(&pd, 0, 0), None);
    }

    #[test]
    fn a_held_item_finds_the_row_that_places_it() {
        let mut c = DeployContent::EMPTY;
        c.defs[0] = DeployDef {
            arch: ARCH_DOOR,
            item: 41,
            ..DeployDef::INERT
        };
        c.defs[1] = DeployDef {
            arch: sim_core::deploy::ARCH_BOX,
            item: 42,
            ..DeployDef::INERT
        };
        c.def_count = 2;
        assert_eq!(row_for_item(&c, 2, 42), Some(1));
        assert_eq!(row_for_item(&c, 2, 41), Some(0));
        assert_eq!(row_for_item(&c, 2, 99), None);
        // Item 0 is "empty slot", never a deployable.
        assert_eq!(row_for_item(&c, 2, 0), None);
        // A row past what has dripped is not searched — its `item` is
        // INERT's and would match the wrong hand.
        assert_eq!(row_for_item(&c, 1, 42), None);
    }
}
