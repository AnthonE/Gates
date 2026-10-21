//! A player's profile card: who a wallet is on Elo Pros, whether they are
//! playing, how long they have played Gates, and where they stand on your
//! friends list. Opened by a click on a name: a standings row, a chat line.
//!
//! Everything it reads is public on the platform: `GET /api/face/{wallet}`
//! (name, picture, agent or human) and `GET /api/playtime/{wallet}` (playing
//! now, last seen, hours per game), plus the friends list `crate::friends`
//! already holds. The one write is a friends op, through that module's
//! prepare → sign → act.
//!
//! Pure and on every target, wasm included; `render/profile.rs` owns the
//! reads.

use protocol::Address;
use serde::Deserialize;

use crate::friends::{self, Op, Playing, View};

/// Largest `/api/face/{wallet}` body read.
pub const MAX_FACE_BYTES: usize = 8 * 1024;
/// Largest `/api/playtime/{wallet}` body read: a row per game played.
pub const MAX_PLAYTIME_BYTES: usize = 32 * 1024;

/// `GET /api/face/{wallet}`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct Face {
    #[serde(default)]
    pub name: Option<String>,
    /// The picture's revision, the first eight hex digits of its sha256.
    #[serde(default)]
    pub pic: Option<String>,
    /// `agent` or `human`, as the wallet's own signed face says.
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub harness: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
}

impl Face {
    /// The picture revision `render::faces::Pics` keys on, if there is one.
    pub fn pic_rev(&self) -> Option<u32> {
        let v = self.pic.as_deref()?;
        if v.is_empty() || v.len() > 8 {
            return None;
        }
        u32::from_str_radix(v, 16).ok().map(|r| r.max(1))
    }

    pub fn agent(&self) -> bool {
        self.kind.as_deref() == Some("agent")
    }
}

/// `GET /api/playtime/{wallet}`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct Playtime {
    #[serde(default)]
    pub online: bool,
    #[serde(default)]
    pub playing: Option<Playing>,
    #[serde(default)]
    pub last_online: Option<u64>,
    #[serde(default)]
    pub games: Vec<GameTime>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct GameTime {
    #[serde(default)]
    pub game: String,
    #[serde(default)]
    pub seconds: u64,
}

fn tidy(s: &mut Option<String>, cap: usize) {
    *s = s
        .take()
        .map(|t| t.trim().chars().take(cap).collect::<String>())
        .filter(|t| !t.is_empty() && !t.chars().any(char::is_control));
}

pub fn parse_face(bytes: &[u8]) -> Result<Face, String> {
    if bytes.len() > MAX_FACE_BYTES {
        return Err("their Elo Pros face was too long to read".into());
    }
    let mut f: Face =
        serde_json::from_slice(bytes).map_err(|_| "their Elo Pros face did not read")?;
    tidy(&mut f.name, friends::MAX_NAME_CHARS);
    tidy(&mut f.harness, friends::MAX_NAME_CHARS);
    tidy(&mut f.model, friends::MAX_NAME_CHARS);
    Ok(f)
}

pub fn parse_playtime(bytes: &[u8]) -> Result<Playtime, String> {
    if bytes.len() > MAX_PLAYTIME_BYTES {
        return Err("their playtime was too long to read".into());
    }
    serde_json::from_slice(bytes).map_err(|_| "their playtime did not read".to_string())
}

/// `{origin}/api/face/{wallet}`.
pub fn face_url(origin: &str, wallet: &str) -> String {
    format!(
        "{}/api/face/{}",
        origin.trim_end_matches('/'),
        wallet.to_ascii_lowercase()
    )
}

/// `{origin}/api/playtime/{wallet}`.
pub fn playtime_url(origin: &str, wallet: &str) -> String {
    format!(
        "{}/api/playtime/{}",
        origin.trim_end_matches('/'),
        wallet.to_ascii_lowercase()
    )
}

/// Their page on Elo Pros, where the same buttons work in a browser.
pub fn page_url(origin: &str, wallet: &str) -> String {
    format!(
        "{}{}?who={}",
        origin.trim_end_matches('/'),
        friends::PROFILE_PATH,
        wallet.to_ascii_lowercase()
    )
}

/// Where a wallet stands on your friends list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rel {
    /// Nobody is signed in here, so there is no list to stand on.
    SignedOut,
    /// Your own card.
    Me,
    Friend,
    /// One of your own agents: on the list without an ask.
    Buddy,
    /// You asked; they have not answered.
    AskedThem,
    /// They asked you.
    AskedYou,
    Stranger,
    /// Signed in, and the list has not been read yet.
    Unknown,
}

/// `them` on `me`'s list, as `view` last read it.
pub fn rel(me: Option<Address>, view: Option<&View>, them: &Address) -> Rel {
    let Some(me) = me else {
        return Rel::SignedOut;
    };
    if me == *them {
        return Rel::Me;
    }
    let Some(v) = view else {
        return Rel::Unknown;
    };
    let is = |w: &str| Address::from_hex(w.as_bytes()).as_ref() == Some(them);
    if v.friends.iter().any(|f| is(&f.wallet)) {
        Rel::Friend
    } else if v.buddies.iter().any(|f| is(&f.wallet)) {
        Rel::Buddy
    } else if v.asks_in.iter().any(|a| is(&a.wallet)) {
        Rel::AskedYou
    } else if v.asks_out.iter().any(|a| is(&a.wallet)) {
        Rel::AskedThem
    } else {
        Rel::Stranger
    }
}

/// The line on the card that says where you stand.
pub fn rel_line(rel: Rel) -> &'static str {
    match rel {
        Rel::SignedOut => "Sign in with a wallet to add friends.",
        Rel::Me => "This is you.",
        Rel::Friend => "FRIENDS",
        Rel::Buddy => "YOUR AGENT",
        Rel::AskedThem => "Friend request sent. Waiting on them.",
        Rel::AskedYou => "They want to be your friend.",
        Rel::Stranger => "",
        Rel::Unknown => "Reading your friends list...",
    }
}

/// The friends buttons the card offers, primary first.
pub fn acts(rel: Rel) -> &'static [(&'static str, Op)] {
    match rel {
        Rel::Stranger => &[("ADD FRIEND", Op::Ask)],
        Rel::AskedYou => &[("ACCEPT", Op::Accept), ("DECLINE", Op::Decline)],
        Rel::AskedThem => &[("CANCEL REQUEST", Op::Cancel)],
        Rel::Friend => &[("REMOVE FRIEND", Op::Remove)],
        Rel::SignedOut | Rel::Me | Rel::Buddy | Rel::Unknown => &[],
    }
}

/// `312 h in Gates`, `40 min in Gates`, or `None` before they played.
pub fn hours_line(play: &Playtime) -> Option<String> {
    let secs = play
        .games
        .iter()
        .filter(|g| g.game == friends::GAME)
        .map(|g| g.seconds)
        .sum::<u64>();
    match secs {
        0 => None,
        s if s < 3_600 => Some(format!("{} min in Gates", (s / 60).max(1))),
        s => Some(format!("{} h in Gates", s / 3_600)),
    }
}

/// `Playing Gates · EU 1`, `online`, `last seen 3h ago` or `offline`: the
/// friends list's own line, from what playtime says.
pub fn seen_line(play: &Playtime, now: u64) -> String {
    friends::presence_line(
        &friends::Friend {
            online: play.online,
            playing: play.playing.clone(),
            last_online: play.last_online,
            ..Default::default()
        },
        now,
    )
}

/// The name to put on the card: the platform's, else the one the click
/// carried, else the short wallet.
pub fn title(face: Option<&Face>, hint: &str, wallet: &str) -> String {
    face.and_then(|f| f.name.clone())
        .or_else(|| {
            let h = hint.trim();
            (!h.is_empty()).then(|| h.to_string())
        })
        .unwrap_or_else(|| friends::short_wallet(wallet))
}

/// `AGENT · claude-code · claude-opus`: what a program says it runs on.
pub fn agent_line(face: &Face) -> Option<String> {
    if !face.agent() {
        return None;
    }
    let mut parts = vec!["AGENT".to_string()];
    parts.extend(face.harness.clone());
    parts.extend(face.model.clone());
    Some(parts.join(" · "))
}
