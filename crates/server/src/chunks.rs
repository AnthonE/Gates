//! Bounded class-S subscription bookkeeping. Changes are committed only
//! after their reliable lifecycle frame is accepted by the connection ring.
use protocol::chunk::{self, VISIBLE_CAP};

#[derive(Clone, Copy, Debug, Default)]
pub struct Subscription {
    pub chunk: u16,
    pub version: u32,
    pub pending: bool,
}

pub struct Subscriptions {
    entries: [Subscription; VISIBLE_CAP],
    len: usize,
    commit_cursor: usize,
    pub reset: bool,
}

impl Default for Subscriptions {
    fn default() -> Self {
        Self {
            entries: [Subscription::default(); VISIBLE_CAP],
            len: 0,
            commit_cursor: 0,
            reset: true,
        }
    }
}

impl Subscriptions {
    pub fn entries(&self) -> &[Subscription] {
        &self.entries[..self.len]
    }

    pub fn contains(&self, chunk: u16) -> bool {
        self.entries().iter().any(|s| s.chunk == chunk)
    }

    pub fn pending(&self, chunk: u16) -> bool {
        self.entries().iter().any(|s| s.chunk == chunk && s.pending)
    }

    pub fn enter(&mut self, chunk: u16, version: u32) -> bool {
        if self.contains(chunk) {
            return true;
        }
        if self.len == VISIBLE_CAP || usize::from(chunk) >= chunk::COUNT {
            return false;
        }
        self.entries[self.len] = Subscription {
            chunk,
            version,
            pending: true,
        };
        self.len += 1;
        true
    }

    pub fn leave(&mut self, chunk: u16) {
        if let Some(i) = self.entries().iter().position(|s| s.chunk == chunk) {
            self.len -= 1;
            self.entries[i] = self.entries[self.len];
        }
    }

    /// Rotate successful commits so constantly mutating chunks cannot starve
    /// a later subscription's completion behind the per-tick message budget.
    pub fn next_commit(&self, versions: &[u32]) -> Option<Subscription> {
        (0..self.len)
            .map(|offset| self.entries[(self.commit_cursor + offset) % self.len])
            .find(|s| s.pending || s.version != versions[s.chunk as usize])
    }

    pub fn ready(&mut self, chunk: u16, version: u32) {
        if let Some(i) = self.entries().iter().position(|s| s.chunk == chunk) {
            self.entries[i].version = version;
            self.entries[i].pending = false;
            self.commit_cursor = (i + 1) % self.len;
        }
    }

    pub fn clear(&mut self) {
        self.len = 0;
        self.commit_cursor = 0;
        self.reset = true;
    }

    /// Leaving is drained before entering, so a teleport never needs the
    /// union of two AOI windows to fit in the fixed subscription array.
    pub fn next_leave(&self, here: (i64, i64)) -> Option<u16> {
        self.entries()
            .iter()
            .find(|s| !chunk::in_range(s.chunk, here, true))
            .map(|s| s.chunk)
    }

    pub fn next_enter(&self, here: (i64, i64)) -> Option<u16> {
        let radius = sim_core::limits::AOI_ENTER_CM;
        let lo = |v: i64| (v.saturating_sub(radius).div_euclid(chunk::SIDE_CM)).max(0);
        let hi = |v: i64| {
            (v.saturating_add(radius).div_euclid(chunk::SIDE_CM)).min(chunk::SIDE as i64 - 1)
        };
        let mut best = None;
        let mut distance = i64::MAX;
        for z in lo(here.1)..=hi(here.1) {
            for x in lo(here.0)..=hi(here.0) {
                let id = (z * chunk::SIDE as i64 + x) as u16;
                if self.contains(id) || !chunk::in_range(id, here, false) {
                    continue;
                }
                let dx = x * chunk::SIDE_CM + chunk::SIDE_CM / 2 - here.0;
                let dz = z * chunk::SIDE_CM + chunk::SIDE_CM / 2 - here.1;
                let d = dx * dx + dz * dz;
                if d < distance {
                    best = Some(id);
                    distance = d;
                }
            }
        }
        best
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edge_motion_keeps_subscriptions_and_teleport_drains_before_entering() {
        let mut s = Subscriptions::default();
        let a = (100_000, 100_000);
        while let Some(id) = s.next_enter(a) {
            assert!(s.enter(id, 7));
        }
        assert!(s.entries().len() < VISIBLE_CAP);
        let first = s.entries()[0].chunk;
        assert!(s.pending(first));
        s.ready(first, 8);
        assert!(!s.pending(first));
        assert!(s.next_leave((a.0 + 1, a.1)).is_none());
        let b = (200_000, 200_000);
        while let Some(id) = s.next_leave(b) {
            s.leave(id);
        }
        assert!(s.entries().is_empty());
        while let Some(id) = s.next_enter(b) {
            assert!(s.enter(id, 12));
        }
        assert!(!s.contains(first));
    }

    #[test]
    fn an_unaccepted_transition_does_not_change_membership() {
        let s = Subscriptions::default();
        let first = s.next_enter((10_000, 10_000));
        assert!(first.is_some());
        assert_eq!(s.next_enter((10_000, 10_000)), first);
        assert!(s.entries().is_empty());
    }
    #[test]
    fn hot_chunks_do_not_starve_pending_subscriptions() {
        let mut subscriptions = Subscriptions::default();
        let mut versions = [1; chunk::COUNT];
        for id in 0..6 {
            assert!(subscriptions.enter(id, 0));
        }
        for _ in 0..6 {
            let next = subscriptions.next_commit(&versions).unwrap();
            subscriptions.ready(next.chunk, versions[next.chunk as usize]);
            versions[0] += 1;
        }
        assert!(subscriptions.entries().iter().all(|s| !s.pending));
    }
}
