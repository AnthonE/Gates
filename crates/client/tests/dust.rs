//! Gate: the puff a blow knocks loose grows, thins, rises and goes out, and
//! the pool it lives in stays inside its bound.
//!
//! `tests/impact.rs`'s posture: `render/dust.rs` keeps every law on `Dust`
//! as a pure method, so all of this runs with no `World`, no GPU and no
//! shard. Scheduling is not claimed here.

#![cfg(feature = "render")]

use bevy::math::Vec3;
use bevy::mesh::{Mesh, VertexAttributeValues};
use client::render::dust::{
    billboard_mesh, burst_size, facing, puff_texture, Dust, DUST_ALPHA, DUST_LIFE_S, DUST_POOL,
    DUST_R0_M, DUST_R1_M, DUST_RISE_MPS,
};
use client::render::impact::{Burst, Matter};
use client::render::Eye;

fn burst_at(at: Vec3, away: Vec3, matter: Matter) -> Burst {
    Burst { at, away, matter }
}

/// Wall 4: drop-oldest, so a saturated pool keeps taking bursts and draws
/// exactly its cap.
#[test]
fn the_pool_is_bounded_and_says_so() {
    let mut p = Dust::default();
    for _ in 0..60 {
        p.ignite(&burst_at(Vec3::ZERO, Vec3::Y, Matter::Stone));
    }
    assert_eq!(p.bursts, 60);
    assert_eq!(p.live(), DUST_POOL);
    assert!(
        p.stolen >= (60 * burst_size(Matter::Stone) - DUST_POOL) as u64,
        "past the cap every puff comes off an older one, got {} steals",
        p.stolen
    );
}

/// Every solid matter knocks dust loose and a body does not: `ART.md` has
/// no blood in it, and a cloud off a person would be that.
#[test]
fn every_solid_matter_puffs_and_flesh_does_not() {
    assert_eq!(burst_size(Matter::Flesh), 0, "a blow on a body raised a cloud");
    for m in [
        Matter::Wood,
        Matter::Stone,
        Matter::Metal,
        Matter::Plant,
        Matter::Dirt,
    ] {
        assert!(burst_size(m) > 0, "{m:?} knocks no dust loose");
    }
    assert!(
        burst_size(Matter::Stone) >= burst_size(Matter::Wood),
        "a pick on rock raises less than a hatchet on a trunk"
    );
    let mut p = Dust::default();
    p.ignite(&burst_at(Vec3::ZERO, Vec3::Y, Matter::Flesh));
    assert_eq!((p.live(), p.bursts), (0, 0));
}

/// A puff grows from its birth radius toward its death radius, thins from
/// its birth alpha to nothing, and is gone inside its longest life. Both
/// curves are checked for monotony frame by frame, because a cloud that
/// flickered on one frame would pass any two-point check.
#[test]
fn a_puff_grows_thins_and_goes_out() {
    let mut p = Dust::default();
    p.ignite(&burst_at(Vec3::ZERO, Vec3::Y, Matter::Dirt));
    let n = p.live();
    assert_eq!(n, burst_size(Matter::Dirt));
    let dt = 1.0 / 60.0;
    let mut last: Vec<Option<(f32, f32)>> = vec![None; DUST_POOL];
    let mut frames = 0;
    while p.live() > 0 {
        for (i, seen) in last.iter_mut().enumerate() {
            let Some(d) = p.draw(i) else { continue };
            assert!(
                d.radius >= DUST_R0_M - 1e-5 && d.radius <= DUST_R1_M + 1e-5,
                "a puff drew at radius {:.3}, outside [{DUST_R0_M}, {DUST_R1_M}]",
                d.radius
            );
            assert!(
                d.alpha <= DUST_ALPHA + 1e-5 && d.alpha >= 0.0,
                "a puff drew at alpha {:.3}",
                d.alpha
            );
            if let Some((r, a)) = *seen {
                assert!(d.radius >= r - 1e-6, "puff {i} shrank");
                assert!(d.alpha <= a + 1e-6, "puff {i} thickened");
            }
            *seen = Some((d.radius, d.alpha));
        }
        p.step(dt);
        frames += 1;
        assert!(
            frames as f32 * dt < DUST_LIFE_S * 1.3,
            "a puff outlived its longest life"
        );
    }
    // Every puff ended faint: the last alpha drawn was under a tenth of the
    // birth alpha, so nothing popped out at half strength.
    for a in last.iter().flatten() {
        assert!(
            a.1 < DUST_ALPHA * 0.1,
            "a puff was still at alpha {:.3} on its last frame",
            a.1
        );
    }
}

/// The kick decays and the rise does not: late in its life a puff drifts
/// up at the rise speed and hardly sideways at all.
#[test]
fn a_puff_rises_and_stops_drifting() {
    let mut p = Dust::default();
    p.ignite(&burst_at(Vec3::ZERO, Vec3::new(1.0, 0.0, 0.0), Matter::Stone));
    let dt = 1.0 / 60.0;
    let sample = |p: &mut Dust| -> (f32, f32) {
        let a: Vec<Vec3> = (0..DUST_POOL)
            .filter_map(|i| p.draw(i).map(|d| d.pos))
            .collect();
        p.step(dt);
        let b: Vec<Vec3> = (0..DUST_POOL)
            .filter_map(|i| p.draw(i).map(|d| d.pos))
            .collect();
        let n = a.len() as f32;
        let planar = a
            .iter()
            .zip(&b)
            .map(|(x, y)| ((y.x - x.x).powi(2) + (y.z - x.z).powi(2)).sqrt())
            .sum::<f32>()
            / dt
            / n;
        let up = a.iter().zip(&b).map(|(x, y)| y.y - x.y).sum::<f32>() / dt / n;
        (planar, up)
    };
    let (planar0, _) = sample(&mut p);
    for _ in 0..40 {
        p.step(dt);
    }
    let (planar1, up1) = sample(&mut p);
    assert!(
        planar1 < planar0 * 0.3,
        "the cloud is still drifting sideways at {planar1:.2} m/s after {planar0:.2} — no drag"
    );
    assert!(
        (up1 - DUST_RISE_MPS).abs() < DUST_RISE_MPS * 0.25,
        "late in life a puff rises at {up1:.3} m/s, not the {DUST_RISE_MPS} it should"
    );
}

/// The alpha write is quantized, and it says when it moved — so the draw
/// touches a material once per step and not once per frame.
#[test]
fn the_alpha_step_writes_only_when_it_moves() {
    let mut p = Dust::default();
    p.ignite(&burst_at(Vec3::ZERO, Vec3::Y, Matter::Wood));
    let (a0, moved0) = p.alpha_step(0, 0.30);
    let (a1, moved1) = p.alpha_step(0, 0.30);
    assert!(moved0, "the first write of a slot is a write");
    assert!(!moved1, "the same alpha again is a write");
    assert_eq!(a0, a1);
    assert!((a0 - 0.30).abs() < 0.05, "quantized alpha {a0} is far from 0.30");
    let (_, moved2) = p.alpha_step(0, 0.10);
    assert!(moved2, "a fade that moved a fifth did not write");
}

/// The mask is transparent at the quad's edge, so the corners never draw,
/// and solid enough in the middle to be a cloud at all.
#[test]
fn the_mask_is_padded_transparent_at_its_edges() {
    let img = puff_texture();
    let w = img.texture_descriptor.size.width as usize;
    let data = img.data.as_ref().expect("a generated image carries its bytes");
    let alpha = |x: usize, y: usize| data[(y * w + x) * 4 + 3];
    assert_eq!(alpha(0, 0), 0, "a corner texel is opaque — the quad's edge will draw");
    assert_eq!(alpha(w - 1, 0), 0);
    assert_eq!(alpha(0, w - 1), 0);
    assert!(alpha(w / 2, w / 2) > 200, "the centre is thin: {}", alpha(w / 2, w / 2));
    // And it is not a perfect disc: the three lobes leave the mask
    // asymmetric, which is what keeps a puff from reading as a lens flare.
    let left = alpha(w / 4, w / 2);
    let right = alpha(3 * w / 4, w / 2);
    assert_ne!(left, right, "the mask is radially symmetric — it will read as a flare");
}

/// The billboard's normal is +Y whatever way it faces, so it is lit like
/// the ground it came off and not like a wall facing the eye.
#[test]
fn a_billboard_is_lit_as_the_ground_is() {
    let m: Mesh = billboard_mesh();
    let Some(VertexAttributeValues::Float32x3(pos)) = m.attribute(Mesh::ATTRIBUTE_POSITION)
    else {
        panic!("no positions");
    };
    let Some(VertexAttributeValues::Float32x3(nrm)) = m.attribute(Mesh::ATTRIBUTE_NORMAL)
    else {
        panic!("no normals");
    };
    assert_eq!(pos.len(), 4);
    for p in pos {
        assert_eq!(p[2], 0.0, "the quad is not in its own XY plane");
    }
    for n in nrm {
        assert_eq!(*n, [0.0, 1.0, 0.0], "a billboard normal is not +Y");
    }
}

/// `facing` turns the quad's +Z back at the eye for any look direction.
#[test]
fn facing_turns_the_quad_to_the_eye() {
    for (yaw, pitch) in [(0.0f32, 0.0f32), (1.2, -0.5), (-2.4, 0.7), (3.0, -1.2)] {
        let eye = Eye {
            placed: true,
            pos: Vec3::ZERO,
            yaw,
            pitch,
        };
        let cp = pitch.cos();
        let look = Vec3::new(yaw.sin() * cp, pitch.sin(), yaw.cos() * cp);
        let z = facing(&eye) * Vec3::Z;
        assert!(
            z.dot(-look) > 0.9999,
            "at yaw {yaw} pitch {pitch} the billboard faces {z:?} against a look of {look:?}"
        );
    }
}
