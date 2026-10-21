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

/// A door hangs in a doorway on a triangle's side (freeform W4): placed at
/// the doorway's spelled address, it shuts the opening to bodies, opens and
/// shuts by the use verb from in reach, and comes down with its doorway.
#[test]
fn a_door_hangs_in_a_triangles_doorway() {
    use sim_core::world::Command;
    const DOOR: u16 = 2; // `DeployContent::probe_fixture`'s door row
    let (mut w, p, at) = fixture();
    w.deploy = DeployContent::probe_fixture();
    w.tick(&[Command::Join { id: p.id }]);
    let slot = w
        .players
        .iter()
        .position(|q| q.active && q.id == p.id)
        .unwrap();
    w.players[slot].body = p.body;
    let site = core(&w, at, 13_000);
    let (x_cm, z_cm) = site.mint.unwrap().origin_cm;
    w.tick(&[Command::PlaceFree {
        id: p.id,
        row: FOUNDATION,
        x_cm,
        z_cm,
        yaw: 13_000,
        tri: false,
        plate: 0,
    }]);
    let core_rec = w.pieces.entries()[0];
    let bld = core_rec.bld;
    let frame = *w.pieces.frame(bld).unwrap();
    let (sides, _) = core_rec.sock.sides();
    let side = sides[1];
    let out = -side.side_of(core_rec.sock.at);
    let (_, tri) = beside(&side, out);
    w.tick(&[Command::PlaceAt {
        id: p.id,
        row: TRI_FOUNDATION,
        bld,
        level: 0,
        sock: tri,
    }]);
    let (ts, k) = tri.sides();
    let outer = *ts[..k].iter().find(|e| **e != side).unwrap();
    w.tick(&[Command::PlaceAt {
        id: p.id,
        row: DOORWAY,
        bld,
        level: 0,
        sock: outer,
    }]);
    let doorway = *w
        .pieces
        .entries()
        .iter()
        .find(|r| r.sock == outer)
        .expect("the doorway");
    let (cx, cz, level, loc) = doorway.target();
    assert_eq!(loc, LOC_NONE);

    // Stand on the triangle beside the doorway and hang a door in it.
    let (mx, mz) = frame.to_world(outer.at);
    let (tx, tz) = frame.to_world(tri.at);
    let inside = (mx + (tx - mx) * 0.6, mz + (tz - mz) * 0.6);
    w.players[slot].body = Body::at(SEED, &w.haven, inside.0, inside.1);
    w.players[slot].body.qy =
        sim_core::fmath::round(band_y(frame.band) / movement::POS_Y_Q) as i32 + 1;
    w.players[slot].inv[0] = sim_core::gather::ItemStack {
        item: w.deploy.defs[DOOR as usize].item,
        count: 1,
        cond: 0,
        skin: 0,
    };
    w.tick(&[Command::PlaceDeploy {
        id: p.id,
        row: DOOR,
        cx,
        cz,
        level,
        loc,
        pose: sim_core::footprint::Pose::CENTRE,
    }]);
    let door = w
        .deploys
        .find(cx, cz, level, loc)
        .copied()
        .expect("the door hangs");
    assert!(!door.open, "a door hangs shut");

    // Shut, the doorway stops a body; open, it lets one through.
    let floor_y = band_y(frame.band);
    let (nx, nz) = frame.axis((outer.dir + 3) % 12);
    let cross = |w: &World| {
        let a = (mx - nx * 0.4, mz - nz * 0.4);
        let b = (mx + nx * 0.4, mz + nz * 0.4);
        !collide::blocked(SEED, &w.haven, w.pieces.cols(), a.0, a.1, b.0, b.1, floor_y)
    };
    assert!(!cross(&w), "a shut door is a wall");
    w.tick(&[Command::Use {
        id: p.id,
        cx,
        cz,
        level,
        loc,
    }]);
    assert!(
        w.deploys.find(cx, cz, level, loc).unwrap().open,
        "the use verb swings it"
    );
    assert!(cross(&w), "an open doorway is a way through");
    w.tick(&[Command::Use {
        id: p.id,
        cx,
        cz,
        level,
        loc,
    }]);
    assert!(!cross(&w), "and shut again it is a wall again");

    // The doorway taken down takes its door with it.
    w.tick(&[Command::Demolish {
        id: p.id,
        deploy: false,
        cx,
        cz,
        level,
        loc,
    }]);
    assert!(w.pieces.find(cx, cz, level, loc).is_none());
    assert!(
        w.deploys.find(cx, cz, level, loc).is_none(),
        "the door went with its doorway"
    );
}

/// A body deployable on slabs the grid cannot name (freeform W4): a bench
/// spans a turned square and the triangle beside it at their floor, may not
/// hang off the triangle's open side or stand in a wall on it, and comes
/// down with the triangle.
#[test]
fn a_bench_spans_a_triangle_and_falls_with_it() {
    use sim_core::deploy::{self, ARCH_WORKBENCH, REFUSE_D_SPOT, REFUSE_D_SUPPORT};
    use sim_core::footprint::Pose;
    use sim_core::world::Command;
    const BENCH: u16 = 1; // `DeployContent::probe_fixture`'s workbench row
    let (mut w, p, at) = fixture();
    w.deploy = DeployContent::probe_fixture();
    w.tick(&[Command::Join { id: p.id }]);
    let slot = w
        .players
        .iter()
        .position(|q| q.active && q.id == p.id)
        .unwrap();
    w.players[slot].body = p.body;
    let site = core(&w, at, 13_000);
    let (x_cm, z_cm) = site.mint.unwrap().origin_cm;
    w.tick(&[Command::PlaceFree {
        id: p.id,
        row: FOUNDATION,
        x_cm,
        z_cm,
        yaw: 13_000,
        tri: false,
        plate: 0,
    }]);
    let core_rec = w.pieces.entries()[0];
    let bld = core_rec.bld;
    let frame = *w.pieces.frame(bld).unwrap();
    let (sides, _) = core_rec.sock.sides();
    let side = sides[1];
    let out = -side.side_of(core_rec.sock.at);
    let (_, tri) = beside(&side, out);
    w.tick(&[Command::PlaceAt {
        id: p.id,
        row: TRI_FOUNDATION,
        bld,
        level: 0,
        sock: tri,
    }]);
    let tri_rec = *w
        .pieces
        .entries()
        .iter()
        .find(|r| r.sock == tri)
        .expect("the triangle");
    let floor_y = band_y(frame.band);
    w.players[slot].body = Body::at(SEED, &w.haven, at.0, at.1);
    w.players[slot].body.qy = sim_core::fmath::round(floor_y / movement::POS_Y_Q) as i32 + 1;

    // A bench across the shared side, its length along the side's normal:
    // half on the square, half on the triangle.
    let (mx, mz) = frame.to_world(side.at);
    let (tx, tz) = frame.to_world(tri.at);
    let d = sim_core::fmath::hypot(tx - mx, tz - mz);
    let (nx, nz) = ((tx - mx) / d, (tz - mz) / d);
    // The yaw whose local +x (a bench's length) runs along (ax, az).
    let yaw_along = |ax: f32, az: f32| {
        let turns = sim_core::fmath::atan2(-az, ax) / core::f32::consts::TAU;
        (sim_core::fmath::round(turns * 256.0) as i32).rem_euclid(256) as u8
    };
    let yaw = yaw_along(nx, nz);
    let pose_at = |x: f32, z: f32, yaw: u8| {
        let (cx, cz, ox, oz) = sim_core::footprint::quantize(x, z).unwrap();
        (cx, cz, Pose { ox, oz, yaw })
    };
    let site_at = |w: &World, x: f32, z: f32, yaw: u8| {
        let (cx, cz, pose) = pose_at(x, z, yaw);
        deploy::body_site(
            SEED,
            &w.haven,
            w.pieces.cols(),
            w.deploys.entries(),
            &w.deploy,
            w.deploy.defs[BENCH as usize].placement,
            ARCH_WORKBENCH,
            cx,
            cz,
            0,
            pose,
        )
    };
    let on = site_at(&w, mx, mz, yaw).expect("a bench spans the square and its triangle");
    assert!(!on.ground);
    assert_eq!(
        on.base_y, floor_y,
        "at the building's floor, not the column's"
    );

    // Pushed out to the triangle's apex it hangs off the slab.
    let apex = (mx + nx * 2.2, mz + nz * 2.2);
    assert_eq!(site_at(&w, apex.0, apex.1, yaw), Err(REFUSE_D_SUPPORT));

    // A wall on the square's far side keeps a bench along it out of its
    // strip.
    let back = sides[3];
    let (bx, bz) = frame.to_world(back.at);
    let (ax, az) = frame.axis(back.dir);
    let (cx0, cz0) = frame.to_world(core_rec.sock.at);
    let din = sim_core::fmath::hypot(cx0 - bx, cz0 - bz);
    let (ix, iz) = ((cx0 - bx) / din, (cz0 - bz) / din);
    let (ex, ez) = (bx + ix * 0.45, bz + iz * 0.45);
    let along = yaw_along(ax, az);
    let clear = site_at(&w, ex, ez, along);
    w.tick(&[Command::PlaceAt {
        id: p.id,
        row: WALL,
        bld,
        level: 0,
        sock: back,
    }]);
    assert!(
        w.pieces.entries().iter().any(|r| r.sock == back),
        "the wall stands"
    );
    let walled = site_at(&w, ex, ez, along);
    assert!(
        clear.is_ok() && walled == Err(REFUSE_D_SPOT),
        "the wall's strip refuses it: before {clear:?}, after {walled:?}"
    );

    // Placed, it stands at the floor; the triangle taken down takes it.
    let (cx, cz, pose) = pose_at(mx, mz, yaw);
    w.players[slot].inv[0] = sim_core::gather::ItemStack {
        item: w.deploy.defs[BENCH as usize].item,
        count: 1,
        cond: 0,
        skin: 0,
    };
    w.tick(&[Command::PlaceDeploy {
        id: p.id,
        row: BENCH,
        cx,
        cz,
        level: 0,
        loc: LOC_PLANE,
        pose,
    }]);
    let bench = *w
        .deploys
        .entries()
        .iter()
        .find(|d| d.row == BENCH as u8)
        .expect("the bench stands");
    assert_eq!(
        deploy::body_base_y(SEED, &w.haven, w.pieces.cols(), &bench, ARCH_WORKBENCH),
        floor_y
    );
    let (tcx, tcz, tl, tloc) = tri_rec.target();
    w.tick(&[Command::Demolish {
        id: p.id,
        deploy: false,
        cx: tcx,
        cz: tcz,
        level: tl,
        loc: tloc,
    }]);
    assert!(
        w.pieces.find(tcx, tcz, tl, tloc).is_none(),
        "the triangle is down"
    );
    assert!(
        !w.deploys.entries().iter().any(|d| d.row == BENCH as u8),
        "the bench went with the triangle under half of it"
    );
}

/// A foundation the grid cannot name is not built through a deployable on
/// the ground where it would go (`Place`'s rule, freeform W4).
#[test]
fn a_triangle_is_not_built_over_a_fire() {
    use sim_core::footprint::Pose;
    use sim_core::world::Command;
    const FIRE: u16 = 4; // `DeployContent::probe_fixture`'s fire row
    let (mut w, p, at) = fixture();
    w.deploy = DeployContent::probe_fixture();
    w.tick(&[Command::Join { id: p.id }]);
    let slot = w
        .players
        .iter()
        .position(|q| q.active && q.id == p.id)
        .unwrap();
    w.players[slot].body = p.body;
    let site = core(&w, at, 13_000);
    let (x_cm, z_cm) = site.mint.unwrap().origin_cm;
    w.tick(&[Command::PlaceFree {
        id: p.id,
        row: FOUNDATION,
        x_cm,
        z_cm,
        yaw: 13_000,
        tri: false,
        plate: 0,
    }]);
    let core_rec = w.pieces.entries()[0];
    let frame = *w.pieces.frame(core_rec.bld).unwrap();
    let (sides, _) = core_rec.sock.sides();
    let out = -sides[1].side_of(core_rec.sock.at);
    let (_, tri) = beside(&sides[1], out);
    let (tx, tz) = frame.to_world(tri.at);
    let (cx, cz, ox, oz) = sim_core::footprint::quantize(tx, tz).unwrap();
    w.players[slot].inv[0] = sim_core::gather::ItemStack {
        item: w.deploy.defs[FIRE as usize].item,
        count: 1,
        cond: 0,
        skin: 0,
    };
    w.tick(&[Command::PlaceDeploy {
        id: p.id,
        row: FIRE,
        cx,
        cz,
        level: 0,
        loc: LOC_PLANE,
        pose: Pose { ox, oz, yaw: 0 },
    }]);
    assert_eq!(w.deploys.entries().len(), 1, "the fire is lit");
    w.tick(&[Command::PlaceAt {
        id: p.id,
        row: TRI_FOUNDATION,
        bld: core_rec.bld,
        level: 0,
        sock: tri,
    }]);
    assert!(
        !w.pieces.entries().iter().any(|r| r.sock == tri),
        "no triangle through the fire"
    );
}
