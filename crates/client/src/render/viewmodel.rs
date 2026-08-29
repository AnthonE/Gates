//! The held item, and the motion that makes it read as carried.
//!
//! `ART.md` §6: the reference frames are first-person with a **visible held
//! item** in most of them, and §8's last bullet makes "evidence a person is
//! playing" a pass condition. `hud.rs` satisfied the letter of that with two
//! `Cuboid`s parented to the camera — untextured, and welded rigidly to the
//! view, so it read as a decal on the lens rather than as something a body is
//! holding.
//!
//! **Three motions, and each one is a different input.** They are separated
//! because they answer different questions, and one blended "animate" curve
//! conflates them:
//!
//!   · **Bob** — am I walking? Driven by DISTANCE TRAVELLED, never by elapsed
//!     time. A time-driven bob keeps swinging while the player stands still,
//!     which is the tell that the motion is decorative rather than caused; a
//!     distance-driven one stops when the feet stop and stretches correctly
//!     when a hill slows the player, with no extra term.
//!   · **Sway** — am I turning? A first-order lag on the look rate, so the item
//!     trails the camera and catches up. This is the one that costs least and
//!     buys most: a rigidly parented object betrays itself the instant the
//!     mouse moves.
//!   · **Swing** — did I just do something? Driven by the client's mirror of
//!     the sim's own swing cadence ([`crate::ui::swing`]), with [`Feed`] as a
//!     backstop.
//!
//! ### The swing trigger moved, and why
//!
//! It was [`Feed`] alone — a landed hit or a gather payout — and the argument
//! was *"the swing worth drawing is the one that LANDED; reading the button
//! animates swings the server refused."* That is right about refusals and
//! wrong about **misses**, which are not refusals: a rock swung at open air
//! is a swing the sim fully took (it paid the cadence, whiffed the node scan,
//! found nothing in reach) and announced nothing, because there is nothing to
//! announce. So the commonest swing in the game drew no motion at all, and
//! the item sat still in frame while the sound played.
//!
//! What replaces it mirrors the sim's rule rather than guessing at it —
//! `BTN_PRIMARY` down and `tick >= next_swing` — over the tick estimate
//! [`Feed`] already carries. It still decides nothing (`RENDER.md` §1): it
//! picks a pose, on the same liberty `render::input`'s swing cue has taken
//! since audio v0.
//!
//! [`Feed`] stays as a **backstop, gated on the arm being at rest**: a hit
//! lands about a round trip after the local prediction started the stroke, so
//! an ungated retrigger would restart the arc mid-swing and read as a stutter.
//! At rest it can only mean the prediction did not fire — a clock that had
//! slipped, a frame the input system did not run — and then drawing something
//! beats drawing nothing.
//!
//! ## What this module may and may not read
//!
//! It reads [`Feed`] and never `ClientCore`'s `pop_*` directly. Those rings are
//! destructive and hand each fact over exactly once; a second drain site is the
//! silent-merge defect `CLAUDE.md`'s trap list records and `feed.rs`'s header
//! narrates at length. `Res<Feed>` cannot consume anything, which is exactly
//! why adding this third reader is free.
//!
//! It also decides nothing (`RENDER.md` §1). Speed comes from the eye's own
//! position delta rather than from a velocity the wire does not carry, and the
//! swing is a reaction to a fact the server already confirmed.

use bevy::camera::visibility::NoFrustumCulling;
use bevy::prelude::*;

use super::feed::Feed;
use super::rig::EyeCam;
use super::textures::PropMaps;
use super::Eye;
use super::Net;

/// Where the item sits in view space, metres. Low and to the right — the
/// reference frames all show the held item entering from the lower right and
/// leaving frame at the bottom, and a first cut near centre at arm's length
/// read as a prop floating in the world rather than as something carried.
pub const VIEWMODEL_HOLD: Vec3 = Vec3::new(0.32, -0.30, -0.52);

/// Wrist-to-palm correction, in the held item's own (tilted) frame, metres.
///
/// The grip point lands on the `RightHand` BONE, and a hand bone's origin is
/// the WRIST: the palm channel — where a rod actually crosses a fist — sits
/// a few centimetres beyond it toward the fingers. Without this, every item
/// rode low and right of the open palm and read as floating beside the hand
/// (operator, 2026-08-21: *"its not in the hand though?"*). Judged off
/// capture frames rather than derived: left and up into the finger curl, and
/// a touch DEEPER than the palm so the bind pose's open fingers draw in
/// front of the shaft — occlusion is what sells a grip on a rig whose hand
/// has no finger bones to close (`NOW.md` §0chr).
pub const VIEWMODEL_PALM: Vec3 = Vec3::new(-0.040, 0.030, -0.015);

/// The item's resting orientation in view space, YXZ Euler radians.
///
/// **Its absence was a real defect and the capture caught it.** The old
/// `hud.rs` viewmodel baked a tilt into the transform it spawned; moving the
/// hold onto a parent so one transform could be animated dropped it, and
/// `animate` then WROTE `rotation` outright every frame — so the rest pose
/// silently became identity and the axe sat axis-aligned with the view,
/// reading as a grey plate stuck to the lens. The sway and the swing compose
/// ONTO this now rather than replacing it.
pub const VIEWMODEL_TILT: Vec3 = Vec3::new(-0.50, 0.34, 0.14);

/// Bob cycles per metre walked. One cycle is TWO footfalls, and a stride is a
/// little under a metre, so this puts roughly one dip per step. In
/// cycles-per-metre and not cycles-per-second on purpose — see the header.
pub const VIEWMODEL_BOB_PER_M: f32 = 0.55;
/// Lateral bob amplitude at full walking speed, metres.
pub const VIEWMODEL_BOB_X: f32 = 0.014;
/// Vertical bob amplitude at full walking speed, metres.
pub const VIEWMODEL_BOB_Y: f32 = 0.011;
/// The speed at which the bob reaches full amplitude, m/s. Above it the
/// amplitude CLAMPS rather than growing, so a sprint does not throw the item
/// out of frame.
pub const VIEWMODEL_BOB_FULL_MPS: f32 = 4.0;

/// How far the item trails a turn — radians of trail per radian/second of look
/// rate.
pub const VIEWMODEL_SWAY_GAIN: f32 = 0.055;
/// How fast the trail catches up, per second; higher is stiffer. Applied as
/// `1 - exp(-k·dt)` rather than a raw per-frame `lerp`, because a raw lerp
/// makes the stiffness depend on the frame rate and the item would sway
/// differently at 30 and 144 fps.
pub const VIEWMODEL_SWAY_CATCHUP: f32 = 9.0;
/// The most the item may trail, radians — a hard clamp, so a mouse flick
/// cannot throw it across the frame.
pub const VIEWMODEL_SWAY_MAX: f32 = 0.16;

/// How long one swing takes, seconds.
pub const VIEWMODEL_SWING_S: f32 = 0.32;
/// How far the swing rotates, radians.
pub const VIEWMODEL_SWING_PITCH: f32 = 1.15;
/// How far the swing pushes away from the eye, metres.
pub const VIEWMODEL_SWING_PUSH: f32 = 0.13;

/// The parent of the held item's meshes. One transform to animate, so the
/// handle and the head cannot drift apart.
#[derive(Component)]
pub struct HeldItem;

/// The emitter a lit held item hangs in the world. One per session, spawned
/// dark beside the model and driven by [`hand_light`].
///
/// **A child of [`HeldItem`] and not of [`HeldModel`]**, which is the whole
/// reason it needs no work to follow the hand: `HeldItem` is the entity
/// `animate` writes, so the light inherits bob, sway and the swing arc for
/// free, and it is NOT the entity `swap` re-poses per item — a light hung
/// there would be rotated by `grip_m`'s slide and by `pose_yaw`, which are
/// corrections for where a MESH sits in a fist and mean nothing to a point
/// source. Its own offset is [`crate::ui::hold::HeldModelDef::flame_m`], up
/// the hold frame's +Y, and every row that declares a light is upright, so
/// that axis is the item's axis too.
///
/// **Driven by intensity, never by `Visibility`**, exactly as
/// `structures::FireLight` is: it is one entity that outlives every swap, so
/// there is nothing to spawn or despawn when the hand changes.
#[derive(Component)]
pub struct HandLight;

/// The child that carries whichever model is in hand. Separate from
/// [`HeldItem`] so `animate` keeps writing exactly one transform and the swap
/// below writes only handles — two systems, one entity each, no contention.
#[derive(Component)]
pub struct HeldModel {
    /// The [`crate::ui::hold::HELD_MODELS`] row on screen, or `None` for an
    /// empty hand. Cached so the swap is a comparison and not a respawn every
    /// frame: `Mesh3d` is a handle, and writing it unconditionally would
    /// re-trigger Bevy's change detection on the render world forever.
    shown: Option<usize>,
}

/// A generated model is authored standing up with its feet at y = 0 (see
/// `ci/import_meshy.py`), and a tool in hand points away from the eye. This is
/// the quarter-turn between those two facts: +Y becomes −Z.
///
/// Applied on the model child rather than folded into [`VIEWMODEL_TILT`],
/// because the tilt is the *pose of the hand* and this is a property of how
/// the asset was authored — one is art direction and the other is a file
/// format convention, and merging them makes the next asset's fix ambiguous.
const MODEL_UPRIGHT_TO_HELD: f32 = -std::f32::consts::FRAC_PI_2;

// The grip is per item and lives in `ui::hold::HeldModelDef::grip_m` — a
// point up the model's own +Y that `swap` rotates with the pose and lands on
// the fist. **One shared offset was the first cut and it does not survive the
// set**: a 0.10 m rock and a 1.80 m spear are a factor of eighteen apart, so
// an offset that seats the rock puts the spear's butt through the camera.
// `swap` writes it when the model changes.
//
// ⚠ **A fraction is only as good as the axis it is a fraction OF**, and that
// cost the rock a shipped frame: the table declared each model's longest axis
// while `grip_m` spent the fraction up +Y, which agree on a haft and do not
// agree on a stone that is twice as wide as it is tall. See
// `HeldModelDef::height_m`.

/// The motion state. A resource rather than a component: there is exactly one
/// held item, and this keeps `animate` one cheap system that queries only the
/// transform it writes.
#[derive(Resource, Default)]
pub struct Motion {
    /// Bob phase, radians. Accumulates with distance and never resets, so
    /// stopping and starting has no discontinuity in it.
    bob_phase: f32,
    /// Current trail, radians — x is yaw, y is pitch.
    sway: Vec2,
    last_pos: Vec3,
    last_yaw: f32,
    last_pitch: f32,
    /// Seeded on the first frame, so the first delta is not the whole world.
    /// Without it the eye's jump from the origin to the spawn — 2,179 m on the
    /// measured seed (`RENDER.md` §1.1) — lands in the first frame's speed.
    started: bool,
    /// Swing progress, counting DOWN from 1 to 0. Zero is at rest.
    swing: f32,
    /// The client's mirror of `Player::next_swing` — what turns a held
    /// mouse button into a chop at the sim's cadence instead of a blur at
    /// the frame rate. See [`crate::ui::swing`].
    cadence: crate::ui::swing::SwingCadence,
}

/// Spawn the held item under the camera, once.
///
/// **An `Update` system with a latch, and not `OnEnter(Screen::Loading)` where
/// `hud::setup` sits.** `mod.rs` records the reason and this slice paid it:
/// **`OnEnter(Screen::Loading)` runs BEFORE `Startup`** on a connected start,
/// because Bevy schedules the first state transition with
/// `insert_startup_before(PreStartup, …)`. `textures::load` is a `Startup`
/// system, so a viewmodel spawned on that transition asks for [`PropMaps`]
/// one schedule too early and the run dies with "Resource does not exist".
/// The HUD does not hit this only because it never wanted a texture.
///
/// The latch is a `Local<bool>` rather than a state, matching `props::stream`'s
/// `Local<Option<PropAssets>>`: one branch on a bool per frame, against the
/// alternative of a state transition that exists to fire one spawn.
pub fn spawn_item(
    mut commands: Commands,
    mut done: Local<bool>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    maps: Res<PropMaps>,
    cam: Query<Entity, With<EyeCam>>,
) {
    if *done {
        return;
    }
    let Ok(cam) = cam.single() else { return };
    *done = true;

    let wood = materials.add(StandardMaterial {
        base_color: Color::WHITE,
        base_color_texture: Some(maps.wood.albedo.clone()),
        normal_map_texture: Some(maps.wood.normal.clone()),
        perceptual_roughness: 0.82,
        // See `render::fresnel`: 0.14 was F0 0.31%.
        reflectance: super::fresnel::DIELECTRIC,
        ..default()
    });
    // **The head carries no map, and that is a sourcing gap stated rather than
    // papered over.** The only metal in `assets/` is `CorrugatedSteel009` —
    // photoscanned RIBBED SHEET. Two tile rates were tried, 9.0 and 1.1, and
    // both showed corrugation across a 10 cm blade, because the ribs are dense
    // in the source and no scaling removes a feature the map is made of. That
    // is the `Bricks089`-on-a-boulder failure a second time: **a map whose own
    // features are wrong for the identity cannot be rescaled into the right
    // one.** A plain worn-steel albedo is the entry `MANIFEST.md` is missing,
    // and until one is sourced this is flat steel with per-face value
    // break-up from the mesh — a knowing, recorded exception to `ART.md`
    // rule 1 on one 10 cm object, not a silent one.
    let steel = materials.add(StandardMaterial {
        base_color: Color::srgb(0.44, 0.45, 0.47),
        // Not a mirror. An earlier cut ran roughness 0.38 / metallic 0.85, and
        // a near-specular metal with no image-based lighting has nothing to
        // reflect but the atmosphere — the head came back pale sky-blue. A
        // polished one needs a reflection probe, which is a slice, not a knob.
        perceptual_roughness: 0.52,
        metallic: 0.55,
        reflectance: super::fresnel::METAL_DIELECTRIC,
        ..default()
    });

    commands.entity(cam).with_children(|c| {
        c.spawn((
            HeldItem,
            Transform::from_translation(VIEWMODEL_HOLD).with_rotation(Quat::from_euler(
                EulerRot::YXZ,
                VIEWMODEL_TILT.x,
                VIEWMODEL_TILT.y,
                VIEWMODEL_TILT.z,
            )),
            Visibility::Inherited,
        ))
        .with_children(|item| {
            // The generic tool, kept as the fallback for every item with no
            // model of its own. It is NOT what an empty hand draws — see
            // `swap` — it is what a revolver draws until a revolver is made.
            item.spawn((
                Mesh3d(meshes.add(super::heldgen::handle_mesh())),
                MeshMaterial3d(wood.clone()),
                Transform::from_translation(VIEWMODEL_PALM),
                Fallback,
            ));
            item.spawn((
                Mesh3d(meshes.add(super::heldgen::head_mesh())),
                MeshMaterial3d(steel.clone()),
                Transform::from_translation(VIEWMODEL_PALM),
                Fallback,
            ));
            // The model in hand. Spawned empty and filled by `swap`, which is
            // what keeps this file free of the inventory: it reads a row index
            // from `ui::hold` and never an item id.
            // **The empty handles are load-bearing, not tidiness.** `swap`'s
            // query is `(&mut HeldModel, &mut Mesh3d, &mut MeshMaterial3d,
            // &mut Visibility)`, and a Bevy query matches only entities that
            // have EVERY component in it. Spawned without these two the
            // entity exists, holds its transform, and is invisible to the one
            // system that fills it — which is not a compile error, not a
            // panic, and not a warning: the hand is simply always empty.
            // Cost one capture to find.
            item.spawn((
                HeldModel { shown: None },
                Mesh3d(Handle::default()),
                MeshMaterial3d::<StandardMaterial>(Handle::default()),
                Transform::from_rotation(Quat::from_rotation_x(MODEL_UPRIGHT_TO_HELD)),
                Visibility::Hidden,
            ));
            // What the item puts into the world, dark until `hand_light`
            // says otherwise. See [`HandLight`] for why it hangs here and
            // not on the model, and `structures::FireLight` for why its
            // shadows are off: a shadow-casting point light is six faces of
            // re-rasterised geometry, this one MOVES every frame, and the
            // sun already spends four cascades.
            item.spawn((
                HandLight,
                PointLight {
                    color: super::structures::FIRE_COLOR,
                    intensity: 0.0,
                    range: 0.0,
                    shadows_enabled: false,
                    ..default()
                },
                Transform::IDENTITY,
            ));
        });
    });
}

/// The two-primitive stand-in, so `swap` can hide it without knowing what it
/// is made of.
#[derive(Component)]
pub struct Fallback;

/// The first-person arms: the player's own character, from the inside.
///
/// ## What made this possible, and what nearly made it impossible
///
/// A viewmodel needs the arms and **nothing else** — and, it turned out,
/// only ONE of them: the hold clip is a two-handed grip and the support hand
/// tangles with the item, so [`VIEWMODEL_HIDDEN_ARM`] collapses the other arm
/// and carries that whole argument. What follows is about the body.
///
/// The obvious way to hide the body does not work on this character: it is one
/// mesh with one material, so `Visibility` (per entity) has no limb to reach,
/// and the only lever that does — collapsing a joint to zero scale — inherits
/// down the hierarchy, so hiding the torso hides the arms hanging off it. That was
/// reported as "not reachable on this asset", and the report was wrong in a
/// useful way: what was missing was not a trick but **something to hide**.
/// `ci/split_arms.py` makes one, splitting the mesh by skin weight into
/// [`anim::ARMS_NODE`] and [`anim::BODY_NODE`] — two nodes sharing one
/// skeleton, one material and one set of vertex buffers, differing only in
/// their index array. Hiding half is then a `Visibility`.
///
/// ## The placement is derived, not dialled in
///
/// [`VIEWMODEL_ARMS`] is arithmetic: the rig is measured to face **+Z**
/// (`headfront` sits +0.18 m in Z from `Head`, and the toes lead the hips),
/// a Bevy camera looks down `-Z`, so the arms are yawed 180°; then the offset
/// is whatever puts the hold clip's right hand exactly on [`VIEWMODEL_HOLD`],
/// where the item already sits. That is what lets the item parent to the HAND
/// while every constant in this file keeps its meaning — the bob, the sway and
/// the swing still move the viewmodel, they just move an arm that is holding
/// the thing instead of a thing floating beside it.
///
/// ⚠ **Derived is not the same as judged.** The numbers put the hand on the
/// hold point; whether an arm entering frame from that angle *reads* is a
/// question for a person with a GPU, and this box renders one frame every few
/// minutes. `--bin modelview <file> --eye --hide char1_body` previews the same
/// geometry.
#[derive(Component)]
pub struct ViewArms {
    /// The `RightHand` bone, once the scene has spawned it. The held item is
    /// re-parented onto this.
    hand: Option<Entity>,
    /// Whether the body half has been hidden and the player bound.
    dressed: bool,
}

/// Where the arms rig's own origin — the character's FEET — sits in view
/// space, and the yaw that turns it to face the way the camera looks.
///
/// **Derived** (see [`ViewArms`]): the rig's feet start `EYE_HEIGHT` below the
/// eye, and the offset added to that is `VIEWMODEL_HOLD` minus where
/// [`anim::ARMS_HOLD_CLIP`] puts the right hand in view space — measured at
/// `(0.082, -0.423, -0.279)`. So the hand lands on the hold point by
/// construction rather than by taste.
pub const VIEWMODEL_ARMS: Vec3 = Vec3::new(0.238, -1.477, -0.241);

/// The arm the first-person view does **not** draw, by bone name.
///
/// ## Why an arm is deleted rather than posed
///
/// [`super::anim::ARMS_HOLD_CLIP`] is `Pistol_Idle_Loop`, and it was chosen
/// because it is the rig's only **two-handed** hold that loops. That is the
/// right property for a pistol and the wrong one for everything this game puts
/// in a hand: a two-handed grip is a support hand clamped onto a weapon that
/// is not there, so the second hand lands on top of the first with nothing
/// between them. Measured off the shipped file across the whole 1.667 s loop,
/// with [`VIEWMODEL_ARMS`] applied:
///
///   · the hands stay **62–66 mm apart** for the whole loop — the tightest of
///     the rig's 22 looping clips by a factor of 2.9 (`Swim_Fwd_Loop` is next,
///     at 181 mm), and beaten across all 53 only by `Pistol_Aim_Up`,
///     `Pistol_Aim_Down`, `Pistol_Reload` and `Pistol_Shoot` — the same
///     two-handed grip, none of which loops;
///   · the left hand sits **31 mm NEARER the eye** than the right, so the open
///     support palm draws in FRONT of the held item;
///   · the left arm crosses the body's midline to get there — its shoulder is
///     on +X, its hand on −X, which on this rig (right = −X, proven by the
///     T-pose and by which hand `Punch_Cross` throws) is the far side.
///
/// That is the tangle the operator reported as *"our models hands are a bit
/// crossed?"* (2026-08-20), and it is a property of the POSE, so no offset
/// fixes it. Two things could: pick a one-handed clip, or stop drawing the
/// hand that is not holding anything.
///
/// **Hiding won, and the reason is the other half of the file.** Every
/// one-handed idle this rig owns presents its LEFT hand (`Idle_Torch_Loop`,
/// `Spell_Simple_Idle_Loop`, `Sword_Idle` — the torch, the spell and the
/// off-hand shield are all on +X), so switching clips moves the item into the
/// wrong hand: it re-derives [`VIEWMODEL_ARMS`] away from the one placement
/// that has been measured in a running client, it puts a left hand's chirality
/// at the lower right of every frame, and it points the hold away from
/// `Sword_Attack`, which is the operator's spoken gather swing (2026-08-17)
/// and swings the RIGHT arm. Hiding costs none of that, and one arm is what
/// the reference game draws for a one-handed tool anyway.
///
/// **Collapsing the shoulder is the mechanism**, not `Visibility`: both arms
/// are one node, one material and one index array (`ci/split_arms.py`), so
/// there is no entity to hide. A joint scaled to nothing takes every vertex
/// weighted to it — and its whole child chain — onto its own origin, which for
/// this bone sits at ndc y ≈ −1.8, comfortably under the bottom of the frame,
/// and further out still under the swing's push. `--bin modelview --hide` is
/// the same trick and its doc comment carries the same limit.
///
/// ⚠ **The write is one-shot, and that is only safe because this clip carries
/// no scale channel.** `Pistol_Idle_Loop` animates rotation on 22 joints and
/// translation on the hips, nothing else, so nothing overwrites the collapse
/// after [`dress_arms`] runs. `Idle_Loop` DOES animate scale, on all 24 — so a
/// future clip swap here would pop the arm back with no compile error and no
/// log line. `tests/viewmodel_arms.rs` gates exactly that, which is what buys
/// the zero per-frame cost.
pub const VIEWMODEL_HIDDEN_ARM: &str = "LeftShoulder";

/// What a collapsed joint is scaled to. Not zero: a zero-scale skinning matrix
/// is a degenerate basis, and every normal derived through it is a division by
/// nothing. At 1e-4 a vertex half a metre out lands 50 µm from the origin,
/// which is the same picture with arithmetic that stays finite.
pub const VIEWMODEL_HIDDEN_SCALE: f32 = 1e-4;

/// Spawn the arms once the rig's glTF is in.
///
/// A child of the camera, so the arms follow the view the way a viewmodel
/// must. **Not** a second instance of the world body — that one is drawn at
/// the player's own position by `bodies::stream` for everybody else, and the
/// local player's is never drawn at all.
pub fn spawn_arms(
    mut commands: Commands,
    mut done: Local<bool>,
    rig: Res<super::anim::Rig>,
    cam: Query<Entity, With<EyeCam>>,
) {
    if *done || !rig.ready() {
        return;
    }
    let (Ok(cam), Some(scene)) = (cam.single(), rig.scene.clone()) else {
        return;
    };
    *done = true;
    commands.entity(cam).with_children(|c| {
        c.spawn((
            ViewArms {
                hand: None,
                dressed: false,
            },
            SceneRoot(scene),
            Transform::from_translation(VIEWMODEL_ARMS)
                .with_rotation(Quat::from_rotation_y(std::f32::consts::PI))
                .with_scale(Vec3::splat(rig.scale)),
            Visibility::Inherited,
        ));
    });
}

/// Hide the body half, find the hand, start the hold clip, and move the held
/// item into the hand.
///
/// Runs until it has done all of that once — the scene spawns asynchronously,
/// so a one-shot pass at spawn time would find an empty entity and silently
/// leave a whole body wrapped around the camera.
/// Nine parameters, which clippy counts. A `SystemParam` struct would exist
/// only to satisfy the count — every one of these is a distinct thing the
/// dressing genuinely reads, and it runs once per session.
#[allow(clippy::too_many_arguments)]
pub fn dress_arms(
    mut commands: Commands,
    rig: Res<super::anim::Rig>,
    mut arms: Query<(Entity, &mut ViewArms)>,
    children: Query<&Children>,
    names: Query<&Name>,
    mut vis: Query<&mut Visibility>,
    mut xf: Query<&mut Transform>,
    players: Query<Entity, With<AnimationPlayer>>,
    meshes: Query<(), With<Mesh3d>>,
    item: Query<Entity, With<HeldItem>>,
) {
    let Ok((root, mut arms)) = arms.single_mut() else {
        return;
    };
    if arms.dressed {
        return;
    }
    // Bounded walk of the spawned scene — `anim::reshade`'s cap and its reason.
    let mut stack = vec![root];
    let mut seen = 0usize;
    let (mut body_half, mut hand, mut player) = (None, None, None);
    let mut off_arm = None;
    let mut drawn = Vec::new();
    while let Some(e) = stack.pop() {
        seen += 1;
        if seen > 512 {
            break;
        }
        if meshes.get(e).is_ok() {
            drawn.push(e);
        }
        if let Ok(n) = names.get(e) {
            match n.as_str() {
                super::anim::BODY_NODE => body_half = Some(e),
                "RightHand" => hand = Some(e),
                VIEWMODEL_HIDDEN_ARM => off_arm = Some(e),
                _ => {}
            }
        }
        if players.get(e).is_ok() {
            player = Some(e);
        }
        if let Ok(kids) = children.get(e) {
            stack.extend(kids.iter());
        }
    }
    // Required, not optional, and `body_half` is the precedent: the dressing
    // is one atomic step, so a name the asset stopped carrying leaves the
    // whole viewmodel undressed and loudly wrong rather than half-applied.
    // `tests/viewmodel_arms.rs` is what stops that reaching a build — it
    // resolves every name here against the shipped file.
    let (Some(body_half), Some(hand), Some(player), Some(off_arm)) =
        (body_half, hand, player, off_arm)
    else {
        return;
    };

    // **Frustum culling has to be turned off, and finding out why cost a
    // capture.** Everything below was already right — the hold clip played,
    // and a diagnostic measured the hand landing at (0.322, -0.305, -0.522)
    // against a target of (0.32, -0.30, -0.52) — and the arms still did not
    // appear in a single frame. The culler was throwing them away: a skinned
    // mesh's `Aabb` is its BIND box in mesh space, tested against the mesh
    // NODE's global transform, and that node hangs under an armature carrying
    // `scale 0.01`. So the box the culler tests is a 2 cm blob sitting 1.5 m
    // below the eye, comfortably outside the frustum, while the vertices the
    // GPU actually skins are right in front of the camera.
    //
    // This is the same fact that made the bench report a 1.8 m character as
    // 18 mm — **a skinned mesh is not where its node says it is** — arriving a
    // third time, in a third disguise. It is safe to disable here and only
    // here: the arms are a handful of triangles that are in view by
    // construction, so there is nothing for a culler to save.
    for e in drawn {
        commands.entity(e).insert(NoFrustumCulling);
    }

    // The half that is not arms. `Visibility` and not scale: a skinned mesh's
    // own node transform is ignored by the spec, so scaling it does nothing at
    // all — which is exactly the way this failed first time in the bench.
    if let Ok(mut v) = vis.get_mut(body_half) {
        *v = Visibility::Hidden;
    }

    // The arm that is not holding anything — see [`VIEWMODEL_HIDDEN_ARM`] for
    // the measurement and for why this is a scale and not a `Visibility`.
    // Scale is the one channel of the three that `Pistol_Idle_Loop` never
    // writes, so this survives every frame the clip plays without a system to
    // re-apply it.
    if let Ok(mut t) = xf.get_mut(off_arm) {
        t.scale = Vec3::splat(VIEWMODEL_HIDDEN_SCALE);
    }

    if let Some(graph) = rig.graph.clone() {
        let mut transitions = AnimationTransitions::new();
        let mut p = AnimationPlayer::default();
        transitions
            .play(&mut p, rig.arms_node(), std::time::Duration::ZERO)
            .repeat();
        commands
            .entity(player)
            .insert((AnimationGraphHandle(graph), transitions, p));
    }

    // **The item is deliberately NOT re-parented onto the hand yet**, and the
    // reason is honesty about what can be checked here. `VIEWMODEL_ARMS` puts
    // the hand ON the hold point, so the item already appears in the hand and
    // `animate` moves both with one motion — but a child of the hand needs its
    // grip offset and tilt re-derived against the arm's own frame, and a grip
    // is judged by looking at it, not computed. The hand is recorded so that
    // change is one line when somebody with a GPU can see the result.
    let _ = &item;

    arms.hand = Some(hand);
    arms.dressed = true;
    info!("viewmodel: arms up, body half hidden, {VIEWMODEL_HIDDEN_ARM} collapsed");
}

/// Say where the hand actually ended up, once, in VIEW space.
///
/// **A diagnostic that earns its place rather than a debug print left in.**
/// [`VIEWMODEL_ARMS`] is derived arithmetic — it should put the hand on
/// [`VIEWMODEL_HOLD`] — and the only way to know whether the derivation
/// survived contact with the scene graph is to measure the result in the
/// running client. It fires once, ~2 s in, costs nothing after, and it is what
/// turns "the arms are not in frame" from a guess into a number. The frame
/// delay is not decoration: the scene spawns over several frames and the
/// animation has to pose it before a bone is anywhere in particular.
pub fn arms_report(
    mut at: Local<u32>,
    arms: Query<&ViewArms>,
    cam: Query<&GlobalTransform, With<EyeCam>>,
    xf: Query<&GlobalTransform>,
) {
    if *at > 45 {
        return;
    }
    *at += 1;
    if *at != 45 {
        return;
    }
    let (Ok(arms), Ok(cam)) = (arms.single(), cam.single()) else {
        warn!("viewmodel: no arms entity or no camera to measure against");
        return;
    };
    let Some(hand) = arms.hand else {
        warn!("viewmodel: the arms never found a hand");
        return;
    };
    let Ok(h) = xf.get(hand) else { return };
    let local = cam.affine().inverse().transform_point3(h.translation());
    info!(
        "viewmodel: the hand sits at {:.3}, {:.3}, {:.3} in view space \
         (VIEWMODEL_HOLD is {:.2}, {:.2}, {:.2})",
        local.x, local.y, local.z, VIEWMODEL_HOLD.x, VIEWMODEL_HOLD.y, VIEWMODEL_HOLD.z
    );
}

/// Put the selected item's model in the hand.
///
/// **The three states are deliberately different pictures**, because
/// collapsing any two of them tells the player something false:
///
///   · a modelled item → its own model, stand-in hidden
///   · an item with no model yet → the stand-in tool, as before
///   · an EMPTY hand → neither, because a tool that appears when you are
///     holding nothing is a lie about your own inventory, and the hotbar
///     right next to it says the cell is empty
///
/// Handles are loaded once into [`Models`] rather than per swap: `AssetServer`
/// dedups, but a `load` per frame still walks a path and hashes it, and this
/// runs every frame by construction.
pub fn swap(
    net: Option<NonSend<Net>>,
    models: Res<Models>,
    mut q: Query<(
        &mut HeldModel,
        &mut Mesh3d,
        &mut MeshMaterial3d<StandardMaterial>,
        &mut Visibility,
        &mut Transform,
    )>,
    mut fallback: Query<&mut Visibility, (With<Fallback>, Without<HeldModel>)>,
) {
    let (want, empty) = match net.as_deref() {
        Some(n) => {
            let core = &n.session.core;
            let stack = core
                .inv
                .get(usize::from(n.sel).min(core.inv.len() - 1))
                .copied();
            (
                crate::ui::hold::held_model_in_hand(&core.catalog, &core.inv, n.sel),
                stack.is_none_or(|s| s.count == 0),
            )
        }
        None => (None, true),
    };

    for (mut held, mut mesh, mut mat, mut vis, mut tf) in &mut q {
        if held.shown != want {
            match want {
                Some(i) => {
                    let def = &crate::ui::hold::HELD_MODELS[i];
                    mesh.0 = models.mesh[i].clone();
                    mat.0 = models.mat[i].clone();
                    // The whole pose is a property of the ITEM, so all of it
                    // is written here and none at spawn: lay a tool forward,
                    // keep a carried thing upright, then slide the model so
                    // its grip point — `grip_m` up its own +Y — lands on this
                    // entity's origin, which is where the fist is. The grip
                    // vector is rotated WITH the model; writing it on a fixed
                    // axis is the bug this replaces, which hung every tool
                    // `grip_m` below the hand (63 cm, for the spear).
                    let lay = if def.lay_forward {
                        Quat::from_rotation_x(MODEL_UPRIGHT_TO_HELD)
                    } else {
                        Quat::IDENTITY
                    };
                    // The presentation yaw composes in the hand's frame, so
                    // it turns the item about the fist rather than about its
                    // own foot, and the grip point below stays in the palm
                    // under any yaw.
                    let rot = Quat::from_rotation_y(def.pose_yaw) * lay;
                    tf.rotation = rot;
                    tf.translation = VIEWMODEL_PALM - (rot * (Vec3::Y * def.grip_m()));
                    tf.scale = Vec3::splat(def.scale);
                }
                None => {
                    mesh.0 = Handle::default();
                    mat.0 = Handle::default();
                }
            }
            held.shown = want;
        }
        *vis = if want.is_some() {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
    // The stand-in covers "no model of its own", never "nothing in hand".
    let show_fallback = want.is_none() && !empty;
    for mut v in &mut fallback {
        *v = if show_fallback {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
}

/// Drive the hand's emitter off whichever [`crate::ui::hold::HELD_MODELS`]
/// row is in the fist.
///
/// **Night had no counter before this.** `rig::day_night` takes the sun to
/// zero illuminance, kills the environment map and the sky brightness, and
/// leaves `NIGHT_AMBIENT_LUX` — 60 lux of direction-free ambient — as the
/// entire lighting of a tenth of every cycle, while `mob::think` sends the
/// wolves out into it. The starter kit has put a torch on hotbar slot 2
/// since the kit existed (`content/balance.toml`), and holding it did
/// nothing at all.
///
/// **It reads the same function `swap` reads and that is not a second
/// drain.** `hold::held_model_in_hand` is a pure lookup over the catalog
/// and the inventory mirror — the single-consumer trap `feed.rs` narrates
/// is about the destructive `pop_*` rings, and this touches none of them.
/// Two readers of a pure function is two calls.
///
/// No allocation, no branch per frame beyond the row lookup: the emitter is
/// one entity that outlives every swap.
pub fn hand_light(
    net: Option<NonSend<Net>>,
    q: Query<(&mut PointLight, &mut Transform), With<HandLight>>,
) {
    let want = net.as_deref().and_then(|n| {
        let core = &n.session.core;
        crate::ui::hold::held_model_in_hand(&core.catalog, &core.inv, n.sel)
    });
    apply_hand_light(want, q);
}

/// [`hand_light`] with the held row as a value. The half a gate can drive —
/// `structures::apply_fire_lights` is the same split for the same reason:
/// the socket is the only thing the system adds.
///
/// A row with no `light` and an empty hand are one case on purpose. There is
/// no third state: an emitter that is off is a zero, not a hidden entity,
/// so nothing here can leave a light burning for an item that is no longer
/// in the hand.
pub fn apply_hand_light(
    want: Option<usize>,
    mut q: Query<(&mut PointLight, &mut Transform), With<HandLight>>,
) {
    let (lumens, range, lift) = match want.map(|i| &crate::ui::hold::HELD_MODELS[i]) {
        Some(row) => match row.light {
            Some(l) => (l.lumens, l.range_m, row.flame_m()),
            None => (0.0, 0.0, 0.0),
        },
        None => (0.0, 0.0, 0.0),
    };
    for (mut light, mut tf) in &mut q {
        // Assign only on a change, `apply_fire_lights`' reason exactly:
        // `PointLight` is `Changed`-tracked and the render world re-extracts
        // what moved, so writing the same lumens every frame would re-upload
        // this light forever. The transform is the same — and it is the
        // stronger case here, because a `Transform` write on a parent of
        // nothing still dirties the propagation.
        if light.intensity != lumens {
            light.intensity = lumens;
        }
        if light.range != range {
            light.range = range;
        }
        if tf.translation.y != lift {
            tf.translation.y = lift;
        }
    }
}

/// The held-item models, index-aligned with [`crate::ui::hold::HELD_MODELS`].
#[derive(Resource)]
pub struct Models {
    mesh: Vec<Handle<Mesh>>,
    mat: Vec<Handle<StandardMaterial>>,
}

/// Load every held model once at startup. For the `.glb` rows, the same
/// `Primitive`/`Material` pair `structures::build_kit` uses, and for the same
/// reason: these are single-primitive assets, so the label lands in an
/// ordinary handle and no scene hierarchy is spawned. `tests/held_assets.rs`
/// is what keeps that true. The generated rows are built here, once — the
/// same startup, the same vectors, so `swap` cannot tell the sources apart.
pub fn load_models(
    mut commands: Commands,
    assets: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let (mut mesh, mut mat) = (Vec::new(), Vec::new());
    for m in &crate::ui::hold::HELD_MODELS {
        match m.src {
            crate::ui::hold::HeldSrc::Glb(path) => {
                mesh.push(
                    assets.load(
                        GltfAssetLabel::Primitive {
                            mesh: 0,
                            primitive: 0,
                        }
                        .from_asset(path),
                    ),
                );
                mat.push(
                    assets.load(
                        GltfAssetLabel::Material {
                            index: 0,
                            is_scale_inverted: false,
                        }
                        .from_asset(path),
                    ),
                );
            }
            crate::ui::hold::HeldSrc::Gen(name) => {
                mesh.push(meshes.add(super::heldgen::mesh(name)));
                mat.push(materials.add(super::heldgen::material(name)));
            }
        }
    }
    commands.insert_resource(Models { mesh, mat });
}

/// Integrate the three motions and write the one transform.
pub fn animate(
    time: Res<Time>,
    eye: Res<Eye>,
    feed: Res<Feed>,
    // `Option`, like `swap`'s: a capture run has no session, and a
    // viewmodel that refused to draw without one would take the held item
    // out of every probe frame.
    net: Option<NonSend<Net>>,
    mut m: ResMut<Motion>,
    // `Without<ViewArms>` is not decoration: two `&mut Transform` queries in
    // one system have to be PROVABLY disjoint, and two different `With`
    // markers do not prove it — an entity could carry both.
    mut q: Query<&mut Transform, (With<HeldItem>, Without<ViewArms>)>,
    mut arms: Query<&mut Transform, With<ViewArms>>,
) {
    let Ok(mut t) = q.single_mut() else { return };
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }
    // The byte the sim will act on, not the mouse — see `ClientCore::buttons`.
    let swinging = net
        .as_deref()
        .is_some_and(|n| n.session.core.buttons() & sim_core::input::BTN_PRIMARY != 0);

    if !m.started {
        m.started = true;
        m.last_pos = eye.pos;
        m.last_yaw = eye.yaw;
        m.last_pitch = eye.pitch;
        return;
    }

    // ── Bob, off distance ────────────────────────────────────────────────
    // Horizontal only: falling is not walking, and counting the vertical
    // component would bob the item while the player drops off a ledge.
    let step = eye.pos - m.last_pos;
    let dist = Vec2::new(step.x, step.z).length();
    m.last_pos = eye.pos;
    m.bob_phase += dist * VIEWMODEL_BOB_PER_M * std::f32::consts::TAU;
    let speed = (dist / dt).min(VIEWMODEL_BOB_FULL_MPS);
    let amp = speed / VIEWMODEL_BOB_FULL_MPS;
    // The vertical term is at twice the lateral frequency: a body dips once
    // per FOOTFALL but shifts weight side to side once per STRIDE, which is
    // two footfalls. One frequency for both is the tell of a canned bob.
    let bob = Vec3::new(
        m.bob_phase.sin() * VIEWMODEL_BOB_X * amp,
        -(m.bob_phase * 2.0).cos().abs() * VIEWMODEL_BOB_Y * amp,
        0.0,
    );

    // ── Sway, off look rate ──────────────────────────────────────────────
    // Yaw wraps, so the delta is taken on the shortest arc; without this a
    // pass through ±π is a full-circle spike and the item snaps.
    let dyaw = wrap_pi(eye.yaw - m.last_yaw);
    let dpitch = eye.pitch - m.last_pitch;
    m.last_yaw = eye.yaw;
    m.last_pitch = eye.pitch;
    let target = Vec2::new(
        (-dyaw / dt * VIEWMODEL_SWAY_GAIN).clamp(-VIEWMODEL_SWAY_MAX, VIEWMODEL_SWAY_MAX),
        (-dpitch / dt * VIEWMODEL_SWAY_GAIN).clamp(-VIEWMODEL_SWAY_MAX, VIEWMODEL_SWAY_MAX),
    );
    // Frame-rate independent first-order lag.
    let k = 1.0 - (-VIEWMODEL_SWAY_CATCHUP * dt).exp();
    let sway = m.sway;
    m.sway = sway + (target - sway) * k;

    // ── Swing, off the cadence ───────────────────────────────────────────
    // The sim's own rule, mirrored: button down and the cooldown lapsed.
    // This is what draws a MISS, which is most swings — see the header.
    // Retriggers from the top rather than adding, so holding the button
    // down is a steady chop instead of a wind-up that never resolves.
    //
    // Both terms are bound before the branch rather than short-circuited:
    // `poll` ADVANCES the cadence, so it has to run on every frame the arm
    // is held whatever the other term says, or a swing the feed happened to
    // draw would leave the predictor's window unspent and the next one would
    // come early.
    let predicted = m.cadence.poll(swinging, feed.server_tick_est);
    // The backstop, and the `<= 0.0` is the whole of it: a landed hit
    // arrives about a round trip into a stroke the prediction already
    // started, and restarting the arc there is a visible stutter. At rest
    // it can only mean the prediction missed one, and a swing drawn late
    // beats a swing not drawn.
    let landed_at_rest = m.swing <= 0.0 && (feed.hits > 0 || !feed.gathered().is_empty());
    if predicted || landed_at_rest {
        m.swing = 1.0;
    }
    if m.swing > 0.0 {
        m.swing = (m.swing - dt / VIEWMODEL_SWING_S).max(0.0);
    }
    // 0 at rest, 1 at the middle of the stroke: a half sine over the swing, so
    // it accelerates in and decelerates out with no discontinuity at either
    // end. `1 - swing` runs the stroke forwards as `swing` counts down.
    let arc = (std::f32::consts::PI * (1.0 - m.swing)).sin();

    t.translation = VIEWMODEL_HOLD + bob + Vec3::new(0.0, arc * 0.05, arc * VIEWMODEL_SWING_PUSH);
    // Composed onto the rest pose, never replacing it — see `VIEWMODEL_TILT`.
    let rest = Quat::from_euler(
        EulerRot::YXZ,
        VIEWMODEL_TILT.x,
        VIEWMODEL_TILT.y,
        VIEWMODEL_TILT.z,
    );
    let motion = Quat::from_euler(
        EulerRot::YXZ,
        m.sway.x,
        m.sway.y - arc * VIEWMODEL_SWING_PITCH,
        0.0,
    );
    t.rotation = motion * rest;

    // ── The arms ride the same motion ────────────────────────────────────
    //
    // **Translation only, and the omission is the point.** The item rotates
    // about its own origin, which is in the hand; the arms rotate about the
    // character's FEET, a metre and a half away, so the same sway applied to
    // both would swing the hands out of frame while the item stayed put. So
    // the arms take the bob and the swing's push — which are displacements of
    // the whole body and read correctly from either pivot — and not the sway
    // or the swing's pitch.
    //
    // The two therefore travel together, which is what keeps the hand on the
    // item without the item being parented to the hand. Parenting it is the
    // better design and the follow-up: it needs the grip retuned against the
    // arm, and a grip is judged by looking rather than derived.
    if let Ok(mut a) = arms.single_mut() {
        a.translation =
            VIEWMODEL_ARMS + bob + Vec3::new(0.0, arc * 0.05, arc * VIEWMODEL_SWING_PUSH);
    }
}

/// Forget everything the last session put in [`Motion`] — `map::forget`'s
/// twin, registered on the same two transitions.
///
/// **Every field in `Motion` is session-scoped and none of them was being
/// cleared.** `started`/`last_pos` are the pair `Motion`'s own doc explains:
/// seeded on the first frame so the eye's jump from the origin to the spawn
/// — 2,179 m on the measured seed — does not land in the first frame's
/// speed. That seeding only happens while `started` is false, so a *second*
/// session inherited the first one's last position and paid the jump the
/// latch exists to avoid. The swing cadence has the same shape for a
/// sharper reason: it holds an absolute server tick, and reconnecting to a
/// shard whose clock is *behind* the last one leaves the arm on a cooldown
/// measured in somebody else's ticks (`ui::swing`'s
/// `a_reset_lets_a_fresher_shard_swing_again`).
pub fn forget(mut m: ResMut<Motion>) {
    *m = Motion::default();
}

/// Shortest signed arc into `-π..π`.
fn wrap_pi(a: f32) -> f32 {
    let tau = std::f32::consts::TAU;
    let mut x = a % tau;
    if x > std::f32::consts::PI {
        x -= tau;
    } else if x < -std::f32::consts::PI {
        x += tau;
    }
    x
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yaw_wraps_the_short_way() {
        // A step across the ±π seam is a small delta, not a full circle. This
        // is the assertion that stops the item snapping once per revolution.
        let d = wrap_pi(-3.13 - 3.13);
        assert!(d.abs() < 0.03, "wrapped delta was {d}");
        assert!((wrap_pi(0.2) - 0.2).abs() < 1e-6);
    }
}
