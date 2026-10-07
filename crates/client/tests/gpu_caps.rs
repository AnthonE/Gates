//! Gate: the WebGPU texture budget turns rows off in the right order, and
//! only rows that would have cost the device a frame.
//!
//! [`GpuCaps`] is read once per process from the device, so this file is its
//! own test binary: [`set_gpu_caps`] here cannot leak into `quality.rs`,
//! which runs every row unconstrained.

use client::config::{Ao, Quality};
use client::render::quality::{
    effective, gpu_caps, preset, set_gpu_caps, Gfx, GpuCaps, FRAGMENT_TEXTURES,
};

fn caps(sampled: u32, storage: u32) -> GpuCaps {
    GpuCaps {
        sampled_textures: sampled,
        storage_textures: storage,
        float32_filterable: true,
        rg11b10_renderable: true,
        dual_source_blending: true,
    }
}

#[test]
fn the_room_is_what_a_prepass_can_still_bind() {
    // WebGPU's floor: the ground under the atmosphere fills it exactly.
    let floor = caps(FRAGMENT_TEXTURES, 4);
    assert_eq!(floor.prepass_room(), 0);
    assert!(!floor.ao(false) && !floor.taa(false));
    // Two spare: depth + one of normal (AO) or motion (TAA), never both.
    let two = caps(FRAGMENT_TEXTURES + 2, 8);
    assert!(two.ao(false) && two.taa(false));
    assert!(!two.ao(true) && !two.taa(true));
    // Chrome's upper tier: everything.
    let tier = caps(48, 8);
    assert!(tier.ao(true) && tier.taa(true));
    // AO also wants five storage textures and filterable 32-bit floats,
    // whatever the room — Bevy's SSAO plugin declines without them.
    assert!(!caps(48, 4).ao(false));
    let unfilterable = GpuCaps {
        float32_filterable: false,
        ..tier
    };
    assert!(!unfilterable.ao(false) && unfilterable.taa(true));
    assert!(GpuCaps::UNBOUNDED.ao(true) && GpuCaps::UNBOUNDED.taa(true));
}

#[test]
fn with_room_for_one_the_occlusion_stays_and_taa_goes() {
    set_gpu_caps(caps(FRAGMENT_TEXTURES + 2, 8));
    assert_eq!(gpu_caps(), caps(FRAGMENT_TEXTURES + 2, 8));
    let high = preset(Quality::High);
    assert!(high.ao != Ao::Off && high.taa, "High asks for both");
    let got = effective(high);
    assert_ne!(got.ao, Ao::Off, "occlusion is kept: it pays for the fill");
    assert!(!got.taa, "TAA is the one that does not fit");
    assert!(got.smaa, "SMAA stands in, at no texture cost");
    assert_eq!(effective(got), got, "the clamp is idempotent");
    // SMAA stands in for a refused TAA even where its own row was switched
    // off while TAA replaced it: a player who chose TAA chose anti-aliasing.
    let smaa_off = effective(Gfx {
        smaa: false,
        ..high
    });
    assert!(!smaa_off.taa && smaa_off.smaa);
    assert_eq!(effective(smaa_off), smaa_off);
    // A player who turns the occlusion off gets TAA back in its room.
    let no_ao = effective(Gfx {
        ao: Ao::Off,
        ..high
    });
    assert!(no_ao.taa);
}
