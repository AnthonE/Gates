//! The auto turret's own controls, under its box on the inventory page
//! (`sim_core::turret`): who is on its list, and the three list buttons —
//! AUTHORIZE (put yourself on), DEAUTHORIZE (take yourself off) and CLEAR
//! LIST (empty it, owner and all). Each sends `ACT_ACCESS` with its
//! `ACCESS_OP_TURRET_*` at the turret's address; the sim's verdict comes
//! back as the turret view (`EventMsg::Turret`) or a deploy refusal.

use bevy::prelude::*;
use client_core::core::ClientCore;
use sim_core::deploy::{
    box_key, ACCESS_OP_TURRET_AUTH, ACCESS_OP_TURRET_CLEAR, ACCESS_OP_TURRET_DEAUTH, ARCH_TURRET,
};
use sim_core::inventory::CONT_BOX;

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
    col.spawn(Node {
        flex_direction: FlexDirection::Row,
        column_gap: Val::Px(6.0),
        ..default()
    })
    .with_children(|row| {
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
