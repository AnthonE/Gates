//! The in-world keys: what the crosshair is on, and what `E`, `J` and `H` do
//! about it.
//!
//! **Twelve of the wire's sixteen action verbs had no key in this client.**
//! `ACT_USE`, `ACT_LOOT`, `ACT_CONTAINER`, `ACT_DRINK` and `ACT_FEED` are the
//! five this module lands; the sim has gated and tested all of them since M1
//! and the native client simply never sent one. The container panel was the
//! sharpest case — `panels/mod.rs` says it "opens itself when the sim says
//! one is open", and nothing existed that could tell the sim to open one, so
//! ~500 lines of drawn panel were unreachable.
//!
//! **Bevy draws, it does not decide** (`RENDER.md` §1): every question with
//! arithmetic in it is [`crate::ui::interact`]'s, which is pure and gated in
//! the code tier. This file reads keys, calls that resolver, and hands the
//! encoder's bytes to the session. It owns no reach, no cone and no tiebreak.
//!
//! ## Why the pick is resolved every frame and not on the keypress
//!
//! The prompt has to name the thing `E` will act on, and a prompt computed
//! from different inputs than the dispatch is a prompt that can lie. So the
//! pick is resolved once per frame into [`Aimed`], the prompt draws it, and
//! the keypress reads the same value rather than resolving again. The browser
//! resolves twice — once on the HUD timer at 4 Hz and again inside `tryUse` —
//! and gets away with it only because the resolver is a pure function of a
//! world that rarely moves between the two.

use bevy::prelude::*;
use sim_core::inventory::{CONT_BAG, CONT_BOX, CONT_SELF, CONT_WORLD};

use crate::look::yaw_u16;
use crate::ui::interact::{self, Aim, Pick, SwingAim, SwingPick, Verb};
use crate::ui::structure::{self, Store, Target};

use super::hud::Toast;
use super::input::Look;
use super::panels::{Panel, Ui};
use super::Net;

/// What the crosshair is on this frame. Written by [`resolve`], read by the
/// prompt and by [`keys`] — one value, so the two cannot disagree.
#[derive(Resource, Default)]
pub struct Aimed(pub Pick);

/// What a SWING would hit this frame — the scatter's answer, where [`Aimed`]
/// is the deployables'. Its own resource for the same reason [`Near`] is:
/// the left button and `E` address different worlds, and one value read by
/// both the prompt and the swing keeps them from disagreeing about which.
#[derive(Resource, Default)]
pub struct Swung(pub SwingPick);

/// Whether the player stands in the announced weak sector of the node the
/// crosshair is on. Resolved beside the pick because it is the same frame's
/// answer to the same question — where you are standing, and what that buys.
#[derive(Resource, Default)]
pub struct InWeak(pub bool);

/// The code lock's keypad, when one is up (lock v1). A resource rather
/// than a `Panel` because it deliberately does **not** grab the pointer:
/// the door is in front of you and the person on the other side of it is
/// not waiting (`crate::ui::keypad` has the argument).
#[derive(Resource, Default)]
pub struct Pad(pub crate::ui::keypad::Keypad);

/// The nearest structure, either store. Its own resource beside [`Aimed`]
/// because `L`, `U`, `R` and the raid verb address a structure and `E` does
/// not — see `ui::structure`'s header for why they cannot share a metric.
#[derive(Resource, Default)]
pub struct Near(pub Option<Target>);

/// Resolve the pick from the predictor's own position and the sim's own
/// bearing.
///
/// **The bearing is quantized first**, and that is not a detail: the sim
/// faces one of 256 bearings (`yaw_lut.rs`) and gates every one of these
/// verbs on the direction it holds, not on the client's free-running float.
/// A client that resolved on the unquantized yaw would offer a verb the
/// server declines at the edge of the aim radius — the quantize-both-sides
/// law (`CLAUDE.md`) applied to aiming, which is exactly what the browser's
/// `aimDir` does for the same reason.
/// `NonSendMut` rather than `NonSend` for one reason: the swing pick reads
/// the island through `ClientCore`'s own `SlotCache` — the predictor's, warm
/// with the very cells this frame's movement step just resolved — and a memo
/// is written to. It costs no extra scheduling: `keys` below already takes
/// the session mutably, so these two were serialized before this line changed.
pub fn resolve(
    mut net: NonSendMut<Net>,
    look: Res<Look>,
    mut aimed: ResMut<Aimed>,
    mut near: ResMut<Near>,
    mut swung: ResMut<Swung>,
    mut in_weak: ResMut<InWeak>,
    world: Option<Res<crate::render::WorldId>>,
) {
    let core = &mut net.session.core;
    let [x, y, z] = core.predict.render_position();
    let (fx, fz) = sim_core::yaw_dir(yaw_u16(look.yaw));
    aimed.0 = interact::resolve(
        Aim::new(x, z, fx, fz),
        core.deploys.entries(),
        &core.deploy_defs,
        core.deploy_defs_have,
        core.bags.entries(),
    );
    // The one field the resolver cannot fill: it is handed the deploy
    // records, and whether a fire is burning is deliberately not on one
    // (`client-core/core.rs`). Stamped here, where the core is in hand.
    aimed.0.lit = matches!(
        aimed.0.verb,
        interact::Verb::Fire | interact::Verb::Recycler
    ) && core.ovens().is_lit(aimed.0.cx, aimed.0.cz, aimed.0.level);
    // The weak-spot chase, read before the island borrows the core mutably.
    // Both are `Copy` scalars, so this is a read and not a hold.
    let (mark_cell, mark8) = (core.mark_cell, core.mark8);
    // The scatter pick needs the island, which does not exist until the
    // welcome names a seed — so this is `Option` and stands down rather than
    // guessing one. `render::world_placed`'s discipline, applied to a verb.
    //
    // The island comes from `ClientCore::island` rather than from `WorldId`,
    // and the two are the same triple derived twice from the same seed. The
    // core's copy is the one that owns the cache, and handing the cache a
    // *second* seed would flush it every frame — see that method's header.
    // `w` is still what says a world exists at all.
    (swung.0, in_weak.0) = match world.as_deref() {
        Some(_) => {
            let (seed, occ) = core.island();
            let mut island = interact::Island {
                seed,
                table: occ.table,
                haven: occ.haven,
                harvested: occ.harvested,
                cache: occ.cache,
            };
            let pick = interact::resolve_swing(SwingAim { x, y, z, fx, fz }, &mut island);
            // The open pick, on the same island borrow and the same aim.
            // Folded into `aimed` rather than kept beside it, because to
            // the player there is one `E` and one centre prompt — a
            // second pick with its own key would teach two verbs for one
            // gesture. It loses every tie to a deployable on purpose: a
            // box placed against a waystation cache is a thing a player
            // built and meant, and the authored container is not going
            // anywhere.
            if aimed.0.is_none() {
                let open = interact::resolve_open(SwingAim { x, y, z, fx, fz }, &mut island);
                if open.occupant != 0 {
                    aimed.0 = interact::Pick {
                        verb: interact::Verb::Crate,
                        cx: open.cx,
                        cz: open.cz,
                        // The wire handle is the cell key — the same value
                        // `EV_SLOT_HARVESTED` carries for this cell, and
                        // the same one `worldcont::index_of` resolves.
                        handle: sim_core::gather::cell_key(open.cx, open.cz),
                        d2: open.d2,
                        aimed: true,
                        ..Default::default()
                    };
                }
            }
            // The weak sector, but only for the node actually aimed at: the
            // chase is per-node and the server restarts it when the player
            // switches targets, so a mark for the tree behind you says
            // nothing about this one. The cell was just resolved by the scan
            // above, so this read is a cache hit by construction.
            let weak = if interact::mark_is_for(mark_cell, &pick) {
                let s = island.slot(pick.cx as i32, pick.cz as i32);
                interact::in_weak_sector(x, z, s.x, s.z, mark8)
            } else {
                false
            };
            (pick, weak)
        }
        None => (SwingPick::default(), false),
    };
    near.0 = structure::nearest(
        (x, z),
        core.pieces.entries(),
        &core.piece_defs,
        core.piece_defs_have,
        core.deploys.entries(),
        &core.deploy_defs,
        core.deploy_defs_have,
    );
}

/// `E`, `J`, `H`.
///
/// Runs in `Screen::InWorld` only and stands down while a panel owns the
/// pointer, for `input::gather`'s reason: every verb here spends something —
/// a swing, a door, a mouthful — and a player typing into the craft search
/// box asked for none of it.
// Seven, and each is a distinct source: the keyboard, the session, the two
// picks, the toast, the panels, and the chat composer.
#[allow(clippy::too_many_arguments)]
pub fn keys(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut net: NonSendMut<Net>,
    aimed: Res<Aimed>,
    near: Res<Near>,
    mut toast: ResMut<Toast>,
    mut pad: ResMut<Pad>,
    ui: Option<ResMut<Ui>>,
    chat: Option<Res<super::chat::Chat>>,
) {
    let mut ui = ui;
    if ui
        .as_ref()
        .map(|u| u.panel.grabs_pointer())
        .unwrap_or(false)
        || chat.map(|c| c.open()).unwrap_or(false)
    {
        return;
    }
    // `R` and `F` belong to the build ghost while the wheel is up, and to
    // repair otherwise. The ordering IS the binding — `web/src/main.js` pins
    // the same precedence in a gate for the same reason — so the wheel's
    // claim is checked here rather than left to whoever reorders these next.
    let wheel_up = ui
        .as_ref()
        .map(|u| u.panel == Panel::Wheel)
        .unwrap_or(false);

    if keys.just_pressed(KeyCode::KeyE) {
        use_aimed(&mut net, &aimed.0, &mut toast, ui.as_deref_mut());
    }
    // The keypad claims its own keys while it is up, and gives every
    // other binding back the moment it closes. Checked before the rest,
    // so typing 1234 at a door cannot also select four hotbar slots.
    if pad.0.is_open() {
        keypad_keys(&keys, &net, &mut pad, &mut toast);
        return;
    }
    if keys.just_pressed(KeyCode::KeyL) {
        access_aimed(&net, &aimed.0, &mut pad, &mut toast, false);
    }
    // `K` is the crew's leave, and it is only a hearth key: at a door the
    // same letter is the keypad's LOCK, which is why `access_aimed` takes
    // the bit rather than this reading the pick twice.
    if keys.just_pressed(KeyCode::KeyK) {
        access_aimed(&net, &aimed.0, &mut pad, &mut toast, true);
    }
    if keys.just_pressed(KeyCode::KeyU) {
        upgrade_near(&net, &near.0, &mut toast);
    }
    if !wheel_up && keys.just_pressed(KeyCode::KeyR) {
        repair_near(&net, &near.0, &mut toast);
    }
    // **The hammer's left click is the repair swing** — the reference's own
    // binding, and free because the hammer has no attack (damage total 0).
    // `R` stays as well: it is what a player without a hammer out still has,
    // and nothing in the reference forbids a keyboard shortcut.
    //
    // Not while a panel owns the pointer, for the reason `place_key` states:
    // a left click on the wheel is a wedge being chosen.
    let hand =
        crate::ui::hold::held_in_hand(&net.session.core.catalog, &net.session.core.inv, net.sel);
    let busy = ui.as_ref().map(|u| u.panel != Panel::None).unwrap_or(false);
    if hand.repairs() && !busy && mouse.just_pressed(MouseButton::Left) {
        repair_near(&net, &near.0, &mut toast);
    }
    if keys.just_pressed(KeyCode::KeyX) {
        throw_near(&net, &near.0, &mut toast);
    }
    if keys.just_pressed(KeyCode::KeyC) {
        light_aimed(&net, &aimed.0, &mut toast);
    }
    // `Backspace` — take the nearest structure back down. A destructive
    // verb gets a key nothing else is near, and one a hand resting on WASD
    // cannot reach by accident: the sim gates it on a ten-minute window
    // and a claim, but a misfire inside both is still a foundation gone.
    if keys.just_pressed(KeyCode::Backspace) {
        demolish_near(&net, &near.0, &mut toast);
    }
    if keys.just_pressed(KeyCode::KeyJ) {
        // Eat what is in the selected hotbar slot. A key rather than a
        // right-click because the swing arm is already spoken for, and a
        // consume that shared it would fire every time you chopped a tree
        // holding berries. Whether the slot holds food is the sim's verdict,
        // announced back either way (`survival.rs`).
        //
        // **`J`, and it was `G` until 2026-08-16**, when `G` became the map
        // (`DECISIONS.md`, the control scheme). What the old binding was
        // chosen for survives the move intact: the reason was never the
        // letter `G`, it was that eat and drink sit under one hand as a pair,
        // and `J`–`H` are adjacent exactly as `G`–`H` were.
        let slot = net.sel;
        send(&net, &mut toast, "eat", |buf| {
            protocol::encode_action_consume(slot, buf)
        });
    }
    if keys.just_pressed(KeyCode::KeyH) {
        // Drink from the water at your feet. `H` because `J` is the eat and
        // the two are one gesture from the player's side — adjacent keys, one
        // hand. Payload-free: the sim reads the heightfield under the body,
        // so there is nothing to aim and no reach for the client to guess.
        send(&net, &mut toast, "drink", protocol::encode_action_drink);
    }
}

/// Dispatch `E` on the resolved pick.
fn use_aimed(net: &mut Net, pick: &Pick, toast: &mut Toast, ui: Option<&mut Ui>) {
    match pick.verb {
        Verb::Door => {
            let (cx, cz, level, loc) = (pick.cx, pick.cz, pick.level, pick.loc);
            if send(net, toast, "use", |buf| {
                protocol::encode_action_use(cx, cz, level, loc, buf)
            }) {
                // Your own door plays on input, remote doors on the event
                // (`NETCODE.md` §6.1). `predict_door` toggles the mirror the
                // structures renderer reconciles against, so the leaf swings
                // this frame and the core owns rolling it back if the sim
                // refuses — the client never has a second copy to diverge.
                net.session.core.predict_door(cx, cz, level, loc);
            }
        }
        Verb::Box => {
            let handle = pick.handle;
            if send(net, toast, "open", |buf| {
                protocol::encode_action_container(CONT_BOX, handle, buf)
            }) {
                open_panel(ui);
            }
        }
        Verb::Bag => {
            // Opening beats emptying: the panel lets you leave the stone and
            // take the gunpowder. The payload-free take-all is what happens
            // when the open will not encode — it carries no target, the sim
            // picks inside the same reach, so it is only ever reached for a
            // bag the resolver already found.
            let handle = pick.handle;
            let opened = send(net, toast, "open", |buf| {
                protocol::encode_action_container(CONT_BAG, handle, buf)
            });
            if opened {
                open_panel(ui);
            } else {
                send(net, toast, "loot", protocol::encode_action_loot);
            }
        }
        Verb::Fire => {
            // `E` opens it, because the panel is where the wood goes and a
            // fire with nothing in it will not light. The match is `C`
            // below — two presses where the reference has one hold-`E`
            // menu with two entries, which is the same two verbs without
            // a radial menu this client does not have.
            let handle = pick.handle;
            if send(net, toast, "open", |buf| {
                protocol::encode_action_container(CONT_BOX, handle, buf)
            }) {
                open_panel(ui);
            }
        }
        Verb::Recycler => {
            // `E` opens it for the fire's reason exactly — the panel is
            // where the salvage goes — and by the same action, because a
            // recycler's contents ARE a box's (`deploy::holds_items`).
            // The switch is `C` below.
            let handle = pick.handle;
            if send(net, toast, "open", |buf| {
                protocol::encode_action_container(CONT_BOX, handle, buf)
            }) {
                open_panel(ui);
            }
        }
        Verb::Research => {
            // The one `E` that spends what is in your hand rather than
            // opening what is at the address. The slot is the hotbar
            // selection — the same `net.sel` the eat verb uses — because
            // "the held item" is the only thing the prompt can honestly
            // name, and the sim refuses a slot that holds the wrong thing
            // with a sentence of its own.
            let slot = net.sel;
            send(net, toast, "research", |buf| {
                protocol::encode_action_research(slot, buf)
            });
        }
        Verb::Hearth => {
            let (cx, cz, level) = (pick.cx, pick.cz, pick.level);
            send(net, toast, "feed", |buf| {
                protocol::encode_action_feed(cx, cz, level, buf)
            });
        }
        // A world container opens exactly the way a box does — the same
        // `ACT_CONTAINER`, the same panel — and the only difference is
        // which kind travels in two bits. That is the whole reason it was
        // built as a container kind rather than as a verb of its own:
        // there is no second screen and no second move path to keep in
        // step with this one.
        //
        // Unlike a box, the open also reaches the sim, because a crate's
        // contents do not exist until this arrives (`server/core.rs` on
        // the `ActionMsg::Container` split). Nothing is predicted: the
        // roll is the server's and the client has no seed to guess it
        // with, so the panel opens on the `ContSync` that follows rather
        // than on the keypress.
        Verb::Crate => {
            let handle = pick.handle;
            if send(net, toast, "open crate", |buf| {
                protocol::encode_action_container(CONT_WORLD, handle, buf)
            }) {
                open_panel(ui);
            }
        }
        // The bench's `E` opens the tree and sends nothing (tech tree
        // v0): the panel is drawn from tables already dripped, and the
        // wire is only touched when a node is actually bought
        // (`panels::tech::clicks`). No `open_panel` — that helper opens
        // the INVENTORY, and this is the one verb that opens something
        // else.
        Verb::TechTree => {
            if let Some(ui) = ui {
                if ui.panel == Panel::None {
                    ui.panel = Panel::Tech;
                    // The header's LEVEL badge is the bench actually under
                    // the crosshair — display only; the sim re-derives the
                    // demanded rung per node.
                    ui.tech_tier = sim_core::deploy::bench_tier(pick.arch).max(1);
                    ui.dirty = true;
                }
            }
        }
        // The honest answer, and the one a chain of `if`s could not give: the
        // browser's old dispatch reported "no hearth in reach" on an empty
        // island because the hearth happened to be the last link tried.
        Verb::None => toast.warn("nothing in reach"),
    }
}

/// `C` — light the aimed fire, or put it out.
///
/// One action for both directions, and no state on the wire: `ACT_USE`
/// carries an address and the sim toggles what stands there (`oven.rs`).
/// That is the opposite of `L`'s absolute lock bit, and deliberately —
/// a lock races with itself when two hands press it, while a fire is a
/// thing you are standing at. What the client does NOT do is predict it:
/// whether there is fuel inside is the sim's verdict, and a flame drawn
/// on the press and taken back a tick later is worse than a flame that
/// arrives a tick late.
fn light_aimed(net: &Net, pick: &Pick, toast: &mut Toast) {
    // A recycler takes the same key and the same action: `ACT_USE` carries
    // an address, and `oven::toggle` switches whatever converter stands
    // there. The refusal that separates them is the sim's — a fire with
    // nothing to burn answers `REFUSE_D_FUEL` and a recycler never does.
    if !matches!(pick.verb, Verb::Fire | Verb::Recycler) {
        toast.warn("nothing to switch in reach");
        return;
    }
    let (cx, cz, level, loc) = (pick.cx, pick.cz, pick.level, pick.loc);
    send(net, toast, "light", |buf| {
        protocol::encode_action_use(cx, cz, level, loc, buf)
    });
}

/// `L` and `K` — the access verb, on whatever the crosshair is on.
///
/// **One key, two stores, because the sim's verb is one verb.** At
/// anything a lock bolts to — a door or a box, and the set is the sim's
/// own `deploy::lockable` by way of `ui::keypad::lock_target`, never a
/// list here — it opens the keypad (which then speaks: six ops share one
/// action code and which one a press means depends on four digits nobody
/// has typed yet). At a hearth it sends a crew op immediately: there is
/// nothing to type, and a pad that asked for four digits at a cupboard
/// would be asking the wrong question.
///
/// `leave` is the `K` half. Passing it in rather than reading the pick
/// twice is what keeps `K` meaning LOCK at a door and LEAVE at a hearth
/// without two resolvers that could disagree about which is aimed at.
///
/// A lockable with no lock bolted on says so rather than opening an empty
/// pad: the wire carries `has_lock` precisely so this prompt can be honest
/// without the client learning anything about who the lock remembers.
fn access_aimed(net: &Net, pick: &Pick, pad: &mut Pad, toast: &mut Toast, leave: bool) {
    use crate::ui::keypad::{lock_target, LockTarget};
    if pick.verb == Verb::Hearth {
        let (cx, cz, level) = (pick.cx, pick.cz, pick.level);
        let op = if leave {
            sim_core::deploy::ACCESS_OP_CREW_LEAVE
        } else {
            sim_core::deploy::ACCESS_OP_CREW_JOIN
        };
        send(net, toast, "crew", |buf| {
            protocol::encode_action_access(
                cx,
                cz,
                level,
                sim_core::build::LOC_PLANE,
                op,
                sim_core::lock::CODE_NONE,
                buf,
            )
        });
        return;
    }
    // `K` at a door is the keypad's own LOCK and is handled there; outside
    // the pad and away from a hearth it means nothing.
    if leave {
        return;
    }
    match lock_target(pick) {
        LockTarget::Pad(cx, cz, level, loc) => pad.0.open(cx, cz, level, loc),
        LockTarget::Bare => toast.warn(format!(
            "no lock on that {} — deploy one",
            // "DOOR"/"BOX", said the toast's way. The label is never
            // empty here: a bare `lock_target` means the pick resolved a
            // real lockable archetype, and every verb of one has a noun.
            pick.verb.label().to_lowercase()
        )),
        LockTarget::None => toast.warn("nothing to authorize in reach"),
    }
}

/// The keypad's own keys: digits, backspace, escape, and the six ops.
///
/// Every verdict here is the sim's — this decides only *what to ask*. The
/// three ops that need a code refuse to send without four digits, because
/// a short code is a different code and a wrong one costs hp.
fn keypad_keys(keys: &ButtonInput<KeyCode>, net: &Net, pad: &mut Pad, toast: &mut Toast) {
    use crate::ui::keypad::{op_for, KeypadKey, Needs};

    if keys.just_pressed(KeyCode::Escape) {
        pad.0.close();
        return;
    }
    if keys.just_pressed(KeyCode::Backspace) {
        pad.0.backspace();
    }
    const DIGITS: [(KeyCode, u8); 20] = [
        (KeyCode::Digit0, 0),
        (KeyCode::Digit1, 1),
        (KeyCode::Digit2, 2),
        (KeyCode::Digit3, 3),
        (KeyCode::Digit4, 4),
        (KeyCode::Digit5, 5),
        (KeyCode::Digit6, 6),
        (KeyCode::Digit7, 7),
        (KeyCode::Digit8, 8),
        (KeyCode::Digit9, 9),
        (KeyCode::Numpad0, 0),
        (KeyCode::Numpad1, 1),
        (KeyCode::Numpad2, 2),
        (KeyCode::Numpad3, 3),
        (KeyCode::Numpad4, 4),
        (KeyCode::Numpad5, 5),
        (KeyCode::Numpad6, 6),
        (KeyCode::Numpad7, 7),
        (KeyCode::Numpad8, 8),
        (KeyCode::Numpad9, 9),
    ];
    for (key, d) in DIGITS {
        if keys.just_pressed(key) {
            pad.0.push(d);
        }
    }

    const OPS: [(KeyCode, KeypadKey); 6] = [
        (KeyCode::Enter, KeypadKey::Try),
        (KeyCode::NumpadEnter, KeypadKey::Try),
        (KeyCode::KeyS, KeypadKey::Set),
        (KeyCode::KeyG, KeypadKey::Guest),
        (KeyCode::KeyK, KeypadKey::Lock),
        (KeyCode::KeyU, KeypadKey::Unlock),
    ];
    let mut want = None;
    for (key, k) in OPS {
        if keys.just_pressed(key) {
            want = Some(k);
        }
    }
    if keys.just_pressed(KeyCode::KeyT) {
        want = Some(KeypadKey::Take);
    }
    let Some(k) = want else { return };
    let Some((op, needs)) = op_for(k) else { return };
    let code = match needs {
        Needs::Code => match pad.0.code() {
            Some(c) => c,
            None => {
                toast.warn("four digits");
                return;
            }
        },
        Needs::Nothing => sim_core::lock::CODE_NONE,
    };
    let Some((cx, cz, level, loc)) = pad.0.at else {
        return;
    };
    send(net, toast, "lock", |buf| {
        protocol::encode_action_access(cx, cz, level, loc, op, code, buf)
    });
    // One press, one op, pad gone. The alternative — leaving it up so the
    // next key is another op — is a pad that eats `W` while a raider is
    // walking through the door you just unlocked.
    pad.0.close();
}

/// The hammer wheel's release (`NOW.md` §0p2 item 1): fire the picked
/// segment at the nearest structure, through the same encoders `U`, `R`
/// and `Backspace` press — zero new wire.
///
/// Every decision — the store filters, the next rung, the refusal
/// sentences — is [`crate::ui::hammer::act`], pure and gated in
/// `tests/ui.rs` §K. This translates its answer into a send, exactly as
/// the keypad's `op_for` split keeps `keypad_keys` decisionless.
pub fn hammer_fire(net: &Net, near: &Option<Target>, seg: usize, toast: &mut Toast) {
    use crate::ui::hammer::{self, Act};
    let Some(&verb) = hammer::VERBS.get(seg) else {
        return;
    };
    let core = &net.session.core;
    match hammer::act(verb, near.as_ref(), &core.piece_defs, core.piece_defs_have) {
        Act::Demolish {
            deploy,
            cx,
            cz,
            level,
            loc,
        } => {
            send(net, toast, "demolish", |buf| {
                protocol::encode_action_demolish(deploy, cx, cz, level, loc, buf)
            });
        }
        Act::Upgrade {
            cx,
            cz,
            level,
            loc,
            material,
        } => {
            send(net, toast, "upgrade", |buf| {
                protocol::encode_action_upgrade(cx, cz, level, loc, material, buf)
            });
        }
        Act::Repair {
            deploy,
            cx,
            cz,
            level,
            loc,
        } => {
            send(net, toast, "repair", |buf| {
                protocol::encode_action_repair(deploy, cx, cz, level, loc, buf)
            });
        }
        Act::Say(s) => toast.warn(s),
    }
}

/// `Backspace` — take the nearest structure back down (demolish v1).
///
/// Reads the same [`Near`] target `R` repairs and `U` upgrades, and
/// carries the same store bit, because the three verbs address the same
/// pair of stores and a fourth resolver would be a fourth chance to pick
/// the doorway when the player meant the door.
///
/// Whether the grace window is still open is **not** asked here. It is
/// arithmetic over a tick the client does not hold, and a prompt that
/// guessed would tell a player their base is disposable ten minutes after
/// it stopped being.
fn demolish_near(net: &Net, near: &Option<Target>, toast: &mut Toast) {
    let Some(t) = near else {
        toast.warn("nothing to take down in reach");
        return;
    };
    let (deploy, cx, cz, level, loc) = (t.store == Store::Deploy, t.cx, t.cz, t.level, t.loc);
    send(net, toast, "demolish", |buf| {
        protocol::encode_action_demolish(deploy, cx, cz, level, loc, buf)
    });
}

/// `U` — take the nearest piece one rung up the material ladder.
///
/// Deployables have no ladder, so a nearest that resolved to one is reported
/// rather than sent: `ACT_UPGRADE` addresses the piece store only, and the
/// grid lets a door and its doorway share an address, so "upgrade the thing
/// in front of me" is genuinely ambiguous at exactly one kind of spot.
fn upgrade_near(net: &Net, near: &Option<Target>, toast: &mut Toast) {
    let Some(t) = near else {
        toast.warn("nothing to upgrade in reach");
        return;
    };
    if t.store != Store::Piece {
        toast.warn("that is not a building piece");
        return;
    }
    let core = &net.session.core;
    let Some(material) = structure::next_material(&core.piece_defs, core.piece_defs_have, t.row)
    else {
        // `REFUSE_B_TIER`'s own sentence, said before the round trip.
        toast.warn("nothing to upgrade into");
        return;
    };
    let (cx, cz, level, loc) = (t.cx, t.cz, t.level, t.loc);
    send(net, toast, "upgrade", |buf| {
        protocol::encode_action_upgrade(cx, cz, level, loc, material, buf)
    });
}

/// `R` — mend the nearest structure, either store.
fn repair_near(net: &Net, near: &Option<Target>, toast: &mut Toast) {
    let Some(t) = near else {
        toast.warn("nothing to repair in reach");
        return;
    };
    if !t.damaged() && t.hp_max > 0 {
        // `REFUSE_B_INTACT`'s sentence, said before the round trip. Guarded
        // on a known maximum: an undripped row reports 0 and the server is
        // the one that knows.
        toast.warn("not damaged");
        return;
    }
    let (deploy, cx, cz, level, loc) = (t.store.is_deploy(), t.cx, t.cz, t.level, t.loc);
    send(net, toast, "repair", |buf| {
        protocol::encode_action_repair(deploy, cx, cz, level, loc, buf)
    });
}

/// `X` — plant the held throwable on the nearest structure.
///
/// The raid verb, and it was unwired on **both** clients (`NOW.md` §0r item
/// 1: "No key plants one — the ui lane owns this. `client_action_throw` ...
/// is exported; nothing in `web/src` calls it"). `X` because every adjacent
/// letter is taken and the reference genre puts throwables off the main
/// verb row.
///
/// Whether the held item is a throwable at all is the sim's verdict — the
/// client carries no fuse table and inventing one would be a countdown that
/// disagrees with the charge.
fn throw_near(net: &Net, near: &Option<Target>, toast: &mut Toast) {
    let Some(t) = near else {
        toast.warn("nothing to plant one on");
        return;
    };
    let (deploy, cx, cz, level, loc) = (t.store.is_deploy(), t.cx, t.cz, t.level, t.loc);
    send(net, toast, "throw", |buf| {
        protocol::encode_action_throw(deploy, cx, cz, level, loc, buf)
    });
}

/// A container panel is only useful with the inventory up — every drag it
/// exists for crosses between the two — so opening one opens that as well.
///
/// **Nothing is drawn here.** The view arrives as a container sync on the
/// event lane and the panel draws it then: the server owns whether this
/// container is open at all, and a panel that opened itself on the keypress
/// would be predicting visibility rather than contents.
fn open_panel(ui: Option<&mut Ui>) {
    if let Some(ui) = ui {
        if ui.panel == Panel::None {
            ui.panel = Panel::Inventory;
            ui.dirty = true;
        }
    }
}

// **There is no `open_worn`, and its absence is the feature.**
//
// Armor v1 shipped one here: a body has no world verb to be opened by —
// every other container is reached by pointing at a thing, `E` on a box,
// a crate, a bag — so the open was attached to the screen that draws it.
// The cost was that the body then competed for the server's single
// container subscription, and a box always won: you could not reach a
// wear slot while looting, which is the move the feature exists for
// (`NOW.md` §0eq item 4).
//
// The body has its own stream now and is dripped unconditionally, so
// there is nothing to ask for. An old client's press still decodes and
// the server answers it with a resync of that stream rather than a
// refusal (`ClientNetState::open_container`) — the honest answer to
// "send me my body", which it is already doing.

/// Close whatever container the sim has open, if any.
///
/// Called when the inventory panel closes, because the server's idea of an
/// open container outlives the panel that was drawing it — and a container
/// left open is one the sim keeps syncing to a screen nobody is looking at.
pub fn close_container(net: &Net, toast: &mut Toast) {
    if net.session.core.cont_kind == CONT_SELF {
        return;
    }
    send(net, toast, "close", |buf| {
        protocol::encode_action_container(CONT_SELF, 0, buf)
    });
}

/// Encode and queue one action, reporting both failure modes rather than
/// swallowing either.
///
/// An encoder refusal is a CLIENT bug and a full lane is a server that is
/// behind, and they are different sentences. Neither is silent: a bad action
/// frame ends the reader task server-side (`server/src/net.rs`), so refusing
/// to encode keeps the bug local instead of arriving as a disconnect, and a
/// full lane means the move was **not** sent (wall 4's stated overflow policy
/// for the reliable lane is to report, never drop).
fn send(
    net: &Net,
    toast: &mut Toast,
    what: &str,
    encode: impl FnOnce(&mut [u8]) -> Result<usize, protocol::WireError>,
) -> bool {
    let mut buf = [0u8; protocol::MAX_STREAM_MSG_BYTES];
    match encode(&mut buf) {
        Ok(len) => match net.session.send_action(&buf[..len]) {
            Ok(()) => true,
            Err(e) => {
                toast.warn(e.to_string());
                false
            }
        },
        Err(e) => {
            toast.warn(format!("{what} would not encode ({e:?})"));
            false
        }
    }
}
