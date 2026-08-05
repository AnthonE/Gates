//! `test_inventory_move` — the walls under the move verb.
//!
//! `CLAUDE.md` names this verb the most bug-prone thing in the reference
//! and names how it fails: **as the server disconnecting the client**.
//! Three Oxide fixes in twenty-eight minutes on one 2019 day, the third
//! titled as a fix of the fix, all one-line splice-point moves on
//! move/stack/loot, all landing as a dropped session because container
//! state diverged and a request computed against a container the server
//! disagreed about read as a forged one. The bug is validation ordering
//! against the mutation, never arithmetic.
//!
//! So the suite is not a set of examples. Three of these tests are
//! **properties driven over pseudo-random states and pseudo-random
//! commands**, because the defect class is "some input nobody wrote an
//! example for", and an example suite is exactly what the reference had:
//!
//! 1. `refused_moves_never_mutate` — on every refusal, both containers are
//!    bit-identical to what they were. This is the ordering wall. It can
//!    only be passed by validating before writing, and `inventory.rs` is
//!    built so that is the only thing a caller *can* do (the planner takes
//!    `ItemStack` by value and holds no `&mut`) — this asserts the shape
//!    actually delivers it.
//! 2. `moves_conserve_every_item` — the total of each item id across the
//!    player and every bag is invariant across any move, accepted or
//!    refused. **The anti-dupe wall**, and the one that would have caught
//!    the 2019 day: a splice point that mutates then bails duplicates or
//!    destroys the stack it was holding, and a conservation sum sees that
//!    whatever line it happened on.
//! 3. `every_move_answers_and_none_panics` — every command shape, forged
//!    or not, produces exactly one event and never a panic. This is the
//!    anti-disconnect wall: the sim's answer to a nonsense move is a
//!    sentence, not a dropped session.
//!
//! The bounds are sim ticks and iteration counts — deterministic state,
//! never a clock (`CLAUDE.md`).

use sim_core::backpack::BackpackContent;
use sim_core::gather::{GatherContent, ItemStack};
use sim_core::inventory::{
    plan_move, resolve, MovePlan, CONT_BAG, CONT_MAX, CONT_SELF, REFUSE_M_COUNT, REFUSE_M_EMPTY,
    REFUSE_M_MAX, REFUSE_M_NO_CONTAINER, REFUSE_M_NO_ROOM, REFUSE_M_REACH, REFUSE_M_SLOT,
    REFUSE_M_UNSTACKABLE,
};
use sim_core::limits::{INV_SLOTS, MAX_ITEM_DEFS};
use sim_core::rng::Pcg32;
use sim_core::world::{Command, World, EV_MOVED, EV_MOVE_REFUSED};

const SEED: u64 = 0x5EED_C011_ED17;
const PLAYER: u32 = 7;
/// `GatherContent::probe_fixture` arms items 0..8 with `stack_max` 100.
const STACK_MAX: u16 = 100;
const ITEMS: u16 = 8;

/// A world with one live player, the gather ladder installed (so
/// `stack_max_of` answers) and the backpack module armed (so a bag can
/// exist to move into).
fn move_world() -> World {
    let mut w = World::new(SEED);
    w.gather = GatherContent::probe_fixture();
    // The backpack module armed, with the despawn ladder pushed past this
    // suite's whole tick budget. Each property below spends one tick per
    // iteration and runs thousands, so the stock probe ladder retires the
    // fixture bag mid-run — which is correct behaviour and a ruinous
    // *fixture*: a conservation property would then be reading a legitimate
    // despawn as a leak, and would fail for a reason that has nothing to do
    // with the verb under test. The ladder itself is `backpack.rs`'s gate,
    // not this file's.
    let mut bc = BackpackContent::probe_fixture();
    bc.despawn_ticks = [0; MAX_ITEM_DEFS];
    bc.base_ticks = 1 << 30;
    w.backpack = bc;
    w.dev_spawn = Some(w.spawn_pos(PLAYER));
    w.tick(&[Command::Join { id: PLAYER }]);
    w
}

/// Put a bag on the ground exactly where the player is standing, holding
/// `stacks`. Returns its id.
///
/// Built by dropping a scratch body's inventory rather than by reaching
/// into the store: `drop_for` is the only route a bag reaches the world by
/// in play, so a fixture that took a different one could pass while the
/// real one was broken.
fn bag_at_feet(w: &mut World, stacks: &[(u16, u16)]) -> u32 {
    let mut body = w.players[0];
    body.inv = [ItemStack::default(); INV_SLOTS];
    for (i, &(item, count)) in stacks.iter().enumerate() {
        body.inv[i] = ItemStack { item, count };
    }
    let tick = w.tick;
    w.backpacks
        .drop_for(&w.backpack, &body, tick, &mut w.events)
        .expect("the probe ladder is armed and the body is carrying something")
}

/// Every item id's total across the player's inventory and every bag in
/// the store. The conservation quantity — nothing a move does may change
/// any entry of it.
fn ledger(w: &World) -> [u64; MAX_ITEM_DEFS] {
    let mut total = [0u64; MAX_ITEM_DEFS];
    for s in w.players[0].inv.iter() {
        if s.count > 0 && (s.item as usize) < MAX_ITEM_DEFS {
            total[s.item as usize] += s.count as u64;
        }
    }
    for b in w.backpacks.entries() {
        for s in b.items.iter() {
            if s.count > 0 && (s.item as usize) < MAX_ITEM_DEFS {
                total[s.item as usize] += s.count as u64;
            }
        }
    }
    total
}

/// One slot of one container, read the way the verb reads it.
fn stack_at(w: &World, kind: u8, slot: u8) -> ItemStack {
    if kind == CONT_BAG {
        w.backpacks.slot(0, slot as usize)
    } else {
        w.players[0].inv[slot as usize]
    }
}

/// The first occupied slot at or after `start`, wrapping — or `start`
/// itself when the container is empty. Deterministic: a pure function of
/// world state and the index handed in, so the fuzz stays replayable.
fn occupied_slot(w: &World, kind: u8, start: usize) -> u8 {
    for i in 0..INV_SLOTS {
        let s = (start + i) % INV_SLOTS;
        let stack = if kind == CONT_BAG {
            w.backpacks.slot(0, s)
        } else {
            w.players[0].inv[s]
        };
        if stack.count > 0 {
            return s as u8;
        }
    }
    start as u8
}

/// The player's inventory and every bag's, as one comparable value.
fn snapshot(w: &World) -> (Vec<ItemStack>, Vec<ItemStack>) {
    let inv = w.players[0].inv.to_vec();
    let bags = w
        .backpacks
        .entries()
        .iter()
        .flat_map(|b| b.items.iter().copied())
        .collect();
    (inv, bags)
}

/// Run one move and return the single event it produced: `(code, b, c)`.
///
/// Asserts *exactly one* move event on the tick, which makes this a
/// double-emit gate as well — an emit site that announced twice would be
/// two rollback instructions for one drag on the client.
#[allow(clippy::too_many_arguments)]
fn do_move(
    w: &mut World,
    cont: u32,
    from_kind: u8,
    from_slot: u8,
    to_kind: u8,
    to_slot: u8,
    count: u16,
) -> (u8, u32, u32) {
    w.tick(&[Command::Move {
        id: PLAYER,
        cont,
        from_kind,
        from_slot,
        to_kind,
        to_slot,
        count,
    }]);
    let found: Vec<_> = w
        .events
        .entries()
        .iter()
        .filter(|e| e.code == EV_MOVED || e.code == EV_MOVE_REFUSED)
        .map(|e| (e.code, e.b, e.c))
        .collect();
    assert_eq!(
        found.len(),
        1,
        "a move must answer exactly once — got {} events for \
         ({from_kind}:{from_slot} -> {to_kind}:{to_slot} x{count}, container {cont})",
        found.len()
    );
    found[0]
}

// ---------------------------------------------------------------------------
// The three properties
// ---------------------------------------------------------------------------

/// **The ordering wall.** A refused move leaves both containers exactly as
/// it found them — every byte, not just the two slots it named.
///
/// A "validate, mutate, notice a problem, bail" splice point passes an
/// example suite and fails here on the first state where the bail is
/// reachable, because this compares the whole of both containers rather
/// than the pair the command mentioned.
#[test]
fn refused_moves_never_mutate() {
    let mut w = move_world();
    let mut rng = Pcg32::new(SEED, 1);
    let bag = bag_at_feet(&mut w, &[(1, 40), (3, 90), (5, 7)]);
    for s in 0..6 {
        w.players[0].inv[s] = ItemStack {
            item: (s as u16) % ITEMS,
            count: (10 + s as u16 * 13) % STACK_MAX + 1,
        };
    }

    let mut refusals = 0usize;
    let mut seen = [false; (REFUSE_M_MAX + 1) as usize];
    for _ in 0..4_000 {
        // Deliberately past the live ranges on both axes: forged kinds,
        // forged slots, a stale bag id and counts past any stack are the
        // inputs the refusal paths exist for.
        let from_kind = (rng.next_bounded(CONT_MAX as u32 + 2)) as u8;
        let to_kind = (rng.next_bounded(CONT_MAX as u32 + 2)) as u8;
        let from_slot = rng.next_bounded(INV_SLOTS as u32 + 2) as u8;
        let to_slot = rng.next_bounded(INV_SLOTS as u32 + 2) as u8;
        let count = rng.next_bounded(STACK_MAX as u32 + 40) as u16;
        let named_bag = if rng.next_bounded(4) == 0 {
            999_999
        } else {
            bag
        };

        let before = snapshot(&w);
        let (code, b, c) = do_move(
            &mut w, named_bag, from_kind, from_slot, to_kind, to_slot, count,
        );
        if code == EV_MOVE_REFUSED {
            refusals += 1;
            assert!(
                (1..=REFUSE_M_MAX).contains(&b),
                "a refusal must name a reason inside the published range, got {b}"
            );
            seen[b as usize] = true;
            assert_eq!(
                snapshot(&w),
                before,
                "a refused move mutated a container: reason {b}, addr {c:#010x}"
            );
        }
    }

    assert!(
        refusals > 100,
        "the fuzz never exercised the refusal paths ({refusals} refusals) — \
         it is asserting nothing"
    );
    // Every published reason must be reachable. A reason that no input can
    // produce is a branch nobody has ever run, which is where the next
    // defect hides.
    for reason in [
        REFUSE_M_SLOT,
        REFUSE_M_EMPTY,
        REFUSE_M_COUNT,
        REFUSE_M_NO_ROOM,
        REFUSE_M_NO_CONTAINER,
    ] {
        assert!(
            seen[reason as usize],
            "reason {reason} was never produced by 4000 random moves — \
             either it is unreachable or the fuzz cannot reach it"
        );
    }
}

/// **The anti-dupe wall.** No move, accepted or refused, changes the total
/// count of any item across the player and every bag.
///
/// This is the one that catches the reference's 2019 day whatever line the
/// splice point sat on: a mutation that half-lands either invents items or
/// destroys them, and a per-item sum sees both. It is deliberately blind
/// to *where* the items are — that is the point. Conservation is the
/// invariant a container system either has or does not.
#[test]
fn moves_conserve_every_item() {
    let mut w = move_world();
    let mut rng = Pcg32::new(SEED, 2);
    let bag = bag_at_feet(&mut w, &[(2, 55), (2, 30), (6, 12), (0, 99)]);
    for s in 0..INV_SLOTS {
        if rng.next_bounded(3) == 0 {
            continue; // leave some slots empty — the merge/fill paths need them
        }
        w.players[0].inv[s] = ItemStack {
            item: rng.next_bounded(ITEMS as u32) as u16,
            count: rng.next_bounded(STACK_MAX as u32) as u16 + 1,
        };
    }

    let opening = ledger(&w);
    let mut accepted = 0usize;
    // Ten thousand draws, and the floor below is a tenth of them. The
    // count is set by what makes the *coverage* claim robust rather than
    // by what this seed happens to produce: a floor tuned down to fit one
    // seed's yield is a floor that stops meaning anything the moment the
    // fixture moves. One tick per draw, and the whole test runs in a
    // fraction of a second.
    for _ in 0..10_000 {
        let from_kind = rng.next_bounded(CONT_MAX as u32 + 1) as u8;
        let to_kind = rng.next_bounded(CONT_MAX as u32 + 1) as u8;
        // Source drawn from an *occupied* slot seven times in eight. A
        // uniform draw lands on an empty slot most of the time and refuses
        // before it reaches any arithmetic, which is how the acceptance
        // rate ends up under a tenth — and the accept path is precisely
        // what a conservation property has to walk to prove anything. The
        // remaining eighth stays uniform so the empty-source exit is still
        // sampled here too, and `refused_moves_never_mutate` covers the
        // refusal space properly in any case.
        let from_slot = if rng.next_bounded(8) == 0 {
            rng.next_bounded(INV_SLOTS as u32) as u8
        } else {
            occupied_slot(&w, from_kind, rng.next_bounded(INV_SLOTS as u32) as usize)
        };
        let to_slot = rng.next_bounded(INV_SLOTS as u32) as u8;
        // **Count drawn from the source stack, not independently of it.**
        // Measured: a count drawn blind over a fixed range refuses as
        // `REFUSE_M_COUNT` on 65% of all draws — the source is occupied but
        // smaller than the ask — and the survivors then fragment every
        // stack they split, so both containers saturate and the acceptance
        // rate decays as the run proceeds. That is a sampler that spends
        // ten thousand ticks proving `refused_moves_never_mutate`'s point
        // instead of this one.
        //
        // Half of the well-formed draws take the whole stack, which is what
        // reaches the swap plan and what keeps the fixture from shredding
        // itself; the rest split. One draw in eight is still wild, so the
        // over-the-stack and over-the-ladder exits stay sampled here too.
        let src = stack_at(&w, from_kind, from_slot);
        let count = if rng.next_bounded(8) == 0 {
            rng.next_bounded(STACK_MAX as u32 + 10) as u16
        } else if src.count == 0 {
            1
        } else if rng.next_bounded(2) == 0 {
            src.count
        } else {
            rng.next_bounded(src.count as u32) as u16 + 1
        };
        let (code, _, _) = do_move(&mut w, bag, from_kind, from_slot, to_kind, to_slot, count);
        if code == EV_MOVED {
            accepted += 1;
        }
        // Checked every iteration, not once at the end: a dupe followed by
        // a matching loss would net to zero over a long run and read as
        // clean. The invariant is per-move.
        assert_eq!(
            ledger(&w),
            opening,
            "a move changed the item ledger ({from_kind}:{from_slot} -> \
             {to_kind}:{to_slot} x{count})"
        );
    }
    assert!(
        accepted > 1_000,
        "the fuzz never landed a move ({accepted} accepted) — a conservation \
         proof over nothing but refusals proves nothing"
    );
}

/// **The anti-disconnect wall.** Every shape a `Command::Move` can take —
/// including every forged one — is answered with exactly one event and
/// never panics.
///
/// Exhaustive over the address space (both kinds past their range, both
/// slots past theirs), with a small spread of counts. `do_move`'s
/// exactly-one assertion carries the "answered" half; arriving here at all
/// carries the "never panics" half.
#[test]
fn every_move_answers_and_none_panics() {
    let mut w = move_world();
    let bag = bag_at_feet(&mut w, &[(4, 20)]);
    w.players[0].inv[0] = ItemStack { item: 4, count: 60 };
    w.players[0].inv[1] = ItemStack { item: 1, count: 5 };

    // One past every live bound on all four address axes, so a decoder that
    // is bypassed (a WAL, a bot, a test) still lands on a refusal.
    for from_kind in 0..=(CONT_MAX + 1) {
        for to_kind in 0..=(CONT_MAX + 1) {
            for from_slot in [0u8, 1, 29, INV_SLOTS as u8, 31] {
                for to_slot in [0u8, 2, 29, INV_SLOTS as u8, 31] {
                    for count in [0u16, 1, 60, u16::MAX] {
                        let (code, b, _) =
                            do_move(&mut w, bag, from_kind, from_slot, to_kind, to_slot, count);
                        if code == EV_MOVE_REFUSED {
                            assert!(
                                (1..=REFUSE_M_MAX).contains(&b),
                                "refusal reason {b} is outside the published range"
                            );
                        }
                    }
                }
            }
        }
    }

    // And a bag id that names nothing, from both directions.
    let (code, b, _) = do_move(&mut w, 0, CONT_BAG, 0, CONT_SELF, 3, 1);
    assert_eq!(code, EV_MOVE_REFUSED);
    assert_eq!(b, REFUSE_M_NO_CONTAINER, "a zero bag id names no container");
}

// ---------------------------------------------------------------------------
// What the verb is actually for
// ---------------------------------------------------------------------------

/// Arranging the hotbar: slots `0..HOTBAR_SLOTS` are the first six of the
/// same array, so "choose what you are holding" is this verb and nothing
/// else. Two occupied slots holding different items exchange wholesale.
#[test]
fn a_whole_stack_onto_another_item_swaps() {
    let mut w = move_world();
    w.players[0].inv[0] = ItemStack { item: 1, count: 9 };
    w.players[0].inv[7] = ItemStack { item: 4, count: 3 };

    let (code, b, c) = do_move(&mut w, 0, CONT_SELF, 7, CONT_SELF, 0, 3);
    assert_eq!(code, EV_MOVED);
    assert_eq!(
        b,
        sim_core::inventory::addr(CONT_SELF, 7, CONT_SELF, 0),
        "the ack must carry the address that was asked for"
    );
    assert_eq!(
        c,
        (3u32 << 16) | 4,
        "count << 16 | the item that left `from`"
    );
    assert_eq!(w.players[0].inv[0], ItemStack { item: 4, count: 3 });
    assert_eq!(w.players[0].inv[7], ItemStack { item: 1, count: 9 });
}

/// Splitting a stack into an empty slot, and the emptied source zeroing
/// **both** fields — `gather.rs`'s canonical-empty rule, which is a
/// `state_hash` requirement and not a tidiness one.
#[test]
fn a_partial_move_splits_and_empties_canonically() {
    let mut w = move_world();
    w.players[0].inv[2] = ItemStack { item: 6, count: 30 };

    let (code, _, _) = do_move(&mut w, 0, CONT_SELF, 2, CONT_SELF, 9, 12);
    assert_eq!(code, EV_MOVED);
    assert_eq!(w.players[0].inv[2], ItemStack { item: 6, count: 18 });
    assert_eq!(w.players[0].inv[9], ItemStack { item: 6, count: 12 });

    // Now move the rest: the source must end as a canonical empty.
    let (code, _, _) = do_move(&mut w, 0, CONT_SELF, 2, CONT_SELF, 9, 18);
    assert_eq!(code, EV_MOVED);
    assert_eq!(
        w.players[0].inv[2],
        ItemStack::default(),
        "an emptied slot zeroes item as well as count"
    );
    assert_eq!(w.players[0].inv[9], ItemStack { item: 6, count: 30 });
}

/// **Never partially.** A merge that does not fit entirely is refused, not
/// clamped — the quantize-both-sides law applied to containers. A clamp
/// here is a silent divergence: the client drew a move of 40 and the sim
/// did 25, and nothing on either side ever notices.
#[test]
fn a_merge_that_does_not_fit_is_refused_not_clamped() {
    let mut w = move_world();
    w.players[0].inv[0] = ItemStack {
        item: 3,
        count: STACK_MAX,
    };
    w.players[0].inv[1] = ItemStack {
        item: 3,
        count: STACK_MAX - 10,
    };

    let (code, b, _) = do_move(&mut w, 0, CONT_SELF, 0, CONT_SELF, 1, 40);
    assert_eq!(code, EV_MOVE_REFUSED);
    assert_eq!(b, REFUSE_M_NO_ROOM);
    assert_eq!(
        w.players[0].inv[0].count, STACK_MAX,
        "the refused source is untouched — no partial 10 moved"
    );
    assert_eq!(w.players[0].inv[1].count, STACK_MAX - 10);

    // Exactly the room that is there does land.
    let (code, _, _) = do_move(&mut w, 0, CONT_SELF, 0, CONT_SELF, 1, 10);
    assert_eq!(code, EV_MOVED);
    assert_eq!(w.players[0].inv[0].count, STACK_MAX - 10);
    assert_eq!(w.players[0].inv[1].count, STACK_MAX);
}

/// Withdraw from a bag and deposit back into it — the half of `Loot` that
/// take-everything-that-fits takes away. This is "choose what to carry
/// into a raid and what to leave behind", which is the consequence the
/// judge ranked first.
#[test]
fn a_bag_can_be_taken_from_a_slot_at_a_time_and_put_back() {
    let mut w = move_world();
    let bag = bag_at_feet(&mut w, &[(1, 50), (5, 8)]);
    for s in w.players[0].inv.iter_mut() {
        *s = ItemStack::default();
    }

    // Take twelve of the fifty — not the stack, not the bag.
    let (code, _, c) = do_move(&mut w, bag, CONT_BAG, 0, CONT_SELF, 0, 12);
    assert_eq!(code, EV_MOVED);
    assert_eq!(c, (12u32 << 16) | 1);
    assert_eq!(w.players[0].inv[0], ItemStack { item: 1, count: 12 });
    assert_eq!(w.backpacks.entries()[0].items[0].count, 38);
    assert_eq!(
        w.backpacks.entries()[0].items[1].count,
        8,
        "the other slot is untouched"
    );

    // Put five back.
    let (code, _, _) = do_move(&mut w, bag, CONT_SELF, 0, CONT_BAG, 0, 5);
    assert_eq!(code, EV_MOVED);
    assert_eq!(w.players[0].inv[0], ItemStack { item: 1, count: 7 });
    assert_eq!(w.backpacks.entries()[0].items[0].count, 43);
}

/// A bag emptied by withdrawals leaves the world by the same route
/// `loot_nearest` retires one — so the wire's sync-walk restart contract
/// is keyed to one event whichever verb emptied it.
#[test]
fn a_bag_emptied_by_moves_is_retired() {
    let mut w = move_world();
    let bag = bag_at_feet(&mut w, &[(2, 6)]);
    for s in w.players[0].inv.iter_mut() {
        *s = ItemStack::default();
    }
    assert_eq!(w.backpacks.len(), 1);

    let (code, _, _) = do_move(&mut w, bag, CONT_BAG, 0, CONT_SELF, 0, 6);
    assert_eq!(code, EV_MOVED);
    assert_eq!(w.backpacks.len(), 0, "the emptied bag left the store");
    assert!(
        w.events
            .entries()
            .iter()
            .any(|e| e.code == sim_core::world::EV_BAG_REMOVED
                && e.b == sim_core::backpack::BAG_GONE_EMPTIED),
        "and announced itself as emptied, exactly as a take-all would"
    );
}

/// Reach is a fact about *now*, not about when a panel opened. Walking
/// away mid-drag refuses, and refuses with its own reason — "it is over
/// there" and "it is gone" are different things to tell a player.
#[test]
fn a_bag_out_of_reach_refuses_with_its_own_reason() {
    let mut w = move_world();
    let bag = bag_at_feet(&mut w, &[(1, 20)]);

    let (code, _, _) = do_move(&mut w, bag, CONT_BAG, 0, CONT_SELF, 3, 5);
    assert_eq!(code, EV_MOVED, "in reach to begin with");

    // Walk the body a long way off. Position is sim state, so this is the
    // same displacement a walk produces, without spending ticks on it.
    w.players[0].body.qx += 100_000;
    let (code, b, _) = do_move(&mut w, bag, CONT_BAG, 0, CONT_SELF, 4, 5);
    assert_eq!(code, EV_MOVE_REFUSED);
    assert_eq!(b, REFUSE_M_REACH);
}

/// A slot moved onto itself is refused rather than treated as a no-op.
///
/// Not pedantry: the caller reads both sides as copies and writes both
/// back, so a same-slot move would write the source and then let the
/// destination write win — which duplicates `count` items out of nothing.
/// The refusal is what makes the copy-then-write shape safe.
#[test]
fn a_slot_moved_onto_itself_is_refused() {
    let mut w = move_world();
    w.players[0].inv[4] = ItemStack { item: 2, count: 10 };
    let before = ledger(&w);

    let (code, b, _) = do_move(&mut w, 0, CONT_SELF, 4, CONT_SELF, 4, 5);
    assert_eq!(code, EV_MOVE_REFUSED);
    assert_eq!(b, REFUSE_M_SLOT);
    assert_eq!(w.players[0].inv[4], ItemStack { item: 2, count: 10 });
    assert_eq!(ledger(&w), before, "and it invented nothing");
}

/// An item the stack ladder cannot size cannot be moved — the same rule
/// `loot_nearest` already applies when it skips such a slot, stated here
/// as its own reason so a player is told rather than ignored.
#[test]
fn an_item_off_the_ladder_refuses() {
    let mut w = move_world();
    // `probe_fixture` arms 0..8; anything above that has `stack_max` 0.
    w.players[0].inv[0] = ItemStack {
        item: ITEMS + 5,
        count: 3,
    };
    let (code, b, _) = do_move(&mut w, 0, CONT_SELF, 0, CONT_SELF, 1, 1);
    assert_eq!(code, EV_MOVE_REFUSED);
    assert_eq!(b, REFUSE_M_UNSTACKABLE);
}

/// A dead body does not rearrange its pockets. `live_slot_of` is the
/// gate — the same one every other verb passes through — and the death
/// screen is not a container UI.
#[test]
fn a_corpse_cannot_move_anything() {
    let mut w = move_world();
    w.players[0].inv[0] = ItemStack { item: 1, count: 4 };
    w.players[0].dead = true;
    let before = snapshot(&w);

    w.tick(&[Command::Move {
        id: PLAYER,
        cont: 0,
        from_kind: CONT_SELF,
        from_slot: 0,
        to_kind: CONT_SELF,
        to_slot: 1,
        count: 2,
    }]);
    assert!(
        !w.events
            .entries()
            .iter()
            .any(|e| e.code == EV_MOVED || e.code == EV_MOVE_REFUSED),
        "a corpse's move is not even answered — it never reached the verb"
    );
    assert_eq!(snapshot(&w), before);
}

// ---------------------------------------------------------------------------
// The planner itself, in isolation
// ---------------------------------------------------------------------------

/// `resolve` conserves. Stated directly against the pure pair, over the
/// whole small state space, because it is cheaper to prove here than to
/// infer from the world-level property — and because a `resolve` that
/// leaked would make every world-level assertion above a coincidence.
#[test]
fn resolve_conserves_over_the_whole_small_space() {
    for src_item in 0..4u16 {
        for src_count in 0..=8u16 {
            for dst_item in 0..4u16 {
                for dst_count in 0..=8u16 {
                    for count in 0..=9u16 {
                        let src = ItemStack {
                            item: src_item,
                            count: src_count,
                        };
                        let dst = ItemStack {
                            item: dst_item,
                            count: dst_count,
                        };
                        let Ok(plan) = plan_move(8, src, dst, count) else {
                            continue;
                        };
                        let (a, b) = resolve(plan, src, dst);
                        // Per-item totals before and after.
                        let mut before = [0u32; 4];
                        let mut after = [0u32; 4];
                        for s in [src, dst] {
                            if s.count > 0 {
                                before[s.item as usize] += s.count as u32;
                            }
                        }
                        for s in [a, b] {
                            if s.count > 0 {
                                after[s.item as usize] += s.count as u32;
                            }
                        }
                        assert_eq!(
                            before, after,
                            "resolve leaked: {src:?} -> {dst:?} x{count} as {plan:?}"
                        );
                        // No stack past its ladder, and canonical empties.
                        for s in [a, b] {
                            assert!(s.count <= 8, "resolve built an over-full stack: {s:?}");
                            if s.count == 0 {
                                assert_eq!(s.item, 0, "an empty slot must be canonical: {s:?}");
                            }
                        }
                    }
                }
            }
        }
    }
}

/// The planner's exits, one example each, so a reason that stops being
/// reachable fails here by name rather than silently in the fuzz's
/// coverage assertion.
#[test]
fn the_planner_names_each_refusal() {
    let full = ItemStack { item: 1, count: 8 };
    let some = ItemStack { item: 1, count: 3 };
    let other = ItemStack { item: 2, count: 3 };
    let empty = ItemStack::default();

    assert_eq!(plan_move(8, empty, some, 1), Err(REFUSE_M_EMPTY));
    assert_eq!(plan_move(8, some, empty, 0), Err(REFUSE_M_COUNT));
    assert_eq!(plan_move(8, some, empty, 4), Err(REFUSE_M_COUNT));
    assert_eq!(plan_move(0, some, empty, 1), Err(REFUSE_M_UNSTACKABLE));
    assert_eq!(plan_move(8, full, full, 1), Err(REFUSE_M_NO_ROOM));
    assert_eq!(
        plan_move(8, some, other, 1),
        Err(REFUSE_M_NO_ROOM),
        "a partial drop onto a different item has nowhere to put the rest"
    );
    assert_eq!(
        plan_move(8, some, other, 3),
        Ok(MovePlan::Swap),
        "but the whole stack swaps"
    );
    assert_eq!(
        plan_move(8, some, empty, 2),
        Ok(MovePlan::Transfer { item: 1, count: 2 })
    );
}
