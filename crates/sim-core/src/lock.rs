//! Code locks — **who is allowed through a door** (lock v1,
//! `reference/DOORS.md`, DECISIONS.md §open).
//!
//! ## What this replaces
//!
//! lock v0 was one bool and one owner id on `DeployRec`: a door was born
//! locked to the hand that placed it and answered to nobody else, ever. It
//! shipped with its own open question written into `DECISIONS.md` — *"no
//! shared access this slice… whether sharing arrives as a code lock, a
//! hearth auth list that doors inherit, or crews is the open question, and
//! it is the operator's to answer"*. The operator answered it on 2026-08-08
//! by pointing at the reference game, and `reference/DOORS.md` is what came
//! back. This module is §9 of that document, built.
//!
//! Three structural facts drive the whole design, and all three are read
//! off the reference's own class table (`DOORS.md` §1):
//!
//! 1. **The lock is a separate thing from the door.** So it lives here, in
//!    a dense side-store keyed by the door's grid address, exactly as
//!    `HearthRec` and `BoxRec` are keyed by theirs. A door *has* a lock or
//!    has none, and having none is a real state rather than an odd one.
//! 2. **A door with no lock is a door anyone may work.** So a door now
//!    places bare, and the lock is an item somebody paid for. The security
//!    is what costs, not the leaf — which is also the reference's economy
//!    (`DOORS.md` §9.2).
//! 3. **The lock is asked on open *and* on close** — one predicate, two
//!    calls, on both of the reference's lock classes. Ours is
//!    [`Locks::grant`], asked once by `deploy::use_door`'s toggle, which
//!    gets fact 3 for free and must keep getting it for free.
//!
//! ## The access model
//!
//! A lock carries a **main code** and an optional **guest code**, each four
//! digits (0..=9999), and two remembered lists. Entering a code does not
//! open the door — it *authorizes you*, and thereafter the door simply
//! works for you (`DOORS.md` §2.2: the single most important mechanic in
//! that document). Full rights come from the main code and from having
//! bolted the lock on; guest rights come from the guest code and are the
//! door verb and nothing else — no code changes, no unlocking, no taking
//! the lock off (Devblog 149's own list).
//!
//! **Setting the main code clears both lists.** It is the only eviction
//! there is, and the reference's own wiki sentence says access lasts
//! "until the code is changed or the lock is removed". The setter is
//! re-remembered in the same call, so nobody can lock themselves out with
//! the verb whose whole purpose is control.
//!
//! ## Wrong codes: a shock, then a shut door
//!
//! 10 000 combinations with unlimited attempts is a 10 000-press door, and
//! the reference needed two goes at this over six years (`DOORS.md` §6.4).
//! We ship both at once: a wrong entry costs [`SHOCK_STEP_HP`] × the
//! consecutive-miss count, the streak forgets itself after
//! [`SHOCK_RESET_TICKS`], and [`LOCKOUT_TRIES`] misses shut the keypad for
//! [`LOCKOUT_TICKS`].
//!
//! **The shock never kills** — it floors at 1 hp. That is a deliberate
//! divergence and the reason is structural rather than kind: this crate
//! keeps one kill site per module that can kill (`combat`'s and
//! `survival::died_by_the_world`), because a death is a count, an event,
//! a backpack and a respawn, and a third site is a third chance for those
//! four to disagree. A door that maims is a deterrent; a door that kills
//! is a third kill site.
//!
//! The counters live **on the lock**, not per player per lock: the door is
//! what is under attack, and a per-player table is unbounded (§9.6).
//!
//! ## The laws this module is held to
//!
//! Pure, like the rest of the crate — no clock (the tick is passed in), no
//! I/O, no allocation, no `HashMap`. Every list is a fixed array with a
//! cap in `limits.rs` and a stated overflow policy, and both refusals are
//! events rather than silent drops. The store is scanned linearly like
//! every other store here: `MAX_LOCKS` is 512 and a lock op is a keypress,
//! not a per-tick sweep.

use crate::craft::inv_count;
use crate::deploy::{
    ACCESS_OP_ENTER, ACCESS_OP_LOCK, ACCESS_OP_MAX, ACCESS_OP_SET_CODE, ACCESS_OP_SET_GUEST,
    ACCESS_OP_TAKE, ACCESS_OP_UNLOCK,
};
use crate::gather::{inv_add, ItemStack};
use crate::limits::{INV_SLOTS, LOCK_AUTH_CAP, LOCK_GUEST_CAP, MAX_LOCKS};
use crate::roster::{Added, Roster};

/// The full-rights list's type, named once so `worldsave` and the hash can
/// spell it without restating the cap.
pub type AuthList = Roster<LOCK_AUTH_CAP>;
/// The guest list's type.
pub type GuestList = Roster<LOCK_GUEST_CAP>;

/// No code set. `u16::MAX` rather than a second bool, and out of the four
/// digit range by three orders of magnitude, so a decoder that lets a
/// forged value through cannot forge *this* one into a match.
pub const CODE_NONE: u16 = u16::MAX;
/// The highest four-digit code. The space is 0000..=9999, and the wire
/// range-checks against this before the sim ever sees a value.
pub const CODE_MAX: u16 = 9_999;

/// What a hand may do at a lock. Ordered by rights, so `>=` is the test.
pub const GRANT_NONE: u8 = 0;
/// Entered the guest code: works the door, and nothing else.
pub const GRANT_GUEST: u8 = 1;
/// Bolted it on, or entered the main code: everything.
pub const GRANT_FULL: u8 = 2;

/// Hp a wrong code costs, per consecutive miss: the first is one of these,
/// the second two, the third three. The reference's own schedule
/// (`DOORS.md` §2.2: steps of 5 starting at 5). Proposed default,
/// DECISIONS.md §open (lock v1).
pub const SHOCK_STEP_HP: u16 = 5;
/// Ticks of no wrong entry that forgive the escalation — 10 s at the 30 Hz
/// tick, the reference's own window. The *lockout* streak does not reset
/// here (see [`LOCKOUT_TRIES`]); only the damage does, which is the
/// reference's split too.
pub const SHOCK_RESET_TICKS: u64 = 300;
/// Wrong entries that shut the keypad. The reference took six years to
/// arrive at eight (`DOORS.md` §6.4) and we start there. Proposed default,
/// DECISIONS.md §open (lock v1).
pub const LOCKOUT_TRIES: u8 = 8;
/// How long the keypad stays shut — 15 min at the 30 Hz tick, the
/// reference's own number. Proposed default, DECISIONS.md §open (lock v1).
pub const LOCKOUT_TICKS: u64 = 27_000;

/// One code lock, bolted to the deployable at its grid address.
///
/// The address is the identity, exactly as it is for a hearth and a box:
/// a lock is furniture on furniture and has no id to hand out. `loc` is
/// carried (unlike `BoxRec`, which can drop it) because a door lives at an
/// *edge* address and two doors can share a cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LockRec {
    pub cx: u16,
    pub cz: u16,
    pub level: u8,
    pub loc: u8,
    /// Who bolted it on. Sim-side only and **not** the access check —
    /// `auth` is, and the owner is simply its first member. Kept because
    /// somebody has to be the one the lock came from.
    pub owner: u32,
    /// The main code, or [`CODE_NONE`]. A lock with no code cannot be
    /// locked: `set_code` is what arms it.
    pub code: u16,
    /// The guest code, or [`CODE_NONE`].
    pub guest_code: u16,
    /// Whether the leaf answers to the lists at all. A fresh lock is
    /// **unlocked** — bolting one on claims nothing until a code is set,
    /// which is the reference's sequence (place, then it asks for a code).
    pub locked: bool,
    /// Who the lock remembers at full rights. A [`Roster`], which is the
    /// same component the hearth's crew uses — the reference built one
    /// access list and hung it on three classes, and this is ours doing
    /// the same (`reference/BUILDING.md` §1 fact 1).
    pub auth: AuthList,
    /// Who it remembers as guests: the door verb and nothing else.
    pub guests: GuestList,
    /// Consecutive wrong entries. Feeds both the shock (scaled, forgiven
    /// after [`SHOCK_RESET_TICKS`]) and the lockout (counted to
    /// [`LOCKOUT_TRIES`], forgiven only by a correct code).
    pub misses: u8,
    /// Tick of the last wrong entry — the shock window's start.
    pub last_miss: u64,
    /// First tick the keypad accepts again. 0 means it never shut.
    pub shut_until: u64,
}

impl Default for LockRec {
    fn default() -> Self {
        Self {
            cx: 0,
            cz: 0,
            level: 0,
            loc: 0,
            owner: 0,
            code: CODE_NONE,
            guest_code: CODE_NONE,
            locked: false,
            auth: AuthList::new(),
            guests: GuestList::new(),
            misses: 0,
            last_miss: 0,
            shut_until: 0,
        }
    }
}

impl LockRec {
    /// A lock as it comes out of the box: bolted on by `owner`, no code,
    /// unlocked, and the owner already remembered so the arming press is
    /// not the press that locks them out.
    pub fn fresh(cx: u16, cz: u16, level: u8, loc: u8, owner: u32) -> Self {
        Self {
            cx,
            cz,
            level,
            loc,
            owner,
            auth: AuthList::of(owner),
            ..Self::default()
        }
    }

    /// What `id` may do here. The one predicate — `deploy::use_door` asks
    /// it for the toggle and every op below asks it for its own rights, so
    /// open, close and the keypad can never disagree about who you are.
    pub fn grant(&self, id: u32) -> u8 {
        if self.auth.contains(id) {
            return GRANT_FULL;
        }
        if self.guests.contains(id) {
            return GRANT_GUEST;
        }
        GRANT_NONE
    }

    /// Whether this lock lets `id` work the leaf. An unlocked lock lets
    /// everyone through — that is what unlocking a shop front means.
    pub fn passes(&self, id: u32) -> bool {
        !self.locked || self.grant(id) != GRANT_NONE
    }

    /// Remember `id` at `grant`. Returns false when the list is full,
    /// which is the caller's cue to refuse (never to evict — the
    /// `Roster`'s own rule, and `limits.rs` names the caps).
    fn remember(&mut self, id: u32, grant: u8) -> bool {
        if grant == GRANT_FULL {
            return self.auth.add(id) != Added::Full;
        }
        if self.grant(id) != GRANT_NONE {
            // Already remembered — a full member entering the guest code
            // keeps their rights rather than demoting themselves.
            return true;
        }
        self.guests.add(id) != Added::Full
    }

    /// Forget everyone and remember `keep` alone, at full rights. Setting
    /// the main code is the only thing that calls this, and it is the only
    /// eviction the model has.
    fn reset_lists(&mut self, keep: u32) {
        self.auth.reset_to(keep);
        self.guests.clear();
    }
}

/// The dense lock list. Same shape as the box store and for the same
/// reason: `DeployRec` is what the deploy-sync packet mirrors, so none of
/// this may ride on it.
///
/// The array is boxed and filled on the heap ([`crate::boxed_array`]),
/// which is a measured fix rather than a style — that function's doc has
/// the measurement and the failure it prevents.
#[derive(Clone)]
pub struct Locks {
    pub(crate) entries: Box<[LockRec; MAX_LOCKS]>,
    pub(crate) len: usize,
}

impl Locks {
    pub fn new() -> Self {
        let Ok(entries) = vec![LockRec::default(); MAX_LOCKS]
            .into_boxed_slice()
            .try_into()
        else {
            unreachable!("the vec is MAX_LOCKS long by construction")
        };
        Self { entries, len: 0 }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn entries(&self) -> &[LockRec] {
        &self.entries[..self.len]
    }

    pub fn find_index(&self, cx: u16, cz: u16, level: u8, loc: u8) -> Option<usize> {
        self.entries[..self.len]
            .iter()
            .position(|l| l.cx == cx && l.cz == cz && l.level == level && l.loc == loc)
    }

    pub fn find(&self, cx: u16, cz: u16, level: u8, loc: u8) -> Option<&LockRec> {
        self.find_index(cx, cz, level, loc)
            .map(|i| &self.entries[i])
    }

    /// What the lock at this address (if any) lets `id` do to the leaf.
    /// **A bare address passes** — a door nobody has paid to secure is a
    /// door anyone may work (`DOORS.md` §1 fact 2), and that is why this
    /// returns true rather than refusing on a missing record.
    pub fn passes(&self, cx: u16, cz: u16, level: u8, loc: u8, id: u32) -> bool {
        match self.find(cx, cz, level, loc) {
            None => true,
            Some(l) => l.passes(id),
        }
    }

    /// Whether the leaf at this address should draw as locked — the bit
    /// `DeployRec::locked` mirrors onto the wire. Derived from this store
    /// so the two can only disagree through a missed call, which is what
    /// `World::rebuild_doors` exists to make impossible after a load.
    pub fn is_locked(&self, cx: u16, cz: u16, level: u8, loc: u8) -> bool {
        self.find(cx, cz, level, loc).is_some_and(|l| l.locked)
    }

    /// Bolt `rec` on. False when the store is full (the caller refuses).
    pub fn insert(&mut self, rec: LockRec) -> bool {
        if self.len == MAX_LOCKS {
            return false;
        }
        self.entries[self.len] = rec;
        self.len += 1;
        true
    }

    /// Swap-remove, the idiom every dense store here uses.
    pub fn remove_at(&mut self, i: usize) {
        self.len -= 1;
        self.entries[i] = self.entries[self.len];
        self.entries[self.len] = LockRec::default();
    }

    /// Take the lock off the deployable at this address, if one is there.
    /// The removal path calls it so a door taken by decay or a raid does
    /// not leave its lock behind — the reference's rule that a lock dies
    /// with what it is bolted to (`DOORS.md` §2.2).
    pub fn remove_at_address(&mut self, cx: u16, cz: u16, level: u8, loc: u8) -> bool {
        match self.find_index(cx, cz, level, loc) {
            Some(i) => {
                self.remove_at(i);
                true
            }
            None => false,
        }
    }
}

impl Default for Locks {
    fn default() -> Self {
        Self::new()
    }
}

/// What one lock op did, for the caller to turn into events. The verb
/// itself pushes nothing: `deploy.rs` owns the refusal codes and the
/// announcement shape, and a second module writing `EV_DEPLOY_REFUSED`
/// would be a second place to get the reason wrong.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The op landed. `relock` is the door's locked bit afterwards, for
    /// the mirror on `DeployRec`.
    Done { relock: bool },
    /// The op landed and the lock came off; the caller returns the item
    /// and re-announces the (now bare) door.
    Removed,
    /// A correct code: `grant` is what the sender now holds.
    Authorized { grant: u8 },
    /// A wrong code. `shock` hp has already been taken off the sender;
    /// `shut` is true when this miss was the one that shut the keypad.
    Wrong { shock: u16, shut: bool },
    /// The keypad is shut and this entry was not even scored.
    LockedOut,
    /// The sender's rights are not enough for this op.
    Denied,
    /// A remembered list is full — refuse, never evict.
    ListFull,
    /// The op or its code is out of range. The wire checks first, so this
    /// only fires for a command the sim was handed directly (a replay of a
    /// forged frame, a test).
    BadOp,
}

/// Apply one lock op to the record at `i`. Pure over the store and
/// nothing else — the shock, every event, the item and both mirror bits
/// are the caller's (`deploy::lock_op`), and this module never learns
/// what a player is beyond an id. `Outcome::Wrong` carries the damage
/// rather than taking it, which is what keeps the one hp write in the
/// one place that already owns `EV_HEALTH`.
pub fn apply(locks: &mut Locks, i: usize, id: u32, op: u8, code: u16, tick: u64) -> Outcome {
    if op > ACCESS_OP_MAX {
        return Outcome::BadOp;
    }
    let needs_code = op == ACCESS_OP_SET_CODE || op == ACCESS_OP_ENTER;
    let code_ok = code <= CODE_MAX || (op == ACCESS_OP_SET_GUEST && code == CODE_NONE);
    if (needs_code || op == ACCESS_OP_SET_GUEST) && !code_ok {
        return Outcome::BadOp;
    }
    let grant = locks.entries[i].grant(id);
    match op {
        ACCESS_OP_ENTER => enter(locks, i, id, code, tick),
        // Everything else is a full-rights op. One test, once, rather
        // than five copies of it — a guest is refused by the same branch
        // whether they reached for the code, the guest code, the lock bit
        // or the lock itself (`DOORS.md` §2.2's own list).
        _ if grant != GRANT_FULL => Outcome::Denied,
        ACCESS_OP_SET_CODE => {
            let l = &mut locks.entries[i];
            l.code = code;
            l.guest_code = CODE_NONE;
            l.locked = true;
            l.misses = 0;
            l.shut_until = 0;
            l.reset_lists(id);
            Outcome::Done { relock: true }
        }
        ACCESS_OP_SET_GUEST => {
            let l = &mut locks.entries[i];
            l.guest_code = code;
            l.guests.clear();
            Outcome::Done { relock: l.locked }
        }
        ACCESS_OP_LOCK => {
            let l = &mut locks.entries[i];
            if l.code == CODE_NONE {
                // Nothing to lock *with*. Refusing rather than locking is
                // what keeps `set_code` the only way a door becomes shut,
                // and stops a lock arming itself with a code nobody knows.
                return Outcome::Denied;
            }
            l.locked = true;
            Outcome::Done { relock: true }
        }
        ACCESS_OP_UNLOCK => {
            locks.entries[i].locked = false;
            Outcome::Done { relock: false }
        }
        ACCESS_OP_TAKE => {
            locks.remove_at(i);
            Outcome::Removed
        }
        _ => Outcome::BadOp,
    }
}

/// A code entry. Split out because it is the only op with arithmetic in
/// it, and the only one that can hurt the sender.
fn enter(locks: &mut Locks, i: usize, id: u32, code: u16, tick: u64) -> Outcome {
    let l = &mut locks.entries[i];
    if tick < l.shut_until {
        return Outcome::LockedOut;
    }
    if l.shut_until != 0 {
        // The shutter ran out: the streak that shut it is spent, so the
        // next eight misses are a fresh ladder rather than an instant
        // re-shut.
        l.shut_until = 0;
        l.misses = 0;
    }
    let want = if code != CODE_NONE && code == l.code {
        GRANT_FULL
    } else if code != CODE_NONE && code == l.guest_code {
        GRANT_GUEST
    } else {
        GRANT_NONE
    };
    if want == GRANT_NONE {
        // Forgive the *damage* ladder after the quiet window, never the
        // lockout count — the reference's own split, and the reason a
        // patient brute force is still only eight presses.
        let step = if tick.saturating_sub(l.last_miss) > SHOCK_RESET_TICKS {
            1
        } else {
            (l.misses as u16).saturating_add(1)
        };
        l.misses = l.misses.saturating_add(1);
        l.last_miss = tick;
        let shut = l.misses >= LOCKOUT_TRIES;
        if shut {
            l.shut_until = tick.saturating_add(LOCKOUT_TICKS);
        }
        return Outcome::Wrong {
            shock: SHOCK_STEP_HP.saturating_mul(step),
            shut,
        };
    }
    if !l.remember(id, want) {
        return Outcome::ListFull;
    }
    l.misses = 0;
    Outcome::Authorized { grant: want }
}

/// Take a shock off a body without ever taking the last point. The floor
/// is the whole reason this is a function and not a subtraction: see the
/// module header — a door may maim, and a third kill site is worse than
/// the divergence.
pub fn shock(hp: &mut u16, amount: u16) -> u16 {
    let took = amount.min(hp.saturating_sub(1));
    *hp -= took;
    took
}

/// Give the lock item back to a hand that took its lock off. Returns what
/// would not fit, which the caller drops on the floor — the same posture
/// every other give in this crate takes toward a full inventory.
pub fn give_back(inv: &mut [ItemStack; INV_SLOTS], item: u16, stack_max: u16) -> u16 {
    inv_add(inv, item, 1, stack_max)
}

/// Whether a hand is holding at least one of `item` — the placement cost
/// check, re-exported here so `deploy.rs`'s lock branch reads in one file.
pub fn holds(inv: &[ItemStack; INV_SLOTS], item: u16) -> bool {
    inv_count(inv, item) >= 1
}

#[cfg(test)]
mod tests {
    use super::*;

    const OWNER: u32 = 7;
    const FRIEND: u32 = 9;
    const RAIDER: u32 = 11;

    fn armed() -> Locks {
        let mut locks = Locks::new();
        assert!(locks.insert(LockRec::fresh(4, 5, 0, 1, OWNER)));
        assert_eq!(
            apply(&mut locks, 0, OWNER, ACCESS_OP_SET_CODE, 1234, 0),
            Outcome::Done { relock: true }
        );
        locks
    }

    #[test]
    fn a_bare_address_passes_and_a_fresh_lock_does_too() {
        let mut locks = Locks::new();
        assert!(
            locks.passes(4, 5, 0, 1, RAIDER),
            "a door nobody secured is anyone's (DOORS.md §1 fact 2)"
        );
        locks.insert(LockRec::fresh(4, 5, 0, 1, OWNER));
        assert!(
            locks.passes(4, 5, 0, 1, RAIDER),
            "a lock with no code is not a locked door — arming it is set_code"
        );
        assert!(!locks.is_locked(4, 5, 0, 1));
    }

    #[test]
    fn the_code_authorizes_the_player_it_does_not_open_the_door() {
        let mut locks = armed();
        assert!(locks.is_locked(4, 5, 0, 1));
        assert!(!locks.passes(4, 5, 0, 1, FRIEND));
        assert_eq!(
            apply(&mut locks, 0, FRIEND, ACCESS_OP_ENTER, 1234, 10),
            Outcome::Authorized { grant: GRANT_FULL }
        );
        assert!(
            locks.passes(4, 5, 0, 1, FRIEND),
            "and thereafter the door simply works — the memory is the mechanic"
        );
    }

    #[test]
    fn a_guest_works_the_door_and_nothing_else() {
        let mut locks = armed();
        apply(&mut locks, 0, OWNER, ACCESS_OP_SET_GUEST, 4321, 10);
        assert_eq!(
            apply(&mut locks, 0, FRIEND, ACCESS_OP_ENTER, 4321, 11),
            Outcome::Authorized { grant: GRANT_GUEST }
        );
        assert!(locks.passes(4, 5, 0, 1, FRIEND), "a guest opens the door");
        for op in [
            ACCESS_OP_SET_CODE,
            ACCESS_OP_SET_GUEST,
            ACCESS_OP_LOCK,
            ACCESS_OP_UNLOCK,
            ACCESS_OP_TAKE,
        ] {
            assert_eq!(
                apply(&mut locks, 0, FRIEND, op, 1111, 12),
                Outcome::Denied,
                "op {op} is a full-rights op (Devblog 149's own list)"
            );
        }
        assert_eq!(locks.len(), 1, "and the lock is still on the door");
    }

    #[test]
    fn setting_the_code_is_the_eviction() {
        let mut locks = armed();
        apply(&mut locks, 0, FRIEND, ACCESS_OP_ENTER, 1234, 10);
        apply(&mut locks, 0, OWNER, ACCESS_OP_SET_GUEST, 4321, 10);
        apply(&mut locks, 0, RAIDER, ACCESS_OP_ENTER, 4321, 10);
        assert_eq!(locks.entries[0].auth.len(), 2);
        assert_eq!(locks.entries[0].guests.len(), 1);

        apply(&mut locks, 0, FRIEND, ACCESS_OP_SET_CODE, 5678, 20);
        let l = &locks.entries[0];
        assert_eq!(
            (l.auth.len(), l.guests.len()),
            (1, 0),
            "both lists forgotten"
        );
        assert_eq!(
            l.auth.members(),
            &[FRIEND],
            "and the setter is remembered, not locked out"
        );
        assert_eq!(l.guest_code, CODE_NONE, "the guest code goes with its list");
        assert!(!locks.passes(4, 5, 0, 1, OWNER), "even the owner is out");
    }

    #[test]
    fn the_shock_escalates_forgives_and_never_kills() {
        let mut locks = armed();
        // Three misses inside the window: 5, 10, 15.
        for (n, expect) in [(0u64, 5u16), (1, 10), (2, 15)] {
            assert_eq!(
                apply(&mut locks, 0, RAIDER, ACCESS_OP_ENTER, 9, n),
                Outcome::Wrong {
                    shock: expect,
                    shut: false
                }
            );
        }
        // Quiet longer than the window: the ladder starts over.
        let t = 2 + SHOCK_RESET_TICKS + 1;
        assert_eq!(
            apply(&mut locks, 0, RAIDER, ACCESS_OP_ENTER, 9, t),
            Outcome::Wrong {
                shock: 5,
                shut: false
            },
            "the damage ladder forgives after the quiet window"
        );
        // ...but the lockout count did not forgive: four misses so far,
        // four more shut it.
        let mut last = Outcome::BadOp;
        for k in 1..=4 {
            last = apply(&mut locks, 0, RAIDER, ACCESS_OP_ENTER, 9, t + k);
        }
        assert!(
            matches!(last, Outcome::Wrong { shut: true, .. }),
            "eight misses shut the keypad, quiet windows or not"
        );
        assert_eq!(
            apply(&mut locks, 0, RAIDER, ACCESS_OP_ENTER, 1234, t + 5),
            Outcome::LockedOut,
            "and a *correct* code is refused while it is shut"
        );

        // The floor. A body at 1 hp cannot be finished by a keypad.
        let mut low = 1u16;
        assert_eq!(shock(&mut low, 40), 0);
        assert_eq!(low, 1);
        let mut some = 12u16;
        assert_eq!(shock(&mut some, 40), 11);
        assert_eq!(some, 1);
    }

    #[test]
    fn the_shutter_lifts_and_the_ladder_is_fresh() {
        let mut locks = armed();
        for k in 0..LOCKOUT_TRIES as u64 {
            apply(&mut locks, 0, RAIDER, ACCESS_OP_ENTER, 9, k);
        }
        let after = locks.entries[0].shut_until;
        assert_eq!(
            apply(&mut locks, 0, RAIDER, ACCESS_OP_ENTER, 1234, after),
            Outcome::Authorized { grant: GRANT_FULL },
            "the shutter lifts on its own tick"
        );
        assert_eq!(
            locks.entries[0].misses, 0,
            "and a correct code clears the streak"
        );
    }

    #[test]
    fn a_full_list_refuses_and_never_evicts() {
        let mut locks = armed();
        // The owner already holds slot 0, so CAP-1 more fill it.
        for k in 0..LOCK_AUTH_CAP as u32 - 1 {
            assert_eq!(
                apply(&mut locks, 0, 100 + k, ACCESS_OP_ENTER, 1234, 1),
                Outcome::Authorized { grant: GRANT_FULL }
            );
        }
        assert_eq!(
            apply(&mut locks, 0, 999, ACCESS_OP_ENTER, 1234, 1),
            Outcome::ListFull
        );
        assert_eq!(
            locks.entries[0].auth.members()[0],
            OWNER,
            "the owner is still first — a full list never forgets, it refuses"
        );
        assert!(!locks.passes(4, 5, 0, 1, 999));
    }

    #[test]
    fn unlock_opens_the_door_to_everyone_and_lock_needs_a_code() {
        let mut locks = armed();
        apply(&mut locks, 0, OWNER, ACCESS_OP_UNLOCK, 0, 1);
        assert!(
            locks.passes(4, 5, 0, 1, RAIDER),
            "an unlocked lock is a shop front"
        );
        assert!(!locks.is_locked(4, 5, 0, 1));
        assert_eq!(
            apply(&mut locks, 0, OWNER, ACCESS_OP_LOCK, 0, 2),
            Outcome::Done { relock: true }
        );

        // A bare lock cannot be locked: nothing to lock it with.
        let mut bare = Locks::new();
        bare.insert(LockRec::fresh(1, 1, 0, 1, OWNER));
        assert_eq!(
            apply(&mut bare, 0, OWNER, ACCESS_OP_LOCK, 0, 0),
            Outcome::Denied
        );
    }

    #[test]
    fn take_removes_it_and_the_door_is_anyones_again() {
        let mut locks = armed();
        assert_eq!(
            apply(&mut locks, 0, OWNER, ACCESS_OP_TAKE, 0, 1),
            Outcome::Removed
        );
        assert_eq!(locks.len(), 0);
        assert!(locks.passes(4, 5, 0, 1, RAIDER));
    }

    #[test]
    fn a_forged_op_or_code_is_refused_not_clamped() {
        let mut locks = armed();
        assert_eq!(
            apply(&mut locks, 0, OWNER, ACCESS_OP_MAX + 1, 0, 1),
            Outcome::BadOp
        );
        assert_eq!(
            apply(&mut locks, 0, OWNER, ACCESS_OP_SET_CODE, CODE_MAX + 1, 1),
            Outcome::BadOp
        );
        assert_eq!(
            locks.entries[0].code, 1234,
            "a refused op leaves the record exactly where it was"
        );
    }

    #[test]
    fn the_store_removes_by_address_and_swaps() {
        let mut locks = Locks::new();
        locks.insert(LockRec::fresh(1, 1, 0, 1, OWNER));
        locks.insert(LockRec::fresh(2, 2, 0, 2, FRIEND));
        assert!(locks.remove_at_address(1, 1, 0, 1));
        assert_eq!(locks.len(), 1);
        assert_eq!(
            locks.entries()[0].owner,
            FRIEND,
            "swap-remove kept the other"
        );
        assert!(!locks.remove_at_address(1, 1, 0, 1), "and it is gone");
    }
}
