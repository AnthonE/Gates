//! The splat weights' ride to the fragment stage.
//!
//! `render::terrain_mesh` resolves four ground identity weights per vertex and
//! has always used them twice-removed: [`vertex_color`] mixes them into one
//! RGB and the mesh carries only that mix. A material that blends four MAP
//! SETS needs the weights themselves at the fragment, so they now ride
//! `ATTRIBUTE_UV_1` packed two `u8` to an `f32`.
//!
//! **What this gate is actually about.** The packing is arithmetic with an
//! exactness claim on it — "an `f32` holds every integer below `2^24`, and the
//! divisor is a power of two" — and an exactness claim that nothing checks is
//! the shape `CLAUDE.md`'s ⚠ names. It is also arithmetic that exists TWICE:
//! once in Rust ([`pack_splat`] / [`unpack_splat`]) and once in WGSL, where
//! this suite cannot reach it. A divergence between the two does not crash and
//! does not fail any other gate — it draws the wrong identity on a chunk, or a
//! seam where two chunks disagree, which is precisely the class of defect the
//! event-role gate exists for one layer down: three green gates over a wrong
//! payload.
//!
//! So the Rust half is gated exhaustively here, and the WGSL half is held to
//! it by being the same four lines — `assets/shaders/ground_splat.wgsl` says
//! so at its `unpack_splat`, and §D below pins the constant both sides divide
//! by so a change to one is a change to a value this file reads.

#![allow(clippy::float_cmp)]

use bevy::mesh::{Mesh, VertexAttributeValues};
use client::render::terrain_mesh::{
    heightfield, pack_splat, unpack_splat, FAR_DROP, FAR_N, FAR_STEP, GROUND_SPLAT_SHADER, NEAR_N,
};

/// The seed the shard ships and every judged frame was shot on.
const SEED: u64 = 20260731;

/// §A. The round trip, **exhaustively over the domain that exists**.
///
/// `pack_splat` is two independent lanes — `w[0..2]` become `uv1[0]` and
/// `w[2..4]` become `uv1[1]` by identical code — so sweeping all 65,536
/// ordered `(a, b)` pairs through both lanes at once covers the whole input
/// space of both. That is exhaustive rather than sampled, which matters
/// because the failure this catches is a boundary one: `floor` landing a ulp
/// low at exactly the values where `b` is 0 or 255.
#[test]
fn the_pack_round_trips_every_pair_that_can_exist() {
    for a in 0u16..=255 {
        for b in 0u16..=255 {
            // `[a, b, 255-a, 255-b]` rather than a palindrome. Both lanes still
            // see all 65,536 ordered pairs — `(a, b)` and its complement sweep
            // the same space — but the vector is no longer symmetric under
            // swapping slots 1 and 2, which a palindrome is. An earlier draft
            // used one, and a pack that transposed those two slots round
            // -tripped through it unharmed.
            let w = [a as u8, b as u8, (255 - a) as u8, (255 - b) as u8];
            let got = unpack_splat(pack_splat(w));
            assert_eq!(
                got, w,
                "pack/unpack is not the identity at {w:?} — got {got:?}.\n\
                 Both lanes are swept here, so a failure at one index is a \
                 failure of the shared arithmetic, not of one position."
            );
        }
    }
}

/// §B. The exactness claim itself, stated as the doc comment states it.
///
/// The packed value must be a non-negative integer no greater than 65,535 —
/// integral because a fractional part means the encode already lost a bit
/// before the shader ever divides, and bounded because `2^24` is the headroom
/// the claim rests on. A `fract() == 0.0` assertion is exact here and is not
/// the float-comparison smell it looks like: the claim is that these ARE
/// integers, so anything but a bit-exact zero fraction refutes it.
#[test]
fn every_packed_weight_is_an_exact_integer_in_range() {
    for a in 0u16..=255 {
        for b in 0u16..=255 {
            let p = pack_splat([a as u8, b as u8, 0, 0]);
            for v in p {
                assert!(
                    v.fract() == 0.0,
                    "pack_splat({a}, {b}) produced {v}, which is not integral — \
                     the encode lost precision before the shader saw it."
                );
                assert!(
                    (0.0..=65_535.0).contains(&v),
                    "pack_splat({a}, {b}) produced {v}, outside [0, 65535]."
                );
            }
        }
    }
}

/// §C. A real chunk, on the seed that ships.
///
/// Two independent things, and the second is the one worth having. Every
/// vertex's packed pair must unpack to weights that **sum to ~255**, which is
/// `sim_core::terrain::splat_from`'s own documented contract ("as bytes
/// summing to ~255") and therefore a check against a source outside this file.
/// A packing that scrambled the byte positions, or dropped one, would still
/// round-trip in §A and would fail here.
///
/// The band is `[251, 259]` rather than exactly 255 because `splat_from`
/// normalizes to 255 and then rounds each of four bytes independently, so the
/// sum carries up to four half-ulp roundings in the same direction.
///
/// ⚠ **This does NOT catch a permutation, and an earlier draft of this comment
/// claimed it did.** A sum is symmetric, so pack and unpack could agree on a
/// scrambled byte order and every assertion here would still pass — which is
/// `CLAUDE.md`'s byte-golden trap exactly ("the right value in the wrong
/// position"). §E is the one that checks positions.
#[test]
fn a_shipped_chunk_carries_weights_that_still_sum_to_255() {
    for (label, n, step, drop) in [
        ("near", NEAR_N, 1.0f32, 0.0f32),
        ("far", FAR_N, FAR_STEP, FAR_DROP),
    ] {
        let mesh = heightfield(SEED, 0.0, 0.0, n, step, drop);
        let uv1 = mesh
            .attribute(Mesh::ATTRIBUTE_UV_1)
            .unwrap_or_else(|| panic!("{label} mesh carries no ATTRIBUTE_UV_1"));
        let VertexAttributeValues::Float32x2(uv1) = uv1 else {
            panic!("{label} mesh's ATTRIBUTE_UV_1 is not Float32x2");
        };

        let verts = n * n;
        assert_eq!(
            uv1.len(),
            verts,
            "{label} mesh has {} UV_1 entries for {verts} vertices",
            uv1.len()
        );

        let (mut worst_lo, mut worst_hi) = (u32::MAX, 0u32);
        for pair in uv1 {
            let w = unpack_splat(*pair);
            // Round-trips on real data, not just on the swept lattice.
            assert_eq!(
                pack_splat(w),
                *pair,
                "{label}: a real vertex's pair {pair:?} does not survive a \
                 round trip (unpacked {w:?})"
            );
            let sum = u32::from(w[0]) + u32::from(w[1]) + u32::from(w[2]) + u32::from(w[3]);
            worst_lo = worst_lo.min(sum);
            worst_hi = worst_hi.max(sum);
        }
        assert!(
            (251..=259).contains(&worst_lo) && (251..=259).contains(&worst_hi),
            "{label} chunk's unpacked weights sum to [{worst_lo}, {worst_hi}], \
             outside splat_from's ~255 contract.\n\
             A sum far from 255 means the byte positions were scrambled by the \
             pack, which §A cannot see because it round-trips whatever it is \
             given."
        );
    }
}

/// §E. **Which slot is which identity** — the role check.
///
/// `pack_splat`'s four bytes are sand · grass · litter · rock, and so are
/// `GROUND_MAP_GAIN`'s four gains and `ground_splat.wgsl`'s four texture
/// bindings. Three orderings, agreed by nothing the compiler can see: every
/// one of them is `[f32; 4]` or four `u8`, so a permutation type-checks, round
/// -trips through §A, and sums to 255 in §C. It draws granite's relief on turf.
///
/// That is `CLAUDE.md`'s byte-golden trap in the exact form
/// `sim-core/tests/event_roles.rs` was written for one layer down, and this is
/// the same construction: pin each slot to a semantic fact about the world
/// rather than to a value. `terrain::splat_from` reaches each identity purely
/// under conditions that are documented in its own body — below the beach band
/// is sand, the cliff mask FORCES rock, and moisture is what separates turf
/// from forest litter — so each row here is an independent statement about
/// which index means what.
#[test]
fn each_slot_survives_the_pack_as_the_identity_it_started_as() {
    // (label, height, moisture, slope, the index that must dominate)
    let cases = [
        ("sand below the beach band", 0.0, 0.0, 0.0, 0usize),
        ("turf: inland, dry, flat", 20.0, 0.0, 0.0, 1),
        ("forest litter: inland, wet, flat", 20.0, 0.5, 0.0, 2),
        ("granite: the cliff mask vetoes", 20.0, 0.0, 10.0, 3),
    ];

    for (label, h, moist, slope, want) in cases {
        let w = sim_core::terrain::splat_from(h, moist, slope);
        let argmax = |v: [u8; 4]| {
            (0..4)
                .max_by_key(|&i| v[i])
                .expect("four elements is not empty")
        };

        // The premise: `splat_from` really does resolve this case to that slot.
        // If this half fails, the world changed and the row is stale — which is
        // worth knowing on its own and is why it asserts separately.
        assert_eq!(
            argmax(w),
            want,
            "{label}: splat_from({h}, {moist}, {slope}) = {w:?}, dominant slot \
             {} rather than {want}.\n\
             The premise of this row is stale — `splat_from`'s bands moved.",
            argmax(w)
        );

        // The claim: the ride to the fragment preserves it.
        let got = unpack_splat(pack_splat(w));
        assert_eq!(
            got, w,
            "{label}: {w:?} came back from the pack as {got:?} — the identities \
             were permuted in transit."
        );
    }
}

/// §D. The divisor both implementations share.
///
/// The WGSL twin cannot be executed from here, so what is pinned instead is
/// the one number a divergence would turn on. If someone re-bases the packing
/// on a different radix in Rust, this fails and points at the shader; the
/// shader's own comment points back here. That is the cheapest available stand
/// -in for running both — and it is the same construction `CLAUDE.md`
/// prescribes for the vendored SDK, where the check is a command rather than a
/// gate because nothing can gate a file in another tree.
#[test]
fn the_radix_is_256_and_the_shader_is_told_so() {
    // High byte and low byte, isolated: if the radix moved, exactly one of
    // these two is wrong, and which one says which direction it moved.
    assert_eq!(
        pack_splat([1, 0, 0, 0])[0],
        256.0,
        "the high byte's radix moved"
    );
    assert_eq!(
        pack_splat([0, 1, 0, 0])[0],
        1.0,
        "the low byte is not the unit"
    );

    // Built from the constant the MATERIAL loads by, not from a second copy of
    // the path — otherwise this gate can pass while pointing at a file the
    // renderer no longer reads, which is the same class of false green as a
    // stale doc citing a deleted gate.
    let shader = format!(
        "{}/../../assets/{}",
        env!("CARGO_MANIFEST_DIR"),
        GROUND_SPLAT_SHADER
    );
    let src = std::fs::read_to_string(&shader).unwrap_or_else(|e| {
        panic!(
            "the ground splat shader is not on disk at {shader}: {e}\n\
             It ships — the material in `render/terrain_mesh.rs` loads it by path."
        )
    });
    assert!(
        src.contains("256.0"),
        "{GROUND_SPLAT_SHADER} does not mention the radix 256.0.\n\
         Its unpack must be the same arithmetic as `unpack_splat` in \
         `render/terrain_mesh.rs`; if it was rewritten, re-read both."
    );
}
