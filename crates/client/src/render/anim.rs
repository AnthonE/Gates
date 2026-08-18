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

use std::time::Duration;

use bevy::gltf::Gltf;
use bevy::prelude::*;

/// The clips this game actually asks for. The library ships 46; the rest are
/// for games that drive, cast spells or hold pistols (`assets/models/
/// MANIFEST.md` says why they are not trimmed out of the file).
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
}

impl Clip {
    /// The name in the glTF. Resolved through `Gltf::named_animations`.
    fn name(self) -> &'static str {
        match self {
            Clip::Idle => "Idle_Loop",
            Clip::Walk => "Walk_Loop",
            Clip::Jog => "Jog_Fwd_Loop",
            Clip::Sprint => "Sprint_Loop",
            // Deliberately not `A_TPose`: a sleeper is a person standing
            // still, and the T-pose is a rig artifact.
            Clip::Sleep => "Idle_Loop",
            // **`Punch_Cross` and not `Sword_Attack`, and the reason is
            // arithmetic rather than taste.** `SWING_INTERVAL_TICKS` is 38
            // at `TICK_HZ` 30, so the sim lets a player swing every
            // 1.267 s; `Sword_Attack` runs 1.500 s and with `ANIM_BLEND_S`
            // on top is 1.68 s, so a player swinging as fast as the rules
            // allow would have every arc cut off by the next one and the
            // body would never finish a stroke. `Punch_Cross` is 1.000 s,
            // 1.18 s with the blend, and fits inside the cadence with room
            // to return to the gait. **It is a punch and the operator asked
            // for a rock swing** — `DECISIONS.md` §open carries that as the
            // open question, because the honest alternatives both need a
            // word: accept the punch, or shorten the sword clip.
            Clip::Swing => "Punch_Cross",
        }
    }

    const ALL: [Clip; 6] = [
        Clip::Idle,
        Clip::Walk,
        Clip::Jog,
        Clip::Sprint,
        Clip::Sleep,
        Clip::Swing,
    ];

    fn slot(self) -> usize {
        match self {
            Clip::Idle => 0,
            Clip::Walk => 1,
            Clip::Jog => 2,
            Clip::Sprint => 3,
            Clip::Sleep => 4,
            Clip::Swing => 5,
        }
    }
}

/// How long the one-shot swing clip runs, seconds — `Punch_Cross`'s own
/// length in the shipped glTF, not a picked number.
pub const SWING_CLIP_S: f32 = 1.0;

/// Speed thresholds, m/s, and the band around each that a body must cross to
/// change its mind. Derived speed is noisy — a packet arriving a millisecond
/// late reads as a lurch — so a bare `>` at a boundary makes a body alternate
/// clips every frame. The hysteresis is the cheapest fix and the only one that
/// does not add latency.
pub const ANIM_WALK_MPS: f32 = 0.6;
pub const ANIM_JOG_MPS: f32 = 3.0;
pub const ANIM_SPRINT_MPS: f32 = 5.4;
/// Half-width of the dead band around each threshold, m/s.
pub const ANIM_SPEED_HYSTERESIS: f32 = 0.35;

/// How long a clip change takes to cross-fade, seconds. Long enough that a
/// walk→jog is not a snap, short enough that it is not a slide.
pub const ANIM_BLEND_S: f32 = 0.18;

/// The rig's own height, metres — **measured off the shipped file, not
/// assumed and not measured at runtime.**
///
/// `mannequin.gltf`'s POSITION accessor bounds are `min.y 0.00046`, `max.y
/// 1.8292`: the rig stands 1.829 m with its feet on its own origin. The sim's
/// player is `Capsule3d::new(0.4, 1.0)` — 1.0 of cylinder plus two 0.4 caps,
/// so **1.8 m** — which means the library is within 2% of our own body and the
/// correct scale is 1.
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
pub const ANIM_RIG_H_M: f32 = 1.829;
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
    nodes: [AnimationNodeIndex; 6],
    /// Uniform scale that puts the rig at [`ANIM_BODY_H_M`]. A constant ratio
    /// of two measured heights, not a runtime fit — see [`ANIM_RIG_H_M`].
    pub scale: f32,
    /// Names the glTF did not have. Non-empty means the library was re-vendored
    /// and a clip was renamed — reported once, loudly, rather than silently
    /// falling back to idle forever.
    missing: Vec<&'static str>,
}

impl Rig {
    /// True once the glTF has loaded and the graph is built. `bodies::stream`
    /// waits on this: spawning a body before it would spawn one with no scene.
    pub fn ready(&self) -> bool {
        self.scene.is_some() && self.graph.is_some()
    }
    fn node(&self, c: Clip) -> AnimationNodeIndex {
        self.nodes[c.slot()]
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
    commands.insert_resource(BodyShades {
        awake: materials.add(StandardMaterial {
            base_color: Color::srgb(0.42, 0.36, 0.28),
            perceptual_roughness: 0.75,
            ..default()
        }),
        sleeping: materials.add(StandardMaterial {
            // Colder and darker than the waking body, not brighter: a sleeper
            // is the thing you sneak up on, and making it the most legible
            // object on the beach would hand the raider more than the wire
            // does.
            base_color: Color::srgb(0.24, 0.26, 0.30),
            perceptual_roughness: 0.9,
            ..default()
        }),
    });
    commands.insert_resource(Rig {
        gltf: assets.load("models/mannequin.gltf"),
        scene: None,
        graph: None,
        nodes: [AnimationNodeIndex::default(); 6],
        scale: ANIM_BODY_H_M / ANIM_RIG_H_M,
        missing: Vec::new(),
    });
}

/// Build the graph once the glTF is in. Runs every frame until it succeeds,
/// then costs one `ready()` branch.
pub fn build(
    mut rig: ResMut<Rig>,
    gltfs: Res<Assets<Gltf>>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
) {
    if rig.ready() {
        return;
    }
    let Some(gltf) = gltfs.get(&rig.gltf) else {
        return;
    };

    let mut graph = AnimationGraph::new();
    let root = graph.root;
    let mut nodes = [AnimationNodeIndex::default(); 6];
    let mut missing = Vec::new();
    for clip in Clip::ALL {
        match gltf.named_animations.get(clip.name()) {
            Some(h) => nodes[clip.slot()] = graph.add_clip(h.clone(), 1.0, root),
            None => missing.push(clip.name()),
        }
    }

    if !missing.is_empty() {
        error!(
            "anim: {} clip name(s) missing from models/mannequin.gltf: {:?} — \
             the library was re-vendored and a clip was renamed",
            missing.len(),
            missing
        );
    }
    rig.missing = missing;
    rig.nodes = nodes;
    rig.graph = Some(graphs.add(graph));
    rig.scene = gltf
        .default_scene
        .clone()
        .or_else(|| gltf.scenes.first().cloned());
}

/// What a body wants to be playing. Written by `bodies::stream` off state the
/// sim sent; read by [`drive`]. A component and not a resource because it is
/// per-body, and render state rather than sim state because the sim has no
/// opinion about which clip anybody is in.
#[derive(Component, Default)]
pub struct BodyAnim {
    pub clip: Option<Clip>,
    /// Metres per second, low-passed. Public so `bodies` can integrate it.
    pub speed: f32,
    /// Last interpolated position, for the difference.
    pub last: Option<Vec3>,
    /// Seconds left of a one-shot swing.
    ///
    /// **Beside the gait rather than inside `clip`, and that is the whole
    /// design.** `observe` recomputes `clip` from speed every single frame,
    /// so a one-shot written there would be stomped the next one — the
    /// transient has to live in a field nothing else recomputes.
    pub swing_s: f32,
    /// Bumped once per swing heard. `drive` compares it against what it
    /// last started, so a second swing arriving while the first arc is
    /// still playing restarts the stroke instead of being swallowed by the
    /// `playing == want` guard.
    pub swing_seq: u32,
}

impl BodyAnim {
    /// Fold a new sample in and choose a clip. `dt` is the frame's delta.
    ///
    /// The low pass is on the SPEED and not on the position: smoothing the
    /// position would fight the interpolator, which is already the authority
    /// on where the body is (`bodies.rs` header).
    pub fn observe(&mut self, pos: Vec3, dt: f32, sleeping: bool) {
        // The one-shot's clock, run here because this is the one function
        // every live body passes through every frame with a `dt` in hand.
        // `bodies::stream` calls this BEFORE it hears the frame's swings,
        // so a swing heard this frame gets its whole span.
        self.swing_s = (self.swing_s - dt).max(0.0);
        if let (Some(last), true) = (self.last, dt > 0.0) {
            // Horizontal only. A body riding terrain up a hill is walking, not
            // climbing, and counting the vertical would read a slope as speed.
            let step = pos - last;
            let raw = Vec2::new(step.x, step.z).length() / dt;
            // One-pole, fixed per-second constant so the smoothing does not
            // change with the frame rate.
            let k = 1.0 - (-12.0 * dt).exp();
            self.speed += (raw - self.speed) * k;
        }
        self.last = Some(pos);

        if sleeping {
            self.clip = Some(Clip::Sleep);
            return;
        }
        // Hysteresis: the threshold to speed UP is above the nominal and the
        // one to slow DOWN is below it, so a body sitting on a boundary keeps
        // whatever it already had.
        let h = ANIM_SPEED_HYSTERESIS;
        let now = self.clip.unwrap_or(Clip::Idle);
        let rank = |c: Clip| match c {
            Clip::Sprint => 3,
            Clip::Jog => 2,
            Clip::Walk => 1,
            _ => 0,
        };
        let up = |t: f32| self.speed > t + h;
        let down = |t: f32| self.speed < t - h;
        let want = if up(ANIM_SPRINT_MPS) || (rank(now) >= 3 && !down(ANIM_SPRINT_MPS)) {
            Clip::Sprint
        } else if up(ANIM_JOG_MPS) || (rank(now) >= 2 && !down(ANIM_JOG_MPS)) {
            Clip::Jog
        } else if up(ANIM_WALK_MPS) || (rank(now) >= 1 && !down(ANIM_WALK_MPS)) {
            Clip::Walk
        } else {
            Clip::Idle
        };
        self.clip = Some(want);
    }

    /// Start a one-shot swing on this body.
    pub fn swing(&mut self) {
        self.swing_s = SWING_CLIP_S;
        self.swing_seq = self.swing_seq.wrapping_add(1);
    }
}

/// A body whose descendants still need our materials painted onto them.
///
/// **Two things make this necessary and neither is optional.** The library's
/// own materials are `M_Main` and `M_Joints` — the yellow-and-purple preview
/// mannequin — so a body spawned untouched arrives in another game's colours.
/// And `bodies.rs` distinguishes a sleeper from a waking player by SHADE, which
/// it calls load-bearing in its own comment: "is that player about to shoot me,
/// or is nobody home" is the question sleepers create, and a client that draws
/// both identically makes it unanswerable. A `SceneRoot` has no material of its
/// own, so both have to reach the spawned descendants.
///
/// Carried as a component and cleared when it lands, because **the scene
/// spawns asynchronously**: at the frame `bodies::stream` inserts this there
/// are no descendants yet, and a one-shot paint would silently miss every body.
#[derive(Component)]
pub struct Reshade(pub bool);

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
        let mat = if want.0 {
            waking.sleeping.clone()
        } else {
            waking.awake.clone()
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
pub struct Playing(Option<Clip>, u32);

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

/// Cross-fade each player to whatever its body wants.
pub fn drive(
    rig: Res<Rig>,
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
    for (owner, mut playing, mut player, mut transitions) in &mut players {
        let Ok(anim) = anims.get(owner.0) else {
            continue;
        };
        // The one-shot outranks the gait while it is running, and the
        // sequence number is what lets a second swing restart the stroke:
        // without it the `playing == want` guard below would swallow every
        // swing after the first for as long as the body kept swinging.
        let swinging = anim.swing_s > 0.0;
        let want = if swinging {
            Clip::Swing
        } else {
            let Some(c) = anim.clip else { continue };
            c
        };
        let restart = swinging && playing.1 != anim.swing_seq;
        if playing.0 == Some(want) && !restart {
            continue;
        }
        playing.0 = Some(want);
        playing.1 = anim.swing_seq;
        let active = transitions.play(
            &mut player,
            rig.node(want),
            Duration::from_secs_f32(ANIM_BLEND_S),
        );
        // **`.repeat()` for a gait and nothing for the swing.**
        // `RepeatAnimation::default()` is `Never`, so the one-shot is an
        // omission rather than a feature — and the return to the gait needs
        // no completion callback either, because `swing_s` runs out and the
        // next frame's `want` is the gait again.
        if want != Clip::Swing {
            active.repeat();
        }
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
            a.observe(p, dt, false);
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
        for (mps, want) in [
            (0.0, Clip::Idle),
            (2.0, Clip::Walk),
            (4.2, Clip::Jog),
            (7.0, Clip::Sprint),
        ] {
            let mut a = BodyAnim::default();
            step(&mut a, mps, 120);
            assert_eq!(a.clip, Some(want), "at {mps} m/s");
        }
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
            a.observe(p, dt, false);
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
        a.observe(a.last.unwrap(), 1.0 / 60.0, true);
        assert_eq!(a.clip, Some(Clip::Sleep));
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
