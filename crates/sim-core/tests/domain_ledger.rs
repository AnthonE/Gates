//! NOW.md §5b: the value domains the wire carries *wider* than the sim
//! means are closed ledgers, and the constant that names each ledger's top
//! must actually be its top — because the server refuses against that
//! constant at its accept/encode boundaries, so a member added past a
//! stale top would be silently refused forever (the 2026-08-05 death-cause
//! failure shape, on a different field).
//!
//! Same method as protocol's `every_domain_fits_its_wire_field`: the
//! constant block is read as text, so a member this file has never heard
//! of still counts. `BAG_GONE_MAX`/`REFUSE_C_MAX` are literals rather than
//! `DEATH_BY_MAX`-style aliases only because protocol's scrape parses
//! their families with an empty exempt list, and that list is a wire-pass
//! edit; this test is what keeps the literals honest until then.

use sim_core::backpack::BAG_GONE_MAX;
use sim_core::input::BTN_MASK;
use sim_core::survival::REFUSE_C_MAX;

/// Scrape `prefix`-family members out of a source file: `(name, value)`
/// per `pub const <prefix><name><ty><value>;` line, skipping `exempt`.
/// Values are plain literals or the `1 << n` bit form.
fn members<'a>(src: &'a str, prefix: &str, ty: &str, exempt: &[&str]) -> Vec<(&'a str, u32)> {
    let mut found = Vec::new();
    for line in src.lines() {
        let Some(rest) = line.trim().strip_prefix(prefix) else {
            continue;
        };
        let Some((name, value)) = rest.split_once(ty) else {
            continue;
        };
        if exempt.contains(&name) {
            continue;
        }
        let value = value.trim_end_matches(';').trim();
        let v = if let Some((base, shift)) = value.split_once("<<") {
            let b: u32 = base.trim().parse().unwrap_or_else(|_| {
                panic!("{prefix}{name}: `{value}` is not a scrapeable literal")
            });
            let s: u32 = shift.trim().parse().unwrap_or_else(|_| {
                panic!("{prefix}{name}: `{value}` is not a scrapeable literal")
            });
            b << s
        } else {
            value.parse().unwrap_or_else(|_| {
                panic!(
                    "{prefix}{name}: `{value}` is not a literal. This gate \
                     reads the block as text to learn the ledger; give the \
                     member a literal or add it to this test's exempt list \
                     with the reason."
                )
            })
        };
        found.push((name, v));
    }
    found
}

#[test]
fn bag_gone_max_names_the_ledgers_top() {
    let m = members(
        include_str!("../src/backpack.rs"),
        "pub const BAG_GONE_",
        ": u32 = ",
        &["MAX"],
    );
    assert!(
        m.len() >= 3,
        "only {} BAG_GONE_* members scraped — the block's shape changed \
         and this gate is reading nothing, which is worse than failing",
        m.len()
    );
    let top = m.iter().map(|(_, v)| *v).max().unwrap();
    assert_eq!(
        BAG_GONE_MAX, top,
        "backpack.rs declares a BAG_GONE_* reason above BAG_GONE_MAX. The \
         server refuses EV_BAG_REMOVED against that constant, so the new \
         reason would be counted and dropped on every emit — move the \
         literal up in the same commit, and read NOW.md §5b for the wire \
         side the widening also owes."
    );
}

#[test]
fn refuse_c_max_names_the_ledgers_top() {
    let m = members(
        include_str!("../src/survival.rs"),
        "pub const REFUSE_C_",
        ": u32 = ",
        &["MAX"],
    );
    assert!(
        m.len() >= 3,
        "only {} REFUSE_C_* members scraped — the block's shape changed \
         and this gate is reading nothing, which is worse than failing",
        m.len()
    );
    let top = m.iter().map(|(_, v)| *v).max().unwrap();
    assert_eq!(
        REFUSE_C_MAX, top,
        "survival.rs declares a REFUSE_C_* reason above REFUSE_C_MAX. The \
         server refuses EV_CONSUME_REFUSED against that constant, so the \
         new reason would be counted and dropped on every emit — move the \
         literal up in the same commit, and read NOW.md §5b for the wire \
         side the widening also owes."
    );
}

#[test]
fn btn_mask_covers_every_declared_button() {
    let m = members(
        include_str!("../src/input.rs"),
        "pub const BTN_",
        ": u8 = ",
        &["MASK"],
    );
    assert!(
        m.len() >= 4,
        "only {} BTN_* members scraped — the block's shape changed and \
         this gate is reading nothing, which is worse than failing",
        m.len()
    );
    let union = m.iter().fold(0u32, |a, (_, v)| a | *v);
    assert_eq!(
        BTN_MASK as u32, union,
        "input.rs declares a button bit outside BTN_MASK. The server \
         refuses any wire frame carrying an unmasked bit (net.rs \
         accept_input), so every press of the new button would be refused \
         at the door — join the mask in the same commit that declares the \
         bit (and that widening is a wire meaning change: input.rs's JUMP \
         note says why it turns PROTO_VER)."
    );
}
