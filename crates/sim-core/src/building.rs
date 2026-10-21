//! The buildings a shard holds (freeform W3): one [`Frame`] each, minted
//! by a foundation placed free and freed with the building's last piece.
//!
//! A building is nothing but its frame and how many pieces stand in it —
//! the pieces are `build::Pieces`' records, each naming its building. Rows
//! are reused lowest-first, so which row a new building takes is a pure
//! function of the store.

use crate::frame::Frame;
use crate::limits::MAX_BUILDINGS;

pub struct Buildings {
    frames: Box<[Frame; MAX_BUILDINGS]>,
    /// Live pieces per row; zero is a free row.
    counts: Box<[u16; MAX_BUILDINGS]>,
    /// Rows with a piece in them.
    live: usize,
}

impl Default for Buildings {
    fn default() -> Self {
        Self::new()
    }
}

impl Buildings {
    pub fn new() -> Self {
        Self {
            frames: crate::boxed_array(Frame::ZERO),
            counts: crate::boxed_array(0),
            live: 0,
        }
    }

    pub fn clear(&mut self) {
        self.frames.fill(Frame::ZERO);
        self.counts.fill(0);
        self.live = 0;
    }

    /// The frame of a live building.
    pub fn get(&self, b: u16) -> Option<&Frame> {
        let i = b as usize;
        (i < MAX_BUILDINGS && self.counts[i] > 0).then(|| &self.frames[i])
    }

    /// Pieces standing in building `b`.
    pub fn count(&self, b: u16) -> u16 {
        self.counts.get(b as usize).copied().unwrap_or(0)
    }

    pub fn live(&self) -> usize {
        self.live
    }

    /// Every live building, lowest row first.
    pub fn iter(&self) -> impl Iterator<Item = (u16, &Frame)> {
        self.counts
            .iter()
            .zip(self.frames.iter())
            .enumerate()
            .filter(|(_, (&n, _))| n > 0)
            .map(|(i, (_, f))| (i as u16, f))
    }

    /// The row a new building would take, if there is one.
    pub fn next_free(&self) -> Option<u16> {
        self.counts.iter().position(|&n| n == 0).map(|i| i as u16)
    }

    /// Count one more piece into building `b`, minting it with `frame` if it
    /// was free. The caller has checked the row (`next_free` or a live one).
    pub fn retain(&mut self, b: u16, frame: Frame) {
        let i = b as usize;
        if self.counts[i] == 0 {
            self.frames[i] = frame;
            self.live += 1;
        }
        self.counts[i] += 1;
    }

    /// One piece fewer in building `b`; the last one frees the row.
    pub fn release(&mut self, b: u16) {
        let i = b as usize;
        if i >= MAX_BUILDINGS || self.counts[i] == 0 {
            return;
        }
        self.counts[i] -= 1;
        if self.counts[i] == 0 {
            self.frames[i] = Frame::ZERO;
            self.live -= 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_are_reused_lowest_first_and_freed_with_their_last_piece() {
        let mut b = Buildings::new();
        let f = Frame::new((100, 200), 7, 1_000);
        assert_eq!(b.next_free(), Some(0));
        b.retain(0, f);
        b.retain(0, Frame::ZERO);
        assert_eq!(b.get(0), Some(&f), "a live row keeps its frame");
        assert_eq!(b.next_free(), Some(1));
        b.retain(1, Frame::new((5, 5), 0, 0));
        b.release(0);
        assert_eq!(b.count(0), 1);
        b.release(0);
        assert!(b.get(0).is_none());
        assert_eq!(b.next_free(), Some(0));
        assert_eq!(b.live(), 1);
        assert_eq!(b.iter().map(|(i, _)| i).collect::<Vec<_>>(), vec![1]);
    }
}
