//! A bounded remembered list — **the one component behind every "who may"**
//! (`reference/BUILDING.md` §1 fact 1).
//!
//! ## Why this is its own module
//!
//! The reference game built one access-list pattern and hung it on three
//! unrelated classes — `BuildingPrivlidge`, `AutoTurret`, `VehiclePrivilege`
//! — with the same three methods each: add, remove-self, clear. Add the
//! locks' remembered list and it is four. That is a component, and the
//! evidence is in their own hook table rather than in anyone's taste.
//!
//! We had one already (`lock.rs`'s auth and guest lists, lock v1) and the
//! hearth's crew is the second. Writing it twice is how the two acquire two
//! different eviction rules, two different overflow policies, and two
//! different answers to "what happens when the owner adds themselves
//! twice" — none of which any gate would notice, because each copy is
//! correct alone. So it is written once, here, and both call it.
//!
//! ## The rules, stated once
//!
//! - **Bounded** by the const parameter, which every caller names from
//!   `limits.rs` (wall 4).
//! - **Refuse, never evict.** A full list rejects the newcomer and keeps
//!   everyone it already holds. Dropping the oldest would make a base that
//!   forgets its founder, which is the one failure this cap may not have.
//! - **Idempotent add.** Adding a member who is already on the list is a
//!   success and changes nothing, so a double keypress is not a bug.
//! - **The tail is zeroed.** `state_hash` folds the whole array rather than
//!   the live prefix (residue would otherwise be invisible state), so a
//!   removal must clear what it vacates. That is the one invariant with
//!   teeth here and [`Roster::remove`] is the only thing that can break it.
//!
//! Pure and allocation-free like everything in this crate. The scan is
//! linear over a list whose cap is single digits — a `contains` on eight
//! `u32`s is not something to be clever about.

/// What an [`Roster::add`] did. Three outcomes rather than a bool, because
/// the caller's *event* differs for each: a fresh member is news, an
/// already-member is silence, and a full list is a refusal with a reason.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Added {
    /// Newly remembered.
    New,
    /// Already remembered; nothing changed and nothing is wrong.
    Already,
    /// The list is at its cap. Refuse — see the module header.
    Full,
}

/// A bounded, ordered, duplicate-free list of player ids.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Roster<const N: usize> {
    ids: [u32; N],
    len: u8,
}

impl<const N: usize> Default for Roster<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> Roster<N> {
    pub const CAP: usize = N;

    pub const fn new() -> Self {
        Self {
            ids: [0; N],
            len: 0,
        }
    }

    /// A list holding exactly one member — the shape every one of these
    /// starts in, because placing the thing is joining its list
    /// (`BUILDING.md` §1 fact 4, and lock v1's own rule).
    pub fn of(id: u32) -> Self {
        let mut r = Self::new();
        r.ids[0] = id;
        r.len = 1;
        r
    }

    pub fn len(&self) -> usize {
        self.len as usize
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn is_full(&self) -> bool {
        self.len as usize == N
    }

    /// The live members, in the order they joined.
    pub fn members(&self) -> &[u32] {
        &self.ids[..self.len as usize]
    }

    /// The whole backing array, live prefix and zeroed tail. **For
    /// `state_hash` and `worldsave` only** — a reader that wants the
    /// membership wants [`members`](Self::members).
    pub fn raw(&self) -> &[u32; N] {
        &self.ids
    }

    pub fn contains(&self, id: u32) -> bool {
        self.ids[..self.len as usize].contains(&id)
    }

    /// Remember `id`. See [`Added`] for the three outcomes.
    pub fn add(&mut self, id: u32) -> Added {
        if self.contains(id) {
            return Added::Already;
        }
        if self.is_full() {
            return Added::Full;
        }
        self.ids[self.len as usize] = id;
        self.len += 1;
        Added::New
    }

    /// Forget `id`; false when it was never there. **Order-preserving**,
    /// deliberately: a swap-remove would reorder the list, and the list's
    /// order is hashed, so two shards that removed the same member from
    /// differently-grown lists would disagree about a world they agree
    /// about. The shift is over a single-digit array.
    pub fn remove(&mut self, id: u32) -> bool {
        let Some(at) = self.ids[..self.len as usize].iter().position(|&m| m == id) else {
            return false;
        };
        for i in at..self.len as usize - 1 {
            self.ids[i] = self.ids[i + 1];
        }
        self.len -= 1;
        // The vacated slot, zeroed — the module header's one invariant
        // with teeth.
        self.ids[self.len as usize] = 0;
        true
    }

    /// Forget everyone.
    pub fn clear(&mut self) {
        self.ids = [0; N];
        self.len = 0;
    }

    /// Forget everyone and remember `keep` alone. The shape of every
    /// "this verb is the eviction" call — one operation, so a caller
    /// cannot clear and then fail to re-add.
    pub fn reset_to(&mut self, keep: u32) {
        self.clear();
        self.ids[0] = keep;
        self.len = 1;
    }

    /// Rebuild from a decoded save. `None` when the file's count is past
    /// the cap — **refused whole, never clamped**, because a clamped count
    /// is a list that quietly forgot somebody and this module's whole
    /// promise is that it never does that silently (`worldsave.rs` takes
    /// the same posture with every other count).
    ///
    /// The tail is re-zeroed rather than trusted: a file carrying residue
    /// past its own count would load to a different `state_hash` than it
    /// was saved from, which is wall 5 failing at the origin.
    pub fn restore(ids: &[u32; N], len: u8) -> Option<Self> {
        if len as usize > N {
            return None;
        }
        let mut r = Self::new();
        r.ids[..len as usize].copy_from_slice(&ids[..len as usize]);
        r.len = len;
        Some(r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type R = Roster<4>;

    #[test]
    fn add_is_idempotent_and_refuses_rather_than_evicting() {
        let mut r = R::of(1);
        assert_eq!(r.add(1), Added::Already, "a double press is not a bug");
        assert_eq!(r.len(), 1);
        for id in 2..=4 {
            assert_eq!(r.add(id), Added::New);
        }
        assert!(r.is_full());
        assert_eq!(r.add(9), Added::Full);
        assert_eq!(
            r.members(),
            &[1, 2, 3, 4],
            "a full list keeps everyone it had — it never forgets its founder"
        );
        assert!(!r.contains(9));
    }

    #[test]
    fn remove_preserves_order_and_zeroes_what_it_vacates() {
        let mut r = R::of(1);
        for id in 2..=4 {
            r.add(id);
        }
        assert!(r.remove(2));
        assert_eq!(
            r.members(),
            &[1, 3, 4],
            "order is hashed, so removal shifts rather than swaps"
        );
        assert_eq!(
            r.raw()[3],
            0,
            "a vacated slot must be zeroed — state_hash folds the whole array"
        );
        assert!(!r.remove(2), "removing twice is false, not a panic");
        assert_eq!(r.len(), 3);
    }

    #[test]
    fn the_zeroed_tail_makes_two_equal_lists_equal() {
        // The invariant stated as the thing it protects: two rosters that
        // hold the same members must be byte-equal, whatever route they
        // took to get there. Without the zeroing in `remove` this fails,
        // and `state_hash` would call two identical worlds different.
        let mut grown = R::of(1);
        grown.add(2);
        grown.add(3);
        grown.remove(3);

        let mut direct = R::of(1);
        direct.add(2);

        assert_eq!(grown, direct);
        assert_eq!(grown.raw(), direct.raw());
    }

    #[test]
    fn reset_to_is_one_operation() {
        let mut r = R::of(1);
        r.add(2);
        r.add(3);
        r.reset_to(7);
        assert_eq!(r.members(), &[7]);
        assert_eq!(r.raw()[1..], [0, 0, 0], "and it clears the tail too");
    }

    #[test]
    fn restore_refuses_a_count_past_the_cap() {
        let ids = [1, 2, 3, 4];
        assert!(R::restore(&ids, 5).is_none(), "clamping would forget one");
        let r = R::restore(&ids, 2).expect("a legal count");
        assert_eq!(r.members(), &[1, 2]);
        assert_eq!(
            r.raw()[2..],
            [0, 0],
            "residue past the count is dropped, not loaded — a saved world \
             must reload to the hash it was saved at"
        );
    }

    #[test]
    fn clear_empties_and_stays_usable() {
        let mut r = R::of(1);
        r.add(2);
        r.clear();
        assert!(r.is_empty());
        assert!(!r.contains(1));
        assert_eq!(r.add(5), Added::New);
        assert_eq!(r.members(), &[5]);
    }
}
