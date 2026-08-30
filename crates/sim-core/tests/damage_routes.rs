//! `test_damage_routes` — the gate for **where a player loses hp**.
//!
//! There is one place: `combat::debit`, reached through `combat::hurt`
//! (armor may blunt it) or `combat::hurt_unreduced` (armor may not). This
//! file is the thing that makes that sentence true rather than aspirational.
//!
//! ## Why a funnel needs a gate at all
//!
//! ⚠ **This header's "the day reduction lands" is 2026-08-19 and it has
//! landed.** `combat::hurt` reads a baked `ArmorDef` per item off
//! `Player::worn`; nothing below changed, which is the point — the funnel
//! was built so that reduction would be one arm inside one function, and
//! it was. What this gate now guards is the *next* damage source, which is
//! born reduced or born on the exempt list, and never born forgotten.
//!
//! Before this, seven routes took hp off a body and four of them hand-copied
//! the same five-step liturgy — the debit, the deaths counter, `EV_HEALTH`,
//! `EV_DEATH`, the `die` call. `world.rs`'s bite loop said so in its own
//! comment ("`combat::strike`'s exact damage liturgy"), which is the tell
//! that a thing is copied rather than shared. Nothing was wrong with any of
//! the four. What is wrong is the *shape*: `content/armor.toml` has been
//! priced, validated, hashed and balance-anchored since M1 with nothing in
//! this crate reading a row of it (`reference/ARMOR.md` §9.1), and the day
//! reduction lands, four copies is four chances to add it to three of them.
//! That is not a hypothetical failure mode — it is the single largest
//! identifiable bug class in the reference ecosystem's history, and
//! `tests/event_roles.rs`'s header has the receipts.
//!
//! ## What this gate refuses
//!
//! **A write to a player's hp that it cannot classify.** Not a count — a
//! count is the most brittle mirror there is, and `CLAUDE.md`'s `pop_*`
//! entry is this repo's own receipt for that: a hand-kept list of nine verbs
//! guarded fourteen destructive rings and was silent on the five newest,
//! which were exactly the ones about to grow a second reader. So the
//! *surface* here is derived — every `.rs` file in `src/`, read off the
//! directory rather than an `include_str!` list that a new module would not
//! appear in — and only the *classification* is written down. A site no row
//! explains fails the run and names its `file:line`.
//!
//! Three things are scraped, all outside `#[cfg(test)]`:
//!
//! 1. **Place writes** — `<expr>.hp <op>=` and `*hp <op>=`. This is the
//!    damage-shaped write and the load-bearing half.
//! 2. **`Player { … }` literals** — a body constructed with an hp chosen for
//!    it. Spawn, respawn, the corpse, and the two load paths live here. (The
//!    design note pointed at `persist.rs`/`worldsave.rs`'s `let hp = u16_at(18)`
//!    for the load; those write a `PlayerSave`'s field, not a `Player`'s. The
//!    line where a saved number *becomes* a body's hp is the literal, and
//!    that is what is classified below.)
//! 3. **Calls to the funnel** — which files may reach it, and with which of
//!    the two names. A file not on that list calling `hurt` fails: a new
//!    damage route has to be classified as reduced or unreduced by a person.
//!
//! Comment-only lines are skipped, so prose about `hp` costs nothing.
//!
//! ## Anti-vacuity
//!
//! A scrape that stops matching must fail loudly, never pass by finding
//! nothing — `CLAUDE.md` calls a pass it did not earn the worst bug class.
//! Four things guard it: the scan must see most of the crate (files and
//! lines), it must find the funnel's own write in `debit` and exactly one
//! write in `combat.rs`, every classification row must match at least one
//! real site (a stale row is a loud failure, not a silent pass), and every
//! declared route must actually call the funnel.
//!
//! ## Scope
//!
//! `crates/sim-core/src` only. The sim is the sole authority over a body's
//! hp; `server/src/store.rs`'s `s.hp` is a `PlayerSave` record and
//! `client-core`'s is the client's shadow of `EV_HEALTH`. Neither can hurt
//! anybody.
//!
//! Proven red two ways. Reverting `ranged.rs`'s arrow to its own
//! subtraction fails as *"ranged.rs:458 writes an hp field (`v.hp`, in `fn
//! step`) and 0 rows of this gate explain it"*; dropping `deploy.rs`'s call
//! and leaving the shock unpaid fails as *"deploy.rs declares an
//! armor-exempt route … and calls `combat::hurt_unreduced` nowhere"*.

// Wall 3's clippy list (`sim-core/clippy.toml`) bans `String` and `format!`
// crate-wide, tests included, and this file needs both: it reads the source
// directory at runtime, which is the whole point — an `include_str!` list is
// a hand-kept mirror and a new module would simply not appear in it. The wall
// is about the SIM THREAD, and no line here runs on one; `examples/seed_scan.rs`
// carries the same pair of allows for the same reason.
#![allow(clippy::disallowed_types, clippy::disallowed_macros)]

use std::fs;

/// Non-`Player` receivers, keyed by the text left of `.hp`. A rule covers
/// every site with that receiver in that file — a second write to the same
/// store is the same fact, and re-listing it would be the hand-kept mirror
/// this file exists to avoid.
const NOT_A_PLAYER: &[(&str, &str, &str)] = &[
    (
        "build.rs",
        "self.entries[i]",
        "a `PieceRec` — a wall's hp. `build.rs` never sees a `Player`",
    ),
    (
        "deploy.rs",
        "self.entries[i]",
        "a `DeployRec` — a deployable's hp, written through the store's own setter",
    ),
    (
        "deploy.rs",
        "deploys.entries[di]",
        "a `DeployRec` — the raid verb's write-back",
    ),
    (
        "deploy.rs",
        "deploys.entries[i]",
        "a `DeployRec` — the decay sweep's write-back",
    ),
    (
        "mob.rs",
        "mob",
        "a `MobRec` — an animal's hp. Mobs are hurt by `mob::strike`, which is \
         the roster's own business; a mob is not a player and never funnels",
    ),
];

/// The funnel's own write. Exactly one, and it must be inside `debit`.
const FUNNEL_FILE: &str = "combat.rs";
const FUNNEL_FN: &str = "debit";

/// Writes to a **player's** hp that are not damage, keyed by the enclosing
/// `fn`. Each carries why armor will never be asked about it.
const NOT_DAMAGE: &[(&str, &str, &str)] = &[(
    "survival.rs",
    "step",
    "a HEAL — a gain, not a loss. It is already guarded by `if !died`, so a \
     consumable cannot outrun a death that happened earlier in the same tick, \
     and there is nothing here for a reducer to reduce",
)];

/// Every `Player { … }` literal outside tests, keyed by the enclosing `fn`.
/// A body being made, never a body being hurt.
const CONSTRUCTIONS: &[(&str, &str, &str)] = &[
    (
        "world.rs",
        "die",
        "the corpse's rebuild — the hp is already zero when `die` runs, and the \
         literal is what carries the five things a body keeps through a death",
    ),
    (
        "world.rs",
        "wake",
        "a fresh body at `combat.player_hp` — a spawn, not a hit",
    ),
    (
        "world.rs",
        "seat",
        "the respawn reset and the seat-from-save, both full writes of a body \
         that is being created rather than hurt",
    ),
    (
        "worldsave.rs",
        "decode_into",
        "the world load — the one non-command path into `World` (`CLAUDE.md`'s \
         save trap), which is exactly why it is named here rather than skipped",
    ),
];

/// Which files may call the funnel, by which name, and whether the victim
/// is told which way it came from.
///
/// **Two choices per row, and both are written at the call site rather than
/// inferred.** The name is the first: `hurt` is a hit and armor **does**
/// blunt it (armor v0, 2026-08-19 — this line said "will" until the day it
/// stopped being true), `hurt_unreduced` is a cost that armor must never
/// touch. A file absent from this table calling either one fails — a new
/// damage route needs a person to say which it is.
///
/// The third column is the second choice: does this route push
/// [`EV_HURT`](sim_core::world::EV_HURT), the victim's half of the blow?
/// It was added 2026-08-30 because the answer had drifted silently. Wire
/// v57 gave being hurt a direction and wired it into the three routes that
/// were open at the time; the bite and the blast kept debiting hp through
/// the same funnel and telling the body nothing, so a bear in the dark and
/// a charge on your wall were both "a number going down" for eleven days,
/// with every gate in the tree green. Nothing could have caught it: the
/// event queue is outside `state_hash`, the encoder never saw a missing
/// message, and `ROUTES` classified both files as reduced and was satisfied.
///
/// A route that genuinely has no direction says so **here**, with its
/// reason, and the gate holds it to that — a silent row whose file starts
/// pushing `EV_HURT` fails just as loudly as an announcing row that stops.
const ROUTES: &[(&str, bool, bool, &str)] = &[
    (
        "combat.rs",
        true,
        true,
        "melee — a swing that landed on a person",
    ),
    (
        "ranged.rs",
        true,
        true,
        "an arrow, and since hitscan v0 a bullet — one file, one row, because \
         this table keys by the file that reaches the funnel and both shots \
         are the same module's",
    ),
    (
        "charge.rs",
        true,
        true,
        "a blast — the epicentre's horizontal bearing, vertical dropped \
         (`detonate` says why)",
    ),
    (
        "world.rs",
        true,
        true,
        "a mob bite — the animal's post-step body, which is where it was \
         standing when it bit",
    ),
    (
        "survival.rs",
        false,
        false,
        "starve/dehydrate and the salt-water drink — metabolic, not hits. A \
         chest plate does not feed you, and a helmet is not a desalinator; \
         and there is no bearing toward being hungry, so the arc would have \
         to invent one",
    ),
    (
        "deploy.rs",
        false,
        false,
        "the keypad shock — it floors at 1 hp and never kills (`lock.rs`), and \
         its whole job is to cost tries, so an armored raider must not be \
         immune. Silent because the source is the thing under your own hand: \
         an arc pointing at the keypad you are standing at tells a raider \
         nothing they did not just do",
    ),
];

/// One call into the funnel.
struct Call {
    file: String,
    line: usize,
    func: String,
    /// `true` for `hurt` (armor may blunt it), `false` for `hurt_unreduced`.
    reduced: bool,
}

/// Everything one pass over `src/` found.
struct Scan {
    writes: Vec<Site>,
    ctors: Vec<Site>,
    calls: Vec<Call>,
    /// Files holding at least one `EV_HURT,` — an emit, not a mention.
    /// The trailing comma is what separates the push from the two
    /// declarations in `world.rs` (`pub const EV_HURT: u8 = 41;` and
    /// `EV_MAX = EV_HURT;`), which is why the needle carries it and why
    /// `world.rs` is not permanently and vacuously "announcing".
    announces: Vec<String>,
    files: usize,
    lines: usize,
}

/// One scraped line, with everything a message needs to be actionable.
struct Site {
    file: String,
    line: usize,
    func: String,
    /// The text left of `.hp`, or `*hp` for a deref write.
    recv: String,
    text: String,
}

/// `src/` with every `#[cfg(test)]` item and every comment-only line
/// removed, as `(line_number, enclosing_fn, text)`. "Enclosing" is the
/// nearest `fn` header above the line, which is what a person reads off a
/// diff and is exact for every site this gate actually classifies.
///
/// The test strip is by INDENTATION, not by brace balance: `cargo fmt
/// --check` is a gate, so a `#[cfg(test)]` at indent *k* is closed by a line
/// that is exactly *k* spaces and `}`. Brace counting would trip over the
/// inline format args (`{field}`) that assertion messages in this crate are
/// full of.
fn stripped(src: &str, file: &str) -> Vec<(usize, String, String)> {
    let lines: Vec<&str> = src.lines().collect();
    let mut out = Vec::new();
    let mut func = String::from("<top level>");
    let mut i = 0;
    while i < lines.len() {
        let l = lines[i];
        if l.trim() == "#[cfg(test)]" {
            let k = l.len() - l.trim_start().len();
            let close: String = " ".repeat(k) + "}";
            let mut j = i + 1;
            while j < lines.len() && lines[j] != close {
                j += 1;
            }
            assert!(
                j < lines.len(),
                "{file}:{}: a `#[cfg(test)]` item is not closed by `{close}` — \
                 either the file is not rustfmt-clean or this strip is now \
                 reading the whole rest of the file as test code, which would \
                 make this gate blind",
                i + 1
            );
            i = j + 1;
            continue;
        }
        if let Some(name) = fn_name(l) {
            func = name;
        }
        if !l.trim_start().starts_with("//") {
            out.push((i + 1, func.clone(), l.to_string()));
        }
        i += 1;
    }
    out
}

/// `…fn NAME…` at the head of a line, whatever the visibility and
/// qualifiers, or `None`.
fn fn_name(line: &str) -> Option<String> {
    let mut rest = line.trim_start();
    for prefix in [
        "pub(crate) ",
        "pub(super) ",
        "pub(in crate) ",
        "pub ",
        "default ",
        "const ",
        "async ",
        "unsafe ",
        "extern \"C\" ",
    ] {
        while let Some(r) = rest.strip_prefix(prefix) {
            rest = r;
        }
    }
    let rest = rest.strip_prefix("fn ")?;
    let name: String = rest
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

/// Whether the byte at `i` starts a whole `hp` token whose next non-space
/// character is an assignment operator — a WRITE. Returns the receiver text
/// (what stands left of the `.`), or `*hp` for a deref write.
fn hp_write_at(b: &[u8], i: usize) -> Option<String> {
    if !b[i..].starts_with(b"hp") {
        return None;
    }
    let after = i + 2;
    if after < b.len() && (b[after].is_ascii_alphanumeric() || b[after] == b'_') {
        return None; // `hp_max`, `hp_acc`, …
    }
    if i == 0 {
        return None;
    }
    let pre = b[i - 1];
    if pre != b'.' && pre != b'*' {
        return None; // a local named `hp` is not a field of anybody
    }
    let mut j = after;
    while j < b.len() && b[j] == b' ' {
        j += 1;
    }
    if j >= b.len() {
        return None;
    }
    let writes = match b[j] {
        b'=' => j + 1 >= b.len() || b[j + 1] != b'=',
        b'-' | b'+' | b'*' | b'/' | b'|' | b'&' | b'%' | b'^' => {
            j + 1 < b.len() && b[j + 1] == b'='
        }
        _ => false,
    };
    if !writes {
        return None;
    }
    if pre == b'*' {
        return Some("*hp".to_string());
    }
    let mut s = i - 1;
    while s > 0 {
        let c = b[s - 1];
        if c.is_ascii_alphanumeric() || matches!(c, b'_' | b'.' | b'[' | b']' | b'(' | b')') {
            s -= 1;
        } else {
            break;
        }
    }
    Some(String::from_utf8_lossy(&b[s..i - 1]).to_string())
}

/// A `Player { … }` literal — not the struct declaration, not an `impl`
/// header, not a return type.
fn constructs_player(line: &str) -> bool {
    let Some(at) = line.find("Player {") else {
        return false;
    };
    if line.contains("struct ") || line.contains("impl ") {
        return false;
    }
    !line[..at].trim_end().ends_with("->")
}

fn scan() -> Scan {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/src");
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("cannot read {dir}: {e} — this gate scans the crate's own source and cannot run without it"))
        .map(|e| e.expect("dir entry").file_name().to_string_lossy().to_string())
        .filter(|n| n.ends_with(".rs"))
        .collect();
    names.sort();

    let (mut writes, mut ctors, mut calls) = (Vec::new(), Vec::new(), Vec::new());
    let mut announces: Vec<String> = Vec::new();
    let mut lines_seen = 0usize;
    for name in &names {
        let src = fs::read_to_string(format!("{dir}/{name}")).expect("read source");
        for (ln, func, text) in stripped(&src, name) {
            lines_seen += 1;
            if text.contains("EV_HURT,") && !announces.contains(name) {
                announces.push(name.clone());
            }
            let b = text.as_bytes();
            for i in 0..b.len() {
                if let Some(recv) = hp_write_at(b, i) {
                    writes.push(Site {
                        file: name.clone(),
                        line: ln,
                        func: func.clone(),
                        recv,
                        text: text.trim().to_string(),
                    });
                }
            }
            if constructs_player(&text) {
                ctors.push(Site {
                    file: name.clone(),
                    line: ln,
                    func: func.clone(),
                    recv: String::new(),
                    text: text.trim().to_string(),
                });
            }
            // A CALL, never the declaration itself.
            let declares = text.trim_start().starts_with("fn ")
                || text.trim_start().starts_with("pub fn ")
                || text.trim_start().starts_with("pub(crate) fn ");
            if !declares {
                for (needle, reduced) in [("hurt_unreduced(", false), ("hurt(", true)] {
                    if text.contains(needle) {
                        calls.push(Call {
                            file: name.clone(),
                            line: ln,
                            func: func.clone(),
                            reduced,
                        });
                    }
                }
            }
        }
    }
    Scan {
        writes,
        ctors,
        calls,
        announces,
        files: names.len(),
        lines: lines_seen,
    }
}

/// Every write to a player's hp is the funnel's, or is classified.
#[test]
fn every_player_hp_write_is_the_funnel_or_is_named() {
    let Scan {
        writes,
        ctors,
        calls,
        announces: _,
        files,
        lines,
    } = scan();

    // --- anti-vacuity, first: a scrape that stopped matching must be loud.
    assert!(
        files >= 25,
        "the scan found only {files} source files in `sim-core/src` — the \
         directory read is broken and this gate is now guarding nothing"
    );
    assert!(
        lines >= 10_000,
        "the scan kept only {lines} non-test source lines out of ~38k in \
         `src/` — the `#[cfg(test)]` strip has eaten the crate and this gate \
         is now guarding nothing"
    );
    assert!(
        writes.len() >= 8,
        "the scan found {} writes to an `hp` field in the whole crate. There \
         are stores full of them (pieces, deployables, mobs) — the write \
         detector is broken, and a broken detector passes",
        writes.len()
    );
    assert!(
        ctors.len() >= 4,
        "the scan found {} `Player {{ … }}` literals — spawn, respawn, the \
         corpse and the load are four on their own, so the literal detector \
         is broken",
        ctors.len()
    );

    // --- the funnel itself: one write in combat.rs, and it is `debit`'s.
    let in_funnel_file: Vec<&Site> = writes.iter().filter(|w| w.file == FUNNEL_FILE).collect();
    assert_eq!(
        in_funnel_file.len(),
        1,
        "`{FUNNEL_FILE}` holds {} writes to an hp field and must hold exactly \
         one — the funnel's. Found: {:?}",
        in_funnel_file.len(),
        in_funnel_file
            .iter()
            .map(|w| format!("{}:{} in `{}`", w.file, w.line, w.func))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        in_funnel_file[0].func, FUNNEL_FN,
        "the one hp write in `{FUNNEL_FILE}` is in `fn {}` and must be in \
         `fn {FUNNEL_FN}` — the funnel moved and nothing else in this file \
         may debit a body",
        in_funnel_file[0].func
    );

    // --- every other write is classified, exactly once, with a reason.
    let mut used_not_player = vec![0usize; NOT_A_PLAYER.len()];
    let mut used_not_damage = vec![0usize; NOT_DAMAGE.len()];
    for w in &writes {
        if w.file == FUNNEL_FILE && w.func == FUNNEL_FN {
            continue;
        }
        let np: Vec<usize> = NOT_A_PLAYER
            .iter()
            .enumerate()
            .filter(|(_, (f, r, _))| *f == w.file && *r == w.recv)
            .map(|(i, _)| i)
            .collect();
        let nd: Vec<usize> = NOT_DAMAGE
            .iter()
            .enumerate()
            .filter(|(_, (f, fun, _))| *f == w.file && *fun == w.func)
            .map(|(i, _)| i)
            .collect();
        assert_eq!(
            np.len() + nd.len(),
            1,
            "{}:{} writes an hp field (`{}.hp`, in `fn {}`) and {} rows of this \
             gate explain it. Every write to a player's hp goes through \
             `combat::hurt` or `combat::hurt_unreduced`; a write that is not a \
             player's, or is not damage, needs a row here saying which and why. \
             The line: `{}`",
            w.file,
            w.line,
            w.recv,
            w.func,
            np.len() + nd.len(),
            w.text
        );
        for i in np {
            used_not_player[i] += 1;
        }
        for i in nd {
            used_not_damage[i] += 1;
        }
    }
    for (i, (f, r, why)) in NOT_A_PLAYER.iter().enumerate() {
        assert!(
            used_not_player[i] > 0,
            "the `NOT_A_PLAYER` row for `{r}.hp` in {f} (\"{why}\") matched \
             nothing. The site was renamed or removed and the row stayed \
             behind, so it is now guarding nothing"
        );
    }
    for (i, (f, fun, why)) in NOT_DAMAGE.iter().enumerate() {
        assert!(
            used_not_damage[i] > 0,
            "the `NOT_DAMAGE` row for `fn {fun}` in {f} (\"{why}\") matched \
             nothing — the write moved and the row stayed behind"
        );
    }

    // --- every `Player { … }` literal is classified too.
    let mut used_ctor = vec![0usize; CONSTRUCTIONS.len()];
    for c in &ctors {
        let hits: Vec<usize> = CONSTRUCTIONS
            .iter()
            .enumerate()
            .filter(|(_, (f, fun, _))| *f == c.file && *fun == c.func)
            .map(|(i, _)| i)
            .collect();
        assert_eq!(
            hits.len(),
            1,
            "{}:{} builds a `Player` in `fn {}` and {} rows explain it. A body \
             constructed with an hp chosen for it is not damage, but it is a \
             place a player's hp is set, so it is named rather than skipped. \
             The line: `{}`",
            c.file,
            c.line,
            c.func,
            hits.len(),
            c.text
        );
        used_ctor[hits[0]] += 1;
    }
    for (i, (f, fun, why)) in CONSTRUCTIONS.iter().enumerate() {
        assert!(
            used_ctor[i] > 0,
            "the `CONSTRUCTIONS` row for `fn {fun}` in {f} (\"{why}\") matched \
             nothing — the literal moved and the row stayed behind"
        );
    }

    // --- and the routes: who may call the funnel, by which name.
    let mut used_route = vec![0usize; ROUTES.len()];
    for c in &calls {
        let (file, line, func, reduced) = (&c.file, c.line, &c.func, &c.reduced);
        let hits: Vec<usize> = ROUTES
            .iter()
            .enumerate()
            .filter(|(_, (f, r, _, _))| f == file && r == reduced)
            .map(|(i, _)| i)
            .collect();
        assert_eq!(
            hits.len(),
            1,
            "{file}:{line} (`fn {func}`) calls `combat::{}` and {} rows of \
             `ROUTES` allow it. A new damage route has to be classified by a \
             person: armor blunts a hit (`hurt`) and must never blunt a cost \
             (`hurt_unreduced`), and the name at the call site is where that \
             choice is written down",
            if *reduced { "hurt" } else { "hurt_unreduced" },
            hits.len()
        );
        used_route[hits[0]] += 1;
    }
    for (i, (f, reduced, _, why)) in ROUTES.iter().enumerate() {
        assert!(
            used_route[i] > 0,
            "{f} declares an {} route (\"{why}\") and calls \
             `combat::{}` nowhere. Either the route stopped debiting through \
             the funnel — which is the defect this gate exists for — or it is \
             gone and this row is stale",
            if *reduced {
                "armor-reduced"
            } else {
                "armor-exempt"
            },
            if *reduced { "hurt" } else { "hurt_unreduced" }
        );
    }
}

/// Every route that debits a body **tells that body where it came from**,
/// or is on the record as having nowhere to point.
///
/// The mirror of `ROUTES`' second column, and it exists for the same reason:
/// the choice is per-route, a person has to make it, and until it was
/// written down the answer drifted. Wire v57 landed `EV_HURT` on the three
/// routes a duel goes through and left the bite and the blast silent —
/// eleven days in which a bear in the dark took hp off you and the only
/// thing on screen was the bar. `ROUTES` already knew both files were
/// damage routes and had no column to be wrong in.
///
/// ## What it refuses
///
/// A **disagreement** between the table and the source, in either
/// direction, per file:
///
/// - a route declared announcing whose file emits no `EV_HURT` — the
///   regression that was actually shipped, and the one a new route repeats
///   by copying the emit-less liturgy from the file next door;
/// - a route declared silent whose file emits one — either a real direction
///   was found for it (good, and the row's stated reason is now a lie that
///   the next reader will believe) or an emit landed in the wrong module.
///
/// ## Anti-vacuity
///
/// Three things, because a scrape that stops matching is the failure this
/// crate's own trap list calls the worst bug class. The needle must find
/// *some* announcing file; every announcing file it finds must be a
/// declared route, so an emit in an unclassified module is a loud failure
/// rather than an unnoticed pass; and the table must contain at least one
/// row of each kind, so deleting the silent rows cannot make the gate
/// trivially satisfiable.
#[test]
fn every_damage_route_points_somewhere_or_says_why_not() {
    let Scan { announces, .. } = scan();

    assert!(
        !announces.is_empty(),
        "no file in `sim-core/src` emits `EV_HURT,` — either the victim's \
         half of a blow has been deleted from the sim, or the `EV_HURT,` \
         needle stopped matching and this gate is now guarding nothing"
    );
    assert!(
        ROUTES.iter().any(|(_, _, a, _)| *a) && ROUTES.iter().any(|(_, _, a, _)| !*a),
        "`ROUTES` no longer holds both an announcing and a silent row — with \
         one kind left this gate cannot fail in one of its two directions"
    );

    for f in &announces {
        assert!(
            ROUTES.iter().any(|(file, _, _, _)| file == f),
            "{f} emits `EV_HURT` and is not a declared damage route. Either \
             it debits a body without going through `combat::hurt` — which \
             the funnel gate above should already have caught — or the \
             victim's half of a blow is being raised by a module that does \
             not deal the blow, which addresses an arc to a body nothing hit"
        );
    }

    for (f, _, announce, why) in ROUTES {
        let emits = announces.iter().any(|a| a == f);
        assert_eq!(
            emits,
            *announce,
            "{f} is declared {} (\"{why}\") and {} `EV_HURT`. A route that \
             debits a body either tells that body which way the blow came \
             from or has a stated reason it cannot — and the reason lives in \
             `ROUTES`, not in whichever module happened to be edited last. \
             If this route has just grown a direction, flip its column and \
             rewrite its reason; if it has just lost one, that is the \
             regression this gate is here for",
            if *announce {
                "as pointing somewhere"
            } else {
                "as having nowhere to point"
            },
            if emits { "emits" } else { "does not emit" },
        );
    }
}
