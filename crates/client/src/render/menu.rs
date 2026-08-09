//! The intro screen: pick a shard, then join it.
//!
//! **What this changes structurally.** Before this module the client
//! connected *before* the window existed and `exit(1)`'d if it could not —
//! a design that is correct for a probe harness and impossible for a player,
//! because the one thing a player must be able to do after a failed connect
//! is try a different server. Connecting is therefore a **state** here, not
//! a precondition, and the screens are the states:
//!
//! ```text
//!   Menu ──pick──▶ Connecting ──welcome──▶ Loading ──built──▶ InWorld
//!    ▲   ▲              │                     │                  │
//!    │   └why it failed─┘                     │              Esc │
//!    │                                        │                  ▼
//!    └────────── Disconnect ──────────────────┴───────────── Paused
//!                                                                │
//!    Settings ◀───────────────────────────────────────────────────
//! ```
//!
//! **`Connecting` ends at the welcome, not at a playable world.** The states
//! after it are the other two modules in this trio: `loading` owns the
//! interval where the rings fill (the welcome carries a seed, and a seed is
//! not a world), and `pause` owns the Esc menu, which is this screen seen from
//! inside the world. `settings` hangs off both.
//!
//! The `Disconnect` arrow above is the *voluntary* leave. Its involuntary
//! twin — the shard hanging up mid-play — lands on `Disconnected` first
//! (`disconnected`), a screen that says so, and comes back here through it.
//!
//! **Bevy still does not decide.** Nothing in this module touches gameplay
//! state; it owns an address string and a list of rows, and the moment a
//! `Session` exists it hands it to `Net` and gets out of the way. `WorldId`
//! cannot be built until the welcome names the seed, which is exactly why it
//! is inserted by `poll_connect` rather than at plugin construction — the menu
//! is what made the old shape impossible, not a preference.
//!
//! **Two doors are deliberately NOT this screen.** A `--capture` run and a
//! `--server` launch both skip it: the probe harness is a gate and must not
//! wait on a click, and a player who already picked a shard in the scry
//! launcher's own Servers window has chosen once and must not be asked
//! twice. `args::server_given` is that bit, and an unfilled `{server}`
//! placeholder deliberately does not set it — a launcher that started the
//! game without picking a shard lands here, which is the intended path.

use bevy::prelude::*;

use super::ui;
use crate::shardlist::{self, Shard, MAX_DOC_BYTES};

/// Where the client is. `Menu` is the default because the *absence* of a
/// chosen server is the state a bare `gates` starts in; the two callers that
/// have already chosen (`--server`, `--capture`) jump straight to `Loading`,
/// which is the first state that has a world to build.
#[derive(States, Default, Debug, Clone, PartialEq, Eq, Hash)]
pub enum Screen {
    #[default]
    Menu,
    Connecting,
    /// The welcome has landed and the rings are filling. `loading`.
    Loading,
    InWorld,
    /// The Esc menu. The world is still connected and still pumping. `pause`.
    Paused,
    /// This body died and has not answered the respawn yet. The world is
    /// still connected, still pumping and still streaming behind the wash —
    /// what stops is `input::gather`, so a corpse does not walk or swing.
    /// `death`.
    Dead,
    /// The island map. A screen you read: the pointer is released, the world
    /// keeps pumping behind it, and `input::gather` stands down. `map`.
    Map,
    /// Reachable from `Menu` and from `Paused`; `settings::Settings::back`
    /// carries which. `settings`.
    Settings,
    /// The shard hung up mid-play — the INVOLUNTARY half of leaving, where
    /// `pause::Verb::Disconnect` is the verb a player takes. The world is
    /// torn down on entry (the session under it is dead, and a live world
    /// drawn over a dead wire is a lie), and the screen names the reason
    /// before offering the way back. `disconnected`.
    Disconnected,
}

/// One row a player may join. The direct row is always present and always
/// first — a shard list that fails to load must never leave the player with
/// nothing to click, which is the same rule the launcher's dark panels obey.
#[derive(Debug, Clone)]
pub struct Row {
    pub name: String,
    pub addr: String,
    pub detail: String,
    /// The list row this came from. `None` on the Direct row, which is not a
    /// shard anybody published — it is the address this binary was started
    /// with, and there is no document behind it to poll.
    ///
    /// Kept whole rather than as loose fields so `detail` has exactly one
    /// place it is computed: a poll rewrites the count *in the shard* and the
    /// line is re-derived, which is what keeps a refreshed row from losing
    /// the map and ping it already had.
    pub shard: Option<Shard>,
}

impl Row {
    fn direct(addr: &str) -> Self {
        Self {
            name: "Direct".into(),
            addr: addr.to_string(),
            detail: "the address this client was started with".into(),
            shard: None,
        }
    }

    /// The second line of a row: population, then whatever else it states.
    fn detail_of(s: &Shard) -> String {
        let mut detail = s.population();
        if let Some(m) = &s.map {
            detail.push_str("  ");
            detail.push_str(m);
        }
        if let Some(p) = s.ping_ms {
            detail.push_str(&format!("  {p} ms"));
        }
        detail
    }

    fn from_shard(s: &Shard) -> Self {
        Self {
            name: s.name.clone(),
            addr: s.addr.clone(),
            detail: Self::detail_of(s),
            shard: Some(s.clone()),
        }
    }

    /// Where this row's live count is polled from, if anywhere.
    fn status_url(&self) -> Option<&str> {
        self.shard.as_ref()?.status_url.as_deref()
    }

    /// Fold a poll in. Returns whether the drawn line actually changed, which
    /// is what decides a rebuild: re-spawning the whole screen every ten
    /// seconds to redraw an identical `3/100` is a visible flicker for
    /// nothing.
    fn apply_status(&mut self, st: &shardlist::Status) -> bool {
        let Some(s) = self.shard.as_mut() else {
            return false;
        };
        s.apply_status(st);
        let redrawn = Self::detail_of(s);
        let changed = redrawn != self.detail;
        self.detail = redrawn;
        changed
    }
}

/// The menu's whole state. A plain resource — no gameplay state, no session.
#[derive(Resource)]
pub struct Menu {
    pub rows: Vec<Row>,
    /// What the screen says under the title. Always something: a menu that
    /// is empty and silent about why is the defect both repos call a dark
    /// panel that cannot say what would light it.
    pub status: String,
    /// Set when `rows` grew and the screen has to be rebuilt. An explicit
    /// flag rather than a `Local` row count, because a count starts at zero
    /// and would make the first frame in the menu rebuild what `setup` had
    /// just spawned — the screen built twice on every entry.
    pub dirty: bool,
    pub servers_url: Option<String>,
    /// The in-flight shard-list fetch. `None` once it has been collected.
    ///
    /// **tokio's channel, not `std::sync::mpsc`**, and the reason is a Bevy
    /// one: a `Resource` must be `Send + Sync`, and `std`'s `Receiver` is
    /// `Send` but *not* `Sync`. Holding one here makes `Menu` a non-send
    /// resource, which drags every system that touches the status line onto
    /// the main thread for no benefit. tokio's is `Sync`, its unbounded
    /// sender is not async, and tokio is already a dependency.
    fetch: Option<tokio::sync::mpsc::UnboundedReceiver<Result<Vec<Shard>, String>>>,
    /// The in-flight round of status polls, one per row that names an
    /// endpoint. Collected as a batch rather than per row so the frame does
    /// one `try_recv` however many shards are listed.
    status_poll: Option<tokio::sync::mpsc::UnboundedReceiver<Vec<(usize, shardlist::Status)>>>,
    /// Seconds since the last round went out. Bevy's frame delta, not an
    /// `Instant` — the screen has one clock and it is the renderer's, which
    /// is the same rule `Connecting::waited_s` follows.
    since_poll: f32,
}

impl Menu {
    pub fn new(direct: &str, servers_url: Option<String>) -> Self {
        Self {
            rows: vec![Row::direct(direct)],
            status: match &servers_url {
                Some(u) => format!("fetching the shard list from {u}"),
                // The honest empty state, and it names what would fill it.
                None => "no shard list to fetch - pass --servers URL, or start \
                         the game from the scry launcher's Servers window"
                    .into(),
            },
            dirty: false,
            servers_url,
            fetch: None,
            status_poll: None,
            since_poll: 0.0,
        }
    }
}

/// How often the menu re-polls every listed shard's `/status.json`, in
/// seconds.
///
/// **PROPOSED — `DECISIONS.md` §open, "shard status poll v0".** The count is
/// polled rather than read out of the shard list because a baked number is
/// stale the moment the document is written (`shardlist.rs` module docs). Ten
/// seconds is chosen against what the number is *for*: a player deciding which
/// of four shards to join needs "busy or empty", not a live scoreboard, and
/// the endpoint is a plain TCP GET a shard answers serially on one thread.
/// With the sixty-four-row cap that is at most 6.4 requests a second across
/// the whole list from one client, and a shard that does not answer costs the
/// row nothing — it keeps whatever count it had.
pub const STATUS_POLL_SECS: f32 = 10.0;

/// How long a single status poll may take before it is abandoned, in seconds.
///
/// **PROPOSED — `DECISIONS.md` §open, "shard status poll v0".** Shorter than
/// `STATUS_POLL_SECS` on purpose: a round must finish before the next one is
/// due, or a slow shard would stack rounds until the list is polling itself
/// in a loop. Comfortably past a transatlantic round trip, and a shard that
/// misses it reads as `?` rather than as zero.
pub const STATUS_TIMEOUT_S: u64 = 4;

/// How long the connect screen waits before giving up, in seconds.
///
/// **PROPOSED — `DECISIONS.md` §open, "connect timeout v0".** It exists
/// because of a measured failure, not a guess: clicking a shard that is not
/// running leaves the screen on "connecting to …" indefinitely. QUIC to a
/// closed UDP port gets no refusal — there is no TCP RST to deliver — so the
/// attempt sits in quinn's handshake until *its* timeout, and the player has
/// no way back. Measured under Xvfb against a dead loopback shard: still
/// connecting 18 s after the click.
///
/// 20 s is the conservative end: comfortably past a slow handshake across an
/// ocean, and short enough that a dead shard reads as dead rather than as a
/// hung client. Esc cancels immediately regardless, which is the half that
/// needs no number.
pub const CONNECT_TIMEOUT_S: f32 = 20.0;

/// The in-flight connection attempt. Held here rather than awaited inline:
/// `block_on` in a system freezes the window for the whole of a failing
/// connect, which on an unreachable host is seconds of a frozen title
/// screen and reads exactly like a crash.
#[derive(Default)]
pub struct Connecting {
    pub addr: String,
    /// The address this connect claims (`Address::GUEST` for none), carried
    /// here rather than re-read at connect time because the connect runs on
    /// the runtime, off the frame, and must not reach back into the app for
    /// it.
    pub address: protocol::Address,
    pub rx: Option<std::sync::mpsc::Receiver<Result<crate::Session, String>>>,
    /// Seconds spent on this attempt. Accumulated from Bevy's frame delta
    /// rather than an `Instant`, so the screen has one clock and it is the
    /// renderer's.
    pub waited_s: f32,
}

/// The tokio runtime, owned for the life of the app.
///
/// Split out of `Net` because the runtime now has to exist *before* any
/// session does — the connect attempt runs on it, and a failed attempt
/// leaves no `Net` behind to have held it.
pub struct Rt(pub tokio::runtime::Runtime);

/// Marks the menu's root node, so leaving the screen despawns exactly it.
#[derive(Component)]
pub struct MenuRoot;

/// A clickable row, by index into `Menu::rows`.
#[derive(Component)]
pub struct RowButton(pub usize);

/// The one row on this screen that is not a shard.
#[derive(Component)]
pub struct SettingsButton;

/// The status line.
#[derive(Component)]
pub struct StatusLine;

/// Kick off the shard-list fetch, on a thread.
///
/// `ureq` is blocking and that is fine *here* and nowhere else: this runs
/// once on entering the menu, on its own thread, and the frame loop only
/// ever does a `try_recv`. The client is a hot path too (CLAUDE.md traps) —
/// a synchronous GET inside a system would be a multi-second frame.
pub fn begin_fetch(mut menu: ResMut<Menu>) {
    let Some(url) = menu.servers_url.clone() else {
        return;
    };
    if menu.fetch.is_some() {
        return;
    }
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    std::thread::spawn(move || {
        let _ = tx.send(fetch_blocking(&url));
    });
    menu.fetch = Some(rx);
}

/// One GET, capped. Separated so the thread body stays readable and so the
/// cap is visible next to the read rather than buried in a builder chain.
fn fetch_blocking(url: &str) -> Result<Vec<Shard>, String> {
    let mut res = ureq::get(url)
        .call()
        .map_err(|e| format!("shard list: {e}"))?;
    let bytes = res
        .body_mut()
        .with_config()
        // Wall 4 applied to a network read: a server that streams forever
        // must not be able to hold the menu open or grow the heap. One byte
        // over and this errors rather than truncating, which is the same
        // refuse-don't-truncate policy `shardlist::parse` states.
        .limit((MAX_DOC_BYTES + 1) as u64)
        .read_to_vec()
        .map_err(|e| format!("shard list: {e}"))?;
    shardlist::parse(&bytes)
}

/// Collect the fetch if it has landed. Non-blocking, every frame.
pub fn poll_fetch(mut menu: ResMut<Menu>) {
    let Some(rx) = &mut menu.fetch else {
        return;
    };
    let got = match rx.try_recv() {
        Ok(got) => got,
        Err(tokio::sync::mpsc::error::TryRecvError::Empty) => return,
        // The thread died without sending. Say so rather than spinning.
        Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
            menu.fetch = None;
            menu.status = "the shard list fetch did not finish".into();
            return;
        }
    };
    menu.fetch = None;
    match got {
        Ok(shards) if shards.is_empty() => {
            // Not an error, and not drawn as one: "nobody is running a shard
            // right now" is a real answer and the launcher renders it too.
            menu.status = "the shard list is up and lists no shards".into();
        }
        Ok(shards) => {
            menu.status = format!("{} shard(s)", shards.len());
            menu.rows.extend(shards.iter().map(Row::from_shard));
            menu.dirty = true;
            // Poll the counts immediately rather than after the interval: the
            // rows have just appeared reading `?`, and the whole point of the
            // endpoint is that they do not stay that way while the player
            // looks at them.
            menu.since_poll = STATUS_POLL_SECS;
        }
        // The reason is drawn verbatim. `shardlist::parse` writes these for
        // a reader, which is why they name the row and the cap.
        Err(why) => menu.status = why,
    }
}

/// Send a round of status polls, on a thread, at most one round at a time.
///
/// **This is what makes the count real.** The shard list names where each
/// shard answers `GET /status.json` and this is the half that asks — see
/// `shardlist.rs`'s module docs for why the number is not simply written into
/// the document by the generator.
///
/// One thread for the whole round, walked serially. A thread per row would be
/// sixty-four threads on a full list to save a few hundred milliseconds of
/// something nobody is waiting on, and the frame loop only ever `try_recv`s
/// either way.
pub fn begin_status_poll(time: Res<Time>, mut menu: ResMut<Menu>) {
    if menu.status_poll.is_some() {
        // A round is still out. Do not stack another on top of it — that is
        // how a slow shard turns a ten-second poll into a busy loop.
        return;
    }
    menu.since_poll += time.delta_secs();
    if menu.since_poll < STATUS_POLL_SECS {
        return;
    }
    menu.since_poll = 0.0;

    let targets: Vec<(usize, String)> = menu
        .rows
        .iter()
        .enumerate()
        .filter_map(|(i, r)| r.status_url().map(|u| (i, u.to_string())))
        .collect();
    if targets.is_empty() {
        return;
    }

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    std::thread::spawn(move || {
        let got: Vec<(usize, shardlist::Status)> = targets
            .iter()
            // A shard that does not answer contributes NOTHING to this vec.
            // That is the whole honesty rule in one `filter_map`: the row keeps
            // the count it had, rather than being zeroed by a timeout and
            // reading as "everyone left".
            .filter_map(|(i, url)| Some((*i, status_blocking(url).ok()?)))
            .collect();
        let _ = tx.send(got);
    });
    menu.status_poll = Some(rx);
}

/// One status GET, capped and timed out. The sibling of `fetch_blocking`, and
/// bounded for the same reason: this is a request to a host named in a
/// document fetched from another host.
fn status_blocking(url: &str) -> Result<shardlist::Status, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(STATUS_TIMEOUT_S)))
        .build()
        .into();
    let mut res = agent.get(url).call().map_err(|e| format!("status: {e}"))?;
    let bytes = res
        .body_mut()
        .with_config()
        .limit((shardlist::MAX_STATUS_BYTES + 1) as u64)
        .read_to_vec()
        .map_err(|e| format!("status: {e}"))?;
    shardlist::parse_status(&bytes)
}

/// Collect a finished round and redraw only if a number moved.
pub fn poll_status(mut menu: ResMut<Menu>) {
    let Some(rx) = &mut menu.status_poll else {
        return;
    };
    let got = match rx.try_recv() {
        Ok(got) => got,
        Err(tokio::sync::mpsc::error::TryRecvError::Empty) => return,
        // The thread died without sending. Drop the round and let the timer
        // start the next one; a status poll is not worth a status line, which
        // belongs to the shard list itself.
        Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
            menu.status_poll = None;
            return;
        }
    };
    menu.status_poll = None;
    let mut changed = false;
    for (i, st) in got {
        if let Some(row) = menu.rows.get_mut(i) {
            changed |= row.apply_status(&st);
        }
    }
    if changed {
        menu.dirty = true;
    }
}

/// Build the screen. Rebuilt from scratch on entry, so the row list is
/// whatever `Menu` holds at that moment.
pub fn setup(mut commands: Commands, menu: Res<Menu>) {
    build(&mut commands, &menu);
}

/// The UI, as a plain function rather than a system.
///
/// Two callers need it — `setup` on entering the screen and
/// `rebuild_on_new_rows` when the fetch lands — and a system cannot call
/// another system directly. Extracting it is cheaper than registering a
/// cached one-shot system and keeps both paths provably identical.
fn build(commands: &mut Commands, menu: &Menu) {
    // A camera, because the menu may be the first thing that ever exists —
    // the render rig only spawns on entering the world.
    commands.spawn((Camera2d, MenuRoot));

    commands
        .spawn((MenuRoot, ui::screen(ui::BG)))
        .with_children(|root| {
            root.spawn(ui::title("GATES"));
            root.spawn((
                StatusLine,
                ui::label(menu.status.clone(), 15.0, ui::DIM),
                Node {
                    max_width: Val::Px(620.0),
                    margin: UiRect::bottom(Val::Px(14.0)),
                    ..default()
                },
            ));

            for (i, row) in menu.rows.iter().enumerate() {
                root.spawn((ui::row(ROW_W), RowButton(i)))
                    .with_children(|b| {
                        // The number is drawn because the key works — a bind the
                        // player is never told about is a bind that does not
                        // exist, which is the rule `LEARN_TASKS` already holds
                        // the in-world verbs to.
                        b.spawn(ui::strong(
                            format!("{}  {}", i + 1, row.name),
                            20.0,
                            ui::TEXT,
                        ));
                        b.spawn(ui::label(
                            format!("{}   {}", row.addr, row.detail),
                            13.0,
                            ui::DIM,
                        ));
                    });
            }

            // Settings, which is the same screen the Esc menu opens. Drawn
            // apart from the shard rows and keyed on a letter rather than a
            // number, because the numbers belong to the list — a settings row
            // that took the next digit would renumber itself every time a
            // shard appeared.
            root.spawn((ui::row(ROW_W), SettingsButton))
                .with_children(|b| {
                    b.spawn(ui::strong("S  Settings", 20.0, ui::TEXT));
                    b.spawn(ui::label(
                        "view, controls, screen - and the keybind list",
                        13.0,
                        ui::DIM,
                    ));
                });

            root.spawn((
                ui::label(
                    "click a shard, or press its number    -    S settings    -    Esc quits",
                    13.0,
                    ui::FAINT,
                ),
                Node {
                    margin: UiRect::top(Val::Px(16.0)),
                    ..default()
                },
            ));
        });
}

/// Row width, pixels — the shard rows and the settings row are one column.
const ROW_W: f32 = 560.0;

/// Redraw the status line in place. The rows are rebuilt only on entry, so
/// this is what makes an in-flight fetch visible.
pub fn refresh_status(menu: Res<Menu>, mut line: Query<&mut Text, With<StatusLine>>) {
    if !menu.is_changed() {
        return;
    }
    if let Ok(mut t) = line.single_mut() {
        if t.0 != menu.status {
            t.0 = menu.status.clone();
        }
    }
}

/// Rows arriving after the screen was built need the screen rebuilt. Cheap
/// and rare — it happens once, when the fetch lands.
pub fn rebuild_on_new_rows(
    mut commands: Commands,
    mut menu: ResMut<Menu>,
    roots: Query<Entity, With<MenuRoot>>,
) {
    if !menu.dirty {
        return;
    }
    menu.dirty = false;
    for e in roots.iter() {
        commands.entity(e).despawn();
    }
    build(&mut commands, &menu);
}

/// Mouse. One click picks a shard, or opens settings. Hover is `ui::hover`'s
/// job now, which is why this only reads `Pressed`: one hover handler across
/// four screens cannot disagree with itself.
pub fn click(
    rows: Query<(&Interaction, &RowButton), Changed<Interaction>>,
    settings: Query<&Interaction, (Changed<Interaction>, With<SettingsButton>)>,
    mut picked: ResMut<Picked>,
) {
    for (interaction, row) in rows.iter() {
        if *interaction == Interaction::Pressed {
            picked.0 = Some(Pick::Row(row.0));
        }
    }
    for interaction in settings.iter() {
        if *interaction == Interaction::Pressed {
            picked.0 = Some(Pick::Settings);
        }
    }
}

/// Keyboard. Number keys pick a shard, S opens settings, Esc leaves.
pub fn keys(
    keyboard: Res<ButtonInput<KeyCode>>,
    menu: Res<Menu>,
    mut picked: ResMut<Picked>,
    // `MessageWriter`, not `EventWriter` — Bevy 0.18 renamed events to
    // messages and the prelude carries only the new name.
    mut exit: MessageWriter<AppExit>,
) {
    if keyboard.just_pressed(KeyCode::Escape) {
        exit.write(AppExit::Success);
        return;
    }
    if keyboard.just_pressed(KeyCode::KeyS) {
        picked.0 = Some(Pick::Settings);
        return;
    }
    const DIGITS: [KeyCode; 9] = [
        KeyCode::Digit1,
        KeyCode::Digit2,
        KeyCode::Digit3,
        KeyCode::Digit4,
        KeyCode::Digit5,
        KeyCode::Digit6,
        KeyCode::Digit7,
        KeyCode::Digit8,
        KeyCode::Digit9,
    ];
    for (i, k) in DIGITS.iter().enumerate() {
        if keyboard.just_pressed(*k) && i < menu.rows.len() {
            picked.0 = Some(Pick::Row(i));
        }
    }
}

/// What this screen can be asked for. Two things, and only one of them is a
/// row — which is why the pick is an enum rather than the row index it used
/// to be: a settings entry smuggled in as `rows.len()` would have been an
/// index that means "not a row" and nothing in the type would say so.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pick {
    Row(usize),
    Settings,
}

/// What was chosen this frame, if anything. A resource rather than an event
/// so the two input paths cannot both fire a connect in one frame.
#[derive(Resource, Default)]
pub struct Picked(pub Option<Pick>);

/// Turn a pick into a state change.
pub fn take_pick(
    mut picked: ResMut<Picked>,
    menu: Res<Menu>,
    mut settings: ResMut<super::settings::Settings>,
    mut connecting: NonSendMut<Connecting>,
    mut next: ResMut<NextState<Screen>>,
) {
    let Some(pick) = picked.0.take() else {
        return;
    };
    let i = match pick {
        Pick::Settings => {
            settings.back = Screen::Menu;
            settings.dirty = false;
            next.set(Screen::Settings);
            return;
        }
        Pick::Row(i) => i,
    };
    let Some(row) = menu.rows.get(i) else {
        return;
    };
    connecting.addr = row.addr.clone();
    connecting.rx = None;
    next.set(Screen::Connecting);
}

/// Leave the menu: despawn everything it owns.
pub fn teardown(mut commands: Commands, roots: Query<Entity, With<MenuRoot>>) {
    for e in roots.iter() {
        commands.entity(e).despawn();
    }
}

/// Start the connect, on the runtime, off the frame.
pub fn begin_connect(rt: NonSend<Rt>, mut connecting: NonSendMut<Connecting>) {
    let addr = connecting.addr.clone();
    let address = connecting.address;
    connecting.waited_s = 0.0;
    let (tx, rx) = std::sync::mpsc::channel();
    rt.0.spawn(async move {
        let result = match crate::client_endpoint() {
            Ok(endpoint) => {
                crate::Session::connect(&endpoint, &addr, address, crate::scry::sign_siwe).await
            }
            Err(e) => Err(e),
        };
        let _ = tx.send(result);
    });
    connecting.rx = Some(rx);
}

/// A one-line "connecting to …" screen. Deliberately not the menu greyed
/// out: the menu is despawned, so there is nothing to click twice.
pub fn connecting_screen(mut commands: Commands, connecting: NonSend<Connecting>) {
    commands.spawn((Camera2d, MenuRoot));
    commands.spawn((
        MenuRoot,
        ui::screen(ui::BG),
        children![
            ui::strong(
                format!("connecting to {}...", connecting.addr),
                22.0,
                ui::TITLE
            ),
            // Says the way out, because there has to be one: a shard that is
            // not running never refuses a QUIC connect, it just goes quiet.
            ui::label("Esc goes back", 13.0, ui::FAINT),
        ],
    ));
}

/// Collect the connect. Success inserts `Net`; failure goes back to the menu
/// **with the reason**, which is the entire point of the state machine —
/// the old path called `exit(1)` here and the player never saw a word of it.
pub fn poll_connect(
    mut commands: Commands,
    mut connecting: NonSendMut<Connecting>,
    mut menu: ResMut<Menu>,
    mut next: ResMut<NextState<Screen>>,
    time: Res<Time>,
    keyboard: Res<ButtonInput<KeyCode>>,
) {
    if connecting.rx.is_none() {
        return;
    }

    // Two ways out that do not depend on the transport answering. Without
    // them a shard that is simply not running holds this screen forever:
    // QUIC gets no refusal from a closed UDP port, so the attempt waits on
    // quinn's own handshake timeout with nothing on screen but "connecting".
    connecting.waited_s += time.delta_secs();
    let gave_up = if keyboard.just_pressed(KeyCode::Escape) {
        Some("cancelled".to_string())
    } else if connecting.waited_s >= CONNECT_TIMEOUT_S {
        Some(format!("no answer after {CONNECT_TIMEOUT_S:.0}s"))
    } else {
        None
    };
    if let Some(why) = gave_up {
        // The attempt is abandoned, not aborted: dropping the receiver is
        // enough, because nothing reads it outside this state and the task
        // ends on its own send failure.
        connecting.rx = None;
        menu.status = format!("{}: {why}", connecting.addr);
        next.set(Screen::Menu);
        return;
    }

    let Some(rx) = &connecting.rx else {
        return;
    };
    let got = match rx.try_recv() {
        Ok(got) => got,
        Err(std::sync::mpsc::TryRecvError::Empty) => return,
        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
            Err("the connect attempt did not finish".to_string())
        }
    };
    connecting.rx = None;
    match got {
        Ok(session) => {
            info!(
                "gates: in the world — player {} seed {} tick {}",
                session.welcome.player_id, session.welcome.seed, session.welcome.tick
            );
            commands.insert_resource(super::WorldId::new(session.welcome.seed));
            // `Commands` has no `insert_non_send_resource`, so this goes
            // through the world directly. `Net` is non-send because the
            // session owns tokio channel receivers (`render::mod`).
            commands.queue(move |world: &mut World| {
                world.insert_non_send_resource(super::Net { session, sel: 0 });
            });
            // `Loading`, not `InWorld`: the welcome names a seed and the seed
            // is not a world. What comes next is three rings and a far mesh at
            // one build of each per frame, and the player used to watch that
            // happen from inside it.
            next.set(Screen::Loading);
        }
        Err(why) => {
            menu.status = format!("{}: {why}", connecting.addr);
            next.set(Screen::Menu);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_direct_row_is_always_there() {
        // A shard list that never loads must still leave something to click.
        let m = Menu::new("127.0.0.1:4433", None);
        assert_eq!(m.rows.len(), 1);
        assert_eq!(m.rows[0].addr, "127.0.0.1:4433");
        // ...and the empty menu says what would fill it, rather than being
        // silently bare.
        assert!(m.status.contains("--servers"), "{}", m.status);
    }

    fn shard(addr: &str) -> Shard {
        Shard {
            id: "a".into(),
            name: "A".into(),
            addr: addr.into(),
            players: None,
            max_players: None,
            map: None,
            ping_ms: None,
            status_url: None,
        }
    }

    #[test]
    fn a_row_never_invents_a_population() {
        // `?`, never `0/0` — the row states what it knows and no more.
        assert_eq!(Row::from_shard(&shard("h:1")).detail, "?");
    }

    #[test]
    fn a_poll_rewrites_the_count_and_keeps_the_rest_of_the_line() {
        // The regression this row shape exists to prevent: `detail` used to
        // be a string built once, so folding a count in would have had to
        // rebuild it — and the obvious rebuild loses the map and the ping.
        let mut s = shard("h:1");
        s.map = Some("island 20260731".into());
        s.ping_ms = Some(31);
        let mut row = Row::from_shard(&s);
        assert_eq!(row.detail, "?  island 20260731  31 ms");

        let st = shardlist::parse_status(br#"{"players":3,"max_players":100}"#).unwrap();
        assert!(row.apply_status(&st), "the line changed and must redraw");
        assert_eq!(row.detail, "3/100  island 20260731  31 ms");

        // The same count again is not a redraw. Rebuilding the screen every
        // ten seconds to draw an identical line is a flicker for nothing.
        assert!(!row.apply_status(&st), "an unchanged line must not redraw");
    }

    #[test]
    fn only_a_listed_shard_is_polled() {
        // The Direct row is the address this binary was started with, not a
        // shard anybody published — there is no document behind it naming a
        // status endpoint, and it must not be invented.
        let m = Menu::new("127.0.0.1:4433", None);
        assert_eq!(m.rows[0].status_url(), None);
        let st = shardlist::parse_status(br#"{"players":3,"max_players":100}"#).unwrap();
        let mut direct = Row::direct("127.0.0.1:4433");
        assert!(!direct.apply_status(&st), "the direct row has no count to set");
        assert_eq!(direct.detail, "the address this client was started with");

        // A listed shard that names one is polled; one that does not, is not.
        let mut s = shard("h:1");
        assert_eq!(Row::from_shard(&s).status_url(), None);
        s.status_url = Some("https://h:8080/status.json".into());
        assert_eq!(
            Row::from_shard(&s).status_url(),
            Some("https://h:8080/status.json")
        );
    }

    #[test]
    fn the_poll_interval_outlasts_its_own_timeout() {
        // A round must finish before the next is due, or a slow shard stacks
        // rounds until the list is polling itself in a loop. `begin_status_poll`
        // also refuses to start a second round, so this is belt and braces —
        // but the constant that makes it true should not drift silently.
        assert!(
            (STATUS_TIMEOUT_S as f32) < STATUS_POLL_SECS,
            "a poll can outlive its own interval"
        );
    }
}
