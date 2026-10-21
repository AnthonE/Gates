//! Freeform building (W3): squares and equilateral triangles at 30° steps,
//! in a building turned any way, placed, borne, collided with and brought
//! down on the building's own lattice.
//!
//! The grid's own suites hold the grid; this one holds what the grid could
//! not build: Rust's honeycomb (a square core ringed with triangles, walls
//! on their outer sides), a hexagon of six triangles, and a base that faces
//! no axis at all.

use sim_core::build::{self, *};
use sim_core::collide;
use sim_core::deploy::DeployContent;
use sim_core::lattice::LPt;
use sim_core::limits::MAX_REMOVALS_PER_TICK;
use sim_core::movement::{self, Body};
use sim_core::sock::{beside, PieceAddr, Sock};
use sim_core::world::{EventQueue, Player, World};

const SEED: u64 = 20260731;

const FOUNDATION: u16 = 0;
const WALL: u16 = 1;
const FLOOR: u16 = 2;
const DOORWAY: u16 = 3;
const TRI_FOUNDATION: u16 = 4;
const TRI_FLOOR: u16 = 5;
const TRI_SPIRAL: u16 = 6;

fn fixture() -> (Box<World>, Player, (f32, f32)) {
    let mut w = Box::new(World::new(SEED));
    w.build = BuildContent::EMPTY;
    let rows = [
        SHAPE_FOUNDATION,
        SHAPE_WALL,
        SHAPE_FLOOR,
        SHAPE_DOORWAY,
        SHAPE_TRI_FOUNDATION,
        SHAPE_TRI_FLOOR,
        SHAPE_STAIRS_TRI_SPIRAL,
    ];
    w.build.piece_count = rows.len() as u16;
    for (row, &shape) in rows.iter().enumerate() {
        w.build.pieces[row] = PieceDef {
            shape,
            material: MAT_TWIG,
            hp: 100,
            ..PieceDef::INERT
        };
    }
    // Flat ground for a 12 m base, searched outward from the middle.
    let at = (0..400)
        .flat_map(|r| (-r..=r).flat_map(move |dz| (-r..=r).map(move |dx| (dx, dz, r))))
        .filter(|&(dx, dz, r)| {
            dx * dx + dz * dz <= r * r && (dx * dx + dz * dz) > (r - 1) * (r - 1)
        })
        .map(|(dx, dz, _)| (2048.0 + dx as f32 * 3.0, 2048.0 + dz as f32 * 3.0))
        .find(|&(x, z)| {
            (-2..=2).all(|i| {
                (-2..=2).all(|j| {
                    build::foundation_terrain_ok(
                        SEED,
                        &w.haven,
                        x + i as f32 * 3.0,
                        z + j as f32 * 3.0,
                    )
                })
            })
        })
        .expect("flat ground");
    let p = Player {
        id: 7,
        active: true,
        body: Body::at(SEED, &w.haven, at.0, at.1),
        ..Player::default()
    };
    (w, p, at)
}

fn put(w: &mut World, p: &mut Player, row: u16, site: &Site) -> Result<(), u32> {
    let px = p.body.qx as f32 * movement::POS_XZ_Q;
    let pz = p.body.qz as f32 * movement::POS_XZ_Q;
    let at = check_site(
        SEED,
        &w.haven,
        &w.build,
        &w.pieces,
        (px, pz),
        &p.inv,
        row,
        site,
        |_, _| false,
    )?;
    let mut ev = EventQueue::default();
    if place_site(&w.build, &mut w.pieces, p, 0, row, site, at, &mut ev) {
        Ok(())
    } else {
        Err(REFUSE_B_FULL)
    }
}

fn on(bld: u16, level: u8, sock: Sock) -> Site {
    Site {
        addr: PieceAddr { bld, level, sock },
        mint: None,
        grid: None,
    }
}

/// The square foundation a base starts from, its middle on `at`, turned
/// `yaw`: the origin is its first corner, so step back half a side along
/// each of the frame's axes.
fn core(w: &World, at: (f32, f32), yaw: u16) -> Site {
    let probe = sim_core::frame::Frame::new((0, 0), 0, yaw);
    let (ox, oz) = probe.to_world(LPt::new(3, 0, 0, 3));
    let origin = (((at.0 - ox) * 100.0) as i32, ((at.1 - oz) * 100.0) as i32);
    free_site(SEED, &w.haven, &w.pieces, origin, yaw, false, 0).unwrap()
}

/// Rust's honeycomb on a base turned 27.5°: a square core, a triangle on
/// each side, walls round the outside — a doorway in one — and triangle
/// floors on those walls. Every piece is borne by the lattice and none by
/// the grid.
#[test]
fn a_turned_honeycomb_stands_on_its_own_lattice() {
    let (mut w, mut p, at) = fixture();
    let site = core(&w, at, 5_000);
    put(&mut w, &mut p, FOUNDATION, &site).expect("the core");
    let bld = site.addr.bld;
    let sq = site.addr.sock;
    assert_eq!(w.pieces.buildings().live(), 1);
    assert!(
        build::grid_origin(w.pieces.frame(bld).unwrap()).is_none(),
        "off the grid"
    );

    // A triangle on each side, outward.
    let (sides, n) = sq.sides();
    let mut tris = Vec::new();
    for e in &sides[..n] {
        let (_, tri) = beside(e, -e.side_of(sq.at));
        put(&mut w, &mut p, TRI_FOUNDATION, &on(bld, 0, tri)).expect("a ring triangle");
        tris.push(tri);
    }
    // Walls on every outer side, the first a doorway.
    let mut outer = Vec::new();
    for t in &tris {
        let (ts, k) = t.sides();
        for e in &ts[..k] {
            if !sides[..n].contains(e) {
                outer.push(*e);
            }
        }
    }
    assert_eq!(outer.len(), 8, "a ring of eight walls");
    for (i, e) in outer.iter().enumerate() {
        let row = if i == 0 { DOORWAY } else { WALL };
        put(&mut w, &mut p, row, &on(bld, 0, *e)).expect("an outer wall");
    }
    // Triangle floors over the ring, each on its two walls.
    for t in &tris {
        put(&mut w, &mut p, TRI_FLOOR, &on(bld, 1, *t)).expect("a triangle floor");
    }
    // The core's floor has no wall under any side: nothing bears it.
    assert_eq!(
        put(&mut w, &mut p, FLOOR, &on(bld, 1, sq)),
        Err(REFUSE_B_SUPPORT)
    );
    assert_eq!(w.pieces.len(), 1 + 4 + 8 + 4);
    // None of it has a grid name.
    assert!(w.pieces.entries().iter().all(|r| !r.on_grid()));

    // A wall on a triangle's side is a wall: walking out through one is
    // blocked, standing on a triangle floor is standing.
    let frame = *w.pieces.frame(bld).unwrap();
    let wall = outer[3];
    let (mx, mz) = frame.to_world(wall.at);
    let (nx, nz) = frame.axis((wall.dir + 3) % 12);
    let floor_y = band_y(frame.band);
    let side = wall.side_of(sq.at) as f32;
    let (ix, iz) = (mx - nx * side * 0.6, mz - nz * side * 0.6);
    let (ox, oz) = (mx + nx * side * 0.1, mz + nz * side * 0.1);
    let cols = w.pieces.cols();
    assert!(collide::blocked(
        SEED, &w.haven, cols, ix, iz, ox, oz, floor_y
    ));
    let (tx, tz) = frame.to_world(tris[2].at);
    let up = floor_y + LEVEL_H_M;
    assert_eq!(
        collide::piece_ground(SEED, &w.haven, cols, tx, tz, up),
        up,
        "a triangle floor is ground"
    );
}

/// Six triangles round one corner close a hexagon; a seventh overlaps the
/// first and is refused, as is a square over any of them.
#[test]
fn six_triangles_make_a_hexagon_and_a_seventh_does_not_fit() {
    let (mut w, mut p, at) = fixture();
    let probe = sim_core::frame::Frame::new((0, 0), 0, 9_000);
    let (ox, oz) = probe.to_world(LPt::ZERO);
    let origin = (((at.0 - ox) * 100.0) as i32, ((at.1 - oz) * 100.0) as i32);
    let first = free_site(SEED, &w.haven, &w.pieces, origin, 9_000, true, 0).unwrap();
    put(&mut w, &mut p, TRI_FOUNDATION, &first).expect("the first triangle");
    let bld = first.addr.bld;
    for k in 1..6u8 {
        let t = Sock::tri_from(LPt::ZERO, 2 * k);
        put(&mut w, &mut p, TRI_FOUNDATION, &on(bld, 0, t)).expect("a hexagon triangle");
    }
    assert_eq!(w.pieces.len(), 6);
    // The seventh would be the first again, turned a full circle — the
    // same footprint — and one offset by 30° overlaps two of them.
    assert_eq!(
        put(
            &mut w,
            &mut p,
            TRI_FOUNDATION,
            &on(bld, 0, Sock::tri_from(LPt::ZERO, 12))
        ),
        Err(REFUSE_B_SPOT)
    );
    assert_eq!(
        put(
            &mut w,
            &mut p,
            TRI_FOUNDATION,
            &on(bld, 0, Sock::tri_from(LPt::ZERO, 1))
        ),
        Err(REFUSE_B_SPOT)
    );
    assert_eq!(
        put(
            &mut w,
            &mut p,
            FOUNDATION,
            &on(bld, 0, Sock::square_from(LPt::ZERO, 1))
        ),
        Err(REFUSE_B_SPOT)
    );
    // Its outer sides take walls like any other.
    for k in 0..6u8 {
        let t = Sock::tri_from(LPt::ZERO, 2 * k);
        let (s, n) = t.sides();
        let out = s[..n].iter().find(|e| e.side_of(LPt::ZERO) != 0).unwrap();
        put(&mut w, &mut p, WALL, &on(bld, 0, *out)).expect("a hexagon wall");
    }
    assert_eq!(w.pieces.len(), 12);
}

/// A second building may not stand where the first does.
#[test]
fn two_buildings_may_not_share_ground() {
    let (mut w, mut p, at) = fixture();
    let a = core(&w, at, 3_000);
    put(&mut w, &mut p, FOUNDATION, &a).expect("the first base");
    // The same square from another frame half a turn round.
    let b = core(&w, at, 3_000 + 32_768);
    assert_eq!(put(&mut w, &mut p, FOUNDATION, &b), Err(REFUSE_B_SPOT));
    // And one turned 45° over the same middle.
    let c = core(&w, at, 3_000 + 8_192);
    assert_eq!(put(&mut w, &mut p, FOUNDATION, &c), Err(REFUSE_B_SPOT));
    // Beside it, clear of it, is a second building.
    let d = core(&w, (at.0 + 4.6, at.1), 11_000);
    put(&mut w, &mut p, FOUNDATION, &d).expect("a neighbour");
    assert_eq!(w.pieces.buildings().live(), 2);
}

/// Take a ring triangle out of the honeycomb and what it alone bore comes
/// down with it — its two outer walls and the floor on them — and nothing
/// else does.
#[test]
fn a_ring_triangle_takes_its_walls_and_floor_down() {
    let (mut w, mut p, at) = fixture();
    let site = core(&w, at, 21_000);
    put(&mut w, &mut p, FOUNDATION, &site).unwrap();
    let bld = site.addr.bld;
    let sq = site.addr.sock;
    let (sides, n) = sq.sides();
    let mut tris = Vec::new();
    for e in &sides[..n] {
        let (_, tri) = beside(e, -e.side_of(sq.at));
        put(&mut w, &mut p, TRI_FOUNDATION, &on(bld, 0, tri)).unwrap();
        tris.push(tri);
    }
    for t in &tris {
        let (ts, k) = t.sides();
        for e in &ts[..k] {
            if !sides[..n].contains(e) {
                put(&mut w, &mut p, WALL, &on(bld, 0, *e)).unwrap();
            }
        }
        put(&mut w, &mut p, TRI_FLOOR, &on(bld, 1, *t)).unwrap();
    }
    assert_eq!(w.pieces.len(), 1 + 4 + 8 + 4);

    let gone = PieceAddr {
        bld,
        level: 0,
        sock: tris[1],
    };
    let i = w.pieces.find_addr(&gone).unwrap();
    let mut ev = EventQueue::default();
    let mut budget = MAX_REMOVALS_PER_TICK;
    let fell = build::take_down(
        &DeployContent::EMPTY,
        &w.build,
        &mut w.pieces,
        &mut w.deploys,
        i,
        &mut budget,
        &mut ev,
    );
    assert_eq!(fell, 3, "two walls and the floor on them");
    assert_eq!(w.pieces.len(), 1 + 3 + 6 + 3);
    let (ts, k) = tris[1].sides();
    for e in &ts[..k] {
        assert!(w
            .pieces
            .find_addr(&PieceAddr {
                bld,
                level: 0,
                sock: *e
            })
            .is_none());
    }
    assert!(w
        .pieces
        .find_addr(&PieceAddr {
            bld,
            level: 1,
            sock: tris[1]
        })
        .is_none());
}

/// The triangle's spiral: a flight up each side of an equilateral body, half
/// a storey each, every one starting where the one under it ended.
#[test]
fn a_triangle_spiral_climbs_round_its_body() {
    let (mut w, mut p, at) = fixture();
    let probe = sim_core::frame::Frame::new((0, 0), 0, 14_000);
    let body = Sock::tri_from(LPt::ZERO, 0);
    let (cx, cz) = probe.to_world(body.at);
    let origin = (((at.0 - cx) * 100.0) as i32, ((at.1 - cz) * 100.0) as i32);
    let first = free_site(SEED, &w.haven, &w.pieces, origin, 14_000, true, 0).unwrap();
    put(&mut w, &mut p, TRI_FOUNDATION, &first).unwrap();
    let bld = first.addr.bld;
    let body = first.addr.sock;
    // Up the first side at the ground, the second half a storey up, the
    // third a storey up: levels 0, 8 (half of 0), 1.
    let flights = [
        (0u8, Sock::tri_riser(body.at, body.dir)),
        (8, Sock::tri_riser(body.at, body.dir + 4)),
        (1, Sock::tri_riser(body.at, body.dir + 8)),
    ];
    for (level, f) in flights {
        put(&mut w, &mut p, TRI_SPIRAL, &on(bld, level, f)).expect("a stacked flight");
    }
    // A fourth on the first one's side at the ground is the same footprint.
    assert_eq!(
        put(
            &mut w,
            &mut p,
            TRI_SPIRAL,
            &on(bld, 0, Sock::tri_riser(body.at, body.dir + 4))
        ),
        Err(REFUSE_B_SPOT)
    );
    // Each flight's ramp rises half a storey along its side.
    let frame = *w.pieces.frame(bld).unwrap();
    let floor = band_y(frame.band);
    let cols = w.pieces.cols();
    for (k, (level, f)) in flights.iter().enumerate() {
        let (o, ux, uz) = f.local_axes();
        let (ox, oz) = frame.to_world(o);
        let (ax, az) = frame.axis(ux);
        let (bx, bz) = frame.axis(uz);
        let base = floor + level_y(*level);
        for (t, rise) in [(0.6f32, 0.3f32), (2.4, 1.2)] {
            let (x, z) = (ox + ax * t + bx * 0.35, oz + az * t + bz * 0.35);
            let g = collide::piece_ground(SEED, &w.haven, cols, x, z, base + rise);
            assert!(
                sim_core::fmath::fabs(g - (base + rise)) < 1e-3,
                "flight {k} at {t}: {g} vs {}",
                base + rise
            );
        }
    }
    // Take the middle flight away and the one above it falls.
    let mid = PieceAddr {
        bld,
        level: 8,
        sock: flights[1].1,
    };
    let i = w.pieces.find_addr(&mid).unwrap();
    let mut budget = MAX_REMOVALS_PER_TICK;
    let fell = build::take_down(
        &DeployContent::EMPTY,
        &w.build,
        &mut w.pieces,
        &mut w.deploys,
        i,
        &mut budget,
        &mut EventQueue::default(),
    );
    assert_eq!(fell, 1);
    assert_eq!(w.pieces.len(), 2);
}

/// Through the commands the wire carries (freeform W2): a foundation placed
/// free starts a turned building, a triangle goes on one of its sides by
/// footprint, and every event names the piece by an address its id is
/// spelled in — the address the store answers to and a demolish takes
/// back down.
#[test]
fn a_turned_base_is_built_and_unbuilt_through_the_commands() {
    use sim_core::world::{Command, EV_PIECE_PLACED, EV_PIECE_REMOVED};

    let (mut w, p, at) = fixture();
    w.tick(&[Command::Join { id: p.id }]);
    let slot = w
        .players
        .iter()
        .position(|q| q.active && q.id == p.id)
        .unwrap();
    w.players[slot].body = p.body;
    let site = core(&w, at, 7_777);
    let (x_cm, z_cm) = site.mint.unwrap().origin_cm;
    let placed = |w: &World| {
        w.events
            .entries()
            .iter()
            .filter(|e| e.code == EV_PIECE_PLACED)
            .map(|e| {
                let (cx, cz) = ((e.a >> 16) as u16, e.a as u16);
                let (level, loc) = ((e.b >> 16) as u8, (e.b >> 8) as u8);
                (cx, cz, level, loc)
            })
            .collect::<Vec<_>>()
    };
    w.tick(&[Command::PlaceFree {
        id: p.id,
        row: FOUNDATION,
        x_cm,
        z_cm,
        yaw: 7_777,
        tri: false,
        plate: 0,
    }]);
    let named = placed(&w);
    assert_eq!(named.len(), 1, "the foundation is announced");
    let core_rec = *w
        .pieces
        .find(named[0].0, named[0].1, named[0].2, named[0].3)
        .unwrap();
    assert!(!core_rec.on_grid());
    assert_eq!(named[0], spell_id(core_rec.id));
    assert_eq!(
        spelled_id(named[0].0, named[0].1, named[0].2, named[0].3),
        Some(core_rec.id)
    );

    // A triangle on its first side, outward.
    let sq = core_rec.sock;
    let (sides, _) = sq.sides();
    let (_, tri) = beside(&sides[0], -sides[0].side_of(sq.at));
    w.tick(&[Command::PlaceAt {
        id: p.id,
        row: TRI_FOUNDATION,
        bld: core_rec.bld,
        level: 0,
        sock: tri,
    }]);
    let named = placed(&w);
    assert_eq!(named.len(), 1, "the triangle is announced");
    let tri_rec = *w
        .pieces
        .find(named[0].0, named[0].1, named[0].2, named[0].3)
        .unwrap();
    assert_eq!((tri_rec.bld, tri_rec.sock), (core_rec.bld, tri));
    // Filed under the cell its middle stands in, for interest.
    let frame = *w.pieces.frame(core_rec.bld).unwrap();
    assert_eq!((tri_rec.cx, tri_rec.cz), anchor_cell(&frame, &tri));

    // The same footprint again is the same spot.
    w.tick(&[Command::PlaceAt {
        id: p.id,
        row: TRI_FOUNDATION,
        bld: core_rec.bld,
        level: 0,
        sock: tri,
    }]);
    assert!(placed(&w).is_empty());

    // Taken back down by the address the event named.
    let (cx, cz, level, loc) = spell_id(tri_rec.id);
    w.tick(&[Command::Demolish {
        id: p.id,
        deploy: false,
        cx,
        cz,
        level,
        loc,
    }]);
    let gone = w
        .events
        .entries()
        .iter()
        .find(|e| e.code == EV_PIECE_REMOVED)
        .expect("the triangle came down");
    assert_eq!((gone.a, gone.b & !0xFF), tri_rec.ev_addr());
    assert_eq!(w.pieces.len(), 1);
}

/// The ghost's snap (freeform W2): aimed beside a turned foundation, a
/// triangle lands on the side nearest the aim, a wall on the side itself,
/// and a taken footprint is passed over for the next free one.
#[test]
fn a_placement_snaps_to_the_nearest_free_socket() {
    let (mut w, mut p, at) = fixture();
    let site = core(&w, at, 3_333);
    put(&mut w, &mut p, FOUNDATION, &site).unwrap();
    let bld = site.addr.bld;
    let sq = site.addr.sock;
    let frame = *w.pieces.frame(bld).unwrap();
    let (sides, n) = sq.sides();
    for e in &sides[..n] {
        // A metre and a half out from the side's middle.
        let out = -e.side_of(sq.at) as f32;
        let (mx, mz) = frame.to_world(e.at);
        let (nx, nz) = frame.axis((e.dir + 3) % 12);
        let (ax, az) = (mx + nx * out * 1.5, mz + nz * out * 1.5);
        let s =
            snap(&w.pieces, SHAPE_TRI_FOUNDATION, 0, ax, az, 6.0, 0).expect("a triangle socket");
        let (_, tri) = beside(e, out as i32);
        assert_eq!((s.bld, s.sock), (bld, tri));
        let s = snap(&w.pieces, SHAPE_WALL, 0, mx, mz, 6.0, 0).expect("a wall socket");
        assert_eq!(s.sock, *e);
    }
    // Take the first side's triangle: aimed there again, the snap moves on.
    let e = sides[0];
    let out = -e.side_of(sq.at);
    let (_, tri) = beside(&e, out);
    put(&mut w, &mut p, TRI_FOUNDATION, &on(bld, 0, tri)).unwrap();
    let (tx, tz) = frame.to_world(tri.at);
    let s = snap(&w.pieces, SHAPE_TRI_FOUNDATION, 0, tx, tz, 6.0, 0).expect("another socket");
    assert_ne!(s.sock, tri);
    assert!(w
        .pieces
        .find_addr(&PieceAddr {
            bld,
            level: 0,
            sock: s.sock
        })
        .is_none());
    // Nothing near, nothing to snap to.
    assert!(snap(&w.pieces, SHAPE_WALL, 0, at.0 + 40.0, at.1, 6.0, 0).is_none());
}
