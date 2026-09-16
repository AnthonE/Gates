//! Boot-time preconditions on the seed a shard was told to run.
//!
//! Content validates at boot (CLAUDE.md wall 7) and so does the config. The
//! island did not. `terrain::haven(seed)` resolves the authored sites — the
//! haven pad plus `MINOR_SITES` lesser ones, across two tiers — and
//! `pick_minor` leaves a slot dead when no candidate its tier can reach
//! clears `WAYSTATION_MIN_SEP_M`. That is the right call, and `terrain.rs` says why:
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

use sim_core::terrain::{
    self, Haven, SiteKind, INLAND_SITES, MINOR_SITES, WAYSTATIONS, WAYSTATION_MIN_SEP_M,
};

/// Every authored site an island is supposed to carry: the haven pad, which
/// `haven()` returns unconditionally, plus the lesser tier. This is the
/// number `sites_live` counts up to — it includes the pad, which is the
/// opposite convention from `tests/waystation.rs`, where `live` counts
/// waystations alone against `WAYSTATIONS`.
///
/// **Every lesser tier, not just the ring one.** It was `WAYSTATIONS + 1`
/// while the ring was the only lesser tier; `MINOR_SITES` is the length of
/// `Haven::minor`, which is the thing `sites_live` actually walks.
pub const AUTHORED_SITES: u32 = MINOR_SITES as u32 + 1;

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
    // Counted per TIER, off the dead slot's own `kind` rather than off its
    // index — `terrain::empty_minor` is what makes that readable. The two
    // tiers fail for different reasons and an operator does different things
    // about them, so a message that said "waystations" for a short interior
    // would send them to look at the road ring.
    let dead_ring = dead_of(haven, SiteKind::Waystation);
    let dead_inland = dead_of(haven, SiteKind::Inland);
    Err(format!(
        "island refused: seed {seed} fills {live} of {AUTHORED_SITES} authored sites \
         — {dead_ring} of {WAYSTATIONS} waystations found no candidate on the road ring, \
         {dead_inland} of {INLAND_SITES} inland sites none in the interior, \
         clearing WAYSTATION_MIN_SEP_M = {WAYSTATION_MIN_SEP_M} m. A short tier is \
         a deliberate refusal to crowd two sites together, not a defect, but it \
         ships a smaller island in silence: pick another seed, or move the floor \
         and register the knob."
    ))
}

/// Dead slots of one tier. The dead slot carries its own `kind`
/// (`terrain::empty_minor`), so this is a filter and not an index range.
fn dead_of(haven: &Haven, kind: SiteKind) -> usize {
    haven
        .minor
        .iter()
        .filter(|w| !w.live && w.kind == kind)
        .count()
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
        haven.minor = terrain::empty_minor();
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
        assert!(
            err.contains(&format!("{INLAND_SITES} of {INLAND_SITES} inland")),
            "an empty roster must count the interior tier separately, or the \
             operator is sent to look at the road ring: {err}"
        );
    }
}
