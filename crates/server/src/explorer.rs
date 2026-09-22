//! A local collect-wood skill over the human client's decoded state.
//! The model explores; this controller approaches and harvests a visible tree.
//! Geometry is shared with the renderer/sim, never read from a server World.

use crate::botclient::BotDriver;
use crate::jev::Driver;
use client_core::core::ClientCore;
use client_core::view::ClientView;
use protocol::{EntityState, Welcome, WireError};
use sim_core::collide;
use sim_core::gather::{cell_key, REACH_M};
use sim_core::input::{InputFrame, BTN_PRIMARY, BTN_SPRINT};
use sim_core::limits::{ARROW_STEP_MM, HOTBAR_SLOTS, TICK_HZ};
use sim_core::melee;
use sim_core::movement::{Body, POS_XZ_Q, POS_Y_Q, WADE_GROUND_MAX};
use sim_core::ranged::{ARROW_EYE_MM, MM_PER_M};
use sim_core::terrain::{self, Haven, Occupant, Slot, CELL_SIZE};
use sim_core::{pitch_dir, yaw_dir};
use std::time::Instant;

// Experiment defaults, DECISIONS.md §open. One cell is examined per input
// frame, so neither the search nor its storage grows with island size.
pub const SIGHT_M: f32 = 32.0;
pub const SIGHT_CELLS: i32 = (SIGHT_M / CELL_SIZE) as i32;
pub const NO_PROGRESS_TICKS: u32 = 3 * TICK_HZ;
pub const RECOVER_TICKS: u32 = TICK_HZ;
pub const FLEE_TICKS: u32 = 6 * TICK_HZ;
const SIGHT_WIDTH: i32 = 2 * SIGHT_CELLS + 1;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Phase {
    #[default]
    Waiting,
    Dead,
    Wounded,
    Sleeping,
    Stale,
    Exploring,
    Approaching,
    Harvesting,
    Recovering,
    Fleeing,
    NoTool,
    Full,
}

#[derive(Debug, Default)]
pub struct GatherStats {
    pub phase: Phase,
    pub targets_seen: u64,
    pub targets_completed: u64,
    pub targets_abandoned: u64,
    pub wood: u32,
    pub wood_gained: u32,
    pub gather_awards: u64,
    pub refusals: u64,
    pub retreats: u64,
}

#[derive(Clone, Copy, Debug)]
struct Target {
    cx: u16,
    cz: u16,
    slot: Slot,
}

impl Target {
    fn key(self) -> u32 {
        cell_key(self.cx, self.cz)
    }
}

pub struct Gatherer {
    pub explorer: Driver,
    pub stats: GatherStats,
    core: Option<Box<ClientCore>>,
    haven: Option<Haven>,
    target: Option<Target>,
    skipped: Option<u32>,
    scan: i32,
    seen_tick: Option<u32>,
    seen_at: Instant,
    progress_tick: u32,
    best_distance: f32,
    initial_wood: Option<u32>,
    target_wood: u32,
    recovery: Option<(u32, u16)>,
    retreat: Option<(u32, u16)>,
    resume: bool,
}

impl Gatherer {
    pub fn new(explorer: Driver) -> Self {
        Self {
            explorer,
            stats: GatherStats::default(),
            core: None,
            haven: None,
            target: None,
            skipped: None,
            scan: 0,
            seen_tick: None,
            seen_at: Instant::now(),
            progress_tick: 0,
            best_distance: f32::INFINITY,
            initial_wood: None,
            target_wood: 0,
            recovery: None,
            retreat: None,
            resume: false,
        }
    }

    fn abandon(&mut self, tick: u32, yaw: u16) {
        if let Some(target) = self.target.take() {
            self.skipped = Some(target.key());
            self.stats.targets_abandoned += 1;
        }
        self.recovery = Some((tick, yaw.wrapping_add(1 << 14)));
        self.resume = true;
    }

    fn frame_at(&mut self, view: &ClientView, player: u32, seq: u16, now: Instant) -> InputFrame {
        let mut frame = InputFrame {
            seq,
            pitch: 128,
            ..InputFrame::default()
        };
        if view.newest_applied.is_some() && view.newest_applied != self.seen_tick {
            self.seen_tick = view.newest_applied;
            self.seen_at = now;
        }
        let Some(body) = view.get(player).copied() else {
            self.stats.phase = Phase::Waiting;
            return frame;
        };
        frame.yaw = body.yaw;
        if body.dead
            || body.sleeping
            || body.wounded
            || now.duration_since(self.seen_at) >= self.explorer.snapshot_timeout()
        {
            self.target = None;
            self.recovery = None;
            self.retreat = None;
            self.stats.phase = if body.dead {
                Phase::Dead
            } else if body.wounded {
                Phase::Wounded
            } else if body.sleeping {
                Phase::Sleeping
            } else {
                Phase::Stale
            };
            return frame;
        }
        let tick = view.newest_applied.unwrap_or_default();
        let Some(core) = self.core.as_mut() else {
            return frame;
        };
        let wood = amount(core, b"Wood");
        if wood > self.stats.wood {
            self.progress_tick = tick;
        }
        self.stats.wood = wood;
        if let Some(initial) = self.initial_wood {
            self.stats.wood_gained = wood.saturating_sub(initial);
        }
        // A received damage bearing is the human's hit indicator, not an
        // opponent position. Escape interrupts work even with a full pack
        // or broken tool, and every fresh hit renews the bounded retreat.
        if let Some((mut start, mut yaw)) = self.retreat {
            if visible_pursuer(
                core,
                self.haven.as_ref().expect("connected haven"),
                &body,
                view,
            ) {
                start = tick;
                self.retreat = Some((start, yaw));
            }
            if tick.wrapping_sub(start) < FLEE_TICKS {
                if into_deeper_water(core, &body, yaw) {
                    yaw = yaw.wrapping_add(1 << 14);
                    self.retreat = Some((start, yaw));
                }
                self.stats.phase = Phase::Fleeing;
                // Keep the pursuer in view while retreating through ordinary
                // backwards movement. No unseen-body distance check is used.
                frame.yaw = yaw.wrapping_add(1 << 15);
                frame.move_z = -127;
                frame.buttons = BTN_SPRINT;
                return frame;
            }
            self.retreat = None;
            // We were looking backwards at the pursuer. Resume travelling
            // away, rather than turning the retreat into a return trip.
            self.explorer.face(yaw);
            self.resume = false;
        }
        // Catalog chunks can arrive after the inventory. Do not count
        // starting wood as newly gathered when its name becomes known.
        if self.initial_wood.is_none()
            && !(0..usize::from(core.catalog.count)).any(|row| core.catalog.name(row) == b"Wood")
        {
            self.stats.phase = Phase::Waiting;
            return frame;
        }
        let Some(sel) = tool(core) else {
            self.target = None;
            self.stats.phase = Phase::NoTool;
            return frame;
        };
        frame.sel = sel;
        let initial = *self.initial_wood.get_or_insert(wood);
        self.stats.wood_gained = wood.saturating_sub(initial);
        if !room_for_wood(core) {
            self.target = None;
            self.stats.phase = Phase::Full;
            return frame;
        }
        if let Some((start, yaw)) = self.recovery {
            if tick.wrapping_sub(start) < RECOVER_TICKS {
                self.stats.phase = Phase::Recovering;
                frame.yaw = yaw;
                frame.move_z = 127;
                return frame;
            }
            self.recovery = None;
        }
        if let Some(target) = self.target {
            if core.harvested.contains(target.key()) {
                if wood > self.target_wood {
                    self.stats.targets_completed += 1;
                }
                self.target = None;
                self.resume = true;
            }
        }
        if self.target.is_none() {
            let dx = self.scan % SIGHT_WIDTH - SIGHT_CELLS;
            let dz = self.scan / SIGHT_WIDTH - SIGHT_CELLS;
            self.scan = (self.scan + 1) % (SIGHT_WIDTH * SIGHT_WIDTH);
            let cx = (body.qx as f32 * POS_XZ_Q / CELL_SIZE).floor() as i32 + dx;
            let cz = (body.qz as f32 * POS_XZ_Q / CELL_SIZE).floor() as i32 + dz;
            if (0..terrain::CELLS_PER_SIDE).contains(&cx)
                && (0..terrain::CELLS_PER_SIDE).contains(&cz)
            {
                let (seed, island) = core.island();
                let slot = island.cache.slot(seed, island.table, island.haven, cx, cz);
                let target = Target {
                    cx: cx as u16,
                    cz: cz as u16,
                    slot,
                };
                if slot.occupant == Occupant::Tree
                    && Some(target.key()) != self.skipped
                    && !core.harvested.contains(target.key())
                    && in_view(&body, &slot)
                    && visible(
                        core,
                        self.haven.as_ref().expect("connected haven"),
                        &body,
                        target,
                    )
                {
                    self.target = Some(target);
                    self.target_wood = wood;
                    self.progress_tick = tick;
                    self.best_distance = f32::INFINITY;
                    self.stats.targets_seen += 1;
                    self.resume = true;
                }
            }
        }
        let Some(target) = self.target else {
            self.stats.phase = Phase::Exploring;
            if std::mem::take(&mut self.resume) {
                self.explorer.face(body.yaw);
            }
            let mut frame = self.explorer.frame(view, player, seq);
            frame.sel = sel;
            // The skill can see the shoreline under its next steps. Turn
            // toward another search direction before wading into deeper sea.
            if frame.move_z > 0 && into_deeper_water(core, &body, frame.yaw) {
                frame.yaw = frame.yaw.wrapping_add(1 << 14);
                frame.move_z = 0;
                frame.buttons = 0;
                self.explorer.face(frame.yaw);
            }
            return frame;
        };
        let (yaw, pitch, distance) = aim(&body, &target.slot);
        frame.yaw = yaw;
        frame.pitch = pitch;
        if distance + POS_XZ_Q < self.best_distance {
            self.best_distance = distance;
            self.progress_tick = tick;
        }
        if tick.wrapping_sub(self.progress_tick) >= NO_PROGRESS_TICKS {
            self.abandon(tick, body.yaw);
            return frame;
        }
        let ray = melee::ray(&as_body(body), yaw, pitch, REACH_M * MM_PER_M);
        let (seed, mut island) = core.island();
        let reached = melee::node_cast(seed, &mut island, &ray)
            .is_some_and(|hit| hit.cx == target.cx && hit.cz == target.cz);
        if reached {
            // Holding primary is the human harvesting verb. Keep a bystander
            // out of the swing: this skill never deliberately attacks a body.
            self.stats.phase = Phase::Harvesting;
            if !view.entities.iter().any(|(id, other)| {
                *id != player
                    && !other.dead
                    && ((other.qx - body.qx) as f32 * POS_XZ_Q)
                        .hypot((other.qz - body.qz) as f32 * POS_XZ_Q)
                        <= 2.0 * REACH_M
            }) && visible(
                core,
                self.haven.as_ref().expect("connected haven"),
                &body,
                target,
            ) {
                frame.buttons = BTN_PRIMARY;
            }
        } else {
            self.stats.phase = Phase::Approaching;
            frame.move_z = 127;
        }
        frame
    }
}

impl BotDriver for Gatherer {
    fn welcome(&mut self, welcome: &Welcome) {
        let mut core = Box::new(ClientCore::new(
            welcome.seed,
            welcome.player_id,
            welcome.tick,
        ));
        self.haven = Some(*core.island().1.haven);
        self.core = Some(core);
        self.seen_tick = Some(welcome.tick);
        self.seen_at = Instant::now();
    }

    fn event(&mut self, bytes: &[u8]) -> Result<(), WireError> {
        if let Some(core) = self.core.as_mut() {
            let flags = core.on_stream(bytes)?;
            while core.pop_toast().is_some() {
                self.stats.gather_awards += 1;
            }
            while core.pop_gather_refusal().is_some() {
                self.stats.refusals += 1;
                self.target = None;
                self.resume = true;
            }
            while let Some((sector, damage)) = core.pop_hurt() {
                if damage == 0 {
                    continue;
                }
                let toward =
                    (u32::from(sector) * 65536 / u32::from(sim_core::combat::HURT_SECTORS)) as u16;
                // Compass bearings turn toward -X; wire yaw turns toward +X.
                let away = 0u16.wrapping_sub(toward).wrapping_add(1 << 15);
                self.retreat = Some((self.seen_tick.unwrap_or_default(), away));
                self.recovery = None;
                if let Some(target) = self.target.take() {
                    self.skipped = Some(target.key());
                    self.stats.targets_abandoned += 1;
                }
                self.stats.retreats += 1;
                self.resume = true;
            }
            // Include the final swing's inventory reply during transport
            // settling, even when no further input frame will be requested.
            if flags & client_core::core::APPLIED_INV != 0 {
                let wood = amount(core, b"Wood");
                if wood > self.stats.wood {
                    self.progress_tick = self.seen_tick.unwrap_or_default();
                }
                self.stats.wood = wood;
                if let Some(initial) = self.initial_wood {
                    self.stats.wood_gained = wood.saturating_sub(initial);
                }
            }
        }
        Ok(())
    }

    fn frame(&mut self, view: &ClientView, player: u32, seq: u16) -> InputFrame {
        self.frame_at(view, player, seq, Instant::now())
    }
}

fn amount(core: &ClientCore, name: &[u8]) -> u32 {
    core.inv
        .iter()
        .filter(|s| core.catalog.name(s.item as usize) == name)
        .map(|s| u32::from(s.count))
        .sum()
}

fn tool(core: &ClientCore) -> Option<u8> {
    // Resolve names through the same catalog a human sees; never bake rows,
    // yields, durability or inventory positions into the controller.
    for name in [b"Metal Hatchet".as_slice(), b"Stone Hatchet", b"Rock"] {
        for (slot, stack) in core.inv[..HOTBAR_SLOTS].iter().enumerate() {
            if stack.count > 0
                && core.catalog.name(stack.item as usize) == name
                && (core.catalog.rows[stack.item as usize].cond_max == 0 || stack.cond > 0)
            {
                return Some(slot as u8);
            }
        }
    }
    None
}

fn room_for_wood(core: &ClientCore) -> bool {
    core.inv.iter().any(|s| {
        s.count == 0
            || (core.catalog.name(s.item as usize) == b"Wood"
                && s.count < core.catalog.rows[s.item as usize].stack_max)
    })
}

fn as_body(body: EntityState) -> Body {
    Body {
        qx: body.qx,
        qy: body.qy,
        qz: body.qz,
        qvy: body.qvy,
        grounded: body.grounded,
    }
}

fn into_deeper_water(core: &mut ClientCore, body: &EntityState, yaw: u16) -> bool {
    let x = body.qx as f32 * POS_XZ_Q;
    let z = body.qz as f32 * POS_XZ_Q;
    let (dx, dz) = yaw_dir(yaw);
    let (seed, island) = core.island();
    let here = terrain::ground(seed, island.haven, x, z);
    let ahead = terrain::ground(seed, island.haven, x + dx * REACH_M, z + dz * REACH_M);
    ahead <= WADE_GROUND_MAX && ahead < here
}

fn in_view(body: &EntityState, slot: &Slot) -> bool {
    in_cone(body, slot.x, slot.z)
}

fn in_cone(body: &EntityState, x: f32, z: f32) -> bool {
    let dx = x - body.qx as f32 * POS_XZ_Q;
    let dz = z - body.qz as f32 * POS_XZ_Q;
    let distance = dx.hypot(dz);
    let (fx, fz) = yaw_dir(body.yaw);
    distance <= SIGHT_M && dx * fx + dz * fz >= distance * std::f32::consts::FRAC_1_SQRT_2
}

fn aim(body: &EntityState, slot: &Slot) -> (u16, u8, f32) {
    let dx = slot.x - body.qx as f32 * POS_XZ_Q;
    let dz = slot.z - body.qz as f32 * POS_XZ_Q;
    let distance = dx.hypot(dz);
    let eye = body.qy as f32 * POS_Y_Q + ARROW_EYE_MM as f32 / MM_PER_M;
    let top = terrain::occupant_volume(slot.occupant).1 * slot.scale;
    let y = eye.clamp(
        slot.y + melee::MELEE_PROBE_M,
        slot.y + top - melee::MELEE_PROBE_M,
    );
    let yaw = (((dx.atan2(dz) / std::f32::consts::TAU * 256.0).round() as i32).rem_euclid(256)
        as u16)
        << 8;
    let pitch = (((y - eye).atan2(distance) / std::f32::consts::PI + 0.5) * 255.0).round() as u8;
    (yaw, pitch, distance)
}

/// Conservative line of sight to the near face of the trunk. Uses the same
/// terrain, occupant and built-volume queries as a player's projectile.
/// Split borrows let us reuse ClientCore's cache and column index directly.
fn visible(core: &mut ClientCore, haven: &Haven, body: &EntityState, target: Target) -> bool {
    let (_, pitch, distance) = aim(body, &target.slot);
    if distance > SIGHT_M || distance <= 0.0 {
        return false;
    }
    let eye = body.qy as f32 * POS_Y_Q + ARROW_EYE_MM as f32 / MM_PER_M;
    let radius = terrain::occupant_volume(target.slot.occupant).0 * target.slot.scale;
    let stop = (distance - radius - melee::MELEE_PROBE_M).max(0.0);
    let dx = (target.slot.x - body.qx as f32 * POS_XZ_Q) / distance;
    let dz = (target.slot.z - body.qz as f32 * POS_XZ_Q) / distance;
    let (horizontal, vertical) = pitch_dir(pitch);
    let dy = if horizontal > 0.0 {
        vertical / horizontal
    } else {
        0.0
    };
    let origin = (body.qx as f32 * POS_XZ_Q, body.qz as f32 * POS_XZ_Q);
    clear_line(
        core,
        haven,
        (origin.0, eye, origin.1),
        (origin.0 + dx * stop, eye + dy * stop, origin.1 + dz * stop),
    )
}

/// Check just the nearest candidate in the current view cone: at most one
/// extra sight ray per frame. An occluded nearer body can hide a farther one,
/// which is conservative; no body behind cover prolongs the retreat.
fn visible_pursuer(
    core: &mut ClientCore,
    haven: &Haven,
    body: &EntityState,
    view: &ClientView,
) -> bool {
    let mut nearest = None;
    let mut distance = f32::INFINITY;
    for (_, other) in &view.entities {
        if other.id == body.id || other.dead || other.sleeping {
            continue;
        }
        let x = other.qx as f32 * POS_XZ_Q;
        let z = other.qz as f32 * POS_XZ_Q;
        let d = (x - body.qx as f32 * POS_XZ_Q).hypot(z - body.qz as f32 * POS_XZ_Q);
        if d < distance && in_cone(body, x, z) {
            distance = d;
            nearest = Some((x, other.qy as f32 * POS_Y_Q + collide::CAPSULE_RADIUS_M, z));
        }
    }
    nearest.is_some_and(|to| {
        clear_line(
            core,
            haven,
            (
                body.qx as f32 * POS_XZ_Q,
                body.qy as f32 * POS_Y_Q + ARROW_EYE_MM as f32 / MM_PER_M,
                body.qz as f32 * POS_XZ_Q,
            ),
            to,
        )
    })
}

fn clear_line(
    core: &mut ClientCore,
    haven: &Haven,
    from: (f32, f32, f32),
    to: (f32, f32, f32),
) -> bool {
    let delta = (to.0 - from.0, to.1 - from.1, to.2 - from.2);
    let length = delta.0.hypot(delta.1).hypot(delta.2);
    if length > SIGHT_M {
        return false;
    }
    let steps = (length * MM_PER_M / ARROW_STEP_MM as f32).ceil() as usize;
    let point = |i: usize| {
        let t = i as f32 / steps.max(1) as f32;
        (
            from.0 + delta.0 * t,
            from.1 + delta.1 * t,
            from.2 + delta.2 * t,
        )
    };
    let (seed, mut island) = core.island();
    for i in 1..=steps {
        let (x, y, z) = point(i);
        if y <= terrain::ground(seed, haven, x, z) || island.blocks_volume(seed, x, z, y, 0.0, 0.0)
        {
            return false;
        }
    }
    let mut prev = (from.0, from.2);
    for i in 1..=steps {
        let (x, y, z) = point(i);
        if collide::shot_blocked(
            seed,
            haven,
            core.pieces.cols(),
            prev.0,
            prev.1,
            x,
            z,
            y,
            0.0,
        ) || collide::deploy_stop(seed, haven, core.pieces.cols(), x, z, y, 0.0).is_some()
        {
            return false;
        }
        prev = (x, z);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jev::{
        Action, Decision, DecisionSource, Observation, REQUEST_TIMEOUT, THINK_INTERVAL,
    };
    use protocol::{InvSlot, ItemRow};
    use sim_core::gather::ItemStack;
    use sim_core::movement::{quant_xz, quant_y};

    struct Wait;
    impl DecisionSource for Wait {
        fn decide(&mut self, _: Observation) -> Result<Decision, String> {
            Ok(Decision {
                action: Action::Wait,
                confidence: 1.0,
                input_tokens: 0,
            })
        }
    }

    fn fixture() -> (Gatherer, ClientView, Target) {
        let mut bot = Gatherer::new(Driver::new(Wait, THINK_INTERVAL, REQUEST_TIMEOUT).unwrap());
        bot.welcome(&Welcome {
            seed: 20260731,
            player_id: 1,
            tick: 0,
            dev: true,
        });
        let core = bot.core.as_mut().unwrap();
        core.catalog.count = 6;
        core.catalog
            .set(
                3,
                b"Rock",
                ItemRow {
                    cond_max: 100,
                    stack_max: 1,
                    ..ItemRow::EMPTY
                },
            )
            .unwrap();
        core.catalog
            .set(
                5,
                b"Wood",
                ItemRow {
                    stack_max: 1000,
                    ..ItemRow::EMPTY
                },
            )
            .unwrap();
        // Neither row nor hotbar slot matches the production spawn kit.
        core.inv[4] = ItemStack {
            item: 3,
            count: 1,
            cond: 100,
        };
        for cz in 40..216 {
            for cx in 40..216 {
                let (seed, island) = core.island();
                let slot = island.cache.slot(seed, island.table, island.haven, cx, cz);
                if slot.occupant != Occupant::Tree {
                    continue;
                }
                let y = terrain::ground(seed, bot.haven.as_ref().unwrap(), slot.x, slot.z - 5.0);
                if y < 1.0 || (y - slot.y).abs() > 0.3 {
                    continue;
                }
                let target = Target {
                    cx: cx as u16,
                    cz: cz as u16,
                    slot,
                };
                let body = EntityState {
                    id: 1,
                    qx: quant_xz(slot.x),
                    qy: quant_y(y),
                    qz: quant_xz(slot.z - 5.0),
                    grounded: true,
                    ..EntityState::default()
                };
                if !visible(core, bot.haven.as_ref().unwrap(), &body, target) {
                    continue;
                }
                let mut view = ClientView::new();
                view.newest_applied = Some(1);
                view.entities.push((1, body));
                return (bot, view, target);
            }
        }
        panic!("no visible tree fixture");
    }

    #[test]
    fn perception_rejects_behind_distant_harvested_and_wall_occluded_trees() {
        let (mut bot, view, target) = fixture();
        let body = view.get(1).unwrap();
        assert!(in_view(body, &target.slot));
        let mut back = *body;
        back.yaw = 1 << 15;
        assert!(!in_view(&back, &target.slot));
        let mut far = *body;
        far.qz -= quant_xz(SIGHT_M);
        assert!(!in_view(&far, &target.slot));
        let core = bot.core.as_mut().unwrap();
        let haven = bot.haven.as_ref().unwrap();
        assert!(visible(core, haven, body, target));
        use sim_core::build::{BuildContent, PieceRec, BUILD_CELL_M, LOC_EDGE_ZLO, SHAPE_WALL};
        core.piece_defs = BuildContent::probe_fixture();
        core.piece_defs_have = core.piece_defs.piece_count;
        let row = core
            .piece_defs
            .pieces
            .iter()
            .position(|p| p.shape == SHAPE_WALL)
            .unwrap() as u8;
        let cx = (target.slot.x / BUILD_CELL_M).floor() as u16;
        let cz = ((body.qz as f32 * POS_XZ_Q / BUILD_CELL_M).floor() as u16) + 1;
        let rec = PieceRec {
            cx,
            cz,
            row,
            loc: LOC_EDGE_ZLO,
            hp: 100,
            ..PieceRec::default()
        };
        let mut buf = [0u8; protocol::event::MAX_EVENT_MSG_BYTES];
        let n = protocol::event::encode_event_piece_sync(true, &[rec], &mut buf).unwrap();
        core.on_stream(&buf[..n]).unwrap();
        assert!(
            !visible(core, haven, body, target),
            "a built wall revealed the tree behind it"
        );
        let n = protocol::event::encode_event_piece_sync(true, &[], &mut buf).unwrap();
        core.on_stream(&buf[..n]).unwrap();
        assert!(visible(core, haven, body, target));
        let n = protocol::event::encode_event_slot_change(true, target.cx, target.cz, &mut buf)
            .unwrap();
        bot.event(&buf[..n]).unwrap();
        let now = Instant::now();
        for _ in 0..SIGHT_WIDTH * SIGHT_WIDTH {
            bot.frame_at(&view, 1, 1, now);
            assert!(
                bot.target.is_none_or(|t| t.key() != target.key()),
                "selected a harvested tree"
            );
        }
    }

    #[test]
    fn stalled_approach_abandons_the_target_instead_of_walking_forever() {
        let (mut bot, mut view, target) = fixture();
        bot.target = Some(target);
        let now = Instant::now();
        assert_ne!(bot.frame_at(&view, 1, 1, now).move_z, 0);
        view.newest_applied = Some(1 + NO_PROGRESS_TICKS);
        bot.frame_at(&view, 1, 2, now);
        assert!(bot.target.is_none());
        assert_eq!(bot.skipped, Some(target.key()));
        let frame = bot.frame_at(&view, 1, 3, now);
        assert_eq!(bot.stats.phase, Phase::Recovering);
        assert_ne!(frame.move_z, 0);
        assert_eq!(frame.buttons, 0);
    }

    #[test]
    fn a_nearby_body_interrupts_the_harvest_swing() {
        let (mut bot, mut view, target) = fixture();
        let now = Instant::now();
        let radius = terrain::occupant_volume(Occupant::Tree).0 * target.slot.scale;
        let body = &mut view.entities[0].1;
        body.qz = quant_xz(target.slot.z - radius - REACH_M / 2.0);
        body.qy = quant_y(terrain::ground(
            20260731,
            bot.haven.as_ref().unwrap(),
            body.qx as f32 * POS_XZ_Q,
            body.qz as f32 * POS_XZ_Q,
        ));
        bot.target = Some(target);
        assert_eq!(bot.frame_at(&view, 1, 1, now).buttons, BTN_PRIMARY);
        let mut bystander = *view.get(1).unwrap();
        bystander.id = 2;
        bystander.qx += quant_xz(REACH_M);
        view.entities.push((2, bystander));
        assert_eq!(bot.frame_at(&view, 1, 2, now).buttons, 0);
        view.entities.pop();
        assert_eq!(bot.frame_at(&view, 1, 3, now).buttons, BTN_PRIMARY);
    }

    #[test]
    fn stale_dead_wounded_broken_and_full_states_stop_inputs() {
        let (mut bot, mut view, target) = fixture();
        let timeout = REQUEST_TIMEOUT / 2;
        bot.explorer = Driver::new(Wait, THINK_INTERVAL, timeout).unwrap();
        let now = Instant::now();
        bot.target = Some(target);
        bot.frame_at(&view, 1, 1, now);
        let frame = bot.frame_at(&view, 1, 2, now + timeout);
        assert_eq!((frame.move_z, frame.buttons), (0, 0));
        for condition in 0..3 {
            view.newest_applied = Some(3 + condition);
            view.entities[0].1.dead = condition == 0;
            view.entities[0].1.wounded = condition == 1;
            view.entities[0].1.sleeping = condition == 2;
            bot.target = Some(target);
            let frame = bot.frame_at(&view, 1, 3, now + REQUEST_TIMEOUT);
            assert_eq!((frame.move_z, frame.buttons), (0, 0));
            assert!(bot.target.is_none());
        }
        view.entities[0].1.sleeping = false;
        let core = bot.core.as_mut().unwrap();
        core.inv[4].cond = 0;
        let frame = bot.frame_at(&view, 1, 4, now + REQUEST_TIMEOUT);
        assert_eq!(bot.stats.phase, Phase::NoTool);
        assert_eq!((frame.move_z, frame.buttons), (0, 0));
        let core = bot.core.as_mut().unwrap();
        core.inv.fill(ItemStack {
            item: 5,
            count: 1000,
            cond: 0,
        });
        core.inv[4] = ItemStack {
            item: 3,
            count: 1,
            cond: 100,
        };
        let frame = bot.frame_at(&view, 1, 5, now + REQUEST_TIMEOUT);
        assert_eq!(bot.stats.phase, Phase::Full);
        assert_eq!((frame.move_z, frame.buttons), (0, 0));
    }

    #[test]
    fn damage_interrupts_work_and_escapes_away_from_every_announced_bearing() {
        let (mut bot, mut view, target) = fixture();
        let now = Instant::now();
        let mut buf = [0u8; protocol::event::MAX_EVENT_MSG_BYTES];
        for sector in 0..sim_core::combat::HURT_SECTORS {
            view.newest_applied = Some(1 + u32::from(sector));
            bot.frame_at(&view, 1, 1, now);
            bot.target = Some(target);
            let n = protocol::event::encode_event_hurt(sector, 15, &mut buf).unwrap();
            bot.event(&buf[..n]).unwrap();
            let frame = bot.frame_at(&view, 1, 2, now);
            assert_eq!(bot.stats.phase, Phase::Fleeing);
            assert_eq!(frame.buttons, BTN_SPRINT, "a retreat must release primary");
            assert!(frame.move_z < 0 && bot.target.is_none());
            let (fx, fz) = yaw_dir(frame.yaw);
            assert_eq!(
                sim_core::combat::bearing_sector((fx * 1000000.0) as i64, (fz * 1000000.0) as i64),
                sector
            );
        }
        let (start, _) = bot.retreat.unwrap();
        // Escape still works when the only tool has broken.
        bot.core.as_mut().unwrap().inv[4].cond = 0;
        assert_eq!(bot.frame_at(&view, 1, 3, now).buttons, BTN_SPRINT);
        view.newest_applied = Some(start + FLEE_TICKS);
        let frame = bot.frame_at(&view, 1, 4, now);
        assert_eq!(bot.stats.phase, Phase::NoTool);
        assert_eq!((frame.move_z, frame.buttons), (0, 0));
    }

    #[test]
    fn retreat_continues_while_a_nearby_pursuer_is_visible_then_expires() {
        let (mut bot, mut view, _) = fixture();
        let now = Instant::now();
        bot.frame_at(&view, 1, 1, now);
        let mut pursuer = *view.get(1).unwrap();
        pursuer.id = 2;
        pursuer.qz += quant_xz(1.0);
        view.entities.push((2, pursuer));
        let mut buf = [0u8; protocol::event::MAX_EVENT_MSG_BYTES];
        let n = protocol::event::encode_event_hurt(0, 15, &mut buf).unwrap();
        bot.event(&buf[..n]).unwrap();
        view.newest_applied = Some(FLEE_TICKS + 1);
        assert_eq!(bot.frame_at(&view, 1, 2, now).buttons, BTN_SPRINT);
        assert_eq!(bot.stats.phase, Phase::Fleeing);
        view.entities.pop();
        view.newest_applied = Some(2 * FLEE_TICKS + 1);
        let frame = bot.frame_at(&view, 1, 3, now);
        assert_ne!(bot.stats.phase, Phase::Fleeing);
        assert_eq!(frame.yaw, 1 << 15, "resumed exploration toward the pursuer");
    }

    #[test]
    fn receipts_do_not_count_as_inventory_until_the_inventory_update_arrives() {
        let (mut bot, view, _) = fixture();
        let now = Instant::now();
        bot.frame_at(&view, 1, 1, now);
        let mut buf = [0u8; protocol::event::MAX_EVENT_MSG_BYTES];
        let n = protocol::event::encode_event_gather(5, 25, &mut buf).unwrap();
        bot.event(&buf[..n]).unwrap();
        bot.frame_at(&view, 1, 2, now);
        assert_eq!(bot.stats.gather_awards, 1);
        assert_eq!(bot.stats.wood_gained, 0);
        let n = protocol::event::encode_event_inv(
            &[InvSlot {
                slot: 2,
                stack: ItemStack {
                    item: 5,
                    count: 25,
                    cond: 0,
                },
            }],
            &mut buf,
        )
        .unwrap();
        bot.event(&buf[..n]).unwrap();
        assert_eq!(bot.stats.wood_gained, 25);
        assert_eq!(tool(bot.core.as_ref().unwrap()), Some(4));
    }

    #[test]
    fn delayed_catalog_does_not_count_starting_wood_as_a_gain() {
        let (mut bot, view, _) = fixture();
        let row = ItemRow {
            stack_max: 1000,
            ..ItemRow::EMPTY
        };
        let core = bot.core.as_mut().unwrap();
        let rock = core.catalog.rows[3];
        core.catalog = Default::default();
        core.catalog.count = 6;
        core.catalog.set(3, b"Rock", rock).unwrap();
        core.inv[2] = ItemStack {
            item: 5,
            count: 25,
            cond: 0,
        };
        let now = Instant::now();
        assert_eq!(bot.frame_at(&view, 1, 1, now).buttons, 0);
        assert_eq!(bot.stats.phase, Phase::Waiting);
        bot.core
            .as_mut()
            .unwrap()
            .catalog
            .set(5, b"Wood", row)
            .unwrap();
        bot.frame_at(&view, 1, 2, now);
        assert_eq!(bot.stats.wood, 25);
        assert_eq!(bot.stats.wood_gained, 0);
    }
}
