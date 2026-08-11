//! `test_snapshot_budget` (DESIGN.md §12): the worst-case scene — the
//! full shard cap clustered inside one AOI cell — and per-client snapshots
//! must hold the 1100 B budget, keep every interest entity inside the
//! staleness ceiling, always carry the client's own entity, and
//! reconstruct byte-exact through the client view once acks flow. All
//! counted/structural asserts: identical on this box and the reference
//! VPS.

use protocol::InputDatagram;
use server::core::{Lane, ShardCore};
use server::stats::ShardStats;
use server::view::{Applied, ClientView};
use sim_core::limits::{
    AOI_ENTER_CM, AOI_RANK_ENTER, AOI_RANK_EXIT, DATAGRAM_BUDGET_BYTES, MAX_MOBS, MAX_PLAYERS,
    MAX_SNAPSHOT_ENTITIES, SNAPSHOT_INTERVAL_TICKS, STALENESS_CEILING,
};

const SEED: u64 = 0xB1D_6E75;

/// Player id for connection slot `s` as `install` mints them (generation 1).
fn id_of(slot: usize) -> u32 {
    (1 << 8) | slot as u32
}

/// A core with the full cap connected and every body herded into a 60 m
/// square (well inside the 176 m enter radius of everyone).
fn clustered_core(stats: &ShardStats) -> ShardCore {
    let mut core = ShardCore::new(SEED);
    for slot in 0..MAX_PLAYERS {
        assert!(core.connect(slot, id_of(slot)), "connect {slot}");
        if (slot + 1) % 32 == 0 || slot + 1 == MAX_PLAYERS {
            // Command budget reserves headroom; land joins in batches.
            core.tick(stats, |_, _, _| true);
        }
    }
    for (i, p) in core.world.players.iter_mut().enumerate() {
        if !p.active {
            continue;
        }
        // 10×10 grid, 6 m pitch, island interior.
        p.body.qx = ((900.0 + (i % 10) as f32 * 6.0) / 0.03) as i32;
        p.body.qz = ((900.0 + (i / 10) as f32 * 6.0) / 0.03) as i32;
    }
    core
}

/// Every entity id connection `slot` currently holds interest in — the
/// server-side set the snapshot fill ranks, read off the two flag arrays
/// that *are* that set (`ClientNetState::interest` / `m_interest`).
/// Structural, not a wire count: what a datagram carried is the budget's
/// business, and this is the set the budget draws from.
fn interest_of(core: &ShardCore, slot: usize) -> std::collections::BTreeSet<u32> {
    let c = &core.clients[slot];
    let players = (0..MAX_PLAYERS)
        .filter(|&w| c.interest[w] && core.world.players[w].active)
        .map(|w| core.world.players[w].id);
    let mobs = (0..MAX_MOBS)
        .filter(|&s| c.m_interest[s] && core.world.mobs.m[s].alive)
        .map(sim_core::mob::mob_id);
    players.chain(mobs).collect()
}

/// Run until the next snapshot cadence tick, collecting (slot, bytes).
fn snapshot_round(core: &mut ShardCore, stats: &ShardStats) -> Vec<(usize, Vec<u8>)> {
    let mut out = Vec::new();
    loop {
        let mut sent = Vec::new();
        core.tick(stats, |lane, slot, bytes| {
            if lane == Lane::Snapshot {
                sent.push((slot, bytes.to_vec()));
            }
            true
        });
        if core.world.tick.is_multiple_of(SNAPSHOT_INTERVAL_TICKS) {
            out.extend(sent);
            return out;
        }
    }
}

#[test]
fn test_snapshot_budget() {
    let stats = ShardStats::default();
    let mut core = clustered_core(&stats);
    let mut views: Vec<ClientView> = (0..MAX_PLAYERS).map(|_| ClientView::new()).collect();

    // Phase 1 — no acks: every snapshot is zero-state absolute, the
    // hardest byte shape.
    let rounds: usize = 12;
    for round in 0..rounds {
        let sent = snapshot_round(&mut core, &stats);
        assert_eq!(
            sent.len(),
            MAX_PLAYERS,
            "round {round}: every connected client gets a snapshot"
        );
        for (slot, bytes) in &sent {
            assert!(
                bytes.len() <= DATAGRAM_BUDGET_BYTES,
                "round {round} slot {slot}: {} B blows the budget",
                bytes.len()
            );
            let view = &mut views[*slot];
            match view.apply(bytes).expect("server datagram decodes") {
                Applied::Ok { .. } => {}
                other => panic!("round {round} slot {slot}: {other:?}"),
            }
            // Own entity rides every snapshot (reconciliation needs it).
            assert!(
                view.get(id_of(*slot)).is_some(),
                "round {round} slot {slot}: own entity missing"
            );
            // Reconstructed states match the world exactly — quantize
            // both sides means equality, not tolerance.
            for (eid, state) in view.entities.iter() {
                let p = core
                    .world
                    .players
                    .iter()
                    .find(|p| p.active && p.id == *eid)
                    .unwrap_or_else(|| panic!("unknown entity {eid} in view"));
                assert_eq!(state.qx, p.body.qx, "qx of {eid}");
                assert_eq!(state.qy, p.body.qy, "qy of {eid}");
                assert_eq!(state.qz, p.body.qz, "qz of {eid}");
            }
        }
    }
    // The staleness ceiling (NETCODE.md §3): no **offered** entity may go
    // more than STALENESS_CEILING snapshots unsent, so the union over any
    // CEILING consecutive snapshots must cover everything the client is
    // subscribed to. During this ack-free phase a snapshot's applied set IS
    // its content (a zero-state apply rebuilds the map), so the union is
    // readable straight off the views.
    //
    // The offered set is the **interest set**, not the shard. Until the
    // rank band existed this line read `MAX_PLAYERS`, and that was the same
    // false premise the band was added to remove: a radius alone let all 99
    // clustered peers be marked interesting, so the ceiling looked like a
    // promise about every player on the shard. It never was one — the
    // accumulator can only rotate what AOI subscribed — and asserting it
    // against the shard would now silently re-assert that a set bounded by
    // `MAX_PLAYERS + MAX_MOBS` fits a wire count of 64.
    let ceiling = STALENESS_CEILING as usize;
    let mut union: Vec<std::collections::BTreeSet<u32>> = vec![Default::default(); MAX_PLAYERS];
    let mut zero_len = 0usize;
    let mut zero_count = 0usize;
    for _ in 0..ceiling {
        let sent = snapshot_round(&mut core, &stats);
        for (slot, bytes) in &sent {
            zero_len += bytes.len();
            zero_count += 1;
            views[*slot].apply(bytes).expect("decodes");
            union[*slot].extend(views[*slot].entities.iter().map(|(id, _)| *id));
        }
    }
    for (slot, seen) in union.iter().enumerate() {
        let mut owed = interest_of(&core, slot);
        owed.insert(id_of(slot)); // own entity rides every snapshot
        assert_eq!(
            seen,
            &owed,
            "slot {slot}: staleness ceiling broken — {} of {} subscribed \
             entities inside {ceiling} snapshots",
            seen.len(),
            owed.len()
        );
        // And the set is a real one, not an empty promise the assert above
        // would satisfy: this is the clustered worst case.
        assert!(
            owed.len() > AOI_RANK_ENTER,
            "slot {slot}: only {} entities subscribed in the clustered \
             scene — the ceiling is being asserted over nothing",
            owed.len()
        );
    }

    // Phase 2 — acks flow: deltas engage and shrink the wire.
    let mut delta_len = 0usize;
    let mut delta_rounds = 0usize;
    for _round in 0..8 {
        // Every client acks its newest applied snapshot.
        for (slot, view) in views.iter().enumerate() {
            let (ack, bits) = view.ack_fields();
            let dg = InputDatagram::new(ack, bits, 1);
            core.push_input(slot, &dg);
        }
        let sent = snapshot_round(&mut core, &stats);
        for (slot, bytes) in &sent {
            assert!(bytes.len() <= DATAGRAM_BUDGET_BYTES);
            let applied = views[*slot].apply(bytes).expect("decodes");
            if let Applied::Ok { delta: true } = applied {
                delta_len += bytes.len();
                delta_rounds += 1;
            }
            for (eid, state) in views[*slot].entities.iter() {
                let p = core
                    .world
                    .players
                    .iter()
                    .find(|p| p.active && p.id == *eid)
                    .expect("entity exists");
                assert_eq!(
                    (state.qx, state.qy, state.qz),
                    (p.body.qx, p.body.qy, p.body.qz),
                    "delta reconstruction of {eid}"
                );
            }
        }
    }
    assert!(
        delta_rounds >= MAX_PLAYERS * 4,
        "deltas engaged on {delta_rounds} snapshots only"
    );
    let avg_zero = zero_len / zero_count;
    let avg_delta = delta_len / delta_rounds;
    assert!(
        avg_delta < avg_zero,
        "delta snapshots ({avg_delta} B avg) not smaller than absolute ({avg_zero} B avg)"
    );

    // The encoder never met an out-of-range body or refused a begin.
    assert_eq!(ShardStats::get(&stats.encode_range_errors), 0);
    assert_eq!(ShardStats::get(&stats.forced_resyncs), 0);
}

/// Shedding is the designed degradation and it must be **visible**
/// (`reference/NETWORK.md` §9.2.3 · §3). A snapshot that cannot carry every
/// ranked entity drops the tail rather than fragmenting — correct, and until
/// `snap_entities_shed` existed it was the only path in the pipeline by
/// which quality falls under load with nothing recording it.
///
/// Two-sided on purpose, which is the whole point of the test: a counter
/// that only ever goes up proves nothing, because a `bump` on the wrong line
/// would pass a one-sided assert. So the same core shape is driven twice —
/// a set at its ceiling, where the fill must refuse; and sparse, where
/// nothing is offered that does not fit and the counter must not move at
/// all.
///
/// **What "cannot fit" means here changed with the rank band**, and saying
/// so is the point. This scene used to be the clustered shard as it stood:
/// a radius-only AOI marked all 99 peers interesting, and the fill refused
/// the 65th record — so what was measured was the *wire's* record cap, not
/// the 1,100 B budget (49 absolute records measure 671 B; the byte budget
/// would hold ~80). With the interest set now rank-capped it fits its own
/// snapshot by construction, which is the slice's whole claim, and the
/// clustered shard sheds nothing. The reachable worst case left is a set
/// **at the eviction rank** — `AOI_RANK_EXIT` entities plus the client's
/// own, one record over what the wire can name — and that is what is
/// staged.
#[test]
fn the_shed_counter_sees_the_budget_refuse() {
    // The full cap inside one AOI cell, then every interest bit forced on:
    // the state a client reaches when it subscribed while the field was
    // sparse and the field then filled in around it. Staged directly rather
    // than walked in tick by tick, because a staged crowd would be testing
    // the admission order (which `a_rank_boundary_entity_does_not_flap`
    // owns) instead of the counter. `update_interest` evicts it back to
    // `AOI_RANK_EXIT` on the next tick — which is itself the assertion that
    // the rank band's exit side runs at all.
    let stats = ShardStats::default();
    let mut core = clustered_core(&stats);
    core.tick(&stats, |_, _, _| true); // populate `tracked_id`
    let ids: Vec<(usize, u32)> = (0..MAX_PLAYERS)
        .filter(|&w| core.world.players[w].active)
        .map(|w| (w, core.world.players[w].id))
        .collect();
    for c in core.clients.iter_mut() {
        for &(w, id) in &ids {
            if id != c.id {
                c.interest[w] = true;
            }
        }
    }
    for _ in 0..4 {
        snapshot_round(&mut core, &stats);
    }
    for slot in 0..MAX_PLAYERS {
        assert_eq!(
            interest_of(&core, slot).len(),
            AOI_RANK_EXIT,
            "slot {slot}: the exit rank did not evict the forced set back to \
             the cap"
        );
    }
    let shed = ShardStats::get(&stats.snap_entities_shed);
    assert!(
        shed > 0,
        "the worst-case scene shed nothing — either the fill stopped \
         refusing or the counter is not wired to it"
    );
    // Sanity on the magnitude, not a pinned number: each of the 100 clients
    // is offered 99 peers per snapshot and can carry a few dozen, so four
    // snapshot rounds shed hundreds at minimum. A counter reading 1 would
    // satisfy `> 0` and mean the bump landed inside a branch it should not.
    assert!(
        shed > MAX_PLAYERS as u64,
        "shed {shed} across 4 rounds of {MAX_PLAYERS} clients is too low to \
         be the tail of a refused fill"
    );

    // Sparse: two players 500 m apart — well past the 208 m exit — and no
    // animals, so every client's candidate list is empty and the only
    // record in a snapshot is its own entity. Nothing is offered that does
    // not fit, so nothing may be counted as shed.
    let stats = ShardStats::default();
    let mut core = ShardCore::new(SEED);
    assert!(core.connect(0, id_of(0)));
    assert!(core.connect(1, id_of(1)));
    core.tick(&stats, |_, _, _| true);
    for m in core.world.mobs.m.iter_mut() {
        // The roster is worldgen's, not this test's: an animal that happens
        // to stand near a player is a candidate, and this assertion is about
        // the counter rather than about where the pigs spawned.
        m.alive = false;
    }
    let q = |m: f32| (m / 0.03) as i32;
    for (i, p) in core
        .world
        .players
        .iter_mut()
        .filter(|p| p.active)
        .enumerate()
    {
        p.body.qx = q(700.0 + i as f32 * 500.0);
        p.body.qz = q(700.0);
    }
    for _ in 0..4 {
        snapshot_round(&mut core, &stats);
    }
    assert_eq!(
        ShardStats::get(&stats.snap_entities_shed),
        0,
        "nothing was offered that did not fit, so nothing may read as shed"
    );
}

/// The three numbers the fill produces, read together: what it was
/// **offered**, what it **carried**, and what it **shed**.
///
/// `snap_entities_shed` alone cannot answer the question a soak is run to
/// answer. A shard reporting a million shed entities is either fine (it
/// offered ten million) or on fire (it offered a million and one), and the
/// counter that separates those two is the denominator. So the fill counts
/// its candidate list and the part of that list it managed to name, and
/// those two plus the existing shed counter are the whole degradation
/// picture at 15 Hz per client.
///
/// Neither new counter includes the client's own entity. It rides every
/// snapshot ahead of the ranked list, is never a candidate, and is not
/// something the budget is allowed to refuse — folding it in would add a
/// constant `snapshots × 1` to both halves and make the carried/offered
/// ratio read better than the fill actually did.
///
/// **The assertion that does the work is the identity
/// `offered == carried + shed`**, and it is here because the three numbers
/// are only comparable if they come from one pass over one list. `core.rs`
/// counts shed once at the end of the fill rather than at the three ways
/// out of the loop, and says why: an entity refused by the budget and then
/// passed over again by the break is one shed entity, not two. Move that
/// accounting into the `Err(WireError::Overflow)` arm and every one-sided
/// assert about shedding still passes — `the_shed_counter_sees_the_budget_
/// refuse` included — while the three counters stop describing the same
/// pass. The identity is the only thing that notices.
///
/// Delta-based on both sides. `clustered_core` ticks four times landing its
/// joins in batches and those ticks encode snapshots, so the absolute
/// counters are not the counts of the rounds this test drives. It also
/// never reconstructs a view, which is how it avoids the reconstruction
/// loop in `test_snapshot_budget` that panics on an entity id it cannot
/// find in `world.players`.
#[test]
fn the_fill_counts_what_it_offered_and_what_it_carried() {
    // (offered, carried, shed) — always read as a triple, because any two
    // of them differenced against a third taken a tick later is a lie.
    fn fill_counters(stats: &ShardStats) -> (u64, u64, u64) {
        (
            ShardStats::get(&stats.snap_candidates),
            ShardStats::get(&stats.snap_entities_sent),
            ShardStats::get(&stats.snap_entities_shed),
        )
    }

    // The budget binding: the full cap in one AOI cell with every interest
    // bit forced on, exactly as the shed counter's own test stages it, so
    // the shed term of the identity is a real number instead of zero.
    let stats = ShardStats::default();
    let mut core = clustered_core(&stats);
    core.tick(&stats, |_, _, _| true); // populate `tracked_id`
    let ids: Vec<(usize, u32)> = (0..MAX_PLAYERS)
        .filter(|&w| core.world.players[w].active)
        .map(|w| (w, core.world.players[w].id))
        .collect();
    for c in core.clients.iter_mut() {
        for &(w, id) in &ids {
            if id != c.id {
                c.interest[w] = true;
            }
        }
    }
    let before = fill_counters(&stats);
    for _ in 0..4 {
        snapshot_round(&mut core, &stats);
    }
    let after = fill_counters(&stats);
    let (offered, carried, shed) = (after.0 - before.0, after.1 - before.1, after.2 - before.2);

    // Each of the 100 clients is offered its whole rank-capped set on every
    // snapshot of four rounds, so the offered count is in the thousands. A
    // magnitude sanity check, not a pinned number: a counter reading 1 would
    // satisfy `> 0` and mean the bump landed inside a branch.
    assert!(
        offered > MAX_PLAYERS as u64,
        "the clustered worst case offered {offered} candidates across 4 \
         rounds of {MAX_PLAYERS} clients — too few to be a ranked interest \
         list"
    );
    assert!(
        carried > 0,
        "{offered} candidates offered and nothing counted as carried — the \
         carried counter is not wired to the fill"
    );
    assert!(
        shed > 0,
        "the worst-case scene shed nothing, so the identity below is being \
         checked where it cannot fail"
    );
    assert_eq!(
        offered,
        carried + shed,
        "offered {offered} ≠ carried {carried} + shed {shed}: the three \
         counters are not describing the same pass over the same candidate \
         list, so no ratio taken from a soak line means anything"
    );

    // The other end, where the budget never binds: two bodies inside the
    // enter radius and nothing else alive, so the fill is offered exactly
    // one peer per snapshot and carries it. This is what proves the carried
    // counter is not a copy of the offered one taken before the fill ran —
    // there `shed == 0` is load-bearing rather than incidental — and that
    // neither counts the own entity, which is present in every one of these
    // snapshots and must appear in neither number.
    let stats = ShardStats::default();
    let mut core = ShardCore::new(SEED);
    assert!(core.connect(0, id_of(0)));
    assert!(core.connect(1, id_of(1)));
    core.tick(&stats, |_, _, _| true);
    for m in core.world.mobs.m.iter_mut() {
        // `alive = false` on its own is a state the roster undoes: a dead
        // slot hatches again on `respawn_at`, and a pig that walks inside
        // 176 m of the pair is a candidate like any other body. That would
        // make these counts a fact about where this seed put its animals.
        // `u64::MAX` is the off switch — the roster never comes back.
        m.alive = false;
        m.respawn_at = u64::MAX;
    }
    let q = |m: f32| (m / 0.03) as i32;
    for (i, p) in core
        .world
        .players
        .iter_mut()
        .filter(|p| p.active)
        .enumerate()
    {
        p.body.qx = q(700.0 + i as f32 * 100.0);
        p.body.qz = q(700.0);
    }
    let before = fill_counters(&stats);
    for _ in 0..4 {
        snapshot_round(&mut core, &stats);
    }
    let after = fill_counters(&stats);
    let (offered, carried, shed) = (after.0 - before.0, after.1 - before.1, after.2 - before.2);
    assert!(
        offered > 0,
        "100 m apart is inside the enter radius: each client owes the other \
         as a candidate and nothing was offered"
    );
    assert_eq!(
        carried, offered,
        "one peer per snapshot fits any budget — {carried} of {offered} \
         carried"
    );
    assert_eq!(
        shed, 0,
        "nothing was offered that did not fit, so nothing may read as shed"
    );
}

/// `snap_candidates`, pinned against a count taken from **outside** the
/// fill — the two things the counter block argues for and that the
/// identity above cannot see.
///
/// `offered == carried + shed` is a statement about three numbers agreeing
/// with each other, and three numbers can agree while all three are wrong
/// in the same direction. It is blind in exactly the two directions the
/// comment over the bumps spends its paragraphs on:
///
/// - **The own entity is not a candidate.** Fold it into `offered` and into
///   `carried` at the same time — drop the two `- 1`s together — and the
///   identity still balances, `carried == offered` in the sparse phase
///   still holds, every magnitude assert still passes, and the ratio a soak
///   is run to read is flattered by a constant `snapshots × 1`.
/// - **Where the bumps sit.** `n_cand` is final at the sort, several lines
///   above a `return None` for an own entity the encoder refuses. Count
///   there and a snapshot that never went out is in the denominator.
///
/// One assertion closes both: sum `interest_of` — the flag arrays that *are*
/// the candidate set, and which never contain the client itself — over the
/// snapshots that actually went out, and require the counter to equal that
/// sum. A fold-in makes the counter exceed it by the snapshot count; a bump
/// above the refusal makes it exceed it by the refused client's set.
///
/// `interest_of` is read after each round because the fill ranks the set
/// exactly as `update_interest` left it earlier in that same tick — nothing
/// between the two touches an interest flag. Delta-based on the counter,
/// for `clustered_core`'s four connect-batch ticks.
#[test]
fn the_offered_counter_is_the_interest_set_and_nothing_else() {
    // --- the clustered worst case, where the sets are at the rank cap and
    // the fill is shedding, so the sum is a large number rather than a
    // handful the identity would also have caught.
    let stats = ShardStats::default();
    let mut core = clustered_core(&stats);
    // Admission settles over ticks; a set still walking in would make the
    // sum below a fact about the ramp rather than about the cap.
    for _ in 0..4 {
        snapshot_round(&mut core, &stats);
    }
    let before = ShardStats::get(&stats.snap_candidates);
    let mut independent = 0u64;
    let mut snapshots = 0u64;
    for _ in 0..4 {
        for (slot, _) in snapshot_round(&mut core, &stats) {
            independent += interest_of(&core, slot).len() as u64;
            snapshots += 1;
        }
    }
    let offered = ShardStats::get(&stats.snap_candidates) - before;
    assert_eq!(
        snapshots,
        4 * MAX_PLAYERS as u64,
        "every connected client owes a snapshot on every cadence tick — \
         {snapshots} of them went out over 4 rounds of {MAX_PLAYERS}"
    );
    assert!(
        independent > snapshots,
        "{independent} candidates over {snapshots} snapshots: the sets are \
         one entity each or emptier, and a fold-in of the own entity is a \
         difference of exactly one per snapshot, so this scene could not \
         tell one from a real candidate"
    );
    assert_eq!(
        offered, independent,
        "the fill counted {offered} candidates where the interest sets it \
         ranked held {independent} — `snap_candidates` is counting something \
         other than the set, and the own entity is the near-certain suspect \
         (it rides every snapshot and is never a candidate)"
    );

    // --- the other half: a snapshot that never goes out. `n_cand` is final
    // at the sort and the own entity is encoded after it, so a client whose
    // own body the wire cannot name reaches that `return None` holding a
    // full candidate list — the one case that tells a bump above the return
    // apart from a bump below it.
    let stats = ShardStats::default();
    let mut core = ShardCore::new(SEED);
    for m in core.world.mobs.m.iter_mut() {
        // `alive = false` alone is undone by `respawn_at`, and a hatched pig
        // is a candidate like any body (`a_near_animal_outranks_a_far_player`
        // pays for this one).
        m.alive = false;
        m.respawn_at = u64::MAX;
    }
    const PEERS: usize = 6;
    for slot in 0..=PEERS {
        assert!(core.connect(slot, id_of(slot)), "connect {slot}");
        core.tick(&stats, |_, _, _| true);
    }
    let q = |m: f32| (m / 0.03) as i32;
    // Slot 0 stands a kilometre up. `POS_Y_BITS`/`POS_Y_BIAS` window the
    // wire's y at 430 m, so `check_ranges` refuses that body and
    // `encode_snapshot` returns None on the own entity — a *sim bug* staged
    // on purpose, which is the only thing that reaches that arm. Its peers
    // stand 3–18 m away in xz, where AOI reads them, and AOI never reads y:
    // slot 0 holds a full interest set right up to the refusal.
    //
    // Re-placed every round because gravity is still running on it. Eight
    // ticks at terminal velocity is ~13 m against 570 m of clearance, so the
    // re-place is belt and braces — and the per-round assert below is what
    // would say so if it were ever not.
    let place = |core: &mut ShardCore| {
        for p in core.world.players.iter_mut() {
            if !p.active {
                continue;
            }
            let Some(slot) = (0..=PEERS).find(|&s| id_of(s) == p.id) else {
                continue;
            };
            p.body.qx = q(900.0 + 3.0 * slot as f32);
            p.body.qz = q(900.0);
            if slot == 0 {
                p.body.qy = q(1000.0);
            }
        }
    };
    let before = ShardStats::get(&stats.snap_candidates);
    let mut independent = 0u64;
    let mut refused = 0u64;
    for round in 0..4 {
        place(&mut core);
        let sent = snapshot_round(&mut core, &stats);
        assert!(
            !sent.iter().any(|(slot, _)| *slot == 0),
            "round {round}: slot 0's snapshot went out, so the encoder is \
             naming a body outside the wire's y window and this half of the \
             test is staged on nothing"
        );
        for (slot, _) in &sent {
            independent += interest_of(&core, *slot).len() as u64;
        }
        refused += interest_of(&core, 0).len() as u64;
    }
    let offered = ShardStats::get(&stats.snap_candidates) - before;
    assert!(
        ShardStats::get(&stats.encode_range_errors) > 0,
        "nothing was refused on range: slot 0 is absent from the sends for \
         some other reason and the refusal arm was never reached"
    );
    assert!(
        refused > 0,
        "the refused client held no candidates, so counting above the \
         `return None` would add nothing here and this assert would pass on \
         a broken fill"
    );
    assert_eq!(
        offered, independent,
        "the fill counted {offered} candidates against {independent} in the \
         sets it actually shipped — the {refused} a refused snapshot ranked \
         and threw away are in the denominator, and every ratio taken off a \
         soak line is that much too kind"
    );
}

/// Wall 4 on the interest set itself. `MAX_SNAPSHOT_ENTITIES` is a
/// per-datagram wire count, and until the rank band existed the interest
/// set was bounded only by a **radius** — so the worst-case scene here, the
/// full shard cap inside one AOI cell, marked all 99 peers interesting on
/// every client and the constant that claims to cap the set was simply
/// false of it.
///
/// Two-sided, for the reason the shed counter's test is: an assert that the
/// count is under the cap passes trivially on a shard where nothing is in
/// range. So the same scene must also prove the cap is what is binding —
/// the set has to be *at* the rank band, not merely under 64.
#[test]
fn the_interest_set_is_capped_at_what_a_snapshot_can_carry() {
    let stats = ShardStats::default();
    let mut core = clustered_core(&stats);
    // Several rounds: the set is settled by admission over ticks, and a cap
    // that only holds on the first tick is not a cap.
    for _ in 0..6 {
        snapshot_round(&mut core, &stats);
    }
    for slot in 0..MAX_PLAYERS {
        let n = interest_of(&core, slot).len();
        assert!(
            n <= MAX_SNAPSHOT_ENTITIES,
            "slot {slot}: {n} entities in the interest set — \
             MAX_SNAPSHOT_ENTITIES ({MAX_SNAPSHOT_ENTITIES}) is not a cap on \
             it, only on one datagram"
        );
        assert!(
            n >= AOI_RANK_ENTER,
            "slot {slot}: only {n} of 99 peers in range are interesting — \
             the cap bound harder than the rank band says it may"
        );
    }
}

/// The rank band, from the side that made it an operator call: clearing an
/// interest bit is welded to a wire removal, which destroys the client's
/// interpolation history for that entity and rebuilds its mannequin. An
/// entity that keeps crossing a single rank threshold therefore does not
/// merely churn bandwidth — it freezes and respawns a body on every
/// snapshot, at 15 Hz.
///
/// So: a field dense enough for the rank cap to bind, with the two bodies
/// straddling the admission rank **swapping places every round**. Once a
/// body is in the set, nothing here may take it out — it is nowhere near
/// the exit rank, and rank hysteresis is exactly the claim that the
/// admission rank is not also the eviction rank.
#[test]
fn a_rank_boundary_entity_does_not_flap() {
    let stats = ShardStats::default();
    let mut core = ShardCore::new(SEED);
    // Two more than the admission rank, so ranks either side of it exist
    // and the two we jitter are the pair that straddles it.
    let n_players = AOI_RANK_ENTER + 3;
    for slot in 0..n_players {
        assert!(core.connect(slot, id_of(slot)), "connect {slot}");
        core.tick(&stats, |_, _, _| true);
    }
    for m in core.world.mobs.m.iter_mut() {
        // Worldgen's roster would rank among the peers and move the
        // boundary; this test is about the boundary, not about the pigs.
        m.alive = false;
    }

    let q = |m: f32| (m / 0.03) as i32;
    // Slot 0's player at the origin of the ranking; every peer on a ray
    // east of it at 20 m + 2 m per rank, so peer `i` has rank `i` and the
    // whole field sits inside the 176 m enter radius.
    let place = |core: &mut ShardCore, swap: bool| {
        for p in core.world.players.iter_mut() {
            if !p.active {
                continue;
            }
            p.body.qz = q(1000.0);
            let Some(rank) = (0..n_players).find(|&s| id_of(s) == p.id) else {
                continue;
            };
            if rank == 0 {
                p.body.qx = q(1000.0);
                continue;
            }
            // The pair straddling the admission rank trade places each
            // round; everyone else holds station.
            let eff = match (swap, rank) {
                (true, r) if r == AOI_RANK_ENTER => AOI_RANK_ENTER + 1,
                (true, r) if r == AOI_RANK_ENTER + 1 => AOI_RANK_ENTER,
                (_, r) => r,
            };
            p.body.qx = q(1000.0 + 20.0 + eff as f32 * 2.0);
        }
    };

    place(&mut core, false);
    for _ in 0..4 {
        snapshot_round(&mut core, &stats);
    }
    let settled = interest_of(&core, 0);
    assert!(
        settled.len() < n_players - 1,
        "the field is not dense enough for the rank cap to bind: {} of {} \
         peers interesting",
        settled.len(),
        n_players - 1
    );

    // Now jitter the boundary pair for a while and watch for a body being
    // taken out of the set.
    let mut swap = true;
    for round in 0..8 {
        place(&mut core, swap);
        swap = !swap;
        snapshot_round(&mut core, &stats);
        let now = interest_of(&core, 0);
        let lost: Vec<u32> = settled.difference(&now).copied().collect();
        assert!(
            lost.is_empty(),
            "round {round}: {lost:?} left slot 0's interest set at the \
             admission rank — a rank threshold that both admits and evicts \
             flaps, and every flap is a destroyed interpolation history"
        );
        assert!(
            now.len() <= AOI_RANK_EXIT,
            "round {round}: {} entities — the eviction rank is the hard cap",
            now.len()
        );
    }
    // Nothing here may cost a resync: the pending-removal set is only
    // touched by an entity actually leaving, and none did.
    assert_eq!(ShardStats::get(&stats.forced_resyncs), 0);
}

/// The band's headline claim, which is the half nothing was watching:
/// players and animals are ranked in **one** field because they compete for
/// the same `MAX_SNAPSHOT_ENTITIES` records. Every other test in this file
/// stands the roster down before it measures anything — twice with a
/// comment saying why — so exempting animals from the rank band entirely
/// left all five green. A rule whose case never runs is a mood, whatever
/// the constant is called.
///
/// Driven from both sides of the joint field, because "one field" is two
/// claims and each fails on its own line:
///
/// **Displacement.** Twenty peers stand at 152–171 m — inside the 176 m
/// enter radius, subscribed, and well under the rank cap while the roster
/// is down. Then sixty-four animals walk in at 30–93 m. No player moved
/// and no radius changed, so the only thing that can take a far peer out
/// of the set is animals holding the ranks. The peer at 12 m stays: rank
/// is distance, and it is still the nearest candidate on the shard.
///
/// **The converse.** Forty-nine peers inside 58 m fill the rank first, and
/// then the whole roster parks at 140–172 m — still inside the enter
/// radius, so a radius-only AOI would carry every animal. None may enter.
/// The exemption is refused in both directions or it is not one field.
///
/// Two things this is careful about, both learned rather than guessed:
///
/// - A dead slot hatches again on `respawn_at`, so the phases that need an
///   empty roster set `respawn_at = u64::MAX` and not merely
///   `alive = false`. Otherwise which pigs are standing is a fact about
///   the seed and the tick count, and the test starts flaking a month
///   later for a reason nobody will connect to this line.
/// - It never reconstructs a `ClientView`. `test_snapshot_budget`'s
///   reconstruction loop looks every id in a view up in `world.players`
///   and panics when it cannot find one, so a live animal reads there as a
///   snapshot bug. This reads the interest flags instead, which is where
///   the ranking decision actually lands.
#[test]
fn a_near_animal_outranks_a_far_player() {
    /// Planar distance² in cm², the AOI convention (`core.rs`: 3 cm quanta
    /// widened into centimetres, in i64). Here to prove the scene rather
    /// than to re-check the server's arithmetic — "the far peer is not in
    /// the set" means nothing unless the far peer was in range to begin
    /// with.
    fn d2_cm(a: (i32, i32), b: (i32, i32)) -> i64 {
        let dx = (a.0 - b.0) as i64 * 3;
        let dz = (a.1 - b.1) as i64 * 3;
        dx * dx + dz * dz
    }
    let pos_of = |core: &ShardCore, id: u32| -> (i32, i32) {
        let p = core
            .world
            .players
            .iter()
            .find(|p| p.active && p.id == id)
            .expect("staged player is in the world");
        (p.body.qx, p.body.qz)
    };
    let q = |m: f32| (m / 0.03) as i32;

    // --- displacement: the herd walks in and the far peers lose the ranks.
    let stats = ShardStats::default();
    let mut core = ShardCore::new(SEED);
    for m in core.world.mobs.m.iter_mut() {
        // Down for the staging phase and unable to hatch back into it.
        m.alive = false;
        m.respawn_at = u64::MAX;
    }
    const FAR: usize = 20;
    let n_ray = 2 + FAR;
    for slot in 0..n_ray {
        assert!(core.connect(slot, id_of(slot)), "connect {slot}");
        core.tick(&stats, |_, _, _| true);
    }
    // Slot 0 observes from the origin of the ranking; slot 1 at 12 m, nearer
    // than anything this scene will ever contain; the other twenty at
    // 152–171 m, inside the enter radius by 5 m at worst. `herd` stands the
    // roster up on the same ray at 30–93 m — every animal nearer than every
    // far peer and farther than the near one, so the joint order is the one
    // the doc comment describes and not an accident of where the pigs were
    // born.
    let place_ray = |core: &mut ShardCore, herd: bool| {
        for p in core.world.players.iter_mut() {
            if !p.active {
                continue;
            }
            p.body.qz = q(1000.0);
            let Some(slot) = (0..n_ray).find(|&s| id_of(s) == p.id) else {
                continue;
            };
            p.body.qx = match slot {
                0 => q(1000.0),
                1 => q(1012.0),
                s => q(1150.0 + s as f32),
            };
        }
        for (s, m) in core.world.mobs.m.iter_mut().enumerate() {
            m.alive = herd;
            if !herd {
                continue;
            }
            m.body.qx = q(1030.0 + s as f32);
            m.body.qz = q(1000.0);
            // Home moves with the body: the leash pulls a grazing pig back
            // to where the scene put it instead of across the island.
            m.home_qx = m.body.qx;
            m.home_qz = m.body.qz;
        }
    };
    for _ in 0..4 {
        place_ray(&mut core, false);
        snapshot_round(&mut core, &stats);
    }
    let peers: std::collections::BTreeSet<u32> = (1..n_ray).map(id_of).collect();
    assert_eq!(
        interest_of(&core, 0),
        peers,
        "with the roster down, {} peers is under the admission rank and every \
         one of them is inside the enter radius — the set is all of them or \
         the scene is not staged",
        n_ray - 1
    );

    for _ in 0..4 {
        place_ray(&mut core, true);
        snapshot_round(&mut core, &stats);
    }
    let set = interest_of(&core, 0);
    let own = pos_of(&core, id_of(0));
    let mobs_in = (0..MAX_MOBS)
        .filter(|&s| set.contains(&sim_core::mob::mob_id(s)))
        .count();
    for slot in 2..n_ray {
        let id = id_of(slot);
        assert!(
            d2_cm(pos_of(&core, id), own) <= AOI_ENTER_CM * AOI_ENTER_CM,
            "peer {id} is outside the enter radius, so the assert below would \
             pass on the distance band alone"
        );
        assert!(
            !set.contains(&id),
            "peer {id} held its rank against {mobs_in} nearer animals — they \
             are not competing for the same {MAX_SNAPSHOT_ENTITIES} records, \
             so the rank cap is a cap on players only"
        );
    }
    assert!(
        set.contains(&id_of(1)),
        "the nearest candidate on the shard was evicted: the herd takes the \
         ranks behind it, not the one in front of it"
    );
    assert!(
        mobs_in >= AOI_RANK_ENTER - 1,
        "only {mobs_in} animals in the set — the far peers left it for some \
         reason other than the herd taking their ranks"
    );
    assert!(
        set.len() <= AOI_RANK_EXIT,
        "{} entities in the joint field — the eviction rank is the hard cap \
         over both kinds",
        set.len()
    );

    // --- the converse: the animals are the ones out of rank, and get no
    // exemption for being animals.
    let stats = ShardStats::default();
    let mut core = ShardCore::new(SEED);
    for m in core.world.mobs.m.iter_mut() {
        m.alive = false;
        m.respawn_at = u64::MAX;
    }
    // Five over the admission rank, so the players fill it on their own
    // before an animal is standing anywhere.
    let n_dense = AOI_RANK_ENTER + 5;
    for slot in 0..n_dense {
        assert!(core.connect(slot, id_of(slot)), "connect {slot}");
        core.tick(&stats, |_, _, _| true);
    }
    let place_dense = |core: &mut ShardCore, herd: bool| {
        for p in core.world.players.iter_mut() {
            if !p.active {
                continue;
            }
            p.body.qz = q(1000.0);
            let Some(slot) = (0..n_dense).find(|&s| id_of(s) == p.id) else {
                continue;
            };
            p.body.qx = if slot == 0 {
                q(1000.0)
            } else {
                q(1010.0 + slot as f32)
            };
        }
        for (s, m) in core.world.mobs.m.iter_mut().enumerate() {
            m.alive = herd;
            if !herd {
                continue;
            }
            // 140–172 m: past every peer in the ranking, inside the enter
            // radius by 4 m at worst.
            m.body.qx = q(1140.0 + s as f32 * 0.5);
            m.body.qz = q(1000.0);
            m.home_qx = m.body.qx;
            m.home_qz = m.body.qz;
        }
    };
    for _ in 0..4 {
        place_dense(&mut core, false);
        snapshot_round(&mut core, &stats);
    }
    assert_eq!(
        interest_of(&core, 0).len(),
        AOI_RANK_ENTER,
        "the peers must fill the admission rank before the roster stands up, \
         or nothing is refusing the animals"
    );

    for _ in 0..4 {
        place_dense(&mut core, true);
        snapshot_round(&mut core, &stats);
    }
    let set = interest_of(&core, 0);
    let own = pos_of(&core, id_of(0));
    for s in 0..MAX_MOBS {
        let m = &core.world.mobs.m[s];
        assert!(
            m.alive && d2_cm((m.body.qx, m.body.qz), own) <= AOI_ENTER_CM * AOI_ENTER_CM,
            "animal {s} is not standing inside the enter radius, so the \
             assert below would pass on the distance band alone"
        );
        assert!(
            !set.contains(&sim_core::mob::mob_id(s)),
            "animal {s} entered a field already full of nearer players — the \
             rank band admits animals where it refuses players, and the set \
             is {} entities against a wire that can name {MAX_SNAPSHOT_ENTITIES}",
            set.len()
        );
    }
    assert!(
        set.len() >= AOI_RANK_ENTER && set.len() <= AOI_RANK_EXIT,
        "{} entities — the players' own field went slack while the animals \
         were being refused",
        set.len()
    );
}

/// AOI hysteresis (DESIGN.md §5.5): enter under 176 m, leave over 208 m,
/// and the band between flaps neither way. Structural, two players.
#[test]
fn test_aoi_hysteresis() {
    let stats = ShardStats::default();
    let mut core = ShardCore::new(SEED);
    assert!(core.connect(0, id_of(0)));
    assert!(core.connect(1, id_of(1)));
    core.tick(&stats, |_, _, _| true);

    let q = |m: f32| (m / 0.03) as i32;
    let place = |core: &mut ShardCore, x0: f32, x1: f32| {
        for p in core.world.players.iter_mut() {
            if !p.active {
                continue;
            }
            p.body.qz = q(1000.0);
            p.body.qx = if p.id == id_of(0) { q(x0) } else { q(x1) };
        }
    };
    let saw_peer = |sent: &[(usize, Vec<u8>)]| {
        let bytes = &sent.iter().find(|(s, _)| *s == 0).expect("slot 0 sent").1;
        let snap = protocol::decode_snapshot(bytes, &[]).expect("zero-state decodes");
        snap.entities().iter().any(|e| e.id == id_of(1))
    };

    // 190 m apart: outside enter radius, never subscribed.
    place(&mut core, 1000.0, 1190.0);
    let sent = snapshot_round(&mut core, &stats);
    assert!(!saw_peer(&sent), "190 m: must not enter");

    // 170 m: inside enter radius — subscribed.
    place(&mut core, 1000.0, 1170.0);
    let sent = snapshot_round(&mut core, &stats);
    assert!(saw_peer(&sent), "170 m: must enter");

    // Back to 190 m: inside the hysteresis band — stays subscribed.
    place(&mut core, 1000.0, 1190.0);
    let sent = snapshot_round(&mut core, &stats);
    assert!(saw_peer(&sent), "190 m after enter: hysteresis holds");

    // 215 m: past exit — unsubscribed.
    place(&mut core, 1000.0, 1215.0);
    let sent = snapshot_round(&mut core, &stats);
    assert!(!saw_peer(&sent), "215 m: must leave");
}

/// The rank band's two thresholds are **selected**, not sorted for
/// (`core.rs` pass 2, 2026-08-11): `select_nth_unstable` over the present
/// candidates replaced a full sort of the whole packed index, because that
/// sort was the largest single item in the shard profile. This is the gate
/// on it, and it is a gate on the *rule* rather than on the code — it
/// re-derives the true ordering here, with a full sort, from the same
/// positions the server ranked, and requires the interest set to be exactly
/// the `AOI_RANK_ENTER` nearest.
///
/// **Both** thresholds, which is what makes it a gate and not a spot check:
/// the enter rank says every candidate below it is in the set, the exit
/// rank says nothing at or above it is, and the two are the two selections.
/// Equality with the 45 nearest is deliberately *not* asserted — the herd
/// happens after the joins land, so bodies admitted at their spawn
/// positions are still held by hysteresis, and that is the band working
/// rather than the ordering failing. What cannot be true either way is a
/// near candidate outside the set or a far one inside it.
///
/// Two ways for the selection to fail and be caught here: take the wrong
/// order statistic, and the admitted set is the wrong size at the wrong
/// distance; drop the absent-padding argument, and a slot with no body in
/// it ranks ahead of a real one, which shows up as a near candidate the
/// enter threshold refused.
#[test]
fn the_rank_band_agrees_with_a_full_sort_of_the_same_field() {
    let stats = ShardStats::default();
    let mut core = clustered_core(&stats);
    // No content is baked here, so every species has zero hit points and no
    // roster slot ever hatches (`mob.rs`). Asserted rather than assumed:
    // one wandering animal would re-rank the field between the settle and
    // the check, and the failure would read as an ordering bug.
    assert!(
        core.world.mobs.m.iter().all(|m| !m.alive),
        "a live animal makes this scene non-static — the rank would move \
         under the check and this test would be measuring the roster"
    );
    for _ in 0..6 {
        snapshot_round(&mut core, &stats);
    }

    for slot in 0..MAX_PLAYERS {
        let me = core.world.players[core.clients[slot].own_wslot].body;
        let own_id = core.clients[slot].id;
        // The true order under the server's own key: planar centimetres
        // squared, ties broken by the packed candidate index.
        let mut field: Vec<(i64, u16, u32)> = (0..MAX_PLAYERS)
            .filter(|&w| core.world.players[w].active && core.world.players[w].id != own_id)
            .map(|w| {
                let b = core.world.players[w].body;
                let dx = (b.qx - me.qx) as i64 * 3;
                let dz = (b.qz - me.qz) as i64 * 3;
                (dx * dx + dz * dz, w as u16, core.world.players[w].id)
            })
            .collect();
        assert!(
            field.len() > AOI_RANK_ENTER,
            "slot {slot}: {} peers cannot make the admission rank bind",
            field.len()
        );
        field.sort_unstable();
        let ids = |n: usize| -> std::collections::BTreeSet<u32> {
            field.iter().take(n).map(|&(_, _, id)| id).collect()
        };
        let got = interest_of(&core, slot);
        let admits = ids(AOI_RANK_ENTER);
        let holds = ids(AOI_RANK_EXIT);
        let missed: Vec<u32> = admits.difference(&got).copied().collect();
        assert!(
            missed.is_empty(),
            "slot {slot}: candidates {missed:?} rank inside AOI_RANK_ENTER \
             ({AOI_RANK_ENTER}) by a full sort of the same field and are NOT \
             in the interest set — the enter threshold is not the \
             {AOI_RANK_ENTER}th smallest key"
        );
        let kept: Vec<u32> = got.difference(&holds).copied().collect();
        assert!(
            kept.is_empty(),
            "slot {slot}: candidates {kept:?} rank at or past AOI_RANK_EXIT \
             ({AOI_RANK_EXIT}) by a full sort of the same field and are still \
             in the interest set — the exit threshold is not the \
             {AOI_RANK_EXIT}th smallest key"
        );
        assert!(
            got.len() <= AOI_RANK_EXIT,
            "slot {slot}: {} entities in the interest set — the exit rank \
             bounds it at {AOI_RANK_EXIT}",
            got.len()
        );
    }
}
