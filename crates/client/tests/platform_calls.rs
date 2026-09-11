//! **The class the compile gate structurally cannot see.** Code tier: no
//! `render` feature, no GPU, no socket, no browser.
//!
//! `ci/gates.sh` builds this crate for `wasm32-unknown-unknown` and that gate
//! is worth having — but it can only catch a call that fails to COMPILE, and
//! the calls that matter most here all compile perfectly. On
//! `wasm32-unknown-unknown`:
//!
//! | call | what actually happens |
//! |---|---|
//! | `std::fs::*` | returns `Err` — benign, the code already handles it |
//! | `std::env::var` | returns `Err` — benign |
//! | `SystemTime::now`, `Instant::now` | **panics**: `RuntimeError: unreachable` |
//! | `thread::spawn`, `thread::sleep` | **panics** |
//! | `std::process::id` | **panics** |
//!
//! The bottom three rows are a runtime trap wearing a target triple, and
//! `findings/web-build-20260909.md` §10.2 already names that shape as the
//! nastier one — `tokio/time` was called out for exactly it. This gate found
//! its first live instance the same day it was written: `shot.rs`'s
//! `now_secs()` called `SystemTime::now()` in a module `lib.rs` compiles
//! unconditionally, so the browser client shipped carrying it.
//!
//! **Scope is a decision, and it is written down here rather than derived.**
//! The scan covers `src/` INCLUDING `render/`, which is not built for wasm32
//! today. Deriving the file list from what compiles for wasm now would exclude
//! `render/` entirely, and the gate would go green over the four
//! `thread::spawn` sites it most exists for — a gate aimed at nothing, wearing
//! the fix for the hand-kept-mirror problem. `CLAUDE.md` has both failures in
//! its trap list; this is the one where the exact gate is the wrong gate.
//!
//! Two exclusions, both by name and both loud:
//! - `src/elo_overlay.rs` is VENDORED from `AnthonE/scry-forge` and unpatchable
//!   here (`CLAUDE.md` §vendored). It is `cfg`'d off wasm at its `mod`
//!   declaration in `elo.rs`, not in the file, so a scan of the file alone
//!   cannot see its guard.
//! - `src/main.rs` and `src/bin/` carry `required-features = ["native"]` and
//!   are never compiled for wasm32 by anything. That is a manifest fact this
//!   test re-reads rather than remembers.
//!
//! **Four mutants run, four caught** (2026-09-10):
//!
//! | mutant | what it models |
//! |---|---|
//! | delete the cfg on `render/settings.rs`'s `thread::sleep` | a guard removed in a refactor |
//! | un-guard `shot.rs::now_secs` | the live bug this was written to find |
//! | a new `std::time::SystemTime::now()` in `lib.rs` | the next one somebody adds |
//! | `render/settings.rs` back to `use std::time::Instant` | the web-safe clock swapped out |
//!
//! The fourth is the one that proves the import-keying works in both
//! directions: this scan ignores a bare `Instant::now()` when the file imports
//! Bevy's clock, and flags the same characters when the file imports `std`'s.
//! A gate that could only do the first would be a gate that never fires.

use std::path::{Path, PathBuf};

/// Calls that abort a wasm module rather than returning an error.
///
/// Matched as `<path segment>::<call>` so both spellings are caught: the
/// fully-qualified `std::thread::spawn(..)` and the imported
/// `use std::time::SystemTime; SystemTime::now()` — the second spelling is
/// what `discord.rs` uses, so a scan that only knew the first would have
/// walked past it.
///
/// ⚠ `Instant` is listed WITHOUT a `std::` prefix requirement and that is
/// deliberate — but `bevy::platform::time::Instant` is the web-safe swap and
/// is already used by `render/settings.rs`, so the match is on the import, not
/// on the bare name. See `imports_std_time` below.
/// Fully-qualified spellings, which mean the same thing in any file.
const QUALIFIED: &[&str] = &[
    "std::thread::spawn(",
    "std::thread::sleep(",
    "std::process::id(",
    "std::time::SystemTime::now(",
    "std::time::Instant::now(",
];

/// Bare spellings, and the `std` type each one needs to be in scope before it
/// means anything.
///
/// ⚠ **The second column is why this is not a plain word list, and it is the
/// difference between a gate and a nuisance.** `render/settings.rs` calls
/// `Instant::now()` twice and both are correct: it imports
/// `bevy::platform::time::Instant`, which is the web-safe clock and the
/// prescribed swap. A scan that flagged the bare name would report the tree's
/// own fix as the defect — and the author would then delete the scan, which is
/// the real cost of a false positive in a gate nobody can silence.
const BARE: &[(&str, &str)] = &[
    ("thread::spawn(", "thread"),
    ("thread::sleep(", "thread"),
    ("process::id(", "process"),
    ("SystemTime::now(", "SystemTime"),
    ("Instant::now(", "Instant"),
];
/// Files the scan does not read, each with the reason it cannot.
const EXEMPT: &[(&str, &str)] = &[
    (
        "src/elo_overlay.rs",
        "vendored from AnthonE/scry-forge and unpatchable here (CLAUDE.md §vendored). \
         Its guard is the `#[cfg(not(target_arch = \"wasm32\"))]` on the `#[path] mod` \
         in elo.rs, which a scan of this file cannot see.",
    ),
    (
        "src/main.rs",
        "the `client` binary, `required-features = [\"native\"]` — never built for wasm32.",
    ),
];

/// Trapping sites that are known, unguarded, and not yet fixable — each with
/// what it is waiting on. A site that leaves this list must leave it in the
/// commit that guards it.
///
/// **Every entry here is in `render/`, and every one is in a module a browser
/// build does not keep.** All four sit in `menu.rs`, `hub.rs` or `boot.rs` —
/// the desktop front end, which reaches a local launcher over a unix socket
/// and fetches over a blocking one. In a tab the page owns the shard address,
/// the wallet and the join, so these three modules are cfg'd off wasm32
/// rather than ported, and every row here leaves with them.
///
/// ⚠ **The line numbers are exact and this list is therefore fragile by
/// design** — that is the trade the scan makes for being able to name a
/// precise site. Moving code in these files reddens the gate, which is the
/// correct behaviour and not a defect: it is the gate asking whether the
/// site it was watching is still the site it meant. Renumber deliberately,
/// after reading what is actually at the new line.
const KNOWN: &[(&str, u32, &str)] = &[
    ("src/render/boot.rs", 120, "the launcher handshake's thread"),
    ("src/render/hub.rs", 41, "the title manifest fetch thread"),
    ("src/render/menu.rs", 255, "the shard-list fetch thread"),
    (
        "src/render/menu.rs",
        376,
        // ⚠ This read "the connect future's runtime thread" until 2026-09-11
        // and named the wrong function: the connect future does not spawn a
        // thread at all, it goes on the tokio runtime `Rt` holds. This is
        // `begin_status_poll`'s batch — one thread for every row that names a
        // status endpoint. An annotation nobody re-reads is how a list stops
        // describing what it pins.
        "the status-poll round's thread",
    ),
];

fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display())) {
        let p = e.expect("readable entry").path();
        if p.is_dir() {
            rs_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

/// Whether `cfg` makes the item it guards unreachable on wasm32.
///
/// `test` counts: a `#[cfg(test)]` module is compiled by `cargo test` on the
/// host and by nothing on wasm. `unix` and `windows` count for the obvious
/// reason. Anything else does not, including a bare `#[cfg(feature = "…")]` —
/// a feature is not a target, which is the distinction `crates/client/
/// Cargo.toml`'s own header spends a paragraph on.
fn shields_wasm(cfg: &str) -> bool {
    let c: String = cfg.chars().filter(|c| !c.is_whitespace()).collect();
    c.contains("not(target_arch=\"wasm32\")")
        || c.contains("cfg(test)")
        || c.contains("cfg(unix)")
        || c.contains("cfg(windows)")
        || c.contains("any(not(target_arch=\"wasm32\"),test)")
}

/// Every trapping call in `path` that no enclosing `#[cfg]` shields, as
/// `(line, what)`.
///
/// **Block-tracked, not line-oriented**, because the tree's own correct code
/// would fail a line-oriented scan: `render/settings.rs` carries its
/// `#[cfg(not(target_arch = "wasm32"))]` two lines above the guarded call,
/// with an `if` between. An attribute applies to the next item that opens a
/// block, and the guard holds until that block closes.
fn unguarded(path: &Path) -> Vec<(u32, String)> {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    // Which `std` types this file actually brought into scope. A bare
    // `Instant::now()` is only a trap if `Instant` is `std::time`'s.
    let std_uses: String = text
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with("use std::"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut out = Vec::new();
    // (depth at which the guard was pushed, shields)
    let mut guards: Vec<(usize, bool)> = Vec::new();
    let mut pending: Option<bool> = None;
    let mut depth = 0usize;

    for (i, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.starts_with("//") {
            continue;
        }
        if let Some(rest) = line.strip_prefix("#[cfg") {
            // Several attributes may stack on one item; a single shielding one
            // is enough, so OR them together until the item arrives.
            let shields = shields_wasm(&format!("cfg{rest}"));
            pending = Some(pending.unwrap_or(false) || shields);
            continue;
        }
        if line.starts_with('#') || line.is_empty() {
            continue;
        }

        let opens = line.matches('{').count();
        let closes = line.matches('}').count();

        // The call is checked at the depth it sits at, BEFORE this line's own
        // braces move it — an attribute one line above has already landed.
        let shielded = guards.iter().any(|(_, s)| *s);
        if !shielded {
            for needle in QUALIFIED {
                if line.contains(needle) {
                    out.push((i as u32 + 1, needle.trim_end_matches('(').to_string()));
                }
            }
            for (needle, wants) in BARE {
                if line.contains(needle)
                    && !line.contains(&format!("std::{needle}"))
                    && std_uses.contains(wants)
                {
                    out.push((i as u32 + 1, needle.trim_end_matches('(').to_string()));
                }
            }
        }

        if opens > closes {
            // This item opens a block: whatever attribute was pending guards
            // it for the block's lifetime.
            depth += opens - closes;
            guards.push((depth, pending.take().unwrap_or(false)));
        } else if closes > opens {
            let shut = closes - opens;
            for _ in 0..shut {
                if guards.last().is_some_and(|(d, _)| *d == depth) {
                    guards.pop();
                }
                depth = depth.saturating_sub(1);
            }
            pending = None;
        } else {
            // A statement that neither opens nor closes consumes the pending
            // attribute (a guarded field, a guarded `use`).
            pending = None;
        }
    }
    out
}

/// **Every trapping call is guarded, or written down as waiting on something.**
#[test]
fn no_unguarded_call_traps_a_wasm_module() {
    let mut files = Vec::new();
    rs_files(Path::new("src"), &mut files);
    files.sort();
    assert!(
        files.len() > 20,
        "the scan found only {} files under src/ — it is reading the wrong tree",
        files.len()
    );

    // The exemptions are checked for existence, so a rename turns them into a
    // loud failure instead of a silent skip.
    for (name, why) in EXEMPT {
        assert!(
            Path::new(name).is_file(),
            "{name} is exempt from this scan because {why} — but it is gone. Remove the \
             exemption, or fix the path."
        );
    }

    let mut found = Vec::new();
    for path in &files {
        let name = path.to_string_lossy().replace('\\', "/");
        if EXEMPT.iter().any(|(e, _)| *e == name) || name.starts_with("src/bin/") {
            continue;
        }
        for (line, what) in unguarded(path) {
            found.push((name.clone(), line, what));
        }
    }

    let known: Vec<(String, u32)> = KNOWN
        .iter()
        .map(|(f, l, _)| ((*f).to_string(), *l))
        .collect();
    let fresh: Vec<_> = found
        .iter()
        .filter(|(f, l, _)| !known.contains(&(f.clone(), *l)))
        .collect();

    assert!(
        fresh.is_empty(),
        "these calls compile for wasm32 and PANIC there, with nothing guarding them:\n{}\n\n\
         On wasm32-unknown-unknown `std::fs` and `std::env::var` return errors, but \
         `SystemTime::now`, `Instant::now`, `thread::spawn`/`sleep` and `process::id` abort \
         the module with `RuntimeError: unreachable` — no message, no line. The compile gate \
         cannot see them, which is why this scan exists.\n\n\
         Either put the site behind `#[cfg(not(target_arch = \"wasm32\"))]` with a browser \
         arm beside it (`shot.rs::now_secs` and `discord.rs::start` are the shape), swap the \
         type (`bevy::platform::time::Instant` is the web-safe clock and \
         `render/settings.rs` already uses it), or add it to KNOWN with what it is waiting on.",
        fresh
            .iter()
            .map(|(f, l, w)| format!("  {f}:{l}  {w}"))
            .collect::<Vec<_>>()
            .join("\n")
    );

    // And the other direction: a KNOWN entry that has been fixed or moved is
    // stale, and a stale exemption list is how a list stops meaning anything.
    for (f, l, why) in KNOWN {
        assert!(
            found.iter().any(|(g, m, _)| g == f && m == l),
            "KNOWN lists {f}:{l} ({why}) but the scan no longer finds a trapping call there. \
             If it was fixed, drop the entry; if it moved, move the entry."
        );
    }
}

/// **And the scan can still see, which is the assertion that keeps the one
/// above from passing by reading nothing.**
///
/// It re-finds the guarded `thread::sleep` in `render/settings.rs` by
/// re-scanning that file with the shield disabled. If the file stops
/// containing a guarded trap, this names it rather than quietly proving less.
#[test]
fn the_scan_still_finds_what_it_is_looking_for() {
    let path = Path::new("src/render/settings.rs");
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert!(
        text.contains("std::thread::sleep(target - now)"),
        "render/settings.rs no longer holds the guarded `thread::sleep` this gate proves \
         itself against — find another shielded trap and re-point this."
    );
    assert!(
        unguarded(path).is_empty(),
        "render/settings.rs's `thread::sleep` IS guarded, and the scan reported it anyway: \
         {:?}. The block tracker is broken, and a broken tracker fails open on every other \
         file.",
        unguarded(path)
    );
}
