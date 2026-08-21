//! The server browser's **model** — the rows, and which of them the filters
//! leave standing.
//!
//! **The frame this answers to** is the reference's PLAY GAME screen: a
//! category rail down the left with a live count under each name, a search
//! box, a couple of filter groups, CLEAR FILTERS and REFRESH at the bottom,
//! and on the right a *table* — `SERVER NAME` / `PLAYERS` / `PING`, a
//! favourite star on every row, and a dim second line carrying the map. Ours
//! is that shape against the fields `elo-shardlist-v1` actually publishes,
//! and no others.
//!
//! **What is deliberately not copied, and why.** The reference's rail reads
//! Official / Community / Modded / Friends / Favourited / History. Four of
//! those are facts about a shard ecosystem we do not have: nothing in the
//! document says who runs a shard, nothing marks one modded, and there is no
//! friend graph. `render/settings.rs` already set the policy for this — *a
//! category with nothing behind it says so in a sentence rather than drawing
//! greyed rows that imply a feature exists* — so the rail here is the two
//! categories that are real. The same goes for the reference's WIPE SCHEDULE
//! group and its coloured region tags: real columns of their document, absent
//! from ours, and a chip that always reads the same word is a decoration
//! pretending to be data.
//!
//! **Pure, and not feature-gated**, for the reason [`crate::ui`]'s header
//! gives: filtering, sorting and the favourite set are arithmetic, they are
//! what a browser actually gets wrong, and arithmetic that needs a window to
//! run needs a window to test. `crates/client/tests/ui.rs` §L drives all of
//! it headless.

// Both of this client's search boxes share one cap. The browser landed with
// its own copy at a different value and `ci/knob_registry.mjs` refused it:
// a name that means two things is a name the registry cannot be
// authoritative about.
use crate::ui::MAX_QUERY_CHARS;

/// The most shards that may be starred.
///
/// **PROPOSED — `DECISIONS.md` §open, "server browser v0".** Bounded because
/// the list is written to the settings file and read back at every launch, so
/// an unbounded one is a file that grows forever on a player who stars a
/// shard a week. Comfortably past `shardlist::MAX_SHARDS` (64) — starring
/// every shard on a full list must never be the thing that hits the cap, or
/// the cap would be a rule about the list rather than about the file.
pub const MAX_FAVOURITES: usize = 128;

/// One row of the browser.
///
/// **Structured, not a pre-baked line.** The row used to carry a `detail`
/// string built once at fetch time; a table with its own PLAYERS and PING
/// columns cannot be drawn from that, and re-splitting a rendered string to
/// get the number back is how a count and a column come to disagree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listing {
    /// Stable key, and the key the favourite set stores. The launcher builds
    /// its own row key the same way (`shardlist::Shard::id`), and a row with
    /// no id falls back to its address so a star still has something to bind
    /// to.
    pub id: String,
    pub name: String,
    pub addr: String,
    pub map: Option<String>,
    pub players: Option<u32>,
    pub max_players: Option<u32>,
    pub ping_ms: Option<u32>,
    /// Where this row's live count is polled from, if anywhere.
    pub status_url: Option<String>,
    /// The address this binary was started with, rather than a shard anybody
    /// published.
    ///
    /// **Pinned first, and exempt from every filter except the search box.**
    /// A shard list that fails to load must never leave the player with
    /// nothing to click — the invariant the intro screen has carried since it
    /// existed — and the *category* and *slot* filters would hide this row for
    /// reasons that are artefacts of it not being a published shard: it is in
    /// nobody's favourites and it has no population, so "hide empty" alone
    /// would take it away and leave a fetch failure with no way out.
    ///
    /// The query is the deliberate exception. Typing is a player looking for
    /// something specific, and a row that survived every search would read as
    /// a broken search box rather than as a safety net — the net is still
    /// there, one keystroke away, and [`empty_reason`] is what says so.
    pub direct: bool,
}

impl Listing {
    /// The row that is always there: the address this binary was started
    /// with. See [`Listing::direct`] for why nothing may filter it away.
    pub fn direct(addr: &str) -> Self {
        Self {
            id: addr.to_string(),
            name: "Direct".into(),
            addr: addr.to_string(),
            map: None,
            players: None,
            max_players: None,
            ping_ms: None,
            status_url: None,
            direct: true,
        }
    }

    /// A row off the fetched `elo-shardlist-v1` document.
    ///
    /// The id falls back to the address, because the favourite set keys on it
    /// and a document that omitted one would otherwise give every such row
    /// the *same* empty key — one star lighting up several rows at once.
    pub fn from_shard(s: &crate::shardlist::Shard) -> Self {
        Self {
            id: if s.id.is_empty() {
                s.addr.clone()
            } else {
                s.id.clone()
            },
            name: s.name.clone(),
            addr: s.addr.clone(),
            map: s.map.clone(),
            players: s.players,
            max_players: s.max_players,
            ping_ms: s.ping_ms,
            status_url: s.status_url.clone(),
            direct: false,
        }
    }

    /// Fold a poll in. Returns whether anything a player can see moved, which
    /// is what decides a rebuild: respawning the screen every ten seconds to
    /// redraw an identical `3/100` is a visible flicker for nothing.
    ///
    /// The Direct row takes nothing — it is not a shard anybody published, so
    /// there is no document behind it naming a status endpoint and no count
    /// to invent.
    pub fn apply_status(&mut self, s: &crate::shardlist::Status) -> bool {
        if self.direct {
            return false;
        }
        let before = (self.players, self.max_players);
        self.players = Some(s.players);
        self.max_players = Some(s.max_players);
        before != (self.players, self.max_players)
    }

    /// `47/100`, `47/?`, or `?` — whatever the row actually states. Never
    /// fills a missing count with a zero, which is the honesty rule
    /// `shardlist::Shard::population` states at more length: a shard that did
    /// not answer keeps what it had rather than reading as "everyone left".
    pub fn population(&self) -> String {
        match (self.players, self.max_players) {
            (Some(p), Some(m)) => format!("{p}/{m}"),
            (Some(p), None) => format!("{p}"),
            (None, Some(m)) => format!("?/{m}"),
            (None, None) => "?".to_string(),
        }
    }

    /// The PING column. Same rule: `?`, never a plausible-looking number.
    pub fn ping(&self) -> String {
        match self.ping_ms {
            Some(p) => format!("{p}"),
            None => "?".to_string(),
        }
    }

    /// The dim second line under the name — the reference puts the map and
    /// the wipe age there. Ours has the map and the address, which are the
    /// two things a player picking between shards can act on.
    pub fn sub_line(&self) -> String {
        match &self.map {
            Some(m) => format!("{}   {m}", self.addr),
            None => self.addr.clone(),
        }
    }

    /// Is anybody on it? `None` is not empty — it is unknown, and the two
    /// must not collapse or the "hide empty" filter would hide every shard
    /// that failed a status poll.
    fn empty(&self) -> bool {
        self.players == Some(0)
    }

    /// Is it full? Unknown on either side is not full, for the same reason.
    fn full(&self) -> bool {
        match (self.players, self.max_players) {
            (Some(p), Some(m)) => m > 0 && p >= m,
            _ => false,
        }
    }

    /// Does this row match a typed query? Name, address and map — the three
    /// things printed on the row, so a search can never hide something on
    /// evidence the player cannot see.
    fn matches(&self, needle: &str) -> bool {
        if needle.is_empty() {
            return true;
        }
        let hay = |s: &str| s.to_lowercase().contains(needle);
        hay(&self.name) || hay(&self.addr) || self.map.as_deref().is_some_and(hay)
    }
}

/// The rail's categories — the two we can actually back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Cat {
    #[default]
    All,
    Favourites,
}

impl Cat {
    pub const ALL: [Cat; 2] = [Cat::All, Cat::Favourites];

    pub fn label(self) -> &'static str {
        match self {
            Cat::All => "All Shards",
            Cat::Favourites => "Favourited",
        }
    }

    /// The line under the name in the rail. The reference puts a live count
    /// there; ours says what the category *is* when the count would be the
    /// whole story anyway.
    pub fn blurb(self) -> &'static str {
        match self {
            Cat::All => "every shard the list names",
            Cat::Favourites => "the ones you starred",
        }
    }
}

/// What the player has narrowed the list to.
#[derive(Debug, Clone, Default)]
pub struct Filter {
    pub cat: Cat,
    pub query: String,
    /// The reference's PLAYER SLOTS group, as two toggles rather than two
    /// radios: theirs are "Show Full" and "Show Empty" — a filter that
    /// *includes* — and both default on, which makes the honest inversion a
    /// pair of hides that default off.
    pub hide_empty: bool,
    pub hide_full: bool,
}

impl Filter {
    /// Is anything narrowed at all? Drives whether CLEAR FILTERS is live —
    /// a button that does nothing when clicked is worse than a dim one.
    pub fn narrowed(&self) -> bool {
        self.cat != Cat::All || !self.query.is_empty() || self.hide_empty || self.hide_full
    }

    /// CLEAR FILTERS. Resets the category too, which is what the reference's
    /// button does: the rail is a filter and the player thinks of it as one.
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// Take a character from the keyboard, capped. Returns whether anything
    /// changed, so the caller only redraws when it did.
    pub fn push(&mut self, c: char) -> bool {
        if c.is_control() || self.query.chars().count() >= MAX_QUERY_CHARS {
            return false;
        }
        self.query.push(c);
        true
    }

    pub fn backspace(&mut self) -> bool {
        self.query.pop().is_some()
    }
}

/// The starred shard ids, in the order they were starred.
///
/// A `Vec` rather than a set, deliberately: wall 1's ban on `HashSet`
/// iteration is a sim-core rule and this is the client, but the *reason*
/// applies anyway — the file this is serialized to has to be byte-stable
/// between saves or every launch rewrites it in a new order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Favourites(Vec<String>);

impl Favourites {
    /// Build from whatever came off disk, dropping blanks and duplicates and
    /// truncating to the cap. A hand-edited file cannot reach a state the
    /// star cannot.
    pub fn from_disk(ids: impl IntoIterator<Item = String>) -> Self {
        let mut out = Self::default();
        for id in ids {
            out.add(id);
        }
        out
    }

    fn add(&mut self, id: String) {
        if id.is_empty() || self.0.len() >= MAX_FAVOURITES || self.0.contains(&id) {
            return;
        }
        self.0.push(id);
    }

    pub fn has(&self, id: &str) -> bool {
        self.0.iter().any(|k| k == id)
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn ids(&self) -> &[String] {
        &self.0
    }

    /// Star or unstar. Returns whether the set actually moved — the caller
    /// writes the file on a change and must not write on a click that hit
    /// the cap.
    pub fn toggle(&mut self, id: &str) -> bool {
        if let Some(i) = self.0.iter().position(|k| k == id) {
            self.0.remove(i);
            return true;
        }
        let before = self.0.len();
        self.add(id.to_string());
        self.0.len() != before
    }
}

/// How many rows each category holds right now, for the counts under the rail
/// names.
///
/// Counted over the whole list rather than over the filtered one: a rail that
/// reported post-filter counts would show `0` beside the category you would
/// have to click to see anything, which is the opposite of what a count on a
/// tab is for.
pub fn counts(rows: &[Listing], favs: &Favourites) -> [usize; 2] {
    let starred = rows.iter().filter(|r| favs.has(&r.id)).count();
    [rows.len(), starred]
}

/// Which rows survive the filter, in the order they are drawn.
///
/// **The order is the reference's**: population descending. Its list reads
/// 145, 110, 101, 54, 53, 51, … top to bottom, which is a busy-first sort,
/// and it is the right one — the question a player is answering on this
/// screen is "where is there a game happening". Ties break by name so the
/// order is stable between two polls that return the same numbers, and a
/// shard whose count is unknown sorts *after* every known one rather than as
/// zero, because "we could not ask" is not "nobody is there".
///
/// The Direct row is pinned to the top ahead of all of it and is exempt from
/// every filter — see [`Listing::direct`].
pub fn visible(rows: &[Listing], filter: &Filter, favs: &Favourites) -> Vec<usize> {
    let needle = filter.query.to_lowercase();
    let mut out: Vec<usize> = rows
        .iter()
        .enumerate()
        .filter(|(_, r)| {
            if !r.matches(&needle) {
                return false;
            }
            if r.direct {
                return true;
            }
            if filter.cat == Cat::Favourites && !favs.has(&r.id) {
                return false;
            }
            if filter.hide_empty && r.empty() {
                return false;
            }
            if filter.hide_full && r.full() {
                return false;
            }
            true
        })
        .map(|(i, _)| i)
        .collect();

    out.sort_by(|&a, &b| {
        let (x, y) = (&rows[a], &rows[b]);
        // `Option`'s own ordering puts `None` first, and this wants it last:
        // an unpolled shard belongs under the ones we know about, not above
        // a shard with a hundred people on it.
        let rank = |r: &Listing| r.players.map(|p| (0u8, std::cmp::Reverse(p)));
        y.direct
            .cmp(&x.direct)
            .then_with(|| match (rank(x), rank(y)) {
                (Some(a), Some(b)) => a.cmp(&b),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            })
            .then_with(|| x.name.cmp(&y.name))
    });
    out
}

/// What the screen says when the filter leaves nothing.
///
/// Always something, and it always names what would fill it — the rule the
/// intro screen has obeyed since it existed, applied one level in: an empty
/// list caused by a search box is a different problem from an empty list
/// caused by a fetch, and a player who cannot tell which is stuck.
///
/// **`shown` is a parameter rather than the caller's guard**, and that is a
/// fix a test forced: the first cut answered whenever the filter was narrowed
/// at all, which is a sentence about a list that is on screen in front of the
/// player. It read correctly only because the one caller happened to ask
/// inside an `if shown.is_empty()`. A function whose name is a claim should
/// be able to check the claim.
pub fn empty_reason(filter: &Filter, total: usize, shown: usize) -> Option<String> {
    if shown > 0 {
        return None;
    }
    if total == 0 {
        return None; // The fetch's own status line owns this one.
    }
    if !filter.narrowed() {
        return None;
    }
    Some(match (&filter.cat, filter.query.is_empty()) {
        (Cat::Favourites, true) => "no shard on this list is starred yet - \
                                    click a star to keep one here"
            .to_string(),
        (_, false) => format!("nothing matches \"{}\" - Esc clears it", filter.query),
        _ => "the filters leave nothing - CLEAR FILTERS puts them back".to_string(),
    })
}
