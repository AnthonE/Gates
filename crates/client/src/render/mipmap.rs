//! The mip chains for the loaded photographs — **the thing Bevy will not
//! build**, and the reason the island reads as television static at range.
//!
//! ## The defect
//!
//! `assets/textures/` ships 37 JPEGs and every one of them reaches the GPU as
//! a **single texture level**. `bevy_image` 0.18 has no mipmap generator for
//! the ordinary container formats: `ImageLoaderSettings` exposes `format`,
//! `texture_format`, `is_srgb`, `sampler`, `asset_usage` and `array_layout`,
//! and not one of them asks for a chain; `Image::new_uninit` writes
//! `mip_level_count: 1` and nothing in the load path ever raises it. Only
//! KTX2 and DDS carry a chain, because their *files* do (`ktx2.rs` reads
//! `level_count` off the header).
//!
//! A one-level 1024² albedo minified onto a hillside two kilometres away has
//! its per-pixel footprint spanning tens of texels, and a bilinear tap reads
//! **four of them**. The signal above the sampling rate does not average out;
//! it folds down, and it folds differently every frame the camera moves. That
//! is the static — not a compression artefact, not a noise field, not the
//! lighting: a texture being point-sampled out of a signal it cannot resolve.
//!
//! ## Why nothing here caught it, and why nothing here *could*
//!
//! `ground_splat.wgsl` is meticulous about mip selection — it scales the wall
//! tap's gradients by the same factor as its UV so "every identity whose tile
//! is not 4 m" picks the right level, and it reaches for `textureSampleGrad`
//! specifically because it is legal in non-uniform control flow. Every one of
//! those decisions is correct and every one of them was selecting among a
//! chain of length one. The shader asks for level 2.7 and the hardware clamps
//! it to 0, silently, forever.
//!
//! The renderer's own AA cannot help either: `rig.rs` runs `Msaa::Off` (SSAO
//! requires it) with SMAA over the top, and SMAA is an **edge** filter. It
//! finds a geometric silhouette and softens it. Minification aliasing has no
//! edge to find — the error is already baked into the shaded value of an
//! interior pixel — so no amount of post-process AA touches it.
//!
//! This crate already knew the mechanism and had solved it **twice**, both
//! times for images built in code rather than loaded from disk:
//! `water::ripple_map` ("an `Image` constructed in code has
//! `mip_level_count = 1`, and a one-level normal map on a surface that
//! reaches 2.6 km is a shimmering carpet of aliased highlights") and
//! `tree::needle_mips`. The photographs were the case nobody wrote it for,
//! and they are the ones covering the whole island.
//!
//! ## Derived, not listed
//!
//! Which images get a chain is decided from the image and its path, never
//! from a table of roles — the shape `prewarm.rs` argues for at length and
//! `CLAUDE.md` warns about twice (the `props.js` citation count, the `pop_*`
//! ring scrape, both of which drifted the moment somebody added one more).
//! Anything loaded out of `assets/textures/` that arrives as an uncompressed
//! RGBA8 power-of-two with one level gets one. A texture a future slice drops
//! in that directory is covered without that slice knowing this file exists,
//! and an image that brought its own chain (the ripple map, the needle mask)
//! is skipped by the level count it already carries.
//!
//! ## Three filters, because averaging is not one operation
//!
//! [`Filter`] is chosen from the path and the format, and the three cases are
//! genuinely different arithmetic:
//!
//! - **sRGB** (`Rgba8UnormSrgb`, every `_albedo`). The bytes are *encoded*,
//!   so a byte-wise average is an average of the wrong quantity. Black and
//!   white in equal measure is linear 0.5, which encodes to **188**, not 128
//!   — a plain average is 0.22 of the range too dark, and it compounds every
//!   level, so distant ground would drift dark and muddy as it recedes. That
//!   is a *different* visible defect from the static and it is what a naive
//!   fix ships. `tests/mipmap.rs` gates the checkerboard.
//!   Measured on the shipped files rather than argued: filtering
//!   `rock_albedo.jpg` down five levels sRGB-correctly holds its linear luma
//!   at **0.2450 → 0.2450**, and a byte-wise average of the same file lands
//!   at 0.2182 — **10.9% dark**, with `gravel` 12.3% and `grass` 2.6%. A mip
//!   chain has to conserve energy or the LOD boundary becomes a tonal seam
//!   across the island. (0.245 is also `textures::PropMaps`' own manifested
//!   figure for `rock`, which is the second source that measurement is
//!   checked against.)
//! - **Normal** (`_normal`). Average the vectors and renormalize — the house
//!   rule `water::ripple_map` states: "a normal map averaged and left
//!   unnormalized shortens toward the origin and reads as a loss of
//!   *lighting*, not of detail". Averaging also pulls the set toward flat,
//!   which is the right *behaviour*: distant relief should go smooth.
//! - **Linear** (`_rough`, `_ao`). Plain average. These are already linear
//!   data and neither is a vector.
//!
//! ## Residuals, named because they are not covered
//!
//! **Roughness is averaged, not re-derived.** Folding the normal map's
//! variance into roughness per level (Toksvig / LEAN) is what removes the
//! *specular* sparkle that survives a correct albedo chain. Averaging is the
//! standard approximation and it is what ships here; the sharper version
//! wants the normal and roughness chains built together, which is a later
//! slice and this module is where it lands.
//!
//! **glTF-embedded textures are not covered.** `assets/models/` reaches the
//! GPU through the gltf loader with its own samplers and no guarantee of
//! power-of-two, so it is a separate question with a separate answer
//! (`MANIFEST.md` already names KTX2-at-1024 as the rule for those).
//!
//! **This is a re-upload, not a load-time build.** The image is uploaded once
//! at one level, then modified and uploaded again with its chain. Bevy offers
//! no hook between decode and extraction, so the alternative is baking the
//! chains offline into KTX2 — which is the better end state and a much larger
//! change (37 files re-encoded, the depot's contents, `MANIFEST.md`'s
//! measured table, and `manifest_measured.rs` re-based on a new decoder). The
//! cost here is one extra upload of ~100 MB spread over the loading screen.

use bevy::asset::AssetId;
use bevy::image::Image;
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;

/// The directory whose textures get a chain, and the whole of the membership
/// test. A directory rather than a list of roles: see the module header.
pub const MIPPED_DIR: &str = "textures/";

/// How many images are given a chain per frame.
///
/// **Budgeted rather than done at once**, for the reason `CLAUDE.md` gives
/// about stream-in: the whole set can finish loading on one frame, and
/// filtering 100 MB of texels in a single system call is a stall measured in
/// hundreds of milliseconds. At two per frame the set drains in under twenty
/// frames of a loading screen that is already streaming the world in, which
/// is invisible; done eagerly it is a visible hitch on the bar.
///
/// Proposed default, not spoken.
pub const PER_FRAME: usize = 2;

/// How a level is reduced from the one above it. See the module header — the
/// three cases are different arithmetic and the sRGB one is the one a naive
/// fix gets wrong.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filter {
    /// Decode to linear, average, re-encode. Every `_albedo`.
    Srgb,
    /// Average the vectors, renormalize. Every `_normal`.
    Normal,
    /// Plain average. `_rough`, `_ao`, and anything else linear.
    Linear,
    /// sRGB on the colour, **coverage-preserved** on the alpha. Any cutout.
    ///
    /// See [`MASK_CUT`] — this is the one filter whose alpha is not an
    /// average, and the reason is that an alpha-tested surface does not draw
    /// its alpha, it draws *the share of texels that survive a threshold*.
    Mask,
}

impl Filter {
    /// Pick the filter from what the asset says about itself: the path names
    /// a normal map, and the format names an encoding.
    ///
    /// **The path is checked first and that order matters.** A normal map is
    /// loaded `is_srgb = false`, so it and a roughness map are the same
    /// `Rgba8Unorm` and the format alone cannot separate them. The `_normal`
    /// suffix is `textures::MapSet::load`'s own naming convention, already
    /// load-bearing there.
    ///
    /// `translucent` is measured off the pixels, not guessed from the
    /// extension: every map here is RGBA8 whether or not it uses the A, so
    /// the format cannot say, and a `.png` is not automatically a cutout.
    /// [`is_translucent`] is what answers it.
    pub fn pick(path: &str, format: TextureFormat, translucent: bool) -> Self {
        if path.contains("_normal.") {
            Self::Normal
        } else if translucent {
            Self::Mask
        } else if format == TextureFormat::Rgba8UnormSrgb {
            Self::Srgb
        } else {
            Self::Linear
        }
    }
}

/// The alpha a cutout is tested against, as a byte.
///
/// **This is `AlphaMode::Mask(0.5)` written as the number the filter can
/// measure**, and the two must agree or the chain preserves a coverage the
/// frame does not draw. 0.5 of the 0..1 range is 128 and not 188: alpha is
/// linear even in an sRGB-encoded texture — only RGB carries the transfer
/// function. `tree::NEEDLE_MASK_BYTE` is the same number for the same reason.
pub const MASK_CUT: u8 = 128;

/// The bisection's upper bound on the alpha rescale.
///
/// Past this the scale is pushing near-empty texels over the cutoff, which
/// INVENTS coverage rather than preserving it — and the bottom levels are a
/// handful of texels where exact coverage is unreachable at any scale.
/// `tree::needle_mips` uses the same ceiling for the same reason.
const MASK_SCALE_MAX: f32 = 8.0;

/// Whether any texel is less than fully opaque — i.e. whether the A channel
/// carries a cutout rather than the padding every RGBA8 image has.
///
/// **Not `< 255`.** A photographic cutout re-encoded through a lossy step can
/// carry 254 across a nominally solid interior, and one such texel would
/// promote a plain albedo to [`Filter::Mask`] and put it through a coverage
/// bisection it does not want. The margin is what makes this a property of
/// the image rather than of its encoder.
pub fn is_translucent(data: &[u8]) -> bool {
    data.chunks_exact(4).any(|t| t[3] < 250)
}

/// Scale a level's alpha until the share of texels over [`MASK_CUT`] matches
/// `want`, by bisection.
///
/// **Castaño's fix, and the reason a box filter alone is wrong here.**
/// Averaging a sparse mask drives every texel toward the mask's mean, and the
/// mean of a grass card is about 0.22 — well under the cutoff — so each level
/// loses coverage against the one above it and the loss compounds down the
/// chain. On the page that reads as grass THINNING with distance, which looks
/// like a density or LOD bug and is neither. `tree::needle_mips` measured its
/// own version of this at 0.53× of level 0's coverage after ONE halving.
///
/// Coverage is monotonic in the scale, so bisection is enough; 12 steps
/// resolve it to better than one part in 4,000 of the span, finer than the
/// 1/255 the channel can store.
///
/// ⚠ **It takes `hi`, not the midpoint, and that is a correctness choice
/// rather than a rounding one.** The bisection's invariant is that `hi`
/// reaches `want` and `lo` does not; their midpoint is on the `lo` side
/// exactly as often as not, and coverage is a STEP function of the scale
/// because alpha is 8-bit — so the midpoint can sit one step below the
/// threshold and preserve nothing at all. Measured on a hard-edged fixture:
/// the search converged to lo = 1.0068 (coverage 0.191) and hi = 1.0085
/// (0.401) against a target of 0.296, and the midpoint 1.0077 returned the
/// 0.191. Taking `hi` can only over-preserve, which thickens distant grass by
/// a texel; taking the midpoint can under-preserve, which is the baldness this
/// function exists to stop.
///
/// ⚠ **`tree::needle_mips` has the same line and takes the midpoint.** It has
/// not been changed here: its alpha is a soft stamp, so its coverage is nearly
/// continuous in the scale and the step is small, and its own gate pins the
/// numbers it currently produces. It is a latent instance of this bug, not a
/// live one — recorded so the next person to touch that file knows.
fn preserve_coverage(level: &mut [u8], want: f32) {
    let coverage = |scale: f32| -> f32 {
        let hit = level
            .chunks_exact(4)
            .filter(|t| (f32::from(t[3]) * scale).min(255.0) as u8 > MASK_CUT)
            .count();
        hit as f32 / (level.len() / 4) as f32
    };
    let (mut lo, mut hi) = (1.0f32, MASK_SCALE_MAX);
    for _ in 0..12 {
        let mid = 0.5 * (lo + hi);
        if coverage(mid) < want {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    // `hi`, never `0.5 * (lo + hi)` — see the warning above. `lo` is read by
    // the loop that narrows it and is deliberately not read after it.
    for t in level.chunks_exact_mut(4) {
        t[3] = (f32::from(t[3]) * hi).min(255.0) as u8;
    }
}

/// The share of texels a cutout actually draws.
pub fn coverage(data: &[u8]) -> f32 {
    let hit = data.chunks_exact(4).filter(|t| t[3] > MASK_CUT).count();
    hit as f32 / (data.len() / 4) as f32
}

/// sRGB → linear for one byte, as a 256-entry table.
///
/// A table because the alternative is a `powf` per channel per *input* texel,
/// and the input is the whole 100 MB set. Encoding is the other direction and
/// only runs on the levels below 0, which is a third as many texels.
fn decode_table() -> [f32; 256] {
    let mut t = [0.0f32; 256];
    for (i, v) in t.iter_mut().enumerate() {
        let c = i as f32 / 255.0;
        *v = if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        };
    }
    t
}

/// linear → sRGB, rounded to the nearest byte. The IEC 61966-2-1 transfer
/// function, and `round` rather than truncate because a truncating encode
/// loses half a code on every level and eleven levels of that is visible.
fn encode(c: f32) -> u8 {
    let c = c.clamp(0.0, 1.0);
    let s = if c <= 0.003_130_8 {
        12.92 * c
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    };
    (s * 255.0).round().clamp(0.0, 255.0) as u8
}

/// How many levels a `w × h` image's chain holds, level 0 included.
///
/// The chain stops when the **smaller** side reaches 1, which is what wgpu
/// requires of a complete chain and what `MipmapLevels` in the test file
/// re-derives independently.
pub fn levels(w: u32, h: u32) -> u32 {
    w.min(h).ilog2() + 1
}

/// How many bytes [`chain`] returns for a `w × h` RGBA8 image — the sum over
/// the whole chain.
///
/// Split out for the same reason `water::ripple_bytes` is: a malformed level
/// count fails inside wgpu at first draw, loudly and far from the cause, and
/// this is the one thing a headless gate can check exactly.
pub fn chain_bytes(w: u32, h: u32) -> usize {
    let mut total = 0usize;
    for lvl in 0..levels(w, h) {
        total += ((w >> lvl).max(1) as usize) * ((h >> lvl).max(1) as usize) * 4;
    }
    total
}

/// Build the whole mip chain for one decoded RGBA8 image, **level 0 first**.
///
/// `level0` is `w · h · 4` bytes, RGBA, and is copied into the result
/// unchanged — a chain must never disturb the level the game samples up
/// close. Every level below it is a 2×2 box reduction of the level above
/// under `filter`.
///
/// wgpu's `create_texture_with_data` reads the levels back out in the order
/// they are written here, which is why the order is stated rather than
/// implied.
///
/// # Panics
///
/// If `level0` is not exactly `w · h · 4` bytes. Callers screen for that —
/// the system below skips an image whose data does not match its descriptor
/// rather than panicking on a malformed asset.
pub fn chain(level0: &[u8], w: u32, h: u32, filter: Filter) -> Vec<u8> {
    assert_eq!(
        level0.len(),
        (w as usize) * (h as usize) * 4,
        "level 0 is not {w}x{h} RGBA8"
    );
    let table = decode_table();
    // The target every level below 0 is rescaled to hold. Taken from level 0
    // rather than from the level above, so the chain cannot drift a little at
    // each step and arrive somewhere else entirely at the bottom.
    let want = (filter == Filter::Mask).then(|| coverage(level0));
    let mut out = Vec::with_capacity(chain_bytes(w, h));
    out.extend_from_slice(level0);

    let mut prev = level0.to_vec();
    let mut pw = w;
    let mut ph = h;
    for _ in 1..levels(w, h) {
        let nw = (pw >> 1).max(1);
        let nh = (ph >> 1).max(1);
        // A side of 1 has nothing to pair with, so it samples itself. Square
        // power-of-two is what ships; this is what keeps a rectangular one
        // from reading past the end of the level above.
        let sx = if pw > 1 { 1 } else { 0 };
        let sy = if ph > 1 { 1 } else { 0 };
        let mut next = Vec::with_capacity((nw as usize) * (nh as usize) * 4);
        for y in 0..nh {
            for x in 0..nw {
                let quad = [
                    texel(&prev, pw, x * 2, y * 2),
                    texel(&prev, pw, x * 2 + sx, y * 2),
                    texel(&prev, pw, x * 2, y * 2 + sy),
                    texel(&prev, pw, x * 2 + sx, y * 2 + sy),
                ];
                next.extend_from_slice(&reduce(quad, filter, &table));
            }
        }
        if let Some(want) = want {
            preserve_coverage(&mut next, want);
        }
        out.extend_from_slice(&next);
        prev = next;
        pw = nw;
        ph = nh;
    }
    out
}

/// One RGBA texel out of a tightly packed level.
fn texel(level: &[u8], w: u32, x: u32, y: u32) -> [u8; 4] {
    let i = ((y as usize) * (w as usize) + (x as usize)) * 4;
    [level[i], level[i + 1], level[i + 2], level[i + 3]]
}

/// Reduce a 2×2 quad to one texel under `filter`.
///
/// **Alpha is a plain average in every case.** It is coverage, not colour and
/// not a vector — gamma-decoding it would be wrong for the same reason
/// gamma-decoding a roughness map is.
fn reduce(quad: [[u8; 4]; 4], filter: Filter, table: &[f32; 256]) -> [u8; 4] {
    let alpha = ((quad.iter().map(|t| t[3] as f32).sum::<f32>()) * 0.25).round() as u8;
    match filter {
        // **Mask shares Srgb's colour path and differs only after the level is
        // whole.** Its RGB is an albedo and wants the same linear-space
        // average; its alpha is averaged here and then rescaled across the
        // FULL level by `preserve_coverage`, which is a property of the level
        // and cannot be computed from one quad.
        Filter::Srgb | Filter::Mask => {
            let mut rgb = [0u8; 3];
            for (c, out) in rgb.iter_mut().enumerate() {
                let lin = quad.iter().map(|t| table[t[c] as usize]).sum::<f32>() * 0.25;
                *out = encode(lin);
            }
            [rgb[0], rgb[1], rgb[2], alpha]
        }
        Filter::Normal => {
            let mut v = [0.0f32; 3];
            for (c, out) in v.iter_mut().enumerate() {
                *out = quad
                    .iter()
                    .map(|t| t[c] as f32 / 255.0 * 2.0 - 1.0)
                    .sum::<f32>()
                    * 0.25;
            }
            // Guard the degenerate case the same way `water::ripple_map`
            // does: four normals that cancel would divide by zero, and a NaN
            // in a normal map is a black hole in the lighting.
            let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-6);
            let mut rgb = [0u8; 3];
            for (c, out) in rgb.iter_mut().enumerate() {
                *out = (((v[c] / l) * 0.5 + 0.5).clamp(0.0, 1.0) * 255.0).round() as u8;
            }
            [rgb[0], rgb[1], rgb[2], alpha]
        }
        Filter::Linear => {
            let mut rgb = [0u8; 3];
            for (c, out) in rgb.iter_mut().enumerate() {
                *out = ((quad.iter().map(|t| t[c] as f32).sum::<f32>()) * 0.25).round() as u8;
            }
            [rgb[0], rgb[1], rgb[2], alpha]
        }
    }
}

/// Whether an image is one this module builds a chain for, and why not when
/// it is not.
///
/// Every arm is a *skip* rather than a failure on purpose: this runs over
/// every image the asset server produces, most of which are none of its
/// business, and a loud refusal would be noise. What it must never do is
/// build a chain for something that already has one.
pub fn wants(image: &Image) -> bool {
    let d = &image.texture_descriptor;
    // Already has a chain — the ripple map and the needle mask build their
    // own, and a second pass over one would treat its levels as level 0.
    if d.mip_level_count != 1 {
        return false;
    }
    if d.dimension != bevy::render::render_resource::TextureDimension::D2 {
        return false;
    }
    if d.size.depth_or_array_layers != 1 {
        return false;
    }
    if !matches!(
        d.format,
        TextureFormat::Rgba8Unorm | TextureFormat::Rgba8UnormSrgb
    ) {
        return false;
    }
    // Power-of-two on both sides. A box filter on an odd side drops a row,
    // which is a subtly wrong chain rather than an obviously broken one, and
    // every file in `assets/textures/` is 1024² or 512².
    let (w, h) = (d.size.width, d.size.height);
    if w < 2 || h < 2 || !w.is_power_of_two() || !h.is_power_of_two() {
        return false;
    }
    image.data.as_ref().is_some_and(|data| {
        data.len() == (w as usize) * (h as usize) * 4
    })
}

/// Images waiting for a chain.
///
/// Ids only. The filter cannot be settled at enqueue time — [`Filter::pick`]
/// needs the decoded format to tell an albedo from a roughness map, and the
/// format is only in hand once the image is, so both halves of the decision
/// are made in [`drain`] where they are both available.
#[derive(Resource, Default)]
pub struct Pending(pub Vec<AssetId<Image>>);

/// Note every newly loaded `textures/` image for the drain below.
pub fn enqueue(
    mut events: MessageReader<AssetEvent<Image>>,
    mut pending: ResMut<Pending>,
    assets: Res<AssetServer>,
) {
    for event in events.read() {
        let AssetEvent::Added { id } = event else {
            continue;
        };
        // No path means the image was built in code and handed to
        // `Assets::add` — the ripple map and the needle mask, both of which
        // carry their own chain. Nothing to do either way.
        let Some(path) = assets.get_path(*id) else {
            continue;
        };
        let Some(path) = path.path().to_str().map(str::to_owned) else {
            continue;
        };
        if !path.starts_with(MIPPED_DIR) {
            continue;
        }
        pending.0.push(*id);
    }
}

/// Give up to [`PER_FRAME`] pending images their chain.
///
/// Popping from the back rather than draining from the front: the order is
/// arbitrary (they are all wanted before the player sees the world) and a
/// `remove(0)` would shift the tail every frame.
pub fn drain(mut pending: ResMut<Pending>, mut images: ResMut<Assets<Image>>, assets: Res<AssetServer>) {
    for _ in 0..PER_FRAME {
        let Some(id) = pending.0.pop() else {
            return;
        };
        let path = assets
            .get_path(id)
            .and_then(|p| p.path().to_str().map(str::to_owned))
            .unwrap_or_default();
        // `None` means nothing holds the image any more. Skip, exactly as
        // `prewarm::warm` skips a material dropped before it ran.
        let Some(image) = images.get_mut(id) else {
            continue;
        };
        if !wants(image) {
            continue;
        }
        let (w, h) = (image.texture_descriptor.size.width, image.texture_descriptor.size.height);
        let Some(level0) = image.data.as_ref() else {
            continue;
        };
        let filter = Filter::pick(&path, image.texture_descriptor.format, is_translucent(level0));
        let data = chain(level0, w, h, filter);
        // **The count and the buffer are set together or wgpu reads past the
        // end of one of them.** `Image::new` asserts `data.len()` against
        // level 0 alone, which is why `water::ripple_map` also assigns the
        // field directly rather than going through it.
        image.texture_descriptor.mip_level_count = levels(w, h);
        image.data = Some(data);
    }
}
