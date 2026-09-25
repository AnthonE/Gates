//! Rust's item notices — the pure half.
//!
//! When something lands in your inventory, Rust says so down the right of
//! the screen, above the vitals: one row per item, `WOOD` and `+26 (845)` —
//! what arrived and what you now hold — the newest on top, older rows
//! fading. Ours said it in the centre toast as `+26 × Wood`, the same lane as
//! refusals and kills, where ten swings at a tree pushed everything else
//! off. `render::hud` draws these rows; this owns what they say and when
//! they go.
//!
//! One row per ITEM, not per event: a second pickup of wood while the first
//! row is still up adds to it and moves it to the top, which is how a
//! stream of gathers reads as one growing number instead of a column of
//! identical lines.

/// Rows on screen at once. A full stack drops its oldest row for a new
/// item (wall 4's bound, and the stated policy: the newest fact wins).
pub const NOTICE_ROWS: usize = 5;
/// How long a row stays after its last pickup, seconds.
pub const NOTICE_SECS: f32 = 4.0;
/// The last part of that, over which it fades out.
pub const NOTICE_FADE_SECS: f32 = 1.0;

/// One row: what arrived and how long it has left.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Notice {
    pub item: u16,
    /// Everything that arrived while the row was up.
    pub gained: u32,
    pub left: f32,
}

impl Notice {
    /// Opacity now: full, then down to nothing over the fade.
    pub fn alpha(&self) -> f32 {
        (self.left / NOTICE_FADE_SECS).clamp(0.0, 1.0)
    }
}

/// The stack. Fixed storage, newest first.
#[derive(Clone, Copy, Debug, Default)]
pub struct Notices {
    rows: [Notice; NOTICE_ROWS],
    len: usize,
}

impl Notices {
    /// `count` of `item` arrived.
    pub fn add(&mut self, item: u16, count: u16) {
        if count == 0 {
            return;
        }
        let mut row = Notice {
            item,
            gained: 0,
            left: NOTICE_SECS,
        };
        // An item already up is taken out and put back on top with the sum.
        if let Some(i) = self.rows[..self.len].iter().position(|r| r.item == item) {
            row.gained = self.rows[i].gained;
            self.rows.copy_within(i + 1..self.len, i);
            self.len -= 1;
        }
        row.gained = row.gained.saturating_add(count as u32);
        let keep = self.len.min(NOTICE_ROWS - 1);
        self.rows.copy_within(0..keep, 1);
        self.rows[0] = row;
        self.len = keep + 1;
    }

    /// Age every row by `dt` seconds and drop the spent ones.
    pub fn tick(&mut self, dt: f32) {
        let mut kept = 0;
        for i in 0..self.len {
            let mut r = self.rows[i];
            r.left -= dt;
            if r.left > 0.0 {
                self.rows[kept] = r;
                kept += 1;
            }
        }
        self.len = kept;
    }

    /// The live rows, newest first.
    pub fn rows(&self) -> &[Notice] {
        &self.rows[..self.len]
    }
}

/// The right-hand figure: `+26 (845)`, what arrived and what is held.
pub fn amount_label(gained: u32, held: u32) -> String {
    format!("+{gained} ({held})")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_repeat_item_adds_up_and_comes_to_the_top() {
        let mut n = Notices::default();
        n.add(1, 10);
        n.add(2, 3);
        n.add(1, 5);
        let rows = n.rows();
        assert_eq!(rows.len(), 2);
        assert_eq!((rows[0].item, rows[0].gained), (1, 15));
        assert_eq!((rows[1].item, rows[1].gained), (2, 3));
    }

    #[test]
    fn a_full_stack_drops_its_oldest() {
        let mut n = Notices::default();
        for item in 0..NOTICE_ROWS as u16 + 2 {
            n.add(item, 1);
        }
        let items: Vec<u16> = n.rows().iter().map(|r| r.item).collect();
        assert_eq!(items, vec![6, 5, 4, 3, 2]);
    }

    #[test]
    fn rows_fade_then_go() {
        let mut n = Notices::default();
        n.add(7, 1);
        n.tick(NOTICE_SECS - NOTICE_FADE_SECS * 0.5);
        assert_eq!(n.rows().len(), 1);
        assert!((n.rows()[0].alpha() - 0.5).abs() < 1e-4);
        // A new pickup of the same item brings it back to full.
        n.add(7, 1);
        assert_eq!(n.rows()[0].alpha(), 1.0);
        assert_eq!(n.rows()[0].gained, 2);
        n.tick(NOTICE_SECS + 0.1);
        assert!(n.rows().is_empty());
        n.add(3, 0);
        assert!(n.rows().is_empty(), "nothing arrived, nothing to say");
    }

    #[test]
    fn the_amount_reads_like_rusts() {
        assert_eq!(amount_label(26, 845), "+26 (845)");
    }
}
