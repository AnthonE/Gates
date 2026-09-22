//! The spectator's one addition to the screen: whose view this is (wire v73;
//! `NETCODE.md` §2.3). The words are `crate::ui::spectate`'s; this spawns them.
//!
//! **Bevy draws, it does not decide** — the mode lives in the session
//! (`Session::watching`) and in `ClientCore::spectator`, which drives
//! nothing, so nothing here can make a seat act. The rest of the HUD reads
//! the core exactly as a player's does, which is the point: the watcher sees
//! the watched player's health, hotbar and inventory, and any verb it tries
//! is refused before the wire (`Session::send_action` → `SendError::ReadOnly`).

use bevy::prelude::*;

/// The label's node, top centre.
#[derive(Component)]
pub struct SpectateLabel;

/// Spawn the label when this session is a seat; spawn nothing otherwise.
/// A `WorldEntity`, so leaving the world takes it down with everything else.
pub fn setup(mut commands: Commands, net: Option<NonSend<super::Net>>) {
    let Some(w) = net.and_then(|n| n.session.watching) else {
        return;
    };
    spawn_label(&mut commands, &w);
}

/// The label itself, split out so a headless `App` can spawn it
/// (`tests/spectate_label.rs`): a bundle is not type-checked for duplicate
/// components, so it is run rather than read (`CLAUDE.md`'s trap).
pub fn spawn_label(commands: &mut Commands, w: &protocol::Watch) -> Entity {
    commands
        .spawn((
            super::WorldEntity,
            SpectateLabel,
            Text::new(crate::ui::spectate::label(w)),
            super::ui::font_bold(20.0),
            TextColor(Color::srgba(0.97, 0.95, 0.88, 0.95)),
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(14.0),
                width: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                ..default()
            },
            TextLayout::new_with_justify(Justify::Center),
            Pickable::IGNORE,
        ))
        .id()
}
