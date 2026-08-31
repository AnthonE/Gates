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
//! 2. **The pose.** `hand_pose` is `viewmodel::grip` — the FIRST-PERSON
//!    grip, and there is only one — composed with the same `pose`, divided
//!    by the rig's uniform scale because the bone hangs under a scaled
//!    root. The ratio is 1.0 today, so a missing division is invisible
//!    until the rig is re-measured, which is exactly the class of bug that
//!    ships. **The constant this replaced was 0.690 m wrong and on the
//!    wrong shoulder** (`bodies::RETIRED_BODY_PALM`), and it was wrong for
//!    the whole life of the feature because it was a second, independent
//!    answer to a question the tree had already measured once — so what is
//!    gated here is the SHARING and not a number.
//! 3. **The spawn shape and the re-parent**, as call sites rather than
//!    values (`tests/sound.rs`' rule, and `CLAUDE.md`: a spawn is not
//!    type-checked). Whether the two entities are children of the body
//!    root, and whether `bind_hands` then moves them onto the hand bone,
//!    are claims about a hierarchy — and every value above would still be
//!    right if they were siblings floating at the origin.
//!
//! **Not gated here and said plainly:** nobody has seen a remote hand.
//! No capture in this repo contains two players (`NOW.md` §0tl, `§LOOK`).

#![cfg(feature = "render")]

use bevy::prelude::*;
use client::render::bodies::{flame_pose, hand_pose, hand_wants, RETIRED_BODY_PALM};
use client::render::viewmodel::{grip, VIEWMODEL_PALM};
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
fn the_remote_hand_uses_the_first_persons_grip_and_no_second_copy() {
    // **The gate the retired constant did not have.** `BODY_PALM` was an
    // independent second answer to "where does a held thing sit", and being
    // independent is what let it be 0.690 m wrong on the wrong shoulder for
    // as long as remote hands existed — while every assertion about it
    // passed, because they all measured it against itself.
    //
    // There is one grip now and this is what says so: `hand_pose` must be
    // `viewmodel::grip` composed with the same `pose` the first-person hand
    // uses, and a re-introduced offset of its own fails here rather than in
    // a screenshot nobody takes.
    for &scale in &[1.0f32, 0.5, 2.0] {
        for (i, def) in HELD_MODELS.iter().enumerate() {
            let mut g = grip();
            g.translation /= scale;
            g.scale /= scale;
            let want = g * client::render::viewmodel::pose(def, VIEWMODEL_PALM);
            let got = hand_pose(i, scale);
            // Compared componentwise rather than with `Quat::angle_between`:
            // that is `acos(|dot|)`, and the dot of a float quaternion with
            // an exact copy of itself lands a rounding step ABOVE 1, so the
            // arccos is NaN and every comparison against it is false. A
            // gate whose failure mode is "identical values are not equal"
            // is worse than no gate; this is the same two-decoders lesson
            // one type down.
            let same_rot = (0..4).all(|k| {
                let (a, b) = (got.rotation.to_array()[k], want.rotation.to_array()[k]);
                (a - b).abs() < 1e-5 || (a + b).abs() < 1e-5
            });
            assert!(
                got.translation.distance(want.translation) < 1e-5
                    && same_rot
                    && (got.scale - want.scale).length() < 1e-6,
                "{}: hand_pose is not the shared grip composed with the shared \
                 pose (scale {scale})\n  got  {got:?}\n  want {want:?}",
                def.key
            );
        }
    }
}

#[test]
fn the_grip_lands_in_the_fist_at_any_rig_scale() {
    // The same claim the old gate made, one frame further down the chain:
    // the model's grip point — `grip_m` up its own +Y — lands on the palm.
    // What changed is the frame it lands in. It used to be `BODY_PALM` in
    // the body's own space, a place picked by description; it is
    // `VIEWMODEL_PALM` in the HAND BONE's space now, which is the wrist-to-
    // palm correction of an actual fist and is the same centimetres the
    // first-person hand uses.
    //
    // **The bone's own transform cancels out of this**, which is why no GLB
    // is opened here: both sides of the composition are expressed in the
    // bone's frame, so what is left to check is the arithmetic. Where that
    // bone actually is, and that `RETIRED_BODY_PALM` is nowhere near it, is
    // `tests/viewmodel_arms.rs`' — it owns the file reader.
    for &scale in &[1.0f32, 0.5, 2.0] {
        for (i, def) in HELD_MODELS.iter().enumerate() {
            let t = hand_pose(i, scale);
            let mut g = grip();
            g.translation /= scale;
            g.scale /= scale;
            // Walk the model's own grip point out to the bone's frame, then
            // back into the hold frame the palm is expressed in.
            let grip_local = Vec3::Y * def.grip_m() / def.scale;
            let in_bone = t.translation + t.rotation * (grip_local * t.scale);
            let in_hold = g
                .rotation
                .inverse()
                .mul_vec3((in_bone - g.translation) / g.scale.x);
            assert!(
                in_hold.distance(VIEWMODEL_PALM) < 1e-4,
                "{}: the grip point landed at {in_hold:?} in the hold frame, \
                 not on the palm {VIEWMODEL_PALM:?} (scale {scale})",
                def.key
            );
            // And the item ends up its own size in the world. The bone's
            // global scale is the glTF root's 0.01 times the rig's own, so
            // the composed local scale has to cancel both.
            let world = t.scale.x * scale * 0.01;
            assert!(
                (world - def.scale).abs() < 1e-5,
                "{}: world scale {world} against {} (rig scale {scale})",
                def.key,
                def.scale
            );
        }
    }
}

/// The flame's offset uses the same grip, and it is derived from the mesh
/// rather than typed — so a regenerated torch moves the light with it.
/// Asserted against the row's own `flame_m`, which is the number
/// `viewmodel::apply_hand_light` uses for the first-person hand: **one flame
/// height, one grip, two hands.**
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
        let mut g = grip();
        g.translation /= scale;
        g.scale /= scale;
        let t = flame_pose(def.flame_m(), scale);
        // Back into the hold frame: the lift is straight up its +Y and
        // nothing else.
        let in_hold = g
            .rotation
            .inverse()
            .mul_vec3((t.translation - g.translation) / g.scale.x);
        assert!(
            (in_hold - Vec3::Y * def.flame_m()).length() < 1e-4,
            "the flame sits at {in_hold:?} in the hold frame, not {:?} up it \
             (scale {scale})",
            def.flame_m()
        );
        assert!(def.flame_m() > 0.0, "a flame above the fist, not in it");
    }
    // An unlit hand parks the emitter back at the fist rather than leaving
    // it where the last flame was — `update_hand` passes a zero lift.
    let mut g = grip();
    g.translation /= 1.0;
    assert_eq!(flame_pose(0.0, 1.0).translation, g.translation);
}

/// The retired constant is kept, and it is kept for one reason.
#[test]
fn the_retired_offset_is_not_reachable_from_the_draw_path() {
    let src = std::fs::read_to_string("src/render/bodies.rs").expect("bodies.rs");
    // Comments stripped, and the declaration itself skipped: what this
    // forbids is a READ, and `pub const RETIRED_BODY_PALM` is the write.
    let code: String = src
        .lines()
        .map(|l| l.split("//").next().unwrap_or(""))
        .filter(|l| !l.contains("pub const RETIRED_BODY_PALM"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !code.contains("RETIRED_BODY_PALM"),
        "RETIRED_BODY_PALM is referenced by bodies.rs' code — it is a \
         retraction kept for the gate, not an offset to draw with"
    );
    // And it is still the number that shipped, so the measurement in
    // `tests/viewmodel_arms.rs` is about what was actually wrong.
    assert_eq!(RETIRED_BODY_PALM, Vec3::new(0.22, 1.25, 0.18));
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

    // **And both are MOVED onto the hand bone**, which is the half the
    // spawn cannot express and the whole of what 2026-08-31 changed. They
    // are still spawned under the root (they have to exist from the body's
    // first frame — `HeldOnBody`'s doc) and `bind_hands` re-parents them
    // when the scene lands. Deleting either insert leaves every value above
    // correct and puts the axe back at the body's feet at a hundred times
    // its size, with no gate but this one pointing at it.
    let code: String = src
        .lines()
        .map(|l| l.split("//").next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n");
    let bind = code
        .split_once("pub fn bind_hands(")
        .expect("bind_hands is gone — this gate is stale")
        .1;
    let bind = bind.split_once("\npub fn ").map_or(bind, |(b, _)| b);
    for what in ["live.hand", "live.flame"] {
        assert!(
            bind.contains(&format!("commands.entity({what}).insert(ChildOf(bone))")),
            "bind_hands no longer re-parents {what} onto the hand bone"
        );
    }
    assert!(
        bind.contains("live.bone = Some(bone)") && bind.contains("live.held = None"),
        "bind_hands must record the bone AND forget what the hand was \
         showing, or the item keeps the root-space pose it had while unbound"
    );
}
