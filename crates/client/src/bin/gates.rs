//! `cargo run -p client --features render --bin gates -- [addr]` — the
//! desktop client with a window on it. Slice 2 (DECISIONS.md 2026-08-05).
//!
//! **Bevy draws; it does not decide.** `sim-core` is the authority and
//! carries the walls — zero allocation in the tick, no `HashMap` iteration,
//! replay determinism — and `ClientCore` owns prediction and interpolation.
//! Everything below reads those and writes transforms. The day world state
//! starts living in Bevy components is the day those walls stop meaning
//! anything, and nothing in CI would catch it, so the boundary is a rule
//! rather than a preference: **no gameplay state in the ECS.**
//!
//! The session is a NON-SEND resource on purpose. It owns tokio channel
//! receivers, which are `Send` but not `Sync`, so it cannot be a plain
//! `Resource` — and that is the correct shape anyway: one owner, touched
//! from the main schedule only.

use bevy::prelude::*;
use client::{client_endpoint, Session};
use std::net::SocketAddr;

/// The connected session plus the runtime its reader tasks live on. The
/// runtime must outlive the session — dropping it would strand the event
/// and datagram readers.
struct Net {
    _rt: tokio::runtime::Runtime,
    session: Session,
}

/// Marks the cube standing in for one networked body, so the update system
/// can find it again by entity id.
#[derive(Component)]
struct Body(u32);

/// Marks the local player's own body.
#[derive(Component)]
struct Own;

fn main() {
    let server: SocketAddr = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:4433".into())
        .parse()
        .unwrap_or_else(|e| {
            eprintln!("gates: bad addr: {e}");
            std::process::exit(1);
        });

    // Connect before the window opens. A client that draws a world it is
    // not connected to is a client that lies for its first few frames.
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let session = rt.block_on(async {
        let endpoint = client_endpoint().unwrap_or_else(|e| {
            eprintln!("gates: {e}");
            std::process::exit(1);
        });
        Session::connect(&endpoint, server)
            .await
            .unwrap_or_else(|e| {
                eprintln!("gates: {e}");
                std::process::exit(1);
            })
    });
    println!(
        "gates: in the world — player {} seed {}",
        session.welcome.player_id, session.welcome.seed
    );

    let mut app = App::new();
    app.add_plugins(DefaultPlugins);
    app.insert_non_send_resource(Net { _rt: rt, session });
    app.add_systems(Startup, setup);
    app.add_systems(Update, (pump, draw_bodies).chain());
    app.run();
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(0.0, 8.0, 16.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    commands.spawn((
        DirectionalLight {
            shadows_enabled: true,
            ..default()
        },
        Transform::from_xyz(40.0, 80.0, 30.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    // A flat reference plane, NOT the terrain. The real heightfield is a
    // pure function of the seed in `sim_core::terrain` and both sides
    // already agree on it; meshing it is its own slice.
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(512.0, 512.0))),
        MeshMaterial3d(materials.add(Color::srgb(0.30, 0.29, 0.26))),
        Transform::from_xyz(0.0, 0.0, 0.0),
    ));
    // The local body. Spawned once; only its transform moves.
    commands.spawn((
        Own,
        Mesh3d(meshes.add(Cuboid::new(0.6, 1.8, 0.6))),
        MeshMaterial3d(materials.add(Color::srgb(0.85, 0.78, 0.55))),
        Transform::from_xyz(0.0, 0.9, 0.0),
    ));
}

/// One network frame per rendered frame. `pump` never awaits — the client
/// is a hot path too, and a frame that waits on a socket is a dropped
/// frame (CLAUDE.md traps).
fn pump(mut net: NonSendMut<Net>, time: Res<Time>) {
    let dt_ms = time.delta_secs_f64() * 1000.0;
    net.session.pump(dt_ms);
}

/// Read the core, write transforms. This is the whole render contract:
/// the local body comes from the predictor (so it answers input on the
/// frame it was pressed), every other body from the interpolator at the
/// render tick (so it is smooth and late rather than jittery and early).
fn draw_bodies(
    mut net: NonSendMut<Net>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut own_q: Query<&mut Transform, (With<Own>, Without<Body>)>,
    mut body_q: Query<(&Body, &mut Transform), Without<Own>>,
    mut camera_q: Query<&mut Transform, (With<Camera3d>, Without<Own>, Without<Body>)>,
) {
    let core = &net.session.core;
    let [x, y, z] = core.predict.render_position();

    for mut t in own_q.iter_mut() {
        t.translation = Vec3::new(x, y + 0.9, z);
    }
    // Third-person chase, deliberately dumb for now: a fixed offset is
    // enough to prove the loop, and camera direction is its own slice.
    for mut t in camera_q.iter_mut() {
        t.translation = Vec3::new(x, y + 8.0, z + 16.0);
        t.look_at(Vec3::new(x, y + 1.0, z), Vec3::Y);
    }

    let at = core.render_tick();
    let mut rs = client_wasm::interp::RemoteState::default();
    let ids: Vec<u32> = core.interp.ids().collect();
    for id in ids {
        if id == core.player_id || !core.interp.sample(id, at, &mut rs) {
            continue;
        }
        let mut found = false;
        for (b, mut t) in body_q.iter_mut() {
            if b.0 == id {
                t.translation = Vec3::new(rs.x, rs.y + 0.9, rs.z);
                found = true;
                break;
            }
        }
        if !found {
            commands.spawn((
                Body(id),
                Mesh3d(meshes.add(Cuboid::new(0.6, 1.8, 0.6))),
                MeshMaterial3d(materials.add(Color::srgb(0.55, 0.60, 0.70))),
                Transform::from_xyz(rs.x, rs.y + 0.9, rs.z),
            ));
        }
    }
}
