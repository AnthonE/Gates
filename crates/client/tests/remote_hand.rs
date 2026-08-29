//! Gate: another player's hand — the item in it, the flame on it, and
//! where both of them hang.
//!
//! **The gap this closes was ranked first by two consecutive judges**
//! (`findings/pass-20260829-153230-04-judge.md` and `-05`): every remote
//! body was drawn empty-handed and dark, so a bow, a hatchet and a
//! burning torch all read as the same silhouette. `server/tests/
//! remote_hand.rs` owns the wire half — whether the two facts leave the
//! shard. This file owns what happens to them after they arrive.
//!
//! Three things, and no other gate in this crate can see any of them:
//!
//! 1. **The decision.** `bodies::hand_wants` is one function over the two
//!    fields wire v56 added plus `dead`, and it is where a corpse drops
//!    what it was holding. Split out of the system for
//!    `viewmodel::apply_hand_light`'s reason — a decision a gate cannot
//!    call is a decision nothing checks.
//! 2. **The pose.** `hand_pose` divides `viewmodel::pose` by the rig's
//!    uniform scale, because these are children of a scaled root. The
//!    ratio is 1.0 today, so a missing division is invisible until the
//!    rig is re-measured — which is exactly the class of bug that ships.
//! 3. **The spawn shape**, as a call site rather than a value
//!    (`tests/sound.rs`' rule, and `CLAUDE.md`: a spawn is not
//!    type-checked). Whether the two entities are children of the body
//!    root is a claim about a bundle, and every value in them would still
//!    be right if they were siblings floating at the origin.
//!
//! **Not gated here and said plainly:** nobody has seen a remote hand.
//! No capture in this repo contains two players (`NOW.md` §0tl, `§LOOK`).

#![cfg(feature = "render")]

use bevy::prelude::*;
use client::render::bodies::{hand_pose, hand_wants, BODY_PALM};
use client::ui::hold::{HELD_MODELS, TORCH_LIGHT};
use client_core::interp::RemoteState;
use protocol::ItemCatalog;

/// A catalog whose ids are the index of the name given, so a test can say
/// "item 3" and mean the fourth name here.
fn catalog(names: &[&str]) -> ItemCatalog {
    let mut c = ItemCatalog::EMPTY;
    for (i, n) in names.iter().enumerate() {
        c.set(i, n.as_bytes(), protocol::ItemRow::EMPTY)
            .expect("a short name fits");
    }
    c.count = names.len() as u16;
    c
}

fn remote(held: Option<u16>, lit: bool) -> RemoteState {
    RemoteState {
        held,
        lit,
        ..RemoteState::default()
    }
}

/// The row index of a key in `HELD_MODELS`, so the assertions below name
/// items rather than numbers.
fn row(key: &str) -> usize {
    HELD_MODELS
        .iter()
        .position(|m| m.key == key)
        .unwrap_or_else(|| panic!("no held row for {key:?}"))
}

/// **1 · The decision, one fact at a time.** Each line is a case the
/// wire distinguishes and the drawing has to.
#[test]
fn the_hand_reads_the_two_fields_and_the_corpse_bit() {
    let c = catalog(&["Torch", "Stone Hatchet", "Cloth"]);

    // Empty hand: nothing drawn, nothing lit. The commonest state in the
    // game, and it must not fall back to a stand-in — a tool that appears
    // on someone who is holding nothing is a lie about the one thing an
    // encounter opens by reading.
    assert_eq!(hand_wants(&c, &remote(None, false)), (None, None));
    // …and an empty hand cannot be lit even if the wire says so. This is
    // not a case the server produces (`is_lit` needs a stack), so it is
    // here as a *decoder-hostile* one: a forged datagram must not put a
    // light on a body holding nothing.
    assert_eq!(hand_wants(&c, &remote(None, true)), (None, None));

    // An unlit torch: drawn, dark.
    assert_eq!(
        hand_wants(&c, &remote(Some(0), false)),
        (Some(row("torch")), None)
    );
    // A lit one: drawn and burning — the same row twice, because the
    // light hangs off the item that declares it.
    assert_eq!(
        hand_wants(&c, &remote(Some(0), true)),
        (Some(row("torch")), Some(row("torch")))
    );

    // An item with no light row never lights, whatever the wire claims.
    // `light_burn` and `HeldLight` are two spellings of one content fact
    // and this is what happens when they disagree: the item draws and
    // nothing glows, rather than a glow from nowhere.
    assert_eq!(
        hand_wants(&c, &remote(Some(1), true)),
        (Some(row("stone_hatchet")), None)
    );

    // An item with no MODEL draws nothing and lights nothing.
    assert_eq!(hand_wants(&c, &remote(Some(2), true)), (None, None));

    // An id past the catalog: nothing. The wire refuses this band
    // (`protocol::read_held`), so reaching here means a catalog that has
    // not finished dripping — a real state on join, and it must be an
    // empty hand rather than a panic or a wrong row.
    assert_eq!(hand_wants(&c, &remote(Some(40), true)), (None, None));

    // **A corpse drops it, a sleeper does not.** The wire sends both
    // bodies' hands; only one of them is upright.
    let mut dead = remote(Some(0), true);
    dead.dead = true;
    assert_eq!(hand_wants(&c, &dead), (None, None));
    let mut asleep = remote(Some(0), true);
    asleep.sleeping = true;
    assert_eq!(
        hand_wants(&c, &asleep),
        (Some(row("torch")), Some(row("torch"))),
        "a sleeping body is still standing up and still holding it"
    );
}

/// **2 · The pose, in the frame it is actually written into.**
///
/// `hand_pose` answers in the body root's LOCAL space, and that root
/// carries the rig's uniform scale — so everything it returns is divided
/// by it. The ratio is `ANIM_BODY_H_M / ANIM_RIG_H_M` = 1.0 today, which
/// is why this is asserted at a scale that is *not* 1.0: a missing
/// division is bit-identical on the shipped rig and wrong the day the rig
/// is re-measured.
#[test]
fn the_grip_lands_in_the_fist_at_any_rig_scale() {
    for &scale in &[1.0f32, 0.5, 2.0] {
        for (i, def) in HELD_MODELS.iter().enumerate() {
            let t = hand_pose(i, scale);
            // The model's grip point is `grip_m` up its own +Y; the pose
            // rotates the model and slides it so that point lands on the
            // fist. Walk it back through the transform and it must be
            // exactly `BODY_PALM`, in world metres.
            let grip_local = Vec3::Y * def.grip_m() / def.scale;
            let world = (t.translation + t.rotation * (grip_local * t.scale)) * scale;
            assert!(
                world.distance(BODY_PALM) < 1e-4,
                "{}: grip landed at {world:?}, not {BODY_PALM:?} (scale {scale})",
                def.key
            );
            // And the item ends up its own size in the world, not the
            // rig's size times its own.
            assert!(
                (t.scale.x * scale - def.scale).abs() < 1e-5,
                "{}: world scale {} against {} (rig scale {scale})",
                def.key,
                t.scale.x * scale,
                def.scale
            );
        }
    }
}

/// The flame's offset uses the same divisor, and it is derived from the
/// mesh rather than typed — so a regenerated torch moves the light with
/// it. Asserted against the row's own `flame_m`, which is the number
/// `viewmodel::apply_hand_light` uses for the first-person hand: one
/// flame height, two hands.
#[test]
fn the_flame_sits_above_the_fist_by_the_rows_own_lift() {
    let i = row("torch");
    let def = &HELD_MODELS[i];
    assert!(def.light.is_some(), "the torch is the row with a light");
    assert_eq!(
        def.light.expect("checked"),
        TORCH_LIGHT,
        "the row declares the ladder's torch, not a second copy of it"
    );
    for &scale in &[1.0f32, 0.5, 2.0] {
        // What `update_hand` writes, restated: palm plus the row's lift,
        // in the root's local frame.
        let local = (BODY_PALM + Vec3::Y * def.flame_m()) / scale;
        let world = local * scale;
        assert!(
            world.y > BODY_PALM.y,
            "the flame is above the fist, not in it"
        );
        assert!((world.y - BODY_PALM.y - def.flame_m()).abs() < 1e-4);
        assert!((world.x - BODY_PALM.x).abs() < 1e-4);
    }
}

/// **3 · The spawn shape, as a call site.**
///
/// Both hand entities must be children of the body root: that is what
/// makes them inherit the body's position, its facing and its despawn,
/// and it is the one property no value in them can express. A version
/// that spawned them at the world origin would satisfy every assertion
/// above and draw every item in the game at (0, 0, 0).
///
/// Scraped rather than driven, `tests/sound.rs`' rule: the defect would
/// be *where* the spawn happens, and a Bevy app that could observe it
/// needs a GPU this box does not have.
#[test]
fn the_hand_and_the_flame_hang_off_the_body() {
    let src = std::fs::read_to_string("src/render/bodies.rs").expect("bodies.rs");
    let with = src
        .find("with_children")
        .expect("the hand entities are spawned as children of the body root");
    let block = &src[with..];
    let end = block.find("store.live.insert").expect("the block closes");
    let block = &block[..end];
    for name in ["HeldOnBody", "BodyFlame"] {
        assert!(
            block.contains(name),
            "{name} is not spawned inside the body's `with_children` block — \
             a sibling inherits neither the body's transform nor its despawn"
        );
    }
    // And nothing else spawns them, so the block above is the whole
    // claim rather than one of two sites.
    for name in ["HeldOnBody", "BodyFlame"] {
        assert_eq!(
            src.matches(&format!("{name},")).count(),
            1,
            "{name} is spawned in more than one place"
        );
    }
    // The flame is a sibling of the item and not a child of it — see
    // `BodyFlame`'s doc: a light under the item's transform would have
    // its offset scaled by the model's in-hand cheat.
    let hand_at = block.find("HeldOnBody").expect("checked above");
    let flame_at = block.find("BodyFlame").expect("checked above");
    assert!(
        block[hand_at..flame_at].matches("with_children").count() == 0,
        "the flame is nested under the held item"
    );
}
