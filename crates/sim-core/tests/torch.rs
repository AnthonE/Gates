//! Gate: a torch **burns** while it is held up, and it can be put out.
//!
//! `light.rs`'s own unit tests own the arithmetic — the three facts, the
//! cadence, the one-point ceiling, the shipped torch's five minutes. This
//! file owns what that module cannot reach: whether any of it is *wired*.
//!
//! The distinction is not academic. `light::step` was a correct, gated,
//! fully mutant-checked function for the length of one edit before
//! `World::tick` called it, and every gate in this crate was green over a
//! game where nothing burned. So what is asserted here is the path a
//! player's press actually takes — `Command::Input` carrying `BTN_LIGHT`,
//! through `world::apply`'s button mask, into the per-player sweep, out as
//! a smaller `cond` on the held stack — plus the two things that path
//! crosses on its way: the mask (a bit outside `BTN_MASK` is erased, and
//! this one had better be inside it) and the save file (a remainder that
//! did not survive a restart would refund six seconds of flame per
//! reconnect, `persist.rs`'s stated reason for saving `food_acc`).
//!
//! **Not gated here and said plainly:** nobody has seen a lit torch, and
//! no gate in this repo can — `rig::CAPTURE_DAY_FRAC` pins every capture
//! to noon (`NOW.md` §0tl item 1, `§LOOK`).

use sim_core::gather::{GatherContent, ItemStack};
use sim_core::input::{InputFrame, BTN_LIGHT, BTN_PRIMARY};
use sim_core::light::BURN_DEN;
use sim_core::limits::TICK_HZ;
use sim_core::world::{Command, World};
use sim_core::worldsave::WORLD_SAVE_MAX_BYTES;

const SEED: u64 = 20260829;

/// One player holding the fixture's light (item 0) at full condition.
///
/// The stack is written straight into the record rather than gathered for,
/// because what is under test is the burn and not the loot roll — and a
/// gather would wear the tool it paid with, which is the one other thing
/// in this game that moves `cond`.
fn lit_world() -> World {
    let mut w = World::new(SEED);
    w.gather = GatherContent::probe_fixture();
    w.tick(&[Command::Join { id: 1 }]);
    let slot = w
        .players
        .iter()
        .position(|p| p.active && p.id == 1)
        .expect("the join seated a body");
    w.players[slot].inv[0] = ItemStack {
        item: 0,
        count: 1,
        cond: 400,
    };
    w
}

fn body(w: &World) -> &sim_core::world::Player {
    w.players
        .iter()
        .find(|p| p.active && p.id == 1)
        .expect("the body is still seated")
}

fn frame(seq: u16, buttons: u8) -> InputFrame {
    InputFrame {
        seq,
        buttons,
        sel: 0,
        ..InputFrame::default()
    }
}

/// Hold the latch for `n` ticks and answer with what the held stack has
/// left.
fn hold(w: &mut World, buttons: u8, n: u32) -> u16 {
    for t in 0..n {
        w.tick(&[Command::Input {
            id: 1,
            frame: frame(t as u16, buttons),
            favour: 0,
        }]);
    }
    body(w).inv[0].cond
}

/// The whole path, once: a press reaches the sim and costs the player
/// something.
#[test]
fn holding_the_latch_spends_the_torch() {
    let mut w = lit_world();
    let period = BURN_DEN / GatherContent::probe_fixture().light_burn_of(0) as u32;
    let left = hold(&mut w, BTN_LIGHT, period);
    assert_eq!(
        left, 300,
        "a whole point off the held stack after {period} lit ticks — the \
         press is not reaching `light::step`"
    );
}

/// And the other half of the same claim, which is the one `ALPHA.md` §1 is
/// actually asking for: **you can put it out.**
#[test]
fn a_torch_that_is_not_held_up_costs_nothing() {
    let mut w = lit_world();
    let period = BURN_DEN / GatherContent::probe_fixture().light_burn_of(0) as u32;
    // Four full periods of swinging, sprinting and standing still.
    let left = hold(&mut w, BTN_PRIMARY, period * 4);
    assert_eq!(
        left, 400,
        "an unlit torch burned — carrying one must be free, or the \
         tradeoff is a tax instead of a choice"
    );
    // And it lights again from where it was left, on the very next press:
    // the latch is a latch, not a one-shot verb.
    let left = hold(&mut w, BTN_LIGHT, period);
    assert_eq!(left, 300);
}

/// `BTN_LIGHT` is inside `BTN_MASK`, checked where it bites rather than by
/// reading the constant.
///
/// `world::apply` masks a non-wire frame's buttons with `BTN_MASK` and the
/// server's `accept_input` drops a whole datagram carrying a bit outside
/// it. A button declared and left out of the mask therefore does nothing
/// at all while compiling perfectly — `input.rs` says so in as many words
/// ("a new button joins this mask in the same commit that declares its
/// bit"), and this is that sentence as an assertion: the stored frame
/// keeps the bit, and the burn above is what says the bit is read.
#[test]
fn the_light_bit_survives_the_button_mask() {
    let mut w = lit_world();
    w.tick(&[Command::Input {
        id: 1,
        frame: frame(1, BTN_LIGHT),
        favour: 0,
    }]);
    assert_eq!(
        body(&w).frame.buttons & BTN_LIGHT,
        BTN_LIGHT,
        "`world::apply` masked `BTN_LIGHT` off the stored frame, so the \
         bit is declared but not meant"
    );
}

/// The remainder crosses the save file.
///
/// A torch three-quarters of the way to its next point, saved and loaded,
/// must be three-quarters of the way there — not back at zero. Otherwise
/// every reconnect is a free fraction of a torch, which is the exact
/// arithmetic `persist.rs` refuses for `food_acc`, and the exploit is
/// cheaper here because relighting costs nothing.
#[test]
fn a_saved_body_keeps_the_fraction_of_a_point_it_had_burned() {
    let mut w = lit_world();
    let period = BURN_DEN / GatherContent::probe_fixture().light_burn_of(0) as u32;
    // Stop one tick short of the point falling.
    hold(&mut w, BTN_LIGHT, period - 1);
    let want = body(&w).light_acc;
    // Exactly one tick short of a point, and **past `u16`** — 179 400 —
    // so a codec that narrowed the field would fail this before the
    // round trip below ever ran.
    assert_eq!(
        want,
        BURN_DEN - GatherContent::probe_fixture().light_burn_of(0) as u32
    );
    assert!(
        want > u16::MAX as u32,
        "the probe stopped covering the width"
    );
    assert_eq!(body(&w).inv[0].cond, 400, "and no point has fallen yet");

    let mut buf = vec![0u8; WORLD_SAVE_MAX_BYTES];
    let n = w.save_world(&mut buf).expect("a live world encodes");
    let mut back = World::new(SEED);
    back.gather = GatherContent::probe_fixture();
    back.load(&buf[..n]).expect("its own bytes must load");

    assert_eq!(
        body(&back).light_acc,
        want,
        "the torch's remainder did not survive the file — a reconnect \
         would refund it"
    );
    // A save puts an awake body to bed (`worldsave.rs`), so this is a
    // sleeper — and `Command::Wake` is what a returning player sends. One
    // tick of the latch after that, and the point the remainder was one
    // tick short of falls.
    back.tick(&[Command::Wake { id: 1, sleeper: 1 }]);
    assert!(
        !body(&back).sleeping,
        "the wake seated the returning player"
    );
    let left = hold(&mut back, BTN_LIGHT, 1);
    assert_eq!(
        left, 300,
        "the restored remainder was one tick from a point and did not \
         spend it — a reload that zeroed it would cost this assertion \
         `period` more ticks"
    );
}

/// A sleeper's torch is out, and its inventory is not being spent while
/// nobody is there to see the light.
#[test]
fn a_sleeper_burns_nothing() {
    let mut w = lit_world();
    let period = BURN_DEN / GatherContent::probe_fixture().light_burn_of(0) as u32;
    // Light it, then log off holding it up.
    hold(&mut w, BTN_LIGHT, 1);
    w.tick(&[Command::Leave { id: 1 }]);
    assert!(
        body(&w).sleeping,
        "the leave made a sleeper, not a deletion"
    );
    for _ in 0..period * 4 {
        w.tick(&[]);
    }
    assert_eq!(
        body(&w).inv[0].cond,
        400,
        "an offline body burned its own torch down"
    );
}

/// The five minutes, end to end through the tick loop, on the fixture's
/// own rate rather than the shipped one — `light.rs`'s
/// `the_shipped_torch_is_five_minutes_of_light` owns the content number,
/// and this owns that a torch actually reaches zero and stays there.
#[test]
fn a_torch_burns_out_and_stays_out() {
    let mut w = lit_world();
    let rate = GatherContent::probe_fixture().light_burn_of(0) as u32;
    // Four whole points at the fixture's rate — the debit is in POINTS.
    let ticks = 4 * BURN_DEN / rate;
    assert_eq!(ticks, 4 * 300);
    assert_eq!(hold(&mut w, BTN_LIGHT, ticks), 0, "spent");
    // Ten more seconds of holding a dead stick.
    assert_eq!(
        hold(&mut w, BTN_LIGHT, 10 * TICK_HZ),
        0,
        "a spent torch must not go negative or wrap"
    );
    assert_eq!(
        body(&w).inv[0].count,
        1,
        "and the stick stays in the hand — a dead torch is an item you \
         are still carrying, exactly as a dead tool is (`gather`'s Q4)"
    );
}
