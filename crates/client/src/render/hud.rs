//! The HUD and the viewmodel — `ART.md` §6 and §8's last bullet.
//!
//! "A frame with no viewmodel and no HUD reads as a flythrough, which the
//! blind reader has named on every capture so far." It is the cheapest scored
//! criterion in the art rubric and the one the browser client had first, so
//! the native client is not shipping captures without it.
//!
//! The reference set's shape, measured off `Rust Images/`: a **bottom-centre
//! hotbar** with item cells, a **right-side vitals stack** with numbers —
//! small, unobtrusive, never centred — and a **held item** in the lower
//! right of frame.
//!
//! Every number drawn here is the server's. `hp_max` or `max_food` at zero
//! means no such message has ever arrived — a shard whose content disarms
//! combat or has no `[survival]` section never sends one — and the rule that
//! `core.rs` states for those fields is honoured: draw nothing rather than
//! draw an empty bar for a player who cannot be hurt.

use bevy::prelude::*;
use sim_core::limits::HOTBAR_SLOTS;

use super::rig::EyeCam;
use super::verbs::Aimed;
use super::Net;

/// How long a toast stays up, seconds. Cosmetic (`DECISIONS.md` §open,
/// client cosmetics). A clock is fine here and would not be in a gate: this
/// is a fade, not an assertion (`CLAUDE.md`, "a gate that waits on a clock
/// is not a gate").
pub const TOAST_SECS: f32 = 3.0;

/// How long the hitmarker flashes, seconds. Short on purpose — it is
/// confirmation, not a readout.
pub const HITMARK_SECS: f32 = 0.25;

/// The one line that says what just happened in the world: a refusal, a
/// full action lane, a verb that found nothing.
///
/// Distinct from `panels::Ui::status`, which says what happened *in a panel*
/// and is only on screen while one is open. Every refusal the sim announces
/// used to reach this client and stop — `pop_hit`, `pop_toast`,
/// `pop_craft_refusal`, `pop_build_refusal` and `pop_deploy_refusal` had zero
/// call sites in the whole render path.
#[derive(Resource, Default)]
pub struct Toast {
    pub text: String,
    pub left: f32,
    /// Seconds left on the hitmarker, counted down beside the text because
    /// the two are the same kind of thing: a brief confirmation the player
    /// reads without looking away from the crosshair.
    pub hit_left: f32,
    /// Damage the last landed hit dealt, drawn beside the marker.
    pub hit_damage: u16,
}

impl Toast {
    pub fn say(&mut self, what: impl Into<String>) {
        self.text = what.into();
        self.left = TOAST_SECS;
    }

    pub fn hit(&mut self, damage: u16) {
        self.hit_left = HITMARK_SECS;
        self.hit_damage = damage;
    }
}

/// The crosshair's resting colour, and the colour a landed hit flashes it.
/// Cosmetics (`DECISIONS.md` §open, client cosmetics).
const CROSSHAIR: Color = Color::srgba(0.94, 0.93, 0.89, 0.72);
const CROSSHAIR_HIT: Color = Color::srgba(0.98, 0.42, 0.30, 0.95);

/// A hotbar cell, by index.
#[derive(Component)]
pub struct Cell(usize);

/// The vitals readout.
#[derive(Component)]
pub struct Vitals;

/// What the build wheel last chose. Drawn beside the hotbar because a
/// selection nothing shows is a selection the player cannot trust — the
/// wheel latches a piece and does not place one yet (`render/ui/wheel.rs`),
/// so this line is the whole of its visible effect.
#[derive(Component)]
pub struct Plan;

/// The centre-screen verb hint: `[E] OPEN BOX`, and the compass strip's
/// neighbour in the reference frames. **This is how a player learns the
/// island has verbs at all** — a key nothing advertises is a key nobody
/// presses.
#[derive(Component)]
pub struct PromptLine;

/// The toast line, under the prompt.
#[derive(Component)]
pub struct ToastLine;

/// The crosshair's four ticks. A frame with no crosshair reads as a
/// flythrough for the same reason `ART.md` §8 says one with no viewmodel
/// does, and it is the only thing on screen that says where a swing goes.
#[derive(Component)]
pub struct Crosshair;

/// The hitmarker: the crosshair's ticks, recoloured for a quarter second
/// when a swing lands.
#[derive(Component)]
pub struct HitMark;

/// The compass strip. Bearing only — the reference also pins markers to it
/// (death skull, map pin) and ours carries none, because `ALPHA.md` §1 has a
/// rule about position an operator should read before we pin anything.
#[derive(Component)]
pub struct Compass;

pub fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    cam: Query<Entity, With<EyeCam>>,
) {
    // The hotbar: six cells, bottom centre.
    commands
        .spawn((
            super::WorldEntity,
            Node {
                position_type: PositionType::Absolute,
                bottom: Val::Px(18.0),
                width: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                column_gap: Val::Px(6.0),
                ..default()
            },
            Pickable::IGNORE,
        ))
        .with_children(|row| {
            for i in 0..HOTBAR_SLOTS {
                row.spawn((
                    Cell(i),
                    Node {
                        width: Val::Px(46.0),
                        height: Val::Px(46.0),
                        border: UiRect::all(Val::Px(2.0)),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.05, 0.05, 0.06, 0.55)),
                    BorderColor::all(Color::srgba(0.75, 0.72, 0.62, 0.35)),
                ));
            }
        });

    // The vitals stack: right side, small, never centred.
    commands.spawn((
        super::WorldEntity,
        Vitals,
        Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(20.0),
            right: Val::Px(22.0),
            ..default()
        },
        Text::new(""),
        TextFont {
            font_size: 17.0,
            ..default()
        },
        TextColor(Color::srgba(0.93, 0.91, 0.86, 0.92)),
    ));

    // The build plan, bottom left. Only ever text: the piece it names is a
    // client-side latch, not sim state.
    //
    // `WorldEntity` like its two neighbours: leaving a shard is a state change
    // now, and a HUD line that outlived the world would be drawn over the
    // server list.
    commands.spawn((
        super::WorldEntity,
        Plan,
        Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(20.0),
            left: Val::Px(22.0),
            ..default()
        },
        Text::new(""),
        TextFont {
            font_size: 14.0,
            ..default()
        },
        TextColor(Color::srgba(0.86, 0.83, 0.76, 0.80)),
        Pickable::IGNORE,
    ));

    // The crosshair: four ticks around a gap, never a dot. A dot vanishes
    // against light ground and a full cross hides the thing you are aiming
    // at; the reference uses ticks for both reasons.
    commands
        .spawn((
            super::WorldEntity,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Percent(50.0),
                top: Val::Percent(50.0),
                width: Val::Px(0.0),
                height: Val::Px(0.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            Pickable::IGNORE,
        ))
        .with_children(|c| {
            // (offset x, offset y, w, h) for the four ticks.
            for (dx, dy, w, h) in [
                (0.0, -9.0, 2.0, 7.0),
                (0.0, 9.0, 2.0, 7.0),
                (-9.0, 0.0, 7.0, 2.0),
                (9.0, 0.0, 7.0, 2.0),
            ] {
                c.spawn((
                    Crosshair,
                    HitMark,
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(dx - w * 0.5),
                        top: Val::Px(dy - h * 0.5),
                        width: Val::Px(w),
                        height: Val::Px(h),
                        ..default()
                    },
                    BackgroundColor(CROSSHAIR),
                    Pickable::IGNORE,
                ));
            }
        });

    // The centre prompt and the toast beneath it. Both sit BELOW the
    // crosshair rather than on it: text over the aim point is text in the way
    // of the thing it is describing.
    commands.spawn((
        super::WorldEntity,
        PromptLine,
        Node {
            position_type: PositionType::Absolute,
            top: Val::Percent(54.0),
            width: Val::Percent(100.0),
            justify_content: JustifyContent::Center,
            ..default()
        },
        Text::new(""),
        TextFont {
            font_size: 15.0,
            ..default()
        },
        TextColor(Color::srgba(0.96, 0.94, 0.88, 0.92)),
        TextLayout::new_with_justify(Justify::Center),
        Pickable::IGNORE,
    ));
    commands.spawn((
        super::WorldEntity,
        ToastLine,
        Node {
            position_type: PositionType::Absolute,
            top: Val::Percent(58.0),
            width: Val::Percent(100.0),
            justify_content: JustifyContent::Center,
            ..default()
        },
        Text::new(""),
        TextFont {
            font_size: 14.0,
            ..default()
        },
        TextColor(Color::srgba(0.98, 0.82, 0.55, 0.0)),
        TextLayout::new_with_justify(Justify::Center),
        Pickable::IGNORE,
    ));

    // The compass strip, top centre: a 90° window on the bearing, letters at
    // the cardinals. `hud.js`'s shape, in text rather than in a canvas.
    commands.spawn((
        super::WorldEntity,
        Compass,
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(14.0),
            width: Val::Percent(100.0),
            justify_content: JustifyContent::Center,
            ..default()
        },
        Text::new(""),
        TextFont {
            font_size: 15.0,
            ..default()
        },
        TextColor(Color::srgba(0.90, 0.88, 0.82, 0.75)),
        TextLayout::new_with_justify(Justify::Center),
        Pickable::IGNORE,
    ));

    // The viewmodel: a held item, lower right, parented to the camera so it
    // rides the view. Not an animation and not a weapon yet — the point of
    // §8's bullet is that the frame contains evidence a person is playing.
    if let Ok(cam) = cam.single() {
        let handle = materials.add(StandardMaterial {
            base_color: Color::srgb(0.30, 0.22, 0.14),
            perceptual_roughness: 0.85,
            ..default()
        });
        let head = materials.add(StandardMaterial {
            base_color: Color::srgb(0.42, 0.43, 0.45),
            perceptual_roughness: 0.42,
            metallic: 0.65,
            ..default()
        });
        commands.entity(cam).with_children(|c| {
            // Held low and to the right, angled across the frame. The first
            // cut put it near centre at arm's length and it read as a prop
            // floating in the world rather than as something carried — the
            // reference frames all show the item entering from the lower
            // right corner and leaving frame at the bottom.
            let hold = Transform::from_xyz(0.34, -0.30, -0.46).with_rotation(Quat::from_euler(
                EulerRot::YXZ,
                -0.42,
                0.42,
                0.10,
            ));
            c.spawn((
                Mesh3d(meshes.add(Cuboid::new(0.038, 0.038, 0.52))),
                MeshMaterial3d(handle),
                hold,
            ));
            c.spawn((
                Mesh3d(meshes.add(Cuboid::new(0.16, 0.045, 0.075))),
                MeshMaterial3d(head),
                hold * Transform::from_xyz(0.0, 0.02, -0.26),
            ));
        });
    }
}

/// Redraw from the core. Cheap enough per frame: six background colours and
/// one string.
#[allow(clippy::type_complexity)]
pub fn update(
    net: NonSend<Net>,
    mut cells: Query<(&Cell, &mut BorderColor, &mut BackgroundColor)>,
    mut vitals: Query<&mut Text, (With<Vitals>, Without<Plan>)>,
    mut plan: Query<&mut Text, (With<Plan>, Without<Vitals>)>,
    // `Option`, because a capture run does not register the menus at all.
    ui: Option<Res<super::panels::Ui>>,
) {
    let core = &net.session.core;

    if let (Ok(mut text), Some(ui)) = (plan.single_mut(), ui.as_ref()) {
        use crate::ui::build::{material_label, row_for, shape_label, MATERIALS, SHAPES};
        let shape = SHAPES[ui.shape.min(SHAPES.len() - 1)];
        let material = MATERIALS[ui.material.min(MATERIALS.len() - 1)];
        // Named only when the content actually has that piece — the wheel
        // draws a dead segment for a pair the shard did not bake, and the
        // HUD must not contradict it.
        let out = match row_for(&core.piece_defs, shape, material) {
            Some(_) => format!(
                "BUILD  {} {}   (hold B)",
                material_label(material),
                shape_label(shape)
            ),
            None => "BUILD  -   (hold B)".to_string(),
        };
        if text.0 != out {
            text.0 = out;
        }
    }

    for (cell, mut border, mut bg) in cells.iter_mut() {
        let selected = cell.0 == net.sel as usize;
        *border = BorderColor::all(if selected {
            Color::srgba(0.98, 0.86, 0.55, 0.95)
        } else {
            Color::srgba(0.75, 0.72, 0.62, 0.35)
        });
        *bg = BackgroundColor(if selected {
            Color::srgba(0.14, 0.13, 0.10, 0.72)
        } else {
            Color::srgba(0.05, 0.05, 0.06, 0.55)
        });
    }

    let Ok(mut text) = vitals.single_mut() else {
        return;
    };
    let mut out = String::new();
    // Zero max means the message has never arrived, which is not the same as
    // a zeroed vital — see the header.
    if core.hp_max > 0 {
        out.push_str(&format!("HP  {}/{}\n", core.hp, core.hp_max));
    }
    if core.max_food > 0 {
        out.push_str(&format!("FOOD {}\n", core.food));
    }
    if core.max_water > 0 {
        out.push_str(&format!("WATER {}", core.water));
    }
    if text.0 != out {
        text.0 = out;
    }
}

/// Drain the core's feedback rings into the toast, and count the timers down.
///
/// **Every one of these had zero readers before this slice.** The sim has
/// announced hits, refusals and gather toasts since M1; `ClientCore` decoded
/// them into bounded rings; the native client popped none of them, so a
/// refused craft, a refused placement and a landed hit were all silence.
///
/// A ring is drained to EMPTY each frame rather than one entry per frame:
/// they are small (`TOAST_RING`) and a backlog drip-fed at frame rate would
/// still be showing the first refusal after the tenth.
pub fn feedback(
    net: NonSend<Net>,
    feed: Res<super::feed::Feed>,
    mut toast: ResMut<Toast>,
    time: Res<Time>,
    mut marks: Query<&mut BackgroundColor, With<HitMark>>,
    mut lines: Query<(&mut Text, &mut TextColor), With<ToastLine>>,
) {
    let core = &net.session.core;

    // **Read, never popped.** `render::feed::drain` is the one caller of
    // `pop_*` in the client; this used to pop the rings itself, and the audio
    // systems popped the same ones — two correct halves that merged cleanly
    // and left the game silent. `feed.rs`'s header has the whole story.

    // Hits first: the marker is the only feedback with a deadline on it.
    if feed.hits > 0 {
        toast.hit(feed.damage);
    }

    // Refusals. Each store answers a different verb, and the reason codes are
    // integers by wall 3 — turning one into a sentence is the client's job
    // and `refusal_text` is where the whole mapping lives.
    for (which, code) in feed.refusals() {
        toast.say(match which {
            super::feed::Refused::Craft => crate::ui::refusals::craft(code),
            super::feed::Refused::Build => crate::ui::refusals::build(code),
            super::feed::Refused::Deploy => crate::ui::refusals::deploy(code),
        });
    }
    // The kill feed. Every death on the shard reaches this ring, and until
    // now the only reader was the mixer (`render::audio`) — so a death was
    // AUDIBLE and invisible, which is the half of `NOW.md` §0x item 6 that
    // cost nothing to close because the fact was already drained and sitting
    // in `Feed`.
    //
    // **Our own death is skipped, and it is in this ring.** `ClientCore`
    // buffers `(victim, killer)` for every `EV_DEATH` unconditionally and
    // only *then* asks whether the victim was us, so the death SCREEN
    // (`core.dead` + the `own_death_*` fields, `ui::death`) and this line
    // are fed by the same event. Without the skip a player would be told
    // they died twice, once in a sentence written for someone watching.
    //
    // No cause here: the ring carries `(victim, killer)` and nothing else,
    // so the feed says who, and the screen — which does have the cause, the
    // weapon and the range — says how.
    for &(victim, killer) in feed.deaths() {
        if let Some(line) = kill_line_for(victim, killer, core.player_id) {
            toast.say(line);
        }
    }

    // Gather and craft toasts are (item, count) pairs — something arrived.
    // `item_label` is the panels' own naming, reused rather than restated:
    // an index no name has dripped for prints as `#12`, which is honest,
    // where an empty cell is the dark-panel defect this repo has a rule
    // against.
    for &(item, count) in feed.gathered() {
        let label = crate::ui::craft::item_label(&core.catalog, item);
        toast.say(format!("+{count} × {label}"));
    }
    for &(item, count) in feed.crafted() {
        let label = crate::ui::craft::item_label(&core.catalog, item);
        toast.say(format!("crafted {count} × {label}"));
    }

    // ---- the timers -----------------------------------------------------
    let dt = time.delta_secs();
    if toast.left > 0.0 {
        toast.left = (toast.left - dt).max(0.0);
    }
    if toast.hit_left > 0.0 {
        toast.hit_left = (toast.hit_left - dt).max(0.0);
    }

    let hot = toast.hit_left > 0.0;
    for mut bg in marks.iter_mut() {
        let want = if hot { CROSSHAIR_HIT } else { CROSSHAIR };
        if bg.0 != want {
            bg.0 = want;
        }
    }

    if let Ok((mut text, mut color)) = lines.single_mut() {
        // Fade over the last second rather than vanishing, so a toast that
        // was replaced reads as replaced and not as a flicker.
        let alpha = (toast.left).min(1.0);
        if text.0 != toast.text {
            text.0 = toast.text.clone();
        }
        color.0 = color.0.with_alpha(alpha);
    }
}

/// The centre prompt and the compass.
pub fn prompt(
    aimed: Res<Aimed>,
    swung: Res<super::verbs::Swung>,
    in_weak: Res<super::verbs::InWeak>,
    look: Res<super::input::Look>,
    mut prompts: Query<&mut Text, (With<PromptLine>, Without<Compass>)>,
    mut compass: Query<&mut Text, (With<Compass>, Without<PromptLine>)>,
) {
    if let Ok(mut text) = prompts.single_mut() {
        // `E` outranks the swing, and the swing is drawn only where `E` is
        // silent. That ordering is the browser's and it is not arbitrary: a
        // box standing against a tree is both openable and choppable, and a
        // player who pressed `E` on a prompt that named a swing would spend
        // the wrong verb. One prompt, and the key it names is the key that
        // acts on it.
        let want = match aimed.0.prompt() {
            s if !s.is_empty() => s,
            _ => swing_prompt_weak(swung.0.occupant, in_weak.0),
        };
        if text.0 != want {
            text.0 = want;
        }
    }
    if let Ok(mut text) = compass.single_mut() {
        let want = compass_strip(look.yaw);
        if text.0 != want {
            text.0 = want;
        }
    }
}

/// The swing prompt, plus the weak-spot cue when the player is standing in
/// the sector the server announced for this node.
///
/// **The cue is the whole teaching surface for the mechanic.** The bonus has
/// been in the sim since gather v0 and the mark has been on the wire and
/// decoded for as long, with nothing drawing it — so a player could only
/// discover it by noticing that some swings paid more, which is not
/// discoverable at all. The suffix is deliberately on the same line as the
/// verb rather than a second element: it is a property of the swing you are
/// about to take, not a separate thing happening.
fn swing_prompt_weak(occupant: u8, in_weak: bool) -> String {
    let base = swing_prompt(occupant);
    if base.is_empty() || !in_weak {
        return base;
    }
    format!("{base}  ·  WEAK SPOT")
}

/// One kill-feed line for a `(victim, killer)` pair.
///
/// `killer == victim` is how the ring reports a death nobody dealt — the
/// clock, the sea, or a player's own hand — because the wire pair carries no
/// cause. The feed says who; the death screen says how.
fn kill_line(victim: u32, killer: u32) -> Option<String> {
    Some(if killer == victim {
        format!("#{victim} died")
    } else {
        format!("#{killer} killed #{victim}")
    })
}

/// [`kill_line`], but silent for our own death: it is in the same ring, and
/// `ui::death` already owns that sentence with the cause and the range.
fn kill_line_for(victim: u32, killer: u32, own: u32) -> Option<String> {
    if victim == own {
        return None;
    }
    kill_line(victim, killer)
}

/// What the crosshair says for a swing pick, or `""` for a whiff.
///
/// `[LMB]` rather than a verb name because the swing is a button, and the
/// button is the thing the player has to connect the text to — the same
/// reasoning `Pick::prompt` uses for naming `[E]`.
fn swing_prompt(occupant: u8) -> String {
    let label = crate::ui::interact::swing_label(occupant);
    if label.is_empty() {
        String::new()
    } else {
        format!("[LMB] {label}")
    }
}

/// The eight-point bearing plus degrees, e.g. `NE  045°`.
///
/// The number comes from [`crate::look::bearing_deg`] rather than from the
/// free-running yaw, so this and the map's heading are one fact drawn twice
/// and not two samples a frame apart.
fn compass_strip(yaw: f32) -> String {
    const POINTS: [&str; 8] = ["N", "NE", "E", "SE", "S", "SW", "W", "NW"];
    let deg = crate::look::bearing_deg(yaw);
    let idx = (((deg / 45.0) + 0.5) as usize) % 8;
    format!("{}   {:03.0}°", POINTS[idx], deg)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `E` outranks the swing and the swing fills the silence. Asserted
    /// here because it is an ORDERING, and an ordering is the one thing a
    /// compile cannot check — the browser shipped this as sixteen swept
    /// combinations in a gate that no longer exists.
    /// The kill-feed line, as a pure function of the pair the ring carries.
    /// Extracted so the sentence has a gate: the system around it needs a
    /// live `Net`, and a sentence assembled inside a Bevy system is one no
    /// headless test can read back (`ui::death`'s header, same argument).
    #[test]
    fn the_kill_feed_names_who_and_skips_our_own() {
        // A stranger killed by another stranger.
        assert_eq!(kill_line(9, 4), Some("#4 killed #9".to_string()));
        // Self-inflicted, or the world: the ring gives killer == victim.
        assert_eq!(kill_line(9, 9), Some("#9 died".to_string()));
        // Ours never reaches the feed — the death SCREEN owns it, and the
        // same EV_DEATH feeds both.
        assert_eq!(kill_line_for(9, 4, 9), None);
        assert_eq!(kill_line_for(9, 9, 9), None);
    }

    #[test]
    fn e_outranks_the_swing_and_the_swing_fills_the_silence() {
        use crate::ui::interact::{Pick, Verb};
        use sim_core::terrain::Occupant;

        let silent = Pick::default();
        assert_eq!(silent.prompt(), "", "a whiffed E must say nothing");

        // Where E is silent, the swing speaks.
        assert_eq!(swing_prompt(Occupant::Tree as u8), "[LMB] CHOP TREE");
        assert_eq!(
            swing_prompt(Occupant::BarrelSlot as u8),
            "[LMB] SMASH BARREL"
        );

        // Where E has something, it wins — the caller takes E's string first
        // and never reaches the swing. This asserts E is non-empty for every
        // verb, which is the precondition that makes that `match` correct.
        for v in [Verb::Door, Verb::Bag, Verb::Box, Verb::Hearth] {
            let p = Pick {
                verb: v,
                ..Default::default()
            };
            assert!(!p.prompt().is_empty(), "{v:?} must claim the prompt");
        }

        // And a swing at nothing is silent rather than "[LMB] ".
        assert_eq!(swing_prompt(0), "");
        assert_eq!(swing_prompt(Occupant::Rock as u8), "");
    }

    #[test]
    fn the_compass_walks_the_sims_bearing() {
        // Yaw 0 is +Z is north; a quarter turn toward +X is east.
        assert!(compass_strip(0.0).starts_with('N'));
        assert!(compass_strip(std::f32::consts::FRAC_PI_2).starts_with('E'));
        assert!(compass_strip(std::f32::consts::PI).starts_with('S'));
        assert!(compass_strip(3.0 * std::f32::consts::FRAC_PI_2).starts_with('W'));
        // And it wraps rather than reading 360.
        assert!(compass_strip(std::f32::consts::TAU - 0.001).starts_with('N'));
    }
}
