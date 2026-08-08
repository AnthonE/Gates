//! The in-world keys: what the crosshair is on, and what `E`, `G` and `H` do
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
use sim_core::inventory::{CONT_BAG, CONT_BOX, CONT_SELF};

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
pub fn resolve(
    net: NonSend<Net>,
    look: Res<Look>,
    mut aimed: ResMut<Aimed>,
    mut near: ResMut<Near>,
    mut swung: ResMut<Swung>,
    mut in_weak: ResMut<InWeak>,
    world: Option<Res<crate::render::WorldId>>,
) {
    let core = &net.session.core;
    let [x, y, z] = core.predict.render_position();
    let (fx, fz) = sim_core::yaw_dir(yaw_u16(look.yaw));
    aimed.0 = interact::resolve(
        Aim::new(x, z, fx, fz),
        core.deploys.entries(),
        &core.deploy_defs,
        core.deploy_defs_have,
        core.bags.entries(),
    );
    // The scatter pick needs the island, which does not exist until the
    // welcome names a seed — so this is `Option` and stands down rather than
    // guessing one. `render::world_placed`'s discipline, applied to a verb.
    swung.0 = match world.as_deref() {
        Some(w) => interact::resolve_swing(
            SwingAim { x, y, z, fx, fz },
            interact::Island {
                seed: w.seed,
                table: &w.table,
                haven: &w.haven,
                harvested: &core.harvested,
            },
        ),
        None => SwingPick::default(),
    };
    // The weak sector, but only for the node actually aimed at: the chase is
    // per-node and the server restarts it when the player switches targets,
    // so a mark for the tree behind you says nothing about this one.
    in_weak.0 = match world.as_deref() {
        Some(w) if interact::mark_is_for(core.mark_cell, &swung.0) => {
            let s = sim_core::terrain::scatter(
                w.seed,
                &w.table,
                &w.haven,
                swung.0.cx as i32,
                swung.0.cz as i32,
            );
            interact::in_weak_sector(x, z, s.x, s.z, core.mark8)
        }
        _ => false,
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

/// `E`, `G`, `H`.
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
    if keys.just_pressed(KeyCode::KeyL) {
        lock_aimed(&net, &aimed.0, &mut toast);
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
    if keys.just_pressed(KeyCode::KeyG) {
        // Eat what is in the selected hotbar slot. `G` rather than a
        // right-click because the swing arm is already spoken for, and a
        // consume that shared it would fire every time you chopped a tree
        // holding berries. Whether the slot holds food is the sim's verdict,
        // announced back either way (`survival.rs`).
        let slot = net.sel;
        send(&net, &mut toast, "eat", |buf| {
            protocol::encode_action_consume(slot, buf)
        });
    }
    if keys.just_pressed(KeyCode::KeyH) {
        // Drink from the water at your feet. `H` because `G` is the eat and
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
        Verb::Hearth => {
            let (cx, cz, level) = (pick.cx, pick.cz, pick.level);
            send(net, toast, "feed", |buf| {
                protocol::encode_action_feed(cx, cz, level, buf)
            });
        }
        // The honest answer, and the one a chain of `if`s could not give: the
        // browser's old dispatch reported "no hearth in reach" on an empty
        // island because the hearth happened to be the last link tried.
        Verb::None => toast.say("nothing in reach"),
    }
}

/// `L` — lock or unlock the aimed door.
///
/// It reads the pick's own `locked` bit and sends the OPPOSITE, because
/// `ACT_LOCK` sets the state absolutely rather than toggling: a client that
/// sent "locked" twice would leave a player pressing a key that does nothing.
/// The wire carries the lock bit but never the owner, so whether this door is
/// yours is the server's answer and arrives as `REFUSE_D_OWNER`.
fn lock_aimed(net: &Net, pick: &Pick, toast: &mut Toast) {
    if pick.verb != Verb::Door {
        toast.say("no door in reach");
        return;
    }
    let (cx, cz, level, loc, want) = (pick.cx, pick.cz, pick.level, pick.loc, !pick.locked);
    send(net, toast, "lock", |buf| {
        protocol::encode_action_lock(cx, cz, level, loc, want, buf)
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
        toast.say("nothing to upgrade in reach");
        return;
    };
    if t.store != Store::Piece {
        toast.say("that is not a building piece");
        return;
    }
    let core = &net.session.core;
    let Some(material) = structure::next_material(&core.piece_defs, core.piece_defs_have, t.row)
    else {
        // `REFUSE_B_TIER`'s own sentence, said before the round trip.
        toast.say("nothing to upgrade into");
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
        toast.say("nothing to repair in reach");
        return;
    };
    if !t.damaged() && t.hp_max > 0 {
        // `REFUSE_B_INTACT`'s sentence, said before the round trip. Guarded
        // on a known maximum: an undripped row reports 0 and the server is
        // the one that knows.
        toast.say("not damaged");
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
        toast.say("nothing to plant one on");
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
                toast.say(e.to_string());
                false
            }
        },
        Err(e) => {
            toast.say(format!("{what} would not encode ({e:?})"));
            false
        }
    }
}
