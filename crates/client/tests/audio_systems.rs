//! Gate: the audio systems that need no world RUN, on a bare `App`, and each
//! one's request reaches the engine as the command it should be.
//!
//! **`tests/music.rs` did this for the score and nothing did it for the
//! rest** (`NOW.md` §0x item 3). `bank.rs` runs `synthesize`, and
//! `impact.rs`/`tracer.rs` call the pure `contact_cue`, `far_layer` and
//! `shot_cue` — but no test scheduled `audio::setup`, `teardown`, `ui_click`,
//! `map_paper`, `fires`, `impacts`, `voices`, `fell` or `pump`, so the only
//! proof that their parameters resolve and that their asks leave as `Cmd`s
//! was a `--capture` looked at by hand. That is the gap `build_bank`'s header
//! records costing a run once already: every gate green, and a system that
//! never ran.
//!
//! "World-free" is the line: these need no `Net` and no `WorldId` to run, so
//! a headless app can hold every resource they ask for. `fell` takes the
//! island as an `Option` and reads it only to tell a picked bush from a
//! broken node, so all of it but the bush-pick runs here. `water`, `feed`,
//! `hurt` and `shots` take `NonSend<Net>`, and `place` and `steps` that and
//! the island; `bed`, `remote_steps` and `remote_hands` take the island as a
//! plain `Res<WorldId>`, so none of those can run without a world. `hands`
//! and `remote_heard` take the session as an `Option` and return before
//! doing anything without one, so scheduling them here would assert
//! nothing. `remote_swings` reads `Feed::swings`, which only `feed::drain`
//! can fill.
//!
//! No window, no GPU, no socket, no device: `MinimalPlugins`, the real bank
//! (`build_bank`, as a boot calls it), and the engine's frame buffer, which is
//! what `audio_out::flush` reads.

use std::f32::consts::TAU;
use std::time::Duration;

use bevy::prelude::*;
use bevy::time::TimePlugin;

use client::render::audio::{
    build_bank, fell, fires, impacts, map_paper, music, music_mode, pump, setup, teardown,
    ui_click, voices, Engine, LastHp, Sound, BEDS, CMD_FRAME_CAP, FIRE_REACH_M,
};
use client::render::feed::Feed;
use client::render::fx::Fx;
use client::render::impact::{Contact, Contacts, Matter, Weapon};
use client::render::mobs::{Animal, Gait};
use client::render::props::{FellPart, Fellable};
use client::render::structures::FireLight;
use client::render::{Eye, Settings};
use client::sound::engine::{Cmd, HELD, HELD_BEDS};
use client::sound::music::Mode;
use client::sound::Cue;

/// Where the ears are: off the origin, so a system that read a zero instead
/// of `Eye` would be caught by the pan.
const EAR: Vec3 = Vec3::new(10.0, 1.6, -20.0);
/// Facing three eighths of a turn — a bearing on the sim's quantized grid
/// (`tests/look.rs`'s eight), and not one where right is an axis.
const YAW: f32 = TAU * 3.0 / 8.0;
/// One frame, seconds. A slow one, so a crackle's 0.45–1.2 s gap is a handful
/// of updates rather than hundreds.
const STEP_S: f32 = 0.1;

/// The world-free audio systems on a bare app, in the order the real chain
/// runs them (`render/mod.rs`'s audio block), `pump` last.
fn app() -> App {
    let mut app = App::new();
    // `TimePlugin` off and `Time` by hand, `music.rs`'s reason: a crackle's
    // gap is half a second and the real clock moves microseconds an update.
    app.add_plugins(MinimalPlugins.build().disable::<TimePlugin>())
        .insert_resource(Time::<()>::default())
        .init_resource::<Sound>()
        .init_resource::<Engine>()
        .init_resource::<LastHp>()
        .init_resource::<Settings>()
        .init_resource::<Contacts>()
        .init_resource::<Feed>()
        .init_resource::<Fx>()
        .init_resource::<Asked>()
        .insert_resource(Eye {
            placed: true,
            pos: EAR,
            yaw: YAW,
            ..default()
        });
    // The real bank: `pump` takes `Res<Bank>`, and the ledger counts a voice
    // only for a cue that has landed — which `teardown`'s test needs.
    build_bank(&mut app);
    app.world_mut()
        .resource_mut::<Engine>()
        .take_installs()
        .for_each(drop);
    app.add_systems(
        Update,
        (impacts, fell, voices, ui_click, fires, asked, pump).chain(),
    );
    app
}

/// Every request the frame's producers put to the mixer, counted before
/// `pump` resolves them. The mixer's per-cue cooldown binds within a frame,
/// so two asks for one cue on one frame leave as one start — and two asks
/// for one tree is the defect `fell`'s test exists to see.
#[derive(Resource, Default)]
struct Asked(usize);

fn asked(sound: Res<Sound>, mut n: ResMut<Asked>) {
    n.0 += sound.mixer.queued();
}

/// Read and zero [`Asked`].
fn take_asked(app: &mut App) -> usize {
    std::mem::take(&mut app.world_mut().resource_mut::<Asked>().0)
}

/// Run `n` frames of [`STEP_S`] and collect every command they sent, in order.
fn frames(app: &mut App, n: usize) -> Vec<Cmd> {
    let mut out = Vec::new();
    for _ in 0..n {
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(Duration::from_secs_f32(STEP_S));
        app.update();
        out.extend(app.world_mut().resource_mut::<Engine>().take());
    }
    out
}

/// Run a one-off system the way its `OnEnter` would, and take what it pushed.
fn once<M>(app: &mut App, sys: impl IntoSystem<(), (), M> + 'static) -> Vec<Cmd> {
    app.world_mut().run_system_cached(sys).expect("system ran");
    app.world_mut().resource_mut::<Engine>().take().collect()
}

/// One admitted one-shot: `(cue, gain_l, gain_r, rate)`.
type Voice = (Cue, f32, f32, f32);

fn starts(cmds: &[Cmd]) -> Vec<Voice> {
    cmds.iter()
        .filter_map(|c| match *c {
            Cmd::Start {
                cue,
                gain_l,
                gain_r,
                rate,
                ..
            } => Some((cue, gain_l, gain_r, rate)),
            _ => None,
        })
        .collect()
}

/// The ears' right in the frame Bevy draws: `render::rig::follow_eye`'s
/// camera, asked for its own basis (`tests/look.rs`'s helper). Taken from
/// Bevy rather than from `look::right_dir` so a pan that went the wrong way
/// is caught here, not restated.
fn bevy_right() -> Vec3 {
    let dir = Vec3::new(YAW.sin(), 0.0, YAW.cos());
    let mut t = Transform::default();
    t.look_to(dir, Vec3::Y);
    t.right().as_vec3()
}

/// An own cue is the interface's, at the head: both ears, equal, heard.
fn assert_own(v: Voice, cue: Cue) {
    let (c, l, r, rate) = v;
    assert_eq!(c, cue);
    assert!(
        l > 0.0 && l == r,
        "{cue:?} is own and must be centred: {l} / {r}"
    );
    assert!(rate > 0.0, "{cue:?} started at rate {rate}");
}

/// A positional cue on the ears' right is in the right ear, hard; `side`
/// −1 is the mirror.
fn assert_side(v: Voice, side: f32) {
    let (cue, l, r, _) = v;
    let (near, far) = if side > 0.0 { (r, l) } else { (l, r) };
    assert!(
        near > 0.0 && far < 0.1 * near,
        "{cue:?} on side {side} panned l {l} / r {r}"
    );
}

/// Entering a world opens every bed on its own held slot, silent, so it can
/// fade in — and a world with nothing in it sends nothing after that.
#[test]
fn setup_opens_every_bed_silent() {
    let mut app = app();
    let cmds = once(&mut app, setup);
    let want: Vec<Cmd> = BEDS
        .iter()
        .enumerate()
        .map(|(i, &cue)| Cmd::Loop {
            slot: i as u8,
            cue,
            gain: 0.0,
        })
        .collect();
    assert_eq!(cmds, want);
    assert_eq!(BEDS.len(), HELD_BEDS);
    // The chain runs and has nothing to say: no light, no blow, no herd, no
    // button.
    let idle = frames(&mut app, 10);
    assert!(idle.is_empty(), "an empty world sent {idle:?}");
}

/// A press clicks, once, at the head. Holding it does not click again (the
/// `Changed` filter), a hover does not click, and something that is not a
/// button does not either.
#[test]
fn a_pressed_button_clicks_once() {
    let mut app = app();
    let button = app.world_mut().spawn((Button, Interaction::Pressed)).id();
    let cmds = frames(&mut app, 1);
    let s = starts(&cmds);
    assert_eq!(s.len(), 1, "one press, {cmds:?}");
    assert_own(s[0], Cue::UiClick);

    let held = frames(&mut app, 3);
    assert!(
        starts(&held).is_empty(),
        "a held press clicked again: {held:?}"
    );

    *app.world_mut().get_mut::<Interaction>(button).unwrap() = Interaction::Hovered;
    let hover = frames(&mut app, 3);
    assert!(starts(&hover).is_empty(), "a hover clicked: {hover:?}");

    // Pressed again: a second click, so the first was not a one-off.
    *app.world_mut().get_mut::<Interaction>(button).unwrap() = Interaction::Pressed;
    let again = starts(&frames(&mut app, 1));
    assert_eq!(again.len(), 1, "a second press did not click");
    assert_own(again[0], Cue::UiClick);

    app.world_mut().spawn(Interaction::Pressed);
    let stray = frames(&mut app, 2);
    assert!(
        starts(&stray).is_empty(),
        "a press off any button clicked: {stray:?}"
    );
}

/// The map's rustle: one own cue per call, the frame after. And it takes the
/// mixer as an `Option`, so a world without one is a no-op, not a panic.
#[test]
fn the_map_rustles_and_needs_no_mixer() {
    let mut app = app();
    assert!(
        once(&mut app, map_paper).is_empty(),
        "map_paper pushed past the mixer"
    );
    let s = starts(&frames(&mut app, 1));
    assert_eq!(s.len(), 1, "the map made {s:?}");
    assert_own(s[0], Cue::MapPaper);

    let mut bare = World::new();
    bare.run_system_cached(map_paper)
        .expect("map_paper without a Sound");
}

/// A lit fire crackles in the ear it is beside, at a cadence and not a
/// metronome; an unlit one and one out of reach do not.
#[test]
fn a_lit_fire_crackles_on_its_side() {
    let mut app = app();
    let right = bevy_right();
    let at = |side: f32, d: f32| GlobalTransform::from_translation(EAR + right * side * d);
    let fire = app
        .world_mut()
        .spawn((
            PointLight {
                intensity: 1_000.0,
                ..default()
            },
            at(1.0, 3.0),
            FireLight {
                cx: 0,
                cz: 0,
                level: 0,
                loc: 0,
            },
        ))
        .id();

    // Thirty frames: the first crackle on the first, then one every 0.45 to
    // 1.2 s — three at the slowest, six at the fastest.
    let s = starts(&frames(&mut app, 30));
    assert!(
        (3..=6).contains(&s.len()),
        "{} crackles in 3 s: {s:?}",
        s.len()
    );
    for v in &s {
        assert_eq!(v.0, Cue::FireCrackle);
        assert_side(*v, 1.0);
    }

    // The same fire on the left.
    *app.world_mut().get_mut::<GlobalTransform>(fire).unwrap() = at(-1.0, 3.0);
    let s = starts(&frames(&mut app, 30));
    assert!(!s.is_empty(), "the fire went quiet on the left");
    for v in &s {
        assert_side(*v, -1.0);
    }

    // Out of reach: silence.
    *app.world_mut().get_mut::<GlobalTransform>(fire).unwrap() = at(1.0, FIRE_REACH_M + 1.0);
    let far = frames(&mut app, 30);
    assert!(
        starts(&far).is_empty(),
        "a fire out of reach crackled: {far:?}"
    );

    // In reach but out: the light is the sim's word that the fire is lit.
    *app.world_mut().get_mut::<GlobalTransform>(fire).unwrap() = at(1.0, 3.0);
    app.world_mut()
        .get_mut::<PointLight>(fire)
        .unwrap()
        .intensity = 0.0;
    let out = frames(&mut app, 30);
    assert!(starts(&out).is_empty(), "an unlit fire crackled: {out:?}");
}

/// A blow is heard as what it struck, where it struck it — once, the frame
/// its contact is on the list.
#[test]
fn a_blow_on_wood_is_heard_where_it_landed() {
    let mut app = app();
    let right = bevy_right();
    app.world_mut().resource_mut::<Contacts>().push(Contact {
        at: EAR + right * 1.5,
        matter: Matter::Wood,
        weapon: Weapon::Melee,
        ..default()
    });
    let s = starts(&frames(&mut app, 1));
    assert_eq!(s.len(), 1, "one contact, {s:?}");
    assert_eq!(s[0].0, Cue::ImpactWood);
    assert_side(s[0], 1.0);

    // `impact::contacts` clears the list every frame; with it cleared there
    // is nothing to hear.
    app.world_mut().resource_mut::<Contacts>().clear();
    let after = frames(&mut app, 3);
    assert!(
        starts(&after).is_empty(),
        "a cleared contact sounded: {after:?}"
    );
}

/// An animal turning on you speaks at once, in its near voice, from where it
/// stands — not whenever its ambient clock next comes round.
#[test]
fn a_roused_pig_snorts_at_once() {
    let mut app = app();
    let right = bevy_right();
    let slot = (0..sim_core::limits::MAX_MOBS)
        .find(|&s| sim_core::mob::kind_of(s) == sim_core::mob::MOB_PIG)
        .expect("a pig slot");
    let pig = app
        .world_mut()
        .spawn((
            Animal(sim_core::mob::mob_id(slot)),
            Transform::from_translation(EAR + right * 4.0),
            Gait::new(slot),
        ))
        .id();
    // First sight primes its clock silently.
    let calm = frames(&mut app, 1);
    assert!(
        starts(&calm).is_empty(),
        "a calm pig spoke on sight: {calm:?}"
    );

    app.world_mut().get_mut::<Gait>(pig).unwrap().hostile = true;
    let s = starts(&frames(&mut app, 1));
    assert_eq!(s.len(), 1, "the hunt started in silence: {s:?}");
    assert_eq!(s[0].0, client::sound::voice::cue_of(slot, true));
    assert_side(s[0], 1.0);
}

/// One piece of a scatter slot as `props` draws it, standing.
fn piece(key: u32, part: FellPart, felled: bool) -> Fellable {
    Fellable {
        key,
        variant: 0,
        base_y: 0.0,
        yaw: 0.0,
        part,
        felled,
        grubbed: false,
    }
}

/// Flip `felled` on each of `parts` on one frame, as the sim retiring (or
/// respawning) their slot does.
fn set_felled(app: &mut App, parts: &[Entity], felled: bool) {
    for &e in parts {
        app.world_mut().get_mut::<Fellable>(e).unwrap().felled = felled;
    }
}

/// A chopped tree falls once, from its trunk, where it stands: one
/// `TreeFall` per slot however many parts the slot is drawn as — the second
/// cue the canopy once added, and the far hull would. Lying down is silent,
/// a respawn is silent, a part with no voice is silent on its own, and a
/// tree drawn already down (a join into a cleared forest) is silent.
#[test]
fn a_felled_tree_falls_once_from_its_trunk() {
    let mut app = app();
    let right = bevy_right();
    let at = |side: f32| GlobalTransform::from_translation(EAR + right * side * 5.0);
    let tree: Vec<Entity> = [
        FellPart::Trunk,
        FellPart::Canopy,
        FellPart::Stump,
        FellPart::Far,
    ]
    .into_iter()
    .map(|part| app.world_mut().spawn((piece(7, part, false), at(1.0))).id())
    .collect();
    // First sight is an add, not a change: a standing tree says nothing.
    let calm = frames(&mut app, 1);
    assert!(calm.is_empty(), "a standing tree sent {calm:?}");

    // The chop: every part of the slot flips on one frame.
    set_felled(&mut app, &tree, true);
    let s = starts(&frames(&mut app, 1));
    assert_eq!(
        take_asked(&mut app),
        1,
        "one chop asked for more than one cue"
    );
    assert_eq!(s.len(), 1, "one chop, {s:?}");
    assert_eq!(s[0].0, Cue::TreeFall);
    assert_side(s[0], 1.0);

    let down = frames(&mut app, 10);
    assert_eq!(
        take_asked(&mut app),
        0,
        "a fallen tree kept asking: {down:?}"
    );

    // The slot respawns, silently; chopped again, it is heard again.
    set_felled(&mut app, &tree, false);
    let back = frames(&mut app, 3);
    assert_eq!(take_asked(&mut app), 0, "a respawn was heard: {back:?}");
    set_felled(&mut app, &tree, true);
    let again = starts(&frames(&mut app, 1));
    assert_eq!(take_asked(&mut app), 1, "the second chop: {again:?}");
    assert_eq!(again.len(), 1);
    assert_eq!(again[0].0, Cue::TreeFall);

    // Grubbing the stump changes a felled stump, and a stump never speaks.
    app.world_mut()
        .get_mut::<Fellable>(tree[2])
        .unwrap()
        .grubbed = true;
    let grub = frames(&mut app, 1);
    assert_eq!(
        take_asked(&mut app),
        0,
        "a grubbed stump was heard: {grub:?}"
    );

    // Each voiceless part, on a slot with no trunk to speak for it.
    let quiet: Vec<Entity> = [
        FellPart::Canopy,
        FellPart::Stump,
        FellPart::Far,
        FellPart::Emptied,
    ]
    .into_iter()
    .map(|part| {
        app.world_mut()
            .spawn((piece(8, part, false), at(-1.0)))
            .id()
    })
    .collect();
    let _ = frames(&mut app, 1);
    set_felled(&mut app, &quiet, true);
    let mute = frames(&mut app, 3);
    assert_eq!(take_asked(&mut app), 0, "a voiceless part spoke: {mute:?}");

    // A trunk that is already down the first time it is drawn.
    app.world_mut()
        .spawn((piece(9, FellPart::Trunk, true), at(-1.0)));
    let drawn = frames(&mut app, 3);
    assert_eq!(take_asked(&mut app), 0, "a tree drawn down fell: {drawn:?}");
}

/// A node that just stops being there — ore, a boulder, a barrel — breaks as
/// stone, where it stood. (A picked bush rustles instead, and telling the two
/// apart reads the island's scatter: the one branch of `fell` this file
/// cannot reach.)
#[test]
fn a_broken_node_is_heard_as_stone() {
    let mut app = app();
    let node = app
        .world_mut()
        .spawn((
            piece(11, FellPart::Vanish, false),
            GlobalTransform::from_translation(EAR - bevy_right() * 2.0),
        ))
        .id();
    let calm = frames(&mut app, 1);
    assert!(calm.is_empty(), "a standing node sent {calm:?}");

    set_felled(&mut app, &[node], true);
    let s = starts(&frames(&mut app, 1));
    assert_eq!(take_asked(&mut app), 1, "one node, {s:?}");
    assert_eq!(s.len(), 1, "one node, {s:?}");
    assert_eq!(s[0].0, Cue::ImpactStone);
    assert_side(s[0], -1.0);

    let gone = frames(&mut app, 3);
    assert_eq!(take_asked(&mut app), 0, "a gone node kept asking: {gone:?}");
}

/// Leaving a world stops every bed, cuts every one-shot — on the renderer
/// and in the ledger — and forgets the last health. A piece of music that is
/// sounding is not the world's: it rings out over the loading screen
/// (`MusicSlot`'s doc), so no `Stop` reaches its slot.
#[test]
fn teardown_ends_what_the_world_started() {
    let mut app = app();
    // A piece on a music slot first. The menu's director opens at once
    // (`tests/music.rs`); `register_system` for `music.rs`'s reason, a
    // capturing closure the cached path refuses.
    app.add_systems(Update, music);
    let id = app.world_mut().register_system(music_mode(Mode::Menu));
    app.world_mut().run_system(id).expect("music_mode ran");
    let opened = frames(&mut app, 2);
    let tune = opened
        .iter()
        .find_map(|c| match *c {
            Cmd::Play { slot, .. } => Some(slot),
            _ => None,
        })
        .expect("the menu opened on no piece");
    assert!(
        (HELD_BEDS as u8..HELD as u8).contains(&tune),
        "the piece went to slot {tune}, which is not a music slot"
    );

    let _ = once(&mut app, setup);
    app.world_mut().spawn((
        PointLight {
            intensity: 1_000.0,
            ..default()
        },
        GlobalTransform::from_translation(EAR + Vec3::X),
        FireLight {
            cx: 0,
            cz: 0,
            level: 0,
            loc: 0,
        },
    ));
    // The menu's frames had no fire, which put `fires` on its half-second
    // idle wait: step to the first crackle rather than assume a frame.
    let lit = (0..10).any(|_| {
        starts(&frames(&mut app, 1))
            .iter()
            .any(|v| v.0 == Cue::FireCrackle)
    });
    assert!(lit, "the fire never crackled");
    assert!(
        app.world().resource::<Engine>().live.count() > 0,
        "the crackle did not reach the ledger"
    );
    app.world_mut().resource_mut::<LastHp>().0 = 55;

    let cmds = once(&mut app, teardown);
    let mut want: Vec<Cmd> = (0..BEDS.len() as u8)
        .map(|slot| Cmd::Stop { slot })
        .collect();
    want.push(Cmd::CutVoices);
    assert_eq!(cmds, want);
    assert!(
        !cmds
            .iter()
            .any(|c| matches!(c, Cmd::Stop { slot } if *slot == tune)),
        "teardown cut the piece on slot {tune}"
    );
    assert_eq!(app.world().resource::<Engine>().live.count(), 0);
    assert_eq!(app.world().resource::<LastHp>().0, 0);

    // And it rings on: nothing after teardown stops it either.
    let after = frames(&mut app, 5);
    assert!(
        !after
            .iter()
            .any(|c| matches!(c, Cmd::Stop { slot } if *slot == tune)),
        "the piece was stopped after teardown: {after:?}"
    );
}

/// The frame buffer's cap: drop the newest and count it, never grow.
#[test]
fn the_frame_buffer_drops_newest_and_counts() {
    let mut engine = Engine::default();
    for slot in 0..=CMD_FRAME_CAP {
        engine.push(Cmd::Stop { slot: slot as u8 });
    }
    assert_eq!(engine.dropped, 1);
    let kept: Vec<Cmd> = engine.take().collect();
    assert_eq!(kept.len(), CMD_FRAME_CAP);
    assert_eq!(
        kept.last(),
        Some(&Cmd::Stop {
            slot: CMD_FRAME_CAP as u8 - 1
        })
    );
    assert_eq!(engine.pending(), 0);
}
