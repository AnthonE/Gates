//! Contacts — Rust's (the June 2021 update): the players you have met. Spend
//! a few seconds near someone and they go in the book as SEEN, with a
//! mugshot, a screen grab taken when you met. Mark one FRIENDLY or ENEMY and
//! their nametag says so; someone who kills you goes in as an ENEMY unless
//! you marked them a friend. Each can carry a note in your own words.
//!
//! This client's own memory, kept on this computer: a local mark, nothing on
//! the wire and nothing on the platform. An Elo Pros friend is a different
//! thing (`crate::friends`), asked and accepted by both wallets.
//!
//! Pure and on every target. `render/contacts.rs` owns the watching, the
//! screen grab and the file.

use protocol::Address;
use serde::{Deserialize, Serialize};

/// How close a player must stand, metres, to be met.
pub const SEEN_M: f32 = 10.0;
/// And for how long, seconds, without stepping out of range or sight.
pub const SEEN_S: f32 = 3.0;
/// A meeting this long after the last sighting is a new meeting: `met` counts
/// encounters, not frames.
pub const ENCOUNTER_GAP_S: u64 = 10 * 60;
/// A mugshot is retaken once it is this old, the next time they are met.
pub const MUGSHOT_REFRESH_S: u64 = 30 * 60;
/// Contacts kept. Past it the longest-unseen SEEN goes; a marked one stays.
pub const MAX_CONTACTS: usize = 200;
/// A mugshot's side, pixels.
pub const MUGSHOT_PX: u32 = 128;
/// Largest book read off disk.
pub const MAX_BOOK_BYTES: usize = 512 * 1024;
/// A name kept, characters.
pub const MAX_NAME_CHARS: usize = 32;
/// A note kept, characters: a line or two in the pane.
pub const MAX_NOTE_CHARS: usize = 120;

/// Where a contact stands with you.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Rel {
    #[default]
    Seen,
    Friendly,
    Enemy,
}

impl Rel {
    pub fn label(self) -> &'static str {
        match self {
            Rel::Seen => "SEEN",
            Rel::Friendly => "FRIENDLY",
            Rel::Enemy => "ENEMY",
        }
    }
}

/// One player met.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct Contact {
    /// Lowercase `0x…`.
    pub wallet: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub rel: Rel,
    /// They played as a declared agent when last met.
    #[serde(default)]
    pub agent: bool,
    /// Unix seconds.
    #[serde(default)]
    pub first_seen: u64,
    #[serde(default)]
    pub last_seen: u64,
    /// Encounters, each [`ENCOUNTER_GAP_S`] apart.
    #[serde(default)]
    pub met: u32,
    /// When the mugshot was taken; 0 for none.
    #[serde(default)]
    pub mugshot_at: u64,
    /// Times they killed you.
    #[serde(default)]
    pub killed_me: u32,
    /// What you wrote about them.
    #[serde(default)]
    pub note: String,
}

impl Contact {
    pub fn address(&self) -> Option<Address> {
        Address::from_hex(self.wallet.as_bytes())
    }

    /// The name, else the short wallet.
    pub fn label(&self) -> String {
        if self.name.is_empty() {
            crate::friends::short_wallet(&self.wallet)
        } else {
            self.name.clone()
        }
    }
}

/// What a sighting changed, so the file is written when it matters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sighting {
    /// Not in the book before.
    New,
    /// Back after [`ENCOUNTER_GAP_S`]: `met` counted it.
    Again,
    /// Still here: only the time moved.
    Still,
    /// The book is full of marked contacts; nobody was dropped for them.
    Full,
}

/// Which contacts a page lists.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Filter {
    #[default]
    All,
    Friendly,
    Enemy,
    Seen,
}

impl Filter {
    pub const ALL: [Filter; 4] = [Filter::All, Filter::Friendly, Filter::Enemy, Filter::Seen];

    pub fn label(self) -> &'static str {
        match self {
            Filter::All => "ALL",
            Filter::Friendly => "FRIENDLY",
            Filter::Enemy => "ENEMY",
            Filter::Seen => "SEEN",
        }
    }

    pub fn keeps(self, rel: Rel) -> bool {
        match self {
            Filter::All => true,
            Filter::Friendly => rel == Rel::Friendly,
            Filter::Enemy => rel == Rel::Enemy,
            Filter::Seen => rel == Rel::Seen,
        }
    }
}

/// The book.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct Book {
    #[serde(default)]
    pub contacts: Vec<Contact>,
}

fn tidy_name(s: &str) -> String {
    s.trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_NAME_CHARS)
        .collect()
}

/// A note as kept: no control characters, no ends of blank, capped.
pub fn tidy_note(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_control())
        .take(MAX_NOTE_CHARS)
        .collect::<String>()
        .trim()
        .to_string()
}

fn key(a: &Address) -> String {
    String::from_utf8_lossy(&a.to_hex()).into_owned()
}

impl Book {
    /// Read a book. What does not read is an empty book, never an error: the
    /// file is this client's own, and a torn one must not cost the session.
    pub fn parse(bytes: &[u8]) -> Book {
        if bytes.len() > MAX_BOOK_BYTES {
            return Book::default();
        }
        let mut book: Book = serde_json::from_slice(bytes).unwrap_or_default();
        let mut kept: Vec<Contact> = Vec::with_capacity(book.contacts.len());
        for mut c in book.contacts.drain(..) {
            c.wallet.make_ascii_lowercase();
            c.name = tidy_name(&c.name);
            c.note = tidy_note(&c.note);
            let ok = c.address().is_some_and(|a| !a.is_guest());
            if ok && !kept.iter().any(|k| k.wallet == c.wallet) {
                kept.push(c);
            }
        }
        book.contacts = kept;
        while book.contacts.len() > MAX_CONTACTS && book.evict() {}
        book.contacts.truncate(MAX_CONTACTS);
        book
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{\"contacts\":[]}".into())
    }

    pub fn get(&self, wallet: &Address) -> Option<&Contact> {
        let k = key(wallet);
        self.contacts.iter().find(|c| c.wallet == k)
    }

    fn get_mut(&mut self, wallet: &Address) -> Option<&mut Contact> {
        let k = key(wallet);
        self.contacts.iter_mut().find(|c| c.wallet == k)
    }

    /// Drop the longest-unseen unmarked contact. False when every one is
    /// marked.
    fn evict(&mut self) -> bool {
        let oldest = self
            .contacts
            .iter()
            .enumerate()
            .filter(|(_, c)| c.rel == Rel::Seen)
            .min_by_key(|(_, c)| c.last_seen)
            .map(|(i, _)| i);
        match oldest {
            Some(i) => {
                self.contacts.remove(i);
                true
            }
            None => false,
        }
    }

    /// `wallet` was met at `now`, called `name`.
    pub fn seen(&mut self, wallet: &Address, name: &str, agent: bool, now: u64) -> Sighting {
        if wallet.is_guest() {
            return Sighting::Full;
        }
        let name = tidy_name(name);
        if let Some(c) = self.get_mut(wallet) {
            let again = now.saturating_sub(c.last_seen) >= ENCOUNTER_GAP_S;
            if again {
                c.met = c.met.saturating_add(1);
            }
            c.last_seen = c.last_seen.max(now);
            c.agent = agent;
            if !name.is_empty() {
                c.name = name;
            }
            return if again {
                Sighting::Again
            } else {
                Sighting::Still
            };
        }
        if self.contacts.len() >= MAX_CONTACTS && !self.evict() {
            return Sighting::Full;
        }
        self.contacts.push(Contact {
            wallet: key(wallet),
            name,
            rel: Rel::Seen,
            agent,
            first_seen: now,
            last_seen: now,
            met: 1,
            mugshot_at: 0,
            killed_me: 0,
            note: String::new(),
        });
        Sighting::New
    }

    /// `wallet` killed you: into the book if they were not, and an enemy
    /// unless you marked them a friend (Rust's rule: a friend who kills you
    /// stays a friend).
    pub fn killed_by(&mut self, wallet: &Address, name: &str, agent: bool, now: u64) {
        if self.seen(wallet, name, agent, now) == Sighting::Full {
            return;
        }
        if let Some(c) = self.get_mut(wallet) {
            c.killed_me = c.killed_me.saturating_add(1);
            if c.rel == Rel::Seen {
                c.rel = Rel::Enemy;
            }
        }
    }

    /// Mark a contact. False when they are not in the book.
    pub fn set_rel(&mut self, wallet: &Address, rel: Rel) -> bool {
        match self.get_mut(wallet) {
            Some(c) => {
                c.rel = rel;
                true
            }
            None => false,
        }
    }

    /// Write a note on a contact (empty clears it). False when they are not
    /// in the book or the note did not change.
    pub fn set_note(&mut self, wallet: &Address, note: &str) -> bool {
        let note = tidy_note(note);
        match self.get_mut(wallet) {
            Some(c) if c.note != note => {
                c.note = note;
                true
            }
            _ => false,
        }
    }

    /// Take a contact out of the book.
    pub fn forget(&mut self, wallet: &Address) -> bool {
        let k = key(wallet);
        let before = self.contacts.len();
        self.contacts.retain(|c| c.wallet != k);
        self.contacts.len() != before
    }

    /// A mugshot of `wallet` was taken at `now`.
    pub fn shot(&mut self, wallet: &Address, now: u64) {
        if let Some(c) = self.get_mut(wallet) {
            c.mugshot_at = now;
        }
    }

    /// Whether `wallet` is owed a mugshot: none yet, or one gone stale.
    pub fn wants_mugshot(&self, wallet: &Address, now: u64) -> bool {
        self.get(wallet).is_some_and(|c| {
            c.mugshot_at == 0 || now.saturating_sub(c.mugshot_at) >= MUGSHOT_REFRESH_S
        })
    }

    /// The contacts `filter` keeps, the most recently seen first.
    pub fn shown(&self, filter: Filter) -> Vec<&Contact> {
        let mut v: Vec<&Contact> = self
            .contacts
            .iter()
            .filter(|c| filter.keeps(c.rel))
            .collect();
        v.sort_by(|a, b| b.last_seen.cmp(&a.last_seen).then(a.wallet.cmp(&b.wallet)));
        v
    }

    pub fn count(&self, filter: Filter) -> usize {
        self.contacts.iter().filter(|c| filter.keeps(c.rel)).count()
    }
}

/// Who has been near for how long: the dwell timer that turns a passer-by
/// into a contact.
#[derive(Clone, Debug, Default)]
pub struct Watch {
    near: Vec<(Address, f32)>,
}

impl Watch {
    /// One look, `dt` seconds after the last: `close` is every player within
    /// [`SEEN_M`] with a clear line. Anyone not in it starts over. Returns
    /// those who have now been close for [`SEEN_S`] or more, and whether
    /// this look is the one that crossed it.
    pub fn step(&mut self, dt: f32, close: &[Address]) -> Vec<(Address, bool)> {
        self.near.retain(|(a, _)| close.contains(a));
        let mut out = Vec::new();
        for a in close {
            let t = match self.near.iter_mut().find(|(n, _)| n == a) {
                Some((_, t)) => t,
                None => {
                    self.near.push((*a, 0.0));
                    &mut self.near.last_mut().expect("just pushed").1
                }
            };
            let before = *t;
            *t += dt.max(0.0);
            if *t >= SEEN_S {
                out.push((*a, before < SEEN_S));
            }
        }
        out
    }

    pub fn clear(&mut self) {
        self.near.clear();
    }
}

/// `5m ago`, `just now`: how long since a contact was seen.
pub fn ago(now: u64, at: u64) -> String {
    crate::friends::ago(now.saturating_sub(at))
}

/// `met 3 times`, `met once`.
pub fn met_line(c: &Contact) -> String {
    match c.met {
        0 | 1 => "met once".into(),
        n => format!("met {n} times"),
    }
}

/// A mugshot crop: the square around a body on screen, from where its feet
/// and head land, clamped inside a `w` × `h` frame. `None` when it is too
/// small to be a picture of anyone (far away) or mostly off the frame.
pub fn mugshot_rect(feet: (f32, f32), head: (f32, f32), w: u32, h: u32) -> Option<(u32, u32, u32)> {
    let tall = (feet.1 - head.1).abs();
    if !tall.is_finite() || tall < MUGSHOT_MIN_PX {
        return None;
    }
    // Head and body, a mugshot rather than a full-length: the square is a
    // little taller than the body is to the head, centred above the waist.
    let side = (tall * 1.05).round().min(w.min(h) as f32);
    let cx = feet.0 + (head.0 - feet.0) * 0.6;
    let cy = feet.1 + (head.1 - feet.1) * 0.6;
    if cx < 0.0 || cy < 0.0 || cx > w as f32 || cy > h as f32 {
        return None;
    }
    let x = (cx - side * 0.5).clamp(0.0, w as f32 - side);
    let y = (cy - side * 0.5).clamp(0.0, h as f32 - side);
    Some((x.round() as u32, y.round() as u32, side as u32))
}

/// A body shorter than this on screen, physical pixels, is too far for a
/// mugshot; one is taken the next time they are met closer.
pub const MUGSHOT_MIN_PX: f32 = 48.0;

/// Cut the square `(x, y, side)` out of a `w`-wide frame of 4-byte pixels
/// and box it down to [`MUGSHOT_PX`]², as RGBA with full alpha. `bgra` swaps
/// the first and third bytes (the window surface's usual order).
pub fn crop_mugshot(
    frame: &[u8],
    w: u32,
    bgra: bool,
    (x, y, side): (u32, u32, u32),
) -> Option<Vec<u8>> {
    let (w, x, y, side) = (w as usize, x as usize, y as usize, side as usize);
    if side == 0 || (y + side) * w * 4 > frame.len() || x + side > w {
        return None;
    }
    let out = MUGSHOT_PX as usize;
    let mut px = vec![0u8; out * out * 4];
    for oy in 0..out {
        let y0 = y + oy * side / out;
        let y1 = (y + (oy + 1) * side / out).max(y0 + 1);
        for ox in 0..out {
            let x0 = x + ox * side / out;
            let x1 = (x + (ox + 1) * side / out).max(x0 + 1);
            let mut sum = [0u32; 3];
            for sy in y0..y1 {
                for sx in x0..x1 {
                    let o = (sy * w + sx) * 4;
                    let p = &frame[o..o + 4];
                    let (r, b) = if bgra { (p[2], p[0]) } else { (p[0], p[2]) };
                    sum[0] += r as u32;
                    sum[1] += p[1] as u32;
                    sum[2] += b as u32;
                }
            }
            let n = ((y1 - y0) * (x1 - x0)) as u32;
            let o = (oy * out + ox) * 4;
            px[o] = (sum[0] / n) as u8;
            px[o + 1] = (sum[1] / n) as u8;
            px[o + 2] = (sum[2] / n) as u8;
            px[o + 3] = 255;
        }
    }
    Some(px)
}
