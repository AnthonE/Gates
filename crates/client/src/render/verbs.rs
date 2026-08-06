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
use crate::ui::interact::{self, Aim, Pick, Verb};

use super::hud::Toast;
use super::input::Look;
use super::panels::{Panel, Ui};
use super::Net;

/// What the crosshair is on this frame. Written by [`resolve`], read by the
/// prompt and by [`keys`] — one value, so the two cannot disagree.
#[derive(Resource, Default)]
pub struct Aimed(pub Pick);

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
pub fn resolve(net: NonSend<Net>, look: Res<Look>, mut aimed: ResMut<Aimed>) {
    let core = &net.session.core;
    let [x, _, z] = core.predict.render_position();
    let (fx, fz) = sim_core::yaw_dir(yaw_u16(look.yaw));
    aimed.0 = interact::resolve(
        Aim::new(x, z, fx, fz),
        core.deploys.entries(),
        &core.deploy_defs,
        core.deploy_defs_have,
        core.bags.entries(),
    );
}

/// `E`, `G`, `H`.
///
/// Runs in `Screen::InWorld` only and stands down while a panel owns the
/// pointer, for `input::gather`'s reason: every verb here spends something —
/// a swing, a door, a mouthful — and a player typing into the craft search
/// box asked for none of it.
pub fn keys(
    keys: Res<ButtonInput<KeyCode>>,
    mut net: NonSendMut<Net>,
    aimed: Res<Aimed>,
    mut toast: ResMut<Toast>,
    ui: Option<ResMut<Ui>>,
) {
    let mut ui = ui;
    if ui
        .as_ref()
        .map(|u| u.panel.grabs_pointer())
        .unwrap_or(false)
    {
        return;
    }

    if keys.just_pressed(KeyCode::KeyE) {
        use_aimed(&mut net, &aimed.0, &mut toast, ui.as_deref_mut());
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
