//! Gate: a generated SITE model is the volume the sim blocks.
//!
//! **This is `tests/deploy_assets.rs`'s job against a different contract, and
//! the difference is the whole reason this file exists rather than a sixth
//! test over there.**
//!
//! A `DEPLOY` row is a render row. Nothing in `sim-core` reads it, so a model
//! is fitted UNIFORMLY inside it and one that comes up short is, in
//! `ci/import_meshy.py`'s own words, "a row to re-measure, not a mesh to
//! stretch". The two authored sites are the opposite case:
//! `terrain::SHELTER_BOXES` and `WAYSTATION_CANOPY_BOXES` are the sim's
//! **collision volume**, and `OCCUPANT_R_M` / `OCCUPANT_TOP_M` are not
//! approximations of them — they are *defined* as their bounds
//! (`SHELTER_CORNER_R_M`'s doc: the plinth's half-diagonal, rounded up;
//! `SHELTER_PEAK_M`: the tower-cap's top). So the drawn thing and the blocked
//! thing are one object, and the box massing satisfied that by construction
//! because it WAS the table.
//!
//! A generated model is not the table, and under a uniform fit it misses in
//! whichever direction its own aspect happens to differ. Measured on the pair
//! that shipped: the shelter's model would have left **1.51 m of blocked air
//! above its roof**, and the canopy's **1.26 m of invisible skirt on each
//! horizontal axis** — a body stopped by nothing it can see, which is exactly
//! the defect `greybox.rs`'s `SLACK_R_M` was closed to a millimetre to stop.
//! They are imported with `--fit-axes` instead, which scales each axis to its
//! own target. This file is what holds that.
//!
//! **What it deliberately does not do is relax
//! `the_authored_pair_bounds_equal_what_the_sim_publishes`.** That test still
//! measures `archetype_mesh` — the box massing — against the published
//! scalars at a millimetre, because that pair is still definitionally equal
//! and is still the fallback draw. This file adds the second claim the model
//! introduced: the ASSET agrees with them too.
//!
//! It runs headless. A GLB header is JSON, a vertex buffer is little-endian
//! f32, and a bounding radius is arithmetic — the same tier as
//! `tests/deploy_assets.rs` and `tests/tree.rs`, and the same reason.

#![cfg(feature = "render")]

use client::render::props::{archetype_lift, site_asset};
use sim_core::terrain::{Occupant, OCCUPANT_R_M, OCCUPANT_TOP_M};

/// The occupants this file covers, written out rather than derived, so a
/// third site added to `site_asset` without a row here is caught by
/// [`every_site_with_a_model_is_covered_here`] instead of by nothing.
const SITES: [Occupant; 2] = [Occupant::HavenShelter, Occupant::WaystationCanopy];

/// Assets live beside the crate, not inside it — `assets/` is what the depot
/// ships and `crates/client/` is what cargo builds. Same hop `tests/ui.rs` and
/// `tests/deploy_assets.rs` make.
fn asset_path(rel: &str) -> std::path::PathBuf {
    std::path::Path::new("../../assets").join(rel)
}

/// A GLB parsed far enough to measure it the way the renderer would.
///
/// **It reads VERTICES where `tests/deploy_assets.rs` reads the accessor's
/// declared `min`/`max`, and that is the point rather than an accident.** The
/// number this file has to compare against `OCCUPANT_R_M` is
/// `render::tree::bounds`' — the largest `hypot(x, z)` over vertices — and a
/// bounding box's corner is not that number unless some vertex actually sits
/// on the corner. On the shipped shelter the gap between the two readings is
/// 21 mm, so taking the cheap one would be comparing the sim against a radius
/// nothing draws.
struct Glb {
    json: serde_json::Value,
    bin: Vec<u8>,
}

impl Glb {
    fn open(path: &std::path::Path) -> Self {
        let raw = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert_eq!(&raw[0..4], b"glTF", "{}: not a GLB", path.display());
        let mut off = 12;
        let (mut json, mut bin) = (None, Vec::new());
        while off + 8 <= raw.len() {
            let len = u32::from_le_bytes(raw[off..off + 4].try_into().unwrap()) as usize;
            let kind = u32::from_le_bytes(raw[off + 4..off + 8].try_into().unwrap());
            let body = &raw[off + 8..off + 8 + len];
            match kind {
                0x4E4F_534A => json = Some(serde_json::from_slice(body).expect("bad JSON chunk")),
                0x004E_4942 => bin = body.to_vec(),
                _ => {}
            }
            off += 8 + len;
        }
        Self {
            json: json.expect("GLB has no JSON chunk"),
            bin,
        }
    }

    fn primitives(&self) -> Vec<&serde_json::Value> {
        self.json["meshes"]
            .as_array()
            .map(|ms| {
                ms.iter()
                    .filter_map(|m| m["primitives"].as_array())
                    .flatten()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Every POSITION, in metres, as the renderer will see it — vertices, not
    /// a declared box.
    fn positions(&self) -> Vec<[f32; 3]> {
        let mut out = Vec::new();
        for p in self.primitives() {
            let ai = p["attributes"]["POSITION"].as_u64().expect("no POSITION") as usize;
            let a = &self.json["accessors"][ai];
            assert_eq!(
                a["componentType"].as_u64(),
                Some(5126),
                "POSITION is not f32"
            );
            assert_eq!(a["type"].as_str(), Some("VEC3"), "POSITION is not VEC3");
            let bv = &self.json["bufferViews"][a["bufferView"].as_u64().unwrap() as usize];
            let start = bv["byteOffset"].as_u64().unwrap_or(0) as usize
                + a["byteOffset"].as_u64().unwrap_or(0) as usize;
            let stride = bv["byteStride"].as_u64().unwrap_or(12) as usize;
            assert_eq!(stride, 12, "interleaved POSITION is not handled");
            for i in 0..a["count"].as_u64().unwrap() as usize {
                let o = start + i * 12;
                let f = |k: usize| f32::from_le_bytes(raw4(&self.bin[o + k * 4..o + k * 4 + 4]));
                out.push([f(0), f(1), f(2)]);
            }
        }
        out
    }

    /// The pair `render::tree::bounds` returns: peak above the mesh's own
    /// origin, and the largest horizontal radius about it.
    fn peak_and_radius(&self) -> (f32, f32) {
        let (mut h, mut r) = (0.0f32, 0.0f32);
        for v in self.positions() {
            h = h.max(v[1]);
            r = r.max((v[0] * v[0] + v[2] * v[2]).sqrt());
        }
        (h, r)
    }

    fn min_y(&self) -> f32 {
        self.positions()
            .iter()
            .map(|v| v[1])
            .fold(f32::MAX, f32::min)
    }

    fn triangles(&self) -> usize {
        self.primitives()
            .iter()
            .map(|p| {
                let i = p["indices"].as_u64().expect("no indices") as usize;
                self.json["accessors"][i]["count"].as_u64().unwrap() as usize / 3
            })
            .sum()
    }
}

fn raw4(s: &[u8]) -> [u8; 4] {
    s.try_into().unwrap()
}

fn glb_of(o: Occupant) -> (&'static str, Glb) {
    let rel = site_asset(o).unwrap_or_else(|| panic!("{o:?} has no model"));
    (rel, Glb::open(&asset_path(rel)))
}

/// Every occupant the sim can place, as `greybox.rs` keeps the same list.
///
/// ⚠ **Not in discriminant order, and it cannot be**: `Occupant` skips 8 on
/// purpose (the client's archetype table's slot 8 is the felled-pine stump,
/// which is a consequence of an occupant rather than one), so the enum is not
/// dense and an index-equals-position check is wrong here. The first draft of
/// this file asserted exactly that and failed on its first run — which is the
/// check working, on the wrong claim.
///
/// **Completeness is `greybox.rs` §C's job**, not this file's: it holds the
/// count. What is asserted below is narrower and is the one this file needs —
/// that the set of occupants declaring a model is the set covered here.
const ALL: [Occupant; 12] = [
    Occupant::None,
    Occupant::Tree,
    Occupant::StoneNode,
    Occupant::MetalNode,
    Occupant::SulfurNode,
    Occupant::Bush,
    Occupant::Rock,
    Occupant::BarrelSlot,
    Occupant::CrateSlot,
    Occupant::CacheSlot,
    Occupant::HavenShelter,
    Occupant::WaystationCanopy,
];

#[test]
fn every_site_with_a_model_is_covered_here() {
    // A duplicate would let a variant hide behind its twin and never be
    // filtered into `declared`.
    for (i, o) in ALL.iter().enumerate() {
        assert!(
            !ALL[..i].contains(o),
            "{o:?} appears twice in ALL — the mirror has drifted"
        );
    }
    // The claim: `site_asset` is the authority and `SITES` is this file's
    // coverage. A model added there without a row here would skip every
    // assertion below and report nothing.
    let declared: Vec<Occupant> = ALL
        .iter()
        .copied()
        .filter(|o| site_asset(*o).is_some())
        .collect();
    assert_eq!(
        declared,
        SITES.to_vec(),
        "site_asset declares models for {declared:?} and this file covers \
         {SITES:?}"
    );
}

#[test]
fn every_declared_model_ships() {
    for o in SITES {
        let rel = site_asset(o).unwrap();
        assert!(
            asset_path(rel).exists(),
            "{o:?} declares {rel} and it is not in the tree — the site would \
             fall back to the box massing with nothing saying so"
        );
    }
}

#[test]
fn each_model_is_the_single_primitive_site_models_loads() {
    for o in SITES {
        let (rel, g) = glb_of(o);
        let n = g.primitives().len();
        assert_eq!(
            n, 1,
            "{o:?} ({rel}) has {n} primitives and `SiteModels::load` asks for \
             `Primitive {{ mesh: 0, primitive: 0 }}` — it would draw 1/{n} of \
             this model and report no error"
        );
        let mats = g.json["materials"].as_array().map_or(0, |m| m.len());
        assert_eq!(
            mats, 1,
            "{o:?} ({rel}) has {mats} materials and `SiteModels::load` asks \
             for `Material {{ index: 0 }}` — the rest would never be applied"
        );
    }
}

/// How far SHORT of the blocked volume a model may be drawn, in metres.
///
/// **Asymmetric on purpose, and the two directions are not the same defect.**
/// Drawn OUTSIDE what the sim blocks is a player walking through stone they
/// can see, so that direction gets float slack and nothing else. Drawn short
/// is a body stopped by air — worse than it sounds, since it is invisible and
/// therefore unreportable, but bounded by how far in it reaches.
///
/// Five centimetres, and the number is picked against the body rather than
/// against the measurement: it is a fifth of `collide::WALL_THICKNESS_M`
/// (0.24) and an order under `STEP_UP`, so a gap this size cannot hold a
/// player anywhere they would notice being held. **Measured on the shipped
/// pair it is 21.4 mm (shelter) and 0.2 mm (canopy)** — the shelter's is
/// larger because its extreme-x and extreme-z vertices are not the same
/// vertex, so the corner the box massing owns is chamfered away by the
/// generator. The headroom over that is deliberate: these assets are
/// regenerable by design (`DECISIONS.md` 2026-08-11 — "replaceable by a file
/// swap with no code change"), and a ratchet pinned to today's mesh would go
/// red on a re-roll that is no worse.
///
/// It is a physical allowance, not a budget. Raising it is re-opening the
/// invisible skirt; the fix for a model that fails is a re-import, or a
/// reference image whose aspect is closer to the box table's.
const SITE_SHORT_M: f32 = 0.05;

/// Drawn outside what the sim stops a body at. A tenth of a millimetre, which
/// is the same float slack `deploy_assets.rs` allows an importer's arithmetic.
const SITE_OVER_M: f32 = 1.0e-4;

#[test]
fn each_model_is_the_volume_the_sim_blocks() {
    for o in SITES {
        let (rel, g) = glb_of(o);
        let (peak, radius) = g.peak_and_radius();
        // These meshes stand on their own base rather than being centred, so
        // the lift is zero and the peak IS `OCCUPANT_TOP_M`. Read it rather
        // than assuming it, so a lift added later fails here loudly.
        let lift = archetype_lift(o);
        let top = lift + peak;
        let (r_pub, top_pub) = (OCCUPANT_R_M[o as usize], OCCUPANT_TOP_M[o as usize]);

        assert!(
            radius <= r_pub + SITE_OVER_M,
            "{o:?} ({rel}) draws out to {radius:.4} m against a blocked radius \
             of {r_pub:.4} — a player would pass through geometry they can see"
        );
        assert!(
            top <= top_pub + SITE_OVER_M,
            "{o:?} ({rel}) draws up to {top:.4} m against a blocked top of \
             {top_pub:.4} — the roof stands above what stops a body"
        );
        assert!(
            r_pub - radius <= SITE_SHORT_M,
            "{o:?} ({rel}) draws to {radius:.4} m inside a blocked radius of \
             {r_pub:.4} — {:.4} m of invisible skirt, past SITE_SHORT_M. \
             Re-import with `--fit-axes`, or the model's aspect is too far \
             from the box table's to stretch.",
            r_pub - radius
        );
        assert!(
            top_pub - top <= SITE_SHORT_M,
            "{o:?} ({rel}) draws to {top:.4} m under a blocked top of \
             {top_pub:.4} — {:.4} m of blocked air above the roof, past \
             SITE_SHORT_M. This is the exact reading a UNIFORM fit produced \
             (1.51 m on the shelter); check the import used `--fit-axes`.",
            top_pub - top
        );
    }
}

#[test]
fn every_model_stands_on_the_ground() {
    for o in SITES {
        let (rel, g) = glb_of(o);
        let y = g.min_y();
        // `spawn_slot` seats these by their own origin at `archetype_lift` = 0
        // above the slot's ground, so a model centred on its middle sinks half
        // of itself into the terrain.
        assert!(
            y.abs() < 1.0e-3,
            "{o:?} ({rel}) has min.y = {y:+.4} — it is not authored with its \
             base at zero and would sit buried or float"
        );
    }
}

#[test]
fn nothing_on_a_site_glows() {
    for o in SITES {
        let (rel, g) = glb_of(o);
        let lit = g.json["materials"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|m| match m.get("emissiveFactor") {
                Some(f) => (0..3).any(|k| f[k].as_f64().unwrap_or(0.0) > 1.0e-6),
                // glTF's default is [0,0,0], so absent is dark.
                None => false,
            });
        assert!(
            !lit,
            "{o:?} ({rel}) is emissive. The generator ships \
             `emissiveFactor = [1,1,1]` on nearly everything and its map is \
             usually junk — a wooden spear measured a 0.53 peak. Neither of \
             these structures burns; `ci/import_meshy.py` strips it unless \
             `--emissive` is passed, so this is an import that skipped the \
             strip."
        );
    }
}

/// Triangles a site model may carry.
///
/// A site stands once (the pad) or twice (the waystations) on an island and is
/// never instanced, so this is nowhere near `RENDER.md` §6's 1.5 M frame
/// ceiling — a single conifer is ~6 k and a scatter ring holds dozens. The
/// ceiling exists so a re-roll that comes back at 200 k is caught before it
/// ships, not because 4 k is expensive. Measured: shelter 4,140, canopy 3,801.
const SITE_TRI_MAX: usize = 12_000;

#[test]
fn a_site_model_is_within_its_triangle_ceiling() {
    for o in SITES {
        let (rel, g) = glb_of(o);
        let t = g.triangles();
        assert!(
            t <= SITE_TRI_MAX,
            "{o:?} ({rel}) is {t} triangles against a ceiling of \
             {SITE_TRI_MAX} — remesh it rather than raising this"
        );
    }
}

#[test]
fn a_site_models_textures_are_compressed_in_vram() {
    for o in SITES {
        let (rel, g) = glb_of(o);
        let imgs = g.json["images"].as_array().cloned().unwrap_or_default();
        assert!(!imgs.is_empty(), "{o:?} ({rel}) carries no texture at all");
        for (i, im) in imgs.iter().enumerate() {
            // A JPEG in a `.glb` decompresses to full RGBA8 on the GPU — a
            // 2048² map is 16.8 MB of VRAM whatever it weighs on disk. KTX2
            // stays compressed there. `ci/ktx_pack.py` has the four encodings
            // that were measured before 1K UASTC was chosen, and the reason
            // the mime type rather than `KHR_texture_basisu` is what marks it
            // (Bevy 0.18 does not implement that extension and rejects the
            // spec-correct file outright).
            assert_eq!(
                im["mimeType"].as_str(),
                Some("image/ktx2"),
                "{o:?} ({rel}) image {i} is {:?} — run ci/ktx_pack.py, or this \
                 one prop costs ~50 MB of video memory",
                im["mimeType"].as_str()
            );
        }
    }
}
