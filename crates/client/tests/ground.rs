//! Gate: the ground mesh's tap sharing is a speed-up and nothing else.
//!
//! `terrain_mesh::heightfield` used to take nine `terrain::height` taps a
//! vertex — the centre, four for the normal's `±d` arms, and four more inside
//! `terrain::splat` for the slope, plus a tenth reading of the centre that
//! `splat` re-derived. Adjacent vertices were sampling each other's points and
//! throwing the answers away. With `generate_tangents` on top of it a 65²
//! chunk cost 28 ms, and `stream` builds one of those per frame: a dropped
//! frame every time the near ring advanced.
//!
//! It now shares those taps, under three f32 identities it checks at the
//! origin it was handed rather than assumes. **A share that is wrong does not
//! crash and does not look obviously wrong** — it shifts a normal or a splat
//! weight by one sample, which reads as slightly different ground. So the gate
//! is equality against a naive rebuild: the same arithmetic, written the slow
//! way, to the bit.
//!
//! The tangent is the one thing here that is NOT bit-equal to what it
//! replaced — mikktspace solves it off the triangles and this writes it off
//! the analytic gradient — so its gate states an angle instead, measured
//! against the solver on the same mesh.
//!
//! Headless — no GPU, no window, no shard.

#![cfg(feature = "render")]

use bevy::math::Vec3;
use bevy::mesh::{Mesh, VertexAttributeValues};
use client::render::terrain_mesh::{self, CHUNK_M, FAR_STEP, NEAR_N};
use sim_core::terrain;

/// The solved authored sites for `seed` — what `terrain::ground` needs in order
/// to know where the carve is. Memoized: `haven` is a few thousand `height`
/// taps and it is a pure function of the seed, so caching cannot change a
/// result.
fn hv(seed: u64) -> &'static sim_core::terrain::Haven {
    use std::cell::RefCell;
    // A thread-local rather than a `Mutex`: `std::sync::Mutex` is on
    // `sim-core/clippy.toml`'s disallowed list (wall 3), and that list is
    // crate-scoped, so it binds this suite too. Per-thread is the right shape
    // anyway — the cache exists to stop a per-assertion recompute, not to be
    // shared.
    thread_local! {
        static CACHE: RefCell<Vec<(u64, &'static sim_core::terrain::Haven)>> =
            const { RefCell::new(Vec::new()) };
    }
    let hit = CACHE.with(|c| c.borrow().iter().find(|(s, _)| *s == seed).map(|&(_, h)| h));
    if let Some(h) = hit {
        return h;
    }
    let h: &'static sim_core::terrain::Haven = Box::leak(Box::new(sim_core::terrain::haven(seed)));
    CACHE.with(|c| c.borrow_mut().push((seed, h)));
    h
}

const SEED: u64 = 0x9E37_79B9_7F4A_7C15;

/// One patch's per-vertex output, in the order the comparison walks it:
/// positions, normals, splat weights, modifiers.
///
/// **The third slot used to be the resolved colour and is now the four splat
/// weights** (2026-08-15, the splat material): the colour is resolved
/// per-pixel in `ground_splat.wgsl` and what the mesh carries is its input.
/// The fourth is new and is the half that would otherwise have gone unwatched
/// — `UV_1`'s macro break-up and waterline are derived from exactly the shared
/// taps this gate exists to check, so they belong under it.
type Attrs = (Vec<[f32; 3]>, Vec<[f32; 3]>, Vec<[f32; 4]>, Vec<[f32; 2]>);

/// `heightfield`'s body with every share taken back out — nine taps a vertex,
/// `terrain::splat` resolving its own height and slope. This is what the
/// optimised path has to reproduce exactly.
fn naive(seed: u64, ox: f32, oz: f32, n: usize, step: f32, drop: f32) -> Attrs {
    let d = (step * 0.5).max(0.5);
    let mut positions = Vec::with_capacity(n * n);
    let mut normals = Vec::with_capacity(n * n);
    let mut colors = Vec::with_capacity(n * n);
    let mut mods = Vec::with_capacity(n * n);
    for iz in 0..n {
        for ix in 0..n {
            let x = ox + ix as f32 * step;
            let z = oz + iz as f32 * step;
            let y = terrain::height(seed, x, z);
            positions.push([x, y - drop, z]);

            let hx = terrain::height(seed, x + d, z) - terrain::height(seed, x - d, z);
            let hz = terrain::height(seed, x, z + d) - terrain::height(seed, x, z - d);
            // glam's own `normalize`, not a restatement of it: this gate is
            // about which points were sampled, and a hand-rolled reciprocal
            // would fail on a rounding difference that has nothing to do with
            // the sharing.
            let n_v = Vec3::new(-hx, 2.0 * d, -hz).normalize();
            normals.push([n_v.x, n_v.y, n_v.z]);

            // `terrain::splat` — the whole point of the comparison. It
            // re-derives the height and takes four fresh taps for the slope;
            // the shipped path hands `splat_from` the two it already has.
            let w = terrain::splat(seed, x, z);
            let grad = ((hx * hx + hz * hz).sqrt()) / (2.0 * d);
            colors.push(terrain_mesh::vertex_splat(w));
            mods.push(terrain_mesh::vertex_mods(y, x, z, grad));
        }
    }
    (positions, normals, colors, mods)
}

fn attrs(mesh: &Mesh) -> Attrs {
    let Some(VertexAttributeValues::Float32x3(p)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) else {
        panic!("no positions");
    };
    let Some(VertexAttributeValues::Float32x3(nn)) = mesh.attribute(Mesh::ATTRIBUTE_NORMAL) else {
        panic!("no normals");
    };
    let Some(VertexAttributeValues::Float32x4(c)) = mesh.attribute(Mesh::ATTRIBUTE_COLOR) else {
        panic!("no splat weights");
    };
    // A missing `UV_1` is the failure that would otherwise be invisible: the
    // shader reads `uv_b` for the break-up and the waterline, and a mesh
    // without it hands them whatever the slot defaults to rather than an error.
    let Some(VertexAttributeValues::Float32x2(m)) = mesh.attribute(Mesh::ATTRIBUTE_UV_1) else {
        panic!("no modifiers on UV_1");
    };
    (p.clone(), nn.clone(), c.clone(), m.clone())
}

fn tangents(mesh: &Mesh) -> Vec<[f32; 4]> {
    let Some(VertexAttributeValues::Float32x4(t)) = mesh.attribute(Mesh::ATTRIBUTE_TANGENT) else {
        panic!("no tangents");
    };
    t.clone()
}

fn compare(label: &str, ox: f32, oz: f32, n: usize, step: f32, drop: f32) {
    let (wp, wn, wc, wm) = attrs(&terrain_mesh::heightfield(
        SEED,
        hv(SEED),
        ox,
        oz,
        n,
        step,
        drop,
    ));
    let (np, nn, nc, nm) = naive(SEED, ox, oz, n, step, drop);
    assert_eq!(wp.len(), np.len(), "{label}: vertex count");
    for i in 0..np.len() {
        for ch in 0..3 {
            assert!(
                wp[i][ch].to_bits() == np[i][ch].to_bits(),
                "{label}: vertex {i} position channel {ch}: {} vs naive {}",
                wp[i][ch],
                np[i][ch]
            );
            assert!(
                wn[i][ch].to_bits() == nn[i][ch].to_bits(),
                "{label}: vertex {i} normal channel {ch}: {} vs naive {}",
                wn[i][ch],
                nn[i][ch]
            );
        }
        for ch in 0..4 {
            assert!(
                wc[i][ch].to_bits() == nc[i][ch].to_bits(),
                "{label}: vertex {i} splat weight {ch}: {} vs naive {}",
                wc[i][ch],
                nc[i][ch]
            );
        }
        for ch in 0..2 {
            assert!(
                wm[i][ch].to_bits() == nm[i][ch].to_bits(),
                "{label}: vertex {i} modifier {ch}: {} vs naive {}",
                wm[i][ch],
                nm[i][ch]
            );
        }
    }
}

/// The tangent is written in closed form instead of solved by mikktspace, and
/// this is the whole of what that trade was: **the frame, bounded against the
/// solver it replaced.**
///
/// `generate_tangents` was 12 ms of a 17 ms near chunk and 229 ms of a 415 ms
/// far mesh — once the tap sharing landed, the most expensive single thing the
/// client did. The UVs are a planar XZ projection at a constant scale, so
/// `∂P/∂u` is `(2d, hx, 0)`, which the vertex loop already holds.
///
/// Three claims, and the third is the one that could go wrong quietly:
/// the frame is orthonormal, its handedness is mikktspace's own (`w = 1` — the
/// sea hand-writes `-1` for the same parameterisation and the two disagree,
/// which is a question for someone with the game booted, not for this pass),
/// and the direction is within a stated angle of what the solver produced.
#[test]
fn the_written_tangent_is_the_frame_mikktspace_solved() {
    let step = CHUNK_M / (NEAR_N - 1) as f32;
    for (label, ox, oz, n, pitch, bound) in [
        ("near ring", 1024.0f32, 512.0f32, NEAR_N, step, 1.5f32),
        // The 8 m pitch is where a triangle-averaged tangent and an analytic
        // one have the most room to disagree, and they do — about five times
        // as much as at 1 m. The bound is set from what was measured, so it
        // is a tripwire on the trade rather than a licence to widen it.
        ("far pitch", 0.0, 0.0, 65, FAR_STEP, 5.0),
    ] {
        let ours = terrain_mesh::heightfield(SEED, hv(SEED), ox, oz, n, pitch, 0.0);
        let mut theirs = terrain_mesh::heightfield(SEED, hv(SEED), ox, oz, n, pitch, 0.0);
        theirs.remove_attribute(Mesh::ATTRIBUTE_TANGENT);
        theirs
            .generate_tangents()
            .expect("the mesh has the UVs and normals mikktspace needs");

        let (_, nor, _, _) = attrs(&ours);
        let a = tangents(&ours);
        let b = tangents(&theirs);
        let mut worst = 0.0f32;
        for i in 0..a.len() {
            let n_v = Vec3::from_array(nor[i]);
            let t = Vec3::new(a[i][0], a[i][1], a[i][2]);
            assert!(
                (t.length() - 1.0).abs() < 1e-6,
                "{label}: vertex {i} tangent is not unit ({})",
                t.length()
            );
            assert!(
                t.dot(n_v).abs() < 1e-6,
                "{label}: vertex {i} tangent is not orthogonal to its normal ({})",
                t.dot(n_v)
            );
            assert_eq!(
                a[i][3], b[i][3],
                "{label}: vertex {i} handedness disagrees with mikktspace"
            );
            let s = Vec3::new(b[i][0], b[i][1], b[i][2]).normalize();
            worst = worst.max(t.dot(s).clamp(-1.0, 1.0).acos().to_degrees());
        }
        assert!(
            worst <= bound,
            "{label}: the written tangent is {worst:.3}° off mikktspace's, over the \
             {bound}° this trade was made at"
        );
    }
}

/// The near ring: 1 m pitch, which is the pitch at which `terrain::slope`'s
/// own ±1 m arm lands on the vertex lattice. This is the path with all three
/// shares live and the one that carries the 3× — so it is the one where a
/// wrong index would be cheapest to introduce and hardest to see.
#[test]
fn the_near_ring_is_bit_identical_to_the_naive_build() {
    let step = CHUNK_M / (NEAR_N - 1) as f32;
    assert_eq!(
        step, 1.0,
        "the near ring's pitch is what puts slope on the lattice"
    );
    // Four chunk origins including the negative quadrant: `ox` is a multiple of
    // 64 and the identities are integer-exact there, but a negative origin
    // exercises the sign the guards would have to survive.
    for (cx, cz) in [(0i32, 0i32), (3, -2), (-5, 7), (16, 16)] {
        compare(
            &format!("near chunk ({cx}, {cz})"),
            cx as f32 * CHUNK_M,
            cz as f32 * CHUNK_M,
            NEAR_N,
            step,
            0.0,
        );
    }
}

/// The far mesh's pitch: 8 m, where the slope arm is a quarter of a step and
/// cannot come off the lattice. Only the two normal-arm shares are live, and
/// the `grid_slope` fallback has to be the one that runs. A patch rather than
/// the whole 257² island — same code path, and this gate is about arithmetic
/// rather than about size.
#[test]
fn the_far_pitch_is_bit_identical_to_the_naive_build() {
    for (ox, oz) in [(0.0f32, 0.0f32), (512.0, 1024.0)] {
        compare(
            &format!("far patch at ({ox}, {oz})"),
            ox,
            oz,
            65,
            FAR_STEP,
            0.15,
        );
    }
}

/// An origin and a pitch nobody ships, to prove the guards are guards. A
/// fractional pitch breaks every one of the three identities (`x_{k-1} + d`
/// stops landing on `x_k − d` to the bit, and the 1 m slope arm is nowhere
/// near the lattice), so this run must take the fallbacks — and still agree.
///
/// **Both patches are deliberately on cliff-steep ground**, and the assert
/// below is what keeps them there. A wrong slope only changes a colour where
/// the slope is inside `splat`'s cliff band; run this over the flats and a
/// `grid_slope` taken at a pitch that is not 1 m compares equal, which is a
/// gate that passes for the wrong reason. Found exactly that way: the first
/// version of this test stayed green under a forced `grid_slope = true`.
#[test]
fn an_unshipped_origin_and_pitch_still_agree() {
    for (label, ox, oz, n, step) in [
        ("odd pitch on a bank", 428.0f32, 1398.0f32, 24usize, 1.7f32),
        ("sub-metre pitch on a bank", 485.0, 1375.0, 20, 0.4),
    ] {
        // `splat_from`'s cliff ramp spans tan(50°)·0.8 … ·1.2; a patch that
        // does not reach into it cannot tell two slopes apart.
        let (mut lo, mut hi) = (f32::INFINITY, 0.0f32);
        for iz in 0..n {
            for ix in 0..n {
                let s = terrain::slope(SEED, ox + ix as f32 * step, oz + iz as f32 * step);
                lo = lo.min(s);
                hi = hi.max(s);
            }
        }
        assert!(
            lo < 1.19 * 1.2 && hi > 1.19 * 0.8,
            "{label}: slopes {lo:.3}..{hi:.3} miss the cliff band — a wrong slope \
             would not change a single colour here"
        );
        compare(label, ox, oz, n, step, 0.0);
    }
}
