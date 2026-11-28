//! Gate: the audio systems that need a world RUN, on a headless app with a
//! replayed session under it, and what they hear reaches the engine.
//!
//! `tests/audio_systems.rs` runs the world-free half. This is the other half
//! (`NOW.md` §0x item 3): `water`, `feed`, `hurt`, `shots`, `place`, `steps`,
//! `bed`, `remote_steps`, `remote_hands` and the rest of the in-world chain
//! take `NonSend<Net>` or `Res<WorldId>`, so until now nothing but a
//! `--capture` looked at by hand proved their parameters resolve. A param
//! conflict in one of them panics on the first frame in the game, and every
//! gate stays green.
//!
//! **The world is real, the shard is not.** `Session::replay` builds an
//! ordinary `Session` with no socket (`tests/disconnect.rs`'s boot), and
//! `WorldId::with_haven` is `menu::poll_connect`'s own insert. The chain is
//! the one `render/mod.rs`'s audio block registers, in its order, `pump`
//! last; the facts the drain would hand it (`Feed`) and the predictor's body
//! are set by hand, so `feed::drain` stays the one `pop_*` reader.
//!
//! No window, no GPU, no device: `MinimalPlugins` with the clock by hand,
//! the asset plugin for the body rig's handle, the real bank, and the
//! engine's frame buffer, which is what `audio_out::flush` reads.

#![cfg(all(feature = "render", feature = "native"))]

use std::f32::consts::TAU;
use std::time::Duration;

use bevy::asset::AssetPlugin;
use bevy::gltf::Gltf;
use bevy::prelude::*;
use bevy::time::TimePlugin;

use client::film::{Recording, Replay};
use client::render::anim::{self, BodyAnim};
use client::render::audio::{
    self, build_bank, Engine, LastHp, RemoteHands, RemoteSteps, Sound, Toll, BEDS,
};
use client::render::bodies::Body;
use client::render::feed::Feed;
use client::render::fx::Fx;
use client::render::gunfeel::GunFeel;
use client::render::impact::Contacts;
use client::render::rig::DayPin;
use client::render::weather::WeatherNow;
use client::render::{Eye, Net, Settings, WorldId, EYE_HEIGHT};
use client::sound::engine::{self, Cmd};
use client::sound::Cue;
use client::Session;
use client_core::core::ClientCore;

const SEED: u64 = 0x5eed_a0d1;
/// Where the feet stand: off the origin and well above the sea, so a system
/// that read a zero instead of the predictor would be caught.
const FEET: Vec3 = Vec3::new(10.0, 5.0, -20.0);
/// Facing three eighths of a turn (`tests/audio_systems.rs`'s bearing).
const YAW: f32 = TAU * 3.0 / 8.0;
/// One frame, seconds: a slow one, so a fade is a handful of updates.
const STEP_S: f32 = 0.1;

/// The in-world audio chain on a headless app over a replayed session, the
/// beds not yet opened. The `Replay` is the recording's end of the lanes:
/// held, so the session is a live one.
fn boot() -> (App, Replay) {
    let mut app = App::new();
    // `TimePlugin` off and `Time` by hand, `music.rs`'s reason.
    app.add_plugins((
        MinimalPlugins.build().disable::<TimePlugin>(),
        AssetPlugin::default(),
    ))
    .init_asset::<StandardMaterial>()
    .init_asset::<Gltf>()
    .insert_resource(Time::<()>::default())
    .init_resource::<Sound>()
    .init_resource::<Engine>()
    .init_resource::<Toll>()
    .init_resource::<LastHp>()
    .init_resource::<Settings>()
    .init_resource::<Contacts>()
    .init_resource::<Feed>()
    .init_resource::<Fx>()
    .init_resource::<GunFeel>()
    .init_resource::<DayPin>()
    .init_resource::<WeatherNow>()
    .insert_resource(Eye {
        placed: true,
        pos: FEET + Vec3::Y * EYE_HEIGHT,
        yaw: YAW,
        height: EYE_HEIGHT,
        ..default()
    })
    .add_message::<bevy::window::CursorMoved>();
    build_bank(&mut app);
    app.world_mut()
        .resource_mut::<Engine>()
        .take_installs()
        .for_each(drop);
    // The rig `remote_steps` paces a body's feet by, as the game's `Startup`
    // makes it: the glTF never lands here, so it stays the fallback.
    app.world_mut()
        .run_system_cached(anim::load)
        .expect("anim::load ran");
    assert!(app.world().contains_resource::<anim::Rig>());

    let (session, replay) = Session::replay(Recording {
        welcome: protocol::Welcome {
            player_id: 1,
            seed: SEED,
            tick: 0,
            dev: true,
            blight_salt: 0,
        },
        watch: None,
        entries: Vec::new(),
    });
    // `menu::poll_connect`'s own two inserts.
    app.insert_resource(WorldId::with_haven(SEED, *session.core.haven()));
    app.insert_non_send_resource(Net {
        session,
        sel: 0,
        light: false,
        farm_sent: None,
    });
    feet(&mut app, FEET);

    // `render/mod.rs`'s audio block, in its order, `pump` last. The
    // `.after`s it carries name producers this app does not run.
    app.add_systems(
        Update,
        (
            audio::water,
            audio::feed,
            audio::impacts,
            audio::tungs,
            audio::place,
            audio::hurt,
            audio::steps,
            audio::remote_steps,
            audio::remote_swings,
            audio::remote_heard,
            audio::far_reports,
            audio::remote_hands,
            audio::shots,
            audio::fell,
            audio::voices,
            (audio::ui_click, audio::ui_hover, audio::panel_sounds).chain(),
            audio::hands,
            (audio::fires, audio::geiger, audio::heartbeat).chain(),
            audio::bed,
            audio::pump,
        )
            .chain(),
    );
    (app, replay)
}

/// [`boot`], then into the world as `OnEnter(Screen::Loading)` goes: the
/// beds opened, and their commands taken.
fn world() -> (App, Replay) {
    let (mut app, replay) = boot();
    app.world_mut()
        .run_system_cached(audio::setup)
        .expect("setup ran");
    app.world_mut()
        .resource_mut::<Engine>()
        .take()
        .for_each(drop);
    (app, replay)
}

fn core(app: &mut App) -> &mut ClientCore {
    &mut app
        .world_mut()
        .non_send_resource_mut::<Net>()
        .into_inner()
        .session
        .core
}

/// Stand the predicted body's feet at `at`, on the ground.
fn feet(app: &mut App, at: Vec3) {
    use sim_core::movement::{quant_xz, quant_y};
    let body = &mut core(app).predict.body;
    body.qx = quant_xz(at.x);
    body.qy = quant_y(at.y);
    body.qz = quant_xz(at.z);
    body.qvy = 0;
    body.grounded = true;
}

/// Put the ears at `at`.
fn ears(app: &mut App, at: Vec3) {
    app.world_mut().resource_mut::<Eye>().pos = at;
}

/// Run `n` frames of [`STEP_S`] and collect every command they sent, in order.
///
/// Every frame, the mixer refused nothing: a refusal is a producer asking
/// for a positional cue with no place, or past the queue's cap, and either
/// is a sound the game never plays. `hurt` asked for the positional flesh
/// hit as an own cue, and every blow on you was silent, until this said so.
fn frames(app: &mut App, n: usize) -> Vec<Cmd> {
    let mut out = Vec::new();
    for _ in 0..n {
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(Duration::from_secs_f32(STEP_S));
        app.update();
        let cmds: Vec<Cmd> = app.world_mut().resource_mut::<Engine>().take().collect();
        assert_eq!(
            app.world().resource::<Sound>().mixer.dropped,
            0,
            "the mixer refused a request on a frame that sent {cmds:?}"
        );
        out.extend(cmds);
    }
    out
}

/// One admitted one-shot: `(cue, gain_l, gain_r, lp)`.
type Voice = (Cue, f32, f32, u8);

fn starts(cmds: &[Cmd]) -> Vec<Voice> {
    cmds.iter()
        .filter_map(|c| match *c {
            Cmd::Start {
                cue,
                gain_l,
                gain_r,
                lp,
                ..
            } => Some((cue, gain_l, gain_r, lp)),
            _ => None,
        })
        .collect()
}

fn of(cmds: &[Cmd], cue: Cue) -> Vec<Voice> {
    starts(cmds).into_iter().filter(|v| v.0 == cue).collect()
}

/// A bed's held slot.
fn slot(cue: Cue) -> u8 {
    BEDS.iter().position(|&c| c == cue).expect("a bed") as u8
}

/// Every level `cmds` sent `cue`'s slot, in order.
fn gains(cmds: &[Cmd], cue: Cue) -> Vec<f32> {
    let s = slot(cue);
    cmds.iter()
        .filter_map(|c| match *c {
            Cmd::Gain { slot, gain } if slot == s => Some(gain),
            _ => None,
        })
        .collect()
}

/// Every low-pass byte `cmds` placed `cue`'s slot at, in order.
fn places(cmds: &[Cmd], cue: Cue) -> Vec<u8> {
    let s = slot(cue);
    cmds.iter()
        .filter_map(|c| match *c {
            Cmd::Place { slot, lp, .. } if slot == s => Some(lp),
            _ => None,
        })
        .collect()
}

/// The ears' right in the frame Bevy draws (`tests/audio_systems.rs`).
fn bevy_right() -> Vec3 {
    let dir = Vec3::new(YAW.sin(), 0.0, YAW.cos());
    let mut t = Transform::default();
    t.look_to(dir, Vec3::Y);
    t.right().as_vec3()
}

/// An own cue is the interface's, at the head: both ears, equal, heard.
fn assert_own(v: Voice, cue: Cue) {
    let (c, l, r, _) = v;
    assert_eq!(c, cue);
    assert!(
        l > 0.0 && l == r,
        "{cue:?} is own and must be centred: {l} / {r}"
    );
}

fn is_falling(g: &[f32]) -> bool {
    g.windows(2).all(|w| w[1] <= w[0])
}

fn is_rising(g: &[f32]) -> bool {
    g.windows(2).all(|w| w[1] >= w[0])
}

/// Entering the world opens every bed, and then the whole chain runs —
/// every system's parameters resolve against a real session and island, or
/// this panics on its first update, as the game would. A world where
/// nothing happens is quiet but for its beds: the wind comes up, the
/// submerged bed stays shut above the water, and the surf is placed once,
/// dry.
#[test]
fn the_world_chain_runs_and_an_idle_world_is_quiet() {
    let (mut app, _replay) = boot();
    app.world_mut()
        .run_system_cached(audio::setup)
        .expect("setup ran");
    let opened: Vec<Cmd> = app.world_mut().resource_mut::<Engine>().take().collect();
    assert_eq!(opened.len(), BEDS.len(), "{opened:?}");

    let cmds = frames(&mut app, 30);
    assert!(
        starts(&cmds).is_empty(),
        "an idle world started {:?}",
        starts(&cmds)
    );
    let wind = gains(&cmds, Cue::BedWind);
    assert!(
        wind.last().is_some_and(|&g| g > 0.0) && is_rising(&wind),
        "the wind did not come up: {wind:?}"
    );
    let under = gains(&cmds, Cue::BedUnder);
    assert!(
        under.iter().all(|&g| g == 0.0),
        "the submerged bed sounded in the air: {under:?}"
    );
    assert_eq!(places(&cmds, Cue::BedSurf), vec![0], "the dry surf's place");
    assert_eq!(app.world().resource::<Engine>().dropped, 0);
}

/// A blow on you is the wet hit, Rust's, as your own: both ears, the row's
/// gain; the frame after is silent, and a fall nobody announced (hunger,
/// thirst) makes no sound.
#[test]
fn a_blow_on_you_lands_as_a_flesh_hit() {
    let (mut app, _replay) = world();
    let c = core(&mut app);
    c.hp_max = 100;
    c.hp = 100;
    // The first reading is silent: a join is not a fall.
    let first = frames(&mut app, 1);
    assert!(starts(&first).is_empty(), "the first health: {first:?}");
    assert_eq!(app.world().resource::<LastHp>().0, 100);

    core(&mut app).hp = 70;
    {
        let mut feed = app.world_mut().resource_mut::<Feed>();
        feed.hurts = 1;
        feed.hurt_damage = 30;
    }
    let s = starts(&frames(&mut app, 1));
    assert_eq!(s.len(), 1, "one blow: {s:?}");
    assert_own(s[0], Cue::FleshHitOwn);
    assert_eq!(app.world().resource::<LastHp>().0, 70);

    // `feed::drain` clears the feed every frame; with it clear, silence.
    *app.world_mut().resource_mut::<Feed>() = Feed::default();
    let after = frames(&mut app, 3);
    assert!(starts(&after).is_empty(), "the blow rang on: {after:?}");

    core(&mut app).hp = 60;
    let hungry = frames(&mut app, 3);
    assert!(
        starts(&hungry).is_empty(),
        "an unannounced fall was heard: {hungry:?}"
    );
}

/// A film's camera is not the recorder's body (`film_hears`): a blow on the
/// recorder is its own sound, and the film never hears it, wherever the
/// camera stands. Played at the Eye, it rang at the lens.
#[test]
fn a_film_does_not_hear_a_blow_on_the_recorder() {
    let (mut app, _replay) = world();
    app.world_mut().resource_mut::<Sound>().film = true;
    let c = core(&mut app);
    c.hp_max = 100;
    c.hp = 100;
    frames(&mut app, 1);

    core(&mut app).hp = 70;
    {
        let mut feed = app.world_mut().resource_mut::<Feed>();
        feed.hurts = 1;
        feed.hurt_damage = 30;
    }
    let s = starts(&frames(&mut app, 1));
    assert!(s.is_empty(), "the film heard the recorder's blow: {s:?}");
}

/// The ears going under turn the beds: the submerged bed comes up as the
/// wind goes, the surf goes through the water's low-pass, and coming back
/// up undoes all three.
#[test]
fn ears_under_water_turn_the_beds_and_muffle_the_surf() {
    let (mut app, _replay) = world();
    let dry = frames(&mut app, 30);
    let wind_dry = *gains(&dry, Cue::BedWind).last().expect("the wind rose");
    assert!(wind_dry > 0.0);
    assert!(gains(&dry, Cue::BedUnder).iter().all(|&g| g == 0.0));

    // Head under, feet on the bottom.
    feet(&mut app, Vec3::new(FEET.x, -3.0, FEET.z));
    ears(&mut app, Vec3::new(FEET.x, -3.0 + EYE_HEIGHT, FEET.z));
    let wet = frames(&mut app, 10);
    let under = gains(&wet, Cue::BedUnder);
    let wind = gains(&wet, Cue::BedWind);
    assert!(
        under.last().is_some_and(|&g| g > 0.0) && is_rising(&under),
        "the submerged bed did not come up: {under:?}"
    );
    assert!(
        wind.last() == Some(&0.0) && is_falling(&wind),
        "the wind did not go under: {wind:?} from {wind_dry}"
    );
    let rate = app.world().resource::<Engine>().out_rate;
    let lp = places(&wet, Cue::BedSurf);
    assert_eq!(
        lp.last(),
        Some(&engine::lp_under(1.0, rate)),
        "the surf under water: {lp:?}"
    );
    assert!(lp.iter().all(|&b| b > 0), "{lp:?}");

    // Back up.
    feet(&mut app, FEET);
    ears(&mut app, FEET + Vec3::Y * EYE_HEIGHT);
    let up = frames(&mut app, 30);
    let under = gains(&up, Cue::BedUnder);
    assert!(
        under.last() == Some(&0.0) && is_falling(&under),
        "the submerged bed stayed: {under:?}"
    );
    assert!(gains(&up, Cue::BedWind).last().is_some_and(|&g| g > 0.0));
    assert_eq!(
        places(&up, Cue::BedSurf).last(),
        Some(&0),
        "the surf stayed muffled"
    );
}

/// The feet breaking the surface splash, at the head and unmuffled — the
/// splash is the body's, the snapshot the ears', and wading in does not put
/// the ears under. Once a crossing, either way.
#[test]
fn feet_breaking_the_surface_splash_with_the_head_dry() {
    let (mut app, _replay) = world();
    let ear = Vec3::new(FEET.x, 1.2, FEET.z);
    ears(&mut app, ear);
    feet(&mut app, Vec3::new(FEET.x, 0.3, FEET.z));
    let wade = frames(&mut app, 3);
    assert!(of(&wade, Cue::Splash).is_empty(), "dry feet splashed");

    feet(&mut app, Vec3::new(FEET.x, -0.4, FEET.z));
    let cmds = frames(&mut app, 1);
    let s = of(&cmds, Cue::Splash);
    assert_eq!(s.len(), 1, "one crossing: {:?}", starts(&cmds));
    assert_own(s[0], Cue::Splash);
    assert_eq!(s[0].3, 0, "the head is dry: no water low-pass");
    assert!(
        gains(&cmds, Cue::BedUnder).iter().all(|&g| g == 0.0),
        "wading put the ears under"
    );

    let stand = frames(&mut app, 5);
    assert!(
        of(&stand, Cue::Splash).is_empty(),
        "standing in it splashed"
    );

    feet(&mut app, Vec3::new(FEET.x, 0.3, FEET.z));
    assert_eq!(
        of(&frames(&mut app, 1), Cue::Splash).len(),
        1,
        "walking out did not splash"
    );
}

/// A walk is heard on the ground underfoot: the island's own splat where
/// the feet are, read through the session's predictor.
#[test]
fn a_walk_steps_on_the_ground_underfoot() {
    let (mut app, _replay) = world();
    let _ = frames(&mut app, 1);
    let mut heard = 0;
    for i in 1..=30 {
        // 3 m/s along x.
        let at = FEET + Vec3::X * (0.3 * i as f32);
        feet(&mut app, at);
        let s = starts(&frames(&mut app, 1));
        let want = client::sound::steps::surface_cue(
            sim_core::terrain::splat(SEED, at.x, at.z),
            false,
            sim_core::terrain::cold(SEED, at.x, at.z, at.y),
        );
        for v in s.into_iter().filter(|v| v.0 != Cue::Brush) {
            assert_own(v, want);
            heard += 1;
        }
    }
    assert!(heard >= 5, "{heard} steps in 9 m");
}

/// Another body coming down from a jump is heard where it lands, on its
/// side; one coming down in the sea splashes. Off `remote_hands`, beside
/// `remote_steps` running on the same body.
#[test]
fn a_remote_body_landing_is_heard_where_it_lands() {
    let (mut app, _replay) = world();
    let right = bevy_right();
    let body = app
        .world_mut()
        .spawn((
            Body(7),
            Transform::from_translation(FEET + right * 4.0),
            BodyAnim {
                airborne: true,
                ..default()
            },
            RemoteSteps::default(),
            RemoteHands::default(),
        ))
        .id();
    let air = frames(&mut app, 5);
    assert!(starts(&air).is_empty(), "a body in the air: {air:?}");

    app.world_mut().get_mut::<BodyAnim>(body).unwrap().airborne = false;
    let s = starts(&frames(&mut app, 1));
    assert_eq!(s.len(), 1, "one landing: {s:?}");
    let (cue, l, r, _) = s[0];
    assert_eq!(cue, Cue::RemoteLand);
    assert!(r > 0.0 && l < 0.1 * r, "a landing on the right: {l} / {r}");

    // A step off a ledge is no landing.
    app.world_mut().get_mut::<BodyAnim>(body).unwrap().airborne = true;
    let _ = frames(&mut app, 1);
    app.world_mut().get_mut::<BodyAnim>(body).unwrap().airborne = false;
    let step = starts(&frames(&mut app, 1));
    assert!(step.is_empty(), "a short drop landed: {step:?}");

    // Into the sea.
    *app.world_mut().get_mut::<Transform>(body).unwrap() =
        Transform::from_translation(Vec3::new(FEET.x, -0.5, FEET.z) + right * 4.0);
    app.world_mut().get_mut::<BodyAnim>(body).unwrap().airborne = true;
    let _ = frames(&mut app, 5);
    app.world_mut().get_mut::<BodyAnim>(body).unwrap().airborne = false;
    let s = of(&frames(&mut app, 1), Cue::RemoteSplash);
    assert_eq!(s.len(), 1, "a landing in the sea did not splash");
}

/// A countdown passing a mark tolls the far bell through `feed`, as an own
/// cue at the head; the frames between marks are silent (`ui::arc::Toll`).
#[test]
fn a_countdown_tolls_through_the_feed() {
    use client::ui::arc::{TOLL_CUE, TOLL_LAST_MIN};
    use sim_core::limits::TICK_HZ;
    use sim_core::works::WORK_COUNTDOWN;
    const MIN: f64 = 60.0 * TICK_HZ as f64;
    const OPENS: u32 = 3_000_000;

    let (mut app, _replay) = world();
    let c = core(&mut app);
    c.arc.count = 1;
    let w = &mut c.arc.works[0];
    w.known = true;
    w.synced = true;
    w.state = WORK_COUNTDOWN;
    w.lights_at = OPENS;
    let at = |app: &mut App, left_min: f64| {
        core(app).clock.server_est = OPENS as f64 - left_min * MIN;
    };
    let last = TOLL_LAST_MIN as f64;

    // The first reading is silent: a join passed no mark.
    at(&mut app, last - 0.5);
    let join = frames(&mut app, 2);
    assert!(of(&join, TOLL_CUE).is_empty(), "the join tolled: {join:?}");

    // A minute on: one toll.
    at(&mut app, last - 1.5);
    let s = of(&frames(&mut app, 1), TOLL_CUE);
    assert_eq!(s.len(), 1, "the mark did not toll");
    assert_own(s[0], TOLL_CUE);

    let between = frames(&mut app, 3);
    assert!(
        of(&between, TOLL_CUE).is_empty(),
        "between marks: {between:?}"
    );
}
