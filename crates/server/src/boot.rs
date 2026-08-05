//! Boot-time preconditions on the seed a shard was told to run.
//!
//! Content validates at boot (CLAUDE.md wall 7) and so does the config. The
//! island did not. `terrain::haven(seed)` resolves the authored sites — the
//! haven pad plus `WAYSTATIONS` lesser ones — and `pick_minor` leaves a
//! waystation dead when no candidate on the road ring clears
//! `WAYSTATION_MIN_SEP_M`. That is the right call, and `terrain.rs` says why:
//! a site placed too close to another is worse than a site missing. It is
//! also *silent*, and a shard boots whatever seed `shard.toml` names, so a
//! seed the ring cannot fill ships an island a third smaller with no counter,
//! event or log line (`pass-20260805-074623-02-judge.md` fix 1, inherited
//! twice).
//!
//! **Measured before this was written, and it changes what this file is.**
//! Seeds 0..20_000 scanned on 2026-08-05: `sites_live == 3` on every one of
//! them, `sites_complete` false on none. The eight seeds this repo actually
//! names — `shard.toml.example` and `shard-public.toml` both run 20260731,
//! plus the test seeds 0, 1, 42, 1024, 0xC0FFEE, 20260804, 0xDEADBEEF — are
//! all complete. So the refusal below is a **tripwire, not a live hole**: no
//! seed reachable today takes the short branch, and nothing here closes a gap
//! a player can currently fall into. It fires when a change to the ring
//! radii, the separation floor, or the candidate search makes a short tier
//! possible — which is exactly the moment nobody is looking at waystations.
//! The counter on the success path is the half with value today: NOW.md's
//! complaint was "no counter, event or log line", and two of those three were
//! missing even on a seed that fills.
//!
//! Pure and socket-free on purpose. `spawn_shard` calls it before it loads an
//! identity or binds an endpoint, so an island refusal costs no port and no
//! certificate, and a test can reach it on a box without IPv6 — which this
//! one is (`CLAUDE.md`: `bot_smoke` is red here for that reason alone).

use sim_core::terrain::{self, Haven, WAYSTATIONS, WAYSTATION_MIN_SEP_M};

/// Every authored site an island is supposed to carry: the haven pad, which
/// `haven()` returns unconditionally, plus the lesser tier. This is the
/// number `sites_live` counts up to — it includes the pad, which is the
/// opposite convention from `tests/waystation.rs`, where `live` counts
/// waystations alone against `WAYSTATIONS`.
pub const AUTHORED_SITES: u32 = WAYSTATIONS as u32 + 1;

/// Refuse an island whose authored sites are short, and report how many are
/// live when they are not.
///
/// `seed` is carried only so the message can name it; the decision is a pure
/// function of `haven`. Split that way because the branch this guards cannot
/// be reached through `terrain::haven` on any seed measured (see the module
/// docs) — a test has to hand it a `Haven` with a dead `Waystation` to prove
/// the refusal says anything at all, and a gate that cannot exercise its own
/// failure path is decoration.
pub fn check_island(seed: u64, haven: &Haven) -> Result<u32, String> {
    let live = terrain::sites_live(haven);
    if terrain::sites_complete(haven) {
        debug_assert_eq!(live, AUTHORED_SITES);
        return Ok(live);
    }
    let dead = WAYSTATIONS - haven.minor.iter().filter(|w| w.live).count();
    Err(format!(
        "island refused: seed {seed} fills {live} of {AUTHORED_SITES} authored sites \
         — {dead} of {WAYSTATIONS} waystations found no candidate on the road ring \
         clearing WAYSTATION_MIN_SEP_M = {WAYSTATION_MIN_SEP_M} m. A short tier is \
         a deliberate refusal to crowd two sites together, not a defect, but it \
         ships a smaller island in silence: pick another seed, or move the floor \
         and register the knob."
    ))
}

/// `check_island` against the island the seed actually generates. What the
/// shard calls at boot.
pub fn check_seed(seed: u64) -> Result<u32, String> {
    check_island(seed, &terrain::haven(seed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim_core::terrain::Waystation;

    /// The seed both shipped configs name. If this ever goes red the public
    /// shard is the thing that changed, not the test.
    #[test]
    fn the_shipped_seed_fills_its_island() {
        assert_eq!(check_seed(20_260_731), Ok(AUTHORED_SITES));
    }

    /// The branch no seed reaches. Built by hand for exactly that reason —
    /// see the module docs for the 20_000-seed scan that says so.
    #[test]
    fn a_short_tier_is_refused_and_the_message_counts() {
        let mut haven = terrain::haven(20_260_731);
        haven.minor[WAYSTATIONS - 1] = Waystation::NONE;
        let err = check_island(7, &haven).expect_err("a dead waystation must refuse");
        assert!(err.contains("seed 7"), "message must name the seed: {err}");
        assert!(
            err.contains(&format!("{} of {AUTHORED_SITES}", AUTHORED_SITES - 1)),
            "message must count live against the target: {err}"
        );
        assert!(
            err.contains("WAYSTATION_MIN_SEP_M"),
            "message must name the floor that refused, so the operator can \
             move it deliberately rather than guess: {err}"
        );
    }

    /// Every waystation dead, not just one — the count in the message has to
    /// track, or it reads as a fixed string.
    #[test]
    fn an_empty_tier_counts_the_pad_alone() {
        let mut haven = terrain::haven(20_260_731);
        haven.minor = [Waystation::NONE; WAYSTATIONS];
        assert_eq!(terrain::sites_live(&haven), 1, "the pad always survives");
        let err = check_island(9, &haven).expect_err("an empty tier must refuse");
        assert!(
            err.contains(&format!("1 of {AUTHORED_SITES}")),
            "an empty tier must report the pad alone: {err}"
        );
        assert!(
            err.contains(&format!("{WAYSTATIONS} of {WAYSTATIONS} waystations")),
            "an empty tier must say all of them are dead: {err}"
        );
    }
}
