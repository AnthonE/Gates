//! **No one acts faster than a person can** — the per-connection pace on the
//! action lane.
//!
//! The lane alone takes one action per client per tick (`ShardCore::tick`),
//! which is 30 a second: a modified client could eat a whole stack in a
//! second, flip a door thirty times a second (every flip broadcast to every
//! client), plant a stack of charges in a third of one, repair a wall on
//! every tick of a raid, or churn a box down and up until joiners never
//! finish their deployable sync. The swing, the shot, the reload, the craft
//! queue, a revive, the research table and a code lock's lockout already
//! keep their own time in the sim; these kept none.
//!
//! So every action belongs to a [`Kind`], and a kind with a gap may act
//! once per that many ticks — counted on the sim's own tick, never a client
//! clock. **An early action waits in the client's hand** until its gap is
//! up: the lane's contract is *defer, never drop*, so a request still gets
//! its answer, just no sooner than a person could have asked. A script
//! hammering a key queues behind itself and gets the same pace an honest
//! player gets; the honest client keeps the tightest of these (a bite a
//! second) on its own side too (`render::verbs::Bite`), so its presses
//! never wait here at all.
//!
//! Server state, not sim state: nothing here reaches `World`, the hash or a
//! save. A held action is not a command until it goes, so the WAL records
//! exactly what the sim was asked, when.

use protocol::ActionMsg;

/// The groups that share one clock. Two verbs share a kind when they are
/// one gesture to the player: a mouthful is a mouthful whether it was food
/// or the sea.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Kind {
    /// Eat and drink.
    Mouth,
    /// A door, an oven, a research table: `Use`.
    Use,
    Repair,
    /// Plant a charge.
    Throw,
    /// Place, upgrade, rotate: a piece of a base.
    Build,
    /// Take a piece or a deployable back down.
    Demolish,
    /// Put a deployable down.
    Deploy,
    /// Items between slots and containers.
    Move,
    /// Open a container, loot a bag, pick up a stack or an arrow, pick a
    /// bush.
    Take,
    /// Everything the sim already paces itself, or that is harmless at the
    /// lane's speed: craft (a queue of four), research and unlock (one
    /// shot each), access (the lock's own lockout), feed, reload (its own
    /// busy clock), respawn, a revive's press and release.
    Free,
}

/// How many kinds there are — [`Pace`]'s clock array.
pub const KINDS: usize = Kind::Free as usize + 1;

/// Every kind, in discriminant order.
pub const ALL: [Kind; KINDS] = [
    Kind::Mouth,
    Kind::Use,
    Kind::Repair,
    Kind::Throw,
    Kind::Build,
    Kind::Demolish,
    Kind::Deploy,
    Kind::Move,
    Kind::Take,
    Kind::Free,
];

/// The kind an action belongs to.
pub fn kind_of(act: &ActionMsg) -> Kind {
    match act {
        ActionMsg::Consume { .. } | ActionMsg::Drink => Kind::Mouth,
        ActionMsg::Vend { .. } => Kind::Move,
        ActionMsg::Swipe { .. } => Kind::Move,
        // A deposit moves a pack's worth of items, like a trade.
        ActionMsg::Arc { .. } => Kind::Move,
        ActionMsg::Use { .. } => Kind::Use,
        ActionMsg::Repair { .. } => Kind::Repair,
        ActionMsg::Throw { .. } => Kind::Throw,
        ActionMsg::Place { .. } | ActionMsg::Upgrade { .. } | ActionMsg::Rotate { .. } => {
            Kind::Build
        }
        ActionMsg::Demolish { .. } => Kind::Demolish,
        ActionMsg::Deploy { .. } => Kind::Deploy,
        ActionMsg::Move { .. } => Kind::Move,
        ActionMsg::Container { .. }
        | ActionMsg::Loot
        | ActionMsg::Pickup
        | ActionMsg::Pick { .. } => Kind::Take,
        ActionMsg::Craft { .. }
        | ActionMsg::CraftCancel { .. }
        | ActionMsg::Reskin { .. }
        | ActionMsg::SkinsRefresh
        | ActionMsg::Research { .. }
        | ActionMsg::Unlock { .. }
        | ActionMsg::Access { .. }
        | ActionMsg::Feed { .. }
        | ActionMsg::Reload
        | ActionMsg::Respawn { .. }
        | ActionMsg::RespawnGate
        | ActionMsg::Assist { .. } => Kind::Free,
    }
}

/// The gap between two actions of a kind, in sim ticks (`TICK_HZ` = 30).
/// **A person's pace, not a balance knob**: each is under what a fast hand
/// does on purpose, so in practice only a script ever waits.
///
/// - **Mouth, 25 (0.83 s)**: Rust's one second between bites. The client
///   waits the full second itself (`render::verbs::BITE_S`); the sixth of a
///   second of slack is network jitter bunching two honest bites together.
/// - **Use, 8 (0.27 s)**: a door opened and shut as fast as a hand can —
///   and every flip is broadcast to every client.
/// - **Repair, 15 (0.5 s)**: a hammer's pace, where it was one a tick
///   through a raid.
/// - **Throw, 30 (1 s)**: one charge a second, where a stack went down in
///   a third of one and could fill the shard's cap alone.
/// - **Build, Demolish, 5 (6 a second)**: a fast builder's clicking; with
///   **Deploy, 8**, what ends the down-and-up churn that restarted every
///   joiner's deployable sync.
/// - **Take, 3; Move, 2 (15 a second)**: macro looting brought down to a
///   fast hand's speed.
/// - **Free, 0**: the lane's own one a tick.
pub const fn gap(kind: Kind) -> u64 {
    match kind {
        Kind::Mouth => 25,
        Kind::Use => 8,
        Kind::Repair => 15,
        Kind::Throw => 30,
        Kind::Build | Kind::Demolish => 5,
        Kind::Deploy => 8,
        Kind::Take => 3,
        Kind::Move => 2,
        Kind::Free => 0,
    }
}

/// One connection's clocks: the first tick each kind may act again.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pace {
    next: [u64; KINDS],
}

impl Pace {
    /// May the action in the hand go at `tick`? Starts its kind's clock
    /// when it does; one that may not stays in the hand and asks again, and
    /// the wait does not restart the clock, so it goes the moment it can.
    pub fn go(&mut self, act: &ActionMsg, tick: u64) -> bool {
        let kind = kind_of(act);
        let at = &mut self.next[kind as usize];
        if tick < *at {
            return false;
        }
        *at = tick + gap(kind);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EAT: ActionMsg = ActionMsg::Consume { slot: 2 };

    /// **A stack is not eaten in a second.** A script asking every tick —
    /// the lane's whole speed — eats about once a second.
    #[test]
    fn a_script_eating_every_tick_eats_about_once_a_second() {
        let mut p = Pace::default();
        let eaten = (0..30 * 10u64).filter(|&t| p.go(&EAT, t)).count();
        assert!((10..=13).contains(&eaten), "{eaten} bites in ten seconds");
    }

    /// An early bite waits and then goes — it is not lost — and the sea is
    /// a mouthful on the same clock.
    #[test]
    fn an_early_bite_waits_for_its_second() {
        let mut p = Pace::default();
        assert!(p.go(&EAT, 100));
        for t in 101..125 {
            assert!(!p.go(&EAT, t), "tick {t}");
            assert!(!p.go(&ActionMsg::Drink, t), "a gulp at tick {t}");
        }
        assert!(p.go(&EAT, 125), "25 ticks on");
    }

    /// Kinds keep separate clocks: a bite does not hold up a door, and a
    /// door flip-flop waits.
    #[test]
    fn one_kind_does_not_wait_on_another() {
        let door = ActionMsg::Use {
            cx: 1,
            cz: 1,
            level: 0,
            loc: 0,
        };
        let mut p = Pace::default();
        assert!(p.go(&EAT, 50));
        assert!(p.go(&door, 51));
        assert!(!p.go(&door, 52), "a door flip-flop");
        assert!(p.go(&ActionMsg::Reload, 52), "a free kind never waits");
        assert!(p.go(&ActionMsg::Reload, 53));
    }

    /// Every gap is a person's pace — at most a second — and an honest
    /// second between bites never waits.
    #[test]
    fn every_gap_is_a_persons_pace() {
        for (i, &kind) in ALL.iter().enumerate() {
            assert_eq!(kind as usize, i, "`ALL` is out of discriminant order");
            assert!(gap(kind) <= 30, "{kind:?} waits {} ticks", gap(kind));
        }
        assert!(gap(Kind::Mouth) < 30, "an honest second must never wait");
    }
}
