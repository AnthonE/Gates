//! Gate: the held-item models exist, load the way `viewmodel` loads them, and
//! answer to names `content/items.toml` actually uses.
//!
//! **The name assertion is the one that catches a real rename.** Items are
//! resolved by normalised display name and by nothing else, because the wire
//! carries no content id (`ui::hold`'s header argues it). That is cheap and it
//! has one failure mode: rename `"Stone Hatchet"` in content and the lookup
//! quietly stops matching, the model stops drawing, and every gate stays
//! green because the string was never wrong — it was only *unpaired*. So this
//! reads the content file and requires a live item behind every model.
//!
//! The structural half mirrors `tests/deploy_assets.rs`: `viewmodel::swap`
//! loads `Primitive { mesh: 0, primitive: 0 }`, which draws a fraction of a
//! multi-primitive model with no error anywhere.
//!
//! **Two sources, one law.** `HeldSrc::Glb` rows are gated off their file;
//! `HeldSrc::Gen` rows are BUILT here — the same constructors the client runs
//! at startup — and measured the same way, so a generated row whose geometry
//! drifts from its table entry is exactly as red as a regenerated asset
//! nobody re-measured. A `Gen` name with no generator panics right here,
//! which is this gate reaching the boot-time panic before a boot does.

#![cfg(feature = "render")]

use bevy::mesh::VertexAttributeValues;
use client::render::heldgen;
use client::ui::hold::{HeldSrc, HELD_MODELS, SKIN_MODELS};
use client::ui::icons::stem;

fn asset_path(rel: &str) -> std::path::PathBuf {
    std::path::Path::new("../../assets").join(rel)
}

fn glb_json(path: &std::path::Path) -> serde_json::Value {
    let raw = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&raw[0..4], b"glTF", "{}: not a GLB", path.display());
    let len = u32::from_le_bytes(raw[12..16].try_into().unwrap()) as usize;
    serde_json::from_slice(&raw[20..20 + len]).expect("GLB JSON chunk")
}

/// The `.glb` rows, which are the ones with a file to hold to.
/// Every `.glb` a hand can hold: the rows' own, and the skins that bring
/// a model of their own (`hold::SKIN_MODELS`).
fn glb_rows() -> impl Iterator<Item = (&'static str, &'static str)> {
    HELD_MODELS
        .iter()
        .filter_map(|m| match m.src {
            HeldSrc::Glb(path) => Some((m.key, path)),
            HeldSrc::Gen(_) => None,
        })
        .chain(SKIN_MODELS.iter().map(|s| (s.key, s.path)))
}

#[test]
fn every_held_model_ships() {
    for (key, rel) in glb_rows() {
        assert!(
            asset_path(rel).exists(),
            "{key} declares {rel} and it is not in the tree — the item would \
             draw an empty hand forever"
        );
    }
}

#[test]
fn each_held_model_is_the_single_primitive_the_viewmodel_loads() {
    for (key, rel) in glb_rows() {
        let g = glb_json(&asset_path(rel));
        let prims: usize = g["meshes"]
            .as_array()
            .map(|ms| {
                ms.iter()
                    .filter_map(|m| m["primitives"].as_array())
                    .map(|p| p.len())
                    .sum()
            })
            .unwrap_or(0);
        assert_eq!(
            prims, 1,
            "{key} ({rel}) has {prims} primitives and `viewmodel::swap` loads \
             `Primitive {{ mesh: 0, primitive: 0 }}` — it would draw 1/{prims} \
             of the model and report nothing"
        );
        let mats = g["materials"].as_array().map_or(0, |m| m.len());
        assert_eq!(mats, 1, "{key} ({rel}) has {mats} materials, not 1");
    }
}

#[test]
fn nothing_held_glows() {
    // Same rule `deploy_assets.rs` states at length: the generator ships
    // `emissiveFactor = [1,1,1]` on nearly everything and its map is junk —
    // the spear here measured a 0.53 peak before the import stripped it.
    // Nothing a player carries emits light. **The torch did not become the
    // exception**, which is worth stating because this comment predicted it
    // would: torch light v0 gave its row a `PointLight` on the hand
    // (`ui::hold::TORCH_LIGHT`) and left its material black, so a carried
    // light and a carried emissive turned out to be different mechanisms
    // and this test needed neither to grow nor to go.
    // `tests/hand_light.rs` gates the other half.
    //
    // This is also why `fire.glb` — reused for the other deployables' rows —
    // has NO row of its own: the world's fire pit is lit and bakes a full
    // emissive, and a carried unlit one must not glow. Point a row at it and
    // this test is the one that goes red.
    //
    // **A glow skin is the one exception, and it says so** (`SkinModelDef::
    // glow`, Rust's glow skins): its emissive is a map of the cracks it was
    // painted with (`ci/meshy_retexture.py --glow`), never a flat factor.
    let glows = |rel: &str| SKIN_MODELS.iter().any(|s| s.path == rel && s.glow);
    for (key, rel) in glb_rows() {
        let g = glb_json(&asset_path(rel));
        for m in g["materials"].as_array().into_iter().flatten() {
            let lit = m
                .get("emissiveFactor")
                .is_some_and(|f| (0..3).any(|k| f[k].as_f64().unwrap_or(0.0) > 1e-6));
            if glows(rel) {
                assert!(
                    lit && m.get("emissiveTexture").is_some(),
                    "{key} ({rel}) is a glow skin with no emissive map"
                );
                continue;
            }
            assert!(
                !lit,
                "{key} ({rel}) has a non-zero emissiveFactor — it would glow \
                 in the dark. `ci/import_meshy.py` strips this unless \
                 `--emissive` is passed, so the import skipped a step."
            );
        }
    }
}

/// A skin with a model is sold and carried as a picture of that model
/// (`SkinModelDef::icon`, `iconbake --skins`): without one the store falls
/// back to the plain item in a colour, which is the look this replaced.
#[test]
fn every_skin_model_has_its_picture() {
    for s in &SKIN_MODELS {
        let icon = s.icon();
        assert!(
            asset_path(&icon).exists(),
            "{} has no picture at assets/{icon}: run `iconbake --skins` then \
             `ci/finish_icons.py --only-renders --renders <dir> --out assets/icons/skins`",
            s.path
        );
    }
}

#[test]
fn nothing_generated_glows_either() {
    // The generated rows' half of `nothing_held_glows`: the material is code,
    // so the assertion runs on the very value the client will register.
    for m in &HELD_MODELS {
        let HeldSrc::Gen(name) = m.src else { continue };
        let mat = heldgen::material(name);
        let e = mat.emissive;
        assert!(
            e.red <= 1e-6 && e.green <= 1e-6 && e.blue <= 1e-6,
            "{} (generated {name:?}) has emissive {e:?} — nothing a player \
             carries emits light",
            m.key
        );
    }
}

#[test]
fn every_model_answers_to_an_item_that_exists() {
    let toml = std::fs::read_to_string("../../content/items.toml").expect("content/items.toml");
    // `name = "Stone Hatchet"` → `stone_hatchet`, the same normalisation the
    // lookup runs at runtime. Parsed by hand rather than by pulling a TOML
    // dependency into a render-feature test: one key, one shape.
    let names: Vec<String> = toml
        .lines()
        .filter_map(|l| l.trim().strip_prefix("name = "))
        .map(|v| stem(v.trim().trim_matches('"')))
        .collect();
    assert!(
        names.len() > 20,
        "parsed only {} names out of items.toml — the parse is wrong, not the \
         content, and a broken parse would pass every assertion below",
        names.len()
    );
    for m in &HELD_MODELS {
        let key = m.key;
        // A vessel with water in it is drawn by its look (`icons::looks`):
        // `full_waterskin`, `salt_waterskin` — the item behind it is the skin.
        let item = ["full_", "salt_"]
            .iter()
            .find_map(|p| key.strip_prefix(p))
            .unwrap_or(key);
        assert!(
            names.iter().any(|n| n == item),
            "a held row answers to {key:?} and no item in content/items.toml \
             normalises to that. Either the item was renamed — in which case \
             the model silently stopped drawing — or the key is a typo."
        );
    }
}

/// The +Y extent of a built mesh's positions, for the generated rows.
fn mesh_height(mesh: &bevy::prelude::Mesh) -> f32 {
    let Some(VertexAttributeValues::Float32x3(pos)) =
        mesh.attribute(bevy::prelude::Mesh::ATTRIBUTE_POSITION)
    else {
        panic!("a generated held mesh has no Float32x3 POSITION attribute");
    };
    let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
    for p in pos {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    hi[1] - lo[1]
}

#[test]
fn the_declared_height_is_the_models_own_height() {
    // The grip is a fraction of `height_m`, so `height_m` being a
    // *restatement* of what the geometry measures is the whole assumption.
    // Regenerate an asset at a different size, forget this table, and the
    // hand silently moves to the wrong place along the haft — no error, just
    // a spear held by its point. This is the gate for that, and it holds the
    // generated rows to the same number their table line declares.
    //
    // **It measures +Y and not the longest axis, and that is the fix rather
    // than a tidy-up.** `grip_m` spends the fraction up +Y; measuring some
    // other axis here made the gate green on a rock whose fist sat 97% of
    // the way up it and on a building plan whose grip point was 8 cm off the
    // object altogether — both models are wider than they are tall, so the
    // number this test compared was never the number the grip was spent on.
    // With +Y on both sides, `every_grip_is_on_the_object`'s 0..=1 assertion
    // finally means what its name says.
    for m in &HELD_MODELS {
        let height = match m.src {
            HeldSrc::Glb(rel) => {
                let g = glb_json(&asset_path(rel));
                let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
                for p in g["meshes"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|x| x["primitives"].as_array())
                    .flatten()
                {
                    let a = &g["accessors"][p["attributes"]["POSITION"].as_u64().unwrap() as usize];
                    for k in 0..3 {
                        lo[k] = lo[k].min(a["min"][k].as_f64().unwrap() as f32);
                        hi[k] = hi[k].max(a["max"][k].as_f64().unwrap() as f32);
                    }
                }
                hi[1] - lo[1]
            }
            HeldSrc::Gen(name) => mesh_height(&heldgen::mesh(name)),
        };
        let err = (height - m.height_m).abs() / m.height_m;
        assert!(
            err < 0.02,
            "{} declares height_m = {:.3} and the geometry measures {:.3} \
             ({:+.0}%). The grip offset is derived from that number, so the \
             hand is now {:.3} m off along the haft.",
            m.key,
            m.height_m,
            height,
            err * 100.0,
            (height - m.height_m).abs() * m.grip_frac
        );
    }
}

#[test]
fn every_model_stands_on_its_own_feet() {
    // `ci/import_meshy.py`'s convention — authored standing, feet at y = 0 —
    // and the other half of the assumption `grip_m` rests on: the grip is
    // measured UP from the model's origin, so an asset whose origin sits in
    // its middle puts the fist half a model away from where the table says,
    // with `the_declared_height_is_the_models_own_height` still green because
    // an extent says nothing about where it sits.
    for (key, rel) in glb_rows() {
        let g = glb_json(&asset_path(rel));
        let mut lo = f32::MAX;
        for p in g["meshes"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|x| x["primitives"].as_array())
            .flatten()
        {
            let a = &g["accessors"][p["attributes"]["POSITION"].as_u64().unwrap() as usize];
            lo = lo.min(a["min"][1].as_f64().unwrap() as f32);
        }
        assert!(
            lo.abs() < 0.01,
            "{key} ({rel}) has its lowest vertex at y = {lo:+.3} rather than \
             on 0. The grip is measured up from the origin, so the fist is \
             {:.3} m out of the model.",
            lo.abs()
        );
    }
}

#[test]
fn every_grip_is_on_the_object() {
    for m in &HELD_MODELS {
        assert!(
            (0.0..=1.0).contains(&m.grip_frac),
            "{} grips at {} of its own height — outside the model, so the item \
             would float beside the hand rather than in it",
            m.key,
            m.grip_frac
        );
        // A viewmodel may shrink a world-sized object into the hand; nothing
        // is ever drawn LARGER than it was authored, and zero would collapse
        // the mesh while `swap` still pointed a handle at it.
        assert!(
            m.scale > 0.0 && m.scale <= 1.0,
            "{} draws at scale {} — a held model shrinks or ships as-is",
            m.key,
            m.scale
        );
        // Positive by construction: the point `swap` lands on the fist is
        // measured up the model from its own foot.
        assert!(m.grip_m() >= 0.0, "{} grip point is below the foot", m.key);
    }
}

#[test]
fn the_lookup_is_unambiguous() {
    for (i, m) in HELD_MODELS.iter().enumerate() {
        let key = m.key;
        assert!(
            !HELD_MODELS[..i].iter().any(|o| o.key == key),
            "{key:?} appears twice in HELD_MODELS — `position` takes the first \
             and the second model is unreachable"
        );
    }
}

/// Half the height a closed fist occupies on the thing it is holding, in
/// **drawn** metres.
///
/// A hand is about nine centimetres from the heel of the palm to the top of
/// the curled fingers, so this is the band of an item that is actually inside
/// the grip. It is a measurement of a hand and not a tolerance to be widened:
/// the assertion below is about whether there is any model in the fist at
/// all, and every row that passes it clears it by centimetres.
const FIST_HALF_M: f32 = 0.045;

/// One model's triangles: positions and the index buffer that draws them.
type Tris = (Vec<[f32; 3]>, Vec<u32>);

/// A `.glb`'s single primitive, in the model's own frame.
///
/// The other gates in this file read the JSON chunk, because an extent is in
/// `accessors[i].min/max` and needs no geometry. This one cannot: the whole
/// question is where the material sits at ONE height, and a bounding box has
/// forgotten that. `each_held_model_is_the_single_primitive_the_viewmodel_loads`
/// is why mesh 0 / primitive 0 is the whole model.
fn glb_tris(path: &std::path::Path) -> Tris {
    let raw = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let jlen = u32::from_le_bytes(raw[12..16].try_into().unwrap()) as usize;
    let g: serde_json::Value = serde_json::from_slice(&raw[20..20 + jlen]).expect("GLB JSON");
    // The BIN chunk follows the JSON chunk's own 8-byte header and body.
    let bin = &raw[20 + jlen + 8..];
    let prim = &g["meshes"][0]["primitives"][0];
    let at = |i: usize| -> (&serde_json::Value, usize, usize) {
        let a = &g["accessors"][i];
        let bv = &g["bufferViews"][a["bufferView"].as_u64().unwrap() as usize];
        let base = bv["byteOffset"].as_u64().unwrap_or(0) as usize
            + a["byteOffset"].as_u64().unwrap_or(0) as usize;
        (a, base, a["count"].as_u64().unwrap() as usize)
    };

    let (a, base, n) = at(prim["attributes"]["POSITION"].as_u64().expect("POSITION") as usize);
    assert_eq!(
        a["componentType"].as_u64(),
        Some(5126),
        "POSITION is not f32"
    );
    assert_eq!(a["type"].as_str(), Some("VEC3"), "POSITION is not VEC3");
    let bv = &g["bufferViews"][a["bufferView"].as_u64().unwrap() as usize];
    let stride = bv["byteStride"].as_u64().unwrap_or(12) as usize;
    assert_eq!(stride, 12, "{}: interleaved POSITION", path.display());
    let f = |o: usize| f32::from_le_bytes(bin[o..o + 4].try_into().unwrap());
    let pos = (0..n)
        .map(|i| {
            let o = base + i * stride;
            [f(o), f(o + 4), f(o + 8)]
        })
        .collect();

    let (a, base, n) = at(prim["indices"].as_u64().expect("indices") as usize);
    let idx = match a["componentType"].as_u64() {
        Some(5123) => (0..n)
            .map(|i| {
                u32::from(u16::from_le_bytes(
                    bin[base + i * 2..base + i * 2 + 2].try_into().unwrap(),
                ))
            })
            .collect(),
        Some(5125) => (0..n)
            .map(|i| u32::from_le_bytes(bin[base + i * 4..base + i * 4 + 4].try_into().unwrap()))
            .collect(),
        other => panic!("{}: index componentType {other:?}", path.display()),
    };
    (pos, idx)
}

/// The same, for a row `render::heldgen` builds rather than ships.
fn mesh_tris(mesh: &bevy::prelude::Mesh) -> Tris {
    let Some(VertexAttributeValues::Float32x3(pos)) =
        mesh.attribute(bevy::prelude::Mesh::ATTRIBUTE_POSITION)
    else {
        panic!("a generated held mesh has no Float32x3 POSITION attribute");
    };
    let idx = mesh
        .indices()
        .expect("a generated held mesh is not indexed");
    (pos.clone(), idx.iter().map(|i| i as u32).collect())
}

fn row_tris(m: &client::ui::hold::HeldModelDef) -> Tris {
    match m.src {
        HeldSrc::Glb(rel) => glb_tris(&asset_path(rel)),
        HeldSrc::Gen(name) => mesh_tris(&heldgen::mesh(name)),
    }
}

/// How far the model's surface reaches in x and z across the horizontal slab
/// `lo..hi`, as `(x_min, x_max, z_min, z_max)`.
///
/// **Vertices are not enough, and the torch is why.** `heldgen` builds its
/// shaft out of two rings, so a 90 mm slab across the middle of a torch holds
/// zero vertices and every square millimetre of the material — the first cut
/// of this gate failed the torch for having geometry too clean to sample.
/// What bounds the surface inside a slab is the triangles' EDGES clipped to
/// it: the intersection of a triangle with a slab is convex, the extreme
/// point of a linear function on a convex region is on its boundary, and that
/// boundary is made of edge segments. So this walks the index buffer, clips,
/// and is blind to how finely anything is tessellated.
fn slab_extent(pos: &[[f32; 3]], idx: &[u32], lo: f32, hi: f32) -> Option<(f32, f32, f32, f32)> {
    let (mut xlo, mut xhi, mut zlo, mut zhi) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
    let mut hit = false;
    let mut take = |p: [f32; 3]| {
        hit = true;
        xlo = xlo.min(p[0]);
        xhi = xhi.max(p[0]);
        zlo = zlo.min(p[2]);
        zhi = zhi.max(p[2]);
    };
    for t in idx.chunks_exact(3) {
        for k in 0..3 {
            let (a, b) = (pos[t[k] as usize], pos[t[(k + 1) % 3] as usize]);
            let (ya, yb) = (a[1], b[1]);
            if ya.max(yb) < lo || ya.min(yb) > hi {
                continue;
            }
            // The parameters where the edge is inside the slab. A horizontal
            // edge either lies in it entirely or was skipped above.
            let (mut t0, mut t1) = (0.0f32, 1.0f32);
            if (yb - ya).abs() > f32::EPSILON {
                let (u, v) = ((lo - ya) / (yb - ya), (hi - ya) / (yb - ya));
                t0 = t0.max(u.min(v));
                t1 = t1.min(u.max(v));
            }
            if t0 > t1 {
                continue;
            }
            for s in [t0, t1] {
                take([
                    a[0] + (b[0] - a[0]) * s,
                    a[1] + (b[1] - a[1]) * s,
                    a[2] + (b[2] - a[2]) * s,
                ]);
            }
        }
    }
    hit.then_some((xlo, xhi, zlo, zhi))
}

#[test]
fn the_fist_closes_on_the_model_and_not_on_air() {
    // **`every_grip_is_on_the_object` never checked that the grip is on the
    // object**, which is the reading its name invites and the hole this
    // fills. It asserts `grip_frac` is in 0..=1 — that the fist is between
    // the model's feet and its crown — and says nothing at all about x or z,
    // because `grip_m` is a scalar and the whole table is written as though
    // the thing a hand closes on lies along the model's own +Y.
    //
    // For a spear, a hammer and a pickaxe it does, so the gap was invisible
    // for as long as nobody looked at anything else. `ci/import_meshy.py`
    // centres the BOUNDING BOX on x/z, and a bounding box is not a haft: put
    // a heavy head on one side of a leaning handle and the box centres on the
    // head. Measured on the shipped `stone_hatchet.glb`, the haft at the
    // declared grip height sat **121 mm** off the +Y axis — three palm-widths
    // — so `viewmodel::pose` slid the model until a point in thin air was in
    // the palm and the axe hung beside the fist, pointing 32° across the
    // frame (operator, 2026-09-01, with the frame). `hunting_bow.glb` was the
    // same defect at 165 mm.
    //
    // So the assertion is the one nobody had written down: **at the height
    // the fist occupies, the model must reach across the axis the fist is
    // on.** It is a statement about the geometry, exact, and it needs no
    // per-row judgement — `ci/stand_grip.py` is the fix it points at.
    //
    // It does NOT gate the grip axis' DIRECTION, and that is deliberate
    // rather than forgotten: the hatchet's haft was also 32° off +Y, and a
    // tilt is only visible here when it also carries the material out of the
    // fist — which is what it did, because the two arrive together (a leaning
    // haft is *why* the bounding-box centre is not the haft). A gate on the
    // direction would need to know which part of each model is the shaft,
    // which is a judgement, and `stand_grip.py` is where that judgement is
    // made and recorded.
    for m in &HELD_MODELS {
        let (pos, idx) = row_tris(m);
        // `every_model_stands_on_its_own_feet` pins the foot to y = 0, so the
        // grip height is the fraction outright.
        let grip = m.height_m * m.grip_frac;
        // The fist's own height in the MODEL's units: a deployable drawn at
        // 0.16 has sixteen centimetres of itself inside a 45 mm grip.
        let band = FIST_HALF_M / m.scale;
        let Some((xlo, xhi, zlo, zhi)) = slab_extent(&pos, &idx, grip - band, grip + band) else {
            panic!(
                "{}: no surface at all within {band:.3} m of the grip height \
                 {grip:.3} — the fist closes on nothing",
                m.key
            );
        };
        // Millimetres of clear air between the fist's axis and the nearest
        // face of what is there, drawn — 0 when the model reaches across it.
        let clear = |lo: f32, hi: f32| {
            if lo <= 0.0 && hi >= 0.0 {
                0.0
            } else {
                lo.abs().min(hi.abs()) * m.scale * 1000.0
            }
        };
        let (cx, cz) = (clear(xlo, xhi), clear(zlo, zhi));
        assert!(
            cx == 0.0 && cz == 0.0,
            "{} hangs {:.0} mm off the fist. At the grip height ({:.3} of a \
             {:.3} m model) its surface spans x [{xlo:+.3}, {xhi:+.3}] and \
             z [{zlo:+.3}, {zhi:+.3}], and `viewmodel::pose` puts \
             (0, {grip:.3}, 0) in the palm — so the hand closes on air and the \
             model floats beside it. Stand the model on the axis its hand \
             grips: `ci/stand_grip.py assets/{} <out.glb> --shaft LO HI \
             --grip {}`, then restate `height_m` from what it prints.",
            m.key,
            cx.max(cz),
            m.grip_frac,
            m.height_m,
            match m.src {
                HeldSrc::Glb(rel) => rel,
                HeldSrc::Gen(name) => name,
            },
            m.grip_frac
        );
    }
}

/// The bow in your own hand draws a live string (`render::bow`), so the one
/// baked into the file has to come out, all of it and nothing else: found,
/// tied on at the two tips, and leaving both limbs whole.
#[test]
fn the_bows_baked_string_comes_out() {
    let row = client::render::bow::bow_row().expect("the hunting bow has a row");
    let def = &HELD_MODELS[row];
    // The plain bow, and every bow skin with a model of its own: each one's
    // first-person copy draws a live string (`bow::FpBow`).
    let skins = SKIN_MODELS
        .iter()
        .filter(|s| s.key == def.key)
        .map(|s| glb_tris(&asset_path(s.path)));
    for (pos, idx) in std::iter::once(row_tris(def)).chain(skins) {
        bow_string_comes_out(def, &pos, &idx);
    }
}

fn bow_string_comes_out(def: &client::ui::hold::HeldModelDef, pos: &[[f32; 3]], idx: &[u32]) {
    let (keep, lo, hi) = client::render::bow::strip_string(pos, idx)
        .expect("found no string in the bow: the first-person bow would draw two");
    let taken = (idx.len() - keep.len()) / 3;
    assert!(
        taken * 10 < idx.len() / 3,
        "{taken} of {} triangles taken for the string",
        idx.len() / 3
    );
    let h = def.height_m;
    assert!(
        lo.y < 0.1 * h && hi.y > 0.9 * h,
        "the string runs {lo} to {hi}, not tip to tip of a {h} m bow"
    );
    let (y0, y1) = keep.iter().fold((f32::MAX, f32::MIN), |(a, b), &i| {
        let y = pos[i as usize][1];
        (a.min(y), b.max(y))
    });
    assert!(
        y0 < 0.01 * h && y1 > 0.99 * h,
        "the bow without its string spans {y0}..{y1} of {h} m: a tip went with it"
    );
}

/// A skin with its own model is held in its row's pose, so it must be its
/// row's height with the grip on the model, and it must be the skin
/// `content/skins.toml` says covers that row's item.
#[test]
fn every_skin_model_fits_its_row() {
    let toml = std::fs::read_to_string("../../content/skins.toml").expect("content/skins.toml");
    let items = std::fs::read_to_string("../../content/items.toml").expect("content/items.toml");
    // `catalog = N` and `covers = "item.x"` per `[[skin]]`, by hand for
    // `every_model_answers_to_an_item_that_exists`'s reason.
    let field = |block: &str, key: &str| -> Option<String> {
        block
            .lines()
            .find_map(|l| l.trim().strip_prefix(key)?.trim().strip_prefix('='))
            .map(|v| v.trim().trim_matches('"').to_string())
    };
    let rows: Vec<(u16, String)> = toml
        .split("[[skin]]")
        .skip(1)
        .filter_map(|b| Some((field(b, "catalog")?.parse().ok()?, field(b, "covers")?)))
        .collect();
    let name_of = |id: &str| -> Option<String> {
        items
            .split("[[item]]")
            .find(|b| field(b, "id").as_deref() == Some(id))
            .and_then(|b| field(b, "name"))
            .map(|n| stem(&n))
    };
    for s in &SKIN_MODELS {
        let covers = rows
            .iter()
            .find(|(c, _)| *c == s.catalog)
            .map(|(_, covers)| covers.clone())
            .unwrap_or_else(|| panic!("{}: no skin has catalog {}", s.path, s.catalog));
        assert_eq!(
            name_of(&covers).as_deref(),
            Some(s.key),
            "{}: skin {} covers {covers}, which is not the {} row",
            s.path,
            s.catalog,
            s.key
        );
        let m = HELD_MODELS
            .iter()
            .find(|m| m.key == s.key)
            .unwrap_or_else(|| panic!("{}: no row {}", s.path, s.key));
        let (pos, idx) = glb_tris(&asset_path(s.path));
        let (lo, hi) = pos
            .iter()
            .fold((f32::MAX, f32::MIN), |(a, b), p| (a.min(p[1]), b.max(p[1])));
        let err = (hi - lo - m.height_m).abs() / m.height_m;
        assert!(
            err < 0.02 && lo.abs() < 0.01,
            "{} stands {lo:.3}..{hi:.3} and its row is {:.3} tall from 0",
            s.path,
            m.height_m
        );
        let grip = m.height_m * m.grip_frac;
        let band = FIST_HALF_M / m.scale;
        let (xlo, xhi, zlo, zhi) = slab_extent(&pos, &idx, grip - band, grip + band)
            .unwrap_or_else(|| panic!("{}: nothing at the grip height", s.path));
        assert!(
            xlo <= 0.0 && xhi >= 0.0 && zlo <= 0.0 && zhi >= 0.0,
            "{} is off the fist at the grip: x [{xlo:+.3}, {xhi:+.3}] z [{zlo:+.3}, {zhi:+.3}]. \
             `ci/stand_grip.py` it.",
            s.path
        );
    }
}

/// A painted bat draws the generated lathe (`heldgen::canvas_mesh`, whose
/// UVs are the paint's contract) while the plain bat draws its file, so the
/// canvas has to stand where the row's file does: its height, its foot, and
/// material at the grip.
#[test]
fn the_bats_paint_canvas_still_fits_its_row() {
    let m = HELD_MODELS
        .iter()
        .find(|m| m.key == "bat")
        .expect("a bat row");
    let canvas = heldgen::canvas_mesh("bat").expect("the bat is a canvas");
    let (pos, idx) = mesh_tris(&canvas);
    let (lo, hi) = pos
        .iter()
        .fold((f32::MAX, f32::MIN), |(a, b), p| (a.min(p[1]), b.max(p[1])));
    let err = (hi - lo - m.height_m).abs() / m.height_m;
    assert!(
        err < 0.02 && lo.abs() < 0.01,
        "the bat's canvas stands {lo:.3}..{hi:.3} and its row is {:.3} tall from 0",
        m.height_m
    );
    let grip = m.height_m * m.grip_frac;
    let band = FIST_HALF_M / m.scale;
    let (xlo, xhi, zlo, zhi) =
        slab_extent(&pos, &idx, grip - band, grip + band).expect("canvas at the grip");
    assert!(
        xlo <= 0.0 && xhi >= 0.0 && zlo <= 0.0 && zhi >= 0.0,
        "the bat's canvas is off the fist at the grip: x [{xlo:+.3}, {xhi:+.3}] z [{zlo:+.3}, {zhi:+.3}]"
    );
}
