//! Gate: each ground identity is laid at its own photograph's real size.
//!
//! A photograph has an authored real-world size and the four ground sources do
//! not share one. Until 2026-08-28 the ground drew all four at the single 4 m
//! `UV_PER_M` reference, so `forrest_ground_01` (a 2 m scan) drew at twice life
//! size and `brown_mud_leaves_01` (1.3 m) at 3.1× — the same defect
//! `piece surface v1` fixed for the building tiers, still open on the ground.
//!
//! Four things are checkable here without a GPU, and each is a real failure
//! with a way of going unnoticed:
//!
//! 1. **The tiles are the sources' published sizes.** They are constants in
//!    Rust describing four `.jpg`s fetched from two libraries. Scraped back out
//!    of `assets/textures/MANIFEST.md` rather than hand-kept — `CLAUDE.md`'s
//!    rule about mirrors, and the manifest is where the piece tiers already
//!    record theirs.
//! 2. **The table sits in the band pieces are held to.** Scraped out of
//!    `tests/pieces.rs` so the two cannot drift apart, because the rule that
//!    sets the band (`ART.md` rule 1 / rule 7) is about a surface and has
//!    nothing to do with whether it is a piece.
//! 3. **The multiplier actually lands the tile.** `UV_PER_M × tile × mult == 1`
//!    is the whole contract between the mesh and the material, and getting it
//!    inverted is a plausible edit that compiles.
//! 4. **Every tap of an identity's maps uses that identity's UV** — including
//!    the wall tap's two GRADIENTS, which pick the mip. Scraped from the WGSL,
//!    because no test in this repo can execute it.
//!
//! **There is no pixel gate here and there must not be one** (`CLAUDE.md`).
//! What is gated about a frame is arithmetic. In particular, nothing below
//! asserts that the ground now *looks* better: the rule 7 cost of a 3.1×
//! more frequent litter repeat is real, unmeasured here, and waiting on
//! `NOW.md` §LOOK.
//!
//! Headless — no GPU, no window, no shard.

#![cfg(feature = "render")]

use client::render::ground_splat::tile_multipliers;
use client::render::terrain_mesh::{
    GRAIN_SHARE, GROUND_TILE_M, GROUND_TILE_MAX_M, GROUND_TILE_MIN_M,
    SAND_GRAIN_SHARE_AT_PUBLISHED, UV_PER_M,
};
use client::render::textures::{FAMILY_LAYERS, ROUGH_AO_LAYER0};

const SHADER: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../assets/shaders/ground_splat.wgsl"
);
const MANIFEST: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../assets/textures/MANIFEST.md"
);
const PIECES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/pieces.rs");

const ROLES: [&str; 4] = ["sand", "grass", "litter", "rock"];

/// Linear-luma field of a shipped ground albedo, as a square `f64` grid.
fn luma_field(role: &str) -> (Vec<f64>, usize) {
    let path = format!(
        "{}/../../assets/textures/{role}_albedo.jpg",
        env!("CARGO_MANIFEST_DIR")
    );
    let img = image::open(&path)
        .unwrap_or_else(|e| panic!("{path}: {e}"))
        .to_rgb8();
    let n = img.width() as usize;
    assert_eq!(
        n,
        img.height() as usize,
        "{role}: the grain statistic assumes a square map"
    );
    let mut f = Vec::with_capacity(n * n);
    for px in img.pixels() {
        let mut lin = [0.0f64; 3];
        for (ch, l) in lin.iter_mut().enumerate() {
            let u = f64::from(px.0[ch]) / 255.0;
            *l = if u <= 0.040_45 {
                u / 12.92
            } else {
                ((u + 0.055) / 1.055).powf(2.4)
            };
        }
        f.push(0.2126 * lin[0] + 0.7152 * lin[1] + 0.0722 * lin[2]);
    }
    (f, n)
}

/// Percent of `f`'s variance finer than 5 cm when the map is drawn over
/// `tile_m` metres.
///
/// Total variance less the variance of the field box-averaged over the block
/// of texels 5 cm spans at that tile — a share of contrast below the rule's
/// own bound, with no FFT and no window function to argue about.
fn grain_share(f: &[f64], n: usize, tile_m: f64) -> f64 {
    let b = ((0.05 / tile_m) * n as f64).round().max(1.0) as usize;
    let m = (n / b) * b;
    let mut sum = 0.0;
    let mut sum_sq = 0.0;
    let mut coarse = Vec::with_capacity((m / b) * (m / b));
    for by in 0..m / b {
        for bx in 0..m / b {
            let mut block = 0.0;
            for y in 0..b {
                for x in 0..b {
                    let v = f[(by * b + y) * n + bx * b + x];
                    block += v;
                    sum += v;
                    sum_sq += v * v;
                }
            }
            coarse.push(block / (b * b) as f64);
        }
    }
    let count = (m * m) as f64;
    let total = sum_sq / count - (sum / count).powi(2);
    let cm = coarse.iter().sum::<f64>() / coarse.len() as f64;
    let cv = coarse.iter().map(|c| (c - cm).powi(2)).sum::<f64>() / coarse.len() as f64;
    (total - cv) / total * 100.0
}

/// Leg 1. Every tile is its source's published physical size, or the ceiling.
///
/// **Scraped from `MANIFEST.md`, not hand-kept.** The authored size is a fact
/// about a file someone else published; the manifest is where this repo writes
/// those down, and a constant that mirrors it by hand is the drift `CLAUDE.md`
/// warns about twice. A row that stops stating its size fails loudly here
/// rather than skipping.
#[test]
fn every_tile_is_its_sources_published_size_or_the_ceiling() {
    let manifest = std::fs::read_to_string(MANIFEST).expect("MANIFEST.md");
    // `sand` publishes 15000 mm and is deliberately NOT drawn there; the
    // manifest row says so and `GROUND_TILE_M` clamps it. `rock` publishes
    // nothing at all. Both are checked below rather than here.
    let published: [Option<f64>; 4] = [Some(15.0), Some(2.0), Some(1.3), None];
    let mut seen = 0;
    for (k, role) in ROLES.iter().enumerate() {
        let row = manifest
            .lines()
            .find(|l| l.starts_with(&format!("| `{role}` |")))
            .unwrap_or_else(|| panic!("no `{role}` row in MANIFEST.md"));
        match published[k] {
            Some(mm) => {
                let want = format!("Authored at {} mm", (mm * 1000.0).round() as i64);
                assert!(
                    row.contains(&want),
                    "{role}: MANIFEST.md's row does not state \"{want}\". The \
                     authored size is what `GROUND_TILE_M[{k}]` is derived \
                     from; if the source changed, re-fetch its published \
                     `dimensions` and update both."
                );
                let want_tile = mm.min(f64::from(GROUND_TILE_MAX_M));
                assert!(
                    (f64::from(GROUND_TILE_M[k]) - want_tile).abs() < 1e-6,
                    "{role}: published at {mm} m, ceiling {GROUND_TILE_MAX_M} m, \
                     so the tile should be {want_tile} m — `GROUND_TILE_M[{k}]` \
                     says {}.",
                    GROUND_TILE_M[k]
                );
                seen += 1;
            }
            None => {
                assert!(
                    row.contains("No authored size published"),
                    "{role}: `GROUND_TILE_M[{k}]` is not derived from a \
                     published size, so MANIFEST.md's row must say so and give \
                     the counted cross-check that stands in for it — the shape \
                     `structures::TIER`'s metal row uses."
                );
                seen += 1;
            }
        }
    }
    assert_eq!(seen, 4, "the manifest scrape matched fewer rows than roles");
}

/// Leg 2. The ground's band is the band pieces are held to — scraped, so the
/// two cannot drift.
///
/// `ART.md` rule 1 (a near-field grain under 5 cm) sets the far end and rule 7
/// (the repeat becoming the pattern) the near end. Neither says anything about
/// whether the surface is a piece or the ground, so a second, independently
/// maintained band would be two numbers for one rule.
#[test]
fn the_ground_band_is_the_one_pieces_are_held_to() {
    let src = std::fs::read_to_string(PIECES).expect("pieces.rs");
    let band = src
        .split_once("(0.25..=4.0).contains(")
        .map(|_| (0.25f32, 4.0f32))
        .expect(
            "`tests/pieces.rs` no longer states its tiles/m band as \
             `(0.25..=4.0).contains(` — this gate reads that literal to keep \
             the ground's ceiling tied to it. Re-point it rather than \
             hard-coding a second copy.",
        );
    // pieces.rs states TILES per metre; this table states METRES per tile.
    assert!(
        (GROUND_TILE_MAX_M - 1.0 / band.0).abs() < 1e-6,
        "the ground's ceiling is {GROUND_TILE_MAX_M} m but pieces.rs allows \
         down to {} tiles/m, i.e. {} m",
        band.0,
        1.0 / band.0
    );
    assert!(
        (GROUND_TILE_MIN_M - 1.0 / band.1).abs() < 1e-6,
        "the ground's floor is {GROUND_TILE_MIN_M} m but pieces.rs allows up \
         to {} tiles/m, i.e. {} m",
        band.1,
        1.0 / band.1
    );
    for (k, role) in ROLES.iter().enumerate() {
        let t = GROUND_TILE_M[k];
        assert!(
            t.is_finite() && (GROUND_TILE_MIN_M..=GROUND_TILE_MAX_M).contains(&t),
            "{role}: GROUND_TILE_M[{k}] = {t} m is outside \
             {GROUND_TILE_MIN_M}..={GROUND_TILE_MAX_M} m"
        );
    }
}

/// Leg 3. The multiplier lands the tile it claims to.
///
/// The mesh writes one UV at [`UV_PER_M`] and the material re-spreads it; this
/// is the entire contract between them. Inverting the expression compiles, and
/// the symptom would be litter at 3.1× life size in the other direction.
#[test]
fn the_multiplier_puts_each_identity_at_its_own_tile() {
    let mult = tile_multipliers();
    for (k, role) in ROLES.iter().enumerate() {
        // One tile of this identity must span exactly GROUND_TILE_M[k] metres:
        // uv advances by UV_PER_M per metre, times the multiplier, and one
        // whole texture repeat is one unit of UV.
        let span = UV_PER_M * GROUND_TILE_M[k] * mult[k];
        assert!(
            (span - 1.0).abs() < 1e-5,
            "{role}: {} m of world advances the scaled UV by {span}, not 1.0 — \
             the identity does not repeat at the tile the table states.",
            GROUND_TILE_M[k]
        );
    }
    // Sand and rock sit at the reference, so their sampling must be untouched
    // by this whole mechanism — the one claim that says the change is
    // additive rather than a re-tune of what already shipped.
    assert!(
        (mult[0] - 1.0).abs() < 1e-6 && (mult[3] - 1.0).abs() < 1e-6,
        "sand and rock are drawn at the {} m reference, so their multipliers \
         must be exactly 1.0 — got {} and {}",
        1.0 / UV_PER_M,
        mult[0],
        mult[3]
    );
}

/// Leg 4. Every tap of an identity's maps uses that identity's UV.
///
/// **Including the wall taps' gradients.** They are what selects the mip, so
/// scaling a wall UV and leaving its gradients alone would have grass sampling
/// a level chosen for a density it is no longer drawn at — a blur, not an
/// error, and invisible to every gate that reads a value. It is the same class
/// as the browser shipping the old wall tap's gradient backwards at ~80×
/// (`DECISIONS.md`, materials v4).
///
/// **The identity is a LAYER now, not a texture name** (2026-09-12): every
/// planar tap is `textureSample(<family>_maps, ground_sampler, uvK, K)`, so the
/// thing to hold is that the layer literal and the UV agree — and, for the
/// data array (2026-10-07), that roughness/AO sits at `ROUGH_AO_LAYER0 + K`
/// and never at `K`, because a roughness/AO tap at the wrong offset reads a
/// NORMAL and the island goes dark where a surface is tilted, which does not
/// look like a wrong layer. Roughness and AO are R and G of ONE tap, so each
/// identity's rough/AO layer must be sampled exactly once — a second tap of
/// it is the cost packing removed. The wall taps
/// live in `wall_plane`, which takes the layer and the tile as arguments, so
/// there the thing to hold is that each call hands identity K its own tile,
/// and that every tap inside reads the tiled UV and both tiled gradients.
///
/// A tap the scrape cannot classify fails loudly rather than being skipped.
#[test]
fn every_tap_uses_its_identitys_uv() {
    let wgsl = std::fs::read_to_string(SHADER).expect("shader");
    let uv_of = ["uv0", "uv1", "uv2", "uv3"];
    let comp = [
        "splat.tile.x",
        "splat.tile.y",
        "splat.tile.z",
        "splat.tile.w",
    ];
    let (planar_src, wall_fn) = split_wall_tap(&wgsl);

    // `(family, identity)` → planar taps seen; every family × identity must
    // be sampled exactly once.
    let mut planar = [[0u32; 4]; FAMILIES.len()];
    let mut checked = 0;
    let mut road = [0u32; FAMILIES.len()];
    for tap in taps(&planar_src) {
        let (family, layer, args) = classify(&tap);
        // The slot within its family: roughness/AO counts from
        // `ROUGH_AO_LAYER0`, and slot 4 of every family is the road's
        // aggregate, which is no identity.
        let k = (layer - LAYER0[family]) as usize;
        let role = if k < 4 { ROLES[k] } else { "aggregate" };
        match args.as_slice() {
            // textureSample(maps, sampler, uv, layer)
            [_, sampler, uv, _] => {
                assert_eq!(*sampler, "ground_sampler", "{tap}");
                if *uv == "uv_road" {
                    assert_eq!(k, 4, "road aggregate must sample its own layer: {tap}");
                    road[family] += 1;
                } else {
                    assert!(k < 4, "only the road samples the aggregate: {tap}");
                    assert_eq!(
                        *uv, uv_of[k],
                        "{family}/{role}: layer {layer} is sampled at `{uv}`, not \
                     `{}`. Every tap of an identity's maps must share one UV or \
                     its relief stops being registered with the colour it came \
                     from.\n  {tap}",
                        uv_of[k]
                    );
                    planar[family][k] += 1;
                }
            }
            _ => panic!(
                "a tap outside `wall_tap` that is not a planar `textureSample`: \
                 {tap}"
            ),
        }
        checked += 1;
    }
    for (f, name) in FAMILIES.iter().enumerate() {
        for (k, role) in ROLES.iter().enumerate() {
            assert_eq!(
                planar[f][k], 1,
                "{name}/{role}: sampled {} times planar — every identity's \
                 every map is sampled exactly once from the top",
                planar[f][k]
            );
        }
    }
    assert_eq!(
        road,
        [1; FAMILIES.len()],
        "road aggregate needs every channel registered: {road:?}"
    );
    assert!(
        wgsl.contains("let uv_road = in.uv * splat.tile.w * splat.pavement.w;"),
        "road projection must have fixed world scale, independent of coverage"
    );
    // 4 identities × (albedo + normal + rough/AO) = 12 planar taps, plus the
    // road's 4. A drop below this is the scrape going blind.
    assert_eq!(checked, 15, "12 terrain + 3 road taps");

    // The wall: each identity hands `wall_tap` its own layer and tile, once.
    for (k, c) in comp.iter().enumerate() {
        let call = format!("wall_tap(wall, {k}, {c})");
        assert_eq!(
            planar_src.matches(&call).count(),
            1,
            "{}: no single `{call}` — its wall taps would be drawn at another \
             identity's density, or not at all",
            ROLES[k]
        );
    }
    assert_eq!(
        planar_src.matches("wall_tap(").count(),
        4,
        "`wall_tap` is called other than once per identity"
    );
    // …and `wall_plane` tiles the UV and both gradients together, and
    // `wall_tap` hands it each plane's own UV and gradients.
    for line in [
        "let uv = uv0 * tile;",
        "let dx = dx0 * tile;",
        "let dy = dy0 * tile;",
    ] {
        assert_eq!(
            wall_fn.matches(line).count(),
            1,
            "`wall_plane` does not tile exactly once with `{line}` — the UV \
             and both gradients pick the mip together"
        );
    }
    for plane in ["a", "b"] {
        let args = format!(
            "wall_plane(wall.uv_{plane}, wall.dx_{plane}, wall.dy_{plane}, wall.axis_{plane}, layer, tile,"
        );
        assert_eq!(
            wall_fn.matches(&args).count(),
            1,
            "`wall_tap` does not read plane {plane} with its own UV and \
             gradients (`{args}`)"
        );
    }
    let ra0 = wgsl
        .lines()
        .find_map(|l| l.trim().strip_prefix("const ROUGH_AO_LAYER0: i32 = "))
        .and_then(|v| v.trim_end_matches(';').parse::<u32>().ok())
        .expect("no `const ROUGH_AO_LAYER0: i32 = <literal>;` in the shader");
    assert_eq!(
        ra0, ROUGH_AO_LAYER0,
        "the shader's ROUGH_AO_LAYER0 is {ra0}, `textures::ROUGH_AO_LAYER0` is \
         {ROUGH_AO_LAYER0}: the wall's roughness/AO taps would read a normal map"
    );
    let mut wall = [0u32; FAMILIES.len()];
    for tap in taps(&wall_fn) {
        match args_of(&tap).as_slice() {
            // textureSampleGrad(maps, sampler, uv, layer, ddx, ddy)
            [maps, sampler, uv, layer, ddx, ddy] => {
                assert!(tap.starts_with("textureSampleGrad("), "{tap}");
                assert_eq!(*sampler, "ground_sampler", "{tap}");
                assert_eq!(
                    (*uv, *ddx, *ddy),
                    ("uv", "dx", "dy"),
                    "a wall tap that does not read its plane's tiled UV and \
                     gradients: {tap}"
                );
                let family = match (*maps, *layer) {
                    ("albedo_maps", "layer") => 0,
                    ("data_maps", "layer") => 1,
                    ("data_maps", "layer + ROUGH_AO_LAYER0") => 2,
                    _ => panic!("a wall tap this scrape cannot classify: {tap}"),
                };
                wall[family] += 1;
            }
            _ => panic!("a wall tap this scrape cannot classify: {tap}"),
        }
    }
    assert_eq!(
        wall,
        [1; FAMILIES.len()],
        "`wall_plane` must read every family exactly once: {wall:?}"
    );
}

/// The shader with the wall functions (`wall_plane`, `wall_tap`) cut out,
/// and those functions' source.
fn split_wall_tap(wgsl: &str) -> (String, String) {
    let mut rest = wgsl.to_string();
    let mut fns = String::new();
    for name in ["fn wall_plane(", "fn wall_tap("] {
        let at = rest
            .find(name)
            .unwrap_or_else(|| panic!("no `{name}` in the shader"));
        let len = rest[at..]
            .find("\n}")
            .unwrap_or_else(|| panic!("unterminated `{name}`"))
            + 2;
        fns.push_str(&rest[at..at + len]);
        fns.push('\n');
        rest.replace_range(at..at + len, "");
    }
    (rest, fns)
}

/// The families in the order [`classify`] numbers them.
const FAMILIES: [&str; 3] = ["albedo", "normal", "rough/AO"];
/// Each family's first layer in its array: albedo has its own, and normal and
/// the packed roughness/AO share `data_maps`.
const LAYER0: [u32; 3] = [0, 0, ROUGH_AO_LAYER0];

/// Every `textureSample…(` call in the fragment, as its text through the
/// matching close paren. Comments are dropped first so a tap in prose is not
/// a tap.
fn taps(wgsl: &str) -> Vec<String> {
    let code: String = wgsl
        .lines()
        .map(|l| l.split("//").next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n");
    let mut out = Vec::new();
    let mut rest = code.as_str();
    while let Some(i) = rest.find("textureSample") {
        let from = &rest[i..];
        let open = from.find('(').expect("textureSample without a paren");
        let mut depth = 0i32;
        let mut close = None;
        for (j, c) in from[open..].char_indices() {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        close = Some(open + j);
                        break;
                    }
                }
                _ => {}
            }
        }
        let close = close.expect("unbalanced textureSample call");
        out.push(from[..=close].to_string());
        rest = &from[close..];
    }
    out
}

/// A call's arguments, split at the call's own depth.
fn args_of(tap: &str) -> Vec<&str> {
    let open = tap.find('(').unwrap();
    let inner = &tap[open + 1..tap.len() - 1];
    let mut args = Vec::new();
    let mut depth = 0i32;
    let mut start = 0;
    for (i, c) in inner.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                args.push(inner[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
    }
    args.push(inner[start..].trim());
    args
}

/// One tap as `(family index, layer, args)`: the family from the array the
/// tap names, the layer from its literal, and the arguments split at the
/// call's own depth. Normal and roughness/AO share `data_maps` and are told
/// apart by the layer against `ROUGH_AO_LAYER0`.
fn classify(tap: &str) -> (usize, u32, Vec<&str>) {
    let args = args_of(tap);
    let layer: u32 = args
        .get(3)
        .unwrap_or_else(|| panic!("no layer argument in {tap}"))
        .parse()
        .unwrap_or_else(|e| panic!("the layer of `{tap}` is not a literal: {e}"));
    let family = match args[0] {
        "albedo_maps" => 0,
        "data_maps" if layer < ROUGH_AO_LAYER0 => 1,
        "data_maps" => 2,
        other => panic!("a tap of `{other}` this scrape cannot classify: {tap}"),
    };
    // Every family carries the four identities and the road's aggregate.
    assert!(
        layer < LAYER0[family] + FAMILY_LAYERS,
        "{tap}: layer {layer} is past the end of its family"
    );
    (family, layer, args)
}

/// Leg 5. The recorded grain shares are the shipped files' own.
///
/// `GRAIN_SHARE` is what says the tile in `GROUND_TILE_M` is a good place to
/// draw that photograph, and `MANIFEST.md` makes swapping a file the *designed*
/// way to change art here. Unpinned, the number that justified a tile survives
/// the source it was measured from.
///
/// ⚠ This asserts a MAGNITUDE per file, never that a wider tile scores lower —
/// that is monotone by construction for any image and would be a gate that
/// passes on every possible input.
#[test]
fn the_grain_shares_are_the_shipped_files_own() {
    for (k, role) in ROLES.iter().enumerate() {
        let (f, n) = luma_field(role);
        let got = grain_share(&f, n, f64::from(GROUND_TILE_M[k]));
        let want = f64::from(GRAIN_SHARE[k]);
        assert!(
            (got - want).abs() < 0.5,
            "{role}: GRAIN_SHARE[{k}] records {want:.3}% of its variance finer \
             than 5 cm at a {} m tile, and the shipped file measures \
             {got:.3}%. If the source was swapped deliberately, re-measure and \
             update `GRAIN_SHARE` — and check the new file still earns its \
             tile.",
            GROUND_TILE_M[k]
        );
    }
    // The counterfactual that carries the whole case for clamping sand.
    let (f, n) = luma_field("sand");
    let got = grain_share(&f, n, 15.0);
    let want = f64::from(SAND_GRAIN_SHARE_AT_PUBLISHED);
    assert!(
        (got - want).abs() < 0.5,
        "sand at its published 15 m measures {got:.3}% and \
         SAND_GRAIN_SHARE_AT_PUBLISHED records {want:.3}%. This number is the \
         reason `GROUND_TILE_M[0]` refuses the size Poly Haven publishes; if it \
         has moved, the refusal wants re-arguing rather than re-recording."
    );
}
