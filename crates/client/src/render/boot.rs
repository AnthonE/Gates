//! The boot splash — the first thing a double-click puts on the screen.
//!
//! **The gap this closes was measured off the code, not guessed.** On the
//! launcher path (`--server`, which is what a depot's launch block passes —
//! `ci/depot.py`) everything below used to happen before a single pixel
//! existed: a blocking `Elo::discover` round trip over a local socket, a
//! blocking QUIC connect whose failure arm was `exit(1)`, wgpu adapter
//! enumeration and window creation, then — inside plugin build and `Startup`
//! — the generated sound bank, ten rasterised wheel rings, two TTF parses,
//! four texture loads and 57 icon PNGs. A player who double-clicked a
//! shortcut saw nothing at all for the whole of it, and a player whose shard
//! was down saw a line on a stderr they had no terminal for.
//!
//! Rust and League both put a window up first for exactly this reason. So do
//! we now, and the window is the app rather than a second process: the
//! blocking work moved *inside*, where it can be reported and recovered from.
//!
//! **Three things fall out of that, and the third is the real prize.**
//!
//! 1. The window is up in the time it takes wgpu to pick an adapter, and
//!    everything after that is drawn.
//! 2. The identity handshake runs on a thread and is *reported* — the
//!    `Elo::discover` call is the same one, moved off the pre-window path.
//! 3. **A failed launcher connect is survivable.** It lands on the server
//!    list with its reason, because this screen hands off to
//!    `Screen::Connecting` — which already owns a timeout, an Esc, and a
//!    failure arm that writes the reason into the menu's status line. That
//!    state machine existed and the launcher path was the one door that
//!    bypassed it.
//!
//! **`--capture` still connects before the window and must keep doing so.**
//! The probe harness is a gate: a client that draws a world it is not
//! connected to lies for its first few frames, and a gate that photographs
//! halfway through a handshake is a gate whose frames depend on the network.
//! `gates.rs` has that argument in full; this module honours it by treating
//! an already-connected start as a boot with nothing to greet.
//!
//! Nothing here reads a clock. The screen ends when [`crate::ui::boot::Boot`]
//! says both waits are answered, which is the same observable-state settle
//! the loading screen and the capture harness use.

use bevy::prelude::*;

use super::menu::{Connecting, Menu, Screen};
use super::{hub, icons, ui};
use crate::elo::{Elo, Player};
use crate::ui::boot::{Boot, Next};

/// Who the launcher says is playing.
///
/// **Moved off `gates.rs` and into a state**, which is the whole point of
/// this module: it used to be resolved before the window by a blocking
/// socket call. Not gameplay state — nothing in the sim reads it and nothing
/// can — so "Bevy draws, it does not decide" is untouched.
#[derive(Resource)]
pub struct Who(pub Player);

impl Default for Who {
    fn default() -> Self {
        Self(Player::Anonymous)
    }
}

/// The splash's own state: the model, plus the in-flight handshake.
///
/// Non-send-free on purpose — the receiver is tokio's for the reason
/// `menu::Menu` states about its own: a `Resource` must be `Send + Sync` and
/// `std`'s `Receiver` is not `Sync`.
#[derive(Resource)]
pub struct Warmup {
    pub boot: Boot,
    /// The launcher handshake, or `None` once collected.
    ///
    /// **The whole `Elo` comes back, not just the `Player`.** The first cut
    /// sent the player and dropped the connection, which closed the socket
    /// that could sign a challenge, name a shard list and open a page — three
    /// seams the SDK is vendored for. It is held now (`Launcher`).
    greet: Option<tokio::sync::mpsc::UnboundedReceiver<Elo>>,
    /// What `--identity` declared, held so the thread can answer without
    /// reaching back into the app.
    declared: Option<String>,
    /// Set when there is no launcher to ask — a capture run, `--no-launcher`,
    /// or a start that already resolved its player before the window.
    skip_greet: bool,
}

impl Warmup {
    pub fn new(chosen: bool, declared: Option<String>, skip_greet: bool) -> Self {
        Self {
            boot: Boot::new(chosen),
            greet: None,
            declared,
            skip_greet,
        }
    }
}

/// Marks everything the splash owns.
#[derive(Component)]
pub struct BootRoot;

/// The line that names the outstanding wait.
#[derive(Component)]
pub struct BootLine;

/// The filled part of the two-step bar.
#[derive(Component)]
pub struct BootBar;

/// Ask the launcher who is playing, on a thread.
///
/// `Elo::discover` opens a unix socket and waits for an answer. Run inside a
/// system that would be a frozen first frame — the exact defect this whole
/// module exists to remove, reintroduced one layer in — so it goes to a
/// thread and the frame loop only ever `try_recv`s, which is
/// `menu::begin_fetch`'s shape and for the same reason.
pub fn begin_greet(mut warm: ResMut<Warmup>) {
    if warm.skip_greet {
        // Nothing to ask. The answer is already whatever `gates.rs` resolved,
        // and the wait is over before it starts.
        warm.boot.greeted = true;
        return;
    }
    if warm.greet.is_some() {
        return;
    }
    let declared = warm.declared.clone();
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    std::thread::spawn(move || {
        let _ = tx.send(Elo::discover(
            declared.as_deref(),
            env!("CARGO_PKG_VERSION"),
        ));
    });
    warm.greet = Some(rx);
}

/// The screen. A wordmark, a line that says what is outstanding, and a bar
/// two steps wide.
///
/// Deliberately plain: it is on screen for as long as an asset load takes and
/// it is the first impression, so it says the game's name and reports
/// honestly. A spinner would be a clock, and this repo does not put clocks on
/// screens that have real state to report.
pub fn setup(mut commands: Commands, warm: Res<Warmup>) {
    commands.spawn((Camera2d, BootRoot));
    commands.spawn((
        BootRoot,
        ui::screen(ui::BG),
        children![
            ui::wordmark(),
            (
                BootLine,
                ui::label(warm.boot.line(), 13.0, ui::DIM),
                Node {
                    margin: UiRect::top(Val::Px(18.0)),
                    ..default()
                },
            ),
            (
                Node {
                    width: Val::Px(280.0),
                    height: Val::Px(4.0),
                    margin: UiRect::top(Val::Px(12.0)),
                    ..default()
                },
                BackgroundColor(ui::ROW_IDLE),
                children![(
                    BootBar,
                    Node {
                        width: Val::Percent(0.0),
                        height: Val::Percent(100.0),
                        ..default()
                    },
                    BackgroundColor(ui::ACCENT),
                )],
            ),
        ],
    ));
}

/// Collect both waits, drive the bar, and hand off.
///
/// **Readiness is asked of the asset server, not counted in frames.** Every
/// handle `Startup` created is queried for its load state; a splash that
/// lifted after N frames would lift early on a cold page cache and late on a
/// warm one, which is the clock-shaped bug `CLAUDE.md` names as its own trap
/// class.
#[allow(clippy::too_many_arguments)]
pub fn update(
    mut commands: Commands,
    mut warm: ResMut<Warmup>,
    mut who: ResMut<Who>,
    mut menu: ResMut<Menu>,
    mut state: ResMut<hub::HubState>,
    assets: Res<AssetServer>,
    icons: Option<Res<icons::Icons>>,
    mut line: Query<&mut Text, With<BootLine>>,
    mut bar: Query<&mut Node, With<BootBar>>,
    mut next: ResMut<NextState<Screen>>,
) {
    // ---- wait 1: is the client warm? ----------------------------------
    // `Icons` is the loader that holds a handle per file, so it is both the
    // readiness question and the proof `Startup` has run at all — the
    // resource does not exist before it. Absent means "not warm", which is
    // what makes the splash cover `Startup` rather than only what `Startup`
    // started.
    if !warm.boot.warm {
        warm.boot.warm = icons.is_some_and(|i| i.handles().all(|h| settled(&assets, h)));
    }

    // ---- wait 2: who is playing? --------------------------------------
    if let Some(rx) = &mut warm.greet {
        match rx.try_recv() {
            Ok(elo) => {
                warm.greet = None;
                warm.boot.greeted = true;
                land(&mut commands, &mut who, &mut menu, &mut state, elo);
            }
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {}
            // The thread died without answering. Anonymous is the correct
            // fallback and it is also the common case (no launcher running),
            // so this is a normal end rather than an error.
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                warm.greet = None;
                warm.boot.greeted = true;
            }
        }
    }

    if let Ok(mut t) = line.single_mut() {
        let want = warm.boot.line();
        if t.0 != want {
            t.0 = want.to_string();
        }
    }
    if let Ok(mut node) = bar.single_mut() {
        node.width = Val::Percent(warm.boot.fraction() * 100.0);
    }

    match warm.boot.next() {
        Some(Next::Menu) => next.set(Screen::Menu),
        Some(Next::Connect) => next.set(Screen::Connecting),
        None => {}
    }
}

/// Take everything the launcher just told us, and keep the connection.
///
/// **Three of these four were being thrown away**, and each is the same class
/// of defect as `Connecting::address`: a field the SDK fills, that this client
/// computed and then dropped on the floor, with nothing failing.
///
/// 1. **The player** — the only one that was already used.
/// 2. **The shard list url.** `Elo::discover` has always asked the launcher
///    for it (`Overlay::servers_url`) and stored it in a field nothing read,
///    so a player who started the game from elo still got *"no shard
///    list to fetch - pass --servers URL"* on a menu the launcher had just
///    told us how to fill. `--servers` still wins, because a flag typed by
///    hand is more specific than a default.
/// 3. **The manifest url**, which is what NEWS / ITEM STORE / WORKSHOP hang
///    off — the fetch starts here so it is in flight while the splash is
///    still up, rather than when the player first clicks one.
/// 4. **The connection itself**, held from here on. It is non-send because
///    `Overlay` owns a `UnixStream`; `Commands` cannot insert one of those,
///    so it goes through the world the same way `Net` does.
fn land(
    commands: &mut Commands,
    who: &mut Who,
    menu: &mut Menu,
    state: &mut hub::HubState,
    mut elo: Elo,
) {
    who.0 = elo.player.clone();
    info!("gates: {}", who.0.line());

    if menu.servers_url.is_none() {
        if let Some(url) = elo.servers_url.clone() {
            info!("gates: the launcher names a shard list - {url}");
            menu.status = format!("fetching the shard list from {url}");
            menu.servers_url = Some(url);
        }
    }

    state.hub.launcher = elo.connected();
    if state.hub.launcher {
        match elo.title_url() {
            Some(url) => hub::begin_fetch(state, url),
            // The launcher is up and does not know this title. A real answer,
            // and one a player can act on (reinstall from the launcher), so it
            // is said rather than left as a spinner.
            None => {
                state.hub.why =
                    Some("the launcher does not know this title - no manifest to read".into())
            }
        }
    }

    commands.queue(move |world: &mut World| {
        world.insert_non_send_resource(elo);
    });
}

/// Has this handle stopped moving?
///
/// **A failed load counts as settled, and an untracked one does too.** Both
/// are terminal — a missing PNG draws a gap and will never arrive, and a
/// handle the asset server does not know about will never progress — so
/// treating either as pending would be a splash that never lifts. That is
/// the one failure mode worse than the black window this screen replaced,
/// and it is why the match is written positively rather than as
/// `!is_loaded(...)`.
fn settled(assets: &AssetServer, h: &Handle<Image>) -> bool {
    use bevy::asset::LoadState;
    match assets.get_load_state(h.id()) {
        Some(LoadState::NotLoaded) | Some(LoadState::Loading) => false,
        Some(LoadState::Loaded) | Some(LoadState::Failed(_)) | None => true,
    }
}

/// Seed the connect screen's address on the way out.
///
/// The launcher path never "picked" a row, so the field every downstream
/// screen names the shard from has to be filled here — it used to be filled
/// at plugin build, which was correct only because the connect had already
/// happened by then.
pub fn teardown(
    mut commands: Commands,
    roots: Query<Entity, With<BootRoot>>,
    mut connecting: NonSendMut<Connecting>,
    direct: Res<Direct>,
) {
    if connecting.addr.is_empty() {
        connecting.addr = direct.0.clone();
    }
    for e in roots.iter() {
        commands.entity(e).despawn();
    }
    // The same line `world_teardown` and `poll_connect` print, for the same
    // reason: the states a player passes through should be readable off a log
    // when the screen is not available. It is also the only evidence a
    // headless run can give that the splash *lifted* — the failure mode that
    // matters here is a client that stays on it forever, and a screen that
    // never ends looks exactly like a screen that is still working.
    info!("gates: warmed up - leaving the splash");
}

/// The address this binary was started with. Held as a resource so the splash
/// can hand it to the connect screen without reaching into the plugin's own
/// construction arguments.
#[derive(Resource)]
pub struct Direct(pub String);
