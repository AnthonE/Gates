//! Where the client is, and the handful of resources that say so.
//!
//! **These seven types were defined in `menu.rs` and `boot.rs`, and that was
//! the wrong home for them the moment there were two clients.** `Screen` is
//! the app's state machine — twelve modules match on it, from `loading` to
//! `pause` to `capture` — while `menu.rs` is one screen that happens to be
//! the first one a desktop player sees. A browser build has no menu at all
//! (the page is the menu: it owns the shard address, the wallet and the
//! join), so `menu.rs`, `hub.rs` and `boot.rs` do not compile for wasm32 —
//! they reach a local launcher over a unix socket, fetch over a blocking
//! socket, and own a tokio runtime. Cfg-ing those three modules off that
//! target while the state enum still lived inside one of them took the build
//! from 14 errors to 103.
//!
//! So this module is the shared half, lifted out whole. Nothing here is new
//! and nothing changed shape — the one edit beyond the move is that `Menu`'s
//! two receiver fields are `pub(super)` instead of private, because their
//! only writer (`menu::fetch_now`, `menu::begin_status_poll`) is now a
//! sibling rather than the same module.
//!
//! **What deliberately did NOT move: `menu::Rt`.** It wraps a
//! `tokio::runtime::Runtime`, which does not exist on this crate's wasm
//! target (`Cargo.toml` gives tokio only `sync` there), and its two readers
//! are both native. It stays with the screen that drives it and dies with it.

use bevy::prelude::*;

use crate::elo::Player;
#[cfg(not(target_arch = "wasm32"))]
use crate::shardlist::{self, Shard};
use crate::ui::hub::Section;
use crate::ui::servers::{Favourites, Filter, Listing};

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
    // Native-only, because their only writer is: a page fetches no shard
    // list — it IS the shard list, the player having chosen the address
    // before Bevy started. Carrying them on wasm would be two fields
    // nothing can ever fill, which clippy calls dead and is right to.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fetch: Option<tokio::sync::mpsc::UnboundedReceiver<Result<Vec<Shard>, String>>>,
    /// The in-flight round of status polls, one per row that names an
    /// endpoint. Collected as a batch rather than per row so the frame does
    /// one `try_recv` however many shards are listed.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) status_poll:
        Option<tokio::sync::mpsc::UnboundedReceiver<Vec<(usize, shardlist::Status)>>>,
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
    pub rx: Option<std::sync::mpsc::Receiver<Result<crate::Session, crate::JoinError>>>,
    /// Seconds spent on this attempt. Accumulated from Bevy's frame delta
    /// rather than an `Instant`, so the screen has one clock and it is the
    /// renderer's.
    pub waited_s: f32,
}

/// Who the launcher says is playing.
///
/// **Moved off `gates.rs` and into a state**, which is the whole point of
/// this module: it used to be resolved before the window by a blocking
/// socket call. Not gameplay state — nothing in the sim reads it and nothing
/// can — so "Bevy draws, it does not decide" is untouched.
#[derive(Resource)]
pub struct Who(pub Player);

/// The address this binary was started with. Held as a resource so the splash
/// can hand it to the connect screen without reaching into the plugin's own
/// construction arguments.
#[derive(Resource)]
pub struct Direct(pub String);

// The constructors came across with the types. `Menu::new`'s empty-state
// sentence names the desktop's two ways to get a shard list, which is
// correct for the only target that draws it — a browser constructs this
// resource and never reads it, because the page is the menu there.
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
            #[cfg(not(target_arch = "wasm32"))]
            fetch: None,
            #[cfg(not(target_arch = "wasm32"))]
            status_poll: None,
        }
    }
}

impl Default for Who {
    fn default() -> Self {
        Self(Player::Anonymous)
    }
}
