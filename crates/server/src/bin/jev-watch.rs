//! Render one local bot and share its viewpoint through a read-only web page.
use bevy::prelude::*;
use bevy::window::WindowResolution;
use client::render::{input, GatesRenderPlugin, Net, Rt, Settings, Start, WorldId};
use server::jev::{Driver, Jev, REQUEST_TIMEOUT, THINK_INTERVAL};
use server::watch::{self, http::Broadcast, Controller};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::Duration;

const USAGE: &str = "jev-watch [--scripted] [--seconds 120] [--listen 127.0.0.1:8081] [--assets PATH]\nStarts a temporary local shard and one wood-collecting bot. Requires a graphics display.\nJev requires TYPESAFE_API_KEY; --scripted is an explicitly labelled offline controller.";

fn run() -> Result<AppExit, String> {
    let mut scripted = false;
    let mut duration = Duration::from_secs(120);
    let mut listen: SocketAddr = "127.0.0.1:8081".parse().expect("loopback");
    let mut assets = std::env::current_dir()
        .map_err(|e| e.to_string())?
        .join("assets");
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => {
                println!("{USAGE}");
                return Ok(AppExit::Success);
            }
            "--scripted" => scripted = true,
            "--seconds" => {
                let seconds: u64 = args
                    .next()
                    .ok_or("--seconds needs a value")?
                    .parse()
                    .map_err(|_| "invalid seconds")?;
                if !(1..=86400).contains(&seconds) {
                    return Err("run duration must be 1..86400 seconds".into());
                }
                duration = Duration::from_secs(seconds);
            }
            "--listen" => {
                listen = args
                    .next()
                    .ok_or("--listen needs an address")?
                    .parse()
                    .map_err(|_| "invalid listen address")?
            }
            "--assets" => assets = PathBuf::from(args.next().ok_or("--assets needs a directory")?),
            _ => return Err(format!("unknown argument: {arg}")),
        }
    }
    let assets = assets.canonicalize().map_err(|e| format!("assets: {e}"))?;
    if !assets.is_dir() {
        return Err("assets must name a directory".into());
    }
    let driver = if scripted {
        Driver::new(
            server::agent_demo::Scripted,
            THINK_INTERVAL,
            REQUEST_TIMEOUT,
        )?
    } else {
        let key = std::env::var("TYPESAFE_API_KEY")
            .map_err(|_| "set TYPESAFE_API_KEY, or explicitly choose --scripted")?;
        Driver::new(
            Jev::new(key, REQUEST_TIMEOUT)?,
            THINK_INTERVAL,
            REQUEST_TIMEOUT,
        )?
    };
    let (broadcast, capture) = Broadcast::start(listen)?;
    println!("shared watch page: http://{}/", broadcast.address);
    println!(
        "controller: {}",
        if scripted {
            "SCRIPTED; no model calls"
        } else {
            server::jev::MODEL
        }
    );
    let rt = tokio::runtime::Runtime::new().map_err(|e| e.to_string())?;
    let shard = rt.block_on(server::agent_demo::spawn_local())?;
    struct Stop(std::sync::Arc<std::sync::atomic::AtomicBool>);
    impl Drop for Stop {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Relaxed);
        }
    }
    let _stop = Stop(shard.shutdown.clone());
    let server = shard.local_addr.to_string();
    let (_endpoint, mut session) = rt.block_on(async {
        let endpoint = client::client_endpoint(&server, Some(&shard.cert_hash))?;
        let session =
            client::Session::connect(&endpoint, &server, protocol::Address::GUEST, |_, _, _| None)
                .await
                .map_err(|e| e.to_string())?;
        Ok::<_, String>((endpoint, session))
    })?;
    let controller = Controller::attach(
        &mut session,
        server::explorer::Gatherer::new(driver),
        scripted,
        duration,
    )?;
    let world = WorldId::new(session.welcome.seed);
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .build()
            .disable::<bevy::audio::AudioPlugin>()
            .set(AssetPlugin {
                file_path: assets.to_string_lossy().into_owned(),
                ..default()
            })
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: "Gates · shared bot view".into(),
                    resolution: WindowResolution::new(watch::WIDTH, watch::HEIGHT)
                        .with_scale_factor_override(1.0),
                    resizable: false,
                    ..default()
                }),
                ..default()
            }),
    );
    app.insert_non_send_resource(Rt(rt));
    app.insert_resource(world);
    app.insert_non_send_resource(Net {
        session,
        sel: 0,
        light: false,
    });
    app.add_plugins(GatesRenderPlugin {
        start: Start {
            direct: server,
            servers_url: None,
            connected: true,
            chosen: true,
            identity: None,
            no_launcher: true,
            no_hud: false,
        },
        capture: None,
    });
    // This broadcaster has its own fixed window. Leave the operator's saved
    // player preferences untouched; ordinary rendering/lighting stays shared.
    app.world_mut()
        .remove_resource::<client::render::settings::Disk>();
    app.insert_resource(Settings {
        fullscreen: false,
        vsync: false,
        max_fps: sim_core::limits::TICK_HZ as u16,
        ..default()
    });
    app.insert_non_send_resource(controller);
    app.insert_non_send_resource(capture);
    app.add_systems(Update, watch::lifetime);
    app.add_systems(
        PreUpdate,
        watch::clear_human_input.after(bevy::input::InputSystems),
    );
    app.add_systems(
        Update,
        watch::drive
            .after(input::gather)
            .before(input::place_eye)
            .run_if(client::render::world_running),
    );
    app.add_systems(
        PostUpdate,
        watch::capture.run_if(client::render::world_running),
    );
    let result = app.run();
    drop(app);
    drop(broadcast);
    Ok(result)
}

fn main() -> AppExit {
    match run() {
        Ok(result) => result,
        Err(error) => {
            eprintln!("jev-watch: {error}\n{USAGE}");
            AppExit::error()
        }
    }
}
