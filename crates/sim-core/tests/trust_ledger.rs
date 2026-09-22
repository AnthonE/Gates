//! `test_trust_ledger`: the trust ring's bound, and the rows it keeps when
//! the event ring cannot (`sim_core::trust`, `NOW.md` §5d items 1–2).
//!
//! `EV_TRUST` rides the 256-seat drop-newest event ring, and a trust row is
//! the one passenger a resync cannot re-derive. The ledger ring beside it is
//! sized to the most rows a tick can make. This file gates both halves of
//! that sentence.
//!
//! ## The derivation, one test per link
//!
//! 1. **One row per command** is the borrow checker's: `TrustSeat` is spent
//!    by value. Its doc test (`trust.rs`) is the red mutant for a `Copy`
//!    derive, and a second `log_trust` in one arm is `E0382`.
//! 2. **One seat per applied command**:
//!    [`seats_are_minted_once_inside_the_command_loop`] scrapes `src/` for
//!    the mint and the clear.
//! 3. **The ring is the product**: [`the_ring_is_the_command_ceiling_times_one_row`].
//!
//! ## The storm, and why ground truth comes off the store
//!
//! `CLAUDE.md`'s trap list: *an event count taken while the ring is
//! overflowing is an undercount of the thing it names.* So this storm never
//! counts trust verbs through the event ring. Each withdrawal takes one unit
//! out of a box, so the box's own count says how many landed; the ledger must
//! hold exactly that many rows, in command order. The event ring is then
//! measured separately, to prove it lost rows the ledger kept.
//!
//! One owner stands a box. Sixteen flooders `Loot` a full bag each, placed
//! **first** in the tick's commands: one `Loot` is thirty `EV_GATHER`s and a
//! removal, so the event ring is full before the first withdrawal runs.
//! Eighty-three movers then take one unit each out of the owner's box: 240
//! trust verbs on a flood tick, 256 on a ceiling tick, 300 offered on an
//! over-ceiling tick. The owner is awake, then asleep, then evicted, so all
//! three presences cross the saturated ring.

// Wall 3's clippy list bans `String` and `format!` crate-wide, tests
// included. The scrape below reads the source directory at runtime, because
// an `include_str!` list is a hand-kept mirror that a new module would not
// appear in. `damage_routes.rs` carries the same pair of allows for the same
// reason. No line here runs on a sim thread.
#![allow(clippy::disallowed_types, clippy::disallowed_macros)]

use sim_core::backpack::BackpackContent;
use sim_core::build::{foundation_terrain_ok, BuildContent, BUILD_CELL_M, LOC_PLANE};
use sim_core::deploy::{box_key, DeployContent, DeployDef, ARCH_BOX, PLACE_FOUNDATION};
use sim_core::gather::{GatherContent, ItemStack};
use sim_core::inventory::{CONT_BOX, CONT_SELF};
use sim_core::limits::{
    BOX_SLOTS, INV_SLOTS, MAX_COMMANDS_PER_TICK, MAX_EVENTS_PER_TICK, MAX_PLAYERS,
    MAX_TRUST_ROWS_PER_TICK, TRUST_ROWS_PER_COMMAND,
};
use sim_core::movement::Body;
use sim_core::trust::TrustRow;
use sim_core::world::{
    Command, World, EV_GATHER, EV_TRUST, PRESENCE_ASLEEP, PRESENCE_AWAKE, PRESENCE_GONE, TRUST_CONT,
};
use sim_core::worldsave::WORLD_SAVE_MAX_BYTES;

/// The solved authored sites for `seed`, memoized per thread. `raid_storm.rs`
/// says why a per-call `terrain::haven` is too slow for a fixture.
fn hv(seed: u64) -> &'static sim_core::terrain::Haven {
    use std::cell::RefCell;
    thread_local! {
        static CACHE: RefCell<Vec<(u64, &'static sim_core::terrain::Haven)>> =
            const { RefCell::new(Vec::new()) };
    }
    let hit = CACHE.with(|c| c.borrow().iter().find(|(s, _)| *s == seed).map(|&(_, h)| h));
    if let Some(h) = hit {
        return h;
    }
    let h: &'static sim_core::terrain::Haven = Box::leak(Box::new(sim_core::terrain::haven(seed)));
    CACHE.with(|c| c.borrow_mut().push((seed, h)));
    h
}

const SEED: u64 = 0x7257_5354;

/// Ids, by role. Every id is distinct from every count and every code this
/// file asserts on, so a transposed field cannot read as a match.
const OWNER: u32 = 1;
const FLOODERS: u32 = 16;
const FIRST_FLOODER: u32 = 2;
const FIRST_MOVER: u32 = FIRST_FLOODER + FLOODERS;
/// Everybody else the shard can seat: 83 movers.
const MOVERS: u32 = MAX_PLAYERS as u32 - FIRST_MOVER + 1;

/// Trust verbs on a flood tick: the command ceiling minus the flooders.
const FLOOD_MOVES: usize = MAX_COMMANDS_PER_TICK - FLOODERS as usize;
/// Offered on the over-ceiling tick. The world applies the first
/// `MAX_COMMANDS_PER_TICK` and leaves the tail to the caller (`limits.rs`:
/// *defer*).
const OVER_MOVES: usize = 300;

/// Ticks per presence phase: flood ticks, then ceiling ticks, then one
/// over-ceiling tick.
const FLOOD_TICKS: usize = 6;
const CEILING_TICKS: usize = 3;

/// The fixture's box: `raid_storm.rs`'s row, appended at 8 / item 9.
const BOX_ROW: u16 = 8;
const BOX_ITEM: u16 = 9;
const FOUNDATION_ROW: u16 = 0;
/// A fixture item with a stack ceiling (100) and no other role here.
const GOODS: u16 = 7;
const STACK: u16 = 100;

fn storm_deploy() -> DeployContent {
    let mut d = DeployContent::probe_fixture();
    d.defs[BOX_ROW as usize] = DeployDef {
        arch: ARCH_BOX,
        placement: PLACE_FOUNDATION,
        hp: 60,
        item: BOX_ITEM,
        ..DeployDef::INERT
    };
    d.def_count = 9;
    d
}

/// The first buildable cell in a ring scan out from the middle of the map,
/// `raid_storm.rs::plots` narrowed to one.
fn cell(seed: u64) -> (u16, u16) {
    for r in 0..64i32 {
        for dz in -r..=r {
            for dx in -r..=r {
                if dx.abs() != r && dz.abs() != r {
                    continue;
                }
                let cx = (512 + dx) as u16;
                let cz = (512 + dz) as u16;
                let (x, z) = center(cx, cz);
                if foundation_terrain_ok(seed, hv(seed), x, z) {
                    return (cx, cz);
                }
            }
        }
    }
    panic!("the generator offered no buildable cell near the middle of seed {seed}");
}

fn center(cx: u16, cz: u16) -> (f32, f32) {
    (
        (cx as f32 + 0.5) * BUILD_CELL_M,
        (cz as f32 + 0.5) * BUILD_CELL_M,
    )
}

fn slot_of(w: &World, id: u32) -> usize {
    w.players
        .iter()
        .position(|p| p.active && p.id == id)
        .expect("a seated player has a slot")
}

/// Mover `k` of a tick's withdrawals: which mover, and into which of its
/// own (emptied) slots, so no two moves in one tick land on one stack.
fn mover_of(k: usize) -> (u32, u8) {
    (
        FIRST_MOVER + (k % MOVERS as usize) as u32,
        (k / MOVERS as usize) as u8,
    )
}

/// One unit out of box slot `k % BOX_SLOTS` into the mover's own slot.
fn withdrawal(k: usize, key: u32) -> Command {
    let (id, to_slot) = mover_of(k);
    Command::Move {
        id,
        cont: key,
        from_kind: CONT_BOX,
        from_slot: (k % BOX_SLOTS) as u8,
        to_kind: CONT_SELF,
        to_slot,
        count: 1,
    }
}

/// What one tick of the storm looked like.
#[derive(Clone, Debug, PartialEq)]
struct Tick {
    tick: u64,
    rows: Vec<TrustRow>,
    /// Withdrawals offered to the tick.
    offered: usize,
    /// Units that left the box: the ground truth, off the store.
    taken: usize,
    /// Withdrawals the tick applied (the command ceiling may refuse the tail).
    applied_moves: usize,
    seats: usize,
    overflow: u32,
    events_dropped: u32,
    /// `EV_TRUST` copies the event ring kept.
    ev_trust: usize,
    /// `EV_GATHER`s the event ring kept: proof the flood was a gather flood.
    ev_gather: usize,
    presence: u8,
    flooded: bool,
}

struct Storm {
    ticks: Vec<Tick>,
    hash: u64,
}

fn world() -> World {
    let mut w = World::new(SEED);
    w.gather = GatherContent::probe_fixture();
    w.build = BuildContent::probe_fixture();
    w.deploy = storm_deploy();
    w.backpack = BackpackContent::probe_fixture();
    w
}

/// Seat everybody on one cell and stand the owner's box. Returns the box's
/// handle and store index.
fn stage(w: &mut World) -> (u32, usize) {
    let (cx, cz) = cell(SEED);
    let (x, z) = center(cx, cz);
    w.dev_spawn = Some((x, z));
    let joins: Vec<Command> = (1..=MAX_PLAYERS as u32)
        .map(|id| Command::Join { id })
        .collect();
    w.tick(&joins);
    assert_eq!(
        w.players.iter().filter(|p| p.active).count(),
        MAX_PLAYERS,
        "every seat filled"
    );
    let at = Body::at(SEED, hv(SEED), x, z);
    for p in w.players.iter_mut() {
        p.body = at;
    }
    let o = slot_of(w, OWNER);
    w.players[o].inv[0] = ItemStack {
        item: 0,
        count: 100,
        cond: 0,
    };
    w.players[o].inv[1] = ItemStack {
        item: BOX_ITEM,
        count: 5,
        cond: 0,
    };
    w.tick(&[Command::Place {
        id: OWNER,
        row: FOUNDATION_ROW,
        cx,
        cz,
        level: 0,
        loc: LOC_PLANE,
        freehand: false,
        plate: 0,
    }]);
    w.tick(&[Command::PlaceDeploy {
        id: OWNER,
        row: BOX_ROW,
        cx,
        cz,
        level: 0,
        loc: LOC_PLANE,
    }]);
    let key = box_key(cx, cz, 0);
    let bi = w
        .deploys
        .box_index(key)
        .expect("the owner's box stood up — the fixture, not the mechanic");
    assert_eq!(w.deploys.boxes()[bi].owner, OWNER, "the box is the owner's");
    (key, bi)
}

/// Refill the box, empty the movers and flooders, and stand one full bag at
/// every flooder's feet. Direct writes, the way `raid_storm.rs` restocks:
/// the storm is about the rings, not the economy.
fn restock(w: &mut World, bi: usize, flood: bool) {
    for s in 0..BOX_SLOTS {
        w.deploys.set_box_slot(
            bi,
            s,
            ItemStack {
                item: GOODS,
                count: STACK,
                cond: 0,
            },
        );
    }
    for p in w.players.iter_mut() {
        if p.active && p.id != OWNER {
            p.inv = [ItemStack::default(); INV_SLOTS];
        }
    }
    if flood {
        let full = [ItemStack {
            item: GOODS,
            count: STACK,
            cond: 0,
        }; INV_SLOTS];
        for f in 0..FLOODERS {
            let id = FIRST_FLOODER + f;
            let b = w.players[slot_of(w, id)].body;
            let tick = w.tick;
            w.backpacks
                .stand_up(
                    &w.backpack,
                    b.qx,
                    b.qy,
                    b.qz,
                    id,
                    &full,
                    tick,
                    &mut w.events,
                )
                .expect("a flooder's bag stood up");
        }
    }
}

fn box_units(w: &World, bi: usize) -> usize {
    (0..BOX_SLOTS)
        .map(|s| w.deploys.box_slot(bi, s).count as usize)
        .sum()
}

/// One storm tick: `flood` puts sixteen `Loot`s first, then `moves`
/// withdrawals from the owner's box.
fn storm_tick(w: &mut World, key: u32, bi: usize, flood: bool, moves: usize) -> Tick {
    restock(w, bi, flood);
    let mut cmds: Vec<Command> = Vec::with_capacity(FLOODERS as usize + moves);
    if flood {
        for f in 0..FLOODERS {
            cmds.push(Command::Loot {
                id: FIRST_FLOODER + f,
            });
        }
    }
    for k in 0..moves {
        cmds.push(withdrawal(k, key));
    }
    let applied_moves = cmds
        .len()
        .min(MAX_COMMANDS_PER_TICK)
        .saturating_sub(if flood { FLOODERS as usize } else { 0 });
    let before = box_units(w, bi);
    w.tick(&cmds);
    let count = |code: u8| w.events.entries().iter().filter(|e| e.code == code).count();
    Tick {
        tick: w.trust.tick(),
        rows: w.trust.rows().to_vec(),
        offered: moves,
        taken: before - box_units(w, bi),
        applied_moves,
        seats: w.trust.seats(),
        overflow: w.trust.overflow(),
        events_dropped: w.events.dropped,
        ev_trust: count(EV_TRUST),
        ev_gather: count(EV_GATHER),
        presence: w.presence_of(OWNER),
        flooded: flood,
    }
}

fn storm() -> Storm {
    let mut w = world();
    let (key, bi) = stage(&mut w);
    let mut ticks = Vec::new();
    for phase in 0..3 {
        match phase {
            0 => {}
            1 => w.tick(&[Command::Leave { id: OWNER }]),
            _ => w.tick(&[Command::Evict { id: OWNER }]),
        }
        for _ in 0..FLOOD_TICKS {
            ticks.push(storm_tick(&mut w, key, bi, true, FLOOD_MOVES));
        }
        for _ in 0..CEILING_TICKS {
            ticks.push(storm_tick(&mut w, key, bi, false, MAX_COMMANDS_PER_TICK));
        }
        ticks.push(storm_tick(&mut w, key, bi, false, OVER_MOVES));
    }
    Storm {
        ticks,
        hash: w.state_hash(),
    }
}

/// Link 3 of `trust.rs`'s derivation. `limits.rs` asserts it at compile
/// time; this names it in `cargo test` output and pins the factor at one.
#[test]
fn the_ring_is_the_command_ceiling_times_one_row() {
    assert_eq!(
        TRUST_ROWS_PER_COMMAND, 1,
        "a TrustSeat is spent by value, so a command mints at most one row; \
         a second means the seat type changed, and the ring must grow with it"
    );
    assert_eq!(
        MAX_TRUST_ROWS_PER_TICK,
        MAX_COMMANDS_PER_TICK * TRUST_ROWS_PER_COMMAND,
        "the trust ring must hold every row a tick can make"
    );
}

/// Link 2: one seat per applied command. The mint (`.seat()`) must appear
/// exactly once in `src/`, inside `World::tick`'s command loop, and the
/// clear (`trust.clear(`) exactly once, in `World::tick` before that loop.
/// A second mint anywhere would make "seats per tick" unbounded by the
/// command ceiling, and the ring's size a guess.
#[test]
fn seats_are_minted_once_inside_the_command_loop() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/src");
    let mut files: Vec<(String, String)> = std::fs::read_dir(dir)
        .expect("read src/")
        .map(|e| e.expect("dir entry").path())
        .filter(|p| p.extension().is_some_and(|x| x == "rs"))
        .map(|p| {
            let name = p.file_name().unwrap().to_string_lossy().into_owned();
            let src = std::fs::read_to_string(&p).expect("read source");
            (name, src)
        })
        .collect();
    files.sort();
    // Liveness: the scan saw the crate, not an empty directory.
    assert!(
        files.len() >= 30,
        "only {} source files scanned",
        files.len()
    );
    assert!(files.iter().any(|(n, _)| n == "world.rs"));

    let mut mints = Vec::new();
    let mut clears = Vec::new();
    for (name, src) in &files {
        let lines: Vec<&str> = src.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            let code = line.trim_start();
            if code.starts_with("//") {
                continue;
            }
            if code.contains(".seat()") {
                mints.push((name.clone(), i, enclosing_fn(&lines, i)));
            }
            if code.contains("trust.clear(") {
                clears.push((name.clone(), i, enclosing_fn(&lines, i)));
            }
        }
    }
    assert_eq!(
        mints.len(),
        1,
        "the trust ring's bound needs exactly one seat mint in src/, found \
         {mints:?}. A second mint is a second way to push a row per command"
    );
    assert_eq!(clears.len(), 1, "exactly one clear, found {clears:?}");
    let (file, at, func) = &mints[0];
    assert_eq!(
        (file.as_str(), func.as_str()),
        ("world.rs", "tick"),
        "the seat is minted in World::tick and nowhere else"
    );
    let (cfile, cat, cfunc) = &clears[0];
    assert_eq!((cfile.as_str(), cfunc.as_str()), ("world.rs", "tick"));

    // Inside the loop that takes at most MAX_COMMANDS_PER_TICK commands: the
    // loop header comes before the mint, and no line between them is at or
    // left of the header's indentation (which would have closed its body).
    let src = &files.iter().find(|(n, _)| n == "world.rs").unwrap().1;
    let lines: Vec<&str> = src.lines().collect();
    let header = (0..*at)
        .rev()
        .find(|&i| {
            lines[i]
                .trim_start()
                .starts_with("for cmd in commands.iter().take(MAX_COMMANDS_PER_TICK)")
        })
        .expect("the mint sits below the command loop's header");
    let indent = |l: &str| l.len() - l.trim_start().len();
    let h = indent(lines[header]);
    for line in &lines[header + 1..*at] {
        assert!(
            line.trim().is_empty() || indent(line) > h,
            "the mint is not inside the command loop: {line:?} closes it first"
        );
    }
    assert!(
        cat < &header,
        "the ring is cleared before the command loop, not after it"
    );
}

/// The name of the `fn` whose body holds line `at`: the nearest line above
/// it that declares one.
fn enclosing_fn(lines: &[&str], at: usize) -> String {
    for i in (0..=at).rev() {
        let t = lines[i].trim_start();
        for prefix in ["fn ", "pub fn ", "pub(crate) fn ", "pub(super) fn "] {
            if let Some(rest) = t.strip_prefix(prefix) {
                let end = rest.find(['(', '<']).unwrap_or(rest.len());
                return rest[..end].to_string();
            }
        }
    }
    String::new()
}

/// **Every trust row survives a saturated event ring**, at every presence.
///
/// Per tick: the ledger holds exactly as many rows as units left the box
/// (ground truth off the store), in command order, each naming its mover,
/// the owner, `TRUST_CONT` and the owner's presence at the time. On a flood
/// tick the event ring is full of gathers before the first withdrawal, so it
/// keeps **none** of the 240 `EV_TRUST` copies. That is the loss the ledger
/// exists for, measured rather than argued.
#[test]
fn every_trust_row_survives_a_saturated_event_ring() {
    let s = storm();
    let mut presences = [false; 4];
    let mut flood_ticks = 0;
    let mut rows_the_ring_lost = 0usize;
    for t in &s.ticks {
        let at = t.tick;
        assert_eq!(t.overflow, 0, "tick {at}: the trust ring overflowed");
        assert!(
            t.seats <= MAX_COMMANDS_PER_TICK,
            "tick {at}: {} seats against a ceiling of {MAX_COMMANDS_PER_TICK}",
            t.seats
        );
        assert_eq!(
            t.taken, t.applied_moves,
            "tick {at}: the fixture expects every withdrawal to land"
        );
        assert_eq!(
            t.rows.len(),
            t.taken,
            "tick {at}: {} units left the box but the ledger holds {} rows",
            t.taken,
            t.rows.len()
        );
        for (k, r) in t.rows.iter().enumerate() {
            let (mover, _) = mover_of(k);
            assert_eq!(
                *r,
                TrustRow {
                    actor: mover,
                    counterparty: OWNER,
                    verb: TRUST_CONT,
                    presence: t.presence,
                },
                "tick {at}: row {k} is not withdrawal {k}"
            );
        }
        presences[t.presence as usize] = true;
        rows_the_ring_lost += t.rows.len() - t.ev_trust;
        if t.flooded {
            flood_ticks += 1;
            assert!(
                t.ev_gather > 0 && t.events_dropped > 0,
                "tick {at}: the gather flood did not saturate the event ring \
                 ({} gathers kept, {} dropped)",
                t.ev_gather,
                t.events_dropped
            );
            assert_eq!(
                t.ev_trust, 0,
                "tick {at}: the flood was supposed to fill the event ring \
                 before the first withdrawal, so no EV_TRUST copy survives"
            );
            assert_eq!(t.rows.len(), FLOOD_MOVES, "tick {at}");
        }
    }
    assert_eq!(flood_ticks, 3 * FLOOD_TICKS, "every phase flooded");
    assert!(
        presences[PRESENCE_AWAKE as usize]
            && presences[PRESENCE_ASLEEP as usize]
            && presences[PRESENCE_GONE as usize],
        "all three presences crossed the saturated ring: {presences:?}"
    );
    assert!(
        rows_the_ring_lost > 3 * FLOOD_TICKS * FLOOD_MOVES,
        "the event ring lost {rows_the_ring_lost} trust rows over the storm, \
         which is fewer than the flood ticks alone should cost it"
    );
}

/// A tick at the command ceiling fills the ring **exactly** and overflows
/// nothing; a tick offered more than the ceiling applies the ceiling and
/// mints no more seats than that. The event ring, meanwhile, keeps one
/// `EV_TRUST` in two, because each trust verb spends two of its seats.
#[test]
fn a_tick_at_the_command_ceiling_fills_the_ring_exactly() {
    let s = storm();
    let ceiling: Vec<&Tick> = s
        .ticks
        .iter()
        .filter(|t| !t.flooded && t.applied_moves == MAX_COMMANDS_PER_TICK)
        .collect();
    assert_eq!(ceiling.len(), 3 * (CEILING_TICKS + 1));
    for t in ceiling {
        assert_eq!(t.rows.len(), MAX_TRUST_ROWS_PER_TICK, "tick {}", t.tick);
        assert_eq!(t.seats, MAX_COMMANDS_PER_TICK, "tick {}", t.tick);
        assert_eq!(t.overflow, 0, "tick {}", t.tick);
        assert_eq!(
            t.ev_trust,
            MAX_EVENTS_PER_TICK / 2,
            "tick {}: EV_MOVED and EV_TRUST alternate, so the event ring keeps \
             half the trust rows the ledger keeps",
            t.tick
        );
    }
    let over: Vec<&Tick> = s.ticks.iter().filter(|t| t.offered == OVER_MOVES).collect();
    assert_eq!(over.len(), 3, "one over-ceiling tick per phase");
    for t in over {
        assert_eq!(
            (t.seats, t.rows.len(), t.taken),
            (
                MAX_COMMANDS_PER_TICK,
                MAX_TRUST_ROWS_PER_TICK,
                MAX_COMMANDS_PER_TICK
            ),
            "tick {}: {OVER_MOVES} offered, so the ceiling is applied, seated and \
             logged, and the tail is the caller's to keep",
            t.tick
        );
    }
}

/// Wall 5 over the derived output: two storms from one seed produce the
/// same rows, tick for tick and field for field, and the same state hash.
#[test]
fn the_ledger_replays_row_for_row() {
    let a = storm();
    let b = storm();
    assert_eq!(a.hash, b.hash, "two identical storms disagreed on the hash");
    assert_eq!(
        a.ticks, b.ticks,
        "two identical storms minted different rows"
    );
}

/// Neither hashed nor saved (`trust.rs`). A world whose last tick minted a
/// row saves, and loads into a world whose ledger is empty — with the same
/// state hash. So the hash never saw the ledger, and the save never carried
/// it. Everybody is put to sleep first: a load puts every body to bed, and a
/// hash over bodies still driving would differ on that bit alone
/// (`tests/worldsave.rs`'s `a_quiet_world`).
#[test]
fn the_ledger_is_neither_hashed_nor_saved() {
    let mut w = world();
    let (key, bi) = stage(&mut w);
    restock(&mut w, bi, false);
    let mut quiet: Vec<Command> = (1..=MAX_PLAYERS as u32)
        .filter(|&id| id != FIRST_MOVER)
        .map(|id| Command::Leave { id })
        .collect();
    w.tick(&quiet);
    // One withdrawal against the sleeping owner, and the mover leaves on the
    // same tick, so the tick ends with every body asleep and one row kept.
    quiet.clear();
    quiet.push(withdrawal(0, key));
    quiet.push(Command::Leave { id: FIRST_MOVER });
    w.tick(&quiet);
    assert_eq!(w.trust.len(), 1, "the last tick minted exactly one row");
    let mut buf = vec![0u8; WORLD_SAVE_MAX_BYTES];
    let n = w.save_world(&mut buf).expect("the world saves");
    let mut back = world();
    back.load(&buf[..n]).expect("and loads");
    assert!(back.trust.is_empty(), "a load must not carry trust rows");
    assert_eq!(
        back.state_hash(),
        w.state_hash(),
        "the loaded world's hash differs from the one that saved it, and the \
         only state its twin still holds is one trust row"
    );
}
