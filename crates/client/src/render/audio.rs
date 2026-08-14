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

use crate::sound::birds::{self, Birds};
use crate::sound::mixer::{Mixer, Request, Start};
use crate::sound::music::{self, Director};
use crate::sound::pig::Snorts;
use crate::sound::steps::Steps;
use crate::sound::water::Waterline;
use crate::sound::{
    synth, Cue, Mix, Snapshot, SnapshotDef, Snapshots, CUE_COUNT, MAX_AUDIBLE_M, VOICE_CAP,
};

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

/// How fast a bed's gain follows the world, per second. A bed that snapped
/// would click every time the tree count under the camera changed by one.
///
/// **This is the WORLD's half of a bed's level, not the mixer's.** A snapshot
/// moves the same gains far faster (`sound::SNAPSHOT_FADE_S`, 0.3 s against
/// 2 s here), and that asymmetry is deliberate: walking out of a forest is a
/// slow fact and putting your head under water is an instant one.
pub const BED_FADE_PER_S: f32 = 0.5;

/// The looping beds, in the order [`Sound::bed_gain`] indexes them.
pub const BEDS: [Cue; 3] = [Cue::BedWind, Cue::BedSurf, Cue::BedUnder];

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
    /// The waterline, as a thing the local body crosses.
    pub waterline: Waterline,
    /// Each roster slot's snort clock (`sound::pig` — pure; [`pigs`] is
    /// the producer that reads it against the drawn herd).
    pub snorts: Snorts,
    /// When a song plays and which piece it is (`sound::music`). Lives on
    /// the resource rather than on a world entity because it runs on the
    /// menus too, where there is no world.
    pub music: Director,
    /// The forest layer's clock (`sound::birds`), driven by [`bed`], which
    /// already has the cover score and the props to perch on.
    pub birds: Birds,
    /// Each bed's current gain, moving toward its target at
    /// [`BED_FADE_PER_S`]. Held rather than recomputed so the crossfade is
    /// state, not a function of a frame.
    bed_gain: [f32; BEDS.len()],
    bed_target: [f32; BEDS.len()],
    /// The snapshot crossfade, and this frame's resolved mixer state.
    snapshots: Snapshots,
    snap: SnapshotDef,
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

/// One looping bed, and which one.
#[derive(Component)]
pub struct Bed(pub Cue);

/// A piece of music that is playing.
///
/// **Deliberately not a [`Voice`] and deliberately not a
/// [`super::WorldEntity`]**, and both are load-bearing:
///
/// - Not a `Voice`, so music does not count against `VOICE_CAP` and cannot
///   be the reason an axe was refused (`sound::mixer` refuses to start a
///   music cue at all — `Cue::is_music`).
/// - Not a `WorldEntity`, so leaving a world does not cut a piece off
///   mid-phrase. A menu piece rings out over the loading screen, which is
///   how music is supposed to carry a transition.
#[derive(Component)]
pub struct MusicVoice {
    /// Which piece this is, so its level comes from the cue table like every
    /// other level in the client rather than from a constant here.
    cue: Cue,
    /// The piece's own gain, 1 while it is playing normally and ramping to
    /// zero once [`ending`](Self::ending) is set.
    fade: f32,
    /// Set when a screen change orphaned this piece: the director under it
    /// has been reset, and something else is about to start on top of it.
    ending: bool,
}

/// How long an orphaned piece takes to fade out, seconds.
///
/// **The one place music fades, and the reference's rule is not being
/// broken.** `reference/AUDIO.md` §8 says pieces cut to each other *without*
/// fading, and they still do — the tail covers that join. This is the other
/// case: leaving a world starts the menu's music immediately, and two pieces
/// playing over each other is not a join, it is two songs. Short enough not
/// to be a swell, long enough not to click (`DECISIONS.md` §open,
/// "music v0").
pub const MUSIC_FADE_S: f32 = 1.2;

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

    // Every bed starts SILENT and fades in. Entering a world at full ambience
    // on the first frame of the loading screen is the audio version of the
    // world popping in, and the fade is already the mechanism.
    //
    // **All three run from the first frame, at zero.** A bed spawned on demand
    // would start its 10.5 s loop wherever the player happened to break the
    // surface, so the submerged bed would open on a bubble or on nothing; and
    // a voice started under an already-open snapshot has no silence to fade
    // up from.
    for cue in BEDS {
        commands.spawn((
            super::WorldEntity,
            Bed(cue),
            AudioPlayer(bank.get(cue)),
            PlaybackSettings {
                mode: PlaybackMode::Loop,
                volume: Volume::SILENT,
                ..default()
            },
        ));
    }
}

/// Reset what the world owned. The bed and the listener go with the camera
/// (`WorldEntity`), but the step odometer is in a resource that outlives the
/// world — and a stale one measures the distance between two worlds as ground
/// covered and fires a burst of footsteps on the first frame of the next.
pub fn teardown(mut sound: ResMut<Sound>, mut last_hp: ResMut<LastHp>) {
    sound.steps.reset();
    // The herd's clocks too: a countdown carried into the next island would
    // voice its pigs on this island's schedule. The forest layer's clock is
    // the same rule and the same reason.
    sound.snorts.reset();
    sound.birds.reset();
    sound.bed_gain = [0.0; BEDS.len()];
    sound.bed_target = [0.0; BEDS.len()];
    // The waterline and the snapshot go with it, and the second one is the
    // reference's own shipped bug: their underwater sound effect stayed on
    // after disconnecting from a server (`reference/WATER.md` §7). A mix state
    // that outlives its cause.
    sound.waterline.reset();
    sound.snapshots.reset();
    sound.snap = SnapshotDef::default();
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

/// One remote body's step odometer — the same `sound::steps::Steps` the
/// local player runs, one per drawn body, carried as a component so it dies
/// with the entity and a body that leaves AOI and returns starts fresh (no
/// map to sweep, no reset to remember). `bodies::stream` inserts it at
/// spawn.
#[derive(Component, Default)]
pub struct RemoteSteps(pub Steps);

/// Another player's footsteps — the sound that decides fights, and until
/// this system nothing produced it: only the local body has a predictor,
/// so a remote's cadence comes off the same distance-integrated odometer
/// fed the INTERPOLATED transform `bodies::stream` just wrote (this runs
/// after `Stream`). The surface is `terrain::splat` at THEIR position, the
/// cue is the local family's positional twin (`steps::remote`), and "only
/// nearby" is the mixer's own falloff at the cue's radius — the falling
/// tree's pattern: push with a position, let the one distance law cull.
///
/// Two honest gaps, both the wire's: there is no grounded bit (`NOW.md`
/// §0v item 1), so a jumping remote ticks the odometer by the horizontal
/// half of its arc; and a teleport (death, respawn) reads as ground
/// covered, which the odometer's own hitch cap bounds at ONE step. A
/// sleeper stands still and the speed floor keeps it silent for free.
pub fn remote_steps(
    world: Res<super::WorldId>,
    time: Res<Time>,
    mut bodies: Query<(&Transform, &mut RemoteSteps), With<super::bodies::Body>>,
    mut sound: ResMut<Sound>,
) {
    let dt = time.delta_secs();
    for (t, mut steps) in bodies.iter_mut() {
        let pos = [t.translation.x, t.translation.y, t.translation.z];
        let Some(step) = steps.0.sample(pos, true, dt) else {
            continue;
        };
        let splat = sim_core::terrain::splat(world.seed, pos[0], pos[2]);
        let below_sea = pos[1] < sim_core::terrain::SEA_LEVEL;
        let cue = crate::sound::steps::remote(crate::sound::steps::surface_cue(splat, below_sea));
        sound.play(Request::at(cue, pos).with_gain(step.gain));
    }
}

/// This frame's own-facts, as cues.
///
/// **Reads [`super::feed::Feed`]; pops nothing.** It used to pop the core's
/// rings directly, and so did `hud::feedback` — two systems written on two
/// branches, each correct alone, that git merged without a conflict into a
/// client whose HUD ate every event before the mixer saw one. `feed.rs`'s
/// header is the whole account; the rule that came out of it is that
/// `feed::drain` is the only `pop_*` call site in the client.
pub fn feed(net: NonSend<Net>, feed: Res<super::feed::Feed>, mut sound: ResMut<Sound>) {
    // One marker per frame however many landed — `Cue::Hit`'s own cooldown
    // would refuse the rest anyway, and asking for four identical clicks so
    // three can be thrown away is work the queue does not need to do.
    if feed.hits > 0 {
        sound.play(Request::own(Cue::Hit));
        // The middle bump. One per frame however many landed, for the same
        // reason the marker is: the director reads its tier once a section,
        // so four bumps in one frame and one bump in one frame are the same
        // musical fact.
        sound.music.bump(music::BUMP_HIT);
    }
    for &(victim, _killer) in feed.deaths() {
        // Someone else dying is not your own fact and has no position on this
        // wire, so only your own death makes a sound. A death rattle for a
        // player across the island would be a lie about where they are.
        if victim == net.session.core.player_id {
            sound.play(Request::own(Cue::Death));
        }
    }
    for _ in feed.gathered() {
        sound.play(Request::own(Cue::Gather));
    }
    for _ in feed.crafted() {
        sound.play(Request::own(Cue::CraftDone));
    }
    // Three refusal kinds, one sound. A player does not need to hear the
    // difference between a refused craft and a refused placement — the toast
    // already says which — they need to hear that the button did nothing.
    if feed.refusals().next().is_some() {
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
        // **One cue per SLOT, not one per entity — and this was already wrong
        // before the tree could topple.** A tree has always been more than one
        // `Fellable`: the canopy is a sibling carrying its own, so a chop fired
        // `TreeFall` for the trunk *and* `ImpactStone` for the needles, at the
        // same position on the same frame. Nothing caught it because both cues
        // are real cues and the mix is the only place the pair is audible.
        // Felling v0 would have made it three, which is what made it visible.
        //
        // `FellPart` names the parts, so the rule can be stated instead of
        // inferred: the trunk speaks for the tree, a `Vanish` node speaks for
        // itself, and the canopy and the stump are silent because they are
        // parts of something that already made a sound.
        let cue = match f.part {
            super::props::FellPart::Trunk => Cue::TreeFall,
            super::props::FellPart::Vanish => Cue::ImpactStone,
            super::props::FellPart::Canopy | super::props::FellPart::Stump => continue,
        };
        let p = t.translation();
        sound.play(Request::at(cue, [p.x, p.y, p.z]));
    }
}

/// A placement landing — the second positional cue, at the cell it landed.
///
/// **Reads [`super::feed::Feed`]; pops nothing** (`feed::drain` is the one
/// pop site). The join-flood guard lives in the CORE, not here: the ring
/// behind `Feed::placed` is fed only by the `PiecePlaced`/`DeployPlaced`
/// broadcasts — a placement *happening* — and never by the sync walks, which
/// merely restate the world. So a fresh join streaming every standing piece,
/// or a resync restating them, produces zero entries here by construction,
/// with no timer deciding when the flood is over. Distance does the rest the
/// way it does for the falling tree: the request carries the position and
/// `sound::falloff` culls it at the cue's own radius.
///
/// The position is [`super::structures::base_transform`]'s — the same
/// anchor the mesh stands at, edge canonicalisation included, so the sound
/// cannot come from a different place than the wall appears in.
pub fn place(feed: Res<super::feed::Feed>, world: Res<super::WorldId>, mut sound: ResMut<Sound>) {
    for &(cx, cz, level, loc, _deploy) in feed.placed() {
        let p = super::structures::base_transform(world.seed, (cx, cz, level, loc)).translation;
        sound.play(Request::at(Cue::Place, [p.x, p.y, p.z]));
    }
}

/// The pig's voice, off the drawn herd's interpolated positions.
///
/// Dormancy-respecting by construction: a `Pig` entity exists only for a
/// mob inside AOI (208 m), and every mob a client can see is awake —
/// `MOB_WAKE_CM` (240 m) deliberately encloses AOI (`limits.rs`), so a
/// voiced pig is always a simmed pig. "Only nearby" is then the mixer's own
/// falloff at the cue's 40 m radius — the falling-tree pattern: push with a
/// position, let the one distance law cull.
///
/// The cadence is `sound::pig`'s — hashed per roster slot and cycle, so it
/// is deterministic (no OS randomness) and not a metronome. The snout
/// height puts the emitter at the head rather than under the hooves.
pub fn pigs(
    herd: Query<(&super::mobs::Animal, &Transform)>,
    time: Res<Time>,
    mut sound: ResMut<Sound>,
) {
    let dt = time.delta_secs();
    for (pig, t) in herd.iter() {
        let Some(slot) = sim_core::mob::slot_of_id(pig.0) else {
            continue;
        };
        if !sound.snorts.due(slot, dt) {
            continue;
        }
        let p = t.translation;
        sound.play(Request::at(
            Cue::Snort,
            [p.x, p.y + super::mobs::PIG_H_M * 0.6, p.z],
        ));
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
        // **The biggest bump, and theirs is too** (`reference/AUDIO.md` §8's
        // published order: a weapon in play < a bullet past your head <
        // taking damage). Two of these inside two sections is what puts the
        // score in its top tier.
        sound.music.bump(music::BUMP_HURT);
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
/// The forest layer rides along: [`bed`] already walks the props and already
/// scores the cover, so a bird costs a countdown and an index. See
/// `sound::birds` for why a layer is not a bed turned down.
// Seven: the mix state to write, where the ears are, which island this is, the
// cover query, the clock, the player's sliders, and the sinks to move.
#[allow(clippy::too_many_arguments)]
pub fn bed(
    mut sound: ResMut<Sound>,
    eye: Res<Eye>,
    world: Res<super::WorldId>,
    props: Query<(&GlobalTransform, &super::props::Fellable)>,
    time: Res<Time>,
    settings: Res<super::Settings>,
    feed: Res<super::feed::Feed>,
    mut sinks: Query<(&Bed, &mut AudioSink)>,
) {
    // How much cover is within earshot, 0..1. `COVER_FULL` scatter slots
    // inside the radius is "in the woods"; none is "on the beach".
    //
    // **It counts every gatherable slot, not only trees** — a boulder field
    // reads as cover here and a pine wood reads the same. That is a
    // simplification and not a claim: a bed that told rock from canopy is the
    // localized-emitter slice (`reference/AUDIO.md` §9.3), not this one.
    //
    // **What it must count is SLOTS, and a slot is more than one entity.** A
    // tree is three `Fellable`s (trunk, canopy, stump) and every other slot is
    // one, so counting entities would score a pine wood 3× a boulder field of
    // the same density — and `COVER_FULL` was calibrated when a tree was two.
    // Counting the parts that are one-per-slot fixes the units. This is the
    // second thing felling v0 found by adding a part: `Fellable` is not a
    // slot, it is a piece of one.
    //
    // **So `COVER_FULL` moves 14 → 7, and that is a unit conversion rather
    // than a retune.** It was tuned by ear in a pine wood, where every slot
    // counted twice, so 14 entities WAS 7 trees. Leaving it at 14 in the
    // corrected unit would silently double how much forest the bed needs and
    // make the woods quieter — a tuning change nobody chose, arriving as a
    // side effect of a bug fix, which is the worst way for one to arrive.
    const FOREST_R2: f32 = 22.0 * 22.0;
    const COVER_FULL: f32 = 7.0;
    // The two halves of "is this a tree near me", as closures, because the
    // bird layer below walks the same set for a perch and an index into a
    // set that was filtered differently is an index into nothing.
    let is_perch = |f: &super::props::Fellable| {
        matches!(
            f.part,
            super::props::FellPart::Trunk | super::props::FellPart::Vanish
        )
    };
    let near_eye = |p: Vec3| p.distance_squared(eye.pos) < FOREST_R2;
    let near = props
        .iter()
        .filter(|(t, f)| is_perch(f) && near_eye(t.translation()))
        .count();
    let cover = (near as f32 / COVER_FULL).clamp(0.0, 1.0);

    // The forest layer. A second walk of the same query, and it runs on the
    // one frame in a few hundred that a call actually lands — a buffer of
    // candidate perches would cost every frame to save that one, and would
    // cap how much forest the layer can see for nothing.
    //
    // Daylight only, now that a day exists (day/night v0): birds roost at
    // night, and the cause is the server's own clock rather than one this
    // layer invented — the refusal `birds.rs`' header recorded is repaid.
    // Crickets are the night companion and still owed (`NOW.md` §0x).
    //
    // Through `world::is_night` rather than the open-coded comparison this
    // used to carry: the sim reads the same boundary now (a predator's
    // notice radius is the hour's), and two hand-written thresholds against
    // one constant is how the birds and the wolves come to disagree about
    // when dusk was.
    let is_day = !sim_core::world::is_night(feed.server_tick_est.max(0.0) as u64);
    if is_day && sound.birds.due(cover, time.delta_secs()) && near > 0 {
        let want = sound.birds.perch(near);
        if let Some(p) = props
            .iter()
            .filter(|(t, f)| is_perch(f) && near_eye(t.translation()))
            .map(|(t, _)| t.translation())
            .nth(want)
        {
            sound.play(Request::at(Cue::Bird, [p.x, p.y + birds::PERCH_H_M, p.z]));
        }
    }
    // Open ground is windier than the inside of a forest, but a forest is not
    // silent — it is the same wind in the canopy. So the bed never drops
    // below half, and cover moves it rather than gating it.
    sound.bed_target[0] = 1.0 - 0.45 * cover;
    // The surf reads how much sea is within earshot, from the same
    // `terrain::height` the water is drawn from — 24 taps, a fixed pattern, so
    // the level cannot flicker as a search finds different water.
    sound.bed_target[1] = crate::sound::water::surf_gain(crate::sound::water::shore_exposure(
        world.seed, eye.pos.x, eye.pos.z,
    ));
    // The submerged bed has no world level of its own: it is entirely the
    // snapshot's, which is the point of a snapshot.
    sound.bed_target[2] = 1.0;

    let d = BED_FADE_PER_S * time.delta_secs();
    for i in 0..BEDS.len() {
        let (g, t) = (sound.bed_gain[i], sound.bed_target[i]);
        sound.bed_gain[i] = if g < t {
            (g + d).min(t)
        } else {
            (g - d).max(t)
        };
    }

    let snap = sound.snap;
    let mix = mix_of(&settings).under(&snap);
    for (bed, mut sink) in sinks.iter_mut() {
        let Some(i) = BEDS.iter().position(|c| *c == bed.0) else {
            continue;
        };
        let def = bed.0.def();
        let level = sound.bed_gain[i] * snap.bed(bed.0) * def.gain * mix.bus_gain(def.bus);
        sink.set_volume(Volume::Linear(level));
    }
}

/// The waterline: which snapshot the mix is in, and the splash on crossing it.
///
/// **Runs before [`bed`] and [`pump`]**, because both read the snapshot this
/// resolves and a mix state one frame stale is a mix that comes up as your
/// head goes back under.
pub fn water(net: NonSend<Net>, eye: Res<Eye>, time: Res<Time>, mut sound: ResMut<Sound>) {
    let dt = time.delta_secs();
    // The EARS, not the feet: the mix changes when your head goes under, and a
    // player wading chest-deep is still hearing the world above.
    let want = if crate::sound::water::submerged(eye.pos.y) {
        Snapshot::Submerged
    } else {
        Snapshot::Above
    };
    sound.snap = sound.snapshots.tick(want, dt);

    // The feet, for the splash: breaking the surface is your body entering the
    // water, which happens well before your head does.
    let feet = net.session.core.predict.render_position()[1];
    if let Some(gain) = sound.waterline.sample(feet, dt) {
        sound.play(Request::own(Cue::Splash).with_gain(gain));
    }
}

/// The score: tick the director, start the piece it asks for, and hold every
/// sounding piece at the player's music level.
///
/// **Runs everywhere, unlike every other system in this file.** There is no
/// `world_running`, no `Net` and no `Eye` in its arguments, because music
/// plays on the menus too — `sound::music::Mode::Menu` is what makes that a
/// different behaviour rather than a different code path.
///
/// Starting a piece is a spawn and nothing else: the previous piece is left
/// alone to ring out under it, which is the whole of `reference/AUDIO.md`
/// §8's transition design. `PlaybackMode::Despawn` collects it when its tail
/// ends.
pub fn music(
    mut commands: Commands,
    mut sound: ResMut<Sound>,
    bank: Res<Bank>,
    time: Res<Time>,
    settings: Res<super::Settings>,
    mut voices: Query<(Entity, &mut MusicVoice, &mut AudioSink)>,
) {
    let dt = time.delta_secs();
    let mix = mix_of(&settings);
    if let Some(piece) = sound.music.tick(dt) {
        let def = piece.cue.def();
        commands.spawn((
            MusicVoice {
                cue: piece.cue,
                fade: 1.0,
                ending: false,
            },
            AudioPlayer(bank.get(piece.cue)),
            PlaybackSettings {
                mode: PlaybackMode::Despawn,
                // **Its real level, not silence.** A spawn is not queryable
                // until the next flush, so the loop below cannot reach this
                // voice this frame — starting it silent would put a step from
                // zero to full one frame into every piece, which is the click
                // `synth::edges` exists to prevent at the other end of the
                // same sample. The loop's job is to FOLLOW the slider, not to
                // set the opening level.
                volume: Volume::Linear(def.gain * mix.bus_gain(def.bus)),
                spatial: false,
                ..default()
            },
        ));
    }

    let step = if MUSIC_FADE_S > 0.0 {
        dt / MUSIC_FADE_S
    } else {
        1.0
    };
    for (e, mut voice, mut sink) in voices.iter_mut() {
        if voice.ending {
            voice.fade -= step;
            if voice.fade <= 0.0 {
                commands.entity(e).despawn();
                continue;
            }
        }
        let def = voice.cue.def();
        sink.set_volume(Volume::Linear(
            def.gain * mix.bus_gain(def.bus) * voice.fade,
        ));
    }
}

/// Put the director on a new screen's schedule.
///
/// Called on entering the menu and on entering a world, which are the only
/// two transitions music has. One function because the rule is one rule, and
/// it is stated in the condition rather than in the caller: **a sounding
/// piece is ended only when the new mode is about to start one on top of
/// it.** Leaving a world does (the menu has no gap), so the old piece fades;
/// joining one does not (`music::FIRST_GAP_S` is half a minute), so the menu
/// piece rings out over the loading screen, which is what music is for.
pub fn music_mode(mode: music::Mode) -> impl Fn(ResMut<Sound>, Query<&mut MusicVoice>) {
    move |mut sound: ResMut<Sound>, mut voices: Query<&mut MusicVoice>| {
        sound.music.reset(mode);
        if sound.music.next_in_s() > crate::sound::music::PIECE_S {
            return;
        }
        for mut v in voices.iter_mut() {
            v.ending = true;
        }
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
    let mix = mix_of(&settings).under(&sound.snap);
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
        music: s.vol_music,
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
