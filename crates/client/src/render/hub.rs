//! The launcher-backed half of the front end: the title manifest, fetched,
//! and the click that hands a section to the launcher.
//!
//! The model is [`crate::ui::hub`] — which section is in which of four states
//! and what the screen says about each. This is the Bevy half: one fetch, one
//! resource, one click handler. `render/menu.rs` draws it.
//!
//! **The launcher is held for the life of the app.** `Scry` used to be built
//! before the window, read for a `Player`, and dropped — so the socket that
//! could sign a challenge, name a shard list and open a page was closed
//! before the first frame. It is a resource now, non-send because
//! `Overlay` owns a `UnixStream`, and it is the same connection the boot
//! splash made.

use bevy::prelude::*;

use crate::manifest::{self, Manifest};
use crate::scry::Scry;
use crate::ui::hub::{Hub, Section};

/// The manifest, and the fetch that is filling it.
#[derive(Resource, Default)]
pub struct HubState {
    pub hub: Hub,
    /// The in-flight GET. tokio's channel for `Menu`'s reason: a `Resource`
    /// must be `Send + Sync` and `std`'s `Receiver` is only `Send`.
    fetch: Option<tokio::sync::mpsc::UnboundedReceiver<Result<Manifest, String>>>,
    /// Set when the drawn screen no longer matches this resource.
    pub dirty: bool,
}

/// Fetch the manifest, on a thread.
///
/// Started when the launcher handshake lands and **not** waited on by the
/// splash: the two waits the splash holds for are local (assets, a unix
/// socket), and this is a real network GET. A section that has not resolved
/// yet reads `Fetching`, which is one of the four states precisely so the
/// screen never has to lie about this interval.
pub fn begin_fetch(state: &mut HubState, url: String) {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    std::thread::spawn(move || {
        let _ = tx.send(fetch_blocking(&url));
    });
    state.fetch = Some(rx);
}

/// One GET, capped. The sibling of `menu::fetch_blocking`, bounded for the
/// same reason and with the same refuse-don't-truncate policy: this is bytes
/// from a host named by another host, and one of the values in it will be
/// handed to the player's desktop to open.
fn fetch_blocking(url: &str) -> Result<Manifest, String> {
    let mut res = ureq::get(url)
        .call()
        .map_err(|e| format!("title manifest: {e}"))?;
    let bytes = res
        .body_mut()
        .with_config()
        .limit((manifest::MAX_MANIFEST_BYTES + 1) as u64)
        .read_to_vec()
        .map_err(|e| format!("title manifest: {e}"))?;
    manifest::parse(&bytes)
}

/// Collect the fetch if it has landed. Non-blocking, every frame the menu is
/// up.
pub fn poll(mut state: ResMut<HubState>) {
    if state.fetch.is_none() {
        // Checked through an immutable deref first, so the overwhelmingly
        // common frame does not flag the resource changed for nothing —
        // `menu::poll_status` makes the same point at more length.
        return;
    }
    let Some(rx) = &mut state.fetch else {
        return;
    };
    let got = match rx.try_recv() {
        Ok(got) => got,
        Err(tokio::sync::mpsc::error::TryRecvError::Empty) => return,
        Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
            Err("the title manifest fetch did not finish".to_string())
        }
    };
    state.fetch = None;
    match got {
        Ok(m) => state.hub.manifest = Some(m),
        // Drawn verbatim on every section, because `manifest::parse` writes
        // these for a reader — they name the cap and the document.
        Err(why) => state.hub.why = Some(why),
    }
    state.dirty = true;
}

/// Open a section, through the launcher.
///
/// Three outcomes and the screen distinguishes all of them, because a button
/// that silently does nothing is worse than one that says it failed:
/// there is no url (the click should not have been offered), the launcher
/// refused, or it opened.
pub fn open(scry: Option<&mut Scry>, hub: &Hub, section: Section) -> Opened {
    let Some(url) = section.status(hub).url().map(str::to_string) else {
        return Opened::NoLink;
    };
    // Checked HERE as well as at parse time. The parse-time check is what
    // keeps a junk link out of the model; this is the last thing between a
    // string from a document and a verb that reaches the player's desktop,
    // and the two are cheap.
    if manifest::check_url(&url).is_err() {
        return Opened::NoLink;
    }
    match scry {
        None => Opened::NoLauncher,
        Some(s) => {
            if s.open(&url) {
                Opened::Opened
            } else {
                Opened::Refused
            }
        }
    }
}

/// What a click on a section actually did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Opened {
    Opened,
    Refused,
    NoLauncher,
    NoLink,
}

impl Opened {
    /// What to put on the status line afterwards. Says what happened in every
    /// case, including success — a page that opens behind the game window is
    /// a page a fullscreen player cannot see, so "it opened" is information.
    pub fn line(self, label: &str) -> String {
        match self {
            Opened::Opened => format!("{label} is open in your browser"),
            Opened::Refused => {
                format!("the launcher would not open {label} - check its window")
            }
            Opened::NoLauncher => {
                "the scry launcher is not running - start the game from it to reach this"
                    .to_string()
            }
            Opened::NoLink => format!("nothing is published at {label} yet"),
        }
    }
}
