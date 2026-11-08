//! The engine's laws with no engine: a pool stays inside its bounds, a spark
//! stays visible and outside its wall, the mesh is written in place, the
//! sprite sheet is pinned, knobs round-trip, and the stage's clock steps.

use fx::atlas::{self, Shape, CELLS, DEFAULT};
use fx::pool::{frame_at, indices, Cam, Orient, Particle, Pool, Sheet, Verts, MIN_PX};
use fx::recipe::{emit, knobs, scaled, Recipe};
use fx::stage::{self, Clock, Framing, WARM};
use fx::v3;

const SPARKS: Recipe = Recipe {
    orient: Orient::Stretch,
    cell: 1,
    speed: (3.0, 9.0),
    spray: 0.65,
    up: 0.8,
    gravity: 9.81,
    drag: 2.4,
    life: (0.16, 0.42),
    size: (0.006, 0.008),
    grow: 0.5,
    c0: [22.0, 13.0, 5.0, 1.0],
    c1: [2.2, 0.35, 0.05, 0.0],
    stretch: 0.016,
    bounce: true,
    ..Recipe::BASE
};

const DUST: Recipe = Recipe {
    cells: 3,
    speed: (0.4, 1.6),
    spray: 0.55,
    up: 0.2,
    gravity: -0.25,
    drag: 2.8,
    life: (0.8, 1.4),
    size: (0.05, 0.08),
    grow: 6.0,
    tinted: true,
    ..Recipe::BASE
};

fn digest(bytes: &[u8]) -> u64 {
    let mut d: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        d = (d ^ b as u64).wrapping_mul(0x100_0000_01b3);
    }
    d
}

/// A firefight cannot grow a pool, and a full pool recycles its oldest
/// rather than dropping the newest blow.
#[test]
fn a_pool_is_bounded_and_recycles() {
    let mut p = Pool::new(64, false);
    for _ in 0..40 {
        emit(&mut p, &SPARKS, 12, v3::ZERO, v3::Y, [1.0; 3], 1.0);
    }
    assert_eq!(p.live(), 64);
    assert_eq!(p.spawned, 480);
    assert_eq!(p.stolen, 480 - 64, "every overflow is counted");
    p.clear();
    assert_eq!(p.live(), 0);
}

/// The quad's extent across its streak (corner 0 to corner 3).
fn quad_width(p: &Particle, cam: &Cam) -> f32 {
    let (pos, _, _) = Pool::quad(p, cam, v3::ZERO).expect("live");
    v3::len(v3::sub(pos[3], pos[0]))
}

/// A spark far away is still at least ~1.5 px wide — the web draws no bloom
/// to widen it.
#[test]
fn a_distant_spark_does_not_vanish() {
    let cam = Cam {
        pos: v3::ZERO,
        right: v3::X,
        up: v3::Y,
    };
    let p = Particle {
        left: 0.3,
        life: 0.3,
        pos: [0.0, 0.0, -60.0],
        vel: [5.0, 0.0, 0.0],
        size0: 0.006,
        size1: 0.006,
        c0: [1.0; 4],
        c1: [1.0; 4],
        orient: Orient::Stretch,
        stretch: 0.016,
        ..Particle::default()
    };
    let w = quad_width(&p, &cam);
    assert!(
        w >= 2.0 * 60.0 * MIN_PX * 0.99,
        "a spark at 60 m draws {w} m wide"
    );
}

/// A streak flying straight from the eye is short on screen: its length lies
/// in depth, behind its head, not sideways across the view.
#[test]
fn a_streak_flying_away_is_foreshortened() {
    let cam = Cam {
        pos: v3::ZERO,
        right: v3::X,
        up: v3::Y,
    };
    let p = Particle {
        left: 0.3,
        life: 0.3,
        pos: [0.0, 0.0, -10.0],
        vel: [0.0, 0.0, -300.0],
        size0: 0.01,
        size1: 0.01,
        c0: [1.0; 4],
        c1: [1.0; 4],
        orient: Orient::Stretch,
        stretch: 0.02,
        len_max: 5.0,
        ..Particle::default()
    };
    let (pos, _, _) = Pool::quad(&p, &cam, v3::ZERO).expect("live");
    for c in pos {
        assert!(c[0].abs() < 0.05 && c[1].abs() < 0.05, "sideways: {c:?}");
    }
    // Its length is behind its head, toward the eye.
    let near = pos.iter().map(|c| c[2]).fold(f32::MIN, f32::max);
    let far = pos.iter().map(|c| c[2]).fold(f32::MAX, f32::min);
    assert!(
        (near - -5.0).abs() < 0.01 && (far - -10.0).abs() < 0.01,
        "{near} to {far}"
    );
}

/// A streak in any direction runs from its head straight back along its
/// flight: both its ends are on that line.
#[test]
fn a_streak_is_a_true_line_from_its_head_back() {
    let cam = Cam {
        pos: v3::ZERO,
        right: v3::X,
        up: v3::Y,
    };
    let dir = v3::norm_or([1.0, 0.5, -1.0], v3::Z);
    let head = [3.0, 1.0, -10.0];
    let p = Particle {
        left: 0.3,
        life: 0.3,
        pos: head,
        vel: v3::scale(dir, 100.0),
        size0: 0.01,
        size1: 0.01,
        c0: [1.0; 4],
        c1: [1.0; 4],
        orient: Orient::Stretch,
        stretch: 0.02,
        len_max: 5.0,
        ..Particle::default()
    };
    let (pos, _, _) = Pool::quad(&p, &cam, v3::ZERO).expect("live");
    let mid = |a: usize, b: usize| v3::scale(v3::add(pos[a], pos[b]), 0.5);
    let (a, b) = (mid(0, 3), mid(1, 2));
    let tail = v3::sub(head, v3::scale(dir, 2.0));
    assert!(v3::dist2(b, head) < 1e-4, "its head at {b:?}");
    assert!(v3::dist2(a, tail) < 1e-4, "its tail at {a:?}");
}

/// Dead head on, it is still something to see; and a longest shorter than the
/// shortest draws the shortest, where the clamp used to panic.
#[test]
fn a_streak_head_on_or_short_is_drawn() {
    let cam = Cam {
        pos: v3::ZERO,
        right: v3::X,
        up: v3::Y,
    };
    let p = Particle {
        left: 0.3,
        life: 0.3,
        pos: [0.0, 0.0, -10.0],
        vel: [0.0, 0.0, -100.0],
        size0: 0.01,
        size1: 0.01,
        c0: [1.0; 4],
        c1: [1.0; 4],
        orient: Orient::Stretch,
        stretch: 0.02,
        len_max: 0.001,
        ..Particle::default()
    };
    let (pos, _, _) = Pool::quad(&p, &cam, v3::ZERO).expect("live");
    let across = pos.iter().map(|c| c[1]).fold(f32::MIN, f32::max)
        - pos.iter().map(|c| c[1]).fold(f32::MAX, f32::min);
    let wide = pos.iter().map(|c| c[0]).fold(f32::MIN, f32::max)
        - pos.iter().map(|c| c[0]).fold(f32::MAX, f32::min);
    assert!(
        across > 0.005 && wide > 0.005,
        "edge on: {across} by {wide}"
    );
}

/// A spark thrown into the surface it came off bounces back out of it.
#[test]
fn a_spark_does_not_fall_through_its_wall() {
    let mut p = Pool::new(8, false);
    p.spawn(Particle {
        left: 1.0,
        life: 1.0,
        pos: [0.0, 1.0, 0.05],
        vel: [0.0, 0.0, -6.0],
        gravity: 9.81,
        plane_p: v3::ZERO,
        plane_n: v3::Z,
        ..Particle::default()
    });
    for _ in 0..30 {
        p.step(1.0 / 60.0);
    }
    let s = p.particles().next().expect("still alive");
    assert!(s.pos[2] >= -1e-4, "through the wall: z = {}", s.pos[2]);
}

struct Mesh {
    pos: Vec<[f32; 3]>,
    uv: Vec<[f32; 2]>,
    col: Vec<[f32; 4]>,
}

impl Mesh {
    fn new(cap: usize) -> Self {
        Self {
            pos: vec![[0.0; 3]; cap * 4],
            uv: vec![[0.0; 2]; cap * 4],
            col: vec![[0.0; 4]; cap * 4],
        }
    }
    fn verts(&mut self) -> Verts<'_> {
        Verts {
            pos: &mut self.pos,
            uv: &mut self.uv,
            col: &mut self.col,
            uv2: None,
            nrm: None,
            aux: None,
        }
    }
}

/// The mesh is written in place: live particles are quads, dead ones are
/// collapsed, and an empty pool writes once more to clear and then stops.
#[test]
fn the_mesh_collapses_the_dead_and_goes_quiet() {
    let mut mesh = Mesh::new(16);
    let mut p = Pool::new(16, true);
    let cam = Cam {
        pos: [0.0, 1.6, 5.0],
        right: v3::X,
        up: v3::Y,
    };
    emit(&mut p, &DUST, 3, v3::ZERO, v3::Y, [0.8, 0.7, 0.6], 1.0);
    assert!(p.needs_write());
    assert!(p.fill(mesh.verts(), &cam, cam.pos));
    let quads = mesh
        .pos
        .chunks(4)
        .filter(|q| q.iter().any(|v| *v != [0.0; 3]))
        .count();
    assert_eq!(quads, 3);
    for _ in 0..200 {
        p.step(0.05);
    }
    assert_eq!(p.live(), 0);
    assert!(p.needs_write(), "the dead must be cleared once");
    assert!(p.fill(mesh.verts(), &cam, cam.pos));
    assert!(mesh.pos.iter().all(|v| *v == [0.0; 3]));
    assert!(!p.needs_write(), "and then the mesh is left alone");
    assert_eq!(indices(16).len(), 16 * 6);
}

/// The alpha pool draws far to near, so overlapping puffs blend in order.
#[test]
fn a_sorted_pool_draws_back_to_front() {
    let mut mesh = Mesh::new(4);
    let mut p = Pool::new(4, true);
    let cam = Cam {
        pos: v3::ZERO,
        right: v3::X,
        up: v3::Y,
    };
    for z in [-2.0, -8.0, -4.0] {
        p.spawn(Particle {
            left: 1.0,
            life: 1.0,
            pos: [0.0, 0.0, z],
            size0: 0.1,
            size1: 0.1,
            c0: [1.0; 4],
            c1: [1.0; 4],
            ..Particle::default()
        });
    }
    p.fill(mesh.verts(), &cam, v3::ZERO);
    let z: Vec<f32> = mesh.pos.chunks(4).take(3).map(|q| q[0][2]).collect();
    assert_eq!(z, vec![-8.0, -4.0, -2.0]);
}

/// A tinted recipe takes the caller's colour and an untinted one ignores it;
/// `scale` grows the particles; the seed makes a run repeatable.
#[test]
fn emit_paints_scales_and_repeats() {
    let mut a = Pool::new(8, false);
    emit(&mut a, &DUST, 1, v3::ZERO, v3::Y, [0.5, 0.25, 1.0], 2.0);
    let d = *a.particles().next().unwrap();
    assert_eq!(d.c0, [0.5, 0.25, 1.0, 1.0]);
    assert!(d.size0 >= 2.0 * DUST.size.0 - 1e-6);
    let mut b = Pool::new(8, false);
    emit(&mut b, &SPARKS, 1, v3::ZERO, v3::Y, [0.0; 3], 1.0);
    assert_eq!(b.particles().next().unwrap().c0, SPARKS.c0);
    let run = |seed| {
        let mut p = Pool::new(8, false);
        p.seed(seed);
        emit(&mut p, &SPARKS, 4, v3::ZERO, v3::Y, [1.0; 3], 1.0);
        p.particles().map(|q| q.vel).collect::<Vec<_>>()
    };
    assert_eq!(run(7), run(7));
    assert_ne!(run(7), run(8));
    assert_eq!(scaled(1, 0.1), 1);
    assert_eq!(scaled(12, 1.0), 12);
    assert_eq!(scaled(5, 0.0), 0);
}

/// A flipbook walks its frames over its life, fading into the next and
/// holding the last; a variant rolls to the next flipbook along; a still
/// is one cell at a blend of 0; the sheet's cut places each cell.
#[test]
fn a_flipbook_walks_its_frames() {
    assert_eq!(frame_at(0, 0.5), (0, 0, 0.0));
    assert_eq!(frame_at(1, 0.9), (0, 0, 0.0));
    assert_eq!(frame_at(4, 0.0), (0, 1, 0.0));
    let (a, b, k) = frame_at(4, 0.3);
    assert_eq!((a, b), (1, 2));
    assert!((k - 0.2).abs() < 1e-5, "{k}");
    assert_eq!(frame_at(4, 0.99), (3, 3, 0.0));
    assert_eq!(frame_at(4, 1.5), (3, 3, 0.0));
    let sheet = Sheet {
        cols: 8,
        rows: 8,
        tex: 128,
    };
    let cam = Cam {
        pos: [0.0, 0.0, 4.0],
        right: v3::X,
        up: v3::Y,
    };
    let p = Particle {
        left: 0.5,
        life: 1.0,
        size0: 0.1,
        size1: 0.1,
        c0: [1.0; 4],
        c1: [1.0; 4],
        cell: 16,
        frames: 8,
        seed: 0.25,
        ..Particle::default()
    };
    let ((_, uv, _), (uv2, right, aux)) = Pool::quad_on(&p, &cam, v3::ZERO, &sheet).expect("live");
    // Half way: frame 4 of 8, which is cell 20, the third row's fifth.
    assert_eq!(uv, sheet.corners(20));
    assert_eq!(uv2, sheet.corners(21));
    assert_eq!(aux, [0.0, 0.5, 0.0, 0.0]);
    assert!(
        (v3::len(right) - 1.0).abs() < 1e-5 && right[0] > 0.99,
        "unrolled: {right:?}"
    );
    // A seed past a half mirrors the flipbook: the corners swap in u.
    let twin = Particle { seed: 0.75, ..p };
    let ((_, m, _), (m2, _, _)) = Pool::quad_on(&twin, &cam, v3::ZERO, &sheet).expect("live");
    assert_eq!([m[1], m[0], m[3], m[2]], uv);
    assert_eq!([m2[1], m2[0], m2[3], m2[2]], uv2);
    let inset = 0.5 / 128.0;
    assert!((uv[3][0] - (4.0 + inset) / 8.0).abs() < 1e-6);
    assert!((uv[3][1] - (2.0 + inset) / 8.0).abs() < 1e-6);
    // A pool on that sheet writes the extras; a still pool's are its cell twice.
    let mut pool = Pool::with_sheet(2, false, sheet);
    let film = Recipe {
        cell: 0,
        cells: 2,
        frames: 8,
        ..Recipe::BASE
    };
    emit(&mut pool, &film, 2, v3::ZERO, v3::Y, [1.0; 3], 1.0);
    assert!(pool
        .particles()
        .all(|q| q.frames == 8 && (q.cell == 0 || q.cell == 8)));
    let (mut pos, mut uv, mut col) = (
        vec![[0.0f32; 3]; 8],
        vec![[0.0f32; 2]; 8],
        vec![[0.0f32; 4]; 8],
    );
    let (mut uv2, mut nrm, mut aux) = (
        vec![[0.0f32; 2]; 8],
        vec![[0.0f32; 3]; 8],
        vec![[0.0f32; 4]; 8],
    );
    pool.fill(
        Verts {
            pos: &mut pos,
            uv: &mut uv,
            col: &mut col,
            uv2: Some(&mut uv2),
            nrm: Some(&mut nrm),
            aux: Some(&mut aux),
        },
        &cam,
        v3::ZERO,
    );
    assert!(nrm.iter().all(|n| (v3::len(*n) - 1.0).abs() < 1e-4));
    // Born, each fades toward its second frame: cell 1 or 9, mirrored or not.
    let next: Vec<[f32; 2]> = [1, 9]
        .iter()
        .flat_map(|&c| {
            let k = sheet.corners(c);
            [k[0], k[1]]
        })
        .collect();
    assert!(next.contains(&uv2[0]), "{:?}", uv2[0]);
    assert!(aux
        .iter()
        .all(|a| a[1] == 0.0 && a[2] == 0.0 && a[3] == 1.0));
    let still = Particle { frames: 0, ..p };
    let ((_, s1, _), (s2, _, a)) = Pool::quad_on(&still, &cam, v3::ZERO, &sheet).expect("live");
    assert_eq!(s1, s2);
    assert_eq!(a[0], 0.0);
}

/// Gates' sheet, pinned: a change to a sprite is a change to every effect
/// that draws it, so look at the strips (`ci/fx.sh`) and then repin.
#[test]
fn the_default_sheet_is_pinned() {
    let (px, w, h) = atlas::sheet(&DEFAULT);
    assert_eq!((w, h), (256, 256));
    assert_eq!(px.len(), 256 * 256 * 4);
    let d = digest(&px);
    assert_eq!(d, 0xcfa8_8247_627f_258f, "sheet digest {d:#018x}");
}

/// Every shape draws something, inside its cell, and keeps off the edge.
#[test]
fn every_shape_has_a_body_and_a_margin() {
    let shapes = [
        Shape::Glow,
        Shape::Streak,
        Shape::Star(0),
        Shape::Puff,
        Shape::Smoke,
        Shape::Droplet,
        Shape::Ring,
        Shape::Speck,
        Shape::Leaf,
        Shape::Splinter,
        Shape::Flame,
        Shape::Arc,
        Shape::Shard,
    ];
    for (i, &s) in shapes.iter().enumerate() {
        let cell = (i % CELLS) as u8;
        let mut cover = 0.0;
        for k in 0..64 {
            for j in 0..64 {
                let u = (k as f32 + 0.5) / 32.0 - 1.0;
                let v = (j as f32 + 0.5) / 32.0 - 1.0;
                cover += atlas::sprite(s, cell, u, v).1;
            }
        }
        assert!(cover > 4.0, "{s:?} is empty");
        assert_eq!(atlas::sprite(s, cell, 0.995, 0.995).1, 0.0, "{s:?} edge");
    }
    assert_eq!(atlas::sprite(Shape::Empty, 0, 0.0, 0.0), (0.0, 0.0));
}

/// Knobs parse, apply, refuse what is not a knob, and a recipe prints back
/// into knobs that rebuild it exactly.
#[test]
fn knobs_round_trip() {
    let ks: Vec<_> = knobs("Sparks.speed=4,12; Flash.life = 0.04,0.07\n\n")
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(ks.len(), 2);
    assert_eq!(
        (ks[0].layer, ks[0].key, ks[0].val),
        ("Sparks", "speed", "4,12")
    );
    let mut r = SPARKS;
    r.set(ks[0].key, ks[0].val).unwrap();
    assert_eq!(r.speed, (4.0, 12.0));
    r.set("life", "0.3").unwrap();
    assert_eq!(r.life, (0.3, 0.3));
    r.set("orient", "flat").unwrap();
    r.set("c0", "1,2,3,0.5").unwrap();
    assert!(r.set("speed", "fast").is_err());
    assert!(r.set("colour", "1").is_err());
    assert!(r.set("life", "0").is_err());
    assert!(r.set("c1", "1,2").is_err());
    assert!(knobs("Sparks speed=4").next().unwrap().is_err());
    let text = r.to_string();
    let mut back = Recipe::BASE;
    for kv in text.split(';') {
        let (k, v) = kv.split_once('=').unwrap();
        back.set(k, v).unwrap_or_else(|e| panic!("{e} in {text}"));
    }
    assert_eq!(back, r, "{text}");
    for key in Recipe::KEYS {
        assert!(text.contains(&format!("{key}=")), "{key} not printed");
    }
}

/// The clock warms up on the plate, fires once on step 1, steps to what a
/// shooter asks and holds there, and loops only when nobody drives it.
#[test]
fn the_stage_clock_steps_and_holds() {
    let mut c = Clock::new();
    for _ in 0..WARM {
        let t = c.frame(Some(5));
        assert_eq!((t.dt, t.step, t.fire), (0.0, 0, false));
    }
    assert!(c.ready());
    let mut fired = 0;
    for _ in 0..5 {
        let t = c.frame(Some(5));
        assert_eq!(t.dt, stage::DT);
        fired += t.fire as u32;
    }
    assert_eq!((c.step(), fired), (5, 1));
    for k in 0..3 {
        assert_eq!(c.frame(Some(5)).dt, 0.0);
        assert_eq!(c.held(), k + 1);
    }
    assert_eq!(c.frame(Some(6)).step, 6);
    assert_eq!(c.held(), 0);
    let mut l = Clock::looping(3);
    let ticks: Vec<_> = (0..WARM + 6).map(|_| l.frame(None)).collect();
    let tail: Vec<_> = ticks[WARM as usize..]
        .iter()
        .map(|t| (t.step, t.fire, t.clear))
        .collect();
    assert_eq!(
        tail,
        vec![
            (1, true, false),
            (2, false, false),
            (3, false, false),
            (0, false, true),
            (1, true, false),
            (2, false, false)
        ]
    );
}

/// A strip starts on the plate, ends at the effect's span and packs its
/// frames toward the start.
#[test]
fn a_strip_packs_its_frames_early() {
    let s = stage::steps(1.0, 11);
    assert_eq!(s[0], 0);
    assert_eq!(s[1], 1);
    assert_eq!(*s.last().unwrap(), 60);
    assert!(s.windows(2).all(|w| w[0] < w[1]));
    assert!(s[5] < 20, "half the frames in the first third: {s:?}");
    assert_eq!(stage::steps(0.0, 3), vec![0, 1]);
}

/// The framing sees both anchors, puts each over its own card, and throws
/// toward the camera.
#[test]
fn the_framing_frames_both_cards() {
    let f = Framing::new(0.5);
    let half_h = (f.fov_y * 0.5).tan() * f.eye[2];
    let half_w = half_h * 16.0 / 9.0;
    for (a, c) in f.anchors.iter().zip(f.cards.iter()) {
        assert!(a[0].abs() < half_w && a[1].abs() < half_h);
        assert!((a[0] - c.centre[0]).abs() < c.half[0]);
        assert!(c.half[1] > half_h, "a card fills the frame's height");
    }
    assert!(f.cards[0].centre[0] < 0.0 && f.cards[1].centre[0] > 0.0);
    assert!(f.cards[0].srgb < f.cards[1].srgb);
    assert!(f.dir[2] > 0.0);
}
