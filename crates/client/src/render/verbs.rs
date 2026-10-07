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

use crate::look::{pitch_u8, yaw_u16};
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

/// The hearth's upkeep panel, when one is up: the hearth's address. A
/// resource rather than a `Panel` for [`Pad`]'s reason — it must not hold
/// the player still (`crate::ui::hearth`).
#[derive(Resource, Default)]
pub struct HearthView(pub Option<(u16, u16, u8)>);

/// Rust's one second between bites, kept on this side: when this client
/// last sent a mouthful (food, a heal or the sea), in `Time` seconds. The
/// server throws away a bite inside the last one's gap (`server::pace`,
/// `Kind::Mouth`, a little under a second for network jitter), so an honest
/// press that waits the full second here never meets that refusal — and a
/// hammered eat key eats once a second instead of queuing a stack.
#[derive(Resource, Default)]
pub struct Bite(Option<f64>);

/// The wait between two bites, seconds — Rust's consume cooldown.
pub const BITE_S: f64 = 1.0;

impl Bite {
    /// May a mouthful go at `now`? Starts the clock when it does.
    pub fn take(&mut self, now: f64) -> bool {
        if self.0.is_some_and(|last| now - last < BITE_S) {
            return false;
        }
        self.0 = Some(now);
        true
    }
}

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
    // The wire's own two bytes for the look, so the swing prompt is cast
    // from exactly what the frame will carry (`SwingAim`'s header).
    let (yaw, pitch) = (yaw_u16(look.yaw), pitch_u8(look.pitch));
    // The stance the sim will swing from, so the prompt's ray leaves the
    // crouched eye when the swing will (v83).
    let crouched = core.crouched();
    let (fx, fz) = sim_core::yaw_dir(yaw);
    // The eye's ray, for the deployables `E` is offered on only when the
    // crosshair is on them (free placement): the eye the sim would cast
    // from and the look it would carry, against the box the renderer draws.
    let span =
        |rec: &sim_core::deploy::DeployRec, arch: u8| body_span(world.as_deref(), core, rec, arch);
    let (ch, sv) = sim_core::pitch_dir(pitch);
    let eye = [
        x,
        y + if crouched {
            super::CROUCH_EYE_M
        } else {
            super::EYE_HEIGHT
        },
        z,
    ];
    let dir = [fx * ch, sv, fz * ch];
    let mut aim = Aim::new(x, z, fx, fz);
    if world.is_some() {
        aim.sight = Some(interact::Sight {
            eye,
            dir,
            span: &span,
        });
    }
    aimed.0 = interact::resolve(
        aim,
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
        interact::Verb::Fire | interact::Verb::Recycler | interact::Verb::Research
    ) && core
        .ovens()
        .is_lit(aimed.0.cx, aimed.0.cz, aimed.0.level, aimed.0.loc);
    aimed.0.public = matches!(
        aimed.0.verb,
        interact::Verb::Recycler | interact::Verb::Research | interact::Verb::TechTree
    ) && interact::town_station(&core.haven().town, aimed.0.cx, aimed.0.cz);
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
                doors: occ.doors,
                seed,
                table: occ.table,
                haven: occ.haven,
                harvested: occ.harvested,
                cache: occ.cache,
            };
            let pick = interact::resolve_swing(
                SwingAim {
                    x,
                    y,
                    z,
                    yaw,
                    pitch,
                    crouched,
                },
                &mut island,
            );
            // The open pick, on the same island borrow and the same aim.
            // Folded into `aimed` rather than kept beside it, because to
            // the player there is one `E` and one centre prompt — a
            // second pick with its own key would teach two verbs for one
            // gesture. It loses every tie to a deployable on purpose: a
            // box placed against a waystation cache is a thing a player
            // built and meant, and the authored container is not going
            // anywhere.
            if aimed.0.is_none() {
                let open = interact::resolve_open(
                    SwingAim {
                        x,
                        y,
                        z,
                        yaw,
                        pitch,
                        crouched,
                    },
                    &mut island,
                );
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
            // A berry bush or hemp under the crosshair is picked with `E`
            // (it is never swung at), folded in exactly as the crate is: it
            // loses to anything a player built or an authored container.
            if aimed.0.is_none() {
                let bush = interact::resolve_pick(
                    SwingAim {
                        x,
                        y,
                        z,
                        yaw,
                        pitch,
                        crouched,
                    },
                    &mut island,
                );
                if bush.occupant != 0 {
                    aimed.0 = interact::Pick {
                        verb: interact::Verb::Pick,
                        occupant: bush.occupant,
                        cx: bush.cx,
                        cz: bush.cz,
                        handle: sim_core::gather::cell_key(bush.cx, bush.cz),
                        d2: bush.d2,
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
    if !core.wounded && !core.dead {
        let help = interact::resolve_assist(
            SwingAim {
                x,
                y,
                z,
                yaw,
                pitch,
                crouched,
            },
            core.player_id,
            &core.view.entities,
        );
        if !help.is_none() && (aimed.0.is_none() || help.d2 < aimed.0.d2) {
            aimed.0 = help;
        }
    }
    // A loose stack (ground items v0), last of the three `E` picks.
    //
    // **Outside the island block on purpose**: `core.island()` holds the
    // core mutably (it owns the scatter cache), and this pick needs no
    // island at all — a loose stack is a set the server states, not
    // something derived from the seed. So it resolves here, after that
    // borrow ends, which is also the right place in the chain.
    //
    // **Not aim-weighted**, so it can only ever be a fallback: a player
    // looking at a box with a sack at their feet means the box, and
    // `resolve_take` has no aim to lose that tie with. Third, so a
    // deployable wins and an authored container wins — both are things
    // you walked to on purpose, and a sack is the thing you are standing
    // in.
    if aimed.0.is_none() {
        let take = take_or_pull(core, x, z);
        if take.verb != interact::Verb::None {
            aimed.0 = take;
        }
    }
    // A town kiosk (THE GATE): last, by nearness — the counter is a place
    // you walk up to, not a thing you aim at.
    if aimed.0.is_none() {
        let trade = interact::resolve_trade(x, z, &core.haven().town);
        if trade.verb != interact::Verb::None {
            aimed.0 = trade;
        }
    }
    // A ziggurat door's reader or lever, the same way.
    if aimed.0.is_none() {
        let swipe = interact::resolve_swipe(x, y, z, &core.haven().ziggurat);
        if swipe.verb != interact::Verb::None {
            aimed.0 = swipe;
        }
    }
    // A work's terminal (`ARC.md` F1), the same way.
    if aimed.0.is_none() {
        let work = interact::resolve_work(x, y, z, core.haven(), &core.arc);
        if work.verb != interact::Verb::None {
            aimed.0 = work;
        }
    }
    // Down, the only prompt worth drawing is a door's (wounded v0): every
    // other `E` would be refused.
    if core.wounded && !crate::ui::wounded::allows(aimed.0.verb) {
        aimed.0 = interact::Pick::default();
    }
    // The hammer's and the charge's target: the free-placed deployable the
    // crosshair is on, by `E`'s own ray, else the structure nearest the feet.
    let span =
        |rec: &sim_core::deploy::DeployRec, arch: u8| body_span(world.as_deref(), core, rec, arch);
    let seen = world.as_deref().and_then(|w| {
        structure::seen_deploy(
            w.seed,
            &w.haven,
            core.pieces.cols(),
            (x, z),
            &interact::Sight {
                eye,
                dir,
                span: &span,
            },
            core.deploys.entries(),
            &core.deploy_defs,
            core.deploy_defs_have,
        )
    });
    near.0 = seen.or_else(|| {
        structure::nearest(
            (x, z),
            core.pieces.entries(),
            &core.piece_defs,
            core.piece_defs_have,
            core.deploys.entries(),
            &core.deploy_defs,
            core.deploy_defs_have,
        )
    });
}

/// `(bottom, top)` of a body deployable as the renderer stands it — its
/// floor and that plus its drawn height — for [`interact::Sight::span`].
pub(crate) fn body_span(
    world: Option<&crate::render::WorldId>,
    core: &client_core::core::ClientCore,
    rec: &sim_core::deploy::DeployRec,
    arch: u8,
) -> (f32, f32) {
    let Some(w) = world else {
        return (f32::NEG_INFINITY, f32::INFINITY);
    };
    let bottom = sim_core::deploy::body_base_y(w.seed, &w.haven, core.pieces.cols(), rec, arch);
    (
        bottom,
        bottom + super::structures::deploy_size(arch as usize).y,
    )
}

/// `E`, `J`, `H`.
///
/// Runs in `Screen::InWorld` only and stands down while a panel owns the
/// pointer, for `input::gather`'s reason: every verb here spends something —
/// a swing, a door, a mouthful — and a player typing into the craft search
/// box asked for none of it.
// Each is a distinct source: the keyboard, the session, the two picks, the
// toast, the panels, the chat composer, and the clock the crew clear's
// second press is timed on.
#[allow(clippy::too_many_arguments)]
pub fn keys(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut net: NonSendMut<Net>,
    aimed: Res<Aimed>,
    near: Res<Near>,
    mut toast: ResMut<Toast>,
    mut pad: ResMut<Pad>,
    mut hearth: ResMut<HearthView>,
    ui: Option<ResMut<Ui>>,
    chat: Option<Res<super::chat::Chat>>,
    time: Res<Time>,
    mut bite: ResMut<Bite>,
    mut clear_armed_until: Local<f64>,
    // The arm, for the hammer's repair swing: a repair is an action, not
    // the swing button, so nothing else moves it.
    mut motion: ResMut<super::viewmodel::Motion>,
) {
    let mut ui = ui;
    let now = time.elapsed_secs_f64();
    if keys.just_released(KeyCode::KeyE) {
        send(&net, &mut toast, "release help", |buf| {
            protocol::encode_action_assist(0, buf)
        });
    }
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

    // **Down, the hands are gone** (wounded v0). The sim refuses every verb
    // but a door's without a word, so the keys answer here instead of
    // sending into nothing: `E` still works a door, the rest say why.
    if net.session.core.wounded {
        pad.0.close();
        hearth.0 = None;
        const HAND_KEYS: [KeyCode; 11] = [
            KeyCode::KeyE,
            KeyCode::KeyL,
            KeyCode::KeyK,
            KeyCode::KeyU,
            KeyCode::KeyR,
            KeyCode::KeyX,
            KeyCode::KeyC,
            KeyCode::KeyJ,
            KeyCode::KeyV,
            KeyCode::KeyH,
            KeyCode::Backspace,
        ];
        if keys.just_pressed(KeyCode::KeyE) && crate::ui::wounded::allows(aimed.0.verb) {
            use_aimed(&mut net, &aimed.0, &mut toast, ui.as_deref_mut());
        } else if HAND_KEYS.iter().any(|&k| keys.just_pressed(k))
            // The left click too: it places, eats, reads and repairs, and
            // the sim drops every one of those from a downed body unsaid.
            || mouse.just_pressed(MouseButton::Left)
        {
            toast.warn(crate::ui::wounded::HANDS_LINE);
        }
        return;
    }

    if keys.just_pressed(KeyCode::KeyE) {
        // A second `E` closes the hearth's panel rather than feeding again.
        if hearth.0.is_some() {
            hearth.0 = None;
        } else {
            use_aimed(&mut net, &aimed.0, &mut toast, ui.as_deref_mut());
            if aimed.0.verb == Verb::Hearth {
                hearth.0 = Some((aimed.0.cx, aimed.0.cz, aimed.0.level));
            }
        }
    }
    // The keypad claims its own keys while it is up, and gives every
    // other binding back the moment it closes. Checked before the rest,
    // so typing 1234 at a door cannot also select four hotbar slots.
    if pad.0.is_open() {
        keypad_keys(&keys, &net, &mut pad, &mut toast);
        return;
    }
    // **A food slot's key eats one** and leaves the hand as it was — the
    // other half of `input::gather`'s hotbar loop, off the same test
    // (`hold::eats_on_key`). Below the keypad's claim, so a code typed into
    // a door never eats the mushrooms in slot 3.
    if !net.session.core.wounded {
        for (i, k) in super::input::HOTBAR_KEYS.iter().enumerate() {
            let core = &net.session.core;
            let food = core
                .inv
                .get(i)
                .is_some_and(|&s| crate::ui::hold::eats_on_key(&core.catalog, s));
            if keys.just_pressed(*k) && food {
                use_slot(&net, &mut toast, &mut bite, now, i as u8);
            }
        }
    }
    if keys.just_pressed(KeyCode::KeyL) {
        access_aimed(&net, &aimed.0, &mut pad, &mut toast, Access::Join);
    }
    // `K` is the crew's leave and `Shift+K` its clear, and both are only
    // hearth keys: at a door the same letter is the keypad's LOCK, which is
    // why `access_aimed` takes the ask rather than this reading the pick
    // twice.
    if keys.just_pressed(KeyCode::KeyK) {
        let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
        let now = time.elapsed_secs_f64();
        let ask = if !shift {
            Some(Access::Leave)
        } else if now <= *clear_armed_until {
            *clear_armed_until = 0.0;
            Some(Access::Clear)
        } else {
            // A clear takes everyone else off the crew, so it asks twice:
            // the first press arms it, a second within the window sends.
            if aimed.0.verb == Verb::Hearth {
                *clear_armed_until = now + CLEAR_CONFIRM_S;
                toast.warn("SHIFT+K again to clear the crew to just you");
            }
            None
        };
        if let Some(ask) = ask {
            access_aimed(&net, &aimed.0, &mut pad, &mut toast, ask);
        }
    }
    if keys.just_pressed(KeyCode::KeyU) {
        upgrade_near(&net, &near.0, &mut toast);
    }
    // **`R` is modal on the hand since reload v1: hammer repairs, anything
    // else reloads.** `R` is the genre's reload key and the reference's, so
    // taking it is the standing instruction rather than a preference — and
    // the hand is what the rest of this file already switches on
    // (`hand.repairs()` five lines down, `hand.opens_a_wheel()` at the
    // wheel). What the move costs is repair-with-no-hammer-out, which the
    // comment below used to call the point of the binding; what it buys is
    // that a gun in your hand answers the key every player already reaches
    // for. Proposed default, `DECISIONS.md` §open (reload v1).
    let hand =
        crate::ui::hold::held_in_hand(&net.session.core.catalog, &net.session.core.inv, net.sel);
    // ...and the building plan takes `R` (and `F`) for the foundation
    // height nudge (`ghost::height_keys`, foundation height v0) — the same
    // modal rule, one hand further along: a plan has nothing to reload. A
    // deployable in hand takes it too, to turn its ghost (`ghost::turn_key`,
    // the reference's rotate): a box has nothing to reload either.
    let deploying = {
        let core = &net.session.core;
        let held = core.inv[(net.sel as usize).min(core.inv.len().saturating_sub(1))];
        crate::ui::hold::click_of(
            &core.catalog,
            &core.research,
            &core.deploy_defs,
            core.deploy_defs_have,
            held,
        ) == crate::ui::hold::Click::Deploy
    };
    if !wheel_up && !hand.places() && !deploying && keys.just_pressed(KeyCode::KeyR) {
        if hand.repairs() {
            if repair_near(&net, &near.0, &mut toast) {
                motion.strike();
            }
        } else {
            // Payload-free, `V`'s shape: the sim reads the hand it already
            // has, so there is nothing to aim and no amount for the client
            // to guess. `just_pressed` for `V`'s reason too — the action
            // lane takes one pending action per client per tick, so a held
            // key would send a frame's worth and queue them all behind
            // each other (`server::pace` holds, it never drops).
            //
            // Sent blind, and the refusal is what makes that work: a press
            // with a rock in hand, a full cylinder or an empty pack each
            // come back as their own `EV_RELOAD_REFUSED` sentence, so the
            // key never does nothing silently. Deciding it here off
            // `mag()` instead would be worse, not better — the readout is
            // whatever the last event stated, so a revolver just picked up
            // reads `(0, 0)` and the client would swallow the one press
            // that matters.
            send(&net, &mut toast, "reload", protocol::encode_action_reload);
        }
    }
    // **The hammer's left click is the repair swing** — the reference's own
    // binding, and free because the hammer has no attack (damage total 0).
    // `R` stays as well: it is what a player without a hammer out still has,
    // and nothing in the reference forbids a keyboard shortcut.
    //
    // Not while a panel owns the pointer, for the reason `place_key` states:
    // a left click on the wheel is a wedge being chosen.
    let busy = ui.as_ref().map(|u| u.panel != Panel::None).unwrap_or(false);
    if hand.repairs()
        && !busy
        && mouse.just_pressed(MouseButton::Left)
        && repair_near(&net, &near.0, &mut toast)
    {
        motion.strike();
    }
    // **Left click eats what is in your hand** — Rust's belt: select the
    // mushrooms, click, eat one. Until this the only way was `J`, which no
    // prompt named, and the click swung the mushrooms at a tree instead
    // (`input::gather` no longer sends that swing — `Click::swings`). A
    // blueprint in the hand is read the same way. One per press: the action
    // lane holds one action per client per tick, and a held button eating
    // the whole stack would be the waste a meal ought not to be.
    //
    // Not while down: the sim ignores a downed body's hand
    // (`World::live_slot_of`), so a sent eat would be a click that did
    // nothing and said nothing.
    if !busy && !net.session.core.wounded && mouse.just_pressed(MouseButton::Left) {
        let core = &net.session.core;
        let click = crate::ui::hold::click_in_hand(
            &core.catalog,
            &core.research,
            &core.deploy_defs,
            core.deploy_defs_have,
            &core.inv,
            net.sel,
        );
        if matches!(
            click,
            crate::ui::hold::Click::Eat | crate::ui::hold::Click::Read
        ) {
            use_slot(&net, &mut toast, &mut bite, now, net.sel);
        }
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
        // Eat what is in the selected hotbar slot — the keyboard twin of the
        // left click above, kept because it works with anything in the slot
        // and says why when that is not food. Whether the slot holds food is
        // the sim's verdict, announced back either way (`survival.rs`).
        //
        // **`J`, and it was `G` until 2026-08-16**, when `G` became the map
        // (`DECISIONS.md`, the control scheme). What the old binding was
        // chosen for survives the move intact: the reason was never the
        // letter `G`, it was that eat and drink sit under one hand as a pair,
        // and `J`–`H` are adjacent exactly as `G`–`H` were.
        //
        // A blueprint in the hand is READ rather than eaten (research table
        // v1): `ui::research::use_as` makes the call the inventory panel's
        // right-click makes, so the key and the click cannot disagree.
        use_slot(&net, &mut toast, &mut bite, now, net.sel);
    }
    if keys.just_pressed(KeyCode::KeyV) {
        // Take the nearest loose stack in reach — the take `E`'s prompt
        // names, landed arrows included (`sim-core/spent.rs`), on a key the
        // hand reaches without leaving `WASD` while it walks the field it
        // just shot across. Payload-free: the sim picks, from the body it
        // already has. `just_pressed` because the action lane takes one
        // pending action per client per tick. Nothing in the sim's own
        // reach (`resolve_take`) is said here: the sim answers that press
        // with no event at all.
        let core = &net.session.core;
        let [x, _, z] = core.predict.render_position();
        if take_or_pull(core, x, z).verb == Verb::None {
            toast.warn("nothing to pick up here");
        } else {
            send(&net, &mut toast, "pick up", protocol::encode_action_pickup);
        }
    }
    if keys.just_pressed(KeyCode::KeyH) {
        // Drink from the water at your feet. `H` because `J` is the eat and
        // the two are one gesture from the player's side — adjacent keys, one
        // hand. Payload-free: the sim reads the heightfield under the body,
        // so there is nothing to aim and no reach for the client to guess.
        // A gulp is a mouthful: it waits on the bite clock with the food.
        if bite.take(now) {
            send(&net, &mut toast, "drink", protocol::encode_action_drink);
        }
    }
}

/// Use inventory `slot`: read it if it is a blueprint, eat it otherwise —
/// `J`'s verb and the left click's, one call so the two cannot disagree.
/// Whether it does anything is the sim's verdict, announced either way.
///
/// A mouthful waits on [`Bite`]: inside the last one's second it is not
/// sent at all, as the reference drops it.
fn use_slot(net: &Net, toast: &mut Toast, bite: &mut Bite, now: f64, slot: u8) {
    let stack = net
        .session
        .core
        .inv
        .get(slot as usize)
        .copied()
        .unwrap_or_default();
    match crate::ui::research::use_as(&net.session.core.research, stack) {
        crate::ui::research::UseAs::Read => {
            send(net, toast, "read", |buf| {
                protocol::encode_action_research(slot, buf)
            });
        }
        crate::ui::research::UseAs::Consume => {
            if bite.take(now) {
                send(net, toast, "eat", |buf| {
                    protocol::encode_action_consume(slot, buf)
                });
            }
        }
    }
}

/// Dispatch `E` on the resolved pick.
fn use_aimed(net: &mut Net, pick: &Pick, toast: &mut Toast, ui: Option<&mut Ui>) {
    match pick.verb {
        Verb::Swipe => {
            let door = pick.handle as u8;
            send(net, toast, "swipe", |buf| {
                protocol::encode_action_swipe(door, buf)
            });
        }
        Verb::Assist => {
            send(net, toast, "help up", |buf| {
                protocol::encode_action_assist(pick.handle, buf)
            });
        }
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
            // A container since research table v1, so `E` opens it for the
            // recycler's reason exactly — the panel is where the sample and
            // the junk go — and by the same action. `C` below starts it.
            // (Until v1 this `E` researched the held item on the spot; the
            // timed table replaced that, and paper is read from the pack.)
            let handle = pick.handle;
            if send(net, toast, "open", |buf| {
                protocol::encode_action_container(CONT_BOX, handle, buf)
            }) {
                open_panel(ui);
            }
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
        // A loose stack (ground items v0). **The same payload-free
        // `encode_action_pickup` the `V` key sends** — one opcode for
        // "pick up the thing at my feet", which is what keeps the CHOICE
        // in the sim: the prompt named the nearest stack because
        // `resolve_take` uses the sim's own rule, so the key and the line
        // above it cannot disagree.
        //
        // No panel: there is nothing to open. What the player gets is the
        // `EV_GATHER` toast every other payout arrives as, and the sack
        // stops being drawn on the next sync — which is server truth
        // rather than this side guessing that the take worked.
        Verb::Take => {
            send(net, toast, "take", protocol::encode_action_pickup);
        }
        // A bush: the cell key is the claim, and the sim re-derives the bush
        // and the reach (`gather::pick`). Nothing is predicted — the bush
        // goes on `EV_SLOT_HARVESTED` and the berries arrive as `EV_GATHER`.
        Verb::Pick => {
            let cell = pick.handle;
            send(net, toast, "pick", |buf| {
                protocol::encode_action_pick(cell, buf)
            });
        }
        // The bench's `E` opens the tree and sends nothing (tech tree
        // v0): the panel is drawn from tables already dripped, and the
        // wire is only touched when a node is actually bought
        // (`panels::tech::clicks`). No `open_panel` — that helper opens
        // the INVENTORY, and this is the one verb that opens something
        // else.
        // A kiosk opens its stall's offers and sends nothing; the buy
        // buttons send (`panels::vendor::clicks`).
        Verb::Trade => {
            if let Some(ui) = ui {
                if ui.panel == Panel::None {
                    ui.panel = Panel::Vendor;
                    ui.vendor = pick.handle as u8;
                    ui.status.clear();
                    ui.dirty = true;
                }
            }
        }
        // A work's terminal opens its panel and sends nothing; DEPOSIT and
        // FUEL send (`panels::arc::clicks`).
        Verb::Work => {
            if let Some(ui) = ui {
                if ui.panel == Panel::None {
                    ui.panel = Panel::Work;
                    ui.work = pick.handle as u8;
                    ui.status.clear();
                    ui.dirty = true;
                }
            }
        }
        Verb::TechTree => {
            if let Some(ui) = ui {
                if ui.panel == Panel::None {
                    ui.panel = Panel::Tech;
                    // The bench actually under the crosshair: the highest
                    // tab, and the rung the panel's reach check holds it to.
                    // The sim still re-derives the demanded rung per node.
                    ui.tech_tier = sim_core::deploy::bench_tier(pick.arch).max(1);
                    // Opens on the bench's own tree; the lower tiers are
                    // tabs (operator, 2026-09-22).
                    ui.tech_tab = ui.tech_tier;
                    ui.tech_sel = None;
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
    // And a research table (research table v1): its start is the same
    // switch, and `research::begin` answers with the table's own refusal.
    if !matches!(pick.verb, Verb::Fire | Verb::Recycler | Verb::Research) {
        toast.warn("nothing to switch in reach");
        return;
    }
    let (cx, cz, level, loc) = (pick.cx, pick.cz, pick.level, pick.loc);
    send(net, toast, "light", |buf| {
        protocol::encode_action_use(cx, cz, level, loc, buf)
    });
}

/// How long a first `Shift+K` at a hearth stays armed for the second.
const CLEAR_CONFIRM_S: f64 = 3.0;

/// Which access key was pressed: `L`, `K` or `Shift+K`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Access {
    Join,
    Leave,
    Clear,
}

/// `L` and `K` — the access verb, on whatever the crosshair is on.
///
/// **One key, two stores, because the sim's verb is one verb.** At
/// anything a lock bolts to — a door, a box, a hearth with a lock on it;
/// the set is the sim's own `deploy::lockable` by way of
/// `ui::keypad::lock_target`, never a list here — it opens the keypad
/// (which then speaks: six ops share one action code and which one a press
/// means depends on four digits nobody has typed yet). At a bare hearth it sends a crew op immediately: there
/// is nothing to type. A hearth with a lock bolted on opens its pad like a
/// door's, because there the four digits ARE the question — the code is
/// how a crew invites a hand (hearth lock v0).
///
/// `ask` says which key: `L` joins, `K` leaves, `Shift+K` clears the crew
/// to the one pressing it (the reference's "clear list"; the sim refuses a
/// hand not on the crew). Passing it in rather than reading the pick twice
/// is what keeps `K` meaning LOCK at a door and LEAVE at a hearth without
/// two resolvers that could disagree about which is aimed at.
///
/// A lockable with no lock bolted on says so rather than opening an empty
/// pad: the wire carries `has_lock` precisely so this prompt can be honest
/// without the client learning anything about who the lock remembers.
fn access_aimed(net: &Net, pick: &Pick, pad: &mut Pad, toast: &mut Toast, ask: Access) {
    use crate::ui::keypad::{lock_target, LockTarget};
    // A hearth with a lock bolted on takes `L` to its keypad (hearth lock
    // v0): the right code is the invitation, and the sim puts the hand it
    // remembers on the crew in the same act. `K` and `Shift+K` stay the
    // crew's leave and clear on every hearth, locked or bare — neither is
    // ever a question for a pad.
    if pick.verb == Verb::Hearth && (ask != Access::Join || !pick.has_lock) {
        let (cx, cz, level, loc) = (pick.cx, pick.cz, pick.level, pick.loc);
        let op = match ask {
            Access::Join => sim_core::deploy::ACCESS_OP_CREW_JOIN,
            Access::Leave => sim_core::deploy::ACCESS_OP_CREW_LEAVE,
            Access::Clear => sim_core::deploy::ACCESS_OP_CREW_CLEAR,
        };
        send(net, toast, "crew", |buf| {
            protocol::encode_action_access(cx, cz, level, loc, op, sim_core::lock::CODE_NONE, buf)
        });
        return;
    }
    // `K` at a door is the keypad's own LOCK and is handled there; outside
    // the pad and away from a hearth it means nothing, and says so.
    if ask != Access::Join {
        toast.warn(if ask == Access::Leave {
            "no hearth in reach to leave"
        } else {
            "no hearth in reach to clear"
        });
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
        Act::Rotate { cx, cz, level, loc } => {
            send(net, toast, "rotate", |buf| {
                protocol::encode_action_rotate(cx, cz, level, loc, buf)
            });
        }
        // THE GATE's stations are the town's: the sim refuses the pick-up
        // (`deploy::WORLD_OWNER`), and the reason is not a lock.
        Act::Demolish {
            deploy: true,
            cx,
            cz,
            ..
        } if crate::ui::interact::town_station(&core.haven().town, cx, cz) => {
            toast.warn(TOWN_PROPERTY);
        }
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

/// What a pick-up of one of THE GATE's own stations says.
const TOWN_PROPERTY: &str = "that belongs to THE GATE - it can't be picked up";

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
    if deploy && crate::ui::interact::town_station(&net.session.core.haven().town, cx, cz) {
        toast.warn(TOWN_PROPERTY);
        return;
    }
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

/// `R` — mend the nearest structure, either store. Whether a repair went.
fn repair_near(net: &Net, near: &Option<Target>, toast: &mut Toast) -> bool {
    let Some(t) = near else {
        toast.warn("nothing to repair in reach");
        return false;
    };
    if !t.damaged() && t.hp_max > 0 {
        // `REFUSE_B_INTACT`'s sentence, said before the round trip. Guarded
        // on a known maximum: an undripped row reports 0 and the server is
        // the one that knows.
        toast.warn("not damaged");
        return false;
    }
    let (deploy, cx, cz, level, loc) = (t.store.is_deploy(), t.cx, t.cz, t.level, t.loc);
    send(net, toast, "repair", |buf| {
        protocol::encode_action_repair(deploy, cx, cz, level, loc, buf)
    });
    true
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

/// Close the hearth's panel on `Esc`, on walking out of feeding reach, or
/// when the hearth is gone. Ordered before `pause::open` and consuming the
/// `Esc`, so closing the panel does not also open the pause menu.
pub fn hearth_close(
    mut keys: ResMut<ButtonInput<KeyCode>>,
    net: NonSend<Net>,
    mut hearth: ResMut<HearthView>,
) {
    let Some((cx, cz, level)) = hearth.0 else {
        return;
    };
    if keys.just_pressed(KeyCode::Escape) {
        keys.clear_just_pressed(KeyCode::Escape);
        hearth.0 = None;
        return;
    }
    let core = &net.session.core;
    let standing = core.deploys.entries().iter().find(|r| {
        (r.cx, r.cz, r.level) == (cx, cz, level)
            && (r.row as u16) < core.deploy_defs_have
            && core.deploy_defs.defs[r.row as usize].arch == sim_core::deploy::ARCH_HEARTH
    });
    if core.wounded
        || core.dead
        || !standing.is_some_and(|r| crate::ui::hearth::in_reach(core.predict.position(), r.xz()))
    {
        hearth.0 = None;
    }
}

/// The pick `E` takes from where the player stands: a loose stack, or an
/// arrow standing in a body in reach — their own, where they stand, or one
/// they can see, where it is drawn.
fn take_or_pull(core: &client_core::core::ClientCore, x: f32, z: f32) -> interact::Pick {
    let at = core.render_tick();
    let mut rs = client_core::interp::RemoteState::default();
    interact::resolve_take_or_pull(x, z, core.ground_items(), core.lodged(), |id| {
        if id == core.player_id {
            Some((x, z))
        } else {
            core.interp.sample(id, at, &mut rs).then_some((rs.x, rs.z))
        }
    })
}
