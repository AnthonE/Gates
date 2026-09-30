//! The hearth's upkeep panel (Rust's cupboard view): what the hearth holds,
//! what a day of the base costs in each material, and how long it stays
//! protected. The words and the arithmetic, kept out of `render` so a
//! headless test can read them; `render/hud.rs::hearth_overlay` draws them.
//!
//! **No wire of its own.** The rows are the stock ack a feed already sends
//! (`EventMsg::Stock`, latched on `ClientCore::stock`): `E` at a hearth
//! feeds it and opens this, and a feed with nothing to give still answers.
//! The server answers only the crew, which is why an empty panel says so.
//!
//! A panel that does **not** grab the pointer, for the keypad's reason
//! (`ui::keypad`): the hearth is in the room you are defending, so walking
//! away closes it instead of a menu holding you still.

/// One upkeep period is one hour: `sim_core::deploy::UPKEEP_PERIOD_TICKS`
/// at the sim's tick rate. A test pins the two together.
pub const PERIOD_MINUTES: u64 = 60;

/// The footer: how to close, and what `E` did.
pub const HINT: &str = "[E] CLOSE  ·  E FEEDS IT UP TO 500 OF EACH FROM YOUR PACK";

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

/// The ack's rows as the panel draws them. `bill` is one period's charge,
/// so a day is 24 of them.
pub fn rows(stock: &[(u16, u32, u32)]) -> Vec<Row> {
    stock
        .iter()
        .map(|&(item, units, bill)| Row {
            item,
            units,
            per_day: bill.saturating_mul(24),
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
        .map(|&(_, units, bill)| units as u64 * PERIOD_MINUTES / bill as u64)
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

/// Is a body at `pos` still in feeding reach of the hearth at `(cx, cz)`?
/// `deploy::feed`'s own test — planar distance to the cell centre against
/// `build::BUILD_REACH_M` — so the panel closes where feeding would refuse.
pub fn in_reach(pos: [f32; 3], cx: u16, cz: u16) -> bool {
    let (hx, hz) = sim_core::deploy::cell_center(cx, cz);
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
        // 100 wood at 10 an hour is 10 h; 30 stone at 4 an hour is 7 h 30 m.
        let stock = [(3, 100, 10), (4, 30, 4)];
        assert_eq!(minutes_left(&stock), Some(450));
        assert_eq!(status_line(&stock), "PROTECTED FOR 7H 30M");
        assert_eq!(rows(&stock)[1].per_day, 96);
        // It agrees with the sim's whole hours.
        assert_eq!(sim_core::upkeep::lasts(&[100, 30], &[10, 4]), Some(7));
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
    fn reach_is_the_feed_radius_off_the_cell_centre() {
        let (hx, hz) = sim_core::deploy::cell_center(100, 100);
        assert!(in_reach([hx + 4.9, 0.0, hz], 100, 100));
        assert!(!in_reach([hx + 5.1, 0.0, hz], 100, 100));
    }
}
