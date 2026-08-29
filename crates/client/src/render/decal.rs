//! The mark an arrow leaves — the first decal in this tree.
//!
//! Ranged combat has landed marks on nothing since it shipped: the sim
//! stopped arrows on ground, trunks and walls, `EV_SHOT` grew a tracer so
//! the arrow was visible in flight, and then the streak simply stopped
//! existing at the surface it hit. `EV_IMPACT` (wire v45) is the fact that
//! was missing; this is what draws it.
//!
//! # Bevy has decals and we were not using them
//!
//! `bevy_pbr::decal` ships two kinds and this takes the **forward** one:
//! a 1×1 quad whose material alpha-blends toward the edges by comparing
//! its own depth against the surface behind it, so a mark laid on uneven
//! ground fades out where the ground pulls away instead of hanging in the
//! air. `ClusteredDecal` is the higher-quality kind and is **not** an
//! option here — it requires bindless textures, which is the same wall
//! `ground_splat.rs` had to route around, on a gate box whose renderer is
//! a CPU rasterizer.
//!
//! **Its one hard prerequisite was already paid for, by accident.**
//! `ForwardDecal` needs a `DepthPrepass` on the camera, and `rig.rs` has
//! carried `ScreenSpaceAmbientOcclusion` since the lighting slice — which
//! is `#[require(DepthPrepass, NormalPrepass)]` in 0.18. So the prepass is
//! up, and `Msaa::Off` (the other caveat in Bevy's own usage notes) is set
//! for SSAO's sake too. Neither is this module's to hold, which is exactly
//! why `tests/decal.rs` asserts the camera still carries the AO: this file
//! would go quietly wrong if that line ever left `rig.rs`, and quietly is
//! the problem.
//!
//! # It decides nothing, and could not
//!
//! `RENDER.md` §1: Bevy draws, it does not decide. A mark is the purest
//! case of that in the tree — nothing reads one, nothing collides with
//! one, and a mark that is late, dropped or in the wrong place costs a
//! scuff. What it is NOT is a guess: the sim says where the arrow stopped
//! and what it stopped on, so this file never re-runs the stop test. The
//! alternative — a client that re-derived what it hit from `collide` and
//! `terrain` — is the two-copies-of-one-rule failure `column_floor_y` was
//! written to end.
//!
//! What this file *does* derive is the mark's **facing**, and that is a
//! different kind of thing: a normal read off shared worldgen is a
//! position lookup, not a second opinion about a collision. See
//! [`facing`].
//!
//! # A fixed pool, never a per-frame spawn
//!
//! [`MARKS`] entities are spawned once at startup and hidden; an impact
//! claims a free one and the fade releases it. The trap list's rule is no
//! per-frame allocations on the client, and spawn-plus-despawn per arrow
//! is exactly that — with the added cost that a Bevy spawn is a structural
//! archetype move. `tracer.rs` and `highlight.rs` make the same call.
//!
//! **The overflow policy is drop-OLDEST**, which is the opposite of the
//! tracer's and deliberately so. A tracer that vanishes mid-flight reads
//! as a bug, so `tracer.rs` refuses the newest rather than stealing a live
//! one. A mark has no motion to interrupt: the oldest is the faintest and
//! the furthest from whatever the player is looking at now, so recycling
//! it is invisible while refusing the newest would drop the mark from the
//! shot they just took — the one impact they are actually watching for.

use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::pbr::decal::{ForwardDecal, ForwardDecalMaterial, ForwardDecalMaterialExt};
use bevy::pbr::ExtendedMaterial;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use super::feed::Feed;
use super::{Eye, WorldId};
use sim_core::build::{self, BUILD_CELL_M, LEVEL_H_M};
use sim_core::collide::{ColIndex, PLANE_THICKNESS_M};
use sim_core::limits::{MAX_BUILD_COORD, MAX_BUILD_LEVELS};
use sim_core::movement::{POS_XZ_Q, POS_Y_Q};
use sim_core::ranged::{SURF_BUILT, SURF_GROUND, SURF_WORLD};
use sim_core::terrain;

/// Marks drawable at once.
///
/// A *view* cap, not a world one — the same split `tracer.rs` makes
/// between its 16 and the sim's `MAX_ARROWS`. Nothing about a mark exists
/// outside this pool: they are not saved, not replicated and not known to
/// the sim, so this number is answering "how much scuffing can be in front
/// of one player" and nothing else.
///
/// 48 is three volleys from four archers with the oldest still up. Each
/// slot costs one hidden entity, one 1×1 quad and one material; the mesh
/// is shared, so the per-slot cost is the material and the transform.
pub const MARKS: usize = 48;

/// How long a mark stays at full strength, seconds.
///
/// Long enough that a firefight leaves a readable record of where the
/// arrows went, short enough that the pool turns over rather than
/// saturating — at [`MARKS`] slots this is about one mark a second before
/// the oldest starts being recycled, which no realistic rate of fire
/// reaches. **Not a knob anybody spoke**: it is this module's documented
/// default, `DECISIONS.md` §open (surface marks v0).
pub const LIFE_S: f32 = 45.0;

/// The tail, seconds — how long the mark takes to fade out once [`LIFE_S`]
/// is up. A mark that vanished on a frame boundary would read as a pop in
/// the corner of the eye, which is the one way a purely cosmetic system
/// can still look like a bug.
pub const FADE_S: f32 = 6.0;

/// The mark's footprint, metres. An arrowhead is centimetres and this is
/// deliberately wider: what is being drawn is the scuff around the hit,
/// not the hole itself, and a mark under ~15 cm is invisible past the few
/// metres at which anybody would be looking at it.
pub const SIZE_M: f32 = 0.22;

/// How far the decal's box reaches along its projection axis before it
/// stops blending, metres (`ForwardDecalMaterialExt::depth_fade_factor`).
///
/// Small on purpose. This is the number that decides whether a mark on a
/// trunk also smears onto the ground two metres behind it, and `ART.md`
/// rule 2's whole complaint about a "clean intersection edge" is what a
/// decal looks like when this is set too generously — a sticker, not a
/// mark. One arrow-length is enough to cover the surface's own relief and
/// nothing beyond it.
pub const DEPTH_FADE_M: f32 = 0.35;

/// Frames the prewarm mark is left visible at startup.
///
/// **This exists because of a live trap with nothing else watching it.**
/// A new material is a new pipeline, and Bevy specializes a pipeline
/// lazily on **first use** — so without this the first arrow to land in a
/// fight would pay a compile stall at the worst possible moment.
/// `CLAUDE.md` carries this as a trap that survived the port and is worse
/// natively than it was in the browser, and notes that the gate which used
/// to assert it (`browser_smoke`) went with the browser and has no native
/// replacement.
///
/// A pipeline specializes when the material is actually **drawn**, so a
/// hidden entity does not pay it and neither does a culled one. Hence a
/// real draw, in frustum, at [`PREWARM_ALPHA`] — invisible to the player
/// and identical to the renderer.
pub const PREWARM_FRAMES: u32 = 8;

/// The prewarm mark's alpha. Non-zero because zero is a value a driver may
/// reasonably discard early, and the whole point is that the fragment path
/// runs; low enough that nobody sees it against ground.
pub const PREWARM_ALPHA: f32 = 0.02;

/// The mark texture's side, texels. A soft round scuff carries no detail
/// worth more than this, and the whole set is one 64×64 RGBA — 16 KB.
const MARK_TEX: u32 = 64;

/// Alpha steps the fade is quantized to.
///
/// **A write per live mark per frame is the thing being avoided.** Each
/// material edit marks the asset changed and re-uploads its uniform, so a
/// continuous fade across [`MARKS`] slots would be 48 uploads a frame for
/// a value no eye can resolve. Quantizing to 1/32 turns the whole fade
/// into 32 writes spread over [`FADE_S`], per mark.
const ALPHA_STEPS: f32 = 32.0;

/// One pooled mark and the fade it is currently running.
#[derive(Default, Clone, Copy)]
struct Mark {
    /// Seconds this mark has left, counting down through [`LIFE_S`] and
    /// then [`FADE_S`]. Zero means the slot is free — `tracer.rs`'s
    /// `life == 0` convention, so the two pools read alike.
    left: f32,
    /// The alpha step last written to this slot's material, so the fade
    /// writes only when the quantized value actually moves.
    step: i32,
    /// Ticks up while the prewarm mark is on screen; the slot is released
    /// when it reaches [`PREWARM_FRAMES`]. Only ever non-zero on the slot
    /// the prewarm claimed.
    warm: u32,
}

/// The pool. Entities and materials are created once and reused.
///
/// `Default` is written out rather than derived: `[T; N]` only derives it
/// up to N = 32 and [`MARKS`] is past that, so a derive stops compiling
/// the moment the pool is sized for the job it is actually doing.
#[derive(Resource)]
pub struct Marks {
    slots: [Mark; MARKS],
    entities: Vec<Entity>,
    materials: Vec<Handle<ForwardDecalMaterial<StandardMaterial>>>,
    /// Whether the prewarm draw has been asked for yet. One-shot: the
    /// pipeline needs specializing once per run, not once per world.
    warmed: bool,
    /// The oldest slot to recycle when every one is busy — a rotating
    /// hand rather than a scan for the least time left, because "oldest"
    /// only has to be approximately right to be invisible and this costs
    /// one add.
    hand: usize,
}

impl Default for Marks {
    fn default() -> Self {
        Self {
            slots: [Mark::default(); MARKS],
            entities: Vec::new(),
            materials: Vec::new(),
            warmed: false,
            hand: 0,
        }
    }
}

/// Drop every mark, because the world they were made in has gone.
///
/// **The pool is spawned at `Startup`, not on entering a world**, which is
/// what makes this necessary: the entities are not `WorldEntity`, so
/// `world_teardown`'s despawn walks straight past them and 48 marks at the
/// last shard's coordinates would still be standing in the next one. That
/// is `world_teardown`'s own stated failure — "view resources that would
/// otherwise carry the last world's eye into the next one" — and a mark is
/// a view resource that happens to be made of entities.
///
/// Hiding goes through `Commands` rather than a query because the teardown
/// runs outside the world schedule and has no reason to hold one.
///
/// **`warmed` is deliberately NOT reset.** A pipeline is specialized once
/// per process, not once per world; re-running the prewarm on every
/// reconnect would pay a real cost to avoid a stall that cannot happen
/// twice.
pub fn forget_in(commands: &mut Commands, pool: &mut Marks) {
    for &e in &pool.entities {
        commands.entity(e).insert(Visibility::Hidden);
    }
    pool.slots = [Mark::default(); MARKS];
    pool.hand = 0;
}

impl Marks {
    /// A free slot, or the oldest live one — the drop-oldest policy this
    /// module's header argues for, and the inverse of `tracer.rs`'s.
    fn claim(&mut self) -> usize {
        if let Some(ix) = self.slots.iter().position(|m| m.left <= 0.0) {
            return ix;
        }
        let ix = self.hand;
        self.hand = (self.hand + 1) % MARKS;
        ix
    }
}

/// Spawn the pool, hidden. One-time cost at startup, so the frame path
/// never spawns an entity for an arrow.
///
/// **`ForwardDecal` is `#[require(Mesh3d)]` and fills that mesh in from an
/// `on_add` hook, so nothing here hands it one.** Writing the obvious
/// `(ForwardDecal, Mesh3d(quad), …)` is precisely the shape of the
/// 2026-08-16 duplicate-component trap — a bundle assembled out of two
/// helpers that each insert the same component, which does not merge and
/// does not replace but panics at spawn inside a command queue, naming the
/// system as `<Enable the debug feature to see the name>`. Every gate in
/// this repo is blind to it. Let the hook own the mesh.
pub fn setup(
    mut commands: Commands,
    mut pool: ResMut<Marks>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<ForwardDecalMaterial<StandardMaterial>>>,
) {
    let tex = images.add(scuff_texture());
    for ix in 0..MARKS {
        let mat = materials.add(ExtendedMaterial {
            base: StandardMaterial {
                base_color: Color::srgba(1.0, 1.0, 1.0, 0.0),
                base_color_texture: Some(tex.clone()),
                // A scuff is dirt and splinters, not a polished surface.
                // Left rough and non-metallic so a mark never catches a
                // specular highlight the surface under it did not.
                perceptual_roughness: 0.95,
                metallic: 0.0,
                alpha_mode: AlphaMode::Blend,
                ..default()
            },
            extension: ForwardDecalMaterialExt {
                depth_fade_factor: DEPTH_FADE_M,
            },
        });
        let entity = commands
            .spawn((
                ForwardDecal,
                MeshMaterial3d(mat.clone()),
                Transform::from_scale(Vec3::splat(SIZE_M)),
                Visibility::Hidden,
            ))
            .id();
        pool.entities.push(entity);
        pool.materials.push(mat);
        pool.slots[ix] = Mark::default();
    }
}

/// The scuff: a soft round mask, white, with transparent edges.
///
/// White because the tint is the material's `base_color` — one texture
/// serves every surface kind and the *colour* is what says whether this
/// was dirt, bark or a splintered plank ([`tint`]).
///
/// The transparent edge is not incidental. Bevy's own usage note for
/// forward decals is that a steep viewing angle distorts them and that
/// padding the texture with transparent pixels is the mitigation; a
/// radial falloff that reaches zero before the quad's edge is that
/// padding, built in.
fn scuff_texture() -> Image {
    let n = MARK_TEX as usize;
    let mut data = vec![0u8; n * n * 4];
    let c = (n as f32 - 1.0) * 0.5;
    for y in 0..n {
        for x in 0..n {
            let (dx, dy) = (x as f32 - c, y as f32 - c);
            let d = (dx * dx + dy * dy).sqrt() / c;
            // Solid to 45% of the radius, then a smooth shoulder to zero
            // at 95% — the last 5% is the transparent padding above.
            let a = if d <= 0.45 {
                1.0
            } else {
                let t = ((0.95 - d) / 0.5).clamp(0.0, 1.0);
                t * t * (3.0 - 2.0 * t)
            };
            let i = (y * n + x) * 4;
            data[i] = 255;
            data[i + 1] = 255;
            data[i + 2] = 255;
            data[i + 3] = (a * 255.0) as u8;
        }
    }
    let mut img = Image::new(
        Extent3d {
            width: MARK_TEX,
            height: MARK_TEX,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        // sRGB: this is multiplied into base colour, and a white mask is
        // white in either encoding. `tree.rs`'s needle mask says the same.
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    img.sampler = ImageSampler::linear();
    img
}

/// What colour a mark on this surface is.
///
/// Three kinds, three colours, and the surface kind is the sim's word
/// rather than a guess (`ranged::SURF_*`). Dark rather than black —
/// `ART.md` rule 3 forbids a crushed shadow, and a mark is a surface like
/// any other: it is lit by the same sky the thing under it is.
fn tint(surf: u8) -> Color {
    match surf {
        // Turned earth: darker and warmer than the litter around it.
        SURF_GROUND => Color::srgb(0.19, 0.14, 0.10),
        // Exposed heartwood under bark — lighter than the trunk, which is
        // what makes a hit on a tree read at all.
        SURF_WORLD => Color::srgb(0.42, 0.31, 0.19),
        // Splintered timber: the same wood, greyer for the cut face.
        SURF_BUILT => Color::srgb(0.36, 0.30, 0.24),
        // Unreachable — the decoder refuses a fourth kind — but a mark is
        // the wrong place to panic, so an unknown surface gets the ground's
        // colour rather than taking the client down.
        _ => Color::srgb(0.19, 0.14, 0.10),
    }
}

/// Which way the mark faces, given where it landed and what it landed on.
///
/// **This is derived, and the distinction from re-deciding matters.** The
/// sim already said *what* was hit; all that is computed here is the
/// direction that surface points, out of worldgen the client and the
/// server both hold. A wrong normal costs a mark that lies at an angle. A
/// wrong *surface* would be the client disagreeing with the sim, which is
/// why the surface is on the wire and this is not.
///
/// - **Ground** is exact: the terrain gradient by central differences on
///   `terrain::ground`, the same function the mesh is built from. **Carved,
///   and that sentence is the reason** — the mesh moved to `ground` when the
///   site carve was armed, so a raw read here would lay the mark on the hill
///   the pad replaced while the pad is what a player sees and shoots at.
/// - **A world occupant** is exact too, and cheaper than it looks: a
///   trunk's axis is its slot's own `x`/`z`, so the outward normal is the
///   horizontal from that axis to the impact. `terrain::scatter` is the
///   lookup `props.rs` already spawns the tree from.
/// - **A built piece** is the approximate one, and it is still the honest
///   gap in this slice — but it is no longer wrong about floors, which is
///   the half that started mattering when `collide::shot_blocked` learned
///   to read a plane (shot planes v0). A wall lies on its cell's edge, so
///   the impact's offset from the cell centre points at that edge and the
///   dominant horizontal axis is right for it; a **floor is horizontal**,
///   and the same snap laid every mark on a floor edge-on, where a decal
///   projects to a smear or to nothing. So the altitude is asked first
///   ([`plane_face`]) and only an impact that is not on a slab's face falls
///   through to the edge snap.
///
///   Closing it *properly* still wants the piece's address on `EV_IMPACT`
///   (`NOW.md` §0mk): what the altitude cannot separate is a slab's face
///   from its **rim**, and [`plane_face`] declines the ambiguous band rather
///   than guessing — so a rim hit keeps exactly the normal it has always
///   had. The change is monotone: every impact this improves was wrong
///   before, and none that were right have moved.
fn facing(world: &WorldId, cols: &ColIndex, x: f32, y: f32, z: f32, surf: u8) -> Vec3 {
    match surf {
        SURF_GROUND => {
            // One arrow-radius of separation: closer and the difference is
            // float noise, wider and the normal is of a hill rather than
            // of the ground under the mark.
            const H: f32 = 0.25;
            let dx = terrain::ground(world.seed, &world.haven, x + H, z)
                - terrain::ground(world.seed, &world.haven, x - H, z);
            let dz = terrain::ground(world.seed, &world.haven, x, z + H)
                - terrain::ground(world.seed, &world.haven, x, z - H);
            Vec3::new(-dx, 2.0 * H, -dz).normalize_or(Vec3::Y)
        }
        SURF_WORLD => {
            let (cx, cz) = (
                (x / terrain::CELL_SIZE).floor() as i32,
                (z / terrain::CELL_SIZE).floor() as i32,
            );
            let slot = terrain::scatter(world.seed, &world.table, &world.haven, cx, cz);
            // A horizontal normal: what was hit is the side of a standing
            // thing, and its axis is vertical. `normalize_or` covers the
            // degenerate case of an impact on the axis itself, which is
            // reachable when the arrow came in from straight above.
            Vec3::new(x - slot.x, 0.0, z - slot.z).normalize_or(Vec3::Y)
        }
        _ => {
            // A slab's face, when the altitude says so and says it
            // unambiguously; otherwise the edge snap below, which is what a
            // wall, a rim and a foundation skirt all want.
            if let Some(n) = plane_face(world, cols, x, y, z) {
                return n;
            }
            let cell = BUILD_CELL_M;
            let ox = x - (x / cell).floor() * cell - cell * 0.5;
            let oz = z - (z / cell).floor() * cell - cell * 0.5;
            if ox.abs() >= oz.abs() {
                Vec3::new(ox.signum(), 0.0, 0.0)
            } else {
                Vec3::new(0.0, 0.0, oz.signum())
            }
        }
    }
}

/// Whether the impact at (`x`, `y`, `z`) landed on the **face** of a built
/// plane, and which way that face points — `None` when the altitude cannot
/// say so without guessing.
///
/// **Derived, like every other arm of [`facing`], and from the mirror the
/// predictor already holds.** A plane's geometry is not a secret the wire has
/// to carry: its top is `column_floor_y(cell, plate) + level·LEVEL_H_M` —
/// `collide`'s own sentence, computed here from the same `ColIndex` the
/// client predicts movement against — and the slab hangs
/// `PLANE_THICKNESS_M` below it. So an impact's height above that column is
/// enough to separate a floor from a wall, which is the distinction the mark
/// actually needs.
///
/// **It declines the rim, and that is the point of the margin.** An arrow
/// stopped by a slab's exposed edge reports a position in the same altitude
/// band as one stopped by its face, and nothing on the wire separates them —
/// so `EDGE_M` fences off the strip where a rim hit is reachable
/// (`ranged::ARROW_R_M` plus one `limits::ARROW_STEP_MM` tap, rounded up) and
/// this returns `None` inside it. `facing`'s edge snap then answers, exactly
/// as it did before shot planes v0. The result is that this function only
/// ever *replaces a wrong normal with a right one*; it never overrules a
/// normal that was already correct.
///
/// A **foundation** (level 0) has a top face and no underside — it is solid
/// to the ground and the drawn skirt is its side — so it claims `+Y` at its
/// top and declines everywhere below, leaving the skirt to the edge snap that
/// suits a vertical face.
fn plane_face(world: &WorldId, cols: &ColIndex, x: f32, y: f32, z: f32) -> Option<Vec3> {
    /// How close to a cell boundary an impact may be and still be called a
    /// face. Inside this, a rim hit is reachable and nothing distinguishes
    /// the two, so the answer is "do not guess".
    const EDGE_M: f32 = 0.25;
    /// The slack around a slab's two faces: an arrow stops at the first
    /// sample inside the band, so its reported height overshoots by up to one
    /// tap. Wider than the sample step so a mark is not thrown back to the
    /// edge snap by quantization.
    ///
    /// Named at this length because the knob registry scans **crate-wide by
    /// name** and a bare `TOL` already means 0.01 in `sim-core/tests/
    /// relief.rs` — the registry cannot be authoritative about a name that
    /// means two things, and it says so out loud rather than picking one.
    const SLAB_FACE_TOL_M: f32 = 0.25;

    let (bx, bz) = (build::build_cell_of(x), build::build_cell_of(z));
    // Both ends of the lattice, `collide`'s own guard: a mark arrives as three
    // quantized wire fields and nothing upstream promises they name a cell.
    if bx < 0 || bz < 0 || bx >= MAX_BUILD_COORD as i32 || bz >= MAX_BUILD_COORD as i32 {
        return None;
    }
    let (bx, bz) = (bx as u16, bz as u16);
    let m = cols.get(bx, bz);
    let tris = m.tri_xlo_zlo | m.tri_xhi_zlo | m.tri_xlo_zhi | m.tri_xhi_zhi;
    if m.planes == 0 && tris == 0 {
        return None;
    }
    // Far enough inside the cell that a rim cannot be what was hit. The
    // triangles' own half tests are deliberately not repeated here: a
    // triangle's hypotenuse is a rim too, and `EDGE_M` off the cell says
    // nothing about it, so a triangle only answers where the square would.
    let cxm = bx as f32 * BUILD_CELL_M + BUILD_CELL_M * 0.5;
    let czm = bz as f32 * BUILD_CELL_M + BUILD_CELL_M * 0.5;
    if (x - cxm).abs() > BUILD_CELL_M * 0.5 - EDGE_M
        || (z - czm).abs() > BUILD_CELL_M * 0.5 - EDGE_M
    {
        return None;
    }
    let base = build::column_floor_y(
        world.seed,
        &world.haven,
        bx,
        bz,
        cols.plate(bx, bz).unwrap_or(0),
    );
    for level in 0..MAX_BUILD_LEVELS {
        let bit = 1u8 << level;
        if (m.planes | tris) & bit == 0 {
            continue;
        }
        let top = base + level as f32 * LEVEL_H_M;
        if y > top + SLAB_FACE_TOL_M {
            continue; // over this slab
        }
        // Level 0 is solid to the ground, so it has a top face and no
        // underside — the drawn skirt is its side, and a hit on that side is
        // at the cell boundary, where `EDGE_M` has already declined.
        if level == 0 {
            if y >= top - SLAB_FACE_TOL_M {
                return Some(Vec3::Y);
            }
            continue;
        }
        let bottom = top - PLANE_THICKNESS_M;
        if y < bottom - SLAB_FACE_TOL_M {
            continue; // under this slab
        }
        // Inside the band: the NEARER face, rather than whichever test was
        // written first. The two windows overlap — the slack is most of the
        // slab's own thickness — so an order-dependent answer would put a
        // ceiling mark face-up on nothing more than statement order.
        return Some(if top - y <= y - bottom {
            Vec3::Y
        } else {
            Vec3::NEG_Y
        });
    }
    None
}

/// Claim a slot for every impact the feed reports.
///
/// Reads `Res<Feed>` — never `pop_impact` — because the drain is
/// single-consumer and a second caller would silently starve the first.
/// That is CLAUDE.md's clean-merge trap, and a pool that draws things is
/// exactly the kind of second reader that caused it.
pub fn mark(
    mut pool: ResMut<Marks>,
    feed: Res<Feed>,
    world: Res<WorldId>,
    // The piece mirror, for the one thing a mark on a built surface needs
    // that the wire does not carry: which way that surface points. Read the
    // way `audio::place` reads it — the placement broadcast that raised a
    // column inserted the record, so the column is in the mirror by the time
    // an impact inside it can arrive.
    net: NonSend<super::Net>,
    eye: Res<Eye>,
    mut materials: ResMut<Assets<ForwardDecalMaterial<StandardMaterial>>>,
    mut q: Query<(&mut Transform, &mut Visibility)>,
) {
    // The prewarm draw, once per run and before any arrow needs it. Placed
    // on the ground a few metres ahead of the eye rather than at the feet:
    // the point is to be *drawn*, and ground directly under the camera can
    // fall outside the frustum's near plane.
    if !pool.warmed && eye.placed {
        pool.warmed = true;
        // The eye's own forward, flattened — `rig.rs` builds the camera's
        // look vector from these two the same way. Flat because the target
        // is the ground ahead, and a player looking at the sky would
        // otherwise put the prewarm mark behind the horizon.
        let ahead = eye.pos + Vec3::new(eye.yaw.sin(), 0.0, eye.yaw.cos()) * 3.0;
        let y = terrain::ground(world.seed, &world.haven, ahead.x, ahead.z);
        let cols = net.session.core.pieces.cols();
        let ix = pool.claim();
        pool.slots[ix] = Mark {
            left: LIFE_S,
            step: -1,
            warm: 1,
        };
        place(
            &mut q,
            &mut materials,
            &pool,
            ix,
            Vec3::new(ahead.x, y, ahead.z),
            facing(&world, cols, ahead.x, y, ahead.z, SURF_GROUND),
            SURF_GROUND,
            PREWARM_ALPHA,
        );
    }

    for &(qx, qy, qz, surf) in feed.impacts() {
        let at = Vec3::new(
            qx as f32 * POS_XZ_Q,
            qy as f32 * POS_Y_Q,
            qz as f32 * POS_XZ_Q,
        );
        let n = facing(
            &world,
            net.session.core.pieces.cols(),
            at.x,
            at.y,
            at.z,
            surf,
        );
        let ix = pool.claim();
        pool.slots[ix] = Mark {
            left: LIFE_S + FADE_S,
            step: -1,
            warm: 0,
        };
        // Lifted off the surface along its own normal by a fraction of the
        // decal's reach, so the quad sits inside the projection volume
        // rather than exactly on the plane it is projecting onto.
        let pos = at + n * (DEPTH_FADE_M * 0.5);
        place(&mut q, &mut materials, &pool, ix, pos, n, surf, 1.0);
    }
}

/// Point one slot's entity at a surface and set its colour.
#[allow(clippy::too_many_arguments)]
fn place(
    q: &mut Query<(&mut Transform, &mut Visibility)>,
    materials: &mut Assets<ForwardDecalMaterial<StandardMaterial>>,
    pool: &Marks,
    ix: usize,
    pos: Vec3,
    normal: Vec3,
    surf: u8,
    alpha: f32,
) {
    let Some(&entity) = pool.entities.get(ix) else {
        return;
    };
    let Ok((mut tf, mut vis)) = q.get_mut(entity) else {
        return;
    };
    tf.translation = pos;
    // The decal projects along its own local **−Y**: `ForwardDecalPlugin`
    // builds its quad by rotating a `Rectangle` from +Z onto +Y, so local
    // +Y is the projection axis and aligning it with the surface normal is
    // what lays the mark flat on that surface.
    tf.rotation = Quat::from_rotation_arc(Vec3::Y, normal);
    tf.scale = Vec3::splat(SIZE_M);
    *vis = Visibility::Visible;
    if let Some(m) = materials.get_mut(&pool.materials[ix]) {
        m.base.base_color = tint(surf).with_alpha(alpha);
    }
}

/// Age every live mark, fade its tail, and release the slot.
pub fn fade(
    mut pool: ResMut<Marks>,
    time: Res<Time>,
    mut materials: ResMut<Assets<ForwardDecalMaterial<StandardMaterial>>>,
    mut q: Query<(
        &mut Visibility,
        &MeshMaterial3d<ForwardDecalMaterial<StandardMaterial>>,
    )>,
) {
    let dt = time.delta_secs();
    // Destructure once for disjoint field borrows rather than cloning the
    // entity list to dodge them. The clone was a heap allocation on every
    // frame of the game, which is the one thing the client's hot-path
    // discipline names outright (`CLAUDE.md` traps) — and `structures::stream`
    // and `water::stream` already use exactly this reborrow for exactly this
    // reason, so it is the house pattern rather than a new one.
    let Marks {
        slots, entities, ..
    } = &mut *pool;
    for (ix, entity) in entities.iter().enumerate() {
        let m = slots[ix];
        if m.left <= 0.0 {
            continue;
        }
        let Ok((mut vis, mat)) = q.get_mut(*entity) else {
            continue;
        };

        // The prewarm slot counts frames, not seconds: what it is waiting
        // for is a number of DRAWS, and a draw is a frame regardless of
        // how long the frame took. Released the moment the pipeline can
        // have specialized.
        if m.warm > 0 {
            slots[ix].warm += 1;
            if slots[ix].warm > PREWARM_FRAMES {
                slots[ix] = Mark::default();
                *vis = Visibility::Hidden;
            }
            continue;
        }

        let left = m.left - dt;
        slots[ix].left = left;
        if left <= 0.0 {
            slots[ix] = Mark::default();
            *vis = Visibility::Hidden;
            continue;
        }

        // Full strength until the tail, then a linear ramp to nothing.
        let alpha = (left / FADE_S).clamp(0.0, 1.0);
        let step = (alpha * ALPHA_STEPS) as i32;
        if step == m.step {
            continue;
        }
        slots[ix].step = step;
        if let Some(asset) = materials.get_mut(&mat.0) {
            asset.base.base_color = asset.base.base_color.with_alpha(step as f32 / ALPHA_STEPS);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The seed the shard ships and every capture is shot on.
    const SEED: u64 = 20260731;

    /// Wall 4 wants a cap **and a stated overflow policy**, and this is
    /// the policy: a full pool recycles its oldest rather than refusing
    /// the newest.
    ///
    /// The direction is the whole point and it is the opposite of
    /// `tracer.rs`'s. Refusing here would drop the mark from the shot the
    /// player just took — the one impact they are watching for — while
    /// recycling costs the faintest mark on screen. So the assertion is
    /// not merely "it is bounded": it is that a claim against a saturated
    /// pool still **hands back a usable slot**, and always one in range.
    #[test]
    fn the_mark_pool_is_bounded_and_recycles_rather_than_refusing() {
        let mut pool = Marks::default();
        assert_eq!(pool.slots.len(), MARKS, "the pool is a fixed array");

        // Every slot busy — the saturated case.
        for m in pool.slots.iter_mut() {
            m.left = LIFE_S;
        }
        // Twice round the pool: every claim is in range, and the hand
        // walks rather than parking on one victim, so a burst does not
        // reuse the same slot repeatedly and leave the rest stale.
        let mut seen = [0u32; MARKS];
        for _ in 0..MARKS * 2 {
            let ix = pool.claim();
            assert!(ix < MARKS, "claim handed back a slot outside the pool");
            seen[ix] += 1;
        }
        assert!(
            seen.iter().all(|&n| n == 2),
            "a saturated pool must spread its recycling over every slot; \
             one slot taking every claim is a pool of one wearing a pool's \
             clothes"
        );

        // A free slot is always preferred to recycling a live one.
        pool.slots[7].left = 0.0;
        assert_eq!(
            pool.claim(),
            7,
            "a free slot must be taken before a live one is stolen"
        );
    }

    /// A mark on flat ground lies flat, and one on a slope leans downhill.
    ///
    /// **This is the check that the derived normal is derived correctly**,
    /// and it is arithmetic rather than a screenshot — `CLAUDE.md` is
    /// explicit that there is no visual gate and that what may be gated
    /// about a frame is arithmetic.
    #[test]
    fn a_ground_mark_takes_the_terrain_it_lies_on() {
        let world = WorldId::new(SEED);

        // Sea level well off the island's shoulder is as close to flat as
        // this worldgen offers; the assertion is deliberately loose,
        // because what would fail it is an axis error, not a gentle slope.
        let empty = ColIndex::new();
        let n = facing(&world, &empty, 1024.0, 0.0, 1024.0, SURF_GROUND);
        assert!(
            n.y > 0.9,
            "a ground mark's normal must point broadly up, got {n:?} — a \
             normal built from the wrong axes lies down"
        );
        assert!(
            (n.length() - 1.0).abs() < 1e-3,
            "the normal must be unit length, got {}",
            n.length()
        );

        // And it must actually respond to the ground rather than being a
        // dressed-up constant: somewhere on the island the normal has to
        // tilt, or this function is `Vec3::Y` with extra steps.
        let mut tilted = 0;
        for i in 0..64 {
            let (x, z) = (300.0 + i as f32 * 20.0, 700.0 + i as f32 * 11.0);
            if facing(&world, &empty, x, 0.0, z, SURF_GROUND).y < 0.999 {
                tilted += 1;
            }
        }
        assert!(
            tilted > 0,
            "no sample in 64 tilted at all — `facing` is ignoring the \
             terrain and returning a constant"
        );
    }

    /// A mark on a standing thing faces out of it, horizontally.
    ///
    /// The trunk's axis is its slot's own position, so the outward normal
    /// is the horizontal from that axis to the impact — and it must have
    /// **no vertical component at all**, because what was hit is the side
    /// of something vertical. A normal that kept its Y would lay the mark
    /// at an angle across the bark.
    #[test]
    fn a_world_mark_faces_out_of_the_thing_it_hit() {
        let world = WorldId::new(SEED);
        let empty = ColIndex::new();
        let n = facing(&world, &empty, 512.3, 0.0, 733.7, SURF_WORLD);
        assert_eq!(n.y, 0.0, "a mark on a trunk's side has no vertical lean");
        assert!(
            (n.length() - 1.0).abs() < 1e-3,
            "the normal must be unit length, got {}",
            n.length()
        );
    }

    /// A mark on a floor lies flat on it, and one on a wall still stands up.
    ///
    /// **The check that shot planes v0 does not arrive drawing smears.**
    /// `collide::shot_blocked` learned to stop an arrow on a plane, so a
    /// built impact can now be a *floor* — and every built normal used to be
    /// horizontal, which lays a decal edge-on to the surface it is projecting
    /// onto. Both halves are asserted together on purpose: a `plane_face`
    /// that answered `+Y` everywhere would fix the floor and flatten every
    /// mark on every wall, and the wall half is the only thing that says so.
    ///
    /// Arithmetic, not a screenshot — `CLAUDE.md` is explicit that there is
    /// no visual gate and that what may be gated about a frame is arithmetic.
    #[test]
    fn a_mark_on_a_floor_lies_flat_and_one_on_a_wall_stands_up() {
        use sim_core::build::{LOC_PLANE, SHAPE_FLOOR, SHAPE_FOUNDATION};

        let world = WorldId::new(SEED);
        // Boxed: `ColIndex` is a large fixed array, and building one in a
        // stack frame is CLAUDE.md's wasm shadow-stack trap.
        let mut cols = Box::new(ColIndex::new());
        let (cx, cz) = (341u16, 341u16);
        cols.add(cx, cz, 0, LOC_PLANE, SHAPE_FOUNDATION, 0);
        cols.add(cx, cz, 1, LOC_PLANE, SHAPE_FLOOR, 0);

        let base = build::column_floor_y(world.seed, &world.haven, cx, cz, 0);
        let (mx, mz) = (
            cx as f32 * BUILD_CELL_M + BUILD_CELL_M * 0.5,
            cz as f32 * BUILD_CELL_M + BUILD_CELL_M * 0.5,
        );

        // On the foundation's top face, mid-cell.
        let n = facing(&world, &cols, mx, base, mz, SURF_BUILT);
        assert_eq!(
            n,
            Vec3::Y,
            "a mark on a foundation at y={base:.2} faces {n:?} — a horizontal              normal lays the decal edge-on to the floor it is on"
        );

        // On the first storey's floor, and on its underside — the ceiling of
        // the room below, which is the half a downward shot never reaches.
        let up = facing(&world, &cols, mx, base + LEVEL_H_M, mz, SURF_BUILT);
        assert_eq!(up, Vec3::Y, "a mark on the level-1 floor faces {up:?}");
        let down = facing(
            &world,
            &cols,
            mx,
            base + LEVEL_H_M - PLANE_THICKNESS_M,
            mz,
            SURF_BUILT,
        );
        assert_eq!(
            down,
            Vec3::NEG_Y,
            "a mark on the ceiling of the storey below faces {down:?}"
        );

        // A WALL — the same cell, at an altitude no slab occupies. It must
        // keep the horizontal snap, or this helper has flattened every
        // vertical surface in the game.
        let mid = facing(
            &world,
            &cols,
            mx + 1.0,
            base + LEVEL_H_M * 0.5,
            mz,
            SURF_BUILT,
        );
        assert_eq!(
            mid.y, 0.0,
            "a mark between two floors is on something vertical and must              stand up, got {mid:?}"
        );

        // And the RIM: at a slab's altitude but at the cell's edge, where
        // nothing on the wire separates a face from an exposed edge. It must
        // DECLINE rather than guess, which is what keeps this change from
        // overruling a normal that was already right.
        let rim = facing(
            &world,
            &cols,
            cx as f32 * BUILD_CELL_M + 0.05,
            base + LEVEL_H_M,
            mz,
            SURF_BUILT,
        );
        assert_eq!(
            rim.y, 0.0,
            "an impact on the cell boundary is a rim hit as easily as a face              hit; it must keep the edge snap, got {rim:?}"
        );

        // An empty column answers nothing at any altitude — the mirror is
        // what makes this derived rather than invented.
        let none = facing(&world, &ColIndex::new(), mx, base, mz, SURF_BUILT);
        assert_eq!(
            none.y, 0.0,
            "with no piece in the mirror there is no plane to face, got {none:?}"
        );
    }

    /// Every surface kind the sim can send has a colour, and no two are
    /// the same.
    ///
    /// The equality half is what makes this worth writing: three kinds
    /// that all resolved to one colour would draw correctly, place
    /// correctly, and tell the player nothing — the surface field would be
    /// crossing the wire for no reason and nothing would say so.
    #[test]
    fn every_surface_kind_reads_differently() {
        let kinds = [SURF_GROUND, SURF_WORLD, SURF_BUILT];
        for (i, &a) in kinds.iter().enumerate() {
            for &b in &kinds[i + 1..] {
                assert_ne!(
                    tint(a).to_linear().to_f32_array(),
                    tint(b).to_linear().to_f32_array(),
                    "surface kinds {a} and {b} paint the same colour, so \
                     the wire field buys nothing"
                );
            }
        }
        // `ART.md` rule 3: no crushed shadow. A mark is a lit surface like
        // any other and none of these may be black.
        for &k in &kinds {
            let l = tint(k).to_linear();
            assert!(
                l.red + l.green + l.blue > 0.05,
                "surface {k}'s colour is effectively black, which rule 3 \
                 forbids outdoors"
            );
        }
    }

    /// The scuff mask reaches zero before the quad's edge.
    ///
    /// Bevy's own usage note for forward decals is that a steep viewing
    /// angle distorts them and that padding the texture with transparent
    /// pixels is the mitigation. That padding is a property of the
    /// generated image, so it is checkable: the corners and edges must be
    /// fully transparent and the centre fully opaque.
    #[test]
    fn the_scuff_mask_is_padded_transparent_at_its_edges() {
        let img = scuff_texture();
        let data = img.data.as_ref().expect("the mask is built in memory");
        let n = MARK_TEX as usize;
        let alpha = |x: usize, y: usize| data[(y * n + x) * 4 + 3];

        assert_eq!(alpha(n / 2, n / 2), 255, "the centre must be solid");
        for i in 0..n {
            assert_eq!(alpha(i, 0), 0, "the top edge must be transparent");
            assert_eq!(alpha(i, n - 1), 0, "the bottom edge must be transparent");
            assert_eq!(alpha(0, i), 0, "the left edge must be transparent");
            assert_eq!(alpha(n - 1, i), 0, "the right edge must be transparent");
        }
    }
}
