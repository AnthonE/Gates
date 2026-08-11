//! Stamp the commit into the binary, so a running build can say which one it
//! is.
//!
//! `protocol` is the home for this and not `client` or `server`, because both
//! of them depend on it and the version gate it feeds lives here — one crate
//! computes the identity, everybody reads the same answer. It is above
//! `sim-core` in the graph, so nothing here can reach wall 1's purity: the
//! sim never sees a build id, and no value this file emits enters a state
//! hash. A build script is compile-time host code and the walls that ban I/O
//! are about the tick.
//!
//! ## What it emits
//!
//! `GATES_GIT_SHA` — nine hex of `HEAD`, plus `-dirty` when the tree had
//! uncommitted changes. `unknown` when there is no git to ask, which is the
//! case that matters for a source tarball: a build with no commit to name is
//! still a build, and refusing to compile over a missing `.git` would be a
//! worse failure than an honest `unknown`.
//!
//! `GATES_GIT_SHA` in the environment **overrides the probe**, and that is for
//! CI: a tagged release build should carry the tag's commit even when the
//! checkout is shallow or detached in a way `rev-parse` describes oddly.
//!
//! ⚠ **The dirty marker is stamped when this crate last rebuilt, not when the
//! binary was linked.** Cargo reruns a build script when its declared inputs
//! change, and "somebody edited a file in another crate" is not expressible as
//! one — so an edit to `crates/client` after `protocol` compiled leaves the id
//! reading clean. That is a diagnostic wart and deliberately not load-bearing:
//! the shard gates on the semver, never on this string, and `ci/depot.py`
//! keys a dirty build on the launch binary's own sha256, which is exact
//! because it hashes the bytes it is about to ship. Read this field as "the
//! commit this build came from", not as a proof of what is in it.

// Host-side build script: `String`, `Command` and `println!` are literally its
// interface — cargo reads its directives off stdout. The walls that ban those
// (1 and 3) are about the sim thread and the crate's shipped code; a build
// script runs on the developer's machine at compile time and none of it is
// linked into the binary. Same call `examples/gen_goldens.rs` makes in the
// same crate, for the same reason, and scoped to this file rather than
// loosened in `clippy.toml` — the walls stay exactly as tight for everything
// that ships.
#![allow(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    clippy::disallowed_macros
)]

use std::process::Command;

fn main() {
    // The version itself comes from CARGO_PKG_VERSION, which cargo always
    // sets; only the sha needs asking for.
    println!("cargo:rerun-if-env-changed=GATES_GIT_SHA");

    if let Ok(forced) = std::env::var("GATES_GIT_SHA") {
        let forced = forced.trim();
        if !forced.is_empty() {
            println!("cargo:rustc-env=GATES_GIT_SHA={forced}");
            return;
        }
    }

    // Rerun when HEAD moves. `.git/HEAD` covers a checkout or a detach; the
    // ref it names covers a commit on the branch we are already on. Both are
    // relative to the workspace root, two levels up from this crate.
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    if let Some(head) = read_head_ref() {
        println!("cargo:rerun-if-changed=../../.git/{head}");
    }

    println!(
        "cargo:rustc-env=GATES_GIT_SHA={}",
        probe().unwrap_or_else(|| "unknown".into())
    );
}

/// The ref `HEAD` points at, e.g. `refs/heads/main`. `None` on a detached
/// HEAD, where the sha is in `HEAD` itself and the file we already watch is
/// the whole story.
fn read_head_ref() -> Option<String> {
    let head = std::fs::read_to_string("../../.git/HEAD").ok()?;
    let path = head.trim().strip_prefix("ref: ")?;
    Some(path.to_string())
}

/// `<9 hex>` or `<9 hex>-dirty`. `None` when git cannot answer at all.
fn probe() -> Option<String> {
    let sha = git(&["rev-parse", "--short=9", "HEAD"])?;
    if sha.is_empty() {
        return None;
    }
    // An empty porcelain listing is a clean tree. A *failed* status is not
    // evidence of cleanliness, so it reads as dirty: over-reporting is the
    // safe direction for a marker whose whole job is to say "this is not the
    // commit".
    let dirty = match git(&["status", "--porcelain", "--untracked-files=no"]) {
        Some(out) => !out.is_empty(),
        None => true,
    };
    Some(if dirty { format!("{sha}-dirty") } else { sha })
}

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8(out.stdout).ok()?.trim().to_string())
}
