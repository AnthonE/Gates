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
//!     backstop. **Its shape is the row's** ([`Stroke`]): a hatchet chops
//!     ([`swing_pose`]), a hammer taps, a rock or a fist bashes and a spear
//!     thrusts ([`thrust_pose`]), on one clock, so the blow lands at one
//!     instant whichever it is.
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
//! picks a pose, and the swing cue takes the same liberty — from the same
//! line, since 2026-09-13; it used to fire on the mouse press and drifted from
//! the arm the moment a player clicked faster than the sim swings
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
use crate::ui::hold::{held_model_in_hand, HeldModelDef, Stroke, HELD_MODELS};

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
/// (operator, 2026-08-21: *"its not in the hand though?"*).
///
/// ## Solved off the mesh, 2026-09-01, and the old value was 9.6 cm short
///
/// It used to be judged off capture frames — 5.3 cm, "left and up into the
/// finger curl" — and that was the right instinct at the wrong magnitude.
/// Measured in the `RightHand` bone's own frame, the old seat put the haft
/// **5.4 cm from the wrist bone**, at the heel of the palm, with the nearest
/// finger vertex **9.1 cm away** on a hand 21.7 cm long. Nothing was gripping
/// anything; the haft only LOOKED like it crossed the hand because a
/// projection flattens the two together (operator, 2026-09-01: *"the axe need
/// to probably move to the fingers some. its off just a bit"* — the read was
/// right and the "bit" was half a hand).
///
/// The value now is where a cylinder of the haft's own drawn radius actually
/// fits: swept over the bone frame, the seat that maximises clearance while
/// the hand still closes AROUND it. It lands at (4.5, 13.0, −1.5) cm — on the
/// knuckle line, which is where a rod crosses a fist — with 2.57 cm of
/// clearance and hand in **all sixteen** of the directions sampled
/// perpendicular to the haft. The old seat had the hand on one side of it.
///
/// It is a translation, so the item's ORIENTATION is untouched: the pose
/// angles in `ui::hold::HELD_MODELS` still mean exactly what they measured.
/// `bodies::hand_pose` takes the same constant, so the remote body's grip
/// moves with it — which is the point, there is one grip.
pub const VIEWMODEL_PALM: Vec3 = Vec3::new(-0.0854, 0.0426, -0.0981);

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
///
/// ⚠ **PROPOSED — a tunable no doc has spoken, so it ships carrying its
/// derivation** (`RIG_SUN_ARC`'s convention) and belongs in `DECISIONS.md`
/// §open ("viewmodel swing v1") the day it is.
///
/// **It is deliberately NOT [`super::anim::SWING_CLIP_S`]**, which is the
/// stroke a remote body plays for the same event, and the two bounds that
/// pick it are the reason:
///
///   · the whole stroke recovers inside the sim's own cadence
///     (`SWING_INTERVAL_TICKS / TICK_HZ` = 1.267 s) with the arm back at
///     rest, so holding the button is a steady chop and never a wind-up
///     that never resolves — 0.45 s is a third of it;
///   · the arc's **apex** lands at `VIEWMODEL_SWING_WINDUP + (1 −
///     VIEWMODEL_SWING_WINDUP)·VIEWMODEL_SWING_ATTACK` = 0.35 of the
///     stroke, so 0.16 s in. That is the half `SWING_CLIP_S` cannot
///     satisfy: the sim resolves your own swing on the frame the button
///     goes down and `animate` plays the cue there, so an apex a
///     third of a second later reads as the picture lagging the sound.
///     A remote body has no such constraint — nothing of theirs is
///     synchronised to your speakers.
pub const VIEWMODEL_SWING_S: f32 = 0.45;

/// The point the whole viewmodel — arm, hand and item — turns about, in view
/// space, metres.
///
/// **Not the eye, and the difference is the whole reason the arc reads.** The
/// item sits 0.52 m in front of the camera; rotating it about the camera's own
/// origin by the angles below takes it round *toward the lens* and out through
/// the near plane (measured: 0.11 m of depth at a 0.86 rad pitch, against a
/// 0.1 m near plane). Behind and slightly below the eye is roughly where a
/// shoulder is, and swinging about it sweeps the item **across** the frame
/// instead of into it.
pub const VIEWMODEL_SWING_PIVOT: Vec3 = Vec3::new(0.05, -0.10, 0.30);

/// What fraction of the stroke is the wind-up. The rest is the strike and its
/// recovery.
pub const VIEWMODEL_SWING_WINDUP: f32 = 0.30;
/// How much of the STRIKE is the fall toward the apex; the remainder is the
/// recovery back to rest. Under a half, so the blow is faster than the arm's
/// return — a symmetric strike reads as a wave rather than a chop.
pub const VIEWMODEL_SWING_ATTACK: f32 = 0.35;

/// The chop's wind-up at its peak, as a turn of the whole arm about
/// [`VIEWMODEL_SWING_PIVOT`], YXZ radians: up, and a little in toward the
/// head, the way a right hand goes back over the shoulder.
///
/// **No roll, in the wind-up or the strike, and that is the fix.** The arc this
/// replaced swept yaw and roll together by 0.55 rad, which is a chop only for
/// a tool laid along −Z: roll spins about the view axis, so a haft carried
/// head-up (the hatchet, 54.8° up) was laid over sideways like a wiper and its
/// head trailed the fist across the frame. Pitch and yaw tip a haft FORWARD
/// whichever way it rests, and the wrist ([`chop_snap`]) does the rest per row.
pub const VIEWMODEL_SWING_COCK: Vec3 = Vec3::new(0.10, 0.30, 0.0);
/// Where the wind-up displaces the whole rig, metres: up and back toward the
/// shoulder.
pub const VIEWMODEL_SWING_DRAW: Vec3 = Vec3::new(-0.03, 0.10, 0.06);
/// The chop's strike at the apex, YXZ radians about the shoulder: in toward
/// the crosshair and down. Small next to the wrist's turn on purpose — the fist
/// stays in the lower right of the frame and the head travels.
pub const VIEWMODEL_SWING_STRIKE: Vec3 = Vec3::new(0.18, -0.10, 0.0);
/// Where the strike displaces the whole rig, metres: in, down and out.
pub const VIEWMODEL_SWING_THROW: Vec3 = Vec3::new(-0.04, -0.02, -0.08);
/// How far the wrist cocks the item back in the fist at the wind-up's peak,
/// radians, about the axis it then strikes about: the head goes back over the
/// shoulder before it comes down.
pub const VIEWMODEL_SWING_BACK: f32 = 0.25;
/// How far below the view axis the chop lands, radians. The head's crown
/// finishes on this ray ([`chop_snap`]), so a chop comes down THROUGH the
/// crosshair and bites just under it rather than stopping on it.
pub const VIEWMODEL_SWING_DROP: f32 = 0.12;
/// The most the chop's wrist may turn the item, radians: a cap on the solve
/// in [`chop_snap`], for a row too short or too oddly carried to reach the
/// aim ray. Every chopping row in `ui::hold` lands under it, and the gate
/// says so.
pub const VIEWMODEL_SWING_WRIST_MAX: f32 = 1.45;

/// A [`Stroke::Tap`] (the hammer) is the chop at this fraction: the same
/// path, arm and wrist, shorter. A repair knock, not a felling blow.
pub const VIEWMODEL_TAP: f32 = 0.45;

/// A [`Stroke::Swipe`]'s wind-up (the torch), YXZ radians about the shoulder
/// and metres: a short draw up and out from where the torch is already
/// carried, up by the head, rather than the chop's heave over the shoulder.
/// The head comes down onto the chop's aim ray at the chop's apex — a
/// forehand arc from the upper right through the crosshair, with the flame
/// on screen all the way.
pub const VIEWMODEL_SWIPE_COCK: Vec3 = Vec3::new(-0.04, 0.06, 0.0);
pub const VIEWMODEL_SWIPE_DRAW: Vec3 = Vec3::new(0.0, 0.02, 0.05);
/// The swipe's strike: the chop's, swept further in across the body by the
/// out-turn the torch is carried at ([`VIEWMODEL_LIFT`]), so the head comes
/// down across the frame onto the crosshair rather than beside it.
pub const VIEWMODEL_SWIPE_STRIKE: Vec3 = Vec3::new(
    VIEWMODEL_SWING_STRIKE.x - VIEWMODEL_LIFT.x,
    VIEWMODEL_SWING_STRIKE.y,
    VIEWMODEL_SWING_STRIKE.z,
);
/// How far the swipe's wrist cocks the head back, radians — a flick, where
/// the chop's [`VIEWMODEL_SWING_BACK`] lays the head over the shoulder.
pub const VIEWMODEL_SWIPE_BACK: f32 = 0.08;

/// The bash (a rock, an empty fist, anything small): the wind-up draws the
/// fist back and out, YXZ radians about the shoulder…
pub const VIEWMODEL_BASH_COCK: Vec3 = Vec3::new(-0.06, 0.06, 0.0);
/// …and back toward the eye, metres…
pub const VIEWMODEL_BASH_DRAW: Vec3 = Vec3::new(0.02, 0.02, 0.10);
/// …then the strike drives it in and up toward the crosshair…
pub const VIEWMODEL_BASH_STRIKE: Vec3 = Vec3::new(0.20, 0.10, 0.0);
/// …and out, away from the eye: a punch, about 30 cm of reach from the draw.
pub const VIEWMODEL_BASH_THROW: Vec3 = Vec3::new(-0.02, 0.02, -0.20);
/// How far the item in a bashing fist tips forward at the apex, and back at
/// the wind-up, radians — the stone's face leads the blow.
pub const VIEWMODEL_BASH_TURN: f32 = 0.35;
pub const VIEWMODEL_BASH_BACK: f32 = 0.15;

/// How far the thrust's wind-up cocks the rig out to the right, radians — a
/// yaw about [`VIEWMODEL_SWING_PIVOT`]. **(knob)** — `DECISIONS.md` §open,
/// "viewmodel thrust v0", together with the six that follow.
///
/// The thrust is [`swing_pose`]'s shape with a different path: the same
/// wind-up/strike split on the same clock, so [`swing_apex_s`] is one instant
/// for both strokes and everything timed against it (the cue, the remote
/// body's clip) stays timed. What differs is that a chop turns about the
/// shoulder and sweeps, where a thrust mostly TRANSLATES — back, then forward
/// — and the rotations here are small and in service of the line: the fist
/// cocks out and lifts on the draw, then turns in as the arm goes out.
pub const VIEWMODEL_THRUST_COCK: f32 = 0.12;
/// How far the wind-up pitches the rig up, radians: the point lifts as the
/// spear is drawn back, the way a spear is actually cocked.
pub const VIEWMODEL_THRUST_LIFT: f32 = 0.18;
/// How far the strike yaws the rig INWARD, radians — toward the view axis,
/// which is where a fist goes when the arm extends across the chest. About
/// the shoulder pivot this also carries the fist forward, so it is part of
/// the extension as well as of the line.
pub const VIEWMODEL_THRUST_TURN: f32 = 0.18;
/// How far the strike pitches the rig up, radians. Small on purpose: the
/// point's climb to the crosshair is the wrist's job ([`thrust_snap`]), and
/// this only stops the fist dropping as the arm goes out.
pub const VIEWMODEL_THRUST_RISE: f32 = 0.08;
/// Where the wind-up displaces the whole rig, metres: back toward the eye, a
/// little up and out. The draw — 14 cm of it, which is what makes the push
/// read as a push rather than as the item growing.
pub const VIEWMODEL_THRUST_DRAW: Vec3 = Vec3::new(0.03, 0.02, 0.14);
/// Where the strike displaces the whole rig, metres: forward, inward and up.
/// The Z term is the extension — how far the arm goes out past rest — and
/// with [`VIEWMODEL_THRUST_TURN`]'s share it is about 0.21 m at the apex.
///
/// **Sized against the sim's arm and not against a frame.** The point lands
/// where [`thrust_snap`] puts it, and how DEEP that is follows from this push
/// and the row's length: for the spear, ~1.99 m down the view axis, which is
/// the centre of a body standing at the spear's 2 m content reach. Push
/// further and the picture stabs people the sim cannot reach; push less and
/// the arm does not extend. `tests/viewmodel_arms.rs` holds the depth inside
/// that body, ± its capsule radius.
pub const VIEWMODEL_THRUST_PUSH: Vec3 = Vec3::new(-0.04, 0.06, -0.16);
/// The most the thrust's wrist may turn the item, radians. **(knob)**
///
/// Where the chop's [`VIEWMODEL_SWING_WRIST_MAX`] caps an aim every row
/// shares, this caps a SOLVE: [`thrust_snap`] computes the exact turn that
/// puts the row's point on the view axis, and for the spear that is 0.50 rad.
/// The cap is for a row that asks more than a wrist can give — something
/// short carried across the body — and it is a clamp rather than a refusal,
/// so the arithmetic stays finite; the refusal is the gate's, which reads the
/// point's lateral miss and goes red on a clamped row rather than letting the
/// spear land beside the crosshair with nothing red anywhere.
pub const VIEWMODEL_THRUST_WRIST_MAX: f32 = 0.70;

/// Where a raised bow sits, as a displacement of the whole rig, metres: in
/// to the middle of the frame and up, the bow arm lifted to aim
/// (`reference/PROJECTILES.md` §6). Reached within [`VIEWMODEL_RAISE_RATE`]
/// of the right mouse going down.
pub const VIEWMODEL_DRAW_RAISE: Vec3 = Vec3::new(-0.22, 0.14, 0.0);
/// What the draw adds as it fills, metres: the bow pulled back toward the
/// eye as the string comes to the cheek. Full exactly when the sim would
/// let it loose (`ui::draw`), so full draw is something the player sees.
pub const VIEWMODEL_DRAW_PULL: Vec3 = Vec3::new(0.0, 0.0, 0.07);
/// How far a raised bow turns the rig, YXZ Euler radians: yawed in toward
/// the view axis and canted, the archer's hold.
pub const VIEWMODEL_DRAW_TURN: Vec3 = Vec3::new(0.10, 0.04, -0.22);
/// How fast the bow comes up and goes back down, per second, as `1 -
/// exp(-k·dt)` (the sway's form).
pub const VIEWMODEL_RAISE_RATE: f32 = 14.0;
/// The loose: how far the release kicks the rig, metres, and how long.
pub const VIEWMODEL_LOOSE_KICK: Vec3 = Vec3::new(0.0, 0.015, -0.08);
pub const VIEWMODEL_LOOSE_S: f32 = 0.2;
/// A [`Stroke::Shot`] row's recoil on the same clock: back toward the eye
/// and up, metres, with the muzzle climbing, radians.
pub const VIEWMODEL_SHOT_KICK: Vec3 = Vec3::new(0.0, 0.012, 0.05);
pub const VIEWMODEL_SHOT_CLIMB: f32 = 0.10;
/// How much a full draw narrows the view: raised, the bow zooms by most of
/// this; the pull takes the rest.
pub const DRAW_ZOOM: f32 = 0.15;
/// How far a raised bow is canted about its arrow, radians: the top limb
/// tipped in toward the frame's middle, so the limbs and the string read
/// across the view instead of edge-on. See [`bow_aim`].
pub const VIEWMODEL_BOW_CANT: f32 = 0.5;
/// How fast a draw let down without a shot eases the string home, per
/// second, as `1 - exp(-k·dt)`.
pub const VIEWMODEL_STRING_EASE: f32 = 10.0;

/// The heave (`animate`): how much of a jolt in the body's vertical speed
/// the arm takes, the spring that brings it back (stiffness per second²,
/// damping per second: about 1.7 Hz, well damped), the most it moves,
/// metres, and the largest jolt it hears, m/s, so a respawn's teleport is a
/// nudge. A jump dips it about 2 cm on the way up and on landing.
pub const VIEWMODEL_HEAVE_GAIN: f32 = 0.07;
pub const VIEWMODEL_HEAVE_K: f32 = 120.0;
pub const VIEWMODEL_HEAVE_C: f32 = 13.0;
pub const VIEWMODEL_HEAVE_MAX: f32 = 0.04;
pub const VIEWMODEL_HEAVE_JOLT_MAX: f32 = 12.0;

/// How far the view is zoomed for a drawn bow, 0..=1 of [`DRAW_ZOOM`] —
/// written by [`animate`], read by `settings::apply_view`.
#[derive(Resource, Default)]
pub struct DrawZoom(pub f32);

/// Where the item sits in the **`RightHand` bone's own frame** while that
/// hand is exactly as the hold clip poses it — the REST hand, which is what
/// [`hand_rest`] reads and every [`HandFit`] is measured from. The hand a
/// row actually draws is turned off it by [`hand_fit`].
///
/// ## Derived off the shipped file, and the derivation is the whole entry
///
/// `DECISIONS.md`'s 2026-08-17 row closed with *"the item is not parented to
/// the hand, because a grip is judged by looking rather than derived"*, and
/// that was right about a grip and wrong about **this** grip: there is one
/// placement nobody has to judge, which is the one that leaves the picture
/// exactly where it already is. So the constant is not a new pose — it is
/// `hand⁻¹ ∘ (T(VIEWMODEL_HOLD) · R(VIEWMODEL_TILT))`, sampled off
/// [`super::anim::ARMS_HOLD_CLIP`]'s first frame with [`VIEWMODEL_ARMS`]
/// applied. At rest the item is within a millimetre of where it hung as a
/// child of the camera; what changes is that it now **moves with the hand**.
///
/// **The defect it closes was reported from play** — *"held items are shitty
/// looking like again they are not under the parent"* (operator, 2026-08-30)
/// — and it was exactly that: the item and the arms were siblings, so the
/// hold clip posed the hand and the item stayed where the camera put it.
/// Small at rest (the hold loop moves the hand 7 mm) and total under any
/// motion the arm is given.
///
/// `crates/client/tests/viewmodel_arms.rs` re-derives all three of these off
/// the GLB, so a re-import, a retarget or a change to `VIEWMODEL_HOLD` fails
/// the gate instead of hanging the axe beside the fist.
pub const VIEWMODEL_GRIP_M: Vec3 = Vec3::new(0.45838, -0.00628, -0.13165);
/// The item's orientation in the hand bone's frame. See [`VIEWMODEL_GRIP_M`].
pub const VIEWMODEL_GRIP_Q: Quat = Quat::from_xyzw(-0.163194, -0.429258, 0.687723, -0.562265);
/// What the item has to be scaled by to come out life-sized in the hand.
///
/// The rig's own root node carries `scale 0.01` — this skeleton's joint
/// translations are centimetres (`tests/rig_asset.rs` `root_transform` says
/// why) — and a child of a bone inherits it. Every offset under
/// [`HeldItem`] is in metres (`VIEWMODEL_PALM`, `hold::HeldModelDef::grip_m`,
/// the flame lift), and they all keep meaning metres because this puts the
/// item's own world scale back at 1. Same arithmetic, opposite direction,
/// as `bodies::hand_pose` dividing `BODY_PALM` by the body's scale.
pub const VIEWMODEL_GRIP_SCALE: f32 = 100.0;

/// The whole viewmodel assembly — the arms and, through the hand, the item.
///
/// **One node carries every motion, which is what the split it replaces could
/// not do.** Bob, sway and the swing used to be written onto two entities
/// separately: the item took the rotation and the arms took only the
/// translation, because *"the item rotates about its own origin, which is in
/// the hand; the arms rotate about the character's FEET, a metre and a half
/// away, so the same sway applied to both would swing the hands out of
/// frame"*. That reasoning was sound and it was solving the wrong problem —
/// with the item hung off the hand ([`VIEWMODEL_GRIP_M`]) the two are one
/// rigid body, and a rigid body has one pivot: [`VIEWMODEL_SWING_PIVOT`],
/// roughly where a shoulder is. So there is one transform again, and the arm
/// and the thing in it cannot disagree about where the swing went.
#[derive(Component)]
pub struct HeldRig;

/// The parent of the held item's meshes. One transform to animate, so the
/// handle and the head cannot drift apart.
///
/// Spawned under [`HeldRig`] and **re-parented onto the `RightHand` bone** by
/// [`dress_arms`]; [`InHand`] is the marker for which of the two it is, and
/// the fallback matters — a rig that never loads still draws an item at
/// [`VIEWMODEL_HOLD`], which is the picture this shipped for months.
#[derive(Component)]
pub struct HeldItem;

/// On [`HeldItem`] once it hangs off the hand bone rather than off
/// [`HeldRig`]. What [`animate`] composes its wrist snap onto depends on
/// which frame the item is in, and a marker says so in the type system
/// rather than by re-deriving it from the hierarchy every frame.
#[derive(Component)]
pub struct InHand;

/// The grip line: where a haft crosses the hand, in the **`RightHand` bone's
/// own frame**, centimetres (the bone's units).
///
/// Measured off `stumpy.glb`'s hand rather than off a view pose, which is
/// what the old seat could not be: [`VIEWMODEL_GRIP_Q`] is `hand⁻¹ ∘ view`
/// and carries no anatomy, so the haft crossed the knuckles about 90° off
/// the thumb and the frame showed an axe floating over an open palm. In this
/// bone frame the fingers run up +Y and curl toward +X (the palm side), the
/// thumb sits out on +X and toward −Z, and the line runs diagonally across
/// the palm from the heel to the thumb–index web: a 3.2 cm rod on it clears
/// every vertex of the hand and has hand on all sixteen sides.
/// `tests/viewmodel_arms.rs` re-measures both off the file.
pub const VIEWMODEL_SEAT: Vec3 = Vec3::new(4.1, 14.2, -2.4);
/// The grip line's direction through [`VIEWMODEL_SEAT`], butt to head, in the
/// same bone frame: 41° off the curled fingers' channel, toward the thumb.
pub const VIEWMODEL_SEAT_DIR: Vec3 = Vec3::new(0.185, 0.531, -0.827);
/// How fast the hand turns to a newly drawn row's grip, 1/s.
pub const VIEWMODEL_HAND_RATE: f32 = 12.0;

/// The hand's pose in the item's hold frame (`T(VIEWMODEL_HOLD)·R(VIEWMODEL_TILT)`,
/// metres): the bone's rotation, and where its origin — the wrist — is.
///
/// **The item never moves for this; the hand does.** Every row's pose in
/// view is the hold frame's (the angles and scales the operator tuned), so
/// fitting a hand to a row is placing the hand in that frame: the item hangs
/// off the bone by the inverse ([`item_pose`]) and the arm is turned and
/// shifted to put the bone there ([`hand_set`], [`pose_hand`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HandFit {
    pub rot: Quat,
    pub pos: Vec3,
}

/// The hold clip's own hand in the hold frame: [`VIEWMODEL_GRIP_M`] and
/// [`VIEWMODEL_GRIP_Q`] inverted.
pub fn hand_rest() -> HandFit {
    let rot = VIEWMODEL_GRIP_Q.inverse();
    HandFit {
        rot,
        pos: -(rot * VIEWMODEL_GRIP_M) / VIEWMODEL_GRIP_SCALE,
    }
}

/// The hand a row is held in: the rest hand turned so the row's haft (its
/// model +Y, [`item_rest_dir`]) runs along [`VIEWMODEL_SEAT_DIR`], rolled
/// about the haft by the row's `grip_roll`, and slid so [`VIEWMODEL_SEAT`]
/// lands on the palm point the model is posed around ([`VIEWMODEL_PALM`]).
/// A row with no `grip_roll`, and an empty hand, keep the rest hand.
///
/// The turn is worked in the rest hand's own frame — the haft there is
/// `VIEWMODEL_GRIP_Q · rest` — and starts from the smallest turn onto it,
/// so the wrist stays as near the clip's as the row allows.
pub fn hand_fit(def: Option<&HeldModelDef>) -> HandFit {
    let rest = hand_rest();
    let Some((d, roll)) = def.and_then(|d| d.grip_roll.map(|r| (d, r))) else {
        return rest;
    };
    let haft = (VIEWMODEL_GRIP_Q * item_rest_dir(d)).normalize_or(Vec3::Y);
    let turn = Quat::from_axis_angle(haft, roll)
        * turn_toward(VIEWMODEL_SEAT_DIR, haft, std::f32::consts::PI, 1.0);
    let rot = rest.rot * turn;
    HandFit {
        rot,
        pos: VIEWMODEL_PALM - rot * (VIEWMODEL_SEAT / VIEWMODEL_GRIP_SCALE),
    }
}

/// What the arm does to put the bone at `fit`, relative to the hold clip:
/// the hand bone's turn in its own frame, the part of it about the forearm's
/// long axis (radians, which [`pose_hand`] hands to the forearm so the wrist
/// only bends), and the shift of the whole arm in the [`HeldRig`] frame,
/// metres.
pub fn hand_set(fit: HandFit) -> (Quat, f32, Vec3) {
    let rest = hand_rest();
    let turn = rest.rot.inverse() * fit.rot;
    // The twist about +Y. The forearm and the hand share that axis on this
    // rig (the clip's hand is a 30° turn about it), so the forearm can carry
    // the twist and leave the wrist the swing.
    let mut twist = 2.0 * turn.y.atan2(turn.w);
    if twist > std::f32::consts::PI {
        twist -= std::f32::consts::TAU;
    } else if twist < -std::f32::consts::PI {
        twist += std::f32::consts::TAU;
    }
    (turn, twist, tilt() * (fit.pos - rest.pos))
}

/// The grip a row is held by: the item's transform in the `RightHand` bone's
/// frame for the hand [`hand_fit`] gives it. The first-person hand and every
/// remote body's hand (`bodies::hand_pose`) hang the item by this, so one
/// row is held one way.
pub fn grip(def: Option<&HeldModelDef>) -> Transform {
    item_pose(Some(hand_fit(def)), Quat::IDENTITY)
}

/// The item's transform in whichever frame it is hanging in — the hand
/// bone's once dressed (`fit` is the hand it hangs off), the rig's until
/// then.
///
/// `turn` turns the item about the palm in the hold frame: the stroke's
/// wrist snap and a bow's aim.
///
/// ⚠ **The snap turns the item inside the fist, not the fist.** Carrying it
/// on the hand instead was tried: a 1.45 rad turn about the palm swings the
/// wrist 17 cm, and shifting the arm after it walks the upper arm into the
/// lens at the strike; turning about the wrist leaves the item low and the
/// hand out of the bottom of the frame. The strike is 0.1 s; the rest pose
/// is what reads.
pub fn item_pose(fit: Option<HandFit>, turn: Quat) -> Transform {
    if let Some(fit) = fit {
        let rot = fit.rot.inverse();
        // About the palm, not the item frame's origin: turning about the
        // origin slides the haft through the fist. In the bone's own units —
        // a child of the hand inherits the rig's 0.01 scale (see
        // [`VIEWMODEL_GRIP_SCALE`]).
        Transform {
            translation: VIEWMODEL_GRIP_SCALE
                * (rot * (VIEWMODEL_PALM - turn * VIEWMODEL_PALM - fit.pos)),
            rotation: rot * turn,
            scale: Vec3::splat(VIEWMODEL_GRIP_SCALE),
        }
    } else {
        let rest = tilt();
        Transform {
            // The same correction, in the frame this branch lives in: here the
            // item hangs off the camera at scale 1, so the palm offset needs no
            // unit change.
            translation: VIEWMODEL_HOLD + rest * (VIEWMODEL_PALM - turn * VIEWMODEL_PALM),
            rotation: rest * turn,
            scale: Vec3::ONE,
        }
    }
}

/// The wrist turn that aims a raised bow, at raise `k` in 0..=1.
///
/// The bow is carried side-on, so its arrow (the model's −X, through the
/// row's pose) points off to the left. Raised, the turn lays the arrow along
/// the view axis for the rig's full-raise attitude, so the nocked arrow
/// points at the crosshair and the shot leaves where it looks like it will;
/// then it cants the bow about that arrow by [`VIEWMODEL_BOW_CANT`]. Both
/// scale with `k`, so the bow comes round as it comes up.
pub fn bow_aim(def: &crate::ui::hold::HeldModelDef, k: f32) -> Quat {
    let pose = Quat::from_rotation_y(def.pose_yaw) * Quat::from_rotation_x(-def.lay);
    let arrow = pose * Vec3::NEG_X;
    let raised = Quat::from_euler(
        EulerRot::YXZ,
        VIEWMODEL_DRAW_TURN.x,
        VIEWMODEL_DRAW_TURN.y,
        VIEWMODEL_DRAW_TURN.z,
    );
    let want = (raised * tilt()).inverse() * Vec3::NEG_Z;
    turn_toward(arrow, want, std::f32::consts::PI, k)
        * Quat::from_axis_angle(pose * Vec3::X, VIEWMODEL_BOW_CANT * k)
}

/// The turn that carries `from` toward `to`, capped at `cap` radians and
/// scaled by `strike` — the wrist the thrust and the bow share.
///
/// The axis is the cross product with a named fallback rather than
/// `Quat::from_rotation_arc`. That function is one line shorter and picks an
/// arbitrary perpendicular at the antipode; `render/tree.rs` records this repo
/// bending a whole tree sideways on exactly that branch. Here the failure would
/// be one item snapping to a random axis at a pose nobody photographs, and it
/// is reachable from `ui::hold`'s table with no code change at all — so the
/// margin is gated too.
fn turn_toward(from: Vec3, to: Vec3, cap: f32, strike: f32) -> Quat {
    let (axis, turn) = turn_between(from, to);
    Quat::from_axis_angle(axis, turn.min(cap) * strike)
}

/// The axis and the angle of the smallest turn from `from` to `to`. See
/// [`turn_toward`] for the fallback axis.
fn turn_between(from: Vec3, to: Vec3) -> (Vec3, f32) {
    let u = from.normalize_or(Vec3::NEG_Z);
    let v = to.normalize_or(Vec3::NEG_Z);
    let axis = u.cross(v).try_normalize().unwrap_or(Vec3::NEG_X);
    (axis, u.dot(v).clamp(-1.0, 1.0).acos())
}

/// The item's resting orientation in view space — [`VIEWMODEL_TILT`] as one
/// rotation. Every hold-frame direction in this file is expressed under it,
/// and `tests/viewmodel_arms.rs` re-derives the grip against it.
pub fn tilt() -> Quat {
    Quat::from_euler(
        EulerRot::YXZ,
        VIEWMODEL_TILT.x,
        VIEWMODEL_TILT.y,
        VIEWMODEL_TILT.z,
    )
}

/// Where the fist closes, in the rig's own frame at rest: the wrist at
/// [`VIEWMODEL_HOLD`] plus the palm correction turned into view space. The
/// point the wrist snap turns about, and the point a thrust measures from.
pub fn palm_rig() -> Vec3 {
    VIEWMODEL_HOLD + tilt() * VIEWMODEL_PALM
}

/// The thrust's wrist for a row, at strike progress `strike` ∈ 0..1: the turn
/// about the palm that puts the row's POINT on the view axis at the apex.
///
/// ## Solved, not tuned
///
/// At the apex the rig is at [`thrust_pose`] of [`swing_apex_s`], which fixes
/// where the palm is in view space; the row fixes how far past the palm its
/// point lies ([`HeldModelDef::ahead_m`]). A point that far from the palm
/// meets the view axis at exactly one depth in front of the eye —
/// `D = −palm.z + √(ahead² − palm.x² − palm.y²)` — and the direction from the
/// palm to `(0, 0, −D)` is the direction the item's long axis has to take.
/// Brought back into the hold frame through the apex rotation and the tilt,
/// that is a target for the same cross-product turn [`turn_toward`] makes, capped
/// by [`VIEWMODEL_THRUST_WRIST_MAX`] and scaled by `strike` on the way in and
/// out, so the point arrives on the crosshair exactly when the blow lands and
/// nowhere else.
///
/// So the number that matters — the depth the point lands at — is a
/// consequence of the carry and of the reach the pose buys, not a dial, and
/// `tests/viewmodel_arms.rs` holds it against the sim's own arm: the point at
/// the apex sits inside the body of a player standing at the row's content
/// reach (`combat::strike`'s `reach_cm`, ± `collide::CAPSULE_RADIUS_M`). That
/// is what *"lines up with where you aim"* means in this repo, and it is the
/// half a pixel gate could never have seen.
///
/// **What it could not fix, and what closed it the same day**: until melee
/// aim v1 (2026-09-05) the sim aimed in yaw alone, so a thrust drawn at the
/// ground still landed on a body in front of you and this paragraph said the
/// planar cone was a spoken v0 decision not this module's to overrule. The
/// operator overruled it — *"we need to step back and make this a PROPER
/// VIDEO GAME"* — and a swing is a ray along the look now (`melee::cast`,
/// yaw AND pitch), so the picture's pitch and the sim's agree by
/// construction: both read `look.pitch` through `pitch_u8`.
///
/// A row too short to reach the axis from where the palm sits — `ahead²`
/// under the palm's lateral offset squared — aims straight down −Z instead,
/// which is a defined answer and a red gate rather than a NaN in a transform.
pub fn thrust_snap(def: &HeldModelDef, strike: f32) -> Quat {
    let apex = swing_apex_s() / VIEWMODEL_SWING_S;
    let (rot, off) = thrust_pose(apex);
    let palm = rig_transform(rot, off).transform_point(palm_rig());
    let ahead = def.ahead_m();
    let lat2 = palm.x * palm.x + palm.y * palm.y;
    let want_view = if ahead * ahead > lat2 {
        let depth = -palm.z + (ahead * ahead - lat2).sqrt();
        (Vec3::new(0.0, 0.0, -depth) - palm) / ahead
    } else {
        Vec3::NEG_Z
    };
    let want = tilt().inverse() * (rot.inverse() * want_view);
    turn_toward(item_rest_dir(def), want, VIEWMODEL_THRUST_WRIST_MAX, strike)
}

/// The chop's wrist for a row (and the tap's, at [`VIEWMODEL_TAP`]): the turn
/// about the palm, cocked back by [`VIEWMODEL_SWING_BACK`] at the wind-up and
/// carried forward at the apex until the row's crown lies on the aim ray —
/// [`VIEWMODEL_SWING_DROP`] under the crosshair.
///
/// ## The strike axis is the row's, not the view's
///
/// The turn is the smallest one from where the row's haft rests
/// ([`item_rest_dir`]) to where it has to point at the apex, so its axis comes
/// out of the carry: a haft standing head-up tips its head forward and down, a
/// haft laid forward dips it. One fixed axis served one carry and swept every
/// other one sideways.
///
/// Solved the way [`thrust_snap`] is: at the apex the rig is at the stroke's
/// pose of [`swing_apex_s`] under the row's own carry and raise ([`carried`]),
/// which fixes the palm; a crown `ahead_m` past the palm meets the aim ray at
/// one depth, and the direction from the palm to there is the target. A row
/// too short to reach the ray points straight down −Z instead, a defined
/// answer and a red gate rather than a NaN.
pub fn chop_snap(def: &HeldModelDef, cock: f32, strike: f32) -> Quat {
    let k = if def.stroke == Stroke::Tap {
        VIEWMODEL_TAP
    } else {
        1.0
    };
    let apex = swing_apex_s() / VIEWMODEL_SWING_S;
    let (rot, off) = stroke_pose(def.stroke, apex);
    let lit = if def.light.is_some() { 1.0 } else { 0.0 };
    let rig = carried(carry_of(Some(def)), lit, rot, off);
    let palm = rig.transform_point(palm_rig());
    let reach = def.ahead_m() * rig.scale.x;
    let ray = Vec3::new(
        0.0,
        -VIEWMODEL_SWING_DROP.sin(),
        -VIEWMODEL_SWING_DROP.cos(),
    );
    let b = palm.dot(ray);
    let disc = b * b - (palm.length_squared() - reach * reach);
    let want_view = if disc > 0.0 && reach > 0.0 {
        (ray * (b + disc.sqrt()) - palm) / reach
    } else {
        Vec3::NEG_Z
    };
    let want = tilt().inverse() * (rig.rotation.inverse() * want_view);
    let (axis, turn) = turn_between(item_rest_dir(def), want);
    let turn = turn.min(VIEWMODEL_SWING_WRIST_MAX);
    let back = if def.stroke == Stroke::Swipe {
        VIEWMODEL_SWIPE_BACK
    } else {
        VIEWMODEL_SWING_BACK
    };
    Quat::from_axis_angle(axis, k * (turn * strike - back * cock))
}

/// The bash's wrist: the item's long axis tipped toward the view's −Z at the
/// apex, by [`VIEWMODEL_BASH_TURN`], and back by [`VIEWMODEL_BASH_BACK`] at
/// the wind-up — the stone's face leads the punch.
pub fn bash_snap(def: &HeldModelDef, cock: f32, strike: f32) -> Quat {
    let (axis, _) = turn_between(item_rest_dir(def), tilt().inverse() * Vec3::NEG_Z);
    Quat::from_axis_angle(
        axis,
        VIEWMODEL_BASH_TURN * strike - VIEWMODEL_BASH_BACK * cock,
    )
}

/// The wrist for whatever is in hand, at the stroke's `(cock, strike)`
/// ([`swing_phases`]). An empty fist has nothing to turn.
pub fn stroke_snap(def: Option<&HeldModelDef>, cock: f32, strike: f32) -> Quat {
    match def {
        Some(d) => match d.stroke {
            Stroke::Chop | Stroke::Tap | Stroke::Swipe => chop_snap(d, cock, strike),
            Stroke::Bash => bash_snap(d, cock, strike),
            Stroke::Thrust => thrust_snap(d, strike),
            Stroke::Shot => Quat::IDENTITY,
        },
        None => Quat::IDENTITY,
    }
}

/// An item's long axis at rest, in the hold frame — what the wrists turn from.
///
/// `pose` puts the model's +Y through `Ry(pose_yaw) * Rx(-lay)`, so this is
/// that rotation applied to +Y and nothing else. It is a function rather than
/// an expression at the call site because the gate and `ci/posesheet.py` both
/// need the same answer, and a second copy of a two-line rotation is how the
/// two stop agreeing.
pub fn item_rest_dir(def: &crate::ui::hold::HeldModelDef) -> Vec3 {
    Quat::from_rotation_y(def.pose_yaw) * Quat::from_rotation_x(-def.lay) * Vec3::Y
}

/// A 0 → 1 → 0 pulse over `u` ∈ [0, 1] that rises across `attack` of its span
/// and falls across the rest.
///
/// **C¹ at all three of its interesting points**, which is not decoration
/// here: the swing composes two of these, and a slope step is a visible flick
/// in a motion whose whole job is to read as one stroke. `sin²` and `cos²`
/// are flat at 0 and at π/2, so the rise leaves 0 with zero slope, meets the
/// fall at 1 with zero slope from both sides, and returns to 0 the same way.
pub fn bump(u: f32, attack: f32) -> f32 {
    if u <= 0.0 || u >= 1.0 {
        return 0.0;
    }
    let a = attack.clamp(1e-3, 1.0 - 1e-3);
    if u < a {
        let t = std::f32::consts::FRAC_PI_2 * (u / a);
        t.sin() * t.sin()
    } else {
        let t = std::f32::consts::FRAC_PI_2 * ((u - a) / (1.0 - a));
        t.cos() * t.cos()
    }
}

/// The arm about the shoulder for a stroke that turns: `cock`/`draw` at the
/// wind-up's peak and `strike`/`throw` at the apex (YXZ radians, metres), all
/// scaled by `k`.
fn arm_pose(s: f32, k: f32, cock: Vec3, draw: Vec3, strike: Vec3, throw: Vec3) -> (Quat, Vec3) {
    let (c, st) = swing_phases(s);
    let e = (cock * c + strike * st) * k;
    (
        Quat::from_euler(EulerRot::YXZ, e.x, e.y, e.z),
        (draw * c + throw * st) * k,
    )
}

/// The chop's rig rotation and displacement at swing progress `s` ∈ [0, 1],
/// where 0 and 1 are both the rest pose: up and back, then down and in, about
/// the shoulder. The head's own path is mostly the wrist's ([`chop_snap`]).
///
/// Published because the gate walks it: `tests/viewmodel_arms.rs` applies this
/// to the hold point and to the collapsed arm's origin at every step of the
/// stroke and asserts the first stays inside the frame and the second stays
/// outside it. The rig's own `Sword_Attack` carries the right hand behind the
/// camera for 40% of its length, because it is authored for a body seen from
/// outside. See [`VIEWMODEL_SWING_PIVOT`].
pub fn swing_pose(s: f32) -> (Quat, Vec3) {
    arm_pose(
        s,
        1.0,
        VIEWMODEL_SWING_COCK,
        VIEWMODEL_SWING_DRAW,
        VIEWMODEL_SWING_STRIKE,
        VIEWMODEL_SWING_THROW,
    )
}

/// The tap: [`swing_pose`] at [`VIEWMODEL_TAP`].
pub fn tap_pose(s: f32) -> (Quat, Vec3) {
    arm_pose(
        s,
        VIEWMODEL_TAP,
        VIEWMODEL_SWING_COCK,
        VIEWMODEL_SWING_DRAW,
        VIEWMODEL_SWING_STRIKE,
        VIEWMODEL_SWING_THROW,
    )
}

/// The swipe: a short draw ([`VIEWMODEL_SWIPE_COCK`]) and a strike across
/// the body ([`VIEWMODEL_SWIPE_STRIKE`]).
pub fn swipe_pose(s: f32) -> (Quat, Vec3) {
    arm_pose(
        s,
        1.0,
        VIEWMODEL_SWIPE_COCK,
        VIEWMODEL_SWIPE_DRAW,
        VIEWMODEL_SWIPE_STRIKE,
        VIEWMODEL_SWING_THROW,
    )
}

/// The bash: the fist drawn back and out, then punched in toward the
/// crosshair and away from the eye.
pub fn bash_pose(s: f32) -> (Quat, Vec3) {
    arm_pose(
        s,
        1.0,
        VIEWMODEL_BASH_COCK,
        VIEWMODEL_BASH_DRAW,
        VIEWMODEL_BASH_STRIKE,
        VIEWMODEL_BASH_THROW,
    )
}

/// The two pulses of a stroke at progress `s` ∈ [0, 1]: `(cock, strike)`.
/// One function for every stroke and for the wrist, so the arm and the item
/// cannot disagree about when the blow lands — which is also what keeps
/// [`swing_apex_s`] true of all of them.
///
/// The cock peaks halfway through [`VIEWMODEL_SWING_WINDUP`] and falls INTO
/// the apex, overlapping the strike, so the arm comes down from the wind-up in
/// one motion. It used to fall back to rest before the strike began, and a
/// chop that comes down twice with a stop at rest between reads as a stutter.
/// Both are zero, with zero slope, at the apex, so the apex pose is the
/// strike's alone.
pub fn swing_phases(s: f32) -> (f32, f32) {
    let w = VIEWMODEL_SWING_WINDUP;
    let apex = swing_apex_s() / VIEWMODEL_SWING_S;
    let cock = bump(s / apex, 0.5 * w / apex);
    let strike = if s < w {
        0.0
    } else {
        bump((s - w) / (1.0 - w), VIEWMODEL_SWING_ATTACK)
    };
    (cock, strike)
}

/// The rig's rotation and displacement at thrust progress `s` ∈ [0, 1] —
/// [`swing_pose`]'s twin for a [`Stroke::Thrust`] row, on the same clock.
///
/// Same phases, same [`VIEWMODEL_SWING_S`], so the apex is the chop's apex.
/// What differs is the path: this draws the whole assembly back toward the eye
/// and then drives it forward, inward and slightly up. The point's own
/// convergence on the crosshair is not here — it is the wrist,
/// [`thrust_snap`] — because it depends on the row's length and this does
/// not. Published for the same reason [`swing_pose`] is: the gate walks it.
pub fn thrust_pose(s: f32) -> (Quat, Vec3) {
    let (wind, strike) = swing_phases(s);
    let rot = Quat::from_euler(
        EulerRot::YXZ,
        -VIEWMODEL_THRUST_COCK * wind + VIEWMODEL_THRUST_TURN * strike,
        VIEWMODEL_THRUST_LIFT * wind + VIEWMODEL_THRUST_RISE * strike,
        0.0,
    );
    (
        rot,
        VIEWMODEL_THRUST_DRAW * wind + VIEWMODEL_THRUST_PUSH * strike,
    )
}

/// The arm's stroke for the row — or nothing, for a row that is fired rather
/// than swung.
pub fn stroke_pose(stroke: Stroke, s: f32) -> (Quat, Vec3) {
    match stroke {
        Stroke::Chop => swing_pose(s),
        Stroke::Tap => tap_pose(s),
        Stroke::Swipe => swipe_pose(s),
        Stroke::Bash => bash_pose(s),
        Stroke::Thrust => thrust_pose(s),
        Stroke::Shot => (Quat::IDENTITY, Vec3::ZERO),
    }
}

/// When in the stroke the blow lands, seconds from its start.
///
/// The strike's own bump peaks at [`VIEWMODEL_SWING_ATTACK`] of the span
/// left after the wind-up, so this is arithmetic over three constants rather
/// than a fourth number — and it is published because a gate needs it:
/// `tests/rig_asset.rs` holds this within half a tick of where the rig's own
/// `Sword_Attack` strikes, which is what keeps the swinger's view and
/// everybody else's view of one swing pointed at the same instant.
pub fn swing_apex_s() -> f32 {
    (VIEWMODEL_SWING_WINDUP + (1.0 - VIEWMODEL_SWING_WINDUP) * VIEWMODEL_SWING_ATTACK)
        * VIEWMODEL_SWING_S
}

/// The rig's transform for a rotation about [`VIEWMODEL_SWING_PIVOT`] plus a
/// displacement — the arithmetic that turns "turn about the shoulder" into
/// the one `Transform` Bevy wants. This is the bare derivation every constant
/// in this file was measured in; what is drawn lays [`carried`] over it.
pub fn rig_transform(rot: Quat, off: Vec3) -> Transform {
    carried(0.0, 0.0, rot, off)
}

/// How much bigger the arm and what it holds are drawn than the hold they were
/// derived at, scaled about the palm.
///
/// **A presentation over the derivation, not a change to it.** Every constant
/// above — [`VIEWMODEL_HOLD`], the grip, the hidden arm, the swing's pivot —
/// still means what it measured; this is laid over the whole assembly last,
/// the way the reference game frames its viewmodel bigger and further out than
/// the arm would really be (operator, 2026-10-03: *"my hand is a lot bigger…
/// more off to the side"*).
///
/// The stroke is divided by it ([`carried`]), so a bigger arm makes the same
/// sweep across the frame rather than a bigger one: undivided, 1.25 already
/// carried a chop's wind-up off the top of the frame.
pub const VIEWMODEL_CARRY_SCALE: f32 = 1.35;
/// How far the carried assembly is moved, view space metres: out to the
/// right and a little down, which puts the palm at ndc (0.44, −0.55) where it
/// sat at (0.32, −0.49).
pub const VIEWMODEL_CARRY_SHIFT: Vec3 = Vec3::new(0.10, -0.02, 0.0);

/// The raise a lit item is carried at — the whole arm turned about the
/// shoulder ([`VIEWMODEL_SWING_PIVOT`]), YXZ radians: out to the right. The
/// reference game holds a torch up beside the head, flame in the upper right
/// where it lights the way without sitting in it — and all of it on screen:
/// with 0.15 of pitch on top, the flame burned off the top of the frame
/// once it sat on the head (operator, 2026-10-07).
pub const VIEWMODEL_LIFT: Vec3 = Vec3::new(-0.10, 0.0, 0.0);
/// How fast the arm comes up to, or down from, the raise and the carry, 1/s.
pub const VIEWMODEL_LIFT_RATE: f32 = 8.0;
/// Where the arm goes while a deployable is held as a blueprint
/// (`sheet`): down out of the frame, view space metres. The sheet runs off
/// the bottom of the frame, so the hands holding it are down there too.
pub const VIEWMODEL_STOW: Vec3 = Vec3::new(0.06, -0.42, 0.0);
/// How much of the carry a row takes. A thrust takes none: its point is
/// solved onto the crosshair at the depth of the sim's reach
/// ([`thrust_snap`]), and a spear drawn a third longer would visibly run
/// through somebody `combat::strike` cannot reach.
pub fn carry_of(def: Option<&HeldModelDef>) -> f32 {
    match def.map(|d| d.stroke) {
        Some(Stroke::Thrust) => 0.0,
        _ => 1.0,
    }
}

/// The assembly's transform as drawn: the stroke `rot`/`off` about the
/// shoulder, the raise (`lift` 0..1 of [`VIEWMODEL_LIFT`]) about the same
/// shoulder, and the carry (`carry` 0..1 of [`VIEWMODEL_CARRY_SCALE`] and
/// [`VIEWMODEL_CARRY_SHIFT`]) over all of it.
///
/// `carry` is a fraction because two things let go of it: a thrust row
/// ([`carry_of`]) and a drawn bow, whose draw pose brings the arrow to the
/// middle of the frame and would be pushed off the crosshair by it — so
/// `animate` hands it `carry_of(row) · (1 − raise)`.
pub fn carried(carry: f32, lift: f32, rot: Quat, off: Vec3) -> Transform {
    let s = 1.0 + (VIEWMODEL_CARRY_SCALE - 1.0) * carry;
    // The same sweep on screen, not a bigger one.
    let rot = Quat::IDENTITY.slerp(rot, 1.0 / s);
    let off = off / s;
    let up = Quat::from_euler(
        EulerRot::YXZ,
        VIEWMODEL_LIFT.x * lift,
        VIEWMODEL_LIFT.y * lift,
        VIEWMODEL_LIFT.z * lift,
    );
    let rot = up * rot;
    let arm = Transform {
        translation: VIEWMODEL_SWING_PIVOT - rot * VIEWMODEL_SWING_PIVOT + up * off,
        rotation: rot,
        scale: Vec3::ONE,
    };
    let frame = Transform {
        translation: palm_rig() * (1.0 - s) + VIEWMODEL_CARRY_SHIFT * carry,
        rotation: Quat::IDENTITY,
        scale: Vec3::splat(s),
    };
    frame * arm
}

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

/// The fire on a lit torch in your own hand: a small pool of flame tongues
/// simulated in [`HandLight`]'s own frame and drawn as one mesh under it, so
/// it stays on the torch head through every bob, sway and swing
/// ([`hand_fire`]). It replaced two additive spheres that read as one peach
/// blob (operator, 2026-10-07: *"the fire effect kinda sucks"*).
///
/// **In the hand's frame, not the world's.** The world pool's fire is left
/// behind by the body carrying it: a tongue lives a third of a second, a
/// walking body covers a metre and more in that, and a turn of the head
/// sweeps the torch half a metre — so world-space tongues on a torch half a
/// metre from the eye trail off the head the moment anything moves. These
/// ride the torch and take back only [`HAND_FIRE_TRAIL`] of its motion
/// through the air, which bends the flame away from a swing or a run
/// without tearing it off. The tongues are `fx::world::torch_flame`'s, the
/// emitter another player's torch burns with in the world. The embers stay
/// world-space (`fx::world::fires`): a spark left behind is what a spark
/// does.
#[derive(Component)]
pub struct HandFire {
    pool: super::fx::pool::Pool,
    /// Where the emitter was last frame, world space.
    last: Option<Vec3>,
}

/// The first-person torch's world embers, as a fraction of a fire pit's —
/// a size under the remote torch's `fx::world::TORCH_FIRE_SCALE`, because
/// this one burns half a metre from the eye rather than across a clearing.
pub const HAND_FIRE_SCALE: f32 = 0.22;

/// How much of the torch head's motion through the air the flame takes back
/// — 0 is welded to the head, 1 is left where it was.
pub const HAND_FIRE_TRAIL: f32 = 0.45;
/// A jump of the head bigger than this in a frame is a teleport (a respawn,
/// a reconnect), not a motion, metres.
pub const HAND_FIRE_JUMP_M: f32 = 1.0;
/// Particles the hand's fire may hold.
pub const HAND_FIRE_POOL: usize = 96;

/// A flame's brightness over time, about 1: three waves that never line up,
/// so the light the torch throws breathes rather than pulses.
pub fn flame_flicker(t: f32) -> f32 {
    1.0 + 0.07 * (t * 11.0).sin() + 0.05 * (t * 23.0 + 1.3).sin() + 0.03 * (t * 37.0 + 0.4).sin()
}

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
    /// The skin the shown model is drawn in (skins v0), 0 for its own look.
    skin: u16,
    /// The shown model is the first-person copy (`bow::FpBow`, the bow
    /// without its baked string) rather than the row's own mesh.
    fp: bool,
}

impl HeldModel {
    /// The [`crate::ui::hold::HELD_MODELS`] row on screen, if any.
    pub fn shown(&self) -> Option<usize> {
        self.shown
    }
}

/// Tinted copies of the held-model materials (skins v0), one per
/// (`HELD_MODELS` row, skin catalog id), made on first use and kept: a skin
/// is a colour over the item's own surface, so the copy is the row's material
/// with its base colour multiplied. Shared by the first-person hand and
/// every body's hand (`bodies::update_hand`), so one skin on screen twice is
/// one material.
#[derive(Resource, Default)]
pub struct SkinMats(std::collections::HashMap<(usize, u16), Handle<StandardMaterial>>);

impl SkinMats {
    /// Row `row`'s material in skin `skin`, tinted by `tint` (sRGB factors,
    /// `ui::skins::tint_of`). `None` while the row's own material has not
    /// loaded yet (a glTF row), so the caller draws the plain look this
    /// frame and asks again next frame.
    pub fn for_skin(
        &mut self,
        row: usize,
        skin: u16,
        tint: [f32; 3],
        base: &Handle<StandardMaterial>,
        materials: &mut Assets<StandardMaterial>,
    ) -> Option<Handle<StandardMaterial>> {
        if let Some(h) = self.0.get(&(row, skin)) {
            return Some(h.clone());
        }
        let mut m = materials.get(base)?.clone();
        let c = m.base_color.to_linear();
        let t = Color::srgb(tint[0], tint[1], tint[2]).to_linear();
        m.base_color =
            Color::linear_rgba(c.red * t.red, c.green * t.green, c.blue * t.blue, c.alpha);
        let h = materials.add(m);
        self.0.insert((row, skin), h.clone());
        Some(h)
    }
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
    /// The heave: the arm's vertical offset, metres, its speed, and the
    /// body's vertical speed last frame.
    heave: f32,
    heave_v: f32,
    last_vy: f32,
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
    /// Strokes started since the session began — every one of them is one
    /// `Cue::Swing`, so this is the count a test of the sound's cadence
    /// would read. Wraps rather than saturates; it is a counter, not a sum.
    pub strokes: u32,
    /// The bow's draw, mirrored (`ui::draw`).
    draw: crate::ui::draw::DrawClock,
    /// How far the bow is raised, 0..=1, eased toward the aim.
    raise: f32,
    /// How far the arm is raised for a lit item, 0..=1 ([`VIEWMODEL_LIFT`]).
    lift: f32,
    /// How much of the carry the row in hand takes, eased ([`carry_of`]).
    carry: f32,
    /// The loose's kick, counting down from 1 to 0.
    loose: f32,
    /// Loosed shots since the session began, the draw's `strokes`.
    pub looses: u32,
    /// How far the string is drawn, 0..=1: the draw clock on the way back,
    /// zero the instant an arrow leaves, and eased home when the aim is let
    /// go without a shot (`bow::drive` draws it).
    string: f32,
    /// An arrow sits on the string: raised, arrows in the pack, and not
    /// mid-loose.
    nocked: bool,
    /// Where the drawing hand is along the draw, 0..=1: with the string on
    /// the way back, left where it was when the arrow goes, then eased back
    /// onto the string (`bow::draw_arm`).
    hand: f32,
    /// A stroke asked for by something other than the swing button (the
    /// hammer's repair): taken next frame if the arm is at rest, dropped if
    /// it is mid-stroke, so clicking faster does not chain strokes.
    queued: bool,
    /// How far the arm is lowered out of frame for the blueprint sheet,
    /// 0..=1 ([`VIEWMODEL_STOW`]), eased.
    stow: f32,
    /// This frame's sway and bob, for the sheet to ride ([`Motion::idle`]).
    idle_lag: Quat,
    idle_drift: Vec3,
    /// The hand the row in it is held in ([`hand_fit`]), eased toward a new
    /// row's; `None` until the first frame, which takes it outright.
    fit: Option<HandFit>,
    /// This frame's [`hand_set`] — the hand's turn, the forearm's share of
    /// it and the arm's shift — for [`pose_hand`] to write after the clip.
    set: Option<(Quat, f32, Vec3)>,
    /// How closed the fist is on the row in it, 0..=1, eased with `fit`
    /// (`fingers::pose`).
    grip: f32,
}

/// What the bow's string and arrow are drawn from this frame (`bow::drive`).
#[derive(Clone, Copy, Debug, Default)]
pub struct BowPose {
    /// The string's draw, 0..=1.
    pub pull: f32,
    /// The loose, counting down from 1 to 0.
    pub loose: f32,
    /// An arrow is on the string.
    pub nocked: bool,
    /// How far the bow is raised, 0..=1.
    pub raise: f32,
    /// The drawing hand's place along the draw, 0..=1.
    pub hand: f32,
}

impl Motion {
    /// How closed the first-person fist is, 0 at the rest curl and 1 on a
    /// handle (`fingers::pose`).
    pub fn grip(&self) -> f32 {
        self.grip
    }

    /// Swing the arm once, for a verb that is a swing without being the
    /// swing button: the hammer's repair (`verbs::keys`).
    pub fn strike(&mut self) {
        self.queued = true;
    }

    /// The sway and the bob (with the heave) this frame, without the
    /// stroke: what `sheet::drive` carries the blueprint by.
    pub fn idle(&self) -> (Quat, Vec3) {
        (self.idle_lag, self.idle_drift)
    }

    pub fn bow(&self) -> BowPose {
        BowPose {
            pull: self.string,
            loose: self.loose,
            nocked: self.nocked,
            raise: self.raise,
            hand: self.hand,
        }
    }
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
    fx: Res<super::fx::Fx>,
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
        // The assembly node. Everything the viewmodel does to itself — bob,
        // sway, the swing arc — is written here, and the arms spawn under it
        // (`spawn_arms`) so the arm and the item cannot be given different
        // motions. See [`HeldRig`].
        c.spawn((HeldRig, Transform::IDENTITY, Visibility::Inherited))
            .with_children(|rig| {
                rig.spawn((
                    HeldItem,
                    item_pose(None, Quat::IDENTITY),
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
                        HeldModel {
                            shown: None,
                            skin: 0,
                            fp: false,
                        },
                        Mesh3d(Handle::default()),
                        MeshMaterial3d::<StandardMaterial>(Handle::default()),
                        Transform::from_rotation(Quat::from_rotation_x(MODEL_UPRIGHT_TO_HELD)),
                        Visibility::Hidden,
                    ))
                    // The bow's live string and the arrow on it, in the
                    // model's own frame (`bow::drive` shows them).
                    .with_children(|model| {
                        super::bow::spawn_parts(model, &mut meshes, &mut materials)
                    });
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
                        // The embers and the smoke, which the world pool
                        // throws while `hand_light` has the emitter lit; the
                        // tongues are `HandFire`'s, drawn in this frame.
                        // The smoke leaves well above the head so it does
                        // not hang in front of your own eyes.
                        super::fx::world::FireFx {
                            flames: true,
                            flame_dy: 0.0,
                            smoke_dy: 0.6,
                            scale: HAND_FIRE_SCALE,
                            tongues: super::fx::world::Tongues::Owned,
                        },
                        Transform::IDENTITY,
                    ))
                    .with_children(|light| {
                        // The flame you can SEE: one mesh of tongues in the
                        // emitter's frame, so it sits on the torch head
                        // through every bob and swing. Empty, and so drawn
                        // as nothing, while the torch is out.
                        light.spawn((
                            HandFire {
                                pool: super::fx::pool::Pool::new(HAND_FIRE_POOL, false),
                                last: None,
                            },
                            Mesh3d(meshes.add(super::fx::pool::pool_mesh(HAND_FIRE_POOL))),
                            MeshMaterial3d(fx.glow_mat.clone()),
                            Transform::IDENTITY,
                            Visibility::Inherited,
                            // The pool's bounds are its first, empty write.
                            NoFrustumCulling,
                            bevy::light::NotShadowCaster,
                        ));
                    });
                });
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
    /// The scene's `AnimationPlayer`. Recorded rather than re-walked, for
    /// `bodies::Live::hand`'s reason: the walk is a bounded descendant search
    /// and doing it per frame to find one entity that cannot move is a cost
    /// with nothing bought.
    player: Option<Entity>,
    /// The `RightForeArm` bone, which carries the twist of a row's grip
    /// ([`pose_hand`]).
    forearm: Option<Entity>,
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

/// Where the collapsed joint is MOVED to, in its own parent's local units.
///
/// ## Scaling it to nothing was never enough, and the swing is what proved it
///
/// A collapsed joint is a heap of zero-area triangles at that joint's own
/// origin, and this rig's `LeftShoulder` origin sits at ndc (0.69, −1.87) —
/// just under the bottom of the frame, and **0.217 m from the lens**. At that
/// depth the frame is 33 cm tall, so *any* motion of the viewmodel is a
/// large motion in ndc: the swing's wind-up carried it to ndc y −0.97, which
/// is a stray speck of skin in shot, and even a 20 cm translation would have
/// moved it 1.2 ndc. The old arc survived only because it was tiny — which is
/// the defect it was reported for.
///
/// So the joint is moved as well as collapsed, to
/// [`VIEWMODEL_HIDDEN_BEHIND_M`] **behind the camera**, where nothing the rig
/// can do to itself brings it back: a point 4.8 m behind a pivot 0.3 m behind
/// the eye is still 4.5 m behind it after any rotation this file writes.
///
/// **Measured, not typed** — it is `R_Spine⁻¹ · (0, 0, VIEWMODEL_HIDDEN_BEHIND_M)
/// / scale` at the hold clip's rest pose, in the parent's own frame, so the
/// numbers are large and arbitrary-looking for the same reason
/// [`VIEWMODEL_GRIP_M`]'s are: a bone's local axes are wherever the skeleton
/// put them. `tests/viewmodel_arms.rs` re-derives it off the shipped file and
/// walks it through the whole hold loop and the whole swing.
///
/// ⚠ **Safe only because the hold clip animates no translation on this
/// bone** — the same condition the scale collapse already rests on, one
/// channel over, and gated in the same test.
pub const VIEWMODEL_HIDDEN_OFFSET: Vec3 = Vec3::new(-301.23, -75.10, -387.69);

/// How far behind the eye [`VIEWMODEL_HIDDEN_OFFSET`] parks the collapsed
/// joint, metres. Published because it is what the offset MEANS, and because
/// the gate checks the consequence rather than the vector.
pub const VIEWMODEL_HIDDEN_BEHIND_M: f32 = 4.78;

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
    // **Under [`HeldRig`] and not under the camera.** The arms and the item
    // are one rigid assembly now (the item hangs off the hand), so they take
    // one motion from one node; hanging the arms off the camera again would
    // put the swing back on two entities that have to be kept in step by
    // hand, which is the bug this replaced.
    host: Query<Entity, With<HeldRig>>,
) {
    if *done || !rig.ready() {
        return;
    }
    // `spawn_item` spawns the host and both are `Update` systems, so the
    // first frame here can genuinely find nothing — bail without latching,
    // exactly as a missing `Rig` does.
    let (Ok(cam), Some(scene)) = (host.single(), rig.scene.clone()) else {
        return;
    };
    *done = true;
    commands.entity(cam).with_children(|c| {
        c.spawn((
            ViewArms {
                hand: None,
                player: None,
                forearm: None,
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
    mut draw_arm: ResMut<super::bow::DrawArm>,
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
    // The folded arm's joints below the shoulder, for the bow's draw hand.
    let (mut upper, mut fore, mut left_hand) = (None, None, None);
    let mut forearm = None;
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
                "RightForeArm" => forearm = Some(e),
                VIEWMODEL_HIDDEN_ARM => off_arm = Some(e),
                "LeftArm" => upper = Some(e),
                "LeftForeArm" => fore = Some(e),
                "LeftHand" => left_hand = Some(e),
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
    // the measurement and for why this is a scale and not a `Visibility`, and
    // [`VIEWMODEL_HIDDEN_OFFSET`] for why the scale alone does not do it.
    // Neither channel is one `Pistol_Idle_Loop` writes, so this survives every
    // frame the clip plays without a system to re-apply it.
    if let Ok(mut t) = xf.get_mut(off_arm) {
        t.scale = Vec3::splat(VIEWMODEL_HIDDEN_SCALE);
        t.translation = VIEWMODEL_HIDDEN_OFFSET;
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

    // **The item moves into the hand here**, which is the whole of what
    // 2026-08-17 deferred as *"a grip is judged by looking rather than
    // derived"*. That is still true of a grip in general and it is not true
    // of this one: [`VIEWMODEL_GRIP_M`] is whatever leaves the item exactly
    // where the camera was already hanging it, so nothing about the resting
    // frame is being judged — only whether it FOLLOWS, and it now does.
    //
    // `insert` writes the local transform in the same command as the
    // re-parent, so the item is never one frame at the hand's origin wearing
    // the camera's offsets — which at `scale 0.01` would be a hundred-fold
    // item somewhere under the floor for a frame.
    //
    // **A guard and not an `if let`, because the dressing is atomic.**
    // `body_half`/`hand`/`player`/`off_arm` above are all required for that
    // reason; the item is the fifth. `spawn_item` runs first in practice —
    // it waits on `PropMaps`, where this waits on a glTF — but "in practice"
    // is how a latch comes to be set on a frame that did half the work.
    let Ok(item) = item.single() else {
        return;
    };
    commands.entity(item).insert((
        ChildOf(hand),
        InHand,
        item_pose(Some(hand_rest()), Quat::IDENTITY),
    ));

    arms.hand = Some(hand);
    arms.player = Some(player);
    arms.forearm = forearm;
    arms.dressed = true;
    // Optional where the rest is required: without them a raised bow is
    // simply drawn by no hand, which is how it shipped.
    draw_arm.bones = upper
        .zip(fore)
        .zip(left_hand)
        .map(|((u, f), h)| [off_arm, u, f, h]);
    info!("viewmodel: arms up, body half hidden, {VIEWMODEL_HIDDEN_ARM} collapsed, item in hand");
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
/// Where a held row sits relative to the fist that holds it, in metres.
///
/// The whole pose is a property of the ITEM, so all of it is here and none
/// of it at spawn: lay a tool forward, keep a carried thing upright, then
/// slide the model so its grip point — `grip_m` up its own +Y — lands on
/// `palm`. The grip vector is rotated WITH the model; writing it on a fixed
/// axis is the bug this replaces, which hung every tool `grip_m` below the
/// hand (63 cm, for the spear).
///
/// **`palm` is a parameter because there are two fists now.** The
/// viewmodel's is [`VIEWMODEL_PALM`], an offset in eye space; a remote
/// body's is `bodies::BODY_PALM`, an offset in that body's own space. The
/// arithmetic between them is identical and a second copy of it is the
/// mirror-drift `CLAUDE.md` warns about on every hand-kept duplicate — so
/// there is one, and the caller says where the hand is.
pub fn pose(def: &crate::ui::hold::HeldModelDef, palm: Vec3) -> Transform {
    let lay = Quat::from_rotation_x(-def.lay);
    // The presentation yaw composes in the hand's frame, so it turns the
    // item about the fist rather than about its own foot, and the grip
    // point below stays in the palm under any yaw.
    let rot = Quat::from_rotation_y(def.pose_yaw) * lay;
    Transform {
        translation: palm - (rot * (Vec3::Y * def.grip_m())),
        rotation: rot,
        scale: Vec3::splat(def.scale),
    }
}

/// Where a lit row's flame burns, in the item's hold frame (the frame
/// [`HeldItem`] and a remote body's grip hang things in): [`FLAME_LIFT_M`]
/// over the crown of the model as [`pose`] places it.
///
/// Read off the same pose the mesh is drawn with, palm offset and all. The
/// light used to hang at `flame_m` straight up from the frame's origin —
/// the WRIST — while the mesh hangs off [`VIEWMODEL_PALM`], so the fire
/// burned 13 cm beside the torch head (operator, 2026-10-07: *"its not near
/// the actual torch"*).
///
/// [`FLAME_LIFT_M`]: crate::ui::hold::FLAME_LIFT_M
pub fn flame_at(def: &HeldModelDef) -> Vec3 {
    pose(def, VIEWMODEL_PALM).transform_point(Vec3::Y * def.height_m)
        + Vec3::Y * crate::ui::hold::FLAME_LIFT_M
}

#[allow(clippy::type_complexity)]
pub fn swap(
    net: Option<NonSend<Net>>,
    models: Res<Models>,
    fp_bow: Res<super::bow::FpBow>,
    mut skin_mats: ResMut<SkinMats>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut q: Query<(
        &mut HeldModel,
        &mut Mesh3d,
        &mut MeshMaterial3d<StandardMaterial>,
        &mut Visibility,
        &mut Transform,
    )>,
    mut fallback: Query<&mut Visibility, (With<Fallback>, Without<HeldModel>)>,
) {
    let (want, empty, skin, tint, tool) = match net.as_deref() {
        Some(n) => {
            let core = &n.session.core;
            let stack = core
                .inv
                .get(usize::from(n.sel).min(core.inv.len() - 1))
                .copied();
            let skin = stack.map_or(0, |s| if s.count == 0 { 0 } else { s.skin });
            let click = crate::ui::hold::click_in_hand(
                &core.catalog,
                &core.research,
                &core.deploy_defs,
                core.deploy_defs_have,
                &core.inv,
                n.sel,
            );
            // Food and paper are not tools: a left click eats or reads them
            // (`ui::hold::Click`), and the hafted stand-in in the fist said
            // the opposite — a mushroom drawn as an axe.
            let tool = !matches!(
                click,
                crate::ui::hold::Click::Eat | crate::ui::hold::Click::Read
            );
            // **Holstered in THE GATE** (Rust: no weapon can be drawn in a
            // safe zone): the hands are empty until you step out. A
            // deployable is held as a blueprint (`sheet`), not in the fist.
            if core.holstered() || click == crate::ui::hold::Click::Deploy {
                (None, true, 0, None, tool)
            } else {
                (
                    crate::ui::hold::held_model_in_hand(&core.catalog, &core.inv, n.sel),
                    stack.is_none_or(|s| s.count == 0),
                    skin,
                    crate::ui::skins::tint_of(&core.skins, skin),
                    tool,
                )
            }
        }
        None => (None, true, 0, None, true),
    };

    // The bow in your own hand is drawn without its baked string once that
    // copy exists; `bow::drive` draws a live one on it.
    let fp = fp_bow
        .mesh
        .as_ref()
        .filter(|_| want.is_some() && want == super::bow::bow_row());
    for (mut held, mut mesh, mut mat, mut vis, mut tf) in &mut q {
        if held.shown != want || held.skin != skin || held.fp != fp.is_some() {
            held.fp = fp.is_some();
            match want {
                Some(i) => {
                    let (m, mt) = models.row(i);
                    mesh.0 = fp.cloned().unwrap_or(m);
                    *tf = pose(&crate::ui::hold::HELD_MODELS[i], VIEWMODEL_PALM);
                    // The skin's tinted copy when it is ready; the plain look
                    // (and a retry next frame, by not recording the skin)
                    // while the row's own material is still loading.
                    let skinned =
                        tint.and_then(|t| skin_mats.for_skin(i, skin, t, &mt, &mut materials));
                    held.skin = if skinned.is_some() || tint.is_none() {
                        skin
                    } else {
                        u16::MAX
                    };
                    mat.0 = skinned.unwrap_or(mt);
                }
                None => {
                    mesh.0 = Handle::default();
                    mat.0 = Handle::default();
                    held.skin = skin;
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
    // The stand-in covers "a tool with no model of its own", never "nothing
    // in hand" and never a meal.
    let show_fallback = want.is_none() && !empty && tool;
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
/// entire lighting of a tenth of every cycle, while `brain::sense` sends the
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
    time: Res<Time>,
    gain: Res<super::rig::FlameGain>,
    q: Query<(&mut PointLight, &mut Transform), With<HandLight>>,
) {
    let want = net.as_deref().and_then(lit_in_hand);
    let fuel = net.as_deref().map_or(1.0, fuel_in_hand);
    let t = time.elapsed_secs();
    // The night eye's gain (`rig::flame_gain`), the flame's own breath, and
    // the gutter of the last of the fuel.
    apply_hand_light(want, gain.0 * flame_flicker(t) * torch_sputter(t, fuel), q);
}

/// Below this share of its fuel a lit torch gutters: its light dips, deeper
/// and more often as the last of it goes, which is the warning the player
/// gets before [`torch_watch`] says it is out.
pub const TORCH_SPUTTER_FRAC: f32 = 0.10;

/// The light's gain on top of [`flame_flicker`] with `fuel` (0..1) of the
/// torch left: exactly 1 above [`TORCH_SPUTTER_FRAC`], then dips that grow
/// toward empty. Never below 0.15, so a guttering torch still lights.
pub fn torch_sputter(t: f32, fuel: f32) -> f32 {
    let k = (1.0 - fuel / TORCH_SPUTTER_FRAC).clamp(0.0, 1.0);
    if k == 0.0 {
        return 1.0;
    }
    let spike = ((t * 2.3).sin() * (t * 5.9 + 1.1).sin()).max(0.0);
    1.0 - k * (0.15 + 0.7 * spike)
}

/// The share of its fuel the item in your hand has left, 1.0 for anything
/// that does not burn (`cond_max` 0) or an empty hand.
fn fuel_in_hand(n: &Net) -> f32 {
    let core = &n.session.core;
    let Some(stack) = core.inv.get(n.sel as usize) else {
        return 1.0;
    };
    let max = core.catalog.cond_max(stack.item as usize);
    if max == 0 {
        return 1.0;
    }
    stack.cond as f32 / max as f32
}

/// Say so when the torch in your hand burns out: a hiss (`Cue::TorchOut`)
/// and a line. Until this the flame just went dark a round trip after the
/// fuel ran out, with nothing to tell a burn-out from a click.
///
/// The edge is "lit last frame, not lit now" with the latch still up, the
/// same slot selected and the same item in it at zero condition — so
/// putting it out by hand, switching slots, going down or dropping it say
/// nothing.
pub fn torch_watch(
    net: Option<NonSend<Net>>,
    mut toast: ResMut<super::hud::Toast>,
    mut sound: ResMut<super::audio::Sound>,
    mut was: Local<Option<(u8, u16)>>,
) {
    let Some(n) = net.as_deref() else { return };
    let core = &n.session.core;
    let now = lit_in_hand(n).and_then(|_| Some((n.sel, core.inv.get(n.sel as usize)?.item)));
    if let (Some((sel, item)), None) = (*was, now) {
        let spent = core
            .inv
            .get(sel as usize)
            .is_some_and(|s| s.item == item && s.cond == 0);
        if n.light && n.sel == sel && spent && !core.wounded && !core.dead {
            toast.say("your torch burned out");
            sound.play(crate::sound::mixer::Request::own(
                crate::sound::Cue::TorchOut,
            ));
        }
    }
    *was = now;
}

/// The [`crate::ui::hold::HELD_MODELS`] row burning in your own hand, if
/// any — what [`hand_light`] lights the world with and [`hand_fire`] draws.
fn lit_in_hand(n: &Net) -> Option<usize> {
    let core = &n.session.core;
    // A downed body has dropped what it held, and the sim burns no flame on
    // it (`light::is_lit`) — so neither does this screen.
    let latch = n.light && !core.wounded && !core.dead;
    crate::ui::hold::lit_model_in_hand(&core.catalog, &core.inv, n.sel, latch)
}

/// Burn the fire on the torch in your hand ([`HandFire`]): step its tongues,
/// let the air take back a share of the head's motion, light new ones while
/// the torch is lit, and turn the lot to face the eye.
///
/// In `PostUpdate` after the propagation, `fx::draw`'s slot and reason: a
/// billboard faces the camera this frame renders, from where the hand is
/// this frame.
pub fn hand_fire(
    net: Option<NonSend<Net>>,
    time: Res<Time>,
    cam: Query<&GlobalTransform, (With<EyeCam>, Without<HandFire>)>,
    mut q: Query<(&mut HandFire, &Mesh3d, &GlobalTransform)>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    use super::fx::pool::Cam;
    let dt = time.delta_secs().min(0.1);
    let lit = net.as_deref().and_then(lit_in_hand).is_some();
    let Ok(cam) = cam.single() else { return };
    for (mut fire, mesh, gt) in &mut q {
        let fire = &mut *fire;
        let (scale, rot, at) = gt.to_scale_rotation_translation();
        let inv = rot.inverse();
        let k = 1.0 / scale.x.max(1e-4);
        fire.pool.step(dt);
        if let Some(last) = fire.last {
            let moved = at - last;
            if moved.length() < HAND_FIRE_JUMP_M {
                fire.pool.shift(-(inv * moved) * (k * HAND_FIRE_TRAIL));
            }
        }
        fire.last = Some(at);
        // Out, or put away: the emitter goes back to the wrist this frame
        // (`apply_hand_light`), and what was burning would go with it.
        if !lit {
            fire.pool.clear();
        }
        if lit && dt > 0.0 {
            // Fire rises up, whatever angle the torch is held at — mostly
            // up the SCREEN, because the torch is: it rides the view, so a
            // flame rising to the world's up streams at the lens whenever
            // the player looks at their feet.
            let up = (inv * Vec3::from(cam.up()) * 0.7 + inv * Vec3::Y * 0.3).normalize_or(Vec3::Y);
            let crown = Vec3::Y * -crate::ui::hold::FLAME_LIFT_M;
            super::fx::world::torch_flame(&mut fire.pool, crown, up, dt, true);
        }
        if !fire.pool.needs_write() {
            continue;
        }
        let local = Cam {
            pos: gt.affine().inverse().transform_point3(cam.translation()),
            right: inv * Vec3::from(cam.right()),
            up: inv * Vec3::from(cam.up()),
        };
        if let Some(m) = meshes.get_mut(&mesh.0) {
            fire.pool.write(m, &local, Vec3::ZERO);
        }
    }
}

/// [`hand_light`] with the held row and the gain as values. The half a gate
/// can drive — `structures::apply_fire_lights` is the same split for the
/// same reason: the socket is the only thing the system adds.
///
/// A row with no `light` and an empty hand are one case on purpose. There is
/// no third state: an emitter that is off is a zero, not a hidden entity,
/// so nothing here can leave a light burning for an item that is no longer
/// in the hand.
///
/// Since torch fuel v0 the caller narrows it further — `hold::lit_model_in_hand`
/// hands `None` for a torch whose latch is off or whose condition has run
/// out — and that is deliberately *its* job rather than a fourth branch
/// here: this function's contract is "draw the light this row declares",
/// and whether a flame is burning is a question about the sim's three
/// facts, not about a `PointLight`.
pub fn apply_hand_light(
    want: Option<usize>,
    gain: f32,
    mut q: Query<(&mut PointLight, &mut Transform), With<HandLight>>,
) {
    let (lumens, range, at) = match want.map(|i| &crate::ui::hold::HELD_MODELS[i]) {
        Some(row) => match row.light {
            Some(l) => (l.lumens * gain, l.range_m, flame_at(row)),
            None => (0.0, 0.0, Vec3::ZERO),
        },
        None => (0.0, 0.0, Vec3::ZERO),
    };
    for (mut light, mut tf) in &mut q {
        // Assign only on a change, `apply_fire_lights`' reason exactly:
        // `PointLight` is `Changed`-tracked and the render world re-extracts
        // what moved. A lit torch flickers, so it changes every frame; an
        // unlit one is written once. The transform is the same — and it is
        // the stronger case here, because a `Transform` write on a parent
        // still dirties the propagation.
        if light.intensity != lumens {
            light.intensity = lumens;
        }
        if light.range != range {
            light.range = range;
        }
        if tf.translation != at {
            tf.translation = at;
        }
    }
}

/// The held-item models, index-aligned with [`crate::ui::hold::HELD_MODELS`].
#[derive(Resource)]
pub struct Models {
    mesh: Vec<Handle<Mesh>>,
    mat: Vec<Handle<StandardMaterial>>,
}

impl Models {
    /// The geometry and the surface for one [`crate::ui::hold::HELD_MODELS`]
    /// row, cloned handles.
    ///
    /// Read by `swap` through the fields directly and by
    /// `bodies::stream` through here — the second reader is what makes
    /// this an accessor rather than two `pub` fields. Both vectors are
    /// built by [`load_models`] at the same index, so one bounds check
    /// answers for both, and a row index that is out of them is a
    /// `HELD_MODELS` change that skipped the loader (`tests/
    /// held_assets.rs`), not a runtime condition to recover from.
    pub fn row(&self, i: usize) -> (Handle<Mesh>, Handle<StandardMaterial>) {
        (self.mesh[i].clone(), self.mat[i].clone())
    }
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
    commands.insert_resource(SkinMats::default());
}

/// Integrate the three motions and write the one transform.
// Eight parameters, which clippy counts — the mixer is the eighth, and it is
// here because the swing's sound is a fact about the stroke this system
// starts (`bodies::stream`'s allow and the same argument).
#[allow(clippy::too_many_arguments)]
pub fn animate(
    time: Res<Time>,
    eye: Res<Eye>,
    feed: Res<Feed>,
    // `Option`, like `swap`'s: a capture run has no session, and a
    // viewmodel that refused to draw without one would take the held item
    // out of every probe frame.
    net: Option<NonSend<Net>>,
    mut m: ResMut<Motion>,
    // The swing's own sound, played HERE and nowhere else — see the stroke
    // trigger below. `Option` for `net`'s reason: a capture run has no
    // mixer to speak of and the arm still has to move in every probe frame.
    mut sound: Option<ResMut<super::audio::Sound>>,
    // `Without<HeldItem>` is not decoration: two `&mut Transform` queries in
    // one system have to be PROVABLY disjoint, and two different `With`
    // markers do not prove it — an entity could carry both.
    mut q: Query<&mut Transform, (With<HeldRig>, Without<HeldItem>)>,
    mut item: Query<(&mut Transform, Has<InHand>), With<HeldItem>>,
    mut zoom: ResMut<DrawZoom>,
) {
    let Ok(mut t) = q.single_mut() else { return };
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }
    // A bow in hand is drawn and loosed, never chopped: its draw and nock in
    // ticks, and the byte this frame sends.
    let bow = net.as_deref().and_then(|n| {
        let c = &n.session.core;
        crate::ui::hold::draw_in_hand(&c.catalog, &c.inv, n.sel).map(|d| (d, c.buttons()))
    });
    // Which way the tool is carried and how it is swung are both the row's,
    // so the row is resolved once here for the arm and the wrist together.
    // Resolved the way `swap` and `hand_light` resolve it —
    // `held_model_in_hand` is a pure lookup over the catalog and the
    // inventory mirror, and `hand_light`'s doc is the standing argument that
    // a second reader of a pure function is two calls and not the
    // second-drain defect `feed.rs` narrates.
    //
    // `None` is an empty hand, or a capture run with no session at all, and
    // it punches (`Stroke::Bash`).
    let def: Option<&'static HeldModelDef> = net
        .as_deref()
        .and_then(|n| held_model_in_hand(&n.session.core.catalog, &n.session.core.inv, n.sel))
        .map(|i| &HELD_MODELS[i]);
    // A crossbow or a revolver is fired, not swung: it kicks on each shot
    // the sim reports as this player's, and never chops.
    let shoots = def.is_some_and(|d| d.stroke == Stroke::Shot);
    // The byte the sim will act on, not the mouse — see `ClientCore::buttons`.
    let swinging = bow.is_none()
        && !shoots
        && net
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

    // ── Heave, off changes in vertical speed ─────────────────────────────
    // The arm is a weight on a spring from the shoulder: a take-off or a
    // landing jolts it down and it rings back, and a fall lets it float. A
    // change in speed drives it, not the speed, so a steady slope leaves it
    // still. Sub-stepped: a spring this stiff is unstable at a slow frame.
    let vy = step.y / dt;
    let jolt = (vy - m.last_vy).clamp(-VIEWMODEL_HEAVE_JOLT_MAX, VIEWMODEL_HEAVE_JOLT_MAX);
    m.last_vy = vy;
    m.heave_v -= jolt * VIEWMODEL_HEAVE_GAIN;
    let n = (dt * 120.0).ceil().clamp(1.0, 16.0);
    let h = dt / n;
    for _ in 0..n as u32 {
        m.heave_v += (-VIEWMODEL_HEAVE_K * m.heave - VIEWMODEL_HEAVE_C * m.heave_v) * h;
        m.heave += m.heave_v * h;
    }
    m.heave = m.heave.clamp(-VIEWMODEL_HEAVE_MAX, VIEWMODEL_HEAVE_MAX);

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
    //
    // Melee only: a bow's hits are its arrows landing, a flight after the
    // loose, and taking one for a missed swing chopped the bow like an axe.
    let landed_at_rest = bow.is_none()
        && !shoots
        && m.swing <= 0.0
        && (feed.hits > 0 || !feed.gathered().is_empty());
    // A queued stroke only starts from rest, like the backstop.
    let queued = std::mem::take(&mut m.queued) && m.swing <= 0.0;
    if predicted || landed_at_rest || queued {
        m.swing = 1.0;
        m.strokes = m.strokes.wrapping_add(1);
        // **The whoosh is a fact about the arm, so it fires with the arm**
        // (2026-09-13). `render::input` played it on every mouse press from
        // audio v0 until then, and a press is not a swing: the sim takes one
        // per `SWING_INTERVAL_TICKS` however fast the button is worked, so a
        // player spamming the click heard a whoosh a click over an arm that
        // moved once every 1.27 s — *"sound doesnt sync with animation if i
        // spam attack"*. The stroke, the cue and the score's small bump now
        // start on one line, at the sim's cadence, and cannot disagree. The
        // panel guard `input.rs` used to cite is already upstream of this:
        // `BTN_PRIMARY` is not set while a panel eats the click, so
        // `swinging` is false and the cadence never fires.
        //
        // `swing_apex_s` is 0.16 s in and `synth::whoosh` peaks a third of
        // the way through its 0.26 s, so starting the cue with the stroke
        // puts its loudest sample within a frame of the arm's apex.
        if let Some(sound) = sound.as_deref_mut() {
            sound.play(crate::sound::mixer::Request::own(crate::sound::Cue::Swing));
            sound.music.bump(crate::sound::music::BUMP_SWING);
        }
    }
    if m.swing > 0.0 {
        m.swing = (m.swing - dt / VIEWMODEL_SWING_S).max(0.0);
    }

    // ── The draw, off the right mouse ───────────────────────────────────
    //
    // Raised while the aim is held, pulled as the draw fills, and loosed on
    // the frame the sim would let it (`ui::draw`): the left button held at
    // full draw with arrows in the pack. The release sound is the shot's
    // own (`Cue::ShotBow`, off the sim's `EV_SHOT`).
    let now = time.elapsed_secs();
    let mut pull = 0.0;
    match bow {
        Some(((draw, nock), buttons)) => {
            let aiming = buttons & sim_core::input::BTN_AIM != 0;
            let hz = sim_core::limits::TICK_HZ as f32;
            let (draw_s, nock_s) = (draw as f32 / hz, nock as f32 / hz);
            // The creak is the draw starting: the right mouse going down.
            if aiming && !m.draw.aiming() {
                if let Some(sound) = sound.as_deref_mut() {
                    sound.play(crate::sound::mixer::Request::own(
                        crate::sound::Cue::BowDraw,
                    ));
                }
            }
            pull = m.draw.step(now, aiming, draw_s, nock_s);
            let arrows = || {
                net.as_deref().is_some_and(|n| {
                    crate::ui::hold::carries_arrows(&n.session.core.catalog, &n.session.core.inv)
                })
            };
            let loosed = aiming
                && buttons & sim_core::input::BTN_PRIMARY != 0
                && m.draw.ready(now, draw_s, nock_s)
                && arrows();
            if loosed {
                m.draw.loose(now);
                m.loose = 1.0;
                m.looses = m.looses.wrapping_add(1);
                pull = 0.0;
            }
            let k = 1.0 - (-VIEWMODEL_RAISE_RATE * dt).exp();
            let target = if aiming { 1.0 } else { 0.0 };
            m.raise += (target - m.raise) * k;
            // The string follows the draw back and leaves with the arrow,
            // but a draw let down without a shot is eased home rather than
            // snapped, which would read as a dry fire.
            m.string = if loosed || pull >= m.string {
                pull
            } else {
                m.string + (pull - m.string) * (1.0 - (-VIEWMODEL_STRING_EASE * dt).exp())
            };
            m.nocked = m.raise > 0.5 && m.loose <= 0.0 && arrows();
            // The hand lets go of the string rather than riding it home: it
            // holds where it was through the loose, then comes back for the
            // next arrow, and follows the string again once it has it.
            m.hand = if loosed || m.loose > 0.0 {
                m.hand
            } else if m.string >= m.hand {
                m.string
            } else {
                m.hand + (m.string - m.hand) * (1.0 - (-VIEWMODEL_STRING_EASE * dt).exp())
            };
        }
        None => {
            m.draw = crate::ui::draw::DrawClock::default();
            m.raise = 0.0;
            m.string = 0.0;
            m.nocked = false;
            m.hand = 0.0;
            if !shoots {
                m.loose = 0.0;
            }
        }
    }
    // A shot row's kick is the sim's shot, not a prediction of it: nothing
    // on this side mirrors a magazine or a reload, so the event is the one
    // honest source — a round trip late, with the report it plays under.
    if shoots {
        let me = net.as_deref().map(|n| n.session.core.player_id);
        if feed.shots().iter().any(|sh| Some(sh.0) == me) {
            m.loose = 1.0;
            m.looses = m.looses.wrapping_add(1);
        }
    }
    if m.loose > 0.0 {
        m.loose = (m.loose - dt / VIEWMODEL_LOOSE_S).max(0.0);
    }
    zoom.0 = m.raise * (0.7 + 0.3 * pull);
    let raise = m.raise;
    let kick = bump(1.0 - m.loose, 0.25);
    let (kick_off, climb) = if shoots {
        (VIEWMODEL_SHOT_KICK, VIEWMODEL_SHOT_CLIMB)
    } else {
        (VIEWMODEL_LOOSE_KICK, 0.0)
    };
    let draw_turn = Quat::from_euler(
        EulerRot::YXZ,
        VIEWMODEL_DRAW_TURN.x * raise,
        VIEWMODEL_DRAW_TURN.y * raise + climb * kick,
        VIEWMODEL_DRAW_TURN.z * raise,
    );
    let draw_off =
        VIEWMODEL_DRAW_RAISE * raise + VIEWMODEL_DRAW_PULL * (pull * raise) + kick_off * kick;
    // ── One pose for the whole assembly ─────────────────────────────────
    //
    // `1 - swing` runs the stroke forwards as `swing` counts down. The arc
    // is a rotation about the shoulder rather than a rotation of the item
    // about its own grip, which is the difference between the arm swinging
    // and the tool head waggling on a stationary fist — the operator's
    // *"hardly anything moves"* (2026-08-30) was the second of those.
    let s = 1.0 - m.swing;
    let (arc, throw) = stroke_pose(def.map_or(Stroke::Bash, |d| d.stroke), s);
    // The sway rides OUTSIDE the swing, so a turn taken mid-stroke lags the
    // whole assembly rather than bending the stroke.
    let lag = Quat::from_euler(EulerRot::YXZ, m.sway.x, m.sway.y, 0.0);
    // A lit item is carried up by the head; anything else comes back down.
    // Both eased, so a swap from a torch to an axe brings the arm down
    // rather than cutting to it.
    let lit = def.is_some_and(|d| d.light.is_some());
    let k = 1.0 - (-VIEWMODEL_LIFT_RATE * dt).exp();
    m.lift += (f32::from(u8::from(lit)) - m.lift) * k;
    m.carry += (carry_of(def) - m.carry) * k;
    let sheet = net.as_deref().is_some_and(super::sheet::up);
    m.stow += (f32::from(u8::from(sheet)) - m.stow) * k;
    m.idle_lag = lag;
    m.idle_drift = bob + Vec3::Y * m.heave;
    *t = carried(
        m.carry * (1.0 - raise),
        m.lift,
        lag * arc * draw_turn,
        throw + bob + draw_off + Vec3::Y * m.heave,
    );
    let stow = m.stow * m.stow * (3.0 - 2.0 * m.stow);
    t.translation += VIEWMODEL_STOW * stow;

    // ── The wrist, on top of the arm ────────────────────────────────────
    //
    // Everything from the shoulder to the fist is rigid, so the item arrives
    // flat without this. It is composed onto whichever frame the item is
    // hanging in — see [`item_pose`] — and it is written WHOLE every frame
    // rather than nudged, so a swing that ends on a dropped frame cannot
    // leave the grip a few degrees off forever.
    //
    // No contention with the two other writers in this file, and it is worth
    // being exact about why: `swap` and `hand_light` write the item's
    // CHILDREN (the model and the emitter), not the item, so the three
    // systems own one entity each and need no order between them.
    //
    // The hand eases to a new row's grip while the item takes its pose at
    // once, so a swap never swings the item through the frame.
    let (cock, strike) = swing_phases(s);
    let snap = stroke_snap(def, cock, strike);
    // A raised bow turns in the hand to aim; nothing else does.
    let aim = match (bow, def) {
        (Some(_), Some(d)) => bow_aim(d, m.raise),
        _ => Quat::IDENTITY,
    };
    // Holstered in THE GATE the fist is empty (`swap`), so it is the rest hand.
    let holstered = net.as_deref().is_some_and(|n| n.session.core.holstered());
    let want = hand_fit(def.filter(|_| !holstered));
    let k = 1.0 - (-VIEWMODEL_HAND_RATE * dt).exp();
    let fit = match m.fit {
        Some(f) => HandFit {
            rot: f.rot.slerp(want.rot, k),
            pos: f.pos.lerp(want.pos, k),
        },
        None => want,
    };
    m.fit = Some(fit);
    let mut grip = 0.0;
    if let Ok((mut it, in_hand)) = item.single_mut() {
        if in_hand {
            *it = item_pose(Some(fit), snap * aim);
            m.set = Some(hand_set(fit));
            // The fist closes on a row the hand fits round; a palmed or
            // stand-in row keeps the rest curl.
            if def
                .filter(|_| !holstered)
                .is_some_and(|d| d.grip_roll.is_some())
            {
                grip = 1.0;
            }
        } else {
            *it = item_pose(None, snap * aim);
            m.set = None;
        }
    }
    m.grip += (grip - m.grip) * k;
}

/// Put the first-person arm where [`animate`]'s grip wants it, over the hold
/// clip: the arm shifted in the rig, the forearm twisted about its own axis
/// and the hand bent at the wrist for the rest, so the fist closes on the row
/// in it ([`hand_fit`]). Between the animation and the propagation, like
/// `bow::draw_arm`: both bones are animated by the clip every frame, so
/// composing onto them never accumulates (`tests/viewmodel_arms.rs` holds
/// the clip to that).
pub fn pose_hand(
    m: Res<Motion>,
    mut arms: Query<(&ViewArms, &mut Transform)>,
    mut bones: Query<&mut Transform, Without<ViewArms>>,
) {
    let Ok((arms, mut root)) = arms.single_mut() else {
        return;
    };
    let (Some(hand), Some((turn, twist, shift))) = (arms.hand, m.set) else {
        return;
    };
    root.translation = VIEWMODEL_ARMS + shift;
    // Without a forearm the wrist takes the whole turn: the same hand, a
    // worse wrist, and never an item hung off a hand that did not turn.
    let twist = match arms.forearm.and_then(|f| bones.get_mut(f).ok()) {
        Some(mut f) => {
            f.rotation *= Quat::from_rotation_y(twist);
            twist
        }
        None => 0.0,
    };
    if let Ok(mut h) = bones.get_mut(hand) {
        h.rotation = Quat::from_rotation_y(-twist) * h.rotation * turn;
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
pub fn forget(mut m: ResMut<Motion>, mut zoom: ResMut<DrawZoom>) {
    *m = Motion::default();
    zoom.0 = 0.0;
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
