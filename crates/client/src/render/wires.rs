//! Electricity on screen (`sim_core::power`): the wires between parts, and the
//! wire tool's two clicks.
//!
//! **The tool.** With the wire tool in hand, a left click on a part picks its
//! first free output; a second on another part runs the wire to that part's
//! first free input (`ACT_WIRE`), and a click on the first part again lets
//! it go. A right click on a part pulls every wire on it. While a wire is
//! half-run a guide line runs from the part to the crosshair. Which ports are
//! free is read off the wires this client was told (`ClientCore::wires`);
//! the sim answers the press either way.
//!
//! **The wires.** Each one a sagging cable between its two parts, rebuilt
//! when the wires or the deploy records move. A wire whose other end this
//! client does not hold is not drawn.

use bevy::prelude::*;
use sim_core::deploy::{box_key, DeployRec};
use sim_core::power::ports;

use super::hud::Toast;
use super::input::Look;
use super::panels::{Panel, Ui};
use super::verbs::Aimed;
use super::{Net, WorldEntity, WorldId, CROUCH_EYE_M, EYE_HEIGHT};
use crate::ui::hold::{click_in_hand, Click};
use crate::ui::interact::{part_name, Verb};

/// A half-run wire: the part it leaves and the output it leaves from.
#[derive(Resource, Default)]
pub struct WireTool {
    pending: Option<(u32, u8)>,
}

/// One drawn cable.
#[derive(Component)]
pub struct WireCable;

/// What the drawn cables were built from: `ClientCore::wires_gen` and how
/// many deploy records there were, so an end arriving later redraws.
#[derive(Resource, Default)]
pub struct WiresDrawn {
    seen: Option<(u32, usize)>,
    mat: Option<Handle<StandardMaterial>>,
}

/// How far a cable's middle hangs under its ends, as a share of its length.
const SAG: f32 = 0.06;
const CABLE_R_M: f32 = 0.012;
const CABLE: [f32; 4] = [0.08, 0.08, 0.09, 1.0];
/// A cable plugs in at this share of its part's height.
const PLUG_AT: f32 = 0.6;

/// Where a wire plugs into the part `rec` (its archetype `arch`).
fn plug(world: &WorldId, cols: &sim_core::collide::ColIndex, rec: &DeployRec, arch: u8) -> Vec3 {
    let (x, z) = rec.xz();
    let y = sim_core::deploy::body_base_y(world.seed, &world.haven, cols, rec, arch);
    let h = super::structures::deploy_size(arch as usize).y.max(0.2);
    Vec3::new(x, y + h * PLUG_AT, z)
}

/// The part at box address `key`, with its archetype.
fn part(core: &client_core::core::ClientCore, key: u32) -> Option<(DeployRec, u8)> {
    core.deploys.entries().iter().find_map(|d| {
        let arch = (u16::from(d.row) < core.deploy_defs_have)
            .then(|| core.deploy_defs.defs[d.row as usize].arch)?;
        (box_key(d.cx, d.cz, d.level, d.loc) == key && sim_core::power::is_part(arch))
            .then_some((*d, arch))
    })
}

/// The aimed thing as a power part's box key, when it is one.
fn aimed_part(aimed: &Aimed) -> Option<(u32, u8)> {
    let p = &aimed.0;
    let wired = matches!(
        p.verb,
        Verb::Switch | Verb::Part | Verb::Recycler | Verb::Box
    );
    (wired && p.handle != 0 && sim_core::power::is_part(p.arch)).then_some((p.handle, p.arch))
}

/// The wire tool: pick an output, run to an input, or pull a part's wires.
#[allow(clippy::too_many_arguments)]
pub fn tool(
    net: NonSend<Net>,
    aimed: Res<Aimed>,
    mouse: Res<ButtonInput<MouseButton>>,
    ui: Option<Res<Ui>>,
    chat: Option<Res<super::chat::Chat>>,
    mut toast: ResMut<Toast>,
    mut state: ResMut<WireTool>,
) {
    let core = &net.session.core;
    let click = click_in_hand(
        &core.catalog,
        &core.research,
        &core.deploy_defs,
        core.deploy_defs_have,
        &core.inv,
        net.sel,
    );
    if click != Click::Wire || core.dead {
        state.pending = None;
        return;
    }
    let busy = ui.as_ref().is_some_and(|u| u.panel != Panel::None)
        || chat.as_ref().is_some_and(|c| c.open());
    if busy {
        return;
    }
    let wires = core.wires();
    let send = |toast: &mut Toast,
                encode: &dyn Fn(&mut [u8]) -> Result<usize, protocol::WireError>| {
        let mut buf = [0u8; protocol::MAX_STREAM_MSG_BYTES];
        match encode(&mut buf) {
            Ok(len) => {
                if let Err(e) = net.session.send_action(&buf[..len]) {
                    toast.warn(e.to_string());
                }
            }
            Err(e) => toast.warn(format!("that wire would not send ({e:?})")),
        }
    };
    if mouse.just_pressed(MouseButton::Right) {
        if let Some((key, arch)) = aimed_part(&aimed) {
            state.pending = None;
            if wires.iter().any(|w| w.touches(key)) {
                send(&mut toast, &|buf| protocol::encode_action_unwire(key, buf));
            } else {
                toast.say(format!(
                    "the {} has no wires",
                    part_name(arch).to_lowercase()
                ));
            }
        }
        return;
    }
    if !mouse.just_pressed(MouseButton::Left) {
        return;
    }
    let Some((key, arch)) = aimed_part(&aimed) else {
        if state.pending.take().is_some() {
            toast.say("wire let go");
        }
        return;
    };
    let (ins, outs) = ports(arch);
    match state.pending {
        None => {
            let free = (0..outs).find(|&o| !wires.iter().any(|w| w.from == key && w.out == o));
            match free {
                Some(out) => {
                    state.pending = Some((key, out));
                    toast.say(format!(
                        "wire from the {}: click a part to power",
                        part_name(arch).to_lowercase()
                    ));
                }
                None if outs == 0 => toast.warn(format!(
                    "the {} has no output: wire from what feeds it",
                    part_name(arch).to_lowercase()
                )),
                None => toast.warn("every output on that part is wired"),
            }
        }
        Some((from, _)) if from == key => {
            state.pending = None;
            toast.say("wire let go");
        }
        Some((from, out)) => {
            let free = (0..ins).find(|&i| !wires.iter().any(|w| w.to == key && w.inp == i));
            match free {
                Some(inp) => {
                    state.pending = None;
                    send(&mut toast, &|buf| {
                        protocol::encode_action_wire(from, out, key, inp, buf)
                    });
                }
                None if ins == 0 => toast.warn(format!(
                    "the {} takes no power",
                    part_name(arch).to_lowercase()
                )),
                None => toast.warn("every input on that part is wired"),
            }
        }
    }
}

/// The half-run wire's guide, from its part to where the crosshair looks.
pub fn guide(
    net: NonSend<Net>,
    state: Res<WireTool>,
    look: Res<Look>,
    world: Option<Res<WorldId>>,
    mut gizmos: Gizmos,
) {
    let (Some((from, _)), Some(world)) = (state.pending, world) else {
        return;
    };
    let core = &net.session.core;
    let Some((rec, arch)) = part(core, from) else {
        return;
    };
    let a = plug(&world, core.pieces.cols(), &rec, arch);
    let [x, y, z] = core.predict.render_position();
    let eye_h = if core.crouched() {
        CROUCH_EYE_M
    } else {
        EYE_HEIGHT
    };
    let (fx, fz) = look.yaw.sin_cos();
    let (sv, ch) = look.pitch.sin_cos();
    let b = Vec3::new(x, y + eye_h, z) + Vec3::new(fx * ch, sv, fz * ch) * 2.0;
    gizmos.line(a, b, Color::srgb(0.95, 0.8, 0.2));
}

/// Rebuild the cables when the wires or the deploy records move.
pub fn draw(
    mut commands: Commands,
    net: NonSend<Net>,
    world: Option<Res<WorldId>>,
    mut drawn: ResMut<WiresDrawn>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    cables: Query<Entity, With<WireCable>>,
) {
    let Some(world) = world else {
        return;
    };
    let core = &net.session.core;
    let sig = (core.wires_gen, core.deploys.entries().len());
    if drawn.seen == Some(sig) {
        return;
    }
    drawn.seen = Some(sig);
    for e in cables.iter() {
        commands.entity(e).despawn();
    }
    let mat = drawn
        .mat
        .get_or_insert_with(|| {
            materials.add(StandardMaterial {
                base_color: Color::WHITE,
                perceptual_roughness: 0.7,
                ..default()
            })
        })
        .clone();
    let cols = core.pieces.cols();
    for w in core.wires() {
        let (Some((a, aa)), Some((b, ba))) = (part(core, w.from), part(core, w.to)) else {
            continue;
        };
        let (p, q) = (plug(&world, cols, &a, aa), plug(&world, cols, &b, ba));
        let mut soup = super::poles::Soup::default();
        span(&mut soup, p, q);
        if soup.is_empty() {
            continue;
        }
        commands.spawn((
            WireCable,
            WorldEntity,
            Mesh3d(meshes.add(soup.mesh())),
            MeshMaterial3d(mat.clone()),
            Transform::IDENTITY,
        ));
    }
}

/// One sagging cable from `p` to `q`.
fn span(soup: &mut super::poles::Soup, p: Vec3, q: Vec3) {
    let sag = (q - p).length() * SAG;
    const SEGS: usize = 10;
    let mut prev = p;
    for s in 1..=SEGS {
        let t = s as f32 / SEGS as f32;
        let pt = p.lerp(q, t) - Vec3::Y * (sag * 4.0 * t * (1.0 - t));
        soup.tube(prev, pt, CABLE_R_M, CABLE_R_M, 4, CABLE);
        prev = pt;
    }
}
