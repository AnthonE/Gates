//! Gate: the forward decal's prerequisites are still on the camera, and
//! they are held in a file that does not know this module exists.
//!
//! # Why a source scrape and not a runtime check
//!
//! `render::decal` draws `ForwardDecal`s, and Bevy's own usage note for
//! them is unambiguous: *"Any camera rendering a forward decal must have
//! the `DepthPrepass` component."* Ours does — but **not because anything
//! asked for it**. `rig.rs` carries `ScreenSpaceAmbientOcclusion` for the
//! lighting slice's sake, and in Bevy 0.18 that component is
//! `#[require(DepthPrepass, NormalPrepass)]`, so the prepass is up as a
//! side effect of an unrelated decision. `Msaa::Off` is there for the same
//! borrowed reason (SSAO warns without it; Bevy's decal note asks for it
//! on wasm).
//!
//! That is a dependency across a seam with nothing holding it. Someone
//! rebalancing the lighting could drop the AO, gain back a pass, and every
//! gate in this repo would stay green while marks quietly stopped blending
//! against the surfaces they lie on — a defect that shows up as *slightly
//! wrong shading*, which is the hardest kind to attribute and the kind
//! `CLAUDE.md` says a person looking is supposed to catch. A person
//! looking will not catch this one; they will see a decal and assume
//! decals look like that.
//!
//! So this reads `rig.rs` and fails with the sentence, which is
//! `tests/sound.rs`'s shape: **the defect is a call site, not a value**,
//! so the check is a grep for the call site. What it deliberately does NOT
//! do is assert *how* the prepass gets there — if someone adds
//! `DepthPrepass` explicitly and drops the AO, that is a correct fix and
//! this gate says so.
//!
//! ⚠ It is a hand-kept mirror of another file's contents, which
//! `CLAUDE.md` warns about twice. The mitigation is that it mirrors a
//! *predicate* rather than a number: there is no count here to drift.

#![cfg(feature = "render")]

const RIG: &str = include_str!("../src/render/rig.rs");

/// The camera still carries something that brings a depth prepass with it.
#[test]
fn the_camera_still_carries_the_prepass_a_forward_decal_needs() {
    let has_ssao = RIG.contains("ScreenSpaceAmbientOcclusion {");
    let has_explicit = RIG.contains("DepthPrepass");
    assert!(
        has_ssao || has_explicit,
        "`render/rig.rs` no longer spawns the camera with either \
         `ScreenSpaceAmbientOcclusion` (which is \
         `#[require(DepthPrepass, NormalPrepass)]` in Bevy 0.18) or an \
         explicit `DepthPrepass`.\n\n\
         `render::decal` draws `ForwardDecal`s, and a forward decal \
         without a depth prepass on its camera cannot read the surface \
         behind it — so it stops fading toward its edges and renders at \
         full opacity instead. That is `ART.md` rule 2's \"a clean \
         intersection edge reads as a decal\" arriving as a literal one, \
         and no gate in this repo scores a frame.\n\n\
         If the AO was removed deliberately, add `DepthPrepass` to the \
         camera bundle in `rig.rs` and this gate goes green again."
    );
}

/// MSAA is still off.
///
/// SSAO refuses to run with it on and says so in a log line, which is the
/// loud half. The quiet half is the decal's: Bevy's usage note flags MSAA
/// as a constraint for forward decals on wasm, and while this client is
/// native, an `Msaa` setting that changed under the renderer is exactly
/// the kind of thing that gets changed for one pass's sake without the
/// other passes being re-checked.
#[test]
fn msaa_is_still_off_for_the_passes_that_need_it_off() {
    assert!(
        RIG.contains("Msaa::Off"),
        "`render/rig.rs` no longer sets `Msaa::Off` on the camera. SSAO \
         refuses to run without it (and logs), and the forward decals in \
         `render::decal` share the constraint. Turning MSAA on is a \
         decision about at least three passes, not one."
    );
}
