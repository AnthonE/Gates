//! The Bevy half of the client's audio: the bank, the listener, the voices,
//! and the two systems that turn what happened into what is heard.
//!
//! **Bevy plays; it does not decide.** Every level, every cull and every
//! cadence in here came out of `crate::sound`, which is pure and tested
//! headless. This file owns four things and no others: loading the bank into
//! `Assets<AudioSource>`, putting a [`SpatialListener`] on the camera,
//! spawning an entity per voice the mixer asked for, and holding the one
//! looping entity that is the bed.
//!
//! ## The rodio finding, and why `SPATIAL_SCALE` is what it is
//!
//! **Bevy's spatial audio attenuates by `min(1/d², 1)` in world units, and in
//! a metre-scale world that makes everything past ~5 m inaudible.** It is
//! rodio's `Spatial::set_positions`: the per-ear gain is a panning term in
//! [0.5, 1.0] multiplied by `(1/dist_sq).min(1.0)`. At 10 m that second term
//! is 1/100 — 40 dB down — and at 30 m it is −59 dB. A tree falling 40 m away
//! would be silent, and no amount of turning the cue's gain up fixes it,
//! because the law is inverse-square and our falloff is not.
//!
//! The fix is not to fight it: it is to **clamp it out and own the law**.
//! Bevy scales the emitter *and* both ears by `DefaultSpatialScale` before
//! handing them to rodio, so a scale of `1/128` puts every audible emitter
//! (nothing carries past `sound::MAX_AUDIBLE_M` = 96 m) inside one scaled
//! unit, where `(1/dist_sq).min(1.0)` is exactly 1.0. What survives is the
//! panning term — which is scale-invariant, because it is a ratio of ear
//! distances over the ear gap, and both scale together. So:
//!
//! - **rodio pans.** That is all it does.
//! - **`sound::falloff` attenuates**, through `PlaybackSettings::volume`, and
//!   it is the only distance law in the client.
//!
//! Getting this wrong is not a subtle bug — it is "the game has no sound past
//! a few metres" — and nothing in the API says so.

use bevy::audio::{
    AudioSource, DefaultSpatialScale, PlaybackMode, SpatialListener, SpatialScale, Volume,
};
use bevy::prelude::*;

use crate::sound::mixer::{Mixer, Request, Start};
use crate::sound::steps::Steps;
use crate::sound::{synth, Cue, Mix, CUE_COUNT, MAX_AUDIBLE_M, VOICE_CAP};

use super::rig::EyeCam;
use super::{Eye, Net};

/// The scale handed to rodio, so its own inverse-square law clamps to 1 for
/// every audible emitter and only its panning survives. See the header.
///
/// 128 rather than 96 (`MAX_AUDIBLE_M`) because rodio measures from each EAR,
/// not from the listener's centre — at the far edge of the radius an ear is
/// half the ear gap further out, and the margin keeps the clamp exact rather
/// than approximately exact.
pub const SPATIAL_SCALE: f32 = 1.0 / 128.0;

/// Distance between the listener's ears, metres. Bevy defaults to **4.0**,
/// which is a head the size of a car; a real one is ~0.22 m. It changes the
/// panning curve's sharpness, not its range (`DECISIONS.md` §open, audio v0).
pub const EAR_GAP_M: f32 = 0.22;

/// How fast the bed's gain follows the world, per second. A bed that snapped
/// would click every time the tree count under the camera changed by one.
pub const BED_FADE_PER_S: f32 = 0.5;

/// The generated bank: one `AudioSource` per [`Cue`], in `Cue::ALL` order.
#[derive(Resource)]
pub struct Bank {
    handles: [Handle<AudioSource>; CUE_COUNT],
}

impl Bank {
    pub fn get(&self, cue: Cue) -> Handle<AudioSource> {
        self.handles[cue.idx()].clone()
    }
}

/// What anything in the client asks for a sound through.
///
/// A resource rather than a Bevy `Event` on purpose: the queue is **bounded**
/// (`crate::sound::CUE_QUEUE_CAP`) with a stated overflow policy, which is
/// `CLAUDE.md` wall 4, and Bevy's event buffers are unbounded within a frame.
/// Every caller here is ultimately a packet or a keystroke.
#[derive(Resource, Default)]
pub struct Sound {
    pub mixer: Mixer,
    pub steps: Steps,
    /// The bed's current gain, moving toward its target at
    /// [`BED_FADE_PER_S`]. Held rather than recomputed so the crossfade is
    /// state, not a function of a frame.
    bed_gain: f32,
    bed_target: f32,
}

impl Sound {
    /// Ask for a cue. The one door — see [`Sound`].
    pub fn play(&mut self, req: Request) {
        self.mixer.push(req);
    }
}

/// A sounding voice. Counted every frame so the mixer knows its own headroom.
#[derive(Component)]
pub struct Voice;

/// The bed's single looping entity.
#[derive(Component)]
pub struct Bed;

/// Build the bank, at plugin-build time rather than in a schedule.
///
/// **`Startup` is too late, and finding out cost a capture run.** Bevy
/// schedules the first state transition with
/// `insert_startup_before(PreStartup, StateTransition)` — so on a start that
/// opens directly on `Screen::Loading` (which is every `--capture` and every
/// `--server` launch) **`OnEnter(Loading)` runs BEFORE `Startup`**. The bank
/// was a `Startup` system and [`setup`] takes `Res<Bank>`; the first probe run
/// after the audio slice died on *"Parameter failed validation: Resource does
/// not exist"* with the system name compiled out.
///
/// Building it here removes the ordering question rather than answering it:
/// the resource exists before any schedule runs at all. The cost is the same
/// ~1.5 MB of arithmetic, paid a few milliseconds earlier.
///
/// The hazard is general and this file is not the only place it can bite —
/// anything hung on `OnEnter(Loading)` that reads a `Startup`-inserted
/// resource has the same bug. `textures::load` is a `Startup` system today and
/// gets away with it only because nothing reads `Textures` until `Update`.
pub fn build_bank(app: &mut App) {
    let wavs = synth::bank();
    let handles = {
        let mut sources = app.world_mut().resource_mut::<Assets<AudioSource>>();
        wavs.map(|bytes| {
            sources.add(AudioSource {
                bytes: bytes.into(),
            })
        })
    };
    app.insert_resource(Bank { handles });
    app.insert_resource(DefaultSpatialScale(SpatialScale::new(SPATIAL_SCALE)));
    debug_assert!(clamp_holds(), "audio: SPATIAL_SCALE lets rodio attenuate");
}

/// Put the ears on the camera, and start the bed.
///
/// Runs on entering `Screen::Loading` after `rig::setup`, like the HUD and the
/// sky: the listener is the camera, and there is no camera before the rig.
pub fn setup(mut commands: Commands, bank: Res<Bank>, cam: Query<Entity, With<EyeCam>>) {
    let Ok(cam) = cam.single() else {
        return;
    };
    commands.entity(cam).insert(SpatialListener::new(EAR_GAP_M));

    // The bed starts SILENT and fades in. Entering a world at full ambience
    // on the first frame of the loading screen is the audio version of the
    // world popping in, and the fade is already the mechanism.
    commands.spawn((
        super::WorldEntity,
        Bed,
        AudioPlayer(bank.get(Cue::BedWind)),
        PlaybackSettings {
            mode: PlaybackMode::Loop,
            volume: Volume::SILENT,
            ..default()
        },
    ));
}

/// Reset what the world owned. The bed and the listener go with the camera
/// (`WorldEntity`), but the step odometer is in a resource that outlives the
/// world — and a stale one measures the distance between two worlds as ground
/// covered and fires a burst of footsteps on the first frame of the next.
pub fn teardown(mut sound: ResMut<Sound>, mut last_hp: ResMut<LastHp>) {
    sound.steps.reset();
    sound.bed_gain = 0.0;
    sound.bed_target = 0.0;
    // The same rule for health: a stale `LastHp` would read the next world's
    // first health message as a fall from the last world's and play a hurt
    // sound to a player who just joined.
    last_hp.0 = 0;
}

/// Footsteps, from the predictor.
///
/// Runs where the world runs rather than only `InWorld`: a player reading the
/// settings pane is standing still, so this produces nothing, and gating it on
/// the screen would only mean the odometer misses the frames a panel was open
/// and then fires for them all at once when it closes.
pub fn steps(
    net: NonSend<Net>,
    world: Res<super::WorldId>,
    mut sound: ResMut<Sound>,
    time: Res<Time>,
) {
    let body = &net.session.core.predict.body;
    let pos = net.session.core.predict.render_position();
    let Some(step) = sound.steps.sample(pos, body.grounded, time.delta_secs()) else {
        return;
    };
    // The same `splat` the ground under the player is DRAWN with, so the
    // sound cannot disagree with the picture — see `sound::steps`.
    let splat = sim_core::terrain::splat(world.seed, pos[0], pos[2]);
    let below_sea = pos[1] < sim_core::terrain::SEA_LEVEL;
    let cue = crate::sound::steps::surface_cue(splat, below_sea);
    sound.play(Request::own(cue).with_gain(step.gain));
}

/// Drain the client core's own-fact queues into cues.
///
/// **These are destructive pops and this is currently their only reader.**
/// `ClientCore::pop_hit` and friends hand a fact over exactly once, so when
/// the HUD grows a hitmarker (`NOW.md`) the two must not both pop — the second
/// reader would silently get half the events. The fix at that point is one
/// drain writing a per-frame resource both read, not a second `pop_` call
/// site, and this comment is the warning that the shape is load-bearing.
pub fn feed(mut net: NonSendMut<Net>, mut sound: ResMut<Sound>) {
    let core = &mut net.session.core;
    while core.pop_hit().is_some() {
        sound.play(Request::own(Cue::Hit));
    }
    while let Some(who) = core.pop_death() {
        // Someone else dying is not your own fact and has no position on this
        // wire, so only your own death makes a sound. A death rattle for a
        // player across the island would be a lie about where they are.
        if who == core.player_id {
            sound.play(Request::own(Cue::Death));
        }
    }
    while core.pop_toast().is_some() {
        sound.play(Request::own(Cue::Gather));
    }
    while core.pop_craft_toast().is_some() {
        sound.play(Request::own(Cue::CraftDone));
    }
    // Three refusal queues, one sound. A player does not need to hear the
    // difference between a refused craft and a refused placement — the panel
    // already says which — they need to hear that the button did nothing.
    let refused = core.pop_craft_refusal().is_some()
        | core.pop_build_refusal().is_some()
        | core.pop_deploy_refusal().is_some();
    if refused {
        sound.play(Request::own(Cue::Refused));
    }
}

/// A scatter slot going away — the loudest positional cue in the client.
///
/// **Read off change detection rather than off `slot_changes()`**, which is
/// the same call `props.rs` makes and for the same reason: `Session::pump`
/// drains every queued message before the renderer looks, so a frame that
/// received two event messages sees only the second one's change feed. The
/// mesh swap is authoritative and this listens to the swap.
///
/// `Ref` rather than a `Changed<T>` filter because `Changed` also fires on the
/// frame a component is ADDED, and props are added by the streamer every time
/// the player walks a chunk into the ring. Without the `is_added` guard, every
/// already-felled stump in the forest would crash to the ground again each
/// time it streamed in.
pub fn fell(q: Query<(Ref<super::props::Fellable>, &GlobalTransform)>, mut sound: ResMut<Sound>) {
    for (f, t) in q.iter() {
        if !f.is_changed() || f.is_added() || !f.felled {
            // A slot respawning is silent: a tree that comes back after 20-45
            // minutes (`TERRAIN.md` §2) does not do so audibly.
            continue;
        }
        let p = t.translation();
        // A tree is the thing that stumps; everything else that retires is a
        // rock or an ore node coming apart.
        let cue = if f.stumps {
            Cue::TreeFall
        } else {
            Cue::ImpactStone
        };
        sound.play(Request::at(cue, [p.x, p.y, p.z]));
    }
}

/// Health, as a change rather than as an event.
///
/// `EV_HEALTH` is absolute and own-fact (`sim_core::world`), so "I was hurt"
/// is a *fall* in `core.hp`, not a message. Tracked here rather than in the
/// core because it is a presentation fact: the core is right to publish the
/// value and wrong to publish a delta nobody but the HUD and this file wants.
#[derive(Resource, Default)]
pub struct LastHp(pub u16);

pub fn hurt(net: NonSend<Net>, mut last: ResMut<LastHp>, mut sound: ResMut<Sound>) {
    let hp = net.session.core.hp;
    // A rise (heal, respawn) and the first message of all are both silent.
    // `last.0 == 0` is "we have never seen one", which a fresh world is.
    if last.0 > 0 && hp < last.0 {
        sound.play(Request::own(Cue::Hurt));
    }
    last.0 = hp;
}

/// The ambience bed's level: quiet in the open, louder under trees.
///
/// This is the reference's **localized ambience** at the smallest size it can
/// honestly be (`reference/AUDIO.md` §3): there, a set of emitters is culled
/// against the listener and crossfaded by distance, and the performance note
/// in its own devblog is that the culling was updating too often. Ours is one
/// looping voice whose gain reads how many scatter props are drawn nearby —
/// so the bed already answers to the world rather than being a constant, and
/// it costs one query length per frame instead of an emitter set.
pub fn bed(
    mut sound: ResMut<Sound>,
    eye: Res<Eye>,
    props: Query<&GlobalTransform, With<super::props::Fellable>>,
    time: Res<Time>,
    settings: Res<super::Settings>,
    mut sinks: Query<&mut AudioSink, With<Bed>>,
) {
    // How much cover is within earshot, 0..1. `COVER_FULL` scatter slots
    // inside the radius is "in the woods"; none is "on the beach".
    //
    // **It counts every gatherable slot, not only trees** — a boulder field
    // reads as cover here and a pine wood reads the same. That is a
    // simplification and not a claim: the honest version needs the occupant
    // kind, which `Fellable` carries only as `stumps`, and a bed that told
    // rock from canopy is the localized-emitter slice (`reference/AUDIO.md`
    // §9.3), not this one.
    const FOREST_R2: f32 = 22.0 * 22.0;
    const COVER_FULL: f32 = 14.0;
    let near = props
        .iter()
        .filter(|t| t.translation().distance_squared(eye.pos) < FOREST_R2)
        .count() as f32;
    let cover = (near / COVER_FULL).clamp(0.0, 1.0);
    // Open ground is windier than the inside of a forest, but a forest is not
    // silent — it is the same wind in the canopy. So the bed never drops
    // below half, and cover moves it rather than gating it.
    sound.bed_target = 1.0 - 0.45 * cover;

    let d = BED_FADE_PER_S * time.delta_secs();
    let (g, t) = (sound.bed_gain, sound.bed_target);
    sound.bed_gain = if g < t {
        (g + d).min(t)
    } else {
        (g - d).max(t)
    };

    let mix = mix_of(&settings);
    let level = sound.bed_gain * Cue::BedWind.def().gain * mix.bus_gain(Cue::BedWind.def().bus);
    for mut sink in sinks.iter_mut() {
        sink.set_volume(Volume::Linear(level));
    }
}

/// Resolve the frame and start what the mixer chose.
///
/// The voice count is the live entity count, not a number this file keeps: an
/// entity with `PlaybackMode::Despawn` goes away when its sample ends, and
/// nothing tells us when that was. Counting the query is exact and costs a
/// length.
// Eight: the world to spawn into, the mixer, the bank, the listener, the
// clock, the mix, the live voice count, and the drop counter's last report.
// Every one is a distinct source this frame reads.
#[allow(clippy::too_many_arguments)]
pub fn pump(
    mut commands: Commands,
    mut sound: ResMut<Sound>,
    bank: Res<Bank>,
    eye: Res<Eye>,
    time: Res<Time>,
    settings: Res<super::Settings>,
    voices: Query<(), With<Voice>>,
    mut reported: Local<u32>,
) {
    let mix = mix_of(&settings);
    // **One frame stale, deliberately.** A voice spawned through `Commands`
    // is not queryable until the next flush, so `live` is last frame's count
    // and the pool can overshoot by at most one frame's budget —
    // `VOICE_CAP + STARTS_PER_FRAME - 1`. Reading it exactly would mean
    // tracking spawns in a counter that then has to learn when a `Despawn`
    // playback ended, which nothing tells us; a bounded overshoot of four is
    // the cheaper correct answer.
    let live = voices.iter().count();
    let listener = [eye.pos.x, eye.pos.y, eye.pos.z];
    let dt_ms = time.delta_secs() * 1000.0;
    // Copied out because `starts` borrows the mixer and spawning needs the
    // world — at most `STARTS_PER_FRAME` of them, on the stack, no allocation.
    let mut chosen = [None::<Start>; crate::sound::STARTS_PER_FRAME];
    {
        let starts = sound.mixer.tick(dt_ms, listener, live, &mix);
        for (slot, s) in chosen.iter_mut().zip(starts.iter()) {
            *slot = Some(*s);
        }
    }
    for start in chosen.into_iter().flatten() {
        let pb = PlaybackSettings {
            mode: PlaybackMode::Despawn,
            volume: Volume::Linear(start.gain),
            speed: start.speed,
            spatial: start.at.is_some(),
            ..default()
        };
        let mut e = commands.spawn((
            super::WorldEntity,
            Voice,
            AudioPlayer(bank.get(start.cue)),
            pb,
        ));
        match start.at {
            // A spatial player with no `Transform` warns and plays at the
            // origin — the reference's own placement-at-world-origin bug
            // (`reference/AUDIO.md` §6), one API down. The mixer already
            // refuses a positional cue with no position; this is the second
            // half of the same rule.
            Some(p) => {
                e.insert(Transform::from_xyz(p[0], p[1], p[2]));
            }
            None => {
                // Non-spatial voices still need a place in the hierarchy;
                // they are not parented to anything and never move.
                e.insert(Transform::default());
            }
        }
    }
    // A dropped request is a caller over `CUE_QUEUE_CAP`, which is a bug in
    // the caller rather than a load condition (`sound::CUE_QUEUE_CAP`). Said
    // once per new drop and never asserted: the counter is cumulative, so a
    // `debug_assert` would turn one legitimately busy frame — a mass respawn
    // felling thirty slots at once — into a panic on every frame after it.
    if sound.mixer.dropped > *reported {
        *reported = sound.mixer.dropped;
        warn!(
            "sound: {} cue requests dropped since start - a caller is over CUE_QUEUE_CAP",
            sound.mixer.dropped
        );
    }
}

/// The settings screen's three sliders as a [`Mix`].
fn mix_of(s: &super::Settings) -> Mix {
    Mix {
        master: s.vol_master,
        game: s.vol_game,
        ambience: s.vol_ambience,
    }
}

/// The frame budget must not exceed the pool it draws from.
const _: () = assert!(crate::sound::STARTS_PER_FRAME <= VOICE_CAP);

/// The spatial scale must keep every audible emitter inside rodio's clamp, or
/// the header's whole argument is void. Not a `const` assert because float
/// comparison is not const-evaluable; a debug assert on the first frame of
/// audio is early enough, and `tests/sound.rs` asserts the same relation
/// against `MAX_AUDIBLE_M` where it runs in the code tier.
pub fn clamp_holds() -> bool {
    SPATIAL_SCALE * (MAX_AUDIBLE_M + EAR_GAP_M) < 1.0
}
