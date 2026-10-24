//! The auto turret's own controls, under its box on the inventory page
//! (`sim_core::turret`): who is on its list, and the three list buttons —
//! AUTHORIZE (put yourself on), DEAUTHORIZE (take yourself off) and CLEAR
//! LIST (empty it, owner and all) — and its mode, PEACEKEEPER (only the
//! hostile) or ATTACK ALL. Each sends `ACT_ACCESS` with its
//! `ACCESS_OP_TURRET_*` at the turret's address; the sim's verdict comes
//! back as the turret view (`EventMsg::Turret`) or a deploy refusal.

use bevy::prelude::*;
use client_core::core::ClientCore;
use sim_core::deploy::{
    box_key, ACCESS_OP_TURRET_ATTACK_ALL, ACCESS_OP_TURRET_AUTH, ACCESS_OP_TURRET_CLEAR,
    ACCESS_OP_TURRET_DEAUTH, ACCESS_OP_TURRET_PEACEKEEPER, ARCH_TURRET,
};
use sim_core::gather::ItemStack;
use sim_core::inventory::{is_own, CONT_BOX, CONT_SELF, CONT_WEAR};
use sim_core::limits::INV_SLOTS;
use sim_core::turret::{AMMO_SLOTS, WEAPON_SLOT};

use crate::ui::craft::item_label;
use crate::ui::slots::{move_args, Grab, Quick};

use super::{font, font_bold, Panel, Ui, CELL_BG, CELL_HOVER, TEXT, TEXT_DIM};

/// One list button: the op it sends.
#[derive(Component)]
pub struct TurretOp(pub u8);

/// The open container's box key, if it is an auto turret.
pub fn open_turret(core: &ClientCore) -> Option<u32> {
    if core.cont_kind != CONT_BOX || core.cont_handle == 0 {
        return None;
    }
    core.deploys.entries().iter().find_map(|d| {
        let key = box_key(d.cx, d.cz, d.level, d.loc);
        (key == core.cont_handle
            && u16::from(d.row) < core.deploy_defs_have
            && core.deploy_defs.defs[d.row as usize].arch == ARCH_TURRET)
            .then_some(key)
    })
}

/// What the gun has to fire: its round and how many, or what is missing.
pub fn readout(col: &mut ChildSpawnerCommands, core: &ClientCore, key: u32) {
    let line = match core.turret_view(key) {
        None => "…".to_string(),
        Some(v) if v.gun == 0 => "NO WEAPON: put a gun in WEAPON".to_string(),
        Some(v) if v.rounds == 0 => format!(
            "NO AMMO: {} for the {}",
            item_label(&core.catalog, v.ammo).to_uppercase(),
            item_label(&core.catalog, v.gun).to_uppercase()
        ),
        Some(v) => format!(
            "{} · {} ROUNDS",
            item_label(&core.catalog, v.ammo).to_uppercase(),
            v.rounds
        ),
    };
    col.spawn((Text::new(line), font_bold(12.0), TextColor(TEXT)));
    // Rust's turret is dead without its power (`sim_core::power`).
    if let Some(v) = core.turret_view(key) {
        let power = if v.powered {
            format!("POWER · {} rW", v.power_in)
        } else if v.power_in > 0 {
            format!("POWER · {} rW: NOT ENOUGH TO RUN IT", v.power_in)
        } else {
            "NO POWER: wire a generator, a battery or a solar panel to it".to_string()
        };
        col.spawn((Text::new(power), font_bold(12.0), TextColor(TEXT)));
    }
}

/// A right-click into the open turret: a gun to the weapon slot, anything
/// else to the ammo slots, topping up its own stack first. Out of it, the
/// pack's ordinary rule.
pub fn quick_move(
    handle: u32,
    from_kind: u8,
    from_slot: usize,
    catalog: &protocol::ItemCatalog,
    inv: &[ItemStack; INV_SLOTS],
    cont: &[ItemStack; INV_SLOTS],
    worn: &[ItemStack],
) -> Quick {
    if !is_own(from_kind) {
        return crate::ui::slots::quick_move(
            CONT_BOX, handle, from_kind, from_slot, catalog, inv, cont, worn,
        );
    }
    let src = match from_kind {
        CONT_SELF => inv.get(from_slot),
        CONT_WEAR => worn.get(from_slot),
        _ => None,
    }
    .copied()
    .unwrap_or_default();
    if src.count == 0 {
        return Quick::Refused("there is nothing in that slot");
    }
    let band = if catalog.row(src.item as usize).gun.is_some() {
        WEAPON_SLOT..WEAPON_SLOT + 1
    } else {
        AMMO_SLOTS
    };
    let cap = catalog.stack_max(src.item as usize);
    let mut empty = None;
    let mut pick = None;
    for s in band {
        let there = cont[s];
        if there.count == 0 {
            empty = empty.or(Some(s));
        } else if there.item == src.item && there.skin == src.skin && there.cond == src.cond {
            let room = cap.saturating_sub(there.count);
            if room > 0 {
                pick = Some((s, Grab::Fit(room)));
                break;
            }
        }
    }
    let Some((to_slot, grab)) = pick.or(empty.map(|s| (s, Grab::All))) else {
        return Quick::Refused("the turret has no room for that");
    };
    match move_args(
        handle, from_kind, from_slot, CONT_BOX, to_slot, grab, inv, cont, worn,
    ) {
        Some(args) => Quick::Send(args),
        None => Quick::Refused("that move cannot be addressed from here"),
    }
}

/// The list block: who it spares, then the buttons this viewer may press.
pub fn auth_block(col: &mut ChildSpawnerCommands, core: &ClientCore, key: u32) {
    let Some(view) = core.turret_view(key) else {
        col.spawn((Text::new("AUTHORIZED · …"), font(12.0), TextColor(TEXT_DIM)));
        return;
    };
    let me = core.player_id;
    let on = view.listed().contains(&me);
    col.spawn((
        Text::new(format!(
            "AUTHORIZED · {}{}",
            view.n,
            if on { " · YOU ARE ON IT" } else { "" }
        )),
        font_bold(12.0),
        TextColor(TEXT),
    ));
    if view.n == 0 {
        col.spawn((
            Text::new("nobody: it shoots everyone"),
            font(12.0),
            TextColor(TEXT_DIM),
        ));
    }
    for &id in view.listed() {
        col.spawn((
            Text::new(crate::ui::names::shown(core, id)),
            font(12.0),
            TextColor(TEXT_DIM),
        ));
    }
    col.spawn((
        Text::new(if view.peacekeeper {
            "MODE · PEACEKEEPER: only the hostile"
        } else {
            "MODE · ATTACK ALL: anyone off the list"
        }),
        font_bold(12.0),
        TextColor(TEXT),
    ));
    col.spawn(Node {
        flex_direction: FlexDirection::Row,
        column_gap: Val::Px(6.0),
        ..default()
    })
    .with_children(|row| {
        if view.may_edit && view.peacekeeper {
            button(row, "ATTACK ALL", ACCESS_OP_TURRET_ATTACK_ALL);
        } else if view.may_edit {
            button(row, "PEACEKEEPER", ACCESS_OP_TURRET_PEACEKEEPER);
        }
        if view.may_edit && !on {
            button(row, "AUTHORIZE", ACCESS_OP_TURRET_AUTH);
        }
        if on {
            button(row, "DEAUTHORIZE", ACCESS_OP_TURRET_DEAUTH);
        }
        if view.may_edit && view.n > 0 {
            button(row, "CLEAR LIST", ACCESS_OP_TURRET_CLEAR);
        }
    });
}

fn button(parent: &mut ChildSpawnerCommands, label: &str, op: u8) {
    parent
        .spawn((
            Button,
            TurretOp(op),
            Node {
                padding: UiRect::axes(Val::Px(10.0), Val::Px(5.0)),
                ..default()
            },
            BackgroundColor(CELL_BG),
            super::Hover {
                idle: CELL_BG,
                over: CELL_HOVER,
            },
        ))
        .with_children(|b| {
            b.spawn((
                Text::new(label.to_string()),
                font_bold(12.0),
                TextColor(TEXT),
                Pickable::IGNORE,
            ));
        });
}

/// A list button pressed: its op at the open turret's address.
pub fn clicks(
    mut ui: ResMut<Ui>,
    net: NonSend<super::super::Net>,
    buttons: Query<(&Interaction, &TurretOp), Changed<Interaction>>,
) {
    if ui.panel != Panel::Inventory {
        return;
    }
    let Some(op) = buttons
        .iter()
        .find(|(i, _)| **i == Interaction::Pressed)
        .map(|(_, o)| o.0)
    else {
        return;
    };
    let core = &net.session.core;
    let Some(key) = open_turret(core) else {
        return;
    };
    let (cx, cz, level, loc) = sim_core::deploy::box_addr(key);
    let mut buf = [0u8; protocol::MAX_STREAM_MSG_BYTES];
    match protocol::encode_action_access(cx, cz, level, loc, op, 0, &mut buf) {
        Ok(len) => match net.session.send_action(&buf[..len]) {
            Ok(()) => ui.status.clear(),
            Err(e) => ui.say(e.to_string()),
        },
        Err(e) => ui.say(format!("that would not send ({e:?})")),
    }
}
