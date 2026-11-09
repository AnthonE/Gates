//! Tick-start interval error. Fixed atomic histograms; no allocation or clock
//! read here. The boundary loop supplies its existing monotonic timestamps.
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

pub const BUCKETS: usize = 32;

#[derive(Default)]
pub struct TickJitter {
    /// Off unless a measurement explicitly enables it.
    pub enabled: AtomicBool,
    /// Absolute interval error in microseconds: bucket 0 is [0,1), bucket
    /// b is [2^(b-1),2^b), and bucket 31 includes all larger values.
    pub absolute: [AtomicU64; BUCKETS],
    pub early: AtomicU64,
    pub late: AtomicU64,
    pub max_us: AtomicU64,
}

impl TickJitter {
    pub fn record(&self, interval: Duration, nominal: Duration) {
        if !self.enabled.load(Ordering::Relaxed) {
            return;
        }
        let error = interval.abs_diff(nominal);
        let us = error.as_micros().min(u64::MAX as u128) as u64;
        let bucket = (u64::BITS - us.leading_zeros()) as usize;
        self.absolute[bucket.min(BUCKETS - 1)].fetch_add(1, Ordering::Relaxed);
        if interval < nominal {
            self.early.fetch_add(1, Ordering::Relaxed);
        } else if interval > nominal {
            self.late.fetch_add(1, Ordering::Relaxed);
        }
        self.max_us.fetch_max(us, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> [u64; BUCKETS] {
        std::array::from_fn(|i| self.absolute[i].load(Ordering::Relaxed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interval_error_is_not_tick_work_time() {
        let j = TickJitter::default();
        j.enabled.store(true, Ordering::Relaxed);
        let nominal = Duration::from_micros(33_333);
        for us in [33_333, 33_332, 33_335, 33_337, 43_333] {
            j.record(Duration::from_micros(us), nominal);
        }
        let bins = j.snapshot();
        assert_eq!(bins.iter().sum::<u64>(), 5);
        assert_eq!(&bins[..4], &[1, 1, 1, 1]);
        assert_eq!(bins[14], 1);
        assert_eq!(j.early.load(Ordering::Relaxed), 1);
        assert_eq!(j.late.load(Ordering::Relaxed), 3);
        assert_eq!(j.max_us.load(Ordering::Relaxed), 10_000);
        j.record(Duration::MAX, nominal);
        assert_eq!(j.snapshot()[BUCKETS - 1], 1);
    }
}
