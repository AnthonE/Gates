//! The hearth's upkeep panel (Rust's cupboard view): what the hearth holds,
//! what a day of the base costs in each material, and how long it stays
//! protected. The words and the arithmetic, kept out of `render` so a
//! headless test can read them; `render/hud.rs::hearth_overlay` draws them.
//!
//! **No wire of its own.** The rows are the stock ack a feed already sends
//! (`EventMsg::Stock`, latched on `ClientCore::stock`): `E` at a hearth
//! feeds it and opens this, and a feed with nothing to give still answers.
//! The server answers only the crew, which is why an empty panel says so.
//! The same ack is pushed to a crew member standing in the claim every ten
//! seconds, and that is what the HUD's upkeep vital reads ([`vital`]).
//!
//! A panel that does **not** grab the pointer, for the keypad's reason
//! (`ui::keypad`): the hearth is in the room you are defending, so walking
//! away closes it instead of a menu holding you still.

/// One upkeep period is one hour: `sim_core::deploy::UPKEEP_PERIOD_TICKS`
/// at the sim's tick rate. A test pins the two together.
pub const PERIOD_MINUTES: u64 = 60;

/// The footer: how to close, what `E` did, and how the crew takes back.
pub const HINT: &str = "[E] CLOSE  ·  E FEEDS UP TO 500 OF EACH  ·  [1]-[4] TAKE 500 BACK";

/// What the panel says when no stock ack names this hearth: the server
/// reads a hearth out to its crew alone.
pub const NO_REPORT: &str = "ONLY THE CREW CAN READ THIS HEARTH  ·  [L] JOIN CREW";

/// One material row: its item, what the hearth holds, what a day costs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Row {
    pub item: u16,
    pub units: u32,
    pub per_day: u32,
}

/// The ack's rows as the panel draws them. `bill` is already a day's
/// charge (wire v89, `sim_core::upkeep::bill`).
pub fn rows(stock: &[(u16, u32, u32)]) -> Vec<Row> {
    stock
        .iter()
        .map(|&(item, units, bill)| Row {
            item,
            units,
            per_day: bill,
        })
        .collect()
}

/// Minutes of protection left: the least `units / bill` over the billed
/// materials, to the minute — `sim_core::upkeep::lasts`'s rule (the first
/// material to run out is the first part of the base to rot), finer than
/// its whole hours. `None` when nothing is billed.
pub fn minutes_left(stock: &[(u16, u32, u32)]) -> Option<u64> {
    stock
        .iter()
        .filter(|&&(_, _, bill)| bill > 0)
        .map(|&(_, units, bill)| units as u64 * PERIOD_MINUTES * 24 / bill as u64)
        .min()
}

/// Minutes as a player reads a clock: `1D 4H 30M`, `4H 30M`, `30M`.
pub fn duration_label(minutes: u64) -> String {
    let (d, h, m) = (minutes / 1440, minutes / 60 % 24, minutes % 60);
    if d > 0 {
        format!("{d}D {h}H {m}M")
    } else if h > 0 {
        format!("{h}H {m}M")
    } else {
        format!("{m}M")
    }
}

/// The panel's verdict line, the reading the reference's cupboard leads
/// with.
pub fn status_line(stock: &[(u16, u32, u32)]) -> String {
    match minutes_left(stock) {
        None => "NOTHING TO PAY — NO BUILDING IN ITS CLAIM".to_string(),
        Some(0) => "NOT PROTECTED — THE BASE IS DECAYING".to_string(),
        Some(m) => format!("PROTECTED FOR {}", duration_label(m)),
    }
}

/// How long a stock reading stays on the HUD with no fresh one: three of
/// the shard's pushes (`sim_core::deploy::CREW_VITAL_TICKS`). They come
/// only while you stand in a claim you are crew of, so a reading this old
/// means you walked out (or died), and the vital goes with it.
pub const VITAL_STALE_TICKS: u32 = 3 * sim_core::deploy::CREW_VITAL_TICKS as u32;

/// The crew HUD vital (NOW §0up 3): `UPKEEP 1D 4H 30M` while the base is
/// paid, `BASE DECAYING` once a material has run out — the `bool` says it
/// is the warning. `None` when there is nothing to say: nothing billed (a
/// base of twig), or a reading `age` ticks old that the shard stopped
/// refreshing. The clock is the latest push's own; at one every ten
/// seconds there is nothing to count down between them.
pub fn vital(stock: &[(u16, u32, u32)], age: u32) -> Option<(String, bool)> {
    if age > VITAL_STALE_TICKS {
        return None;
    }
    match minutes_left(stock)? {
        0 => Some(("BASE DECAYING".to_string(), true)),
        m => Some((format!("UPKEEP {}", duration_label(m)), false)),
    }
}

/// Is a body at `pos` still in feeding reach of the hearth standing at
/// `(hx, hz)` (`DeployRec::xz`, where it was freely placed)? `deploy::feed`'s
/// own test — planar distance against `build::BUILD_REACH_M` — so the panel
/// closes where feeding would refuse.
pub fn in_reach(pos: [f32; 3], (hx, hz): (f32, f32)) -> bool {
    let (dx, dz) = (pos[0] - hx, pos[2] - hz);
    let r = sim_core::build::BUILD_REACH_M;
    dx * dx + dz * dz <= r * r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_hour_is_one_upkeep_period() {
        assert_eq!(
            sim_core::deploy::UPKEEP_PERIOD_TICKS,
            PERIOD_MINUTES * 60 * sim_core::limits::TICK_HZ as u64
        );
    }

    #[test]
    fn the_first_material_to_run_out_sets_the_clock() {
        // 100 wood at 240 a day is 10 h; 30 stone at 96 a day is 7 h 30 m.
        let stock = [(3, 100, 240), (4, 30, 96)];
        assert_eq!(minutes_left(&stock), Some(450));
        assert_eq!(status_line(&stock), "PROTECTED FOR 7H 30M");
        assert_eq!(rows(&stock)[1].per_day, 96);
        // It agrees with the sim's whole hours.
        assert_eq!(sim_core::upkeep::lasts(&[100, 30], &[240, 96]), Some(7));
    }

    #[test]
    fn an_unbilled_row_is_no_limit_and_short_is_decaying() {
        assert_eq!(minutes_left(&[(3, 100, 0)]), None);
        assert!(status_line(&[(3, 100, 0)]).starts_with("NOTHING TO PAY"));
        assert_eq!(
            status_line(&[(3, 0, 10)]),
            "NOT PROTECTED — THE BASE IS DECAYING"
        );
    }

    #[test]
    fn durations_read_as_a_clock() {
        assert_eq!(duration_label(59), "59M");
        assert_eq!(duration_label(61), "1H 1M");
        assert_eq!(duration_label(1440 + 4 * 60 + 30), "1D 4H 30M");
    }

    #[test]
    fn the_vital_reads_the_clock_and_goes_when_the_pushes_stop() {
        // 480 wood at 240 a day: two days.
        let paid = [(3, 480, 240), (4, 50, 0)];
        assert_eq!(
            vital(&paid, 0),
            Some(("UPKEEP 2D 0H 0M".to_string(), false))
        );
        assert_eq!(
            vital(&[(3, 0, 240)], 0),
            Some(("BASE DECAYING".to_string(), true))
        );
        // Nothing billed says nothing, however fresh.
        assert_eq!(vital(&[(3, 480, 0)], 0), None);
        assert_eq!(vital(&[], 0), None);
        // Up through three missed pushes it holds; past them it goes.
        assert!(vital(&paid, VITAL_STALE_TICKS).is_some());
        assert_eq!(vital(&paid, VITAL_STALE_TICKS + 1), None);
    }

    #[test]
    fn reach_is_the_feed_radius_off_the_hearth() {
        let (hx, hz) = (301.2, 299.4);
        assert!(in_reach([hx + 4.9, 0.0, hz], (hx, hz)));
        assert!(!in_reach([hx + 5.1, 0.0, hz], (hx, hz)));
    }
}
