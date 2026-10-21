//! The front end: a wordmark, a nav column, and whatever the picked nav entry
//! puts beside it — the server list and PLAY, the skins (`locker`), or a
//! launcher hand-off.
//!
//! **It is the browser page's layout, drawn in Bevy** (operator, 2026-10-21:
//! the web start menu looked better and the two had to be unified). Same
//! plate behind it, same wash, same column widths, same green PLAY; the
//! page is `client-web/web/index.html` and a change to one is a change to
//! both. The server browser's rail, search and star went with that: one
//! shard is listed, and the page's radio list says everything they did.
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
//! Beside these, `InWorld` also leads to `Map` (held) and `Dead`, and a shard
//! that hangs up leads to `Disconnected` from any world state.
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
//! file was wrong to leave them off.** INVENTORY and ITEM STORE are drawn
//! here as the reference draws them, as grids of skins (`locker`); buying
//! still hands off. It argued they had no service behind
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
//! list's arithmetic — the busy-first sort, the favourite set —
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

use super::hub::HubState;
use super::icons::Icons;
use super::locker::{self, Locker};
// The types this screen drives now live beside the state machine they are
// part of, because a browser build keeps them and does not keep this file.
use super::screen::{Browse, Connecting, Menu, Nav, Screen, Who};
use super::ui;
use crate::shardlist::{self, Shard, MAX_DOC_BYTES};
use crate::ui::hub::{Hub, Section};
use crate::ui::locker::Shelf;
use crate::ui::servers::{self, Listing};

impl Nav {
    /// The reference's order (the operator's frames, 2026-10-21): PLAY GAME,
    /// NEWS, INVENTORY, ITEM STORE, WORKSHOP, its RUST+ (ours is TEAM APP,
    /// with BUDDIES before it), then OPTIONS and QUIT.
    const ALL: [Nav; 9] = [
        Nav::Play,
        Nav::Hub(Section::News),
        Nav::Inventory,
        Nav::Hub(Section::Store),
        Nav::Hub(Section::Workshop),
        Nav::Hub(Section::Buddies),
        Nav::Hub(Section::Team),
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
            Nav::Inventory => "INVENTORY".to_string(),
            Nav::Hub(s) => s.label(hub),
            // OPTIONS, as the reference and the browser page say it; the
            // screen it opens is still `settings`.
            Nav::Settings => "OPTIONS".to_string(),
            Nav::Quit => "QUIT".to_string(),
        }
    }

    /// Does picking this entry open a pane, or act at once? QUIT and OPTIONS
    /// are verbs; everything else is a screen. Drawn identically either way,
    /// which is the reference's arrangement too.
    fn is_pane(self) -> bool {
        matches!(self, Nav::Play | Nav::Inventory | Nav::Hub(_))
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

/// The tokio runtime, owned for the life of the app.
///
/// Split out of `Net` because the runtime now has to exist *before* any
/// session does — the connect attempt runs on it, and a failed attempt
/// leaves no `Net` behind to have held it.
pub struct Rt(pub tokio::runtime::Runtime);

/// Marks the menu's root node, so leaving the screen despawns exactly it.
#[derive(Component)]
pub struct MenuRoot;

/// A server row, by index into `Menu::rows`. A click picks it; a click on
/// the picked one joins.
#[derive(Component)]
pub struct RowButton(pub usize);

/// The PLAY button: joins the picked row.
#[derive(Component)]
pub struct GoButton;

/// A nav entry.
#[derive(Component)]
pub struct NavButton(pub Nav);

/// OPEN IN LAUNCHER, on a hub pane.
#[derive(Component)]
pub struct HubButton(pub Section);

/// REFRESH, beside the SERVER heading.
#[derive(Component)]
pub struct RefreshButton;

/// The backdrop, which drifts ([`ui::drift`]).
#[derive(Component)]
pub struct Drifts;

/// The status line under PLAY.
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
#[allow(clippy::too_many_arguments)]
pub fn setup(
    mut commands: Commands,
    menu: Res<Menu>,
    browse: Res<Browse>,
    who: Res<Who>,
    my: Res<MyFace>,
    state: Res<HubState>,
    locker: Res<Locker>,
    scroll: Res<ui::ScrollMemo>,
    icons: Option<Res<Icons>>,
    backdrop: Option<Res<ui::Backdrop>>,
    portrait: Option<Res<super::portrait::Portrait>>,
) {
    build(
        &mut commands,
        &View {
            menu: &menu,
            browse: &browse,
            who: &who,
            my: &my,
            hub: &state.hub,
            locker: &locker,
            scroll: scroll.0,
            icons: icons.as_deref(),
            backdrop: backdrop.as_deref(),
            portrait: portrait.as_deref(),
        },
    );
}

/// Your own name and picture on the platform, for the identity chip — read
/// straight from `/api/face/{wallet}` for the address the launcher (or
/// `--identity`) names, because the menu is up before any shard is.
#[derive(Resource, Default)]
pub struct MyFace {
    asked: Option<protocol::Address>,
    name: Option<String>,
    pic: u32,
    handle: Option<Handle<Image>>,
    rx: Option<tokio::sync::mpsc::UnboundedReceiver<Option<(String, u32)>>>,
}

/// Ask for [`MyFace`] when the address changes, and raise `menu.dirty` when
/// the name or the picture lands. Everything network is on a thread.
pub fn my_face(
    who: Res<Who>,
    mut menu: ResMut<Menu>,
    mut my: ResMut<MyFace>,
    mut pics: ResMut<super::faces::Pics>,
) {
    let addr = who
        .0
        .address()
        .and_then(|a| protocol::Address::from_hex(a.as_bytes()))
        .filter(|a| !a.is_guest());
    if addr != my.asked {
        *my = MyFace {
            asked: addr,
            ..MyFace::default()
        };
        menu.dirty = true;
        if let Some(a) = addr {
            pics.origin = crate::ui::names::origin_of(menu.servers_url.as_deref());
            let hex = a.to_hex();
            let url = format!(
                "{}/api/face/{}",
                pics.origin,
                core::str::from_utf8(&hex).unwrap_or("")
            );
            let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
            std::thread::spawn(move || {
                let got = super::faces::get_bytes(&url, 4 * 1024)
                    .and_then(|b| crate::ui::names::parse_face(&b));
                let _ = tx.send(got);
            });
            my.rx = Some(rx);
        }
    }
    if let Some(rx) = &mut my.rx {
        if let Ok(got) = rx.try_recv() {
            my.rx = None;
            if let Some((name, pic)) = got {
                my.name = (!name.is_empty()).then_some(name);
                my.pic = pic;
                menu.dirty = true;
            }
        }
    }
    if my.handle.is_none() && my.pic != 0 {
        if let Some(a) = my.asked {
            if let Some(h) = pics.want(a, my.pic) {
                my.handle = Some(h);
                menu.dirty = true;
            }
        }
    }
}

/// Redraw. **One rebuild path for every reason the screen can change** — a
/// fetch landing, a poll moving a count, a click, a skin read — and it is a
/// full respawn rather than a patch.
///
/// The same call `settings::rebuild` makes and for the same stated reason: a
/// screen whose every drawn value is derived from a few resources is cheaper
/// to rebuild than to diff, and a patch path is exactly where a drawn value
/// drifts from the held one. Which matters more here than there, because the
/// values on this screen are *numbers a player acts on*.
#[allow(clippy::too_many_arguments)]
pub fn rebuild(
    mut commands: Commands,
    mut menu: ResMut<Menu>,
    browse: Res<Browse>,
    who: Res<Who>,
    my: Res<MyFace>,
    mut state: ResMut<HubState>,
    locker: Res<Locker>,
    scroll: Res<ui::ScrollMemo>,
    icons: Option<Res<Icons>>,
    backdrop: Option<Res<ui::Backdrop>>,
    portrait: Option<Res<super::portrait::Portrait>>,
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
        &View {
            menu: &menu,
            browse: &browse,
            who: &who,
            my: &my,
            hub: &state.hub,
            locker: &locker,
            scroll: scroll.0,
            icons: icons.as_deref(),
            backdrop: backdrop.as_deref(),
            portrait: portrait.as_deref(),
        },
    );
}

/// Everything one build of the screen reads.
struct View<'a> {
    menu: &'a Menu,
    browse: &'a Browse,
    who: &'a Who,
    my: &'a MyFace,
    hub: &'a Hub,
    locker: &'a Locker,
    /// Where the page's scroll area was left (`ui::ScrollMemo`).
    scroll: f32,
    icons: Option<&'a Icons>,
    backdrop: Option<&'a ui::Backdrop>,
    portrait: Option<&'a super::portrait::Portrait>,
}

/// Column widths and the side gutter, pixels: the browser page's (`.nav`,
/// `.panel`, `--gut` at a 1280 window), so the two front ends are one layout.
const NAV_W: f32 = 230.0;
const PANEL_W: f32 = 380.0;
const GUT: f32 = 34.0;

/// What the page says under PLAY GAME, word for word.
const PLAY_BLURB: &str =
    "Wake with nothing on a hostile island. Gather, craft, build, raid. Lose it all, and go again.";

/// The UI, as a plain function rather than a system — two callers need it and
/// a system cannot call another system directly.
///
/// **The browser page's arrangement** (`client-web/web/index.html`, which the
/// operator called the better of the two, 2026-10-21): wordmark and account
/// across the top, the nav down the left, and beside it a panel and either a
/// pane or the open scene. PLAY GAME is a server list and one green button
/// over the footage; INVENTORY and ITEM STORE are the reference's grids
/// (`locker`).
fn build(commands: &mut Commands, v: &View) {
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
                overflow: Overflow::clip(),
                ..default()
            },
            BackgroundColor(ui::BG),
        ))
        .with_children(|root| {
            // Footage, then the wash over it, then everything else. Both sit
            // at a negative `ZIndex` rather than trusting spawn order — the
            // shell's children come from several functions and none of them
            // can promise to be first.
            if let Some(b) = v.backdrop {
                root.spawn((ui::backdrop(b.0.clone()), Drifts));
                root.spawn(ui::scrim());
            }
            // On PLAY GAME only: the other pages' panes are translucent and
            // he showed through them.
            if let (Some(p), Nav::Play) = (v.portrait, v.browse.nav) {
                hero(root, p);
            }
            header(root, v.who, v.my);
            root.spawn(Node {
                flex_direction: FlexDirection::Row,
                flex_grow: 1.0,
                min_height: Val::Px(0.0),
                padding: UiRect::horizontal(Val::Px(GUT)),
                ..default()
            })
            .with_children(|body| {
                nav(body, v.browse.nav, v.hub);
                // The payload. OPTIONS and QUIT are verbs rather than screens
                // (`Nav::is_pane`), so they never reach here — the pane keeps
                // whatever was last open while they act.
                match v.browse.nav {
                    Nav::Inventory => {
                        locker::inventory(body, v.locker, v.icons, PANEL_W, v.scroll);
                    }
                    Nav::Hub(Section::Store) => {
                        let note = v.locker.note.as_deref().unwrap_or(
                            "Buying happens in the Elo launcher, from your own wallet: a game \
                             never takes a payment in its own window.",
                        );
                        locker::store(body, v.locker, v.icons, note, v.scroll);
                    }
                    Nav::Hub(section) => hub_pane(body, section, v.hub),
                    _ => {
                        let pick = picked_row(v.menu, v.browse);
                        play_panel(body, v, pick);
                        stage(body, v.menu, pick);
                    }
                }
            });
            footer(root, v);
        });
}

/// Triple T, standing on the beach at the right of the open scene on PLAY
/// GAME: over the wash, under the panel. He stood under the nav until the nav
/// grew to the reference's length and reached down to him.
fn hero(root: &mut ChildSpawnerCommands, portrait: &super::portrait::Portrait) {
    use super::portrait::{view, H_PX, W_PX};
    root.spawn((
        Node {
            position_type: PositionType::Absolute,
            right: Val::Px(GUT + 24.0),
            bottom: Val::Px(40.0),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            ..default()
        },
        // The scrim's index, and spawned after it, so drawn over it.
        ZIndex(-1),
        Pickable::IGNORE,
    ))
    .with_children(|col| {
        let (w, h) = (HERO_H_PX * W_PX as f32 / H_PX as f32, HERO_H_PX);
        col.spawn(view(portrait, w, h));
        if let Some(name) = crate::ui::HERO {
            col.spawn(ui::strong(name, 15.0, ui::DIM));
        }
    });
}

/// The menu's Triple T: tall enough to read at 720p without reaching the
/// account corner.
const HERO_H_PX: f32 = 300.0;

/// The name the account corner and PLAYING AS show: your platform name, or
/// the wallet, or GUEST.
fn shown_name(who: &Who, my: &MyFace) -> String {
    if let Some(n) = &my.name {
        return n.clone();
    }
    match who.0.address() {
        Some(a) if a.len() > 12 => format!("{}...{}", &a[..6], &a[a.len() - 4..]),
        Some(a) => a.to_string(),
        None => "GUEST".to_string(),
    }
}

/// Your picture, or a plate with your initial — the page's `.face`.
fn face(parent: &mut ChildSpawnerCommands, who: &Who, my: &MyFace, px: f32) {
    let node = Node {
        width: Val::Px(px),
        height: Val::Px(px),
        justify_content: JustifyContent::Center,
        align_items: AlignItems::Center,
        border: UiRect::all(Val::Px(1.0)),
        flex_shrink: 0.0,
        ..default()
    };
    match &my.handle {
        Some(h) => {
            parent.spawn((ImageNode::new(h.clone()), node, BorderColor::all(ui::RULE)));
        }
        None => {
            let initial = shown_name(who, my)
                .chars()
                .next()
                .filter(|_| who.0.address().is_some())
                .map_or("·".to_string(), |c| c.to_uppercase().to_string());
            parent.spawn((
                node,
                BackgroundColor(ui::ROW_HOVER),
                BorderColor::all(ui::RULE),
                children![ui::strong(initial, px * 0.4, ui::TITLE)],
            ));
        }
    }
}

/// The wordmark, and who the launcher says is playing.
///
/// **The identity chip is the reference's profile corner.** `Player::line`
/// is what says it, so the menu cannot disagree with the console line
/// printed at boot.
fn header(root: &mut ChildSpawnerCommands, who: &Who, my: &MyFace) {
    root.spawn(Node {
        flex_direction: FlexDirection::Row,
        align_items: AlignItems::Center,
        justify_content: JustifyContent::SpaceBetween,
        padding: UiRect::new(Val::Px(GUT), Val::Px(GUT), Val::Px(28.0), Val::Px(20.0)),
        flex_shrink: 0.0,
        ..default()
    })
    .with_children(|bar| {
        bar.spawn(ui::wordmark_on("A SURVIVAL GAME ON ELO PROS"));
        bar.spawn(Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            column_gap: Val::Px(11.0),
            ..default()
        })
        .with_children(|chip| {
            chip.spawn(Node {
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::End,
                row_gap: Val::Px(2.0),
                ..default()
            })
            .with_children(|col| {
                col.spawn(ui::strong(shown_name(who, my), 16.0, ui::TITLE));
                col.spawn(ui::label(who_line(who), 12.5, ui::DIM));
            });
            face(chip, who, my, 40.0);
        });
    });
}

fn nav(body: &mut ChildSpawnerCommands, picked: Nav, hub: &Hub) {
    body.spawn(Node {
        width: Val::Px(NAV_W),
        flex_shrink: 0.0,
        flex_direction: FlexDirection::Column,
        padding: UiRect::top(Val::Px(6.0)),
        ..default()
    })
    .with_children(|col| {
        for n in Nav::ALL {
            // The reference's breaks: PLAY GAME alone, then the platform's
            // sections, then OPTIONS, then QUIT.
            if matches!(n, Nav::Hub(Section::News) | Nav::Settings | Nav::Quit) {
                col.spawn(Node {
                    height: Val::Px(18.0),
                    ..default()
                });
            }
            col.spawn((ui::nav_item(&n.label(hub), n == picked), NavButton(n)));
        }
    });
}

/// The row PLAY joins: the one picked, or until one is, the top listed
/// shard. The Direct row (the address this binary was started with, a
/// loopback dev shard by default) is the default only when nothing else is
/// listed: PLAY on a fresh install must not dial the player's own machine.
fn picked_row(menu: &Menu, browse: &Browse) -> Option<usize> {
    let shown = servers::visible(&menu.rows, &browse.filter, &browse.favourites);
    shown
        .iter()
        .copied()
        .find(|&i| browse.selected.as_deref() == Some(menu.rows[i].id.as_str()))
        .or_else(|| shown.iter().copied().find(|&i| !menu.rows[i].direct))
        .or_else(|| shown.first().copied())
}

/// One short line on who is playing and how we know: the page's address
/// under the name. `Player::line` says the same at length for the log.
fn who_line(who: &Who) -> String {
    let short = |a: &str| {
        if a.len() > 12 {
            format!("{}...{}", &a[..6], &a[a.len() - 4..])
        } else {
            a.to_string()
        }
    };
    match &who.0 {
        crate::elo::Player::Anonymous => "guest - no Elo launcher".to_string(),
        crate::elo::Player::Declared(a) => format!("{}  ·  declared", short(a)),
        crate::elo::Player::Launcher {
            address: Some(a), ..
        } => format!("{}  ·  signed in with Elo", short(a)),
        crate::elo::Player::Launcher { address: None, .. } => {
            "the Elo launcher is not signed in".to_string()
        }
    }
}

/// PLAY GAME's panel: who is playing, where, and the button — the page's.
fn play_panel(body: &mut ChildSpawnerCommands, v: &View, pick: Option<usize>) {
    let (menu, browse) = (v.menu, v.browse);
    let shown = servers::visible(&menu.rows, &browse.filter, &browse.favourites);
    body.spawn(ui::panel(PANEL_W)).with_children(|p| {
        p.spawn(ui::strong("PLAY GAME", 26.0, ui::TITLE));
        p.spawn(ui::label(PLAY_BLURB, 14.0, ui::DIM));

        p.spawn(ui::group_rule("PLAYING AS"));
        p.spawn((
            Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: Val::Px(12.0),
                padding: UiRect::all(Val::Px(12.0)),
                border: UiRect::all(Val::Px(1.0)),
                flex_shrink: 0.0,
                ..default()
            },
            BackgroundColor(ui::WELL),
            BorderColor::all(ui::RULE),
        ))
        .with_children(|w| {
            face(w, v.who, v.my, 44.0);
            w.spawn(Node {
                flex_grow: 1.0,
                flex_shrink: 1.0,
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(2.0),
                ..default()
            })
            .with_children(|c| {
                c.spawn(ui::strong(shown_name(v.who, v.my), 18.0, ui::TITLE));
                c.spawn(ui::label(who_line(v.who), 13.0, ui::DIM));
            });
        });

        // SERVER, its rule, and REFRESH at the end of it.
        p.spawn(Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            column_gap: Val::Px(8.0),
            margin: UiRect::top(Val::Px(10.0)),
            flex_shrink: 0.0,
            ..default()
        })
        .with_children(|g| {
            g.spawn(ui::strong("SERVER", 12.0, ui::FAINT));
            g.spawn((
                Node {
                    flex_grow: 1.0,
                    height: Val::Px(1.0),
                    ..default()
                },
                BackgroundColor(ui::RULE),
            ));
            g.spawn((
                Button,
                Node {
                    padding: UiRect::axes(Val::Px(6.0), Val::Px(3.0)),
                    ..default()
                },
                BackgroundColor(Color::NONE),
                ui::Hover::new(Color::NONE, ui::ROW_HOVER),
                RefreshButton,
            ))
            .with_children(|b| {
                b.spawn(ui::strong("REFRESH", 11.0, ui::DIM));
            });
        });

        // Shrinks to scroll rather than pushing PLAY off the panel when the
        // list is longer than the window has room for.
        p.spawn((
            Node {
                flex_direction: FlexDirection::Column,
                border: UiRect::all(Val::Px(1.0)),
                flex_shrink: 1.0,
                min_height: Val::Px(0.0),
                ..default()
            },
            BackgroundColor(ui::WELL),
            BorderColor::all(ui::RULE),
        ))
        .with_children(|well| {
            well.spawn(ui::scroll_area(v.scroll)).with_children(|list| {
                // One clock read per build, so every row's "wipes in" agrees.
                let now = unix_now();
                for &i in &shown {
                    server_row(list, &menu.rows[i], i, Some(i) == pick, now);
                }
                if let Some(why) =
                    servers::empty_reason(&browse.filter, menu.rows.len(), shown.len())
                {
                    list.spawn((
                        ui::label(why, 13.0, ui::DIM),
                        Node {
                            margin: UiRect::all(Val::Px(12.0)),
                            ..default()
                        },
                    ));
                }
            });
        });

        p.spawn(ui::spacer());
        let ready = pick.is_some();
        let sub = match pick.map(|i| &menu.rows[i]) {
            Some(l) if l.needs_update() => "UPDATE THE GAME TO JOIN".to_string(),
            Some(l) => l.name.to_uppercase(),
            None => "NO SERVER LISTED".to_string(),
        };
        p.spawn((ui::go(ready), GoButton)).with_children(|b| {
            b.spawn(ui::strong("PLAY", 30.0, ui::go_ink(ready)));
            b.spawn(ui::strong(sub, 11.0, ui::go_ink(ready).with_alpha(0.72)));
        });
        p.spawn((StatusLine, ui::label(menu.status.clone(), 13.0, ui::DIM)));
    });
}

/// One server in the list: the radio dot, the name and its tags, where it
/// is, and who is on it — the page's `.srv`.
fn server_row(
    list: &mut ChildSpawnerCommands,
    listing: &Listing,
    index: usize,
    picked: bool,
    now_unix: u64,
) {
    let (idle, over) = if picked {
        (PICKED_ROW, PICKED_ROW_HOVER)
    } else {
        (Color::NONE, ui::ROW_HOVER)
    };
    list.spawn((
        Button,
        Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            column_gap: Val::Px(10.0),
            padding: UiRect::axes(Val::Px(12.0), Val::Px(10.0)),
            border: UiRect::new(Val::Px(3.0), Val::Px(0.0), Val::Px(0.0), Val::Px(1.0)),
            flex_shrink: 0.0,
            ..default()
        },
        BackgroundColor(idle),
        BorderColor {
            left: if picked {
                ui::ACCENT_HOVER
            } else {
                Color::NONE
            },
            bottom: ui::RULE,
            ..BorderColor::all(Color::NONE)
        },
        ui::Hover::new(idle, over),
        RowButton(index),
    ))
    .with_children(|r| {
        r.spawn((
            Node {
                width: Val::Px(10.0),
                height: Val::Px(10.0),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::MAX,
                flex_shrink: 0.0,
                ..default()
            },
            BackgroundColor(if picked {
                ui::ACCENT_HOVER
            } else {
                Color::NONE
            }),
            BorderColor::all(if picked { ui::ACCENT_HOVER } else { ui::RULE }),
        ));
        r.spawn(Node {
            flex_grow: 1.0,
            flex_shrink: 1.0,
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(3.0),
            ..default()
        })
        .with_children(|c| {
            c.spawn(Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                flex_wrap: FlexWrap::Wrap,
                column_gap: Val::Px(9.0),
                ..default()
            })
            .with_children(|n| {
                n.spawn(ui::strong(listing.name.clone(), 16.0, ui::TEXT));
                if listing.official() {
                    official_tag(n);
                }
                // Said before the click: this build would be refused at
                // the handshake (`REFUSE_VERSION`). Which side moves
                // decides the words: a newer shard wants a newer game, an
                // older one is mid-release and will catch up.
                if listing.needs_update() {
                    n.spawn(ui::strong("UPDATE NEEDED", 11.0, super::panels::TEXT_SHORT));
                } else if listing.server_updating() {
                    n.spawn(ui::strong("SERVER UPDATING", 11.0, ui::DIM));
                }
            });
            let mut at = listing.addr.clone();
            if let Some(m) = &listing.map {
                at.push_str("  ·  ");
                at.push_str(m);
            }
            c.spawn(ui::label(at, 12.0, ui::DIM));
            // Who is on it and when it wipes, the page's `.pop`.
            let who = listing.crowd().or_else(|| {
                listing
                    .players
                    .map(|_| format!("{} players", listing.population()))
            });
            let line = [who, listing.wipe_in(now_unix)]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join("  ·  ");
            if line.is_empty() {
                c.spawn(ui::label("players unknown", 13.0, ui::DIM));
            } else {
                c.spawn(ui::strong(line, 13.0, ui::TEXT));
            }
        });
    });
}

/// The picked row's ground: the page's accent at a third.
const PICKED_ROW: Color = Color::srgba(0.224, 0.510, 0.729, 0.32);
const PICKED_ROW_HOVER: Color = Color::srgba(0.224, 0.510, 0.729, 0.42);

/// The scene left open beside the panel, with the picked server's facts
/// over it where the page puts its latest update — everything the old
/// details drawer said, read off the same `Listing::details`.
fn stage(body: &mut ChildSpawnerCommands, menu: &Menu, pick: Option<usize>) {
    body.spawn(Node {
        flex_grow: 1.0,
        min_width: Val::Px(0.0),
        flex_direction: FlexDirection::Column,
        justify_content: JustifyContent::FlexEnd,
        padding: UiRect::new(Val::Px(48.0), Val::Px(0.0), Val::Px(0.0), Val::Px(8.0)),
        ..default()
    })
    .with_children(|s| {
        let Some(l) = pick.map(|i| &menu.rows[i]) else {
            return;
        };
        s.spawn(Node {
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(8.0),
            max_width: Val::Px(760.0),
            ..default()
        })
        .with_children(|h| {
            h.spawn(Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: Val::Px(10.0),
                ..default()
            })
            .with_children(|k| {
                k.spawn((
                    Node {
                        width: Val::Px(26.0),
                        height: Val::Px(3.0),
                        ..default()
                    },
                    BackgroundColor(ui::MARK),
                ));
                k.spawn((
                    ui::strong(
                        if l.official() {
                            "OFFICIAL SERVER"
                        } else {
                            "COMMUNITY SERVER"
                        },
                        12.0,
                        ui::TITLE,
                    ),
                    ui::TEXT_SHADOW,
                ));
            });
            h.spawn((ui::strong(l.name.clone(), 34.0, ui::TITLE), ui::TEXT_SHADOW));
            for (label, value) in l.details(unix_now()) {
                h.spawn(Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(12.0),
                    ..default()
                })
                .with_children(|row| {
                    row.spawn((
                        ui::strong(label, 11.0, ui::FAINT),
                        ui::TEXT_SHADOW,
                        Node {
                            width: Val::Px(96.0),
                            flex_shrink: 0.0,
                            ..default()
                        },
                    ));
                    row.spawn((ui::label(value, 15.0, ui::TEXT), ui::TEXT_SHADOW));
                });
            }
        });
    });
}

/// The bottom strip, the page's `.foot`: the build, and the keys.
fn footer(root: &mut ChildSpawnerCommands, v: &View) {
    root.spawn(Node {
        flex_direction: FlexDirection::Row,
        align_items: AlignItems::Center,
        column_gap: Val::Px(18.0),
        padding: UiRect::new(Val::Px(GUT), Val::Px(GUT), Val::Px(12.0), Val::Px(16.0)),
        flex_shrink: 0.0,
        ..default()
    })
    .with_children(|f| {
        // The build is here because this is the screen where it answers a
        // question: a shard that refuses you for `REFUSE_BUILD` says "update
        // the game", and the next thing anyone does is look for what they
        // are running.
        f.spawn(ui::label(
            format!("Gates {}", protocol::version::BUILD_ID),
            12.0,
            ui::FAINT,
        ));
        f.spawn(ui::label("Elo Pros", 12.0, ui::DIM));
        f.spawn(ui::spacer());
        // A launcher section's hand-off says how it went here: its panel
        // has no status line of its own.
        if matches!(v.browse.nav, Nav::Hub(s) if s != Section::Store) {
            f.spawn(ui::label(v.menu.status.clone(), 12.0, ui::DIM));
        }
        f.spawn(ui::label(
            if v.browse.nav == Nav::Play {
                "Enter plays  ·  Esc quits"
            } else {
                "Esc goes back"
            },
            12.0,
            ui::FAINT,
        ));
    });
}

/// A launcher-backed section: what it is, what state it is in, and the one
/// button that hands it to the launcher.
///
/// **Deliberately not a shopfront.** `crate::ui::hub` carries the argument:
/// a game draws its own pixels, so a checkout it draws is a checkout it could
/// forge, and the launcher's window is where a real one lives. The item
/// store's catalog is drawn (`locker::store`); its checkout is not.
fn hub_pane(body: &mut ChildSpawnerCommands, section: Section, hub: &Hub) {
    let status = section.status(hub);
    let ready = status.url().is_some();
    let label = section.label(hub);

    body.spawn(ui::panel(PANEL_W)).with_children(|panel| {
        panel.spawn(ui::strong(label.clone(), 26.0, ui::TITLE));
        panel.spawn(ui::label(section.blurb(hub), 14.0, ui::DIM));

        panel.spawn(ui::group_rule("WHERE THIS LIVES"));
        // Always something, and it always names what would change it — the
        // rule the shard list obeys, applied to a different document. The
        // four states are four sentences (`ui::hub::Status::line`).
        panel.spawn(ui::label(status.line(), 13.0, ui::FAINT));

        panel.spawn(ui::spacer());
        panel.spawn(ui::label(section.handoff_reason(), 12.0, ui::FAINT));
        // Drawn dim rather than removed when there is nothing to open: a
        // button that comes and goes is a layout that jumps under the
        // pointer, and the line above already says why.
        panel
            .spawn((ui::button(PANEL_W - 42.0, ready), HubButton(section)))
            .with_children(|b| {
                b.spawn(ui::strong(
                    "OPEN IN LAUNCHER",
                    13.0,
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

/// OFFICIAL beside a shard we run ([`Listing::official`]): the sigil, then
/// the word in the mark's green.
fn official_tag(parent: &mut ChildSpawnerCommands) {
    parent
        .spawn(Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            column_gap: Val::Px(5.0),
            ..default()
        })
        .with_children(|t| {
            t.spawn((
                ImageNode::new(ui::SIGIL),
                Node {
                    width: Val::Px(14.0),
                    height: Val::Px(14.0),
                    ..default()
                },
            ));
            t.spawn(ui::strong("OFFICIAL", 11.0, ui::MARK));
        });
}

/// Unix seconds, for a row's "wipes in". Guarded like the module itself
/// (`render/mod.rs` compiles `menu` off wasm32), where `SystemTime::now` traps.
#[cfg(not(target_arch = "wasm32"))]
fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// The backdrop's push-in, every frame, off the app's clock.
pub fn drift(time: Res<Time>, mut q: Query<&mut UiTransform, With<Drifts>>) {
    for mut xf in q.iter_mut() {
        *xf = ui::drift(time.elapsed_secs());
    }
}

/// A nav label lifts under the pointer and steps right, the page's
/// `.nav .item:hover`.
pub fn nav_hover(
    browse: Res<Browse>,
    mut navs: Query<(&Interaction, &NavButton, &Children, &mut UiTransform), Changed<Interaction>>,
    mut texts: Query<&mut TextColor>,
) {
    for (i, n, children, mut xf) in navs.iter_mut() {
        let over = *i != Interaction::None;
        xf.translation = Val2::px(if over { 4.0 } else { 0.0 }, 0.0);
        for c in children.iter() {
            if let Ok(mut t) = texts.get_mut(c) {
                t.0 = ui::nav_color(n.0 == browse.nav, over);
            }
        }
    }
}

/// Mouse. Everything on the screen, in one pass, into [`Picked`].
///
/// One system because they all write the same resource. A click on a card
/// and on the grid under it cannot both land: Bevy marks only the topmost
/// node `Pressed`.
#[allow(clippy::too_many_arguments)]
pub fn click(
    rows: Query<(&Interaction, &RowButton), Changed<Interaction>>,
    go: Query<&Interaction, (Changed<Interaction>, With<GoButton>)>,
    navs: Query<(&Interaction, &NavButton), Changed<Interaction>>,
    refresh: Query<&Interaction, (Changed<Interaction>, With<RefreshButton>)>,
    hubs: Query<(&Interaction, &HubButton), Changed<Interaction>>,
    shelves: Query<(&Interaction, &locker::ShelfButton), Changed<Interaction>>,
    stores: Query<&Interaction, (Changed<Interaction>, With<locker::OpenStore>)>,
    inventory: Query<&Interaction, (Changed<Interaction>, With<locker::RefreshInventory>)>,
    mut picked: ResMut<Picked>,
) {
    let pressed = |i: &Interaction| *i == Interaction::Pressed;
    for (i, r) in rows.iter() {
        if pressed(i) {
            picked.0 = Some(Pick::Row(r.0));
        }
    }
    if go.iter().any(pressed) {
        picked.0 = Some(Pick::Go);
    }
    for (i, n) in navs.iter() {
        if pressed(i) {
            picked.0 = Some(Pick::Nav(n.0));
        }
    }
    if refresh.iter().any(pressed) {
        picked.0 = Some(Pick::Refresh);
    }
    for (i, h) in hubs.iter() {
        if pressed(i) {
            picked.0 = Some(Pick::Open(h.0));
        }
    }
    for (i, s) in shelves.iter() {
        if pressed(i) {
            picked.0 = Some(Pick::Shelf(s.0));
        }
    }
    if stores.iter().any(pressed) {
        picked.0 = Some(Pick::Open(Section::Store));
    }
    if inventory.iter().any(pressed) {
        picked.0 = Some(Pick::RefreshInventory);
    }
}

/// Keyboard: Enter plays, the page's binding; Esc does the smaller thing
/// first — back to PLAY GAME from another page, and only quits from there.
pub fn keys(
    keyboard: Res<ButtonInput<KeyCode>>,
    browse: Res<Browse>,
    mut picked: ResMut<Picked>,
    // `MessageWriter`, not `EventWriter` — Bevy 0.18 renamed events to
    // messages and the prelude carries only the new name.
    mut exit: MessageWriter<AppExit>,
) {
    if keyboard.just_pressed(KeyCode::Escape) {
        if browse.nav == Nav::Play {
            exit.write(AppExit::Success);
        } else {
            picked.0 = Some(Pick::Nav(Nav::Play));
        }
        return;
    }
    if browse.nav == Nav::Play
        && (keyboard.just_pressed(KeyCode::Enter) || keyboard.just_pressed(KeyCode::NumpadEnter))
    {
        picked.0 = Some(Pick::Go);
    }
}

/// What this screen can be asked for. Every entry below is a thing a player
/// clicked or pressed, and [`take_pick`] is the one place any of them
/// becomes a state change or a mutation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pick {
    /// Pick a row, or join it if it is already the picked one.
    Row(usize),
    /// PLAY: join the picked row.
    Go,
    Nav(Nav),
    /// Hand a launcher-backed section to the launcher.
    Open(Section),
    Refresh,
    /// An item store tab.
    Shelf(Shelf),
    RefreshInventory,
}

/// What was chosen this frame, if anything. A resource rather than an event
/// so the two input paths cannot both fire a connect in one frame.
#[derive(Resource, Default)]
pub struct Picked(pub Option<Pick>);

/// Turn a pick into a state change, a filter change, or a read.
#[allow(clippy::too_many_arguments)]
pub fn take_pick(
    mut picked: ResMut<Picked>,
    mut menu: ResMut<Menu>,
    mut browse: ResMut<Browse>,
    mut locker: ResMut<Locker>,
    mut scroll: ResMut<ui::ScrollMemo>,
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
            if browse.nav != n {
                scroll.0 = 0.0;
            }
            browse.nav = n;
            menu.dirty = true;
        }
        Pick::Open(section) => {
            let outcome = super::hub::open(elo.as_deref_mut(), &state.hub, section);
            let line = outcome.line(&section.label(&state.hub));
            // The store's page has its own line under the cards; every other
            // section's goes to the footer.
            if section == Section::Store {
                locker.note = Some(line);
            } else {
                menu.status = line;
            }
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
        // `is_pane` already took the panes; Settings and Quit have their own
        // arms above. An arm here would be unreachable, and a `_ => {}`
        // would silently swallow a nav entry added without an action —
        // which is exactly the dead row this nav is arranged to prevent.
        Pick::Nav(Nav::Play | Nav::Inventory | Nav::Hub(_)) => unreachable!("handled by is_pane"),
        Pick::Shelf(shelf) => {
            if locker.shelf != shelf {
                scroll.0 = 0.0;
            }
            locker.shelf = shelf;
            menu.dirty = true;
        }
        Pick::RefreshInventory => {
            // `locker::poll` reads both again next frame and redraws on each
            // answer; the panel says "Reading your wallet..." meanwhile.
            locker.refresh = true;
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
        Pick::Row(i) => {
            let Some(row) = menu.rows.get(i) else {
                return;
            };
            if browse.selected.as_deref() != Some(row.id.as_str()) {
                browse.selected = Some(row.id.clone());
                menu.dirty = true;
                return;
            }
            // A second click on the picked row joins, as it did when the
            // row opened a drawer and the drawer had JOIN.
            connecting.addr = row.addr.clone();
            connecting.rx = None;
            next.set(Screen::Connecting);
        }
        Pick::Go => {
            let Some(i) = picked_row(&menu, &browse) else {
                return;
            };
            connecting.addr = menu.rows[i].addr.clone();
            connecting.rx = None;
            next.set(Screen::Connecting);
        }
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
pub fn begin_connect(
    rt: NonSend<Rt>,
    who: Res<Who>,
    pin: Option<Res<super::screen::Pin>>,
    mut connecting: NonSendMut<Connecting>,
) {
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
    let hash = pin.filter(|p| p.addr == addr).map(|p| p.hash.clone());
    connecting.waited_s = 0.0;
    let (tx, rx) = std::sync::mpsc::channel();
    rt.0.spawn(async move {
        // **The pin only for the address it was given with.** A `--server`
        // launch comes through this screen too (`bin/gates.rs`), and its
        // `--cert-hash` used to be dropped here. A shard picked from the
        // list later is not that address and gets no pin; the address
        // decides the rest — loopback permissive, everything else validated.
        let result = match crate::client_endpoint(&addr, hash.as_deref()) {
            Ok(endpoint) => {
                crate::Session::connect(&endpoint, &addr, address, crate::elo::sign_siwe).await
            }
            // An endpoint that would not build never reached a shard, so there
            // is no code to carry — `Failed` is the honest arm.
            Err(e) => Err(crate::JoinError::Failed(e)),
        };
        let _ = tx.send(result);
    });
    connecting.rx = Some(rx);
}

/// A one-line "connecting to …" screen. Deliberately not the menu greyed
/// out: the menu is despawned, so there is nothing to click twice.
pub fn connecting_screen(mut commands: Commands, connecting: NonSend<Connecting>, who: Res<Who>) {
    // A wallet player may have to approve the sign-in somewhere else: in the
    // launcher, or in the browser tab it paired with.
    let hint = if who.0.address().is_some() {
        "Signing in: approve it in your launcher or your paired browser tab. Esc goes back"
    } else {
        "Esc goes back"
    };
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
            ui::label(hint, 12.0, ui::FAINT),
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
    } else if connecting.waited_s >= connect_timeout(connecting.address) {
        Some(format!(
            "no answer after {:.0}s",
            connect_timeout(connecting.address)
        ))
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
        // The connect thread died without sending — no shard answered, so
        // there is no refusal code to carry.
        Err(std::sync::mpsc::TryRecvError::Disconnected) => Err(crate::JoinError::Failed(
            "the connect attempt did not finish".to_string(),
        )),
    };
    connecting.rx = None;
    match got {
        Ok(session) => {
            info!(
                "gates: in the world — player {} seed {} tick {}",
                session.welcome.player_id, session.welcome.seed, session.welcome.tick
            );
            commands.insert_resource(super::WorldId {
                blight_salt: session.welcome.blight_salt,
                ..super::WorldId::with_haven(session.welcome.seed, *session.core.haven())
            });
            // `Commands` has no `insert_non_send_resource`, so this goes
            // through the world directly. `Net` is non-send because the
            // session owns tokio channel receivers (`render::mod`).
            commands.queue(move |world: &mut World| {
                world.insert_non_send_resource(super::Net {
                    session,
                    sel: 0,
                    light: false,
                });
            });
            // `Loading`, not `InWorld`: the welcome names a seed and the seed
            // is not a world. What comes next is three rings and a far mesh at
            // one build of each per frame, and the player used to watch that
            // happen from inside it.
            next.set(Screen::Loading);
        }
        Err(why) => {
            menu.status = match crate::elo::take_refusal() {
                Some(launcher) => format!("{}: {why} ({launcher})", connecting.addr),
                None => format!("{}: {why}", connecting.addr),
            };
            next.set(Screen::Menu);
        }
    }
}

/// The same, for a player signing in: the shard holds the challenge open
/// `SIGN_WAIT_SECS` for a person to approve it (a paired browser wallet is a
/// tab and a MetaMask click away), so giving up at 20 s abandoned sign-ins
/// the shard would have taken, and the retry found the launcher's first ask
/// still open and joined as a guest (2026-10-19).
pub const SIGN_IN_TIMEOUT_S: f32 = protocol::SIGN_WAIT_SECS as f32 + 5.0;

fn connect_timeout(address: protocol::Address) -> f32 {
    if address.is_guest() {
        CONNECT_TIMEOUT_S
    } else {
        SIGN_IN_TIMEOUT_S
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
        // against greyed rows, applied to the column. Five of these nine are
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
