//! Render one local bot and share its viewpoint through a read-only web page.
use bevy::prelude::*;
use bevy::window::WindowResolution;
use client::render::{GatesRenderPlugin, Net, Rt, Settings, Start, WorldId};
use server::agent_demo::{bot_name, check_page, Door, MindArgs, AGENT_NAME, MIND_USAGE};
use server::explorer::Survivor;
use server::watch;
use server::watch::http::{Broadcast, SpectateLink};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

/// A continuous run's bot session: one that never times out. If it ever
/// ends, the process does, and the supervisor restarts it.
const CONTINUOUS_RUN: Duration = Duration::from_secs(30 * 86_400);
/// How long the bot has to join its shard.
const JOIN_TIMEOUT: Duration = Duration::from_secs(60);

fn usage() -> String {
    format!(
        "jev-watch [--seconds 120 | --continuous] [--server HOST:PORT [--cert-hash HEX]] [--listen 127.0.0.1:8081] [--assets PATH] [--content PATH] [--agent-key PATH] [--agent-name jev] [--spectate-page URL] {MIND_USAGE}\nStarts a temporary local shard and one survivor bot, or with --server plays on that shard (a wallet agent: --agent-key). Requires a graphics display.\nAn --external agent is told where its rendered view is served (GATES_FRAME_URL), so it can look at what its body sees.\nDeath is answered in-game; a continuous run ends only when its session or renderer fails, so use a supervisor to restart it.\nThe bot declares itself an agent a spectator can follow (NETCODE.md §2.3), signed with --agent-key's 0600 file if given. --spectate-page names a Gates web page, so the watch page can link viewers on this machine to a seat."
    )
}

fn run() -> Result<AppExit, String> {
    let mut mind = MindArgs::default();
    let mut agent_key: Option<PathBuf> = None;
    let mut agent_name = AGENT_NAME.to_owned();
    let mut spectate_page: Option<String> = None;
    let mut remote: Option<String> = None;
    let mut cert_hash: Option<String> = None;
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
            // A name, not a resolved address: the certificate is checked
            // against the name dialled.
            "--server" => remote = Some(args.next().ok_or("--server needs HOST:PORT")?),
            "--cert-hash" => cert_hash = Some(args.next().ok_or("--cert-hash needs a digest")?),
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
    if cert_hash.is_some() && remote.is_none() {
        return Err("--cert-hash is for --server; the local shard pins its own".into());
    }
    let assets = assets.canonicalize().map_err(|e| format!("assets: {e}"))?;
    if !assets.is_dir() {
        return Err("assets must name a directory".into());
    }
    let name = bot_name(&agent_name, 0, 1)?;
    let key = agent_key.as_deref().map(Door::load_key).transpose()?;
    if let Some(server) = &remote {
        // A guest plays only on this machine; say so before anything starts.
        if key.is_none() {
            Door::new(server.clone(), None, None)?;
        }
    }
    let (broadcast, capture) = Broadcast::start(listen)?;
    println!("shared watch page: http://{}/", broadcast.address);
    // An external agent may look at what its body sees: the frame this page
    // serves, from this machine.
    let frame_host = if broadcast.address.ip().is_unspecified() {
        SocketAddr::from(([127, 0, 0, 1], broadcast.address.port()))
    } else {
        broadcast.address
    };
    mind.frame_url = Some(format!("http://{frame_host}/frame.jpg"));
    let brain = mind.build()?;
    println!(
        "controller: {} · {} hands · {}",
        mind.label(),
        mind.skill.name(),
        mind.temperament.name()
    );
    let rt = tokio::runtime::Runtime::new().map_err(|e| e.to_string())?;
    struct Stop(std::sync::Arc<std::sync::atomic::AtomicBool>);
    impl Drop for Stop {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Relaxed);
        }
    }
    // Its own private island, or the shard named: there the bot plays as
    // any wallet agent does, and the window takes an ordinary seat.
    let (server, cert_hash, _stop) = match remote {
        Some(server) => (server, cert_hash, None),
        None => {
            let shard = rt.block_on(server::agent_demo::spawn_local_with_content(&content))?;
            let stop = Stop(shard.shutdown.clone());
            (
                shard.local_addr.to_string(),
                Some(shard.cert_hash.clone()),
                Some(stop),
            )
        }
    };
    let door = Arc::new(Door::new(server.clone(), cert_hash.clone(), key)?);
    // The bot plays its own session at the shard's tick rate, on the
    // runtime's threads; the window below only watches it (`watch`'s header
    // says why). Declared an agent either way, so a seat can follow it; a
    // key also proves the address, so a viewer can name this wallet.
    let (mut watched, bot) = watch::pair(Survivor::with(brain, mind.opts()));
    let live = bot.live.clone();
    {
        let door = door.clone();
        let run_for = if continuous { CONTINUOUS_RUN } else { duration };
        rt.spawn(async move {
            let result = match door.endpoint() {
                Ok(endpoint) => door.play(&endpoint, name, run_for, &mut watched).await,
                Err(why) => Err(why),
            };
            println!("{}", watch::report(&watched.survivor));
            if let Err(why) = &result {
                eprintln!("jev-watch: the bot's session ended: {why}");
            }
            watched.live.finish(result.is_ok());
        });
    }
    // A seat follows an agent in the world, so the bot is in first.
    rt.block_on(async {
        tokio::time::timeout(JOIN_TIMEOUT, async {
            while !live.joined() && live.ended().is_none() {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await
    })
    .map_err(|_| "the bot did not join its shard")?;
    if live.ended().is_some() {
        return Err("the bot's session ended before the window opened".into());
    }
    let target = door
        .key
        .as_ref()
        .map_or(protocol::Address::GUEST, |k| k.address());
    let (_endpoint, session) = rt.block_on(async {
        let endpoint = client::client_endpoint(&server, cert_hash.as_deref())?;
        let session = client::Session::watch(&endpoint, &server, target)
            .await
            .map_err(|e| e.to_string())?;
        Ok::<_, String>((endpoint, session))
    })?;
    let scope = if door.loopback() {
        "same machine only: this bot's shard listens on 127.0.0.1"
    } else {
        "anyone who can reach the shard: its seats follow agents live"
    };
    println!("spectator seats (NETCODE.md §2.3), {scope}:");
    match door.watch_url(spectate_page.as_deref()) {
        Some(url) => {
            println!("  browser: {url}");
            broadcast.announce(SpectateLink { url, scope });
        }
        None => println!(
            "  browser: a Gates web page with ?{}",
            door.spectate_query()
        ),
    }
    println!("  desktop: {}", door.desktop_command());
    // The welcome's blight salt too: this wipe's outbreaks are drawn from it.
    let world = WorldId {
        blight_salt: session.welcome.blight_salt,
        ..WorldId::with_haven(session.welcome.seed, *session.core.haven())
    };
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
        farm_sent: None,
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
    app.insert_non_send_resource(bot);
    if continuous {
        app.insert_resource(watch::Continuous);
    }
    app.insert_non_send_resource(capture);
    app.add_systems(Update, (watch::lifetime, watch::hide_seat_label));
    app.add_systems(
        PreUpdate,
        watch::clear_human_input.after(bevy::input::InputSystems),
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
