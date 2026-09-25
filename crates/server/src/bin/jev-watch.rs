//! Render one local bot and share its viewpoint through a read-only web page.
use bevy::prelude::*;
use bevy::window::WindowResolution;
use client::render::{input, GatesRenderPlugin, Net, Rt, Settings, Start, WorldId};
use server::agent_demo::{
    bot_name, check_page, spectate_url, Door, MindArgs, AGENT_NAME, MIND_USAGE,
};
use server::explorer::Survivor;
use server::watch::http::{Broadcast, SpectateLink};
use server::watch::{self, Controller};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::Duration;

fn usage() -> String {
    format!(
        "jev-watch [--seconds 120 | --continuous] [--listen 127.0.0.1:8081] [--assets PATH] [--content PATH] [--agent-key PATH] [--agent-name jev] [--spectate-page URL] {MIND_USAGE}\nStarts a temporary local shard and one survivor bot. Requires a graphics display.\nDeath is answered in-game; a continuous run ends only when its session or renderer fails, so use a supervisor to restart it.\nThe bot declares itself an agent a spectator can follow (NETCODE.md §2.3), signed with --agent-key's 0600 file if given. --spectate-page names a Gates web page, so the watch page can link viewers on this machine to a seat."
    )
}

fn run() -> Result<AppExit, String> {
    let mut mind = MindArgs::default();
    let mut agent_key: Option<PathBuf> = None;
    let mut agent_name = AGENT_NAME.to_owned();
    let mut spectate_page: Option<String> = None;
    let mut continuous = false;
    let mut timed = false;
    let mut duration = Duration::from_secs(120);
    let mut listen: SocketAddr = "127.0.0.1:8081".parse().expect("loopback");
    let mut content = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../content");
    let mut assets = std::env::current_dir()
        .map_err(|e| e.to_string())?
        .join("assets");
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if mind.take(&arg, &mut args)? {
            continue;
        }
        match arg.as_str() {
            "--help" | "-h" => {
                println!("{}", usage());
                return Ok(AppExit::Success);
            }
            "--continuous" => continuous = true,
            "--seconds" => {
                timed = true;
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
            "--content" => {
                content = PathBuf::from(args.next().ok_or("--content needs a directory")?)
            }
            // A path only: the key itself never rides argv (`agentkey`).
            "--agent-key" => {
                agent_key = Some(PathBuf::from(
                    args.next().ok_or("--agent-key needs a file path")?,
                ))
            }
            "--agent-name" => agent_name = args.next().ok_or("--agent-name needs a name")?,
            "--spectate-page" => {
                let page = args.next().ok_or("--spectate-page needs a URL")?;
                check_page(&page)?;
                spectate_page = Some(page);
            }
            _ => return Err(format!("unknown argument: {arg}")),
        }
    }
    if continuous && timed {
        return Err("choose --continuous or --seconds, not both".into());
    }
    let assets = assets.canonicalize().map_err(|e| format!("assets: {e}"))?;
    if !assets.is_dir() {
        return Err("assets must name a directory".into());
    }
    let name = bot_name(&agent_name, 0, 1)?;
    let key = agent_key.as_deref().map(Door::load_key).transpose()?;
    let brain = mind.build()?;
    let (broadcast, capture) = Broadcast::start(listen)?;
    println!("shared watch page: http://{}/", broadcast.address);
    println!("controller: {}", mind.label());
    let rt = tokio::runtime::Runtime::new().map_err(|e| e.to_string())?;
    let shard = rt.block_on(server::agent_demo::spawn_local_with_content(&content))?;
    struct Stop(std::sync::Arc<std::sync::atomic::AtomicBool>);
    impl Drop for Stop {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Relaxed);
        }
    }
    let _stop = Stop(shard.shutdown.clone());
    let server = shard.local_addr.to_string();
    let door = Door::new(server.clone(), Some(shard.cert_hash.clone()), key)?;
    let (_endpoint, mut session) = rt.block_on(async {
        let endpoint = client::client_endpoint(&server, Some(&shard.cert_hash))?;
        // Declared an agent either way, so a spectator can follow it; a key
        // also proves the address, so a viewer can name this wallet.
        let session = match &door.key {
            Some(key) => client::Session::connect_agent(&endpoint, &server, key, name).await,
            None => {
                client::Session::connect_as(
                    &endpoint,
                    &server,
                    protocol::Address::GUEST,
                    &client::Join::Agent { name },
                    |_, _, _| None,
                )
                .await
            }
        }
        .map_err(|e| e.to_string())?;
        Ok::<_, String>((endpoint, session))
    })?;
    println!(
        "spectator seats (NETCODE.md §2.3), this machine only — the shard listens on {server}:"
    );
    match &spectate_page {
        Some(page) => {
            let url = spectate_url(page, &door.spectate_query());
            println!("  browser: {url}");
            broadcast.announce(SpectateLink {
                url,
                scope: "same machine only: this bot's shard listens on 127.0.0.1",
            });
        }
        None => println!(
            "  browser: a Gates web page with ?{}",
            door.spectate_query()
        ),
    }
    println!("  desktop: {}", door.desktop_command());
    let controller = Controller::attach(&mut session, Survivor::new(brain), duration)?;
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
            pin_hour: None,
            pin_weather: None,
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
    if continuous {
        app.insert_resource(watch::Continuous);
    }
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
            eprintln!("jev-watch: {error}\n{}", usage());
            AppExit::error()
        }
    }
}
