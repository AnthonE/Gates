//! Weathering on the monuments: what a photographed tile cannot say about a
//! wall that has stood out in the rain for decades.
//!
//! The dressed sites (`town.rs`, `ziggurat.rs`, `landmarks.rs`) wear the
//! game's own photographed surfaces at a metre tile. Up close that is
//! stone; from thirty metres every block of a wall is the same value and the
//! tile repeats (`ART.md` §2 rules 1 and 7). This is the layer `ART.md` §7
//! calls the variation layer, in world space so no two walls and no two
//! landmarks of one kind share it:
//!
//! - **Mottle**: a value break-up at ~1.5 m and ~6 m.
//! - **Streaks**: rain stains (rust, on steel) running down vertical faces,
//!   banded along the face.
//! - **Moss**: patches on upward faces, and lichen crusts on the sides.
//!
//! `StandardMaterial` plus a fragment stage (`weathering.wgsl`); the prepass
//! and shadows are the base material's, since nothing moves.

use bevy::asset::Asset;
use bevy::pbr::{ExtendedMaterial, MaterialExtension, StandardMaterial};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;

use super::depot::Surface;

/// A weathered surface: `StandardMaterial` plus the stains.
pub type MonumentMaterial = ExtendedMaterial<StandardMaterial, Weathering>;

pub const SHADER: &str = "shaders/weathering.wgsl";

/// The per-surface uniform; each lane is named in `weathering.wgsl`.
#[derive(Clone, Copy, Debug, Default, PartialEq, ShaderType)]
pub struct WeatherParams {
    /// x amplitude (± of the value), y the fine frequency /m, z the broad.
    pub mottle: Vec4,
    /// x strength, y frequency across the face /m.
    pub streak: Vec4,
    /// rgb what a streak multiplies the surface by.
    pub streak_tint: Vec4,
    /// x moss on upward faces, y lichen crusts on the sides.
    pub moss: Vec4,
    /// rgb the moss's linear albedo.
    pub moss_tint: Vec4,
}

/// The extension. **Not `#[bindless]`**, for `ground_splat.rs`'s reason.
#[derive(Asset, AsBindGroup, TypePath, Clone)]
pub struct Weathering {
    #[uniform(100)]
    pub params: WeatherParams,
}

impl MaterialExtension for Weathering {
    fn fragment_shader() -> ShaderRef {
        SHADER.into()
    }
    /// Forward only: this client never runs deferred.
    fn deferred_fragment_shader() -> ShaderRef {
        ShaderRef::Default
    }
}

/// Moss: a dark olive, inside `ART.md` §5's albedo band. **(knob)**
const MOSS: [f32; 3] = [0.07, 0.09, 0.035];
/// Rain stains on stone and concrete: darker and a little warm. **(knob)**
const GRIME: [f32; 3] = [0.5, 0.47, 0.42];
/// Rust run off a fixing. **(knob)**
const RUST: [f32; 3] = [0.78, 0.45, 0.24];
/// Lime washed out of black stone: its stains run pale. **(knob)**
const BLOOM: [f32; 3] = [1.45, 1.45, 1.4];

/// How a surface weathers, or `None` for one that does not (gilt, the glow).
pub fn params(surface: Surface) -> Option<WeatherParams> {
    // (mottle, streak, streak tint, moss on tops, lichen)
    let (mottle, streak, tint, moss, lichen) = match surface {
        Surface::Ashlar | Surface::Stone => (0.2, 0.8, GRIME, 0.8, 0.75),
        // Polished, and kept: no moss on the ancient stone, and any would
        // be brighter than it.
        Surface::Obsidian => (0.12, 0.5, BLOOM, 0.0, 0.0),
        Surface::Concrete | Surface::Paint => (0.16, 0.85, GRIME, 0.5, 0.3),
        Surface::Yard => (0.18, 0.0, GRIME, 0.3, 0.0),
        Surface::Timber => (0.14, 0.45, [0.7, 0.68, 0.66], 0.45, 0.2),
        Surface::Steel | Surface::Sheet | Surface::Roof | Surface::Cargo => {
            (0.14, 0.75, RUST, 0.0, 0.0)
        }
        Surface::Canvas => (0.1, 0.3, GRIME, 0.0, 0.0),
        Surface::Gilt | Surface::Lapis | Surface::Bulb => return None,
    };
    Some(WeatherParams {
        mottle: Vec4::new(mottle, 0.65, 0.17, 0.0),
        streak: Vec4::new(streak, 2.2, 0.0, 0.0),
        streak_tint: Vec4::new(tint[0], tint[1], tint[2], 0.0),
        moss: Vec4::new(moss, lichen, 0.0, 0.0),
        moss_tint: Vec4::new(MOSS[0], MOSS[1], MOSS[2], 0.0),
    })
}

/// A dressed site's material for one surface: weathered, or plain.
#[derive(Clone)]
pub enum Dressed {
    Plain(Handle<StandardMaterial>),
    Weathered(Handle<MonumentMaterial>),
}

impl Dressed {
    /// The plain handle, for the surfaces something else animates (the
    /// town's bulbs and lapis follow the night).
    pub fn plain(&self) -> Option<&Handle<StandardMaterial>> {
        match self {
            Dressed::Plain(h) => Some(h),
            Dressed::Weathered(_) => None,
        }
    }

    /// Put this material on an entity.
    pub fn insert(&self, e: &mut EntityCommands) {
        match self {
            Dressed::Plain(h) => e.insert(MeshMaterial3d(h.clone())),
            Dressed::Weathered(h) => e.insert(MeshMaterial3d(h.clone())),
        };
    }
}

/// `base` weathered as `surface` weathers, into the right asset store.
pub fn dress(
    surface: Surface,
    base: StandardMaterial,
    plain: &mut Assets<StandardMaterial>,
    weathered: &mut Assets<MonumentMaterial>,
) -> Dressed {
    match params(surface) {
        Some(params) => Dressed::Weathered(weathered.add(ExtendedMaterial {
            base,
            extension: Weathering { params },
        })),
        None => Dressed::Plain(plain.add(base)),
    }
}

pub fn plugin(app: &mut App) {
    app.add_plugins(MaterialPlugin::<MonumentMaterial>::default());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::depot::SURFACES;

    /// Every surface answers, the glowing and gilded ones stay plain, and no
    /// stain or moss can push a surface out of `ART.md` §5's albedo band on
    /// its own (a multiply of a value already in the band, by at most 1 —
    /// except the pale bloom on black stone, which only black stone wears).
    #[test]
    fn the_weathered_set_is_the_dull_one() {
        for s in SURFACES {
            let p = params(s);
            match s {
                Surface::Gilt | Surface::Lapis | Surface::Bulb => assert!(p.is_none(), "{s:?}"),
                _ => {
                    let p = p.unwrap_or_else(|| panic!("{s:?} unweathered"));
                    assert!(p.mottle.x > 0.0 && p.mottle.x < 0.25, "{s:?}");
                    if s != Surface::Obsidian {
                        assert!(p.streak_tint.truncate().max_element() <= 1.0, "{s:?}");
                    }
                    assert!(p.moss_tint.truncate().max_element() <= 0.55, "{s:?}");
                }
            }
        }
    }
}
