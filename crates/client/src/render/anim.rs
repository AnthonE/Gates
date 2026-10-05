//! Skeletal animation for other players.
//!
//! Remote bodies were `Capsule3d::new(0.4, 1.0)` — a pill that slid across the
//! ground without facing where it was looking, though the wire has carried
//! `yaw` and `pitch` the whole time and `bodies.rs` simply never read them.
//!
//! ## Bevy plays; it does not decide
//!
//! `RENDER.md` §1's rule, one surface over — the same shape `sound/` takes. The
//! clip a body plays is a **pure function of state the sim already sent**:
//! interpolated position, the yaw on the wire, and the sleeping flag. Nothing
//! here writes to `ClientCore`, nothing here is read back by the sim, and no
//! gameplay fact lives in an `AnimationPlayer`. If every animation in this file
//! were deleted the game would play identically and look worse, which is the
//! test for whether a renderer has started deciding things.
//!
//! **Speed is DERIVED, and it has to be.** The wire carries no velocity, so
//! the choice between idle, walk, jog and sprint comes from differencing the
//! interpolated position across a frame. Two consequences worth stating:
//! the value is noisy at low speed, so the thresholds have hysteresis
//! (`SPEED_HYSTERESIS`) rather than bare comparisons — without it a body
//! walking near a boundary flickers between two clips every frame; and it is
//! per-body render state, not sim state, so it lives in a component here and
//! never travels.
//!
//! **Clips are resolved by NAME, never by index.** `GltfAssetLabel::Animation(i)`
//! is positional, and `CLAUDE.md`'s trap list is explicit that positional
//! payloads are where the reference ecosystem actually bled — 27 of Oxide's
//! fixes were the right value in the wrong position. A re-export of the library
//! that inserts one clip would silently renumber every one after it and every
//! body in the game would play the wrong animation with all gates green.
//! `Gltf::named_animations` is a map, so a rename fails loudly at load instead.

use std::f32::consts::{FRAC_PI_2, PI, TAU};
use std::time::Duration;

use bevy::animation::AnimationTargetId;
use bevy::gltf::{Gltf, GltfNode};
use bevy::prelude::*;
use sim_core::movement::{SPRINT_SPEED, WADE_SPEED_MULT, WALK_SPEED};

/// The clips this game actually asks for.
///
/// **The rig changed under this enum twice in one day and the enum never had
/// to move**, which is the payoff for resolving by name. The commissioned
/// character (`assets/models/stumpy.glb`) arrived with seven clips where the
/// Quaternius mannequin had 46, so `Sprint` briefly aliased the jog; then
/// `ci/retarget_anim.py` moved all 46 onto the new skeleton and the alias was
/// deleted. Neither change touched a variant.
///
/// **One alias is left and it is a design choice, not a gap.** `Sleep` plays
/// the idle: a sleeper stands, because the sim hits it with the standing
/// capsule, and there is no pose that would be more honest than a person
/// standing still.
///
/// An unmatched name is not a silent fallback — `nodes[slot]` would stay
/// `AnimationNodeIndex::default()`, that index is the graph ROOT, and playing
/// the root plays nothing, so the body would stand frozen in its bind pose.
/// `build` says so loudly and `tests/rig_asset.rs` fails before it can ship.
///
/// Ordering is meaningful only to `ALL`; nothing depends on the discriminants.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Clip {
    Idle,
    Walk,
    Jog,
    Sprint,
    /// A sleeper. Stands — `NOW.md` §0y item 1 — because the sim hits it with
    /// the standing capsule `combat.rs` uses for everyone, so laying the mesh
    /// down would draw a body outside the volume the server shoots at.
    /// `bodies.rs` already argues this for the sleeper's colour and the same
    /// reasoning binds harder for a pose.
    Sleep,
    /// **The one-shot, and the only clip here that is not a loop.** A remote
    /// body swinging drew nothing before wire v47 — the one thing a fight
    /// needs to read is the wind-up, and another player's arm was perfectly
    /// still (`NOW.md` §0sw).
    Swing,
    /// **A body taking a blow.** The second one-shot, and the shortest
    /// clip this client plays: `Hit_Chest` runs 0.333 s in the shipped
    /// file (see [`FLINCH_CLIP_S`]).
    ///
    /// ⚠ **Only the attacker sees it, and that is a wire fact rather than
    /// a choice made here.** `EV_HIT` is unicast to the attacker
    /// (`sim_core::world`'s `EV_HIT` doc line; `server/src/core.rs` routes
    /// it by `a`), so the id that reaches this client is the body *your*
    /// blow landed on and no other. A flinch everybody could see would be
    /// a second broadcast on the hottest path in a fight, one per landed
    /// blow per player with no AOI filter — priced in `NOW.md` §0pvp and
    /// deliberately not taken. The asymmetry is recorded as a PROPOSED row
    /// in `DECISIONS.md` §open ("attacker-side flinch v0") so it can be
    /// reversed by a word rather than by archaeology.
    Flinch,
    /// A killed body, falling and then staying down. **The second
    /// non-looping clip, and the only one that is a STATE rather than a
    /// transient** — which is the whole reason it needed a wire bit
    /// (`dead`, v48) where the swing needed an event.
    ///
    /// A corpse keeps its slot until its owner leaves the death screen, so
    /// before v48 the client drew a killed player standing at idle: you
    /// could not tell from the body whether the person in front of you was
    /// still in the fight. It outranks everything, including `Sleep` — a
    /// sleeper who is killed is a corpse, and the sim agrees (`die` carries
    /// `sleeping` forward but `hp == 0` is what every weapon tests).
    ///
    /// **Laying this body down is safe in a way `Sleep`'s is not**, which
    /// is why the pose argument that keeps a sleeper standing does not
    /// reach here: `combat::strike`, `ranged` and the blast all skip
    /// `hp == 0`, and players do not collide with each other, so a corpse
    /// is inside no volume the server tests. There is nothing left for the
    /// drawn pose to disagree with.
    Death,
    /// **A body loosing a shot**: a bow, a crossbow or a revolver. The third
    /// one-shot, and like the swing it is an input fact the snapshot cannot
    /// imply, so it rides the shot's own broadcast (`EV_SHOT`). Before it a
    /// remote archer stood at idle while arrows left their face.
    Shoot,
    /// **Off the ground**: a jump, or a fall off something. A looping state
    /// like the gait, off the `grounded` bit every record has carried since
    /// the first wire; until it was read a jumping player rose a metre and
    /// came back down still walking.
    Air,
    /// **Crouched, still** (wire v83). The sim tests a crouched body's
    /// shorter cylinder (`collide::CROUCH_HEIGHT_M`, measured off this very
    /// clip's head), so drawing it standing would show a head where no head
    /// is. A crouch outranks the swing, the shot and the flinch: those clips
    /// drive the hips and legs and would stand the body up for their span.
    CrouchIdle,
    /// Crouched and moving.
    CrouchWalk,
}

impl Clip {
    /// The name in the glTF. Resolved through `Gltf::named_animations`.
    /// Public because the asset gate reads it: `tests/rig_asset.rs` checks
    /// every name this returns against the shipped file, and a gate that
    /// re-typed the list would be checking its own copy rather than the
    /// client's.
    pub fn name(self) -> &'static str {
        match self {
            Clip::Idle => "Idle_Loop",
            Clip::Walk => "Walk_Loop",
            Clip::Jog => "Jog_Fwd_Loop",
            // A real sprint since 2026-08-17 (`ci/retarget_anim.py` moved
            // the mannequin's library onto this skeleton). The locomotion
            // loops play at the rate that keeps their feet planted at the
            // body's real speed — see [`Clip::stride_mps`].
            Clip::Sprint => "Sprint_Loop",
            // Deliberately not a T-pose: a sleeper is a person standing
            // still, and the T-pose is a rig artifact.
            Clip::Sleep => "Idle_Loop",
            // **`Sword_Attack`, retimed at import to fit exactly** — see
            // [`SWING_CLIP_S`]. It was `Punch_Cross` for a day, chosen
            // because it was the only candidate short enough to fit the
            // cadence unmodified, and the operator rejected it on sight:
            // *"our model has a big head and its like leaning forward in
            // that clip and the hands clip all into the head."* Measured
            // against the shipped mesh and they are right — a punch brings
            // a hand to **0.147 m** of the head's centre where the vertices
            // weighted to `Head` reach 0.295 m, so 15 cm of hand is inside
            // the log. `Sword_Attack` stays **0.490 m** clear, which is the
            // widest berth of any swing in the library.
            //
            // **The clip was made to fit rather than the fit made to
            // accept the clip**, which is the whole of this change: a
            // shorter clip existed and was wrong for this body, so the
            // right one was retimed onto the cadence instead
            // (`ci/retarget_anim.py --retime`).
            Clip::Swing => "Sword_Attack",
            // Plays once and **holds its last pose** — the omitted
            // `.repeat()` in `drive`, and `RepeatAnimation::default()` is
            // `Never`, so the body falls and stays fallen for as long as
            // the corpse is on the wire. 2.375 s in the shipped file,
            // against a death screen a player sits on for as long as they
            // like, so there is no cadence for it to fit inside the way
            // the swing has one.
            //
            // **Measured off the shipped file rather than assumed** (the
            // `ANIM_RIG_H_M` habit): sampling the clip's last keyframe
            // through the joint chain puts `Head` at **y 0.107 m** where it
            // starts at 1.229, `Hips` at 0.052, and both feet within 1 cm of
            // the ground — so it ends genuinely prone and not merely
            // slumped. It also carries ~0.95 m of baked root motion
            // backwards along the body's own Z, which means the drawn corpse
            // settles about a metre from the point the wire names. That is
            // cosmetic and stays: nothing in the sim tests a corpse's volume
            // (`combat::strike`, `ranged` and the blast all skip `hp == 0`),
            // so there is no second opinion for it to disagree with — but it
            // is the reason a *loot bag* must keep coming from the wire's
            // position and never from where the body is drawn.
            // **`Hit_Chest` and not `Hit_Head`**, which is the only other
            // candidate the file has (`Hit_Head`, 0.417 s). Nothing on this
            // wire says where a blow landed — `EV_HIT` carries a victim and
            // a damage and no body part — so picking the head clip would be
            // the renderer asserting a fact the sim never sent, which is
            // the one thing `RENDER.md` §1 forbids it. The chest is the
            // honest reading of "you were hit".
            Clip::Flinch => "Hit_Chest",
            Clip::Death => "Death01",
            // Both arms out and a kick: the rig's one firing clip, and it
            // reads for a bow too. The `Archery_Shot_*` clips hold the bow
            // in the LEFT hand, where this client hangs every item off the
            // right (`HAND_BONE`).
            Clip::Shoot => "Pistol_Shoot",
            // Arms out and a knee tucked, looped: a jump is 0.7 s in the air
            // (`movement::JUMP_SPEED`, `GRAVITY`), shorter than any of the
            // rig's take-off or landing clips, so the arc is this and the
            // blends either side.
            Clip::Air => "Jump_Loop",
            Clip::CrouchIdle => "Crouch_Idle_Loop",
            Clip::CrouchWalk => "Crouch_Fwd_Loop",
        }
    }

    /// Public because the asset gate reads it: `tests/rig_asset.rs` walks this
    /// list against the shipped file, and a gate holding its own copy would be
    /// checking itself rather than the client.
    pub const ALL: [Clip; 12] = [
        Clip::Idle,
        Clip::Walk,
        Clip::Jog,
        Clip::Sprint,
        Clip::Sleep,
        Clip::Swing,
        Clip::Flinch,
        Clip::Death,
        Clip::Shoot,
        Clip::Air,
        Clip::CrouchIdle,
        Clip::CrouchWalk,
    ];

    /// The ground speed a locomotion loop's feet were authored for, m/s, and
    /// `None` for everything that is not one.
    ///
    /// **Measured off `stumpy.glb`, not chosen**: the speed of the planted
    /// foot against the hips (forward kinematics over the clip, the lowest
    /// foot or toe within 3 cm of the ground, median over the stance). Walk
    /// 1.05, jog 3.97, sprint 5.66, crouch ~0.5. [`BodyAnim::rate`] divides the
    /// body's real speed by this, which is what stops a jogging body skating
    /// at the sim's 3.0 m/s on a clip authored for 4.
    pub fn stride_mps(self) -> Option<f32> {
        match self {
            Clip::Walk => Some(1.05),
            Clip::Jog => Some(3.97),
            Clip::Sprint => Some(5.66),
            Clip::CrouchWalk => Some(0.5),
            _ => None,
        }
    }

    /// Where in each locomotion loop the left foot is furthest forward, as a
    /// fraction of the clip — measured off the file like [`Clip::stride_mps`].
    /// A gait change lines the incoming loop up on it, so a walk blending
    /// into a jog keeps the same foot in front instead of crossing its legs
    /// for the length of the blend.
    pub fn stride_mark(self) -> Option<f32> {
        match self {
            Clip::Walk => Some(0.25),
            Clip::Jog => Some(0.208),
            Clip::Sprint => Some(0.875),
            Clip::CrouchWalk => Some(0.125),
            _ => None,
        }
    }

    /// Plays once and holds its last frame, rather than looping.
    pub fn one_shot(self) -> bool {
        matches!(self, Clip::Swing | Clip::Flinch | Clip::Death | Clip::Shoot)
    }

    /// The one-shots a body can play over its gait: a swing, a shot, a flinch.
    pub fn transient(self) -> bool {
        matches!(self, Clip::Swing | Clip::Flinch | Clip::Shoot)
    }

    /// The transient's slot in [`Rig::upper`].
    fn upper_slot(self) -> Option<usize> {
        match self {
            Clip::Swing => Some(0),
            Clip::Flinch => Some(1),
            Clip::Shoot => Some(2),
            _ => None,
        }
    }

    fn slot(self) -> usize {
        match self {
            Clip::Idle => 0,
            Clip::Walk => 1,
            Clip::Jog => 2,
            Clip::Sprint => 3,
            Clip::Sleep => 4,
            Clip::Swing => 5,
            Clip::Flinch => 6,
            Clip::Death => 7,
            Clip::Shoot => 8,
            Clip::Air => 9,
            Clip::CrouchIdle => 10,
            Clip::CrouchWalk => 11,
        }
    }
}

/// How long the one-shot swing clip runs, seconds.
///
/// **Derived from the sim's own cadence, and the ASSET is cut to match it** —
/// not the other way round, and not a number anybody typed. A player may swing
/// every `SWING_INTERVAL_TICKS / TICK_HZ` = 1.267 s; the stroke has to be over
/// and blended back into the gait before the next one starts, the blend out
/// costs [`ANIM_BLEND_S`], and one resample frame is left over so "before" is
/// strict. So the clip gets whatever is left.
///
/// `ci/retarget_anim.py --retime Sword_Attack=1.05333` writes the clip that
/// long — the motion is complete and only its clock is compressed, 1.5 s of
/// authored swing played over 1.053 — and
/// `tests/rig_asset.rs::the_swing_clip_fits_the_swing_cadence` fails if the
/// file and this constant ever disagree. **That is the loop worth having**:
/// change the sim's cadence and the constant moves, the shipped clip no longer
/// matches, and the gate says so instead of the body silently never finishing
/// a stroke.
/// ⚠ **A literal, and deliberately not the expression that derives it.** The
/// knob registry (`ci/knob_registry.mjs`) pins every shipped number to a
/// spoken declaration, and it cannot read an expression — writing the
/// derivation here made the gate refuse the file. The coupling is not lost,
/// it moved to where it can be checked against the asset as well:
/// `the_swing_clip_fits_the_swing_cadence` recomputes
/// `SWING_INTERVAL_TICKS / TICK_HZ − ANIM_BLEND_S − one frame` from the sim's
/// own constants and fails if this literal, or the shipped clip, drifts from
/// it. **The frame of margin is not slack**: the retime quantizes the clip to
/// whole 30 Hz frames, so a stroke derived to land exactly ON the cadence can
/// round to just past it, and `tests/anim.rs` asks for a strict inequality for
/// that reason.
pub const SWING_CLIP_S: f32 = 1.05333;

/// How long the one-shot flinch runs, seconds.
///
/// **Measured off the shipped file, not chosen** — the `ANIM_RIG_H_M`
/// habit. `Hit_Chest`'s longest sampler input maxes at 0.33333 s, and
/// `tests/rig_asset.rs::the_flinch_clip_is_the_length_the_client_thinks_it_is`
/// re-reads the file and fails if the two ever disagree. Unlike
/// [`SWING_CLIP_S`] there is nothing to fit it inside: the sim has no
/// cadence on being hit — three arrows can land in a second — so the clip
/// is taken at the length it was authored and the transient rule
/// ([`BodyAnim::flinch`]) is what stops two of them fighting.
pub const FLINCH_CLIP_S: f32 = 0.33333;

/// How long the flinch takes to cross-fade IN, seconds. Shorter than
/// [`ANIM_BLEND_S`], and derived rather than dialled.
///
/// The clip's own build-up is what sets it. Sampling every rotation channel
/// of `Hit_Chest` against its first keyframe, the largest joint deviation
/// climbs 0.04° → 19.92° and peaks at **t = 0.16667 s**, then settles back to
/// 14.32° and holds there to the end. So the pose is fully struck a sixth of
/// a second in, and a blend that has not finished by then draws the apex at
/// partial weight — the body is *most* bent at the one moment it is *least*
/// visible. The general blend is 0.18 s, which is past that peak, so reusing
/// it would damp the only frame of this clip that carries the information.
///
/// Blending in exactly AT the apex is the loosest bound that still holds the
/// property, and it is the file's own number rather than a taste call.
/// `tests/rig_asset.rs::the_flinch_blend_lands_on_the_clips_own_apex`
/// re-measures it out of the binary chunk and fails if the clip is re-cut.
///
/// The blend back OUT is [`ANIM_BLEND_S`] like everything else — a recovery
/// is allowed to be softer than an impact.
pub const FLINCH_BLEND_S: f32 = 0.16667;

/// How long the one-shot shot runs, seconds: `Pistol_Shoot` as authored,
/// measured off the file like [`FLINCH_CLIP_S`] and held to it by
/// `tests/rig_asset.rs`. A bow looses every two seconds at most, so it
/// always finishes.
pub const SHOOT_CLIP_S: f32 = 0.625;

/// Speed thresholds, m/s, and the band around each that a body must cross to
/// change its mind. Derived speed is noisy — a packet arriving a millisecond
/// late reads as a lurch — so a bare `>` at a boundary makes a body alternate
/// clips every frame. The hysteresis is the cheapest fix and the only one that
/// does not add latency.
///
/// **Read off the sim's own speeds.** These were 0.6 / 3.0 / 5.4 with the band
/// on top, so the sim's ordinary pace (`WALK_SPEED`, 3.0) never cleared 3.35
/// and played the walk, and its sprint (5.5) never cleared 5.75 and played the
/// jog: every running player in the game was drawn one gait slow. Each
/// boundary now sits halfway between two speeds the sim actually moves at.
///
/// Low enough that a crouch through water (0.85 m/s) still steps.
pub const ANIM_WALK_MPS: f32 = 0.45;
/// Halfway between a wading walk (1.5 m/s) and a dry one (3.0): on this rig
/// the sim's ordinary pace is a jog, and the walk is for water.
pub const ANIM_JOG_MPS: f32 = (WALK_SPEED * WADE_SPEED_MULT + WALK_SPEED) * 0.5;
/// Halfway between the ordinary pace (3.0) and the sprint (5.5).
pub const ANIM_SPRINT_MPS: f32 = (WALK_SPEED + SPRINT_SPEED) * 0.5;
/// Half-width of the dead band around each threshold, m/s.
pub const ANIM_SPEED_HYSTERESIS: f32 = 0.35;

/// The slowest and fastest a locomotion loop is played, as a multiple of its
/// authored rate ([`BodyAnim::rate`]). Inside the band the feet stay planted;
/// past it they slide, which beats a slow-motion jog or a crouch scuttling at
/// four steps a second. The crouch is the clip that reaches the top: authored
/// at ~0.5 m/s against the sim's 1.7.
pub const ANIM_RATE_MIN: f32 = 0.5;
pub const ANIM_RATE_MAX: f32 = 3.0;

/// The furthest the legs turn away from where the body faces, radians (~69°).
///
/// The rig has no strafe or backpedal clips, so a body moving sideways turns
/// its HIPS toward the travel and its spine back toward the aim
/// ([`pose_spine`]): the legs run the forward loop along the ground they are
/// actually covering and the chest and head stay on target. Past this the
/// twist reads as broken rather than athletic, and the feet slide instead.
pub const ANIM_TWIST_MAX: f32 = 1.2;
/// How far past side-on the travel must swing before the legs flip between
/// running forward and backpedalling, radians each way of 90°. Without it a
/// body strafing exactly side-on would flip every frame.
pub const ANIM_BACK_BAND: f32 = 0.26;
/// How fast the legs swing round to a new travel direction, 1/s.
pub const ANIM_TWIST_EASE: f32 = 10.0;
/// A standing body's feet stay planted while its torso turns this far,
/// radians (~46°); past it they step back round under it.
pub const ANIM_IDLE_TWIST_MAX: f32 = 0.8;
/// How fast they step round once they go, rad/s.
pub const ANIM_IDLE_TURN_RPS: f32 = 5.0;

/// How far a crouch clip's head is lifted back toward level, radians: the
/// difference between `Crouch_Idle_Loop`'s gaze and `Idle_Loop`'s on the
/// shipped rig (−49.6° vs −7.9°).
pub const CROUCH_HEAD_LIFT: f32 = 0.73;

/// How long a clip change takes to cross-fade, seconds. Long enough that a
/// walk→jog is not a snap, short enough that it is not a slide.
pub const ANIM_BLEND_S: f32 = 0.18;

/// The rig's own height, metres — **measured off the shipped file, not
/// assumed and not measured at runtime.**
///
/// `stumpy.glb` measures **1.800 m** with its feet on y = 0, read off the
/// spawned scene by `cargo run -p client --features render --bin modelview`
/// (which computes it the way the GPU does — `JointWorld_0 · IBM_0` over the
/// mesh's bind box — because a skinned mesh is not drawn where its node says
/// it is). The sim's player is `Capsule3d::new(0.4, 1.0)` — 1.0 of cylinder
/// plus two 0.4 caps, so **1.8 m** — so the two agree exactly and the correct
/// scale is 1. The retired mannequin was 1.829 and wanted 0.9843.
///
/// **The first cut measured this at runtime and was wrong in a way worth
/// recording.** It walked every `Mesh` in `Assets<Mesh>` looking for the
/// tallest under 100 m — but that store holds terrain chunks, boulders and
/// 6.6 m conifers, so it would have fitted the scale to a TREE and drawn every
/// player at 27% height. A measurement taken off the wrong population is worse
/// than a constant, because it looks principled. If the rig is ever
/// re-vendored at a different height, this is a number to re-measure with the
/// two-line Python in `assets/models/MANIFEST.md`'s history, not a system to
/// reintroduce.
pub const ANIM_RIG_H_M: f32 = 1.800;
/// What the sim collides and shoots with — `bodies.rs`'s retired capsule, and
/// the height the drawn body must agree with. A renderer that disagrees with
/// the sim about how tall a player is draws a head that cannot be shot.
pub const ANIM_BODY_H_M: f32 = 1.8;

/// The rig: the scene to spawn per body, and the graph every player shares.
#[derive(Resource)]
pub struct Rig {
    gltf: Handle<Gltf>,
    pub scene: Option<Handle<Scene>>,
    pub graph: Option<Handle<AnimationGraph>>,
    /// One node per [`Clip`], indexed by `Clip::slot`.
    ///
    /// ⚠ **This width, `Clip::ALL`'s and the two constructors' are one
    /// number in four places, and nothing but a runtime index-out-of-bounds
    /// connects them.** Adding a variant and moving three of the four
    /// compiles clean and panics the first time that clip is played — which
    /// on a one-shot means the first time anybody swings near you.
    /// `tests/anim.rs` counts them against `Clip::ALL` as text for exactly
    /// that reason.
    nodes: [AnimationNodeIndex; 12],
    /// Each clip's length, seconds, indexed by `Clip::slot` — what the gait
    /// phase sync in [`drive`] converts a seek time into a stride phase with.
    durations: [f32; 12],
    /// The transients again, **masked off the legs** ([`Clip::upper_slot`]):
    /// a swing, flinch or shot played over a moving gait poses the arms and
    /// spine and leaves the hips and legs running. `None` for a clip the file
    /// did not have.
    upper: [Option<AnimationNodeIndex>; 3],
    /// Uniform scale that puts the rig at [`ANIM_BODY_H_M`]. A constant ratio
    /// of two measured heights, not a runtime fit — see [`ANIM_RIG_H_M`].
    pub scale: f32,
    /// Names the glTF did not have. Non-empty means the library was re-vendored
    /// and a clip was renamed — reported once, loudly, rather than silently
    /// falling back to idle forever.
    missing: Vec<&'static str>,
    /// The graph node for [`ARMS_HOLD_CLIP`]. Not in `nodes`, because that
    /// array is indexed by `Clip::slot` and the arms are not a body state —
    /// nothing in `BodyAnim` can ever select this.
    arms: AnimationNodeIndex,
    /// Whether the shade step has reached a conclusion — either it took the
    /// model's material or it refused the file and said why.
    ///
    /// **Its own flag, and not `ready()`, because the two can finish on
    /// different frames.** The graph needs the `Gltf` and nothing else; the
    /// shades additionally need the `StandardMaterial` sub-asset to be *in*
    /// `Assets<StandardMaterial>`. Folding them together is a permanent
    /// failure waiting to happen: `build` returns early once `ready()`, so a
    /// single frame where the material had not landed yet would leave every
    /// player in the world wearing the untextured fallback for the whole
    /// session, with no error anywhere and a body that looks merely drab.
    shaded: bool,
}

impl Rig {
    /// True once the glTF has loaded and the graph is built. `bodies::stream`
    /// waits on this: spawning a body before it would spawn one with no scene.
    pub fn ready(&self) -> bool {
        self.scene.is_some() && self.graph.is_some()
    }
    /// True once [`BodyShades`] holds its final handles. A body painted
    /// before this keeps the fallback for its life, so a one-shot spawn
    /// (THE GATE's shopkeepers) waits on it as well as on [`Rig::ready`].
    pub fn shaded(&self) -> bool {
        self.shaded
    }
    fn node(&self, c: Clip) -> AnimationNodeIndex {
        self.nodes[c.slot()]
    }
    fn duration(&self, c: Clip) -> f32 {
        self.durations[c.slot()].max(1e-3)
    }
    fn upper_node(&self, c: Clip) -> Option<AnimationNodeIndex> {
        self.upper[c.upper_slot()?]
    }
    /// Metres of ground between this body's drawn footsteps, or `None` when
    /// its gait is not a locomotion loop. The loop plants two feet a cycle,
    /// and a cycle lasts the clip's length over the rate it is played at.
    pub fn step_m(&self, anim: &BodyAnim) -> Option<f32> {
        let clip = anim.clip?;
        clip.stride_mps()?;
        let cycle_s = self.duration(clip) / anim.rate().abs();
        Some(anim.speed * cycle_s * 0.5)
    }
    /// The hold pose for the first-person arms.
    pub fn arms_node(&self) -> AnimationNodeIndex {
        self.arms
    }
}

/// Start the load. A `Startup` system beside `textures::load`, for the same
/// reason: an `OnEnter(Screen::Loading)` system runs *before* `Startup` on a
/// connected start, so anything a streamer needs has to be requested here.
pub fn load(
    mut commands: Commands,
    assets: Res<AssetServer>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // **A fallback pair, replaced by [`build`] the moment the glTF lands.**
    // Untextured, because for the frame or two before the asset is in there is
    // nothing to texture with — and because a resource that does not exist
    // yet makes Bevy skip every system that asks for it, silently. Two flat
    // colours that are never seen beat a system that never runs.
    commands.insert_resource(BodyShades {
        awake: materials.add(StandardMaterial {
            base_color: Color::srgb(0.42, 0.36, 0.28),
            perceptual_roughness: 0.75,
            ..default()
        }),
        sleeping: materials.add(StandardMaterial {
            base_color: Color::srgb(0.24, 0.26, 0.30),
            perceptual_roughness: 0.9,
            ..default()
        }),
        keeper: materials.add(StandardMaterial {
            base_color: Color::srgb(0.30, 0.38, 0.22),
            perceptual_roughness: 0.8,
            ..default()
        }),
        from_gltf: false,
    });
    commands.insert_resource(Rig {
        gltf: assets.load("models/stumpy.glb"),
        scene: None,
        graph: None,
        nodes: [AnimationNodeIndex::default(); 12],
        durations: [1.0; 12],
        upper: [None; 3],
        arms: AnimationNodeIndex::default(),
        scale: ANIM_BODY_H_M / ANIM_RIG_H_M,
        missing: Vec::new(),
        shaded: false,
    });
}

/// Build the graph and the shades once the glTF is in. Runs every frame until
/// **both** have finished, then costs two branches.
#[allow(clippy::too_many_arguments)]
pub fn build(
    mut rig: ResMut<Rig>,
    gltfs: Res<Assets<Gltf>>,
    clips: Res<Assets<AnimationClip>>,
    gltf_nodes: Res<Assets<GltfNode>>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut shades: ResMut<BodyShades>,
) {
    if rig.ready() && rig.shaded {
        return;
    }
    let Some(gltf) = gltfs.get(&rig.gltf) else {
        return;
    };

    // **The waking shade is now the MODEL'S OWN material, and that is a
    // reversal.** It used to be a flat brown, because the mannequin's
    // materials were another game's yellow-and-purple preview colours and
    // anything was better. The commissioned character carries a baked bark
    // albedo that is the entire reason it was commissioned, so painting over
    // it would throw away the asset and leave a brown pill with a face.
    //
    // The sleeper still has to be distinguishable — `BodyShades` says why at
    // length, and it is the one thing here that is load-bearing rather than
    // decorative — so it is the same material, TINTED. `base_color` multiplies
    // the base-colour texture in Bevy, so a cold dark factor darkens the bark
    // without replacing it: still obviously the same character, obviously not
    // awake.
    //
    // **One material only, and the guard is loud.** Repainting every mesh with
    // `materials[0]` is exactly right for a single-material character and
    // silently wrong for a two-material one — it would flatten the second onto
    // the first. A model that grows a second material needs `reshade` to
    // remember each mesh's own handle instead, so this refuses rather than
    // guessing, and keeps the flat fallback so the game still draws bodies.
    //
    // **Retried until the material is actually readable, not until the `Gltf`
    // is.** The two are separate assets and the sub-asset can arrive a frame
    // later; giving up on the first miss is what would strand every body in
    // the fallback for a session.
    if !rig.shaded {
        match gltf.materials.as_slice() {
            [only] => {
                // The skin is baked near-black (linear luma 0.056, level with
                // grass: `NOW.md` §0dk), so players read as silhouettes and
                // the first-person hand as a dark glove. Lift the shared
                // material in place, which reaches every body and both arms
                // at once; the sleeper and keeper shades set their own colour.
                if let Some(m) = materials.get_mut(only) {
                    m.base_color = Color::LinearRgba(LinearRgba::rgb(
                        BODY_ALBEDO_LIFT,
                        BODY_ALBEDO_LIFT,
                        BODY_ALBEDO_LIFT,
                    ));
                }
                if let Some(base) = materials.get(only) {
                    let mut tinted = base.clone();
                    tinted.base_color = Color::srgb(0.34, 0.38, 0.46);
                    tinted.perceptual_roughness = (tinted.perceptual_roughness + 0.1).min(1.0);
                    // THE GATE's shopkeepers: the same bark gone mossy, so
                    // a vendor never reads as a player standing at a stall
                    // (Rust's vendors wear what no player can).
                    let mut moss = base.clone();
                    moss.base_color = Color::srgb(0.62, 0.86, 0.50);
                    shades.awake = only.clone();
                    shades.sleeping = materials.add(tinted);
                    shades.keeper = materials.add(moss);
                    shades.from_gltf = true;
                    rig.shaded = true;
                }
            }
            many => {
                error!(
                    "anim: models/stumpy.glb carries {} materials, not 1 — the sleeper \
                     shade cannot repaint a multi-material body without losing one of \
                     them, so bodies keep the untextured fallback",
                    many.len()
                );
                // Concluded, not succeeded. Without this the error repeats
                // every frame for the life of the process, which is a log
                // nobody can read and a refusal nobody can act on.
                rig.shaded = true;
            }
        }
    }
    if rig.ready() {
        return;
    }

    // The clips' lengths and the skeleton's node tree are sub-assets of the
    // glTF and can land a frame after it: wait for both rather than build a
    // graph that cannot phase-sync or cannot mask.
    let mut durations = [1.0; 12];
    for clip in Clip::ALL {
        if let Some(h) = gltf.named_animations.get(clip.name()) {
            let Some(c) = clips.get(h) else { return };
            durations[clip.slot()] = c.duration();
        }
    }
    let Some(legs) = leg_targets(gltf, &gltf_nodes) else {
        return;
    };

    let mut graph = AnimationGraph::new();
    let root = graph.root;
    let mut nodes = [AnimationNodeIndex::default(); 12];
    let mut upper = [None; 3];
    let mut missing = Vec::new();
    for clip in Clip::ALL {
        match gltf.named_animations.get(clip.name()) {
            Some(h) => {
                nodes[clip.slot()] = graph.add_clip(h.clone(), 1.0, root);
                if let Some(i) = clip.upper_slot() {
                    upper[i] = Some(graph.add_clip_with_mask(h.clone(), LEGS_MASK, 1.0, root));
                }
            }
            None => missing.push(clip.name()),
        }
    }
    // The hips and legs are mask group 0, so an upper node never poses them.
    if legs.is_empty() {
        error!(
            "anim: no {HIPS_BONE:?} bone found under the rig's animation root — \
             swings over a moving gait will move the legs too"
        );
    }
    for target in legs {
        graph.add_target_to_mask_group(target, LEGS_GROUP);
    }
    // The arms share the body's graph rather than owning a second one: one
    // graph asset, one handle, and `bind` cannot bind the arms by accident
    // because it requires a `BodyAnim` ancestor and the arms have none.
    let mut arms = AnimationNodeIndex::default();
    match gltf.named_animations.get(ARMS_HOLD_CLIP) {
        Some(h) => arms = graph.add_clip(h.clone(), 1.0, root),
        None => missing.push(ARMS_HOLD_CLIP),
    }

    if !missing.is_empty() {
        error!(
            "anim: {} clip name(s) missing from models/stumpy.glb: {:?} — \
             the rig was re-imported and a clip was renamed. `ci/import_char.py \
             --rename OLD=NEW` is where that is fixed; a missing name draws a \
             body frozen in its bind pose",
            missing.len(),
            missing
        );
    }
    rig.missing = missing;
    rig.nodes = nodes;
    rig.durations = durations;
    rig.upper = upper;
    rig.arms = arms;
    rig.graph = Some(graphs.add(graph));
    rig.scene = gltf
        .default_scene
        .clone()
        .or_else(|| gltf.scenes.first().cloned());
}

/// The mask group the hips and legs belong to, and the mask that keeps an
/// upper-body node off them.
const LEGS_GROUP: u32 = 0;
const LEGS_MASK: u64 = 1 << LEGS_GROUP;

/// The bone the legs and the spine both hang from. Found by name like
/// [`HEAD_BONE`], for the same reason.
const HIPS_BONE: &str = "Hips";

/// The animation targets of the hips and legs: the [`HIPS_BONE`] and every
/// bone under it except the branch that leads to the head.
///
/// Structural rather than a list of nine leg-bone names, so a re-import that
/// renames a toe does not quietly put it in the upper body. The ids are the
/// glTF loader's own — each node's name path from the animation root — which
/// is what `AnimationTargetId::from_names` hashes. `None` while the node
/// sub-assets are still landing.
fn leg_targets(gltf: &Gltf, nodes: &Assets<GltfNode>) -> Option<Vec<AnimationTargetId>> {
    if gltf.nodes.iter().any(|h| !nodes.contains(h)) {
        return None;
    }
    let Some(root) = gltf
        .nodes
        .iter()
        .filter_map(|h| nodes.get(h))
        .find(|n| n.is_animation_root)
    else {
        return Some(Vec::new());
    };
    // Does this subtree hold the head? Bounded like every walk in this file.
    fn holds_head(n: &GltfNode, nodes: &Assets<GltfNode>, depth: u32) -> bool {
        n.name == HEAD_BONE
            || (depth < 32
                && n.children
                    .iter()
                    .filter_map(|c| nodes.get(c))
                    .any(|c| holds_head(c, nodes, depth + 1)))
    }
    let mut out = Vec::new();
    // (node, its name path, inside the legs)
    let mut stack = vec![(root, vec![Name::new(root.name.clone())], false)];
    while let Some((n, path, legs)) = stack.pop() {
        if out.len() > 256 || path.len() > 32 {
            break;
        }
        let legs = legs || n.name == HIPS_BONE;
        if legs {
            out.push(AnimationTargetId::from_names(path.iter()));
        }
        for c in n.children.iter().filter_map(|c| nodes.get(c)) {
            // The spine is the hips' one child that leads to the head.
            if legs && holds_head(c, nodes, 0) {
                continue;
            }
            let mut p = path.clone();
            p.push(Name::new(c.name.clone()));
            stack.push((c, p, legs));
        }
    }
    Some(out)
}

/// What a body wants to be playing. Written by `bodies::stream` off state the
/// sim sent; read by [`drive`] and [`pose_spine`]. A component and not a
/// resource because it is per-body, and render state rather than sim state
/// because the sim has no opinion about which clip anybody is in.
#[derive(Component, Default)]
pub struct BodyAnim {
    /// The gait: what the legs are doing. [`BodyAnim::observe`] recomputes it
    /// every frame; the one-shots live beside it in their own clocks.
    pub clip: Option<Clip>,
    /// Metres per second, low-passed. Public so `bodies` can integrate it.
    pub speed: f32,
    /// Horizontal velocity, world x/z, low-passed beside [`BodyAnim::speed`].
    /// Its direction against [`BodyAnim::facing`] is what turns the legs.
    pub vel: Vec2,
    /// Last interpolated position, for the difference.
    pub last: Option<Vec3>,
    /// Which way the body faces, radians of sim yaw (0 is +Z, increasing
    /// toward +X). Set by the streamer beside `airborne`, before `observe`.
    pub facing: f32,
    /// Last frame's facing, so a standing body knows how far it turned.
    pub last_facing: Option<f32>,
    /// Where this body is looking, radians from level, positive up. Straight
    /// off the wire — `bodies::stream` decodes it — and read by
    /// [`pose_spine`]. A body that looks at you is the difference between a
    /// figure and a person, and it costs no packet.
    pub pitch: f32,
    /// How far the legs are turned from the facing, radians about up:
    /// toward the travel while moving, held planted while standing.
    /// [`pose_spine`] turns the hips by it and the spine back by it.
    pub hip_yaw: f32,
    /// The legs are backpedalling: the gait loop plays in reverse.
    pub backward: bool,
    /// The planted legs of a standing body are stepping back round under it.
    pub turning: bool,
    /// Seconds left of a one-shot swing.
    ///
    /// **Beside the gait rather than inside `clip`, and that is the whole
    /// design.** `observe` recomputes `clip` from speed every single frame,
    /// so a one-shot written there would be stomped the next one — the
    /// transient has to live in a field nothing else recomputes.
    pub swing_s: f32,
    /// Seconds left of a one-shot flinch. One slot with the swing and the
    /// shot: see [`BodyAnim::flinch`].
    pub flinch_s: f32,
    /// Seconds left of a one-shot shot.
    pub shoot_s: f32,
    /// The running one-shot owns the whole body, legs included, because it
    /// began over a standing gait — the swing's lunge is authored with its
    /// feet. Over a moving gait it plays on the upper body only and the legs
    /// keep running ([`BodyAnim::wants_upper`]). Decided when the one-shot
    /// starts and kept for its span, so a body that sets off mid-swing does
    /// not restart the arc on the other layer.
    pub transient_full: bool,
    /// Off the ground this frame, straight off the wire
    /// (`RemoteState::airborne`); `bodies::stream` sets it before
    /// [`BodyAnim::observe`] reads it.
    pub airborne: bool,
    /// Crouched this frame, straight off the wire (`RemoteState::crouched`,
    /// v83); set beside [`BodyAnim::airborne`].
    pub crouched: bool,
    /// Bumped once per one-shot heard. `drive` compares it against what it
    /// last started, so a second swing arriving while the first is still
    /// playing restarts the clip instead of being swallowed by the
    /// `playing == want` guard. One counter for all three, because there is
    /// only ever one transient.
    pub transient_seq: u32,
}

impl BodyAnim {
    /// Fold a new sample in and choose a gait. `dt` is the frame's delta.
    ///
    /// The low pass is on the SPEED and not on the position: smoothing the
    /// position would fight the interpolator, which is already the authority
    /// on where the body is (`bodies.rs` header).
    pub fn observe(&mut self, pos: Vec3, dt: f32, sleeping: bool, dead: bool, wounded: bool) {
        // The one-shots' clocks, run here because this is the one function
        // every live body passes through every frame with a `dt` in hand.
        // `bodies::stream` calls this BEFORE it hears the frame's swings,
        // so a swing heard this frame gets its whole span.
        self.swing_s = (self.swing_s - dt).max(0.0);
        self.flinch_s = (self.flinch_s - dt).max(0.0);
        self.shoot_s = (self.shoot_s - dt).max(0.0);
        if let (Some(last), true) = (self.last, dt > 0.0) {
            // Horizontal only. A body riding terrain up a hill is walking, not
            // climbing, and counting the vertical would read a slope as speed.
            let step = pos - last;
            let raw = Vec2::new(step.x, step.z) / dt;
            // One-pole, fixed per-second constant so the smoothing does not
            // change with the frame rate.
            let k = 1.0 - (-12.0 * dt).exp();
            self.speed += (raw.length() - self.speed) * k;
            self.vel += (raw - self.vel) * k;
        }
        self.last = Some(pos);
        let turned = self.last_facing.map_or(0.0, |f| wrap_pi(self.facing - f));
        self.last_facing = Some(self.facing);

        self.clip = Some(self.gait(sleeping, dead, wounded));
        // A whole-body one-shot stands the body up, and the sim tests a
        // crouched body's shorter cylinder, so a crouch ends it. An
        // upper-body one carries on: its legs are already the crouch's.
        if self.transient_full && matches!(self.clip, Some(Clip::CrouchIdle | Clip::CrouchWalk)) {
            self.swing_s = 0.0;
            self.flinch_s = 0.0;
            self.shoot_s = 0.0;
        }
        self.steer(turned, dt);
    }

    /// The gait for this frame, off the flags and the speed.
    fn gait(&self, sleeping: bool, dead: bool, wounded: bool) -> Clip {
        // **Death outranks everything, including sleep and a swing in
        // flight** — the sim's own order (`hp == 0` is what every weapon
        // tests, and `die` carries `sleeping` forward). A body killed
        // mid-stroke stops swinging: `wants` prefers this over the one-shot
        // while `dead` holds.
        //
        // **Down is drawn as the fall, held** (wounded v0): the rig has no
        // crawl clip, so a downed body plays `Death01` once and keeps its
        // last pose, and a crawl slides that pose along the ground until a
        // drag clip lands (`NOW.md` §0wnd).
        if dead || wounded {
            return Clip::Death;
        }
        if sleeping {
            return Clip::Sleep;
        }
        // In the air the legs are not walking, whatever the speed says. The
        // speed keeps integrating, so the landing picks the right gait.
        if self.airborne {
            return Clip::Air;
        }
        // Hysteresis: the threshold to speed UP is above the nominal and the
        // one to slow DOWN is below it, so a body sitting on a boundary keeps
        // whatever it already had.
        let h = ANIM_SPEED_HYSTERESIS;
        let now = self.clip.unwrap_or(Clip::Idle);
        let rank = |c: Clip| match c {
            Clip::Sprint => 3,
            Clip::Jog => 2,
            Clip::Walk | Clip::CrouchWalk => 1,
            _ => 0,
        };
        let up = |t: f32| self.speed > t + h;
        let down = |t: f32| self.speed < t - h;
        let moving = up(ANIM_WALK_MPS) || (rank(now) >= 1 && !down(ANIM_WALK_MPS));
        // **Crouched** (v83): the sim tests a crouched body's shorter
        // cylinder (`collide::CROUCH_HEIGHT_M`, measured off this clip's
        // head), so drawing it standing would show a head where no head is.
        if self.crouched {
            return if moving {
                Clip::CrouchWalk
            } else {
                Clip::CrouchIdle
            };
        }
        if up(ANIM_SPRINT_MPS) || (rank(now) >= 3 && !down(ANIM_SPRINT_MPS)) {
            Clip::Sprint
        } else if up(ANIM_JOG_MPS) || (rank(now) >= 2 && !down(ANIM_JOG_MPS)) {
            Clip::Jog
        } else if moving {
            Clip::Walk
        } else {
            Clip::Idle
        }
    }

    /// Turn the legs: toward the travel while a locomotion loop runs, held
    /// planted while the body stands, and back under the body otherwise.
    /// `turned` is how far the facing moved since last frame.
    fn steer(&mut self, turned: f32, dt: f32) {
        let ease = 1.0 - (-ANIM_TWIST_EASE * dt).exp();
        match self.clip {
            Some(c) if c.stride_mps().is_some() => {
                self.turning = false;
                // The travel's bearing against the facing, in the sim's yaw
                // sense: atan2(x, z), zero straight ahead.
                let rel = wrap_pi(self.vel.x.atan2(self.vel.y) - self.facing);
                if self.backward {
                    self.backward = rel.abs() > FRAC_PI_2 - ANIM_BACK_BAND;
                } else {
                    self.backward = rel.abs() > FRAC_PI_2 + ANIM_BACK_BAND;
                }
                // Backpedalling turns the legs toward the OPPOSITE of the
                // travel, so the reversed loop walks them along it.
                let want = if self.backward {
                    wrap_pi(rel - PI)
                } else {
                    rel
                };
                let want = want.clamp(-ANIM_TWIST_MAX, ANIM_TWIST_MAX);
                self.hip_yaw += (want - self.hip_yaw) * ease;
            }
            Some(Clip::Idle | Clip::CrouchIdle) => {
                self.backward = false;
                // Planted: the feet keep their heading while the body turns,
                // so the twist takes up the turn...
                self.hip_yaw =
                    wrap_pi(self.hip_yaw - turned).clamp(-ANIM_TWIST_MAX, ANIM_TWIST_MAX);
                // ...until it is too far, and they step back round.
                if self.hip_yaw.abs() > ANIM_IDLE_TWIST_MAX {
                    self.turning = true;
                }
                if self.turning {
                    let step = ANIM_IDLE_TURN_RPS * dt;
                    self.hip_yaw -= self.hip_yaw.clamp(-step, step);
                    self.turning = self.hip_yaw.abs() > 0.02;
                }
            }
            _ => {
                self.backward = false;
                self.turning = false;
                self.hip_yaw -= self.hip_yaw * ease;
            }
        }
    }

    /// The playback rate for the gait: the body's real speed over the speed
    /// the loop was authored for, so the feet stay planted, and negative
    /// while backpedalling. 1 for anything that is not a locomotion loop.
    pub fn rate(&self) -> f32 {
        let Some(stride) = self.clip.and_then(Clip::stride_mps) else {
            return 1.0;
        };
        let r = (self.speed / stride).clamp(ANIM_RATE_MIN, ANIM_RATE_MAX);
        if self.backward {
            -r
        } else {
            r
        }
    }

    /// The running one-shot, if any. They are mutually exclusive by
    /// construction — each one's start clears the others' clocks — so this
    /// order never decides anything.
    fn running(&self) -> Option<Clip> {
        if self.flinch_s > 0.0 {
            Some(Clip::Flinch)
        } else if self.swing_s > 0.0 {
            Some(Clip::Swing)
        } else if self.shoot_s > 0.0 {
            Some(Clip::Shoot)
        } else {
            None
        }
    }

    /// What the whole body plays: the gait, or a one-shot that began over a
    /// standing one. `None` until `observe` has run once.
    ///
    /// **The gait is read first, because one of its values outranks every
    /// one-shot.** A body killed mid-stroke must stop swinging and must not
    /// flinch, so `Clip::Death` switches both layers off. The clocks are
    /// deliberately left running underneath: death is a state that can end
    /// (the bit clears when its owner leaves the death screen), and a
    /// one-shot *cleared* by a corpse could not be told from one that had
    /// expired.
    pub fn wants(&self) -> Option<Clip> {
        let gait = self.clip?;
        if gait == Clip::Death {
            return Some(gait);
        }
        match self.running() {
            Some(t) if self.transient_full => Some(t),
            _ => Some(gait),
        }
    }

    /// What the upper body plays over the gait: a one-shot that began while
    /// the legs were busy — running, crouched, in the air. `drive` plays it
    /// on a node masked off the hips and legs, so they keep their gait.
    pub fn wants_upper(&self) -> Option<Clip> {
        if self.clip? == Clip::Death {
            return None;
        }
        self.running().filter(|_| !self.transient_full)
    }

    /// Whichever one-shot just started: who owns the legs, and a new
    /// sequence number so `drive` restarts the clip.
    fn begin(&mut self) {
        self.transient_full = matches!(self.clip, None | Some(Clip::Idle | Clip::Sleep));
        self.transient_seq = self.transient_seq.wrapping_add(1);
    }

    /// Start a one-shot swing on this body. Clears the other one-shots —
    /// see [`BodyAnim::flinch`] for why they are one slot and the newest wins.
    pub fn swing(&mut self) {
        self.swing_s = SWING_CLIP_S;
        self.flinch_s = 0.0;
        self.shoot_s = 0.0;
        self.begin();
    }

    /// Start a one-shot shot on this body: it just loosed an arrow or fired
    /// a round.
    pub fn shoot(&mut self) {
        self.shoot_s = SHOOT_CLIP_S;
        self.swing_s = 0.0;
        self.flinch_s = 0.0;
        self.begin();
    }

    /// Start a one-shot flinch on this body: it just took a blow of yours.
    ///
    /// ## The one-shots are one slot, and the newest wins
    ///
    /// **1. Deferring would hide the flinch in the one situation it exists
    /// for.** The sim allows a swing every `SWING_INTERVAL_TICKS / TICK_HZ`
    /// = 1.267 s and [`SWING_CLIP_S`] runs 1.053 s of that, so ranking the
    /// swing above the flinch would drop ~83% of flinches in a melee fight.
    ///
    /// **2. Truncating the arc removes no sim fact.** `EV_SWING` is pushed
    /// after the swing has already resolved, so the drawn arc is a replay:
    /// cutting it short predicts nothing and cancels nothing.
    ///
    /// **3. The loser's clock has to be stopped, not just outranked.** A
    /// swing still counting down under a flinch would be wanted again the
    /// moment the flinch expired, and `drive` would replay it from frame
    /// zero.
    pub fn flinch(&mut self) {
        self.flinch_s = FLINCH_CLIP_S;
        self.swing_s = 0.0;
        self.shoot_s = 0.0;
        self.begin();
    }
}

/// An angle wrapped into `(-π, π]`.
fn wrap_pi(a: f32) -> f32 {
    let w = (a + PI).rem_euclid(TAU) - PI;
    if w <= -PI {
        w + TAU
    } else {
        w
    }
}

/// A body whose descendants still need our materials painted onto them.
///
/// **One thing makes this necessary, and it used to be two.** The retired
/// mannequin's own materials were `M_Main` and `M_Joints` — a yellow-and-purple
/// preview rig — so a body spawned untouched arrived in another game's
/// colours; that half is gone, because the commissioned character's material
/// IS what we want a waking body to wear (`build` assigns it as `awake`).
/// What remains is the half that was always load-bearing:
/// `bodies.rs` distinguishes a sleeper from a waking player by SHADE, which
/// it calls load-bearing in its own comment: "is that player about to shoot me,
/// or is nobody home" is the question sleepers create, and a client that draws
/// both identically makes it unanswerable. A `SceneRoot` has no material of its
/// own, so both have to reach the spawned descendants.
///
/// Carried as a component and cleared when it lands, because **the scene
/// spawns asynchronously**: at the frame `bodies::stream` inserts this there
/// are no descendants yet, and a one-shot paint would silently miss every body.
#[derive(Component)]
pub struct Reshade(pub Shade);

/// Which of [`BodyShades`] a body is painted in.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Shade {
    Awake,
    Sleeping,
    /// One of THE GATE's shopkeepers — never a player.
    Keeper,
}

impl Reshade {
    /// A player's body, awake or asleep.
    pub fn body(sleeping: bool) -> Self {
        Reshade(if sleeping {
            Shade::Sleeping
        } else {
            Shade::Awake
        })
    }
}

/// Paint our materials onto a scene's meshes. Runs only for bodies carrying
/// [`Reshade`] — at spawn, and again on a sleep transition — so the descendant
/// walk is not a per-frame cost.
pub fn reshade(
    mut commands: Commands,
    children: Query<&Children>,
    has_mesh: Query<(), With<Mesh3d>>,
    pending: Query<(Entity, &Reshade)>,
    waking: Res<BodyShades>,
) {
    for (body, want) in &pending {
        let mat = match want.0 {
            Shade::Awake => waking.awake.clone(),
            Shade::Sleeping => waking.sleeping.clone(),
            Shade::Keeper => waking.keeper.clone(),
        };
        // Bounded breadth-first walk. The rig is 55 nodes; the cap is a wall-4
        // habit applied to a traversal rather than to a queue, so a malformed
        // hierarchy cannot hang a frame.
        let mut stack = vec![body];
        let mut painted = 0usize;
        let mut visited = 0usize;
        while let Some(e) = stack.pop() {
            visited += 1;
            if visited > 256 {
                break;
            }
            if has_mesh.get(e).is_ok() {
                commands.entity(e).insert(MeshMaterial3d(mat.clone()));
                painted += 1;
            }
            if let Ok(kids) = children.get(e) {
                stack.extend(kids.iter());
            }
        }
        // Nothing painted means the scene has not spawned yet — leave the
        // marker on and try again next frame.
        if painted > 0 {
            commands.entity(body).remove::<Reshade>();
        }
    }
}

/// The two body shades — lifted out of `bodies.rs`, which could hold them on a
/// mesh entity when a body WAS one mesh. A resource so `reshade` does not
/// rebuild them and so every body shares one handle, which is what lets Bevy
/// batch a crowd of players into few draws.
///
/// **Same mesh, same pose, different shade** — and the pose is the deliberate
/// half. A sleeper stands (`NOW.md` §0y item 1), because the sim hits it with
/// the standing capsule `combat.rs` uses for everyone; laying the mesh down
/// would draw a body outside the volume the server blocks and shoots at, which
/// is the one thing `CLAUDE.md` still says is worth gating about a frame. A
/// colour cannot disagree with the sim about where anything is.
///
/// It is programmer art and it is load-bearing anyway: "is that player about
/// to shoot me, or is nobody home" is the question sleepers create, and a
/// client that draws both identically makes the answer unknowable.
#[derive(Resource)]
pub struct BodyShades {
    pub awake: Handle<StandardMaterial>,
    pub sleeping: Handle<StandardMaterial>,
    /// THE GATE's shopkeepers ([`Shade::Keeper`]).
    pub keeper: Handle<StandardMaterial>,
    /// False while these are [`load`]'s untextured placeholders, true once
    /// [`build`] has replaced them with the model's own material and its
    /// tinted copy. Read by nothing yet; it is here so a body drawn brown is
    /// answerable without a debugger — the two states look different and only
    /// one of them is a bug.
    pub from_gltf: bool,
}

/// The multiplier on `stumpy.glb`'s baked skin. 2.4 brings its 0.056 linear
/// luma to litter's 0.135 and clips 0.14% of texels (measured in `NOW.md`
/// §0dk); 3.0 would reach twig's 0.167 at 0.34%.
pub const BODY_ALBEDO_LIFT: f32 = 2.4;

/// How far a body may look from level, radians (~74°), against a wire that
/// can carry ~88°. A clamp and not a scale: scaling makes every glance an
/// understatement, and clamping is only wrong at the extremes.
///
/// The look is shared down the spine ([`SpineBones`]): the head takes the
/// most of it and the chest the rest, the way a person looking at their own
/// feet bends at the chest. It used to be the head alone, clamped at 0.9 rad,
/// with the remainder dropped.
pub const ANIM_LOOK_PITCH_MAX: f32 = 1.3;

/// How a look is split between the bones that share it, by weight: each
/// spine bone 1, the neck 1.5, the head 3. Normalized over the bones a rig
/// actually has, so the shares always sum to the whole look.
const LOOK_SPINE_W: f32 = 1.0;
const LOOK_NECK_W: f32 = 1.5;
const LOOK_HEAD_W: f32 = 3.0;

/// The body's right, in its own frame. **The rig faces +Z** — glTF's own
/// convention, the sim's yaw 0, and what the walk measures: the planted foot
/// travels toward −Z — so its right hand is on −X (`bodies.rs` measured that
/// off the hand bone). "Look up" is a turn about the right.
const BODY_RIGHT: Vec3 = Vec3::NEG_X;

/// One bone [`pose_spine`] turns, and what it last wrote.
struct SpineBone {
    entity: Entity,
    /// Its share of the leg twist: +1 on the hips, minus a third of it on each
    /// spine bone so the chest faces the aim again, 0 above.
    yaw: f32,
    /// Its share of the look.
    pitch: f32,
    /// The head: the crouch's head lift goes here alone.
    head: bool,
    /// The local delta this system last composed in, so it can be taken back
    /// out on a frame the animation did not rewrite the bone.
    applied: Quat,
    /// The rotation this system last wrote. A bone that still holds it was
    /// not touched by the animation this frame.
    written: Quat,
}

/// The bones of one body that the look and the leg twist turn: the hips (if
/// the rig names them) up the spine to the head, root first.
///
/// **The axes are not typed.** Every turn is composed in the BODY's frame —
/// a twist about its up, a look about its right — and carried into each
/// bone's parent frame through the rotations above it, read off the pose the
/// animation just wrote. This rig's bones are not axis-aligned, and a typed
/// axis once produced a head that rolled toward its shoulder instead of
/// nodding.
#[derive(Component)]
pub struct SpineBones {
    /// The rotation from the body root's frame into the first bone's parent,
    /// read at bind: the scene root's stand-up turn. Nothing animates it.
    root: Quat,
    bones: Vec<SpineBone>,
}

/// Find each body's spine and work out how the look and the twist are shared.
///
/// Runs on `Added<AnimationPlayer>` like [`bind`] and for the same reason: it
/// is the one moment the scene's entities exist and nothing has posed them.
pub fn bind_spine(
    mut commands: Commands,
    names: Query<(Entity, &Name)>,
    parents: Query<&ChildOf>,
    transforms: Query<&Transform>,
    bodies: Query<Entity, (With<BodyAnim>, Without<SpineBones>)>,
    added: Query<Entity, Added<AnimationPlayer>>,
) {
    for player in &added {
        let mut at = player;
        let mut body = None;
        for _ in 0..16 {
            if bodies.get(at).is_ok() {
                body = Some(at);
                break;
            }
            match parents.get(at) {
                Ok(p) => at = p.0,
                Err(_) => break,
            }
        }
        let Some(body) = body else { continue };

        // The head, by name, among this body's descendants only — two bodies
        // are in the world at once and a global search would find somebody
        // else's. Its ancestors up to the body are the chain.
        let mut chain = Vec::new();
        for (e, name) in &names {
            if name.as_str() != HEAD_BONE {
                continue;
            }
            let mut walk = vec![e];
            let mut up = e;
            let mut ours = false;
            for _ in 0..16 {
                match parents.get(up) {
                    Ok(p) if p.0 == body => {
                        ours = true;
                        break;
                    }
                    Ok(p) => {
                        up = p.0;
                        walk.push(up);
                    }
                    Err(_) => break,
                }
            }
            if ours {
                walk.reverse();
                chain = walk;
                break;
            }
        }
        if chain.is_empty() {
            error!(
                "anim: no bone named {HEAD_BONE:?} under a body — remote heads \
                 will not follow the wire's pitch"
            );
            commands.entity(body).insert(SpineBones {
                root: Quat::IDENTITY,
                bones: Vec::new(),
            });
            continue;
        }
        // Start at the hips when there are any: above them is the scene's own
        // root, which nothing should turn.
        let is_hips = |e: Entity| names.get(e).is_ok_and(|(_, n)| n.as_str() == HIPS_BONE);
        let hips = chain.iter().position(|&e| is_hips(e));
        let chain = &chain[hips.unwrap_or(0)..];
        // The rest rotation from the body down to the first bone's parent,
        // walked from local transforms: global ones have not been propagated
        // for a scene spawned this frame.
        let mut root = Quat::IDENTITY;
        let mut up = parents.get(chain[0]).map(|p| p.0).unwrap_or(body);
        for _ in 0..16 {
            if up == body {
                break;
            }
            if let Ok(t) = transforms.get(up) {
                root = t.rotation * root;
            }
            match parents.get(up) {
                Ok(p) => up = p.0,
                Err(_) => break,
            }
        }
        // hips? | spine… | neck | head
        let rest = &chain[usize::from(hips.is_some())..];
        let n = rest.len();
        let spine = n.saturating_sub(2);
        let weight = |i: usize| match n - 1 - i {
            0 => LOOK_HEAD_W,
            1 => LOOK_NECK_W,
            _ => LOOK_SPINE_W,
        };
        let total: f32 = (0..n).map(weight).sum();
        let mut bones = Vec::with_capacity(chain.len());
        if hips.is_some() {
            bones.push(SpineBone::new(chain[0], 1.0, 0.0, false));
        }
        for (i, &e) in rest.iter().enumerate() {
            // The spine turns the chest back to the aim. No hips, no twist.
            let yaw = if hips.is_some() && i < spine {
                -1.0 / spine as f32
            } else {
                0.0
            };
            bones.push(SpineBone::new(e, yaw, weight(i) / total, i + 1 == n));
        }
        commands.entity(body).insert(SpineBones { root, bones });
    }
}

impl SpineBone {
    fn new(entity: Entity, yaw: f32, pitch: f32, head: bool) -> Self {
        Self {
            entity,
            yaw,
            pitch,
            head,
            applied: Quat::IDENTITY,
            written: Quat::IDENTITY,
        }
    }
}

/// The bone that nods. A name, because clips resolve by name for the reasons
/// this module's header gives at length, and a skeleton's bone names are the
/// same kind of contract.
const HEAD_BONE: &str = "Head";

/// The bone that HOLDS. Published because two files resolve it — the
/// first-person arms (`viewmodel::dress_arms`) and every remote body
/// (`bodies::bind_hands`) — and it was a bare `"RightHand"` literal in both
/// until 2026-08-31.
///
/// **A literal in two files is the hand-kept mirror `CLAUDE.md` warns about,
/// with a twist that makes it worse than usual**: neither copy fails to
/// compile if the rig renames the bone, and neither logs anything a player
/// would see. The viewmodel would draw an undressed body wrapped around the
/// camera and a remote would carry nothing — two different symptoms, one
/// cause, and no gate pointing at the name itself. `tests/rig_asset.rs`
/// resolves this constant against the shipped file, so a rename is a build
/// failure now rather than two silent regressions.
pub const HAND_BONE: &str = "RightHand";

/// The clip the FIRST-PERSON arms hold (`render/viewmodel.rs`).
///
/// **Chosen by measurement, and the name is the library's rather than ours.**
/// A viewmodel needs a pose with the hands up in front of the eye, and the
/// retargeted library was searched for one by computing where each clip puts
/// the right hand in view space: nine clips put a hand in frame and this is
/// the only one that is both a two-handed hold and a LOOP — `Pistol_Aim_Neutral`
/// is a 0.17 s pose and `Archery_Shot_1` a one-shot. It is called what its
/// source called it, because renaming a retargeted clip would break the one
/// property that makes the library legible: these are the mannequin's names.
pub const ARMS_HOLD_CLIP: &str = "Pistol_Idle_Loop";

/// The half of the mesh a first-person view draws, and the half it hides.
/// Written by `ci/split_arms.py`; `tests/rig_asset.rs` fails if either goes
/// missing, because a re-import that forgets the split silently removes the
/// arms and leaves a body wrapped around the camera.
pub const ARMS_NODE: &str = "char1_arms";
pub const BODY_NODE: &str = "char1_body";

/// Turn every remote's hips toward where its legs are going, its spine back
/// toward where it faces, and its chest and head to where the wire says it
/// is looking.
///
/// **Scheduled between the animation and the transform propagation**, which
/// is the only window where this is cheap: the clip has posed the skeleton and
/// nothing has turned local transforms into world ones yet, so a handful of
/// quaternion multiplies per body and no re-propagation.
///
/// ## The animation rewrites these bones every frame — usually
///
/// Every clip on this rig animates every one of them, so most frames the
/// bone arrives fresh from the clip and the turn composes straight onto it.
/// The previous cut of this system *always* took last frame's delta back out
/// first, which against a freshly animated bone cancels this frame's: every
/// remote head in the game held the clip's own gaze and never followed the
/// wire's pitch, while a fixture with no animation passed. A bone that still
/// holds exactly what this system wrote was not animated this frame, and only
/// then is the old delta removed — so a clip that leaves a bone alone still
/// cannot wind it round sixty times a second.
///
/// Bevy draws, it does not decide (`RENDER.md` §1): the pitch is a value the
/// sim already sent, the twist is derived from where the body moved, this
/// writes transforms, and nothing reads them back.
pub fn pose_spine(
    mut bodies: Query<(&BodyAnim, &mut SpineBones)>,
    mut bones: Query<&mut Transform>,
) {
    for (anim, mut spine) in &mut bodies {
        // **A corpse is not looking at anything.** The wire keeps carrying
        // the pitch a body died holding, and composing it onto `Death01`'s
        // fallen pose cranks the head of a body lying on the ground. Both
        // turns drop to nothing, through the same bookkeeping, so the delta
        // already written is still removed.
        let dead = anim.clip == Some(Clip::Death);
        let pitch = if dead {
            0.0
        } else {
            anim.pitch.clamp(-ANIM_LOOK_PITCH_MAX, ANIM_LOOK_PITCH_MAX)
        };
        // The crouch clips hold the head ~42° further down than `Idle` (gaze
        // −49.6° vs −7.9°), so a crouched body looking level would stare at
        // the ground: lift the head by the difference.
        let lift = match anim.clip {
            Some(Clip::CrouchIdle | Clip::CrouchWalk) => CROUCH_HEAD_LIFT,
            _ => 0.0,
        };
        let yaw = if dead { 0.0 } else { anim.hip_yaw };
        // The body frame → this bone's parent frame, built down the chain.
        let mut w = spine.root;
        for bone in spine.bones.iter_mut() {
            let Ok(mut t) = bones.get_mut(bone.entity) else {
                break;
            };
            let base = if t.rotation == bone.written {
                bone.applied.inverse() * t.rotation
            } else {
                t.rotation
            };
            let look = pitch * bone.pitch + if bone.head { lift } else { 0.0 };
            let turn =
                Quat::from_rotation_y(yaw * bone.yaw) * Quat::from_axis_angle(BODY_RIGHT, look);
            // A turn in the body frame, carried into the parent's.
            // Normalized at every step: a bone the clip never rewrites is
            // composed onto itself every frame, and through a chain of three
            // the rounding compounds into a visibly shrinking head.
            let local = (w.inverse() * turn * w).normalize();
            t.rotation = (local * base).normalize();
            bone.applied = local;
            bone.written = t.rotation;
            w = (w * t.rotation).normalize();
        }
    }
}

/// Which body an `AnimationPlayer` belongs to.
///
/// The player lands on a DESCENDANT of the spawned scene, not on the body root,
/// so the link has to be walked and recorded once rather than searched every
/// frame.
#[derive(Component)]
pub struct PlayerOf(pub Entity);

/// What a player is currently playing, so [`drive`] writes on a transition and
/// not every frame. Assigning unconditionally would restart the clip 60 times
/// a second and every body would stand frozen in its first pose — a failure
/// that looks exactly like "the animation did not load".
#[derive(Component, Default)]
pub struct Playing {
    /// The whole-body clip, and the one-shot sequence it was started for.
    base: Option<Clip>,
    base_seq: u32,
    /// The locomotion loop fading out under `base`, kept stepping in time
    /// with it until its weight is gone.
    fading: Option<Clip>,
    /// The upper-body one-shot, and its sequence.
    upper: Option<Clip>,
    upper_seq: u32,
    /// How much of the arms and spine the upper one-shot owns, 0..1.
    upper_u: f32,
}

/// Attach the graph to every newly spawned player and find its body.
pub fn bind(
    mut commands: Commands,
    rig: Res<Rig>,
    parents: Query<&ChildOf>,
    bodies: Query<Entity, With<BodyAnim>>,
    added: Query<Entity, Added<AnimationPlayer>>,
) {
    let (Some(graph), true) = (rig.graph.clone(), rig.ready()) else {
        return;
    };
    for player in &added {
        // Walk up to the body root. Bounded rather than `loop`: a malformed
        // hierarchy must not hang a frame (`CLAUDE.md` wall 4's habit, applied
        // to a walk rather than to a queue).
        let mut at = player;
        let mut found = None;
        for _ in 0..16 {
            if bodies.get(at).is_ok() {
                found = Some(at);
                break;
            }
            match parents.get(at) {
                Ok(p) => at = p.0,
                Err(_) => break,
            }
        }
        let Some(body) = found else { continue };
        commands.entity(player).insert((
            AnimationGraphHandle(graph.clone()),
            AnimationTransitions::new(),
            PlayerOf(body),
            Playing::default(),
        ));
    }
}

/// How long a cross-fade INTO `want` takes, seconds.
///
/// One value for everything except the flinch, whose blend is derived from
/// its own clip rather than shared — [`FLINCH_BLEND_S`] says why.
fn blend(want: Clip) -> f32 {
    match want {
        Clip::Flinch => FLINCH_BLEND_S,
        _ => ANIM_BLEND_S,
    }
}

/// The most of the arms and spine the upper layer is given, short of 1 so
/// its weight below stays finite.
const UPPER_MAX: f32 = 0.995;

/// A stable phase in `[0, 1)` per body, so a crowd standing still does not
/// breathe in unison — the six shopkeepers at THE GATE did, to the frame.
fn desync(body: Entity) -> f32 {
    let h = body.to_bits().wrapping_mul(0x9E37_79B9_7F4A_7C15);
    (h >> 40) as f32 / (1u64 << 24) as f32
}

/// Cross-fade each player to whatever its body wants, play its gait at the
/// rate that plants its feet, and lay any upper-body one-shot over it.
pub fn drive(
    rig: Res<Rig>,
    time: Res<Time>,
    anims: Query<&BodyAnim>,
    mut players: Query<(
        &PlayerOf,
        &mut Playing,
        &mut AnimationPlayer,
        &mut AnimationTransitions,
    )>,
) {
    if !rig.ready() {
        return;
    }
    let dt = time.delta_secs();
    for (owner, mut playing, mut player, mut transitions) in &mut players {
        let Ok(anim) = anims.get(owner.0) else {
            continue;
        };
        // The ranking lives on `BodyAnim` (`wants`, `wants_upper`) rather
        // than here, so a headless test can assert it without a `World`.
        let Some(want) = anim.wants() else { continue };
        // The sequence number is what lets the next one-shot restart the
        // clip: without it the `playing == want` guard would swallow every
        // swing after the first for as long as the body kept swinging.
        let restart = want.transient() && playing.base_seq != anim.transient_seq;
        if playing.base != Some(want) || restart {
            // Where the outgoing gait is in its stride, so the incoming one
            // starts on the same foot.
            let phase = playing.base.and_then(|from| {
                let mark = from.stride_mark()?;
                let t = player.animation(rig.node(from))?.seek_time();
                Some(t / rig.duration(from) - mark)
            });
            let first = playing.base.is_none();
            playing.fading = playing.base.filter(|c| c.stride_mark().is_some());
            playing.base = Some(want);
            playing.base_seq = anim.transient_seq;
            let d = rig.duration(want);
            let active = transitions.play(
                &mut player,
                rig.node(want),
                Duration::from_secs_f32(blend(want)),
            );
            // **`.repeat()` for a loop and nothing for the four one-shots.**
            // `RepeatAnimation::default()` is `Never`, so the one-shots need
            // no completion callback: their clocks run out and the next
            // frame's `want` is the gait again, and the death simply never
            // stops being wanted, so it holds its final pose.
            if !want.one_shot() {
                active.repeat();
                if let Some(mark) = want.stride_mark() {
                    let p = phase.unwrap_or_else(|| desync(owner.0));
                    active.set_seek_time((p + mark).rem_euclid(1.0) * d);
                } else if first {
                    active.set_seek_time(desync(owner.0) * d);
                }
            }
        }
        // The gait's rate, every frame: the body's speed changes under a
        // loop that does not, and backpedalling is the same loop reversed.
        if want.stride_mps().is_some() {
            let rate = anim.rate();
            if let Some(a) = player.animation_mut(rig.node(want)) {
                a.set_speed(rate);
            }
            // The loop it is fading from steps through its stride at the
            // same pace, so the two never disagree about which foot is down.
            if let Some(from) = playing.fading {
                let pace = rate / rig.duration(want) * rig.duration(from);
                match player.animation_mut(rig.node(from)) {
                    Some(a) if from != want => {
                        a.set_speed(pace);
                    }
                    _ => playing.fading = None,
                }
            }
        } else {
            playing.fading = None;
        }
        upper(&rig, anim, &mut playing, &mut player, dt);
    }
}

/// The upper-body layer: a one-shot on the arms and spine while the legs
/// keep their gait.
///
/// The node is masked off the hips and legs, so they only ever hear the
/// gait. The arms and spine hear both, and Bevy blends whatever reaches a
/// bone by weight: the gait layer's weights always sum to 1
/// (`AnimationTransitions` holds them there), so giving this node `u/(1−u)`
/// hands it exactly `u` of every upper bone. Not `AnimationTransitions`,
/// which owns one main clip and would fade the gait out under it.
fn upper(rig: &Rig, anim: &BodyAnim, playing: &mut Playing, player: &mut AnimationPlayer, dt: f32) {
    let want = anim.wants_upper();
    if let Some(c) = want {
        if playing.upper != want || playing.upper_seq != anim.transient_seq {
            if let Some(old) = playing.upper.and_then(|o| rig.upper_node(o)) {
                player.stop(old);
            }
            if let Some(node) = rig.upper_node(c) {
                player.start(node);
            }
            playing.upper = want;
            playing.upper_seq = anim.transient_seq;
        }
    }
    let Some(c) = playing.upper else {
        return;
    };
    // In at the one-shot's own pace, out at the general one.
    playing.upper_u = if want.is_some() {
        (playing.upper_u + dt / blend(c)).min(1.0)
    } else {
        (playing.upper_u - dt / ANIM_BLEND_S).max(0.0)
    };
    let Some(node) = rig.upper_node(c) else {
        playing.upper = None;
        return;
    };
    if want.is_none() && playing.upper_u <= 0.0 {
        player.stop(node);
        playing.upper = None;
        return;
    }
    if let Some(a) = player.animation_mut(node) {
        let u = playing.upper_u.min(UPPER_MAX);
        a.set_weight(u / (1.0 - u));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::platform::collections::HashMap;

    fn step(a: &mut BodyAnim, speed: f32, frames: usize) {
        let dt = 1.0 / 60.0;
        let mut p = a.last.unwrap_or(Vec3::ZERO);
        for _ in 0..frames {
            p.x += speed * dt;
            a.observe(p, dt, false, false, false);
        }
    }

    #[test]
    fn standing_still_is_idle() {
        let mut a = BodyAnim::default();
        step(&mut a, 0.0, 30);
        assert_eq!(a.clip, Some(Clip::Idle));
    }

    #[test]
    fn speed_picks_the_gait() {
        use sim_core::movement::{CROUCH_SPEED, SPRINT_SPEED, WADE_SPEED_MULT, WALK_SPEED};
        // Every speed the sim actually moves a body at, and the gait it is
        // drawn in. The ordinary pace was drawn as a walk and the sprint as a
        // jog until the thresholds were read off these constants.
        for (mps, crouched, want) in [
            (0.0, false, Clip::Idle),
            (WALK_SPEED * WADE_SPEED_MULT, false, Clip::Walk),
            (WALK_SPEED, false, Clip::Jog),
            (SPRINT_SPEED * WADE_SPEED_MULT, false, Clip::Jog),
            (SPRINT_SPEED, false, Clip::Sprint),
            (0.0, true, Clip::CrouchIdle),
            (CROUCH_SPEED * WADE_SPEED_MULT, true, Clip::CrouchWalk),
            (CROUCH_SPEED, true, Clip::CrouchWalk),
        ] {
            let mut a = BodyAnim {
                crouched,
                ..BodyAnim::default()
            };
            step(&mut a, mps, 120);
            assert_eq!(a.clip, Some(want), "at {mps} m/s, crouched={crouched}");
        }
    }

    #[test]
    fn a_gait_plays_at_the_rate_that_plants_its_feet() {
        use sim_core::movement::{SPRINT_SPEED, WALK_SPEED};
        for mps in [WALK_SPEED, SPRINT_SPEED] {
            let mut a = BodyAnim::default();
            step(&mut a, mps, 120);
            let stride = a.clip.and_then(Clip::stride_mps).unwrap();
            assert!(
                (a.rate() - mps / stride).abs() < 0.02,
                "{:?} at {mps} m/s plays at {} — its feet slide",
                a.clip,
                a.rate()
            );
        }
        // Standing and one-shots run at their authored rate.
        let mut a = BodyAnim::default();
        step(&mut a, 0.0, 30);
        assert_eq!(a.rate(), 1.0);
        // A crouch is clamped rather than scuttled.
        let mut a = BodyAnim {
            crouched: true,
            ..BodyAnim::default()
        };
        step(&mut a, sim_core::movement::CROUCH_SPEED, 120);
        assert_eq!(a.rate(), ANIM_RATE_MAX);
    }

    /// Move a body along a world bearing (sim yaw sense) while it faces
    /// `facing`.
    fn travel(a: &mut BodyAnim, facing: f32, bearing: f32, mps: f32, frames: usize) {
        let dt = 1.0 / 60.0;
        let mut p = a.last.unwrap_or(Vec3::ZERO);
        a.facing = facing;
        for _ in 0..frames {
            p.x += bearing.sin() * mps * dt;
            p.z += bearing.cos() * mps * dt;
            a.observe(p, dt, false, false, false);
        }
    }

    #[test]
    fn a_body_moving_backwards_backpedals() {
        let mut a = BodyAnim::default();
        travel(&mut a, 0.3, 0.3 + PI, 3.0, 120);
        assert_eq!(a.clip, Some(Clip::Jog));
        assert!(a.backward, "running away from its own facing, forwards");
        assert!(a.rate() < 0.0, "the loop is not reversed");
        assert!(
            a.hip_yaw.abs() < 0.01,
            "a straight backpedal twisted the legs"
        );
        // Diagonally back: the legs turn toward the travel, reversed.
        let mut a = BodyAnim::default();
        travel(&mut a, 0.0, PI - 0.5, 3.0, 120);
        assert!(a.backward);
        assert!((a.hip_yaw - -0.5).abs() < 0.01, "hip yaw {}", a.hip_yaw);
    }

    #[test]
    fn a_strafing_body_turns_its_legs_toward_the_travel() {
        let mut a = BodyAnim::default();
        travel(&mut a, 1.0, 1.6, 3.0, 120);
        assert!(!a.backward);
        assert!((a.hip_yaw - 0.6).abs() < 0.01, "hip yaw {}", a.hip_yaw);
        // Side-on: clamped, and still running forwards.
        let mut a = BodyAnim::default();
        travel(&mut a, 0.0, -FRAC_PI_2, 3.0, 120);
        assert!(!a.backward);
        assert!((a.hip_yaw + ANIM_TWIST_MAX).abs() < 0.01);
        // Stopping brings the legs back under the body.
        travel(&mut a, 0.0, 0.0, 0.0, 120);
        assert_eq!(a.clip, Some(Clip::Idle));
        travel(&mut a, 0.0, 0.0, 0.0, 120);
        assert!(
            a.hip_yaw.abs() < 0.03,
            "the legs stayed twisted: {}",
            a.hip_yaw
        );
    }

    #[test]
    fn a_standing_body_keeps_its_feet_until_it_turns_too_far() {
        let mut a = BodyAnim::default();
        travel(&mut a, 0.0, 0.0, 0.0, 30);
        // A small turn: the feet stay where they were.
        travel(&mut a, 0.4, 0.0, 0.0, 30);
        assert!((a.hip_yaw + 0.4).abs() < 1e-4, "hip yaw {}", a.hip_yaw);
        // Past the limit they step back round, all the way.
        travel(&mut a, 1.4, 0.0, 0.0, 60);
        assert!(a.hip_yaw.abs() < 0.03, "hip yaw {}", a.hip_yaw);
        // Turning across ±π is a short turn, not a long one.
        let mut a = BodyAnim::default();
        travel(&mut a, PI - 0.1, 0.0, 0.0, 30);
        travel(&mut a, -PI + 0.1, 0.0, 0.0, 1);
        assert!((a.hip_yaw + 0.2).abs() < 1e-3, "hip yaw {}", a.hip_yaw);
    }

    #[test]
    fn a_one_shot_over_moving_legs_plays_on_the_upper_body() {
        let mut a = BodyAnim::default();
        step(&mut a, 3.0, 120);
        a.swing();
        assert_eq!(a.wants(), Some(Clip::Jog), "the legs stopped running");
        assert_eq!(a.wants_upper(), Some(Clip::Swing));
        // Stopping mid-swing does not move it to the other layer.
        step(&mut a, 0.0, 30);
        assert_eq!(a.wants(), Some(Clip::Idle));
        assert_eq!(a.wants_upper(), Some(Clip::Swing));
        // Standing still, a one-shot owns the whole body: the lunge has feet.
        let mut a = BodyAnim::default();
        step(&mut a, 0.0, 30);
        a.swing();
        assert_eq!(a.wants(), Some(Clip::Swing));
        assert_eq!(a.wants_upper(), None);
    }

    #[test]
    fn angles_wrap_the_short_way() {
        for (a, w) in [
            (0.0, 0.0),
            (PI + 0.1, -PI + 0.1),
            (-PI - 0.1, PI - 0.1),
            (5.0 * TAU + 0.2, 0.2),
        ] {
            assert!(
                (wrap_pi(a) - w).abs() < 1e-4,
                "wrap_pi({a}) = {}",
                wrap_pi(a)
            );
        }
        assert!((wrap_pi(PI) - PI).abs() < 1e-6);
    }

    #[test]
    fn a_body_on_a_threshold_does_not_flicker() {
        // The defect this exists for: derived speed sitting exactly on a
        // boundary alternated clips every frame, which reads as a body
        // vibrating between two gaits.
        let mut a = BodyAnim::default();
        step(&mut a, ANIM_JOG_MPS, 120);
        let settled = a.clip;
        let mut changes = 0;
        let dt = 1.0 / 60.0;
        let mut p = a.last.unwrap();
        for i in 0..240 {
            // Jitter across the threshold, the way a late packet does.
            let wobble = if i % 2 == 0 { 0.2 } else { -0.2 };
            p.x += (ANIM_JOG_MPS + wobble) * dt;
            let before = a.clip;
            a.observe(p, dt, false, false, false);
            if a.clip != before {
                changes += 1;
            }
        }
        assert_eq!(a.clip, settled);
        assert_eq!(changes, 0, "clip changed {changes} times on a threshold");
    }

    #[test]
    fn a_sleeper_is_a_sleeper_whatever_its_speed() {
        let mut a = BodyAnim::default();
        step(&mut a, 6.0, 60);
        a.observe(a.last.unwrap(), 1.0 / 60.0, true, false, false);
        assert_eq!(a.clip, Some(Clip::Sleep));
    }

    #[test]
    fn a_body_in_the_air_is_in_the_air_until_it_lands() {
        let mut a = BodyAnim::default();
        step(&mut a, 4.2, 120);
        a.airborne = true;
        step(&mut a, 4.2, 10);
        assert_eq!(a.wants(), Some(Clip::Air));
        // A swing mid-jump is still a swing, over the jump's legs; a corpse
        // is still a corpse.
        a.swing();
        assert_eq!(a.wants(), Some(Clip::Air));
        assert_eq!(a.wants_upper(), Some(Clip::Swing));
        a.observe(a.last.unwrap(), 1.0 / 60.0, false, true, false);
        assert_eq!(a.wants(), Some(Clip::Death));
        assert_eq!(a.wants_upper(), None);
        let mut a = BodyAnim::default();
        step(&mut a, 4.2, 120);
        a.airborne = true;
        step(&mut a, 4.2, 20);
        a.airborne = false;
        step(&mut a, 4.2, 1);
        assert_eq!(a.clip, Some(Clip::Jog), "landed into the wrong gait");
    }

    #[test]
    fn a_crouched_body_swings_without_standing_up() {
        let mut a = BodyAnim {
            crouched: true,
            ..BodyAnim::default()
        };
        step(&mut a, 0.0, 30);
        assert_eq!(a.wants(), Some(Clip::CrouchIdle));
        step(&mut a, 1.7, 60);
        assert_eq!(a.wants(), Some(Clip::CrouchWalk));
        // The swing goes on the upper body; the legs stay crouched.
        a.swing();
        assert_eq!(a.wants(), Some(Clip::CrouchWalk));
        assert_eq!(a.wants_upper(), Some(Clip::Swing));
        // A whole-body swing begun standing is ended by a crouch rather than
        // standing the body back up for its span.
        let mut a = BodyAnim::default();
        step(&mut a, 0.0, 30);
        a.swing();
        assert_eq!(a.wants(), Some(Clip::Swing));
        a.crouched = true;
        step(&mut a, 0.0, 1);
        assert_eq!(a.wants(), Some(Clip::CrouchIdle));
        a.crouched = false;
        step(&mut a, 0.0, 1);
        assert_eq!(a.wants(), Some(Clip::Idle), "standing up replayed the tail");
        // A crouched corpse is a corpse.
        a.crouched = true;
        a.observe(a.last.unwrap(), 1.0 / 60.0, false, true, false);
        assert_eq!(a.wants(), Some(Clip::Death));
    }

    #[test]
    fn a_corpse_is_a_corpse_whatever_else_is_true() {
        // The defect this exists for: before wire v48 nothing on a remote's
        // record said it had been killed, so a body that a player had just
        // shot went on jogging or standing at idle until its owner left the
        // death screen. Death has to outrank BOTH the gait and the sleeper
        // flag — a sleeper who is killed is a corpse, which is the order the
        // sim itself takes (`hp == 0` is what every weapon tests, and `die`
        // carries `sleeping` forward untouched).
        for (mps, sleeping) in [(0.0, false), (6.0, false), (0.0, true), (6.0, true)] {
            let mut a = BodyAnim::default();
            let dt = 1.0 / 60.0;
            let mut p = a.last.unwrap_or(Vec3::ZERO);
            for _ in 0..120 {
                p.x += mps * dt;
                a.observe(p, dt, sleeping, true, false);
            }
            assert_eq!(
                a.clip,
                Some(Clip::Death),
                "at {mps} m/s with sleeping={sleeping}"
            );
        }
    }

    #[test]
    fn a_body_that_respawns_walks_again() {
        // The other half, and the one a `dead` latch would have broken: the
        // bit clears when the player leaves the death screen and the body is
        // theirs again. A corpse that stayed a corpse would be a player
        // sliding around the island face-down.
        let mut a = BodyAnim::default();
        let dt = 1.0 / 60.0;
        let mut p = Vec3::ZERO;
        for _ in 0..60 {
            a.observe(p, dt, false, true, false);
        }
        assert_eq!(a.clip, Some(Clip::Death));
        for _ in 0..120 {
            p.x += 4.0 * dt;
            a.observe(p, dt, false, false, false);
        }
        assert_eq!(a.clip, Some(Clip::Jog));
    }

    #[test]
    fn death_stops_a_swing_in_flight() {
        // `wants` reads the gait first for this: a body killed mid-stroke
        // has a one-shot still running (`swing_s` counts down on a clock
        // nothing else resets), and the only ordering that does not draw a
        // dead man finishing his punch is death over swing over gait.
        let mut a = BodyAnim::default();
        a.observe(Vec3::ZERO, 1.0 / 60.0, false, false, false);
        a.swing();
        assert!(a.swing_s > 0.0);
        a.observe(Vec3::ZERO, 1.0 / 60.0, false, true, false);
        assert_eq!(a.clip, Some(Clip::Death));
        // The clock is still running — the point is that the ranking prefers
        // the corpse anyway. **This is the real function `drive` calls**, not
        // a copy of its logic: the copy is what this test used to be.
        assert!(a.swing_s > 0.0, "the swing clock is not what death clears");
        assert_eq!(a.wants(), Some(Clip::Death), "a corpse was drawn swinging");
    }

    #[test]
    fn a_corpse_does_not_flinch() {
        // The other half of the same rule, for the second transient: a body
        // that takes a posthumous arrow is still a corpse. `EV_HIT` on a
        // dead body is a live case rather than a hypothetical — `combat`
        // and `ranged` skip `hp == 0`, but a blow already in flight when
        // the victim died lands on the same tick the corpse appears.
        let mut a = BodyAnim::default();
        a.observe(Vec3::ZERO, 1.0 / 60.0, false, true, false);
        a.flinch();
        assert!(a.flinch_s > 0.0, "the flinch clock was not started");
        assert_eq!(a.wants(), Some(Clip::Death), "a corpse flinched");
    }

    #[test]
    fn a_flinch_outranks_a_swing_and_stops_its_clock() {
        // The ranking argument is in `BodyAnim::flinch`'s doc comment; this
        // is the arithmetic half of it. The clock-stopping is the part that
        // is not about taste: a swing left counting down under a flinch is
        // replayed FROM FRAME ZERO the moment the flinch expires, which is
        // an arc that shows its first tenth of a second, disappears for a
        // third of one, and then starts over.
        let mut a = BodyAnim::default();
        a.observe(Vec3::ZERO, 1.0 / 60.0, false, false, false);
        a.swing();
        let seq = a.transient_seq;
        a.flinch();
        assert_eq!(a.wants(), Some(Clip::Flinch));
        assert_eq!(a.swing_s, 0.0, "the interrupted arc will restart");
        assert_ne!(a.transient_seq, seq, "the clip will not restart");

        // Run the flinch out: the gait comes back, not the swing.
        for _ in 0..(60.0 * FLINCH_CLIP_S) as usize + 2 {
            a.observe(Vec3::ZERO, 1.0 / 60.0, false, false, false);
        }
        assert_eq!(a.wants(), Some(Clip::Idle), "the dead arc came back");
    }

    #[test]
    fn a_swing_after_a_flinch_wins_the_slot() {
        // Symmetric, and it has to be: a swing is a fact every client on
        // the shard is told about, so an attacker whose flinch swallowed it
        // for a third of a second would be watching a different fight from
        // everyone else. Newest wins.
        let mut a = BodyAnim::default();
        a.observe(Vec3::ZERO, 1.0 / 60.0, false, false, false);
        a.flinch();
        a.swing();
        assert_eq!(a.wants(), Some(Clip::Swing));
        assert_eq!(a.flinch_s, 0.0, "the flinch clock kept running");
    }

    #[test]
    fn a_shot_plays_once_and_yields_to_a_flinch() {
        let mut a = BodyAnim::default();
        a.observe(Vec3::ZERO, 1.0 / 60.0, false, false, false);
        let seq = a.transient_seq;
        a.shoot();
        assert_eq!(a.wants(), Some(Clip::Shoot));
        assert_ne!(
            a.transient_seq, seq,
            "a second shot would not restart the clip"
        );
        a.flinch();
        assert_eq!(a.wants(), Some(Clip::Flinch));
        assert_eq!(
            a.shoot_s, 0.0,
            "the shot's clock kept running under the flinch"
        );
        a.shoot();
        for _ in 0..(SHOOT_CLIP_S * 60.0).ceil() as usize {
            a.observe(Vec3::ZERO, 1.0 / 60.0, false, false, false);
        }
        assert_eq!(a.wants(), Some(Clip::Idle), "the shot outlived its clip");
    }

    #[test]
    fn a_flinch_expires_on_its_own_clock() {
        // No completion callback anywhere: the clip stops being wanted
        // because `observe` counted its clock down, which is the same shape
        // the swing uses and the reason neither needs `.repeat()`.
        let mut a = BodyAnim::default();
        a.observe(Vec3::ZERO, 1.0 / 60.0, false, false, false);
        a.flinch();
        let frames = (FLINCH_CLIP_S * 60.0).ceil() as usize;
        for i in 0..frames {
            assert_eq!(a.wants(), Some(Clip::Flinch), "flinch ended at frame {i}");
            a.observe(Vec3::ZERO, 1.0 / 60.0, false, false, false);
        }
        assert_eq!(a.wants(), Some(Clip::Idle), "the flinch outlived its clip");
    }

    #[test]
    fn a_sleeper_flinches() {
        // A sleeper is a gait (`Clip::Sleep`), so a transient outranks it —
        // and this is the one place that matters most: hitting a sleeping
        // body is otherwise the least legible attack in the game, because
        // nothing about the target changes at all.
        let mut a = BodyAnim::default();
        a.observe(Vec3::ZERO, 1.0 / 60.0, true, false, false);
        assert_eq!(a.clip, Some(Clip::Sleep));
        a.flinch();
        assert_eq!(a.wants(), Some(Clip::Flinch));
    }

    #[test]
    fn the_flinch_blends_in_faster_than_anything_else() {
        // `FLINCH_BLEND_S`'s reason, as a rule rather than as a paragraph:
        // the clip is 0.333 s long and the general 0.18 s cross-fade would
        // still be running at its apex. `tests/rig_asset.rs` is where the
        // number is checked against the file; this is where the ordering is.
        assert!(blend(Clip::Flinch) < blend(Clip::Swing));
        assert!(blend(Clip::Flinch) < FLINCH_CLIP_S);
        for c in Clip::ALL {
            assert!(
                blend(c) >= blend(Clip::Flinch),
                "{c:?} blends in faster than the flinch"
            );
        }
    }

    #[test]
    fn every_clip_has_a_distinct_slot() {
        // A duplicated slot would make two clips share a graph node and one of
        // them would silently never play.
        let mut seen = HashMap::new();
        for c in Clip::ALL {
            assert!(seen.insert(c.slot(), c).is_none(), "slot clash on {c:?}");
        }
        assert_eq!(seen.len(), Clip::ALL.len());
    }
}
