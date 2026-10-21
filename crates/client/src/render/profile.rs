//! A player's profile card, fetched: the target a click on a name set, and
//! the two public reads that fill it (`/api/face`, `/api/playtime`). The
//! card itself is a panel (`panels::profile`); the friends buttons on it go
//! through [`super::friends::Friends::act`].
//!
//! **Read when it is opened, never on a timer**, the friends list's rule: a
//! card opened twice reads twice, a card left open reads once.
//!
//! All arithmetic is `crate::profile`; this file owns the threads, the
//! channel and the one resource.

use bevy::prelude::*;
use protocol::Address;

use super::friends::hex;
use crate::profile::{self, Face, Playtime};

/// What came back from a thread (natively) or a promise (in a page).
enum Landed {
    Face(Address, Result<Face, String>),
    Play(Address, Result<Playtime, String>),
    // Only the launcher's open sends this, and that is desktop only.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    Said(Address, String),
}

type Tx = tokio::sync::mpsc::UnboundedSender<Landed>;
type Rx = tokio::sync::mpsc::UnboundedReceiver<Landed>;

/// Whose card is up, and what the platform said about them.
#[derive(Resource)]
pub struct Profile {
    /// The wallet the card is about. `None` before any click.
    pub who: Option<Address>,
    /// The name the click carried (a standings row's, a chat line's), shown
    /// until the platform's own lands.
    pub hint: String,
    /// The picture revision the shard's tag carried, if the player is here.
    pub tag_pic: u32,
    pub face: Option<Face>,
    pub play: Option<Playtime>,
    /// Why a read failed, if one did.
    pub why: Option<String>,
    /// What the last press on the card did that the friends list does not
    /// say: opening their page.
    pub said: String,
    /// Bumped whenever anything above changes; the card redraws on a new one.
    pub gen: u64,
    tx: Tx,
    rx: Rx,
}

impl Default for Profile {
    fn default() -> Self {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        Self {
            who: None,
            hint: String::new(),
            tag_pic: 0,
            face: None,
            play: None,
            why: None,
            said: String::new(),
            gen: 0,
            tx,
            rx,
        }
    }
}

impl Profile {
    /// Point the card at `who` and read their face and playtime from
    /// `origin`. What the last card read is dropped at once, so nothing on it
    /// can be another player's.
    pub fn open(&mut self, origin: &str, who: Address, hint: &str, tag_pic: u32) {
        self.who = Some(who);
        self.hint = hint.chars().take(crate::friends::MAX_NAME_CHARS).collect();
        self.tag_pic = tag_pic;
        self.face = None;
        self.play = None;
        self.why = None;
        self.said.clear();
        self.gen += 1;
        let w = hex(who);
        read(profile::face_url(origin, &w), who, false, self.tx.clone());
        read(
            profile::playtime_url(origin, &w),
            who,
            true,
            self.tx.clone(),
        );
    }

    /// The picture to draw: the platform's revision once it has said, else
    /// the tag's.
    pub fn pic(&self) -> u32 {
        self.face
            .as_ref()
            .and_then(Face::pic_rev)
            .unwrap_or(self.tag_pic)
    }
}

fn landed(who: Address, play: bool, body: Option<Vec<u8>>) -> Landed {
    let body = body.ok_or_else(|| "Elo Pros did not answer".to_string());
    if play {
        Landed::Play(who, body.and_then(|b| profile::parse_playtime(&b)))
    } else {
        Landed::Face(who, body.and_then(|b| profile::parse_face(&b)))
    }
}

fn cap(play: bool) -> usize {
    if play {
        profile::MAX_PLAYTIME_BYTES
    } else {
        profile::MAX_FACE_BYTES
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn read(url: String, who: Address, play: bool, tx: Tx) {
    std::thread::spawn(move || {
        let body = super::faces::get_bytes(&url, cap(play));
        let _ = tx.send(landed(who, play, body));
    });
}

#[cfg(target_arch = "wasm32")]
fn read(url: String, who: Address, play: bool, tx: Tx) {
    wasm_bindgen_futures::spawn_local(async move {
        let body = crate::net::web::fetch_bytes(&url, cap(play)).await;
        let _ = tx.send(landed(who, play, body));
    });
}

/// Take what landed. A read for a card no longer up is dropped.
pub fn pump(mut card: ResMut<Profile>) {
    if card.rx.is_empty() {
        return;
    }
    while let Ok(l) = card.rx.try_recv() {
        match l {
            Landed::Face(who, got) if card.who == Some(who) => match got {
                Ok(f) => card.face = Some(f),
                Err(e) => card.why = Some(e),
            },
            Landed::Play(who, got) if card.who == Some(who) => match got {
                Ok(p) => card.play = Some(p),
                Err(e) => card.why = Some(e),
            },
            Landed::Said(who, line) if card.who == Some(who) => card.said = line,
            _ => continue,
        }
        card.gen += 1;
    }
}

impl Profile {
    /// Open their page on Elo Pros: a new tab in a page; natively the
    /// launcher's browser, off the frame, since finding the launcher is a
    /// local connection. [`Profile::said`] says what happened.
    pub fn open_page(&mut self, origin: &str) {
        let Some(who) = self.who else {
            return;
        };
        let url = profile::page_url(origin, &hex(who));
        #[cfg(target_arch = "wasm32")]
        {
            let opened = web_sys::window()
                .and_then(|w| w.open_with_url_and_target(&url, "_blank").ok())
                .flatten()
                .is_some();
            self.said = if opened {
                "Their Elo Pros page is open in a new tab.".into()
            } else {
                format!("Your browser blocked the tab: {url}")
            };
            self.gen += 1;
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.said = "opening their page...".into();
            self.gen += 1;
            let tx = self.tx.clone();
            std::thread::spawn(move || {
                let mut elo = crate::elo::Elo::discover(None, env!("CARGO_PKG_VERSION"));
                let line = if elo.open(&url) {
                    "Their Elo Pros page is open in your browser.".into()
                } else {
                    format!("Open it at {url}")
                };
                let _ = tx.send(Landed::Said(who, line));
            });
        }
    }
}

pub fn register(app: &mut App) {
    app.init_resource::<Profile>().add_systems(Update, pump);
}
