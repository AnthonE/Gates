//! **Solvers read `height`, consumers read `ground`** — the carve's one rule,
//! held as a gate because it is the kind of rule a new call site breaks in
//! silence.
//!
//! `terrain::height` is the raw worldgen surface and `terrain::ground` is that
//! surface plus every authored site's carve (`terrain.rs`, `SITE_STAMP_STRENGTH`).
//! Both typecheck anywhere a seed is in scope, both return an `f32` in metres,
//! and picking the wrong one produces no error at all — it produces a crate
//! floating over a pad, or a player standing in the air above a mesh that knows
//! about the carve when the collision path does not. There is no value to
//! assert and no golden that moves. **The defect is a call site**, so the
//! instrument is a scrape, exactly as it is for `tests/sound.rs`'s single-drain
//! rule.
//!
//! The rule, restated as the two ways to be wrong:
//!
//! - A **consumer** reading raw `height` — anything that *stands on* the world:
//!   the surface under a body, the drawn mesh, a projectile's ground hit, a
//!   foundation's footing, the `y` an authored object is seated at. Missing the
//!   carve here is the visible failure above.
//! - A **solver** reading carved `ground` — anything that *locates* the world:
//!   where a site goes, where the road runs, where a spawn is. `haven(seed)` is
//!   built out of `height` taps, so a solver on carved ground is scoring ground
//!   it already carved. That direction is mostly prevented by the type system
//!   (`ground` needs a `&Haven`, and inside the solver there is not one yet),
//!   which is why this file gates the first direction and states the second.
//!
//! **What it actually asserts**: the set of functions that read raw terrain is
//! exactly the list below. Not a count — the list, by name. A new raw reader in
//! an unlisted function fails, and the author either converts it or adds it here
//! with the reason, which is the review this rule wants and cannot get from a
//! number. A listed function that stops reading raw also fails, so the list
//! cannot rot into a set of names that mean nothing.

use std::path::Path;

/// Every function permitted to read raw `terrain::height` / `terrain::slope`,
/// with the reason it is not a consumer. Sorted by file, then function.
///
/// Adding a row is a claim that the ground under this call does not need to
/// know about the carve. The two admissible shapes of that claim:
///   **locator** — it decides WHERE something goes, and reading the carve would
///   feed the solve its own output;
///   **carve-blind** — the value is compared against something the carve
///   provably cannot move it across (sea level, at a site that sits well above
///   it), or it is a depiction at a scale the carve is invisible at.
const RAW_READERS: &[(&str, &str, &str)] = &[
    // ---- terrain.rs: the solver itself -------------------------------------
    (
        "terrain.rs",
        "height",
        "locator: the raw surface's own definition.",
    ),
    (
        "terrain.rs",
        "ground",
        "the carved reader itself — `ground` IS `height` plus the stamp, so it \
         reads raw by construction. The one row here that is neither locator nor \
         carve-blind, and the gate would be broken if it were absent.",
    ),
    (
        "terrain.rs",
        "slope",
        "locator: the raw surface's gradient. `ground_slope` is the carved twin.",
    ),
    (
        "terrain.rs",
        "road_band",
        "locator: defines where the ring runs, and `haven()` calls it — a carved \
         read here is direct recursion.",
    ),
    (
        "terrain.rs",
        "in_bay",
        "locator: shares `road_band`'s shoreline definition and is only evaluated \
         on cells it classified, so the two must read one surface.",
    ),
    (
        "terrain.rs",
        "haven",
        "locator: the stage 8 argmax — the shoreline march, the bisect and the \
         candidate's own y. This is the function the carve is derived FROM.",
    ),
    (
        "terrain.rs",
        "site_floor_y",
        "locator: it CHOOSES the altitude a site's floor is cut to, by minimax \
         over the raw ground. Reading the carve here would have the datum \
         measure the cut it is about to define — the circularity in miniature, \
         and the one place it could still be written.",
    ),
    (
        "terrain.rs",
        "haven_relief",
        "locator: the flatness term the argmax scores. Carve it and the score \
         measures the carve.",
    ),
    (
        "terrain.rs",
        "haven_ring_phase",
        "locator: chooses the container ring's rotation.",
    ),
    (
        "terrain.rs",
        "haven_shelter_bearing",
        "locator: chooses the shelter's bearing.",
    ),
    (
        "terrain.rs",
        "waystation_ring_phase",
        "locator: `haven_ring_phase` with the lesser tier's numbers.",
    ),
    (
        "terrain.rs",
        "waystation_canopy_bearing",
        "locator: chooses the canopy's bearing.",
    ),
    (
        "terrain.rs",
        "splat",
        "carve-blind BY DECISION, and the one row here that is a residual rather \
         than a rule: `splat(seed, x, z)` has no `&Haven` and its only caller is \
         the client's footstep material. Converting the signature would drag \
         `examples/seed_scan.rs` onto carved ground for a survey that is about \
         raw worldgen. The cost is that a footstep on a carved pad reports the \
         material of the ground that was there before it — inaudible, and \
         `NOW.md` carries it.",
    ),
    // ---- the rest of sim-core ----------------------------------------------
    (
        "world.rs",
        "spawn_pos_n",
        "locator: the beach spawn search — a shoreline bisect plus its accept \
         predicate. The body it chooses a spot for is SEATED by `Body::at`, which \
         is carved; the search itself is locating and stays raw. Spawn targets \
         the beach and the sites sit on the 600..1000 m road ring, so the two \
         cannot meet.",
    ),
    ("mob.rs", "home_of", "locator: picks where an animal lives."),
    (
        "mob.rs",
        "guard_home_of",
        "locator: picks where a site's guard lives — on the site's apron, chosen \
         against the raw ground for the same reason `home_of` is.",
    ),
    (
        "mob.rs",
        "think",
        "carve-blind: a `<= floor` beached test against sea level, and an \
         authored site sits above `LAND_MIN_H` by its own solver's guard.",
    ),
    (
        "survival.rs",
        "water_in_reach",
        "carve-blind: 'is there water within reach', compared against SEA_LEVEL.",
    ),
    (
        "probe.rs",
        "hash_scatter_window",
        "locator: the determinism probe hashes WORLDGEN, which is the raw surface \
         by definition. Hashing the carve would make the fingerprint a function \
         of the site solver's output rather than of the generator.",
    ),
    // ---- the client --------------------------------------------------------
    (
        "map.rs",
        "paint",
        "carve-blind: the minimap depicts the island at ~2 m a pixel and the whole \
         carve is three discs of 11-12 m. Its hillshade gradient is the same \
         claim at the same scale.",
    ),
    (
        "water.rs",
        "stream",
        "carve-blind: depth below SEA_LEVEL. The carve never moves ground across \
         sea level — the flat floor sits at the site's own y, which its solver \
         already held above LAND_MIN_H.",
    ),
    (
        "water.rs",
        "shore_exposure",
        "carve-blind: shore foam, the same sea-level claim as `stream`.",
    ),
];

/// Files this gate reads. `terrain.rs` and the client's render/ui paths are the
/// interesting ones; the rest are swept so a raw read cannot appear in a corner
/// nobody thought to name.
/// Read as `&'static str`, leaked once per file.
///
/// Wall 3 disallows `String` and `format!` in this crate and `event_roles.rs`
/// is right that it binds a test too. A directory sweep cannot use
/// `include_str!` — the whole point is that a file nobody named still counts —
/// so the contents are leaked once and every name and slice below is borrowed
/// out of them. A handful of leaks in a test binary buys a gate with no owned
/// string in it.
fn sources() -> Vec<(&'static str, &'static str)> {
    fn walk(dir: &Path, out: &mut Vec<(&'static str, &'static str)>) {
        let rd =
            std::fs::read_dir(dir).unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()));
        let mut entries: Vec<_> = rd.map(|e| e.expect("readable entry").path()).collect();
        entries.sort();
        for p in entries {
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|x| x == "rs") {
                let name: &'static str = Box::leak(
                    p.file_name()
                        .expect("a file")
                        .to_string_lossy()
                        .into_owned()
                        .into_boxed_str(),
                );
                let text: &'static str = Box::leak(
                    std::fs::read_to_string(&p)
                        .expect("readable source")
                        .into_boxed_str(),
                );
                out.push((name, text));
            }
        }
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut v = Vec::new();
    walk(&root.join("src"), &mut v);
    walk(&root.join("../client/src"), &mut v);
    walk(&root.join("../client-core/src"), &mut v);
    assert!(
        v.len() > 20,
        "the scrape found only {} source files — it is pointed at the wrong tree",
        v.len()
    );
    v
}

/// A raw-terrain read: `terrain::height(`, `height(seed`, and the `slope` twins,
/// in any of the paths the tree writes them (`crate::`, `sim_core::`, bare).
fn is_raw_read(line: &str) -> bool {
    let t = line.trim_start();
    if t.starts_with("//") {
        return false;
    }
    // Spelled out rather than built: wall 3 disallows `format!` here.
    for pat in [
        "terrain::height(",
        "crate::terrain::height(",
        "sim_core::terrain::height(",
        "terrain::slope(",
        "crate::terrain::slope(",
        "sim_core::terrain::slope(",
    ] {
        if line.contains(pat) {
            return true;
        }
    }
    // Bare, inside terrain.rs itself.
    if line.contains("height(seed") && !line.contains("ground_height(") {
        return true;
    }
    if line.contains("slope(seed") && !line.contains("ground_slope(") {
        return true;
    }
    false
}

/// The function a line sits in — the nearest preceding `fn` at top level.
///
/// Refuses to guess: a raw read it cannot attribute is a loud failure, never a
/// skip, for the reason `tests/sound.rs` gives about its own scrape — the floor
/// has slack and a silently dropped case is always the newest one.
fn enclosing_fn<'a>(lines: &[&'a str], i: usize) -> &'a str {
    for j in (0..=i).rev() {
        let l = lines[j];
        let indent = l.len() - l.trim_start().len();
        if indent > 4 {
            continue;
        }
        let t = l.trim_start();
        for pre in [
            "pub fn ",
            "pub(crate) fn ",
            "fn ",
            "pub const fn ",
            "const fn ",
        ] {
            if let Some(rest) = t.strip_prefix(pre) {
                return rest.split(['(', '<', ' ']).next().unwrap_or("?");
            }
        }
    }
    "?"
}

/// Where the file's `#[cfg(test)] mod tests` starts, as a line number.
///
/// Test code is out of scope: a fixture asserting against raw worldgen is a
/// legitimate thing to write, and `tests/carve.rs` is nothing but that.
fn test_mod_line(src: &str) -> usize {
    match src.find("#[cfg(test)]") {
        Some(ix) => src[..ix].matches('\n').count(),
        None => usize::MAX,
    }
}

#[test]
fn only_the_named_solvers_read_raw_terrain() {
    let mut found: Vec<(&'static str, &'static str)> = Vec::new();
    for (name, src) in sources() {
        let lines: Vec<&str> = src.lines().collect();
        let tests_at = test_mod_line(src);
        for (i, l) in lines.iter().enumerate() {
            if i >= tests_at || !is_raw_read(l) {
                continue;
            }
            let f = enclosing_fn(&lines, i);
            assert_ne!(
                f,
                "?",
                "{}:{} reads raw terrain and this gate cannot tell which function \
                 it is in. That is a refusal to classify rather than a skip: fix \
                 the scrape or move the call inside a named fn.\n  {}",
                name,
                i + 1,
                l
            );
            if !found.contains(&(name, f)) {
                found.push((name, f));
            }
        }
    }

    let unlisted: Vec<(&str, &str)> = found
        .iter()
        .copied()
        .filter(|(f, n)| !RAW_READERS.iter().any(|(af, an, _)| af == f && an == n))
        .collect();
    assert!(
        unlisted.is_empty(),
        "these functions read RAW terrain and are not on this gate's list:\n  \
         {unlisted:?}\n\nSolvers read `terrain::height`, consumers read \
         `terrain::ground`. If this function stands something on the world — a \
         body, a mesh vertex, a projectile, a foundation's footing, an authored \
         object's `y` — it is a consumer and wants `ground(seed, haven, x, z)` \
         (or `ground_slope`). If it LOCATES something, add it to RAW_READERS with \
         the reason. Do not add it to make the gate pass."
    );

    let stale: Vec<(&str, &str)> = RAW_READERS
        .iter()
        .map(|(f, n, _)| (*f, *n))
        .filter(|(f, n)| !found.iter().any(|(ff, fnn)| ff == f && fnn == n))
        .collect();
    assert!(
        stale.is_empty(),
        "these are on this gate's raw-reader list and no longer read raw \
         terrain:\n  {stale:?}\n\nDrop the rows. A list carrying names that mean \
         nothing is how the next reader learns to distrust it."
    );
}

/// The carved reader exists and is what the consumers actually call — so the
/// list above cannot be satisfied by a tree in which nothing carves at all.
///
/// Without this, deleting `ground` and reverting every consumer to `height`
/// would leave `only_the_named_solvers_read_raw_terrain` failing loudly, which
/// is fine — but converting them to some *third* reader would pass it in
/// silence. This pins the consumers that must never go back.
#[test]
fn the_load_bearing_consumers_read_the_carved_ground() {
    const MUST_CARVE: &[(&str, &str)] = &[
        ("movement.rs", "the surface under a body"),
        ("ranged.rs", "a projectile's ground hit"),
        ("terrain_mesh.rs", "the drawn ground"),
        ("structures.rs", "a foundation's skirt"),
        ("place.rs", "the placement ghost"),
    ];
    let src = sources();
    for (file, what) in MUST_CARVE {
        let (_, text) = src
            .iter()
            .find(|(n, _)| n == file)
            .unwrap_or_else(|| panic!("{file} is not in the scrape — re-aim this gate"));
        assert!(
            text.contains("ground(seed") || text.contains("terrain::ground("),
            "{file} is {what} and does not read `terrain::ground` anywhere. \
             Either the carve has been reverted there, or it now reads some third \
             surface — both are the failure this gate exists for."
        );
    }
}
