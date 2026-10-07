//! The arc's words and numbers (`ARC.md`): what the HUD banner, the ISLAND
//! page and a work's terminal say. Pure functions of `ClientCore::arc` and
//! the server-tick estimate, so `tests/ui.rs` holds them without a window.

use client_core::arc::{ArcView, WorkView};
use sim_core::limits::TICK_HZ;
use sim_core::works::{
    WORK_EV_EMBERS, WORK_EV_FALLBACK, WORK_EV_LIT, WORK_EV_OPENED, WORK_EV_REKINDLED, WORK_LIT,
    WORK_OPEN, WORK_SEALED,
};

/// Ticks an hour.
const HOUR: f64 = 3600.0 * TICK_HZ as f64;

/// The act's name (`ARC.md` §2).
pub fn act_title(act: u8) -> &'static str {
    match act {
        0 | 1 => "ACT I · RATS",
        2 => "ACT II · POWER",
        3 => "ACT III · THE GATE",
        _ => "ACT IV · DESCENT",
    }
}

/// Whole hours from `now` to `at`, rounded up, 0 once it has passed.
pub fn hours_until(at: u64, now: f64) -> u32 {
    let left = at as f64 - now;
    if left <= 0.0 {
        0
    } else {
        (left / HOUR).ceil() as u32
    }
}

/// "in 9 h", "in 3 d 4 h", or "now".
pub fn span(hours: u32) -> String {
    match hours {
        0 => "now".into(),
        h if h < 48 => format!("in {h} h"),
        h => format!("in {} d {} h", h / 24, h % 24),
    }
}

/// The tank, per cent of full.
pub fn tank_pct(w: &WorkView) -> u32 {
    if w.def.fuel_max == 0 {
        0
    } else {
        (w.fuel as u64 * 100 / w.def.fuel_max as u64) as u32
    }
}

/// One line under a work's name: where it stands and what comes next.
pub fn state_line(w: &WorkView, now: f64) -> String {
    match w.state {
        WORK_SEALED => format!("SEALED · opens {}", span(hours_until(w.def.opens_at, now))),
        WORK_OPEN => format!(
            "{}% · lights itself {}",
            w.progress_pm() / 10,
            span(hours_until(w.def.fallback_at, now))
        ),
        WORK_LIT if w.fuel > 0 => format!("BURNING · tank {}%", tank_pct(w)),
        WORK_LIT => "EMBERS · its tank is dry".into(),
        _ => String::new(),
    }
}

/// What the work's unlocks give, for everybody.
pub fn gives_line(w: &WorkView) -> String {
    let (f, c) = (w.floor_name(), w.ceiling_name());
    match (f.is_empty(), c.is_empty()) {
        (false, false) => format!("{f} for the wipe · {c} while it burns"),
        (false, true) => format!("{f} for the wipe"),
        (true, false) => format!("{c} while it burns"),
        (true, true) => String::new(),
    }
}

/// This player's share of a work, in words, or `None` before they gave.
pub fn share_line(w: &WorkView) -> Option<String> {
    (w.mine > 0).then(|| format!("YOUR SHARE {}.{}%", w.mine / 100, (w.mine % 100) / 10))
}

/// The HUD banner: the act, and the work that matters most right now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Banner {
    pub title: String,
    pub line: String,
    /// The bar under it, per mille; `None` draws no bar.
    pub pm: Option<u32>,
}

/// The work the banner follows: an open one with the most paid in, else the
/// next to open, else the lit one burning lowest.
pub fn focus(arc: &ArcView) -> Option<(usize, &WorkView)> {
    let open = arc
        .known()
        .filter(|(_, w)| w.state == WORK_OPEN)
        .max_by_key(|(_, w)| w.progress_pm());
    if open.is_some() {
        return open;
    }
    let sealed = arc
        .known()
        .filter(|(_, w)| w.state == WORK_SEALED)
        .min_by_key(|(_, w)| w.def.opens_at);
    if sealed.is_some() {
        return sealed;
    }
    arc.known()
        .filter(|(_, w)| w.state == WORK_LIT)
        .min_by_key(|(_, w)| tank_pct(w))
}

pub fn banner(arc: &ArcView, now: f64) -> Option<Banner> {
    let (_, w) = focus(arc)?;
    let pm = match w.state {
        WORK_OPEN => Some(w.progress_pm()),
        WORK_LIT if w.def.fuel_max > 0 => Some(tank_pct(w) * 10),
        _ => None,
    };
    Some(Banner {
        title: act_title(arc.act()).into(),
        line: format!("{} · {}", w.name(), state_line(w, now)),
        pm,
    })
}

/// The sentence a work changing deserves, to everybody. `who` is the
/// player's name when the server named one and it is known here.
pub fn event_line(arc: &ArcView, index: u8, what: u8, who: Option<&str>) -> Option<String> {
    let w = arc.works.get(index as usize).filter(|w| w.known)?;
    let name = w.name();
    let by = who
        .map(|n| format!(" — {n} put the last in"))
        .unwrap_or_default();
    Some(match what as u32 {
        WORK_EV_OPENED => format!("{name} IS OPEN — bring what it needs"),
        WORK_EV_LIT => format!("{name} BURNS{by}"),
        WORK_EV_FALLBACK => format!("{name} WOKE ON ITS OWN — its tank is empty"),
        WORK_EV_EMBERS => format!("{name} HAS GONE TO EMBERS"),
        WORK_EV_REKINDLED => format!("{name} IS FED AGAIN"),
        _ => return None,
    })
}

/// What a recipe or an offer waiting on unlock `u` says: which work must
/// burn, or a plain line before its row has arrived.
pub fn locked_line(arc: &ArcView, u: u8) -> Option<String> {
    if arc.holds(u) {
        return None;
    }
    Some(match arc.granter(u) {
        Some((_, w)) => format!("NEEDS {} TO BURN — [O] ISLAND", w.name()),
        None => "THE ISLAND CANNOT MAKE THIS YET — [O] ISLAND".into(),
    })
}
