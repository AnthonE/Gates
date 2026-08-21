//! The front end: a wordmark, a nav column, and whatever the picked nav entry
//! puts beside it. Today that is one thing — the server browser.
//!
//! **What this changes structurally.** Before this module the client
//! connected *before* the window existed and `exit(1)`'d if it could not —
//! a design that is correct for a probe harness and impossible for a player,
//! because the one thing a player must be able to do after a failed connect
//! is try a different server. Connecting is therefore a **state** here, not
//! a precondition, and the screens are the states:
//!
//! ```text
//!   Boot ──ready──▶ Menu ──pick──▶ Connecting ──welcome──▶ Loading ──▶ InWorld
//!    │               ▲   ▲              │                     │           │
//!    └──chosen───────┼───┴why it failed─┘                     │       Esc │
//!                    │                                        │           ▼
//!                    └────── Disconnect ──────────────────────┴──── Paused
//!                                                                     │
//!    Settings ◀────────────────────────────────────────────────────────
//! ```
//!
//! **`Boot` is the splash a double-click needs** and it is new (`boot`): the
//! window now exists before the launcher handshake and before any connect,
//! so the several seconds that used to be a black nothing are drawn, and the
//! launcher's own connect goes through `Connecting` like every other — which
//! is what makes a dead shard survivable on that path rather than an
//! `exit(1)` into a terminal nobody has.
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
//! ## The shell, and why it is the part worth copying
//!
//! The five reference frames the operator handed over — main menu, PLAY GAME,
//! NEWS, INVENTORY, WORKSHOP — are **one screen with five payloads**: the
//! wordmark never moves, the nav column never re-orders, and picking an entry
//! swaps a tinted control column and a wide content pane. `render/ui.rs`'s
//! shell section owns that arrangement; this module is its first tenant.
//!
//! **The nav carries NEWS, ITEM STORE and WORKSHOP, and the first cut of this
//! file was wrong to leave them off.** It argued they had no service behind
//! them; they have one — the elo launcher (operator, 2026-08-09) — so
//! the argument was a mistake about the product, not an application of the
//! rule. `settings.rs`'s policy is that a section with nothing behind it
//! *says so*, and `crate::ui::hub` is that sentence: four states, never
//! collapsed, from "no launcher is running" to a link. Each hands off to the
//! launcher's own window rather than drawing a shopfront here — see
//! `ui::hub` for why that is a rule about money and not a shortcut.
//!
//! The one entry of theirs we still do not carry is `RUST+`, their companion
//! app. There is no equivalent to be honest about.
//!
//! **The backdrop is footage, not a live scene** (operator, same day: *"rust
//! uses a video for the background"*). `ui::backdrop` owns it and the
//! correction it replaced: rendering the island behind the menu is the
//! expensive way to buy the cheap thing.
//!
//! **Bevy still does not decide.** Nothing in this module touches gameplay
//! state; it owns an address string and a list of rows, and the moment a
//! `Session` exists it hands it to `Net` and gets out of the way. The
//! browser's arithmetic — filtering, the busy-first sort, the favourite set —
//! is `crate::ui::servers`, gated headless in `tests/ui.rs` §L. `WorldId`
//! cannot be built until the welcome names the seed, which is exactly why it
//! is inserted by `poll_connect` rather than at plugin construction.
//!
//! **One door is deliberately NOT this screen.** A `--capture` run skips it:
//! the probe harness is a gate and must not wait on a click. A launcher join
//! (`args::server_given`) no longer skips it either — it passes *through*
//! `Boot` straight into `Connecting`, which is the same "asked once, not
//! twice" outcome with a failure arm that lands here instead of on stderr.

use bevy::prelude::*;

use super::boot::Who;
use super::hub::HubState;
use super::ui;
use crate::shardlist::{self, Shard, MAX_DOC_BYTES};
use crate::ui::hub::{Hub, Section};
use crate::ui::servers::{self, Cat, Favourites, Filter, Listing};

/// Where the client is. `Boot` is the default because the *absence* of a
/// warmed client is the state every launch starts in, including the two that
/// have already chosen a shard — they leave it for `Connecting` rather than
/// for `Menu`, which is one bit and lives in `crate::ui::boot`.
#[derive(States, Default, Debug, Clone, PartialEq, Eq, Hash)]
pub enum Screen {
    /// The splash. The window exists, the client is warming, and nothing has
    /// been asked of the player yet. `boot`.
    #[default]
    Boot,
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

/// The menu's whole state. A plain resource — no gameplay state, no session.
#[derive(Resource)]
pub struct Menu {
    pub rows: Vec<Listing>,
    /// What the screen says under the table. Always something: a menu that
    /// is empty and silent about why is the defect both repos call a dark
    /// panel that cannot say what would light it.
    pub status: String,
    /// Set when the drawn screen no longer matches this resource. An explicit
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
}

impl Menu {
    pub fn new(direct: &str, servers_url: Option<String>) -> Self {
        Self {
            rows: vec![Listing::direct(direct)],
            status: match &servers_url {
                Some(u) => format!("fetching the shard list from {u}"),
                // The honest empty state, and it names what would fill it.
                None => "no shard list to fetch - pass --servers URL, or start \
                         the game from the elo launcher's Servers window"
                    .into(),
            },
            dirty: false,
            servers_url,
            fetch: None,
            status_poll: None,
        }
    }
}

/// What the player has narrowed the list to, and which nav entry is open.
///
/// Split from [`Menu`] because the two change for different reasons and at
/// very different rates: `Menu` moves when the network answers, this moves on
/// every keystroke. One resource for both would mark the fetch's own change
/// detection dirty on every letter typed into the search box.
#[derive(Resource, Default)]
pub struct Browse {
    pub nav: Nav,
    pub filter: Filter,
    pub favourites: Favourites,
}

/// The nav column, in the reference's own order.
///
/// **The three middle entries are the launcher's, not ours** (operator,
/// 2026-08-09: NEWS is the elo community, and the item store and
/// workshop are part of that setup too). The first cut left all three off on
/// the grounds that nothing was behind them — wrong about the product, not
/// about the rule. `crate::ui::hub` owns which of four states each is in;
/// what a pane must never do is draw a greyed row that says nothing.
///
/// The one entry the reference has that we still do not is `RUST+`, its
/// companion app. There is no equivalent to be honest about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Nav {
    #[default]
    Play,
    /// A launcher-backed section: NEWS, ITEM STORE, WORKSHOP.
    Hub(Section),
    Settings,
    Quit,
}

impl Nav {
    const ALL: [Nav; 6] = [
        Nav::Play,
        Nav::Hub(Section::News),
        Nav::Hub(Section::Store),
        Nav::Hub(Section::Workshop),
        Nav::Settings,
        Nav::Quit,
    ];

    /// The drawn label. A hub entry may be renamed by the manifest, so the
    /// platform can rename a section of its own site without a client
    /// release — [`crate::ui::hub::Section::label`] is where the blank-and-
    /// whitespace guard lives.
    fn label(self, hub: &Hub) -> String {
        match self {
            Nav::Play => "PLAY GAME".to_string(),
            Nav::Hub(s) => s.label(hub),
            Nav::Settings => "OPTIONS".to_string(),
            Nav::Quit => "QUIT".to_string(),
        }
    }

    /// Does picking this entry open a pane, or act at once? QUIT and OPTIONS
    /// are verbs; everything else is a screen. Drawn identically either way,
    /// which is the reference's arrangement too.
    fn is_pane(self) -> bool {
        matches!(self, Nav::Play | Nav::Hub(_))
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
    /// The address this connect claims (`Address::GUEST` for none).
    ///
    /// Written by [`begin_connect`] from `Who` and then copied into the task,
    /// because the connect runs on the runtime, off the frame, and must not
    /// reach back into the app for it. It was a field nobody wrote until
    /// 2026-08-09 — see [`begin_connect`] for what that cost.
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

/// The star on a row, by the same index. A separate button inside the row so
/// starring does not join — the reference's star is a hit target of its own
/// for exactly that reason.
#[derive(Component)]
pub struct StarButton(pub usize);

/// A nav entry.
#[derive(Component)]
pub struct NavButton(pub Nav);

/// A rail category.
#[derive(Component)]
pub struct CatButton(pub Cat);

/// One of the two slot filters.
#[derive(Component)]
pub struct SlotFilter {
    /// True for "hide empty", false for "hide full".
    pub empty: bool,
}

/// OPEN IN LAUNCHER, on a hub pane.
#[derive(Component)]
pub struct HubButton(pub Section);

/// CLEAR FILTERS and REFRESH.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
pub enum PanelButton {
    Clear,
    Refresh,
}

/// The status line under the table.
#[derive(Component)]
pub struct StatusLine;

/// Kick off the shard-list fetch, on a thread.
///
/// `ureq` is blocking and that is fine *here* and nowhere else: this runs
/// once on entering the menu, on its own thread, and the frame loop only
/// ever does a `try_recv`. The client is a hot path too (CLAUDE.md traps) —
/// a synchronous GET inside a system would be a multi-second frame.
pub fn begin_fetch(mut menu: ResMut<Menu>) {
    if menu.fetch.is_some() {
        // Already out. Entering the menu twice must not stack two GETs — the
        // guard REFRESH deliberately does not have, which is why the two
        // callers share the body below and not this line.
        return;
    }
    fetch_now(&mut menu);
}

/// Start a fetch, replacing any in flight.
///
/// Two callers: [`begin_fetch`] on entering the screen, and REFRESH, which is
/// a player saying "ask again now" and must not be silently dropped because a
/// slow round is still out.
fn fetch_now(menu: &mut Menu) {
    let Some(url) = menu.servers_url.clone() else {
        return;
    };
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
            menu.dirty = true;
        }
        Ok(shards) => {
            menu.status = format!("{} shard(s)", shards.len());
            // Replace rather than extend: REFRESH re-enters this path, and an
            // extend would double every row on the second fetch.
            menu.rows.retain(|r| r.direct);
            menu.rows.extend(shards.iter().map(Listing::from_shard));
            menu.dirty = true;
            // `begin_status_poll` notices the row count moved and polls at
            // once rather than after the interval: the rows have just appeared
            // reading `?`, and the whole point of the endpoint is that they do
            // not stay that way while the player looks at them.
        }
        // The reason is drawn verbatim. `shardlist::parse` writes these for
        // a reader, which is why they name the row and the cap.
        Err(why) => {
            menu.status = why;
            menu.dirty = true;
        }
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
/// **The timer is a `Local`, not a field on `Menu`, and that is a change-
/// detection fix rather than a style choice.** `ResMut` flags its resource
/// changed on the first *mutable* deref, so a `menu.since_poll += delta` every
/// frame would mark `Menu` changed every frame — and the redraw is guarded on
/// `menu.dirty` precisely to avoid doing work when nothing moved. Keeping the
/// clock outside the resource means this system touches `Menu` mutably only on
/// the frame it actually starts a round.
pub fn begin_status_poll(
    time: Res<Time>,
    mut since: Local<f32>,
    mut seen_rows: Local<usize>,
    mut menu: ResMut<Menu>,
) {
    if menu.status_poll.is_some() {
        // A round is still out. Do not stack another on top of it — that is
        // how a slow shard turns a ten-second poll into a busy loop.
        return;
    }
    // The list just grew (or was replaced). Poll now rather than leaving fresh
    // rows reading `?` for ten seconds — see `poll_fetch`.
    if *seen_rows != menu.rows.len() {
        *seen_rows = menu.rows.len();
        *since = STATUS_POLL_SECS;
    }
    *since += time.delta_secs();
    if *since < STATUS_POLL_SECS {
        return;
    }
    *since = 0.0;

    let targets: Vec<(usize, String)> = menu
        .rows
        .iter()
        .enumerate()
        .filter_map(|(i, r)| r.status_url.as_ref().map(|u| (i, u.to_string())))
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
    // Checked through an IMMUTABLE deref first, so the overwhelmingly common
    // frame — no round in flight — does not flag `Menu` changed for nothing.
    if menu.status_poll.is_none() {
        return;
    }
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
pub fn setup(
    mut commands: Commands,
    menu: Res<Menu>,
    browse: Res<Browse>,
    who: Res<Who>,
    state: Res<HubState>,
    backdrop: Option<Res<ui::Backdrop>>,
) {
    build(
        &mut commands,
        &menu,
        &browse,
        &who,
        &state.hub,
        backdrop.as_deref(),
    );
}

/// Redraw. **One rebuild path for every reason the screen can change** — a
/// fetch landing, a poll moving a count, a keystroke, a star, a filter — and
/// it is a full respawn rather than a patch.
///
/// The same call `settings::rebuild` makes and for the same stated reason: a
/// screen whose every drawn value is derived from two resources is cheaper to
/// rebuild than to diff, and a patch path is exactly where a drawn value
/// drifts from the held one. Which matters more here than there, because the
/// values on this screen are *numbers a player acts on*.
pub fn rebuild(
    mut commands: Commands,
    mut menu: ResMut<Menu>,
    browse: Res<Browse>,
    who: Res<Who>,
    mut state: ResMut<HubState>,
    backdrop: Option<Res<ui::Backdrop>>,
    roots: Query<Entity, With<MenuRoot>>,
) {
    // Either resource landing something redraws the one screen they share —
    // the manifest arriving changes a nav LABEL, not just a pane, so a hub
    // update that only redrew the pane would leave the column stale.
    if !menu.dirty && !state.dirty {
        return;
    }
    menu.dirty = false;
    state.dirty = false;
    for e in roots.iter() {
        commands.entity(e).despawn();
    }
    build(
        &mut commands,
        &menu,
        &browse,
        &who,
        &state.hub,
        backdrop.as_deref(),
    );
}

/// Column widths, pixels. The nav is wide enough for the longest entry at 26
/// px; the panel matches the reference's proportion against a 1280 frame.
const NAV_W: f32 = 250.0;
const PANEL_W: f32 = 300.0;
/// The two right-aligned table columns.
const PLAYERS_W: f32 = 90.0;
const PING_W: f32 = 56.0;

/// The UI, as a plain function rather than a system — two callers need it and
/// a system cannot call another system directly.
fn build(
    commands: &mut Commands,
    menu: &Menu,
    browse: &Browse,
    who: &Who,
    hub: &Hub,
    backdrop: Option<&ui::Backdrop>,
) {
    // A camera, because the menu may be the first thing that ever exists —
    // the render rig only spawns on entering the world.
    commands.spawn((Camera2d, MenuRoot));

    commands
        .spawn((
            MenuRoot,
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                ..default()
            },
            BackgroundColor(ui::BG),
        ))
        .with_children(|root| {
            // Footage, then the wash over it, then everything else. Both sit
            // at a negative `ZIndex` rather than trusting spawn order — the
            // shell's children come from three functions and none of them can
            // promise to be first.
            if let Some(b) = backdrop {
                root.spawn(ui::backdrop(b.0.clone()));
                root.spawn(ui::scrim());
            }
            header(root, who);
            root.spawn(Node {
                flex_direction: FlexDirection::Row,
                flex_grow: 1.0,
                ..default()
            })
            .with_children(|body| {
                nav(body, browse.nav, hub);
                // The payload. OPTIONS and QUIT are verbs rather than screens
                // (`Nav::is_pane`), so they never reach here — the pane keeps
                // whatever was last open while they act.
                match browse.nav {
                    Nav::Hub(section) => hub_pane(body, section, hub),
                    _ => {
                        filters(body, menu, browse);
                        table(body, menu, browse);
                    }
                }
            });
        });
}

/// The wordmark, and who the launcher says is playing.
///
/// **The identity chip is the reference's profile corner, and it is the first
/// thing in this client to draw `Who` at all** — it was resolved at startup
/// and read by nothing. `Player::line` is what says it, so the menu cannot
/// disagree with the console line printed at boot.
fn header(root: &mut ChildSpawnerCommands, who: &Who) {
    root.spawn(Node {
        flex_direction: FlexDirection::Row,
        align_items: AlignItems::Center,
        justify_content: JustifyContent::SpaceBetween,
        padding: UiRect::new(Val::Px(34.0), Val::Px(34.0), Val::Px(26.0), Val::Px(22.0)),
        ..default()
    })
    .with_children(|bar| {
        bar.spawn(ui::wordmark());
        // The identity and the build, stacked right — two facts about *this
        // session* that belong in the same corner. The build is here as well
        // as on the HUD because this is the screen where it answers a
        // question: a shard that refuses you for `REFUSE_BUILD` says "update
        // the game", and the next thing anyone does is look for what they are
        // running. In-world it is only ever evidence in a screenshot.
        bar.spawn((
            Node {
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::End,
                ..default()
            },
            children![
                ui::label(who.0.line(), 13.0, ui::DIM),
                ui::label(protocol::version::BUILD_ID.to_string(), 11.0, ui::DIM),
            ],
        ));
    });
}

fn nav(body: &mut ChildSpawnerCommands, picked: Nav, hub: &Hub) {
    body.spawn(Node {
        width: Val::Px(NAV_W),
        flex_direction: FlexDirection::Column,
        padding: UiRect::new(Val::Px(34.0), Val::Px(0.0), Val::Px(10.0), Val::Px(0.0)),
        ..default()
    })
    .with_children(|col| {
        for n in Nav::ALL {
            col.spawn((ui::nav_item(&n.label(hub), n == picked), NavButton(n)));
        }
    });
}

/// A launcher-backed section: what it is, what state it is in, and the one
/// button that hands it to the launcher.
///
/// **Deliberately not a shopfront.** The reference draws its item store and
/// workshop in-game because it owns the whole client-plus-platform stack;
/// ours must not, and the sentence under the button says why in the player's
/// own terms. `crate::ui::hub` carries the full argument — the short version
/// is that a game draws its own pixels, so a checkout it draws is a checkout
/// it could forge, and the launcher's window is where a real one lives.
///
/// It uses the same panel-plus-pane shell as PLAY GAME rather than a bespoke
/// layout, because a nav that changed the page's whole geometry per entry is
/// the thing the shell exists to prevent.
fn hub_pane(body: &mut ChildSpawnerCommands, section: Section, hub: &Hub) {
    let status = section.status(hub);
    let ready = status.url().is_some();
    let label = section.label(hub);

    body.spawn(ui::panel(PANEL_W)).with_children(|panel| {
        panel.spawn(ui::strong(label.clone(), 22.0, ui::TITLE));
        panel.spawn((
            ui::label(section.blurb(), 13.0, ui::DIM),
            Node {
                margin: UiRect::top(Val::Px(6.0)),
                ..default()
            },
        ));

        panel.spawn(ui::group("WHERE THIS LIVES"));
        // Always something, and it always names what would change it — the
        // rule the shard list obeys, applied to a different document. The
        // four states are four sentences (`ui::hub::Status::line`).
        panel.spawn(ui::label(status.line(), 12.0, ui::FAINT));

        // **Both are pinned absolutely rather than pushed by a spacer.** A
        // wrapping column whose only content was text measured ZERO tall and
        // took its children off the bottom of the window with it — found by
        // tinting the spacer, which then visibly ran past the window's edge.
        // `flex_shrink: 0` did not help, because the node was not being
        // shrunk; it never had a height to begin with.
        panel.spawn((
            ui::label(section.handoff_reason(), 11.0, ui::FAINT),
            ui::pinned_bottom(ui::BUTTON_H + 26.0, PANEL_W - 32.0),
        ));
        // Drawn dim rather than removed when there is nothing to open: a
        // button that comes and goes is a layout that jumps under the
        // pointer, and the line above already says why.
        panel
            .spawn((
                ui::pinned_button(PANEL_W - 32.0, ready, 16.0),
                HubButton(section),
            ))
            .with_children(|b| {
                b.spawn(ui::strong(
                    "OPEN IN LAUNCHER",
                    12.0,
                    if ready { ui::TITLE } else { ui::FAINT },
                ));
            });
    });

    // The content side. Nothing is drawn *in* it, and that is the honest
    // shape: the page is the launcher's, so a pane full of invented chrome
    // would be this client pretending to host a service it hands off.
    body.spawn(ui::pane()).with_children(|pane| {
        pane.spawn((
            ui::label(
                format!(
                    "{label} is part of Elo, the launcher this game is sold \
                     through. It opens there rather than here."
                ),
                14.0,
                ui::DIM,
            ),
            Node {
                margin: UiRect::all(Val::Px(24.0)),
                max_width: Val::Px(560.0),
                ..default()
            },
        ));
    });
}

/// The tinted control column: the rail, the search box, the slot filters and
/// the two buttons — the reference's left panel, minus the groups whose data
/// we do not publish.
fn filters(body: &mut ChildSpawnerCommands, menu: &Menu, browse: &Browse) {
    let counts = servers::counts(&menu.rows, &browse.favourites);
    body.spawn(ui::panel(PANEL_W)).with_children(|panel| {
        for (i, cat) in Cat::ALL.into_iter().enumerate() {
            let picked = cat == browse.filter.cat;
            panel
                .spawn((
                    Button,
                    Node {
                        padding: UiRect::axes(Val::Px(12.0), Val::Px(9.0)),
                        flex_direction: FlexDirection::Column,
                        row_gap: Val::Px(2.0),
                        ..default()
                    },
                    BackgroundColor(if picked { ui::ACCENT } else { ui::ROW_IDLE }),
                    // The picked row keeps its own idle colour, which is the
                    // whole reason `Hover` is carried per entity.
                    ui::Hover::new(
                        if picked { ui::ACCENT } else { ui::ROW_IDLE },
                        if picked {
                            ui::ACCENT_HOVER
                        } else {
                            ui::ROW_HOVER
                        },
                    ),
                    CatButton(cat),
                ))
                .with_children(|b| {
                    b.spawn(ui::strong(
                        format!("{}  ({})", cat.label(), counts[i]),
                        16.0,
                        ui::TEXT,
                    ));
                    b.spawn(ui::label(cat.blurb(), 11.0, ui::DIM));
                });
        }

        // The search box. Not a widget — a rectangle showing what the keyboard
        // has put in the filter, because Bevy has no text input and one built
        // here would be a text-editing engine in a menu. The same call the
        // craft panel makes, for the same reason.
        panel
            .spawn((
                Node {
                    padding: UiRect::axes(Val::Px(10.0), Val::Px(8.0)),
                    margin: UiRect::top(Val::Px(8.0)),
                    border: UiRect::all(Val::Px(1.0)),
                    ..default()
                },
                BackgroundColor(ui::ROW_IDLE),
                BorderColor::all(ui::RULE),
            ))
            .with_children(|b| {
                let empty = browse.filter.query.is_empty();
                b.spawn(ui::label(
                    if empty {
                        "Search shards...".to_string()
                    } else {
                        browse.filter.query.clone()
                    },
                    13.0,
                    if empty { ui::FAINT } else { ui::TEXT },
                ));
            });

        panel.spawn(ui::group("PLAYER SLOTS"));
        for (label, empty) in [("Hide empty", true), ("Hide full", false)] {
            let on = if empty {
                browse.filter.hide_empty
            } else {
                browse.filter.hide_full
            };
            panel
                .spawn((ui::check(on), SlotFilter { empty }))
                .with_children(|b| {
                    b.spawn(ui::label(label, 13.0, ui::DIM));
                });
        }

        // The two buttons, pinned to the bottom the way the reference's are.
        panel.spawn(ui::spacer());
        panel
            .spawn(Node {
                flex_direction: FlexDirection::Row,
                column_gap: Val::Px(8.0),
                flex_shrink: 0.0,
                ..default()
            })
            .with_children(|bar| {
                let w = (PANEL_W - 32.0 - 8.0) / 2.0;
                bar.spawn((ui::button(w, false), PanelButton::Clear))
                    .with_children(|b| {
                        b.spawn(ui::strong(
                            "CLEAR FILTERS",
                            12.0,
                            if browse.filter.narrowed() {
                                ui::TEXT
                            } else {
                                // Nothing to clear. Dimmed rather than
                                // removed: a button that comes and goes is a
                                // layout that jumps under the pointer.
                                ui::FAINT
                            },
                        ));
                    });
                bar.spawn((ui::button(w, true), PanelButton::Refresh))
                    .with_children(|b| {
                        b.spawn(ui::strong("REFRESH", 12.0, ui::TITLE));
                    });
            });
    });
}

/// The content pane: a header row, the rows, and the status line.
fn table(body: &mut ChildSpawnerCommands, menu: &Menu, browse: &Browse) {
    let shown = servers::visible(&menu.rows, &browse.filter, &browse.favourites);
    body.spawn(ui::pane()).with_children(|pane| {
        // ---- the header ----------------------------------------------
        pane.spawn((
            Node {
                width: Val::Percent(100.0),
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: Val::Px(10.0),
                padding: UiRect::axes(Val::Px(12.0), Val::Px(10.0)),
                border: UiRect::bottom(Val::Px(1.0)),
                ..default()
            },
            BorderColor::all(ui::RULE),
        ))
        .with_children(|h| {
            // The star column's width, so the header's name lines up with the
            // rows' rather than sitting a star to the left of them.
            h.spawn(Node {
                width: Val::Px(STAR_W),
                ..default()
            });
            h.spawn((
                ui::strong("SERVER NAME", 12.0, ui::FAINT),
                Node {
                    flex_grow: 1.0,
                    ..default()
                },
            ));
            h.spawn(ui::column(PLAYERS_W)).with_children(|c| {
                c.spawn(ui::strong("PLAYERS", 12.0, ui::FAINT));
            });
            h.spawn(ui::column(PING_W)).with_children(|c| {
                c.spawn(ui::strong("PING", 12.0, ui::FAINT));
            });
        });

        // ---- the rows ------------------------------------------------
        for i in &shown {
            row(
                pane,
                &menu.rows[*i],
                *i,
                browse.favourites.has(&menu.rows[*i].id),
            );
        }

        // Asked unconditionally: the function checks whether the list it is
        // describing is actually empty, so the guard is not duplicated here.
        if let Some(why) = servers::empty_reason(&browse.filter, menu.rows.len(), shown.len()) {
            pane.spawn((
                ui::label(why, 13.0, ui::DIM),
                Node {
                    margin: UiRect::all(Val::Px(16.0)),
                    max_width: Val::Px(520.0),
                    ..default()
                },
            ));
        }

        // ---- the status line -----------------------------------------
        pane.spawn(ui::spacer());
        pane.spawn((
            StatusLine,
            ui::label(menu.status.clone(), 12.0, ui::FAINT),
            Node {
                margin: UiRect::new(Val::Px(12.0), Val::Px(12.0), Val::Px(0.0), Val::Px(10.0)),
                max_width: Val::Px(620.0),
                ..default()
            },
        ));

        pane.spawn((
            ui::label(
                "click a shard to join    -    type to search    -    Esc clears the search, \
                 then quits",
                11.0,
                ui::FAINT,
            ),
            Node {
                margin: UiRect::new(Val::Px(12.0), Val::Px(12.0), Val::Px(0.0), Val::Px(12.0)),
                ..default()
            },
        ));
    });
}

/// The star's hit box. Wide enough to click without aiming, and the header
/// reserves the same width so the two agree.
const STAR_W: f32 = 26.0;

fn row(pane: &mut ChildSpawnerCommands, listing: &Listing, index: usize, starred: bool) {
    pane.spawn((ui::table_row(), RowButton(index)))
        .with_children(|r| {
            // The star. A button inside a button: Bevy's `Interaction` picks
            // the topmost, so a click on the star does not also join — which
            // is the behaviour the reference's star has and the reason it is
            // its own hit target rather than a corner of the row.
            //
            // The Direct row has no star, because there is nothing stable to
            // key one on: its id is the address this binary happened to be
            // started with, and a favourite that changes meaning with a
            // command-line flag is worse than no favourite.
            if listing.direct {
                r.spawn(Node {
                    width: Val::Px(STAR_W),
                    ..default()
                });
            } else {
                r.spawn((
                    Button,
                    Node {
                        width: Val::Px(STAR_W),
                        justify_content: JustifyContent::Center,
                        ..default()
                    },
                    BackgroundColor(Color::NONE),
                    ui::Hover::new(Color::NONE, ui::ROW_HOVER),
                    StarButton(index),
                ))
                .with_children(|s| {
                    // A filled star and a hollow one, not a colour change on
                    // one glyph: the two must be distinguishable in a
                    // screenshot and at a glance, and colour alone is neither.
                    s.spawn(ui::strong(
                        if starred { "\u{2605}" } else { "\u{2606}" },
                        16.0,
                        if starred { ui::ACCENT } else { ui::FAINT },
                    ));
                });
            }

            r.spawn(Node {
                flex_grow: 1.0,
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(2.0),
                ..default()
            })
            .with_children(|c| {
                c.spawn(ui::strong(listing.name.clone(), 17.0, ui::TEXT));
                c.spawn(ui::label(listing.sub_line(), 12.0, ui::DIM));
            });

            r.spawn(ui::column(PLAYERS_W)).with_children(|c| {
                c.spawn(ui::strong(listing.population(), 15.0, ui::TEXT));
            });
            r.spawn(ui::column(PING_W)).with_children(|c| {
                c.spawn(ui::strong(listing.ping(), 15.0, ui::DIM));
            });
        });
}

/// Mouse. Everything on the screen, in one pass, into [`Picked`].
///
/// One system rather than five because they all write the same resource and
/// the ordering between them matters: a click that lands on a star must not
/// also be read as a click on the row under it. Bevy resolves that for us by
/// only marking the topmost node `Pressed`, but collecting them in one place
/// is what makes that checkable rather than incidental.
#[allow(clippy::too_many_arguments)]
pub fn click(
    rows: Query<(&Interaction, &RowButton), Changed<Interaction>>,
    stars: Query<(&Interaction, &StarButton), Changed<Interaction>>,
    navs: Query<(&Interaction, &NavButton), Changed<Interaction>>,
    cats: Query<(&Interaction, &CatButton), Changed<Interaction>>,
    slots: Query<(&Interaction, &SlotFilter), Changed<Interaction>>,
    buttons: Query<(&Interaction, &PanelButton), Changed<Interaction>>,
    hubs: Query<(&Interaction, &HubButton), Changed<Interaction>>,
    mut picked: ResMut<Picked>,
) {
    let pressed = |i: &Interaction| *i == Interaction::Pressed;
    for (i, s) in stars.iter() {
        if pressed(i) {
            picked.0 = Some(Pick::Star(s.0));
        }
    }
    for (i, r) in rows.iter() {
        if pressed(i) {
            picked.0 = Some(Pick::Row(r.0));
        }
    }
    for (i, n) in navs.iter() {
        if pressed(i) {
            picked.0 = Some(Pick::Nav(n.0));
        }
    }
    for (i, c) in cats.iter() {
        if pressed(i) {
            picked.0 = Some(Pick::Category(c.0));
        }
    }
    for (i, s) in slots.iter() {
        if pressed(i) {
            picked.0 = Some(Pick::Slots { empty: s.empty });
        }
    }
    for (i, b) in buttons.iter() {
        if pressed(i) {
            picked.0 = Some(match b {
                PanelButton::Clear => Pick::Clear,
                PanelButton::Refresh => Pick::Refresh,
            });
        }
    }
    for (i, h) in hubs.iter() {
        if pressed(i) {
            picked.0 = Some(Pick::Open(h.0));
        }
    }
}

/// Keyboard. **Typing goes to the search box**, which is what the reference's
/// browser does and what a list of sixty-four rows needs.
///
/// That costs the number-key picks the first cut had, deliberately: a digit
/// cannot be both a row index and a character in `us-west-2`, and a search box
/// that silently ate every number would be the worse of the two. Esc is the
/// one bind that survived, and it now does the smaller thing first — clear the
/// query — so quitting is never one keystroke away from a player who was only
/// trying to undo a search.
pub fn keys(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut chars: MessageReader<bevy::input::keyboard::KeyboardInput>,
    mut browse: ResMut<Browse>,
    mut menu: ResMut<Menu>,
    // `MessageWriter`, not `EventWriter` — Bevy 0.18 renamed events to
    // messages and the prelude carries only the new name.
    mut exit: MessageWriter<AppExit>,
) {
    if keyboard.just_pressed(KeyCode::Escape) {
        chars.clear();
        if browse.filter.query.is_empty() {
            exit.write(AppExit::Success);
        } else {
            browse.filter.query.clear();
            menu.dirty = true;
        }
        return;
    }

    let mut changed = false;
    for ev in chars.read() {
        if !ev.state.is_pressed() {
            continue;
        }
        match &ev.logical_key {
            bevy::input::keyboard::Key::Backspace => changed |= browse.filter.backspace(),
            bevy::input::keyboard::Key::Character(s) => {
                for c in s.chars() {
                    changed |= browse.filter.push(c);
                }
            }
            _ => {}
        }
    }
    if changed {
        menu.dirty = true;
    }
}

/// What this screen can be asked for.
///
/// An enum rather than the row index the first cut used, because a settings
/// entry smuggled in as `rows.len()` would have been an index that means "not
/// a row" with nothing in the type to say so. Every entry below is a thing a
/// player clicked, and [`take_pick`] is the one place any of them becomes a
/// state change or a mutation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pick {
    Row(usize),
    Star(usize),
    Nav(Nav),
    /// Hand a launcher-backed section to the launcher.
    Open(Section),
    Category(Cat),
    Slots {
        empty: bool,
    },
    Clear,
    Refresh,
}

/// What was chosen this frame, if anything. A resource rather than an event
/// so the two input paths cannot both fire a connect in one frame.
#[derive(Resource, Default)]
pub struct Picked(pub Option<Pick>);

/// Turn a pick into a state change, a filter change, or a star.
#[allow(clippy::too_many_arguments)]
pub fn take_pick(
    mut picked: ResMut<Picked>,
    mut menu: ResMut<Menu>,
    mut browse: ResMut<Browse>,
    mut settings: ResMut<super::settings::Settings>,
    mut connecting: NonSendMut<Connecting>,
    state: Res<HubState>,
    // The launcher, if one answered at boot. `Option` because the majority
    // case is no launcher at all, which is playable and never a fault.
    mut elo: Option<NonSendMut<crate::elo::Elo>>,
    mut next: ResMut<NextState<Screen>>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(pick) = picked.0.take() else {
        return;
    };
    match pick {
        // Every entry that opens a pane is the same arm: set it and redraw.
        // Keeping this one branch rather than one per section is what makes
        // adding a section a change to `Nav::ALL` and nothing else.
        Pick::Nav(n) if n.is_pane() => {
            browse.nav = n;
            menu.dirty = true;
        }
        Pick::Open(section) => {
            let outcome = super::hub::open(elo.as_deref_mut(), &state.hub, section);
            menu.status = outcome.line(&section.label(&state.hub));
            menu.dirty = true;
        }
        Pick::Nav(Nav::Settings) => {
            settings.back = Screen::Menu;
            settings.dirty = false;
            next.set(Screen::Settings);
        }
        Pick::Nav(Nav::Quit) => {
            exit.write(AppExit::Success);
        }
        // `is_pane` already took Play and Hub; Settings and Quit have their
        // own arms above. An arm here would be unreachable, and a `_ => {}`
        // would silently swallow a nav entry added without an action —
        // which is exactly the dead row this nav is arranged to prevent.
        Pick::Nav(Nav::Play | Nav::Hub(_)) => unreachable!("handled by is_pane"),
        Pick::Category(cat) => {
            browse.filter.cat = cat;
            menu.dirty = true;
        }
        Pick::Slots { empty } => {
            if empty {
                browse.filter.hide_empty = !browse.filter.hide_empty;
            } else {
                browse.filter.hide_full = !browse.filter.hide_full;
            }
            menu.dirty = true;
        }
        Pick::Clear => {
            browse.filter.clear();
            menu.dirty = true;
        }
        Pick::Refresh => {
            // Re-fetch. The status line says so at once rather than after the
            // round trip, because a button that looks inert for a second is a
            // button a player clicks twice.
            if menu.servers_url.is_some() {
                menu.status = "refreshing the shard list...".into();
                menu.dirty = true;
                fetch_now(&mut menu);
            }
        }
        Pick::Star(i) => {
            let Some(id) = menu.rows.get(i).map(|r| r.id.clone()) else {
                return;
            };
            if browse.favourites.toggle(&id) {
                menu.dirty = true;
            }
        }
        Pick::Row(i) => {
            let Some(row) = menu.rows.get(i) else {
                return;
            };
            connecting.addr = row.addr.clone();
            connecting.rx = None;
            next.set(Screen::Connecting);
        }
    }
}

/// Leave the menu: despawn everything it owns.
pub fn teardown(mut commands: Commands, roots: Query<Entity, With<MenuRoot>>) {
    for e in roots.iter() {
        commands.entity(e).despawn();
    }
}

/// Start the connect, on the runtime, off the frame.
///
/// **The claimed address is resolved HERE, from `Who`, and that is a fix
/// rather than a move.** It used to be a field nothing ever wrote: the
/// pre-window connect in `gates.rs` passed `args::address()` directly, and
/// every connect that went through this screen — every menu pick, and now the
/// launcher join too — sent `Address::GUEST` however the player had
/// identified themselves. Reading `Who` instead covers both provenances, and
/// the launcher-reported one is new: the old path only ever looked at
/// `--identity`, so a player signed in to the launcher and started without
/// the flag reached the shard anonymous.
///
/// It is still a *claim* and the shard still treats it as one — `sign_siwe`
/// is what turns it into anything, and `Player`'s own doc says an address is
/// never authentication.
pub fn begin_connect(rt: NonSend<Rt>, who: Res<Who>, mut connecting: NonSendMut<Connecting>) {
    connecting.address = match who.0.address() {
        None => protocol::Address::GUEST,
        Some(a) => protocol::Address::from_hex(a.as_bytes()).unwrap_or_else(|| {
            // A malformed `--identity` already failed at the command line
            // (`args::address`), so this is the launcher having reported
            // something we cannot parse. Play as a guest and SAY so, rather
            // than refusing to connect over a field that is a claim anyway.
            warn!("gates: launcher reported an unreadable address ({a}) - joining as a guest");
            protocol::Address::GUEST
        }),
    };
    let addr = connecting.addr.clone();
    let address = connecting.address;
    connecting.waited_s = 0.0;
    let (tx, rx) = std::sync::mpsc::channel();
    rt.0.spawn(async move {
        // **No pin, and that is enforced rather than assumed.** This screen
        // dials a shard the player picked from the list, which serves a real
        // certificate; `--cert-hash` is refused without `--server`, and
        // `--server` skips this screen entirely (`args::parse`), so the flag
        // cannot be set on a run that reaches here. The address decides the
        // rest — loopback permissive, everything else validated.
        let result = match crate::client_endpoint(&addr, None) {
            Ok(endpoint) => {
                crate::Session::connect(&endpoint, &addr, address, crate::elo::sign_siwe).await
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
            ui::wordmark(),
            (
                ui::strong(
                    format!("connecting to {}...", connecting.addr),
                    20.0,
                    ui::TEXT
                ),
                Node {
                    margin: UiRect::top(Val::Px(18.0)),
                    ..default()
                },
            ),
            // Says the way out, because there has to be one: a shard that is
            // not running never refuses a QUIC connect, it just goes quiet.
            ui::label("Esc goes back", 12.0, ui::FAINT),
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
        assert!(m.rows[0].direct);
        // ...and the empty menu says what would fill it, rather than being
        // silently bare.
        assert!(m.status.contains("--servers"), "{}", m.status);
    }

    #[test]
    fn every_nav_entry_reads_as_itself() {
        // A nav entry needs something behind it — `settings.rs`'s policy
        // against greyed rows, applied to the column. Three of these six are
        // the launcher's and their "something" is a state sentence rather
        // than a screen we drew (`crate::ui::hub`), which is why they are
        // here rather than left off.
        //
        // Distinctness is the assertion that can actually fail: the three hub
        // entries take their labels from a document nobody in this repo
        // controls, so two of them reading the same word is a real outcome
        // of a manifest edit, and it would leave a player unable to tell
        // which column they clicked.
        let hub = Hub::default();
        let labels: Vec<String> = Nav::ALL.iter().map(|n| n.label(&hub)).collect();
        for l in &labels {
            assert!(!l.trim().is_empty(), "an unclickable hole in the nav");
        }
        let mut sorted = labels.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), labels.len(), "two nav entries read the same");

        // Exactly the entries that open a pane are the ones `take_pick`
        // routes through `is_pane`; the other two are verbs. An entry in
        // neither set would hit the `unreachable!` there, so this pins the
        // split rather than trusting it.
        for n in Nav::ALL {
            let verb = matches!(n, Nav::Settings | Nav::Quit);
            assert_ne!(
                n.is_pane(),
                verb,
                "{n:?} is both a pane and a verb, or neither"
            );
        }
    }
}
