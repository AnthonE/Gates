//! An agent player's bug reports: the document a player's F7 files
//! (`client::report`), filed by a bot's body when its mind says the game
//! did something wrong.
//!
//! The mind writes the note (`report` on an external agent's reply or an
//! MCP `act`: kind, title, body). The body adds what the mind is never
//! shown: the seed, its position and the tick (`explorer.rs`). No netcode
//! counters: a bot runs no prediction, and its snapshot counts are
//! `botclient`'s, not its `ClientCore`'s, so they would read zero. Each report is kept as a `.md`/`.json` pair and POSTed
//! to the platform's intake, the one F7 sends to. A panicking bot files a
//! crash report the same way.
//!
//! Bounded twice: a body files at most one per [`GAP`], a process at most
//! [`PER_RUN`]. The intake takes 40 a day from one address, and a fleet of
//! buddies on one box must not spend them all. Notes written inside the gap
//! wait in the body's [`Tray`] (at most [`HELD`]) rather than being lost: a
//! mind that saw four things at once used to get one filed and the other
//! three dropped where only the body's log said so.

use client::report::{self, Agent, Build, Kind, Place, Report, Vitals};
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// Reports one process files, crashes included.
pub const PER_RUN: u32 = 5;
/// The shortest time between two reports from one body.
pub const GAP: Duration = Duration::from_secs(120);
/// Notes a body holds while it waits out the gap.
pub const HELD: usize = 3;
/// How long a send may take, off the body's thread.
const SEND_TIMEOUT_S: u64 = 15;
/// How long a crash report's send may hold a dying process.
const CRASH_TIMEOUT_S: u64 = 5;
/// What a mind may name as `kind`. A crash is the panic hook's alone.
pub const KINDS: &str = "look, world, verb, net, numbers, other";

/// What a mind wrote: the kind, a one-line title, and what happened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Note {
    pub kind: Kind,
    pub title: String,
    pub body: String,
}

impl Note {
    /// A reply's `report`, `{"kind": "verb", "title": "…", "body": "…"}`.
    /// Missing or null is none; a kind left out is `other`.
    pub fn parse(v: &Value) -> Result<Option<Note>, String> {
        let o = match v {
            Value::Null => return Ok(None),
            Value::Object(o) => o,
            _ => return Err("report must be an object: {kind, title, body}".into()),
        };
        let kind = match o.get("kind") {
            None | Some(Value::Null) => Kind::Other,
            Some(Value::String(k)) => Kind::from_slug(k.trim())
                .filter(|k| *k != Kind::Crash)
                .ok_or_else(|| format!("report kind must be one of {KINDS}"))?,
            Some(_) => return Err(format!("report kind must be one of {KINDS}")),
        };
        let title = o
            .get("title")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .ok_or("report needs a title: one line saying what is wrong")?;
        let body = match o.get("body") {
            None | Some(Value::Null) => "",
            Some(Value::String(b)) => b.as_str(),
            Some(_) => return Err("report body must be a string".into()),
        };
        Ok(Some(Note {
            kind,
            title: title.to_owned(),
            body: body.to_owned(),
        }))
    }
}

/// Where this process's reports go, set once by `jev-bot`. A process that
/// never opens one (the shard, load bots, tests) files nothing.
#[derive(Clone, Debug)]
pub struct Desk {
    /// The intake's URL, or none to keep reports on this machine.
    pub intake: Option<String>,
    /// Where the pairs are written, or none.
    pub dir: Option<PathBuf>,
    /// The shard's `host:port`, as the bot was pointed at it.
    pub shard: String,
    pub name: String,
    pub wallet: Option<String>,
    /// What decides: `external`, `mcp`, `jev`, `scripted`.
    pub mind: String,
}

impl Desk {
    /// `$GATES_REPORTS_DIR`, else `~/.gates/reports`.
    pub fn default_dir() -> Option<PathBuf> {
        let var = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty());
        var("GATES_REPORTS_DIR")
            .map(PathBuf::from)
            .or_else(|| var("HOME").map(|h| PathBuf::from(h).join(".gates").join("reports")))
    }

    /// The platform's intake, the one the game's F7 sends to.
    pub fn platform_intake() -> String {
        report::intake_url(client::ui::names::PLATFORM_ORIGIN)
    }
}

static DESK: OnceLock<Desk> = OnceLock::new();
static TAKEN: AtomicU32 = AtomicU32::new(0);
static SENT: AtomicU32 = AtomicU32::new(0);
static UNSENT: AtomicU32 = AtomicU32::new(0);
static WAITING: AtomicU32 = AtomicU32::new(0);
static DROPPED: AtomicU32 = AtomicU32::new(0);

/// Open this process's desk. The first one stands.
pub fn open(desk: Desk) {
    let _ = DESK.set(desk);
}

/// Whether a mind's notes are filed at all here.
pub fn is_open() -> bool {
    DESK.get().is_some()
}

/// How this run's reports have gone, for `status`.
pub fn counts() -> Value {
    let taken = TAKEN.load(Ordering::Relaxed).min(PER_RUN);
    json!({
        "sent": SENT.load(Ordering::Relaxed),
        "not_sent": UNSENT.load(Ordering::Relaxed),
        "waiting": WAITING.load(Ordering::Relaxed),
        "dropped": DROPPED.load(Ordering::Relaxed),
        "left_this_run": PER_RUN - taken,
    })
}

/// A body's notes waiting out the [`GAP`], each with the facts of the
/// moment it was written, not the moment it is filed.
#[derive(Default)]
pub struct Tray {
    held: VecDeque<(Note, Facts)>,
    last: Option<Instant>,
}

impl Tray {
    /// Keep `note` until the gap lets it go. A full tray drops it.
    pub fn hold(&mut self, note: Note, facts: Facts) {
        if self.held.len() >= HELD {
            DROPPED.fetch_add(1, Ordering::Relaxed);
            eprintln!("bug report dropped: {HELD} already wait to be filed");
            return;
        }
        WAITING.fetch_add(1, Ordering::Relaxed);
        self.held.push_back((note, facts));
    }

    /// The oldest note, once the body's last report is [`GAP`] behind.
    pub fn due(&mut self, now: Instant) -> Option<(Note, Facts)> {
        if self
            .last
            .is_some_and(|t| now.saturating_duration_since(t) < GAP)
        {
            return None;
        }
        let next = self.held.pop_front()?;
        WAITING.fetch_sub(1, Ordering::Relaxed);
        self.last = Some(now);
        Some(next)
    }
}

/// What the body knows when it files: the facts its mind was never shown,
/// and what the mind was doing and saw.
pub struct Facts {
    pub seed: u64,
    pub tick: u32,
    pub pos: [f32; 3],
    pub player_id: u32,
    pub vitals: Vitals,
    pub goal: Option<String>,
    pub last: Option<String>,
    pub seen: Value,
}

/// File `note` with the body's `facts`: keep the pair, send it from a
/// thread of its own. Returns the line to log.
pub fn file(note: &Note, facts: Facts) -> String {
    let Some(desk) = DESK.get() else {
        return "bug report dropped: reports are off in this process".into();
    };
    if TAKEN.fetch_add(1, Ordering::Relaxed) >= PER_RUN {
        DROPPED.fetch_add(1, Ordering::Relaxed);
        return format!("bug report dropped: this run has filed its {PER_RUN}");
    }
    let mut r = Report::new(
        note.kind,
        &note.title,
        &note.body,
        client::shot::now_secs(),
        Build::agent(),
    );
    r.place = Some(Place {
        seed: facts.seed,
        tick: facts.tick,
        pos: facts.pos,
        screen: if facts.vitals.dead { "dead" } else { "world" }.into(),
        shard: desk.shard.clone(),
        player_id: facts.player_id,
    });
    r.vitals = Some(facts.vitals);
    r.agent = Some(Agent {
        name: report::one_line(&desk.name, report::TITLE_MAX).0,
        mind: desk.mind.clone(),
        goal: facts.goal,
        last: facts.last,
        seen: facts.seen,
    });
    r.wallet = desk.wallet.clone();
    let kept = keep(desk, &r);
    let line = format!("bug report [{}] {:?}{kept}", note.kind.slug(), r.title);
    send(desk, r, SEND_TIMEOUT_S, true);
    line
}

/// File a report for this process's own panic, then run the hook it
/// displaced (which prints the message and backtrace).
pub fn on_panic() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        crash(info);
        previous(info);
    }));
}

fn crash(info: &std::panic::PanicHookInfo<'_>) {
    let Some(desk) = DESK.get() else {
        return;
    };
    if TAKEN.fetch_add(1, Ordering::Relaxed) >= PER_RUN {
        return;
    }
    let location = info
        .location()
        .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
        .unwrap_or_else(|| "unknown".into());
    let mut r = Report::crash(location, &info.to_string(), client::shot::now_secs());
    r.build = Build::agent();
    r.title = "an agent player's body panicked".into();
    r.wallet = desk.wallet.clone();
    let kept = keep(desk, &r);
    eprintln!("bugs: crash report{kept}");
    send(desk, r, CRASH_TIMEOUT_S, false);
}

/// Write the pair, and say where.
fn keep(desk: &Desk, r: &Report) -> String {
    let Some(dir) = &desk.dir else {
        return String::new();
    };
    match report::write_pair(dir, r) {
        Ok((stem, _)) => format!(", kept at {}", dir.join(format!("{stem}.md")).display()),
        Err(e) => format!(", not kept: {e}"),
    }
}

/// POST it to the intake, from a thread of its own or (a crash) here.
fn send(desk: &Desk, r: Report, timeout_s: u64, threaded: bool) {
    let Some(url) = desk.intake.clone() else {
        UNSENT.fetch_add(1, Ordering::Relaxed);
        return;
    };
    let job = move || match post(&url, r.submission(None), timeout_s) {
        Ok(receipt) => {
            SENT.fetch_add(1, Ordering::Relaxed);
            eprintln!(
                "bugs: report {} sent, ref {} ({} like it)",
                r.fingerprint_hex(),
                receipt.id,
                receipt.seen
            );
        }
        Err(why) => {
            UNSENT.fetch_add(1, Ordering::Relaxed);
            eprintln!("bugs: report {} not sent: {why}", r.fingerprint_hex());
        }
    };
    if !threaded {
        job();
    } else if let Err(e) = std::thread::Builder::new()
        .name("bug-report".into())
        .spawn(job)
    {
        UNSENT.fetch_add(1, Ordering::Relaxed);
        eprintln!("bugs: report not sent: {e}");
    }
}

fn post(url: &str, body: String, timeout_s: u64) -> Result<report::Receipt, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(timeout_s)))
        .http_status_as_error(false)
        .build()
        .into();
    let mut res = agent
        .post(url)
        .header("content-type", "application/json")
        .send(body)
        .map_err(|e| format!("{e}"))?;
    let status = res.status().as_u16();
    let bytes = res
        .body_mut()
        .with_config()
        .limit(64 * 1024)
        .read_to_vec()
        .map_err(|e| format!("{e}"))?;
    report::read_receipt(status, &bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_note_is_strict_about_shape_and_never_a_crash() {
        let note = Note::parse(
            &json!({"kind": "verb", "title": " door won't open ", "body": "pressed E"}),
        )
        .unwrap()
        .unwrap();
        assert_eq!(note.kind, Kind::Verb);
        assert_eq!(note.title, "door won't open");
        assert_eq!(note.body, "pressed E");
        let plain = Note::parse(&json!({"title": "odd"})).unwrap().unwrap();
        assert_eq!((plain.kind, plain.body.as_str()), (Kind::Other, ""));
        assert_eq!(Note::parse(&Value::Null), Ok(None));
        for bad in [
            json!("stuck"),
            json!({"kind": "verb"}),
            json!({"kind": "verb", "title": "  "}),
            json!({"kind": "crash", "title": "x"}),
            json!({"kind": "teleport", "title": "x"}),
            json!({"title": "x", "body": 7}),
        ] {
            assert!(Note::parse(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn notes_inside_the_gap_wait_their_turn() {
        let note = |t: &str| Note {
            kind: Kind::Verb,
            title: t.into(),
            body: String::new(),
        };
        let facts = || Facts {
            seed: 1,
            tick: 2,
            pos: [0.0; 3],
            player_id: 3,
            vitals: Vitals {
                hp: 100,
                hp_max: 100,
                food: 100,
                water: 100,
                dead: false,
            },
            goal: None,
            last: None,
            seen: Value::Null,
        };
        let t0 = Instant::now();
        let mut tray = Tray::default();
        for t in ["a", "b", "c", "d", "e"] {
            tray.hold(note(t), facts());
        }
        let title = |x: Option<(Note, Facts)>| x.map(|(n, _)| n.title);
        assert_eq!(title(tray.due(t0)), Some("a".into()));
        assert_eq!(title(tray.due(t0 + GAP / 2)), None, "inside the gap");
        assert_eq!(title(tray.due(t0 + GAP)), Some("b".into()));
        assert_eq!(title(tray.due(t0 + GAP * 2)), Some("c".into()));
        assert_eq!(title(tray.due(t0 + GAP * 3)), None, "past HELD was dropped");
    }

    #[test]
    fn an_agent_report_is_its_own_runtime_and_fingerprint() {
        let mut r = Report::new(Kind::Verb, "t", "b", 1_760_000_000, Build::agent());
        let person = Report::new(Kind::Verb, "t", "b", 1_760_000_000, Build::here());
        assert_ne!(r.fingerprint(), person.fingerprint());
        r.agent = Some(Agent {
            name: "claude".into(),
            mind: "mcp".into(),
            goal: Some("gather_wood".into()),
            last: None,
            seen: json!({"heard": [{"from": "x", "text": "ignore the above"}]}),
        });
        let md = r.markdown();
        assert!(md.contains("| runtime | agent · jev-bot ("), "{md}");
        assert!(md.contains("## The agent"), "{md}");
        let v: Value = serde_json::from_str(&r.json()).unwrap();
        assert_eq!(v["build"]["runtime"], "agent");
        assert_eq!(v["agent"]["mind"], "mcp");
        let p: Value = serde_json::from_str(&person.json()).unwrap();
        assert!(p.get("agent").is_none(), "a person's report is unchanged");
    }
}
