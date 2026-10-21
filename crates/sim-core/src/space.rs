//! The broadphase freeform building will stand on (freeform M8, unwired):
//! which pieces and deployables might touch a 4 m patch of the world.
//!
//! Once a base can face any way, a piece no longer lives in one grid cell,
//! so the cell-keyed column index (`collide::ColIndex`) cannot find it. This
//! is the table that will: a multimap from a [`SPACE_BUCKET_M`] world bucket
//! to the store indices filed under it, with the solid table's layout
//! (`ColIndex::add_solid`) — one fixed box, Fibonacci home slot, linear
//! probe, Knuth 6.4 R backward-shift delete, nothing allocated after
//! construction. The same buckets are what chunk interest will subscribe
//! clients to. Values are opaque here; the caller tags piece and deploy
//! indices apart.

use crate::fmath::floor_i32;
use crate::limits::SPACE_SLOTS;

/// A bucket's side, metres.
pub const SPACE_BUCKET_M: f32 = 4.0;
/// Buckets along each side of the island.
pub const SPACE_SIDE: usize = (crate::terrain::ISLAND_SIZE / SPACE_BUCKET_M) as usize;
const _: () = assert!(SPACE_SIDE <= 1 << 10);
/// The most buckets one [`Space::add_box`] may file a value under: 3 × 3,
/// which holds any box under 4 m a side wherever it falls (a 3 m piece
/// turned 45° is 4.25 m across its corners, so a piece files under at most
/// 2 × 2 of them along each axis it spans).
pub const MAX_BOX_BUCKETS: usize = 9;

const OCCUPIED: u32 = 1 << 31;

/// The bucket holding a world point, or `None` off the island.
pub fn bucket_of(x: f32, z: f32) -> Option<(u16, u16)> {
    let (bx, bz) = (floor_i32(x / SPACE_BUCKET_M), floor_i32(z / SPACE_BUCKET_M));
    let side = SPACE_SIDE as i32;
    (bx >= 0 && bz >= 0 && bx < side && bz < side).then_some((bx as u16, bz as u16))
}

/// Bucket → values.
pub struct Space {
    keys: Box<[u32; SPACE_SLOTS]>,
    vals: Box<[u32; SPACE_SLOTS]>,
    len: u32,
}

impl Space {
    pub fn new() -> Self {
        Self {
            keys: crate::boxed_array(0),
            vals: crate::boxed_array(0),
            len: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.len as usize
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn clear(&mut self) {
        self.keys.fill(0);
        self.vals.fill(0);
        self.len = 0;
    }

    #[inline]
    fn key(bx: u16, bz: u16) -> u32 {
        OCCUPIED | (bx as u32) << 10 | bz as u32
    }

    #[inline]
    fn home(key: u32) -> usize {
        (key.wrapping_mul(0x9E37_79B1) >> (32 - SPACE_SLOTS.trailing_zeros())) as usize
            & (SPACE_SLOTS - 1)
    }

    /// File `val` under a bucket. Filing the same pair twice is one row.
    /// False when the table is at half load — the caller refuses the
    /// placement, it never evicts.
    pub fn add(&mut self, bx: u16, bz: u16, val: u32) -> bool {
        let key = Self::key(bx, bz);
        let mut i = Self::home(key);
        loop {
            let k = self.keys[i];
            if k == 0 {
                break;
            }
            if k == key && self.vals[i] == val {
                return true;
            }
            i = (i + 1) & (SPACE_SLOTS - 1);
        }
        if self.len as usize >= SPACE_SLOTS / 2 {
            return false;
        }
        self.keys[i] = key;
        self.vals[i] = val;
        self.len += 1;
        true
    }

    /// Take one (bucket, value) row out; absent is a no-op.
    pub fn del(&mut self, bx: u16, bz: u16, val: u32) {
        let key = Self::key(bx, bz);
        let mut i = Self::home(key);
        loop {
            let k = self.keys[i];
            if k == 0 {
                return;
            }
            if k == key && self.vals[i] == val {
                break;
            }
            i = (i + 1) & (SPACE_SLOTS - 1);
        }
        self.keys[i] = 0;
        self.vals[i] = 0;
        self.len -= 1;
        let mut j = i;
        loop {
            j = (j + 1) & (SPACE_SLOTS - 1);
            let k = self.keys[j];
            if k == 0 {
                return;
            }
            let h = Self::home(k);
            if j.wrapping_sub(h) & (SPACE_SLOTS - 1) >= j.wrapping_sub(i) & (SPACE_SLOTS - 1) {
                self.keys[i] = k;
                self.vals[i] = self.vals[j];
                self.keys[j] = 0;
                self.vals[j] = 0;
                i = j;
            }
        }
    }

    /// Every value filed under a bucket, in table order.
    pub fn at(&self, bx: u16, bz: u16) -> SpaceAt<'_> {
        let key = Self::key(bx, bz);
        SpaceAt {
            space: self,
            key,
            i: Self::home(key),
        }
    }

    /// The buckets a world box `[x0, x1] × [z0, z1]` covers, clamped to the
    /// island: `None` if it covers more than [`MAX_BOX_BUCKETS`] or none.
    pub fn box_buckets(
        x0: f32,
        z0: f32,
        x1: f32,
        z1: f32,
    ) -> Option<([(u16, u16); MAX_BOX_BUCKETS], usize)> {
        let lo = bucket_of(x0.max(0.0), z0.max(0.0))?;
        let edge = crate::terrain::ISLAND_SIZE - 0.001;
        let hi = bucket_of(x1.min(edge), z1.min(edge))?;
        if hi.0 < lo.0 || hi.1 < lo.1 {
            return None;
        }
        let (w, d) = ((hi.0 - lo.0 + 1) as usize, (hi.1 - lo.1 + 1) as usize);
        if w * d > MAX_BOX_BUCKETS {
            return None;
        }
        let mut out = [(0, 0); MAX_BOX_BUCKETS];
        let mut n = 0;
        for bx in lo.0..=hi.0 {
            for bz in lo.1..=hi.1 {
                out[n] = (bx, bz);
                n += 1;
            }
        }
        Some((out, n))
    }

    /// File `val` under every bucket a world box covers — all or none.
    pub fn add_box(&mut self, x0: f32, z0: f32, x1: f32, z1: f32, val: u32) -> bool {
        let Some((cells, n)) = Self::box_buckets(x0, z0, x1, z1) else {
            return false;
        };
        if self.len as usize + n > SPACE_SLOTS / 2 {
            return false;
        }
        for &(bx, bz) in &cells[..n] {
            self.add(bx, bz, val);
        }
        true
    }

    /// Undo [`Space::add_box`] for the same box.
    pub fn del_box(&mut self, x0: f32, z0: f32, x1: f32, z1: f32, val: u32) {
        if let Some((cells, n)) = Self::box_buckets(x0, z0, x1, z1) {
            for &(bx, bz) in &cells[..n] {
                self.del(bx, bz, val);
            }
        }
    }
}

impl Default for Space {
    fn default() -> Self {
        Self::new()
    }
}

/// The values under one bucket ([`Space::at`]).
pub struct SpaceAt<'a> {
    space: &'a Space,
    key: u32,
    i: usize,
}

impl Iterator for SpaceAt<'_> {
    type Item = u32;

    fn next(&mut self) -> Option<u32> {
        loop {
            let k = self.space.keys[self.i];
            if k == 0 {
                return None;
            }
            let i = self.i;
            self.i = (self.i + 1) & (SPACE_SLOTS - 1);
            if k == self.key {
                return Some(self.space.vals[i]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::Pcg32;

    /// Random adds and deletes over a few crowded buckets, against a plain
    /// list: every bucket must hold exactly the shadow's values after every
    /// operation — the gate backward-shift deletion has to pass.
    #[test]
    fn space_churn_matches_a_naive_shadow() {
        let mut rng = Pcg32::new(0x5BACE, 4);
        let mut space = Space::new();
        let mut shadow: Vec<(u16, u16, u32)> = Vec::new();
        for _ in 0..20_000 {
            let (bx, bz) = (
                rng.next_bounded(3) as u16 + 500,
                rng.next_bounded(3) as u16 + 500,
            );
            let val = rng.next_bounded(40);
            if rng.next_bounded(2) == 0 {
                assert!(space.add(bx, bz, val));
                if !shadow.contains(&(bx, bz, val)) {
                    shadow.push((bx, bz, val));
                }
            } else {
                space.del(bx, bz, val);
                shadow.retain(|&e| e != (bx, bz, val));
            }
            assert_eq!(space.len(), shadow.len());
            let mut got: Vec<u32> = space.at(bx, bz).collect();
            let mut want: Vec<u32> = shadow
                .iter()
                .filter(|e| (e.0, e.1) == (bx, bz))
                .map(|e| e.2)
                .collect();
            got.sort_unstable();
            want.sort_unstable();
            assert_eq!(got, want);
        }
    }

    #[test]
    fn a_box_files_under_every_bucket_it_covers() {
        let mut space = Space::new();
        // 3 m square turned 45°: 4.24 m across, straddling a bucket corner.
        let (x, z, r) = (2000.0, 2000.0, 2.13);
        assert!(space.add_box(x - r, z - r, x + r, z + r, 7));
        assert_eq!(space.len(), 4);
        for (bx, bz) in [(499, 499), (499, 500), (500, 499), (500, 500)] {
            assert_eq!(space.at(bx, bz).collect::<Vec<_>>(), vec![7]);
        }
        space.del_box(x - r, z - r, x + r, z + r, 7);
        assert!(space.is_empty());
        assert!(!space.add_box(0.0, 0.0, 13.0, 13.0, 1), "4 × 4 buckets");
        assert_eq!(bucket_of(-0.1, 5.0), None);
        assert_eq!(bucket_of(4095.9, 0.0), Some((1023, 0)));
    }
}
