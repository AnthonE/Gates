//! Ore nodes as you mine them.
//!
//! Until this landed a stone, metal or sulfur node looked the same from the
//! first swing to the last and then vanished, throwing the same chips a plank
//! throws (operator, 2026-10-11: *"lack luster"*). The reference swaps a node
//! to a smaller, broken-down mesh at each stage of its yield and covers the
//! swap with a burst of rock and a crumble (Devblog 166: *"a much needed
//! particle effect and sound to mask the visual change that happens as the
//! node stage changes"*), marks its bonus spot with a shimmering orange glow
//! (Devblog 170), and bursts apart on the last hit.
//!
//! Ours, off facts the sim already announced (`RENDER.md` §1: it decides
//! nothing):
//!
//! - **Stages** (`EV_NODE_STAGE`, wire v106). Each quarter mined, the node
//!   sinks, squats and settles to a new lean and turn ([`stage_pose`]), rubble
//!   cut from its own mesh piles up at its foot, and a crumble covers it.
//! - **Every blow** shudders the node and flicks a chunk of it off, in its own
//!   colours: sulfur dust is yellow, a metal node sparks.
//! - **The break** throws the node apart as tumbling chunks of itself that
//!   bounce and settle under a plume, with the debris crash.
//! - **The hotspot** is a shimmer of light and twinkles where a tree's X would
//!   be, and a blow on it answers with a glint and a tink.
//!
//! The flying chunks are a fixed pool spawned at startup ([`DEBRIS_POOL`],
//! drop-oldest), `impact::Chips`' posture. Rubble is spawned when a stage
//! arrives and despawned when the node breaks or stands back up: an event,
//! never a frame.

use bevy::prelude::*;

use super::audio::Sound;
use super::decal::Marks;
use super::feed::Feed;
use super::fx::atlas::{GLOW, PUFF, STAR};
use super::fx::pool::{Orient, Particle, Pool};
use super::fx::table::{emit, emit_tint, lod, scaled, Layer};
use super::fx::Fx;
use super::impact::{skin_radius, Burst, Chips, Contacts, Matter, Weapon};
use super::shake::Shake;
use super::{Eye, Net, WorldId};
use crate::sound::mixer::Request;
use crate::sound::Cue;
use client_core::core::{APPLIED_MARK, APPLIED_SLOTS};
use sim_core::gather::{NO_CELL, ORE_STAGES, ORE_STAGE_TOP_PM};
use sim_core::ranged::SURF_WORLD;
use sim_core::terrain::{self, Occupant};

/// Width kept at each stage when there is no cut mesh to draw and the whole
/// node is squashed instead (`OreCuts` not ready, or the fallback blob).
pub const STAGE_W: [f32; ORE_STAGES as usize] = [1.0, 0.95, 0.89, 0.83];
/// The most a worn node leans, radians (~7°), and turns from where it stood
/// (~26°) — so two nodes at one stage are two rocks, not one rock twice.
pub const STAGE_LEAN_RAD: f32 = 0.12;
pub const STAGE_TURN_RAD: f32 = 0.45;

/// Half the node mesh's height at scale 1, metres: `measure_glb`'s 1.254 m
/// box, which the kit nodes fill. What the pose keeps the foot planted by.
pub const MESH_HALF_H_M: f32 = 0.627;
/// How far below the sim's top a cut leaves the drawn rock, metres at scale 1:
/// the sim's skin carries a probe's forgiveness, the drawn one need not.
pub const CUT_MARGIN_M: f32 = 0.05;

/// Cut meshes per stage and kind, so two nodes at one stage are not one
/// shape (the turn and lean then make each its own).
pub const CUT_VARIANTS: usize = 2;
/// Chips a stage-3 mesh carries: three cap facets and a shoulder per stage.
const MAX_PLANES: usize = 4 * (ORE_STAGES as usize - 1);

/// A blow's shudder: how long, how far it rocks (radians), how fast, and how
/// much it squashes.
pub const HIT_S: f32 = 0.4;
pub const HIT_ROCK_RAD: f32 = 0.035;
pub const HIT_HZ: f32 = 9.0;
pub const HIT_SQUASH: f32 = 0.05;
/// A crumble's settle: how long the pop rings, and how fast the drawn stage
/// eases onto the new one (stages per second).
pub const CRACK_S: f32 = 0.45;
pub const CRACK_RATE: f32 = 7.0;
pub const CRACK_POP: f32 = 0.07;

/// Rubble left at the foot for each stage reached.
pub const RUBBLE_PER_STAGE: usize = 3;
const RUBBLE_MAX: usize = RUBBLE_PER_STAGE * (ORE_STAGES as usize - 1);

/// Chunks of node in flight at once. A break throws twelve, a crumble five, a
/// blow one or two; a crowd at one rock still fits.
pub const DEBRIS_POOL: usize = 64;
/// Chunks a break, a crumble and a blow throw.
pub const BREAK_CHUNKS: usize = 12;
pub const CRUMBLE_CHUNKS: usize = 5;
/// How long a thrown chunk lies before it is gone, seconds (the mean).
pub const DEBRIS_LIFE_S: f32 = 2.6;
const DEBRIS_GRAVITY: f32 = 9.81;
/// What a bounce keeps of a chunk's fall, and of its slide.
const DEBRIS_BOUNCE: f32 = 0.32;
const DEBRIS_SLIDE: f32 = 0.55;

/// Shake for a crumble and a break: trauma, full inside, none past (metres).
const CRUMBLE_TRAUMA: f32 = 0.12;
const BREAK_TRAUMA: f32 = 0.32;

/// The hotspot's shimmer: core glows a second, twinkles a second, and how far
/// out from the rock it burns (metres).
const SHIMMER_GLOW_HZ: f32 = 30.0;
const SHIMMER_STAR_HZ: f32 = 11.0;
const SHIMMER_LIFT_M: f32 = 0.05;
/// How wide the twinkles scatter round the hotspot, metres.
const SHIMMER_R_M: f32 = 0.08;

/// Which ore a node is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OreKind {
    Stone,
    Metal,
    Sulfur,
}

impl OreKind {
    pub fn of(o: Occupant) -> Option<Self> {
        match o {
            Occupant::StoneNode => Some(Self::Stone),
            Occupant::MetalNode => Some(Self::Metal),
            Occupant::SulfurNode => Some(Self::Sulfur),
            _ => None,
        }
    }

    fn occupant(self) -> Occupant {
        match self {
            Self::Stone => Occupant::StoneNode,
            Self::Metal => Occupant::MetalNode,
            Self::Sulfur => Occupant::SulfurNode,
        }
    }

    /// The cloud it gives off: pale granite, dark iron, sulfur's yellow.
    pub fn dust(self) -> Color {
        match self {
            Self::Stone => Color::srgb(0.66, 0.64, 0.60),
            Self::Metal => Color::srgb(0.48, 0.42, 0.36),
            Self::Sulfur => Color::srgb(0.86, 0.76, 0.32),
        }
    }

    /// The flecks it sheds: quartz-white, rust, crust-yellow.
    pub fn grit(self) -> Color {
        match self {
            Self::Stone => Color::srgb(0.80, 0.78, 0.74),
            Self::Metal => Color::srgb(0.62, 0.36, 0.18),
            Self::Sulfur => Color::srgb(0.95, 0.84, 0.30),
        }
    }

    fn matter(self) -> Matter {
        match self {
            Self::Metal => Matter::Metal,
            _ => Matter::Stone,
        }
    }
}

/// An ore node as drawn. Inserted beside its `props::Fellable` at spawn.
#[derive(Component)]
pub struct OreNode {
    pub key: u32,
    pub kind: OreKind,
    /// Where it stands: the slot's x and z, and the y the bottom of its mesh
    /// sits at whole. Scale and yaw are the slot's.
    pub foot: Vec3,
    pub scale: f32,
    pub yaw: f32,
    /// The whole node's mesh, which the cuts are made from and a respawn
    /// goes back to.
    pub base: Handle<Mesh>,
    /// The stage the wire says, and the stage drawn, eased toward it.
    target: u8,
    shown: f32,
    since_hit: f32,
    hit_axis: Vec3,
    since_crack: f32,
    rubble: [Option<Entity>; RUBBLE_MAX],
    /// Whether this node's pose has been written since it spawned: a node
    /// streamed in already worn must be drawn worn on its first frame.
    posed: bool,
}

impl OreNode {
    pub fn new(
        key: u32,
        kind: OreKind,
        foot: Vec3,
        scale: f32,
        yaw: f32,
        base: Handle<Mesh>,
    ) -> Self {
        Self {
            key,
            kind,
            foot,
            scale,
            yaw,
            base,
            target: 0,
            shown: 0.0,
            since_hit: HIT_S,
            hit_axis: Vec3::X,
            since_crack: CRACK_S,
            rubble: [None; RUBBLE_MAX],
            posed: false,
        }
    }

    /// The stage drawn right now, eased.
    pub fn shown(&self) -> f32 {
        self.shown
    }

    fn animating(&self) -> bool {
        !self.posed
            || self.shown != self.target as f32
            || self.since_hit < HIT_S
            || self.since_crack < CRACK_S
    }

    /// The top of what is left, roughly: where a crumble comes off.
    fn top(&self) -> Vec3 {
        self.foot + Vec3::Y * (stage_top_m(self.kind, self.shown) - CUT_MARGIN_M) * self.scale
    }

    /// The middle of what is left: where a break comes from.
    fn middle(&self) -> Vec3 {
        self.foot + Vec3::Y * stage_top_m(self.kind, self.shown) * 0.55 * self.scale
    }

    /// Which cut of its stage this node draws.
    fn variant(&self) -> usize {
        (super::props::hash01(self.key, 0x0C07_0000) * CUT_VARIANTS as f32) as usize % CUT_VARIANTS
    }

    fn radius(&self) -> f32 {
        skin_radius(self.kind.occupant() as u8) * self.scale
    }
}

/// How far a node's mesh is buried at scale 1: the mesh is centred, and
/// `props::spawn_slot` seats its middle `archetype_lift − SINK_M` above the
/// slot's ground.
fn bury_m(kind: OreKind) -> f32 {
    MESH_HALF_H_M - (super::props::archetype_lift(kind.occupant()) - super::props::SINK_M)
}

/// How high above its mesh's bottom an ore node stands at `stage` (which may
/// be between two), metres at scale 1: the sim's own top for that stage
/// (`ORE_STAGE_TOP_PM` of the occupant's volume), so what is drawn is what a
/// swing strikes and a body stands on.
pub fn stage_top_m(kind: OreKind, stage: f32) -> f32 {
    let s = stage.clamp(0.0, (ORE_STAGES - 1) as f32);
    let (i, t) = (s.floor() as usize, s.fract());
    let j = (i + 1).min(ORE_STAGES as usize - 1);
    let pm = |k: usize| ORE_STAGE_TOP_PM[k] as f32 * 0.001;
    let (_, top) = terrain::occupant_volume(kind.occupant());
    bury_m(kind) + top * (pm(i) + (pm(j) - pm(i)) * t)
}

/// How a stage draws: the height and width kept when the node is squashed
/// rather than cut, the lean (a tilt about x and about z, radians) and the
/// turn from the slot's own yaw.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    pub h: f32,
    pub w: f32,
    pub lean: Vec2,
    pub turn: f32,
}

fn stage_pose_at(kind: OreKind, key: u32, i: usize) -> Pose {
    let i = i.min(ORE_STAGES as usize - 1);
    if i == 0 {
        return Pose {
            h: 1.0,
            w: 1.0,
            lean: Vec2::ZERO,
            turn: 0.0,
        };
    }
    let salt = 0x0E5A_6E00 + i as u32;
    let dir = super::props::hash01(key, salt) * std::f32::consts::TAU;
    let mag = STAGE_LEAN_RAD * (0.5 + 0.5 * super::props::hash01(key, salt ^ 0x00FF_0000));
    let turn = (super::props::hash01(key, salt ^ 0x0F00_0F00) * 2.0 - 1.0) * STAGE_TURN_RAD;
    Pose {
        h: ((stage_top_m(kind, i as f32) - CUT_MARGIN_M) / (2.0 * MESH_HALF_H_M)).min(1.0),
        w: STAGE_W[i],
        lean: Vec2::new(dir.cos(), dir.sin()) * mag,
        turn,
    }
}

/// The pose of node `key` at stage `stage`, which may be between two while it
/// eases: whole at 0, and each worn stage its own lean and turn off the key,
/// so two players see the same rock.
pub fn stage_pose(kind: OreKind, key: u32, stage: f32) -> Pose {
    let s = stage.clamp(0.0, (ORE_STAGES - 1) as f32);
    let (i, t) = (s.floor() as usize, s.fract());
    let (a, b) = (stage_pose_at(kind, key, i), stage_pose_at(kind, key, i + 1));
    Pose {
        h: a.h + (b.h - a.h) * t,
        w: a.w + (b.w - a.w) * t,
        lean: a.lean.lerp(b.lean, t),
        turn: a.turn + (b.turn - a.turn) * t,
    }
}

/// The node's transform for pose `p`, rocked by `rock` (an extra rotation)
/// and squashed by `squash`, its whole size times `pop`. `cut` says the mesh
/// drawn is already cut down to the stage, so only the squash of a squashed
/// node is skipped. The foot stays where it was.
pub fn pose_transform(
    n: &OreNode,
    p: &Pose,
    rock: Quat,
    squash: f32,
    pop: f32,
    cut: bool,
) -> Transform {
    let s = n.scale;
    // A cut mesh carries the stage's height; it still narrows with it.
    let ph = if cut { 1.0 } else { p.h };
    let pw = p.w;
    let h = ph * (1.0 - squash) * pop;
    let w = pw * (1.0 + squash * 0.5) * pop;
    let lean = Quat::from_rotation_x(p.lean.y) * Quat::from_rotation_z(p.lean.x);
    Transform {
        translation: Vec3::new(n.foot.x, n.foot.y + MESH_HALF_H_M * h * s, n.foot.z),
        rotation: rock * lean * Quat::from_rotation_y(n.yaw + p.turn),
        scale: Vec3::new(w * s, h * s, w * s),
    }
}

// ─── The cut meshes ──────────────────────────────────────────────────────────

/// The chips taken off a node to reach `stage`, in mesh space: each a plane
/// `(n, d)` that keeps `n·p ≤ d`. Cumulative, so a stage keeps every cut of
/// the stage before it.
///
/// **The cap is three facets, not one plane.** A single tilted plane down to
/// the stage's height read as a sawn lid on a stump; three faces at rough
/// thirds round the node, each dropped a little below the stage's top, meet
/// in a ridge off the middle — a top that broke rather than was cut. Gently:
/// facets steep enough to reach the ground left stage 3 a pale triangle lying
/// in the dirt (seen in the game, 2026-10-12).
/// The highest point left is never above the sim's top for the stage, less
/// [`CUT_MARGIN_M`]. Then one chip off a shoulder.
pub fn stage_planes(
    kind: OreKind,
    variant: usize,
    stage: usize,
) -> ([(Vec3, f32); MAX_PLANES], usize) {
    let mut out = [(Vec3::Y, f32::MAX); MAX_PLANES];
    let mut n = 0;
    let salt = 0x5EED_0000 + kind as u32 * 0x100 + variant as u32 * 0x10;
    let h = |s: usize, k: u32| super::props::hash01(salt, s as u32 * 16 + k);
    let third = std::f32::consts::TAU / 3.0;
    for s in 1..=stage.min(ORE_STAGES as usize - 1) {
        let top = stage_top_m(kind, s as f32) - CUT_MARGIN_M - MESH_HALF_H_M;
        let base = h(s, 0) * std::f32::consts::TAU;
        for k in 0..3u32 {
            let a = base + k as f32 * third + (h(s, 1 + k) - 0.5) * 0.9;
            let tilt = 0.14 + 0.2 * h(s, 4 + k);
            let nrm = Vec3::new(a.cos() * tilt, 1.0, a.sin() * tilt).normalize();
            // Through the node's axis at `top − drop`: no facet stands past
            // the stage's top, and their different drops move the ridge.
            out[n] = (nrm, nrm.y * (top - 0.12 * h(s, 7 + k)));
            n += 1;
        }
        let b = h(s, 10) * std::f32::consts::TAU;
        let e = 0.6 + 0.4 * h(s, 11);
        let nrm = Vec3::new(b.cos() * e.cos(), e.sin(), b.sin() * e.cos());
        out[n] = (nrm, 0.45 + 0.13 * h(s, 12));
        n += 1;
    }
    (out, n)
}

/// The fresh face a cut leaves, as a vertex tint over the ore's own surface,
/// set off from it either way: the pale granite breaks a shade greyer (paler
/// still washed out in the sun), dark iron breaks brighter, and sulfur
/// breaks yellower.
pub fn fresh_tint(kind: OreKind) -> [f32; 4] {
    match kind {
        OreKind::Stone => [0.84, 0.83, 0.81, 1.0],
        OreKind::Metal => [1.14, 1.06, 0.98, 1.0],
        OreKind::Sulfur => [1.04, 0.96, 0.66, 1.0],
    }
}

/// `src` with every vertex past a plane moved onto it: the chip's flat face,
/// its normal the plane's and its tint `fresh`. `None` for a mesh with no
/// positions or normals to cut.
pub fn cut_mesh(src: &Mesh, planes: &[(Vec3, f32)], fresh: [f32; 4]) -> Option<Mesh> {
    use bevy::mesh::VertexAttributeValues as V;
    let Some(V::Float32x3(pos)) = src.attribute(Mesh::ATTRIBUTE_POSITION) else {
        return None;
    };
    let Some(V::Float32x3(nrm)) = src.attribute(Mesh::ATTRIBUTE_NORMAL) else {
        return None;
    };
    let (mut pos, mut nrm) = (pos.clone(), nrm.clone());
    let mut col = vec![[1.0f32; 4]; pos.len()];
    for i in 0..pos.len() {
        let mut v = Vec3::from(pos[i]);
        let mut cut = None;
        // Twice round: a vertex moved onto one plane can land past another.
        for _ in 0..2 {
            for &(n, d) in planes {
                let over = n.dot(v) - d;
                if over > 0.0 {
                    v -= n * over;
                    cut = Some(n);
                }
            }
        }
        if let Some(n) = cut {
            pos[i] = v.to_array();
            nrm[i] = n.to_array();
            col[i] = fresh;
        }
    }
    let mut m = src.clone();
    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
    m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, nrm);
    m.insert_attribute(Mesh::ATTRIBUTE_COLOR, col);
    if m.attribute(Mesh::ATTRIBUTE_TANGENT).is_some() {
        // The normal map needs tangents that follow the new faces; a mesh
        // that cannot have them keeps its old ones, a little wrong on a chip.
        let _ = m.generate_tangents();
    }
    Some(m)
}

/// The cut meshes, by kind, stage and variant — built once, off the first node
/// of each kind whose mesh has loaded.
#[derive(Resource, Default)]
pub struct OreCuts {
    meshes: [[[Option<Handle<Mesh>>; CUT_VARIANTS]; ORE_STAGES as usize - 1]; 3],
    done: [bool; 3],
}

impl OreCuts {
    /// The mesh node `n` draws at `stage`, or `None` while it is whole or the
    /// cuts are not built.
    fn mesh(&self, n: &OreNode, stage: u8) -> Option<&Handle<Mesh>> {
        let s = (stage as usize).checked_sub(1)?;
        self.meshes[n.kind as usize].get(s)?[n.variant()].as_ref()
    }
}

/// Build each kind's cut meshes once its model is in.
pub fn cut(mut cuts: ResMut<OreCuts>, mut meshes: ResMut<Assets<Mesh>>, nodes: Query<&OreNode>) {
    if cuts.done.iter().all(|d| *d) {
        return;
    }
    for n in &nodes {
        let k = n.kind as usize;
        if cuts.done[k] {
            continue;
        }
        let Some(src) = meshes.get(&n.base).cloned() else {
            continue;
        };
        for s in 1..ORE_STAGES as usize {
            for v in 0..CUT_VARIANTS {
                let (planes, count) = stage_planes(n.kind, v, s);
                if let Some(m) = cut_mesh(&src, &planes[..count], fresh_tint(n.kind)) {
                    cuts.meshes[k][s - 1][v] = Some(meshes.add(m));
                }
            }
        }
        cuts.done[k] = true;
    }
}

/// The rock and squash of a blow `t` seconds ago, about `axis`.
pub fn shudder(t: f32, axis: Vec3) -> (Quat, f32) {
    if !(0.0..HIT_S).contains(&t) {
        return (Quat::IDENTITY, 0.0);
    }
    let rock = HIT_ROCK_RAD * (-t * 12.0).exp() * (t * HIT_HZ * std::f32::consts::TAU).sin();
    let squash = HIT_SQUASH * (-t * 20.0).exp();
    (Quat::from_axis_angle(axis, rock), squash)
}

/// A crumble's pop `t` seconds on: a squeeze that rings out.
pub fn crack_pop(t: f32) -> f32 {
    if !(0.0..CRACK_S).contains(&t) {
        return 1.0;
    }
    1.0 - CRACK_POP * (-t * 10.0).exp() * (t * 4.0 * std::f32::consts::TAU).cos()
}

// ─── The chunks ──────────────────────────────────────────────────────────────

/// One chunk of node in flight, or lying where it fell. `left <= 0` is free.
#[derive(Clone, Default)]
struct Chunk {
    left: f32,
    life: f32,
    pos: Vec3,
    vel: Vec3,
    rot: Quat,
    spin: Vec3,
    size: f32,
    ground: f32,
    mesh: Handle<Mesh>,
    mat: Handle<StandardMaterial>,
    /// The handles changed since they were copied onto the entity.
    dirty: bool,
}

/// What a throw needs: where from, which way, how hard, how big, what of.
pub struct Throw<'a> {
    pub at: Vec3,
    pub away: Vec3,
    pub speed: (f32, f32),
    pub up: f32,
    /// Chunk size as a share of the node mesh (1.83 m across at scale 1).
    pub size: (f32, f32),
    pub life: f32,
    /// How far from `at` the chunks start, metres.
    pub spread: f32,
    pub mesh: &'a Handle<Mesh>,
    pub mat: &'a Handle<StandardMaterial>,
}

/// The chunk pool.
#[derive(Resource, Default)]
pub struct Debris {
    chunks: Vec<Chunk>,
    entities: Vec<Entity>,
    next: usize,
    rng: u32,
    /// Chunks thrown since the world was entered, and live ones recycled.
    pub thrown: u64,
    pub stolen: u64,
}

impl Debris {
    /// A pool of `n` free chunks with no entities: the pure half, for tests.
    pub fn with_cap(n: usize) -> Self {
        Self {
            chunks: vec![Chunk::default(); n],
            ..default()
        }
    }

    fn roll(&mut self) -> f32 {
        if self.rng == 0 {
            self.rng = 0x2545_F491;
        }
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng >> 8) as f32 / (1 << 24) as f32
    }

    fn signed(&mut self) -> f32 {
        self.roll() * 2.0 - 1.0
    }

    fn claim(&mut self) -> Option<usize> {
        if self.chunks.is_empty() {
            return None;
        }
        if let Some(i) = self.chunks.iter().position(|c| c.left <= 0.0) {
            return Some(i);
        }
        self.stolen += 1;
        let i = self.next;
        self.next = (self.next + 1) % self.chunks.len();
        Some(i)
    }

    pub fn live(&self) -> usize {
        self.chunks.iter().filter(|c| c.left > 0.0).count()
    }

    /// Throw `n` chunks; `ground` says where the ground is under a point.
    pub fn throw(&mut self, n: usize, t: &Throw, ground: &dyn Fn(f32, f32) -> f32) {
        let away = t.away.normalize_or(Vec3::Y);
        for _ in 0..n {
            let Some(i) = self.claim() else { return };
            let scatter =
                Vec3::new(self.signed(), self.signed() * 0.5, self.signed()).normalize_or(Vec3::Y);
            let dir = (away * 0.55 + scatter * 0.45).normalize_or(away);
            let speed = t.speed.0 + (t.speed.1 - t.speed.0) * self.roll();
            let size = t.size.0 + (t.size.1 - t.size.0) * self.roll();
            let pos = t.at + scatter * t.spread * self.roll();
            let vel = dir * speed + Vec3::Y * t.up * (0.6 + 0.4 * self.roll());
            let spin = Vec3::new(self.signed(), self.signed(), self.signed()) * 9.0;
            let rot = Quat::from_euler(
                EulerRot::YXZ,
                self.signed() * std::f32::consts::PI,
                self.signed() * std::f32::consts::PI,
                self.signed() * std::f32::consts::PI,
            );
            let life = t.life * (0.75 + 0.5 * self.roll());
            // Where it will come down, near enough: the ground under the
            // point its first arc reaches the foot's height again.
            let g0 = ground(pos.x, pos.z);
            let fall = ((vel.y
                + (vel.y * vel.y + 2.0 * DEBRIS_GRAVITY * (pos.y - g0).max(0.0)).sqrt())
                / DEBRIS_GRAVITY)
                .max(0.0);
            let land = pos + vel * fall;
            let ground_y = ground(land.x, land.z);
            let c = &mut self.chunks[i];
            let dirty = c.mesh != *t.mesh || c.mat != *t.mat;
            if dirty {
                c.mesh = t.mesh.clone();
                c.mat = t.mat.clone();
            }
            c.dirty |= dirty;
            c.left = life;
            c.life = life;
            c.pos = pos;
            c.vel = vel;
            c.rot = rot;
            c.spin = spin;
            c.size = size;
            c.ground = ground_y;
            self.thrown += 1;
        }
    }

    /// Fly, bounce, settle and age every live chunk by `dt`.
    pub fn step(&mut self, dt: f32) {
        if dt <= 0.0 {
            return;
        }
        for c in &mut self.chunks {
            if c.left <= 0.0 {
                continue;
            }
            c.left -= dt;
            if c.left <= 0.0 {
                c.left = 0.0;
                continue;
            }
            // Half a mesh height times the chunk's scale is where its middle
            // rides above the ground it lies on.
            let rest = c.ground + MESH_HALF_H_M * c.size * 0.7;
            let resting = c.pos.y <= rest + 1e-3 && c.vel.y.abs() < 0.05;
            if resting {
                let k = (-8.0 * dt).exp();
                c.vel.x *= k;
                c.vel.z *= k;
                c.spin *= (-6.0 * dt).exp();
            } else {
                c.vel.y -= DEBRIS_GRAVITY * dt;
            }
            c.pos += c.vel * dt;
            if c.pos.y < rest && c.vel.y < 0.0 {
                c.pos.y = rest;
                c.vel.y = -c.vel.y * DEBRIS_BOUNCE;
                c.vel.x *= DEBRIS_SLIDE;
                c.vel.z *= DEBRIS_SLIDE;
                c.spin *= 0.6;
                if c.vel.y < 0.5 {
                    c.vel.y = 0.0;
                }
            }
            c.rot = Quat::from_euler(EulerRot::YXZ, c.spin.x * dt, c.spin.y * dt, c.spin.z * dt)
                * c.rot;
        }
    }

    /// Chunk `i` as drawn — position, rotation, scale — or `None` when free.
    /// It shrinks into the ground over the last third of its life.
    pub fn draw(&self, i: usize) -> Option<(Vec3, Quat, f32)> {
        let c = self.chunks.get(i)?;
        if c.left <= 0.0 {
            return None;
        }
        let k = (c.left / (c.life * 0.33)).min(1.0);
        Some((
            c.pos - Vec3::Y * (1.0 - k) * c.size * 0.4,
            c.rot,
            c.size * k,
        ))
    }

    fn clear(&mut self) {
        for c in &mut self.chunks {
            c.left = 0.0;
        }
        self.thrown = 0;
        self.stolen = 0;
    }
}

/// A chunk's entity; `0` is its index in the pool.
#[derive(Component)]
pub struct DebrisOf(usize);

/// Rubble at a worn node's foot: its node's cell key.
#[derive(Component)]
pub struct Rubble(pub u32);

/// Spawn the chunk pool, hidden.
pub fn setup(
    mut commands: Commands,
    mut debris: ResMut<Debris>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // A placeholder until the first throw hands each chunk the node's own
    // mesh and material.
    let mesh = meshes.add(Cuboid::new(1.0, 1.0, 1.0));
    let mat = materials.add(StandardMaterial::default());
    debris.chunks = vec![Chunk::default(); DEBRIS_POOL];
    debris.entities = (0..DEBRIS_POOL)
        .map(|i| {
            commands
                .spawn((
                    DebrisOf(i),
                    Mesh3d(mesh.clone()),
                    MeshMaterial3d(mat.clone()),
                    Transform::from_scale(Vec3::splat(0.01)),
                    Visibility::Hidden,
                ))
                .id()
        })
        .collect();
}

/// What [`fly`] writes on a chunk's entity.
type ChunkItem = (
    &'static DebrisOf,
    &'static mut Transform,
    &'static mut Visibility,
    &'static mut Mesh3d,
    &'static mut MeshMaterial3d<StandardMaterial>,
);

/// Advance the chunks and copy them onto their entities.
pub fn fly(time: Res<Time>, mut debris: ResMut<Debris>, mut q: Query<ChunkItem, Without<OreNode>>) {
    debris.step(time.delta_secs().min(0.1));
    for (DebrisOf(i), mut tf, mut vis, mut mesh, mut mat) in &mut q {
        match debris.draw(*i) {
            None => {
                if *vis != Visibility::Hidden {
                    *vis = Visibility::Hidden;
                }
            }
            Some((pos, rot, size)) => {
                tf.translation = pos;
                tf.rotation = rot;
                tf.scale = Vec3::splat(size);
                *vis = Visibility::Visible;
                let c = &mut debris.chunks[*i];
                if c.dirty {
                    c.dirty = false;
                    mesh.0 = c.mesh.clone();
                    mat.0 = c.mat.clone();
                }
            }
        }
    }
}

/// Forget every chunk on the way out of a world.
pub fn forget(mut debris: ResMut<Debris>, mut q: Query<&mut Visibility, With<DebrisOf>>) {
    debris.clear();
    for mut v in &mut q {
        *v = Visibility::Hidden;
    }
}

// ─── Stages, blows and breaks ────────────────────────────────────────────────

/// Where the rubble for stage `stage` (1-based) piece `j` lies, how big it is
/// (a share of the node mesh) and how it is turned.
pub fn rubble_spot(n: &OreNode, idx: usize) -> (Vec2, f32, Quat) {
    let h = |salt: u32| super::props::hash01(n.key, 0x0B0B_0000 + salt * 64 + idx as u32);
    let ang = h(0) * std::f32::consts::TAU;
    let r = n.radius() * (0.95 + 0.4 * h(1));
    let size = n.scale * (0.055 + 0.04 * h(2));
    let rot = Quat::from_euler(
        EulerRot::YXZ,
        h(3) * std::f32::consts::TAU,
        (h(4) - 0.5) * 1.6,
        (h(5) - 0.5) * 1.6,
    );
    (
        Vec2::new(n.foot.x + ang.cos() * r, n.foot.z + ang.sin() * r),
        size,
        rot,
    )
}

/// Bring a node's rubble in step with its stage: a pile for each stage it has
/// reached, none when whole or gone.
fn set_rubble(
    commands: &mut Commands,
    world: &WorldId,
    n: &mut OreNode,
    stage: u8,
    parent: Option<Entity>,
    mesh: &Handle<Mesh>,
    mat: &Handle<StandardMaterial>,
) {
    let want = RUBBLE_PER_STAGE * stage.min(ORE_STAGES - 1) as usize;
    for idx in 0..RUBBLE_MAX {
        match (idx < want, n.rubble[idx]) {
            (true, None) => {
                let (xz, size, rot) = rubble_spot(n, idx);
                let y = terrain::ground(world.seed, &world.haven, xz.x, xz.y);
                // Half buried: a third of its height under the grass.
                let at = Vec3::new(xz.x, y + MESH_HALF_H_M * size * 0.35, xz.y);
                let mut e = commands.spawn((
                    Rubble(n.key),
                    Mesh3d(mesh.clone()),
                    MeshMaterial3d(mat.clone()),
                    Transform {
                        translation: at,
                        rotation: rot,
                        scale: Vec3::splat(size),
                    },
                ));
                if let Some(p) = parent {
                    e.insert(ChildOf(p));
                }
                n.rubble[idx] = Some(e.id());
            }
            (false, Some(e)) => {
                commands.entity(e).try_despawn();
                n.rubble[idx] = None;
            }
            _ => {}
        }
    }
}

/// The ground under a point, for the chunks.
fn ground_of(world: &WorldId) -> impl Fn(f32, f32) -> f32 + '_ {
    |x, z| terrain::ground(world.seed, &world.haven, x, z)
}

/// What a blow on an ore node throws on top of the matter's own burst: the
/// ore's colours and a chunk of itself.
#[allow(clippy::too_many_arguments)]
fn blow(
    fx: &mut Fx,
    debris: &mut Debris,
    world: &WorldId,
    n: &OreNode,
    at: Vec3,
    away: Vec3,
    k: f32,
    mesh: &Handle<Mesh>,
    mat: &Handle<StandardMaterial>,
) {
    match n.kind {
        OreKind::Stone => {
            emit_tint(
                &mut fx.soft,
                Layer::Grit,
                scaled(5, k),
                at,
                away,
                n.kind.grit(),
            );
            emit_tint(
                &mut fx.soft,
                Layer::Dust,
                scaled(2, k),
                at,
                away,
                n.kind.dust(),
            );
        }
        OreKind::Metal => {
            emit(
                &mut fx.glow,
                Layer::Sparks,
                scaled(8, k),
                at,
                away,
                Matter::Metal,
            );
            emit_tint(
                &mut fx.soft,
                Layer::Grit,
                scaled(4, k),
                at,
                away,
                n.kind.grit(),
            );
            emit_tint(
                &mut fx.soft,
                Layer::Dust,
                scaled(2, k),
                at,
                away,
                n.kind.dust(),
            );
        }
        OreKind::Sulfur => {
            emit_tint(
                &mut fx.soft,
                Layer::Dust,
                scaled(4, k),
                at,
                away,
                n.kind.dust(),
            );
            emit_tint(
                &mut fx.soft,
                Layer::Grit,
                scaled(6, k),
                at,
                away,
                n.kind.grit(),
            );
        }
    }
    if k > 0.0 {
        let count = 1 + (debris.roll() < 0.5) as usize;
        debris.throw(
            count,
            &Throw {
                at,
                away,
                speed: (1.6, 3.2),
                up: 1.6,
                size: (0.020 * n.scale, 0.034 * n.scale),
                life: DEBRIS_LIFE_S * 0.6,
                spread: 0.03,
                mesh,
                mat,
            },
            &ground_of(world),
        );
    }
}

/// A node crumbled to a new stage: a burst off its top that covers the change.
#[allow(clippy::too_many_arguments)]
fn crumble(
    fx: &mut Fx,
    debris: &mut Debris,
    chips: &mut Chips,
    sound: &mut Sound,
    shake: Option<&mut Shake>,
    world: &WorldId,
    eye: Vec3,
    n: &OreNode,
    mesh: &Handle<Mesh>,
    mat: &Handle<StandardMaterial>,
) {
    let top = n.top();
    sound.play(Request::at(Cue::OreCrumble, top.to_array()));
    if let Some(shake) = shake {
        shake.add_at(CRUMBLE_TRAUMA, top, eye, 2.5, 14.0);
    }
    let k = lod(top.distance(eye));
    if k <= 0.0 {
        return;
    }
    let up = Vec3::Y;
    emit_tint(
        &mut fx.soft,
        Layer::Plume,
        scaled(3, k),
        top,
        up,
        n.kind.dust(),
    );
    emit_tint(
        &mut fx.soft,
        Layer::Dust,
        scaled(8, k),
        top,
        up,
        n.kind.dust(),
    );
    emit_tint(
        &mut fx.soft,
        Layer::Grit,
        scaled(16, k),
        top,
        up,
        n.kind.grit(),
    );
    if n.kind == OreKind::Metal {
        emit(
            &mut fx.glow,
            Layer::Sparks,
            scaled(12, k),
            top,
            up,
            Matter::Metal,
        );
    }
    chips.ignite_n(
        &Burst {
            at: top,
            away: up,
            matter: n.kind.matter(),
        },
        scaled(10, k),
    );
    debris.throw(
        CRUMBLE_CHUNKS,
        &Throw {
            at: top,
            away: up,
            speed: (1.4, 3.0),
            up: 2.0,
            size: (0.035 * n.scale, 0.065 * n.scale),
            life: DEBRIS_LIFE_S * 0.8,
            spread: n.radius() * 0.4,
            mesh,
            mat,
        },
        &ground_of(world),
    );
}

/// A node's last hit: the whole of it thrown apart.
#[allow(clippy::too_many_arguments)]
fn shatter(
    fx: &mut Fx,
    debris: &mut Debris,
    chips: &mut Chips,
    sound: &mut Sound,
    shake: Option<&mut Shake>,
    world: &WorldId,
    eye: Vec3,
    n: &OreNode,
    mesh: &Handle<Mesh>,
    mat: &Handle<StandardMaterial>,
) {
    let mid = n.middle();
    sound.play(Request::at(Cue::OreBreak, mid.to_array()));
    if let Some(shake) = shake {
        shake.add_at(BREAK_TRAUMA, mid, eye, 3.0, 24.0);
    }
    let k = lod(mid.distance(eye));
    if k <= 0.0 {
        return;
    }
    let up = Vec3::Y;
    let r = n.radius();
    let dust = n.kind.dust();
    emit_tint(&mut fx.soft, Layer::Plume, scaled(4, k), mid, up, dust);
    emit_tint(&mut fx.soft, Layer::Dust, scaled(10, k), mid, up, dust);
    // A skirt of cloud round the foot, rolling out along the ground where the
    // rock met it: a column alone read as smoke, not as a rock coming down.
    for i in 0..6 {
        let a = i as f32 / 6.0 * std::f32::consts::TAU;
        let out = Vec3::new(a.cos(), 0.25, a.sin());
        let at = Vec3::new(
            n.foot.x + a.cos() * r * 0.8,
            mid.y - 0.15,
            n.foot.z + a.sin() * r * 0.8,
        );
        emit_tint(&mut fx.soft, Layer::Plume, scaled(1, k), at, out, dust);
        emit_tint(&mut fx.soft, Layer::Dust, scaled(2, k), at, out, dust);
    }
    emit_tint(
        &mut fx.soft,
        Layer::Grit,
        scaled(28, k),
        mid,
        up,
        n.kind.grit(),
    );
    if n.kind == OreKind::Metal {
        emit(
            &mut fx.glow,
            Layer::Sparks,
            scaled(20, k),
            mid,
            up,
            Matter::Metal,
        );
        emit(&mut fx.glow, Layer::Flash, 1, mid, up, Matter::Metal);
    }
    chips.ignite_n(
        &Burst {
            at: mid,
            away: up,
            matter: n.kind.matter(),
        },
        scaled(16, k),
    );
    debris.throw(
        BREAK_CHUNKS,
        &Throw {
            at: mid,
            away: up,
            speed: (2.0, 4.5),
            up: 2.4,
            size: (0.06 * n.scale, 0.12 * n.scale),
            life: DEBRIS_LIFE_S,
            spread: r * 0.6,
            mesh,
            mat,
        },
        &ground_of(world),
    );
}

/// What [`react`] reads and writes on an ore node's entity.
type NodeItem = (
    Entity,
    &'static mut OreNode,
    &'static Mesh3d,
    &'static MeshMaterial3d<StandardMaterial>,
    Option<&'static ChildOf>,
);

/// The frame's crumbles, breaks and blows on ore nodes, and every node's stage
/// kept in step with the wire.
///
/// After `impact::snap`, so a blow's contact already names the node it landed
/// on, and after the drain for the crumbles.
#[allow(clippy::too_many_arguments)]
pub fn react(
    mut commands: Commands,
    feed: Res<Feed>,
    contacts: Res<Contacts>,
    net: Option<NonSend<Net>>,
    world: Option<Res<WorldId>>,
    eye: Res<Eye>,
    mut fx: ResMut<Fx>,
    mut chips: ResMut<Chips>,
    mut debris: ResMut<Debris>,
    mut sound: ResMut<Sound>,
    mut shake: Option<ResMut<Shake>>,
    mut marks: ResMut<Marks>,
    mut nodes: Query<NodeItem>,
) {
    let (Some(net), Some(world)) = (net, world) else {
        return;
    };
    let core = &net.session.core;

    // ── Crumbles and breaks, live ────────────────────────────────────────
    for &(key, stage) in feed.crumbles() {
        let Some((_, mut n, mesh, mat, parent)) = nodes.iter_mut().find(|q| q.1.key == key) else {
            continue;
        };
        let parent = parent.map(|p| p.parent());
        if stage >= ORE_STAGES {
            shatter(
                &mut fx,
                &mut debris,
                &mut chips,
                &mut sound,
                shake.as_deref_mut(),
                &world,
                eye.pos,
                &n,
                &mesh.0,
                &mat.0,
            );
            set_rubble(&mut commands, &world, &mut n, 0, parent, &mesh.0, &mat.0);
            n.target = 0;
            n.shown = 0.0;
            n.posed = false;
            continue;
        }
        if stage <= n.target {
            continue;
        }
        crumble(
            &mut fx,
            &mut debris,
            &mut chips,
            &mut sound,
            shake.as_deref_mut(),
            &world,
            eye.pos,
            &n,
            &mesh.0,
            &mat.0,
        );
        // The face it had is gone, and so are the marks on it.
        marks.forget_later(n.middle(), n.radius() + 0.3);
        n.target = stage;
        n.since_crack = 0.0;
        set_rubble(
            &mut commands,
            &world,
            &mut n,
            stage,
            parent,
            &mesh.0,
            &mat.0,
        );
    }

    // ── Every node's stage, off the mirror ───────────────────────────────
    //
    // A join's walk, a respawn, a node streamed in already worn: drawn as it
    // is, at once, with nothing thrown.
    let moved = feed.applied & APPLIED_SLOTS != 0;
    for (_, mut n, mesh, mat, parent) in &mut nodes {
        if !moved && n.posed {
            continue;
        }
        let stage = if core.harvested.contains(n.key) {
            0
        } else {
            core.harvested.stage(n.key).min(ORE_STAGES - 1)
        };
        if stage != n.target || !n.posed {
            n.target = stage;
            n.shown = stage as f32;
            n.posed = false;
            let parent = parent.map(|p| p.parent());
            set_rubble(
                &mut commands,
                &world,
                &mut n,
                stage,
                parent,
                &mesh.0,
                &mat.0,
            );
        }
    }

    // ── Blows ────────────────────────────────────────────────────────────
    for c in contacts.iter() {
        if c.surf != SURF_WORLD || !matches!(c.matter, Matter::Stone | Matter::Metal) {
            continue;
        }
        let hit = match c.skin {
            Some(e) => nodes.get_mut(e).ok(),
            None => nodes.iter_mut().find(|q| {
                let d = Vec2::new(c.at.x - q.1.foot.x, c.at.z - q.1.foot.z).length();
                d <= q.1.radius() + 0.3
            }),
        };
        let Some((_, mut n, mesh, mat, _)) = hit else {
            continue;
        };
        if core.harvested.contains(n.key) {
            continue;
        }
        n.since_hit = 0.0;
        let push = (c.at - n.foot).with_y(0.0).normalize_or(Vec3::X);
        n.hit_axis = Vec3::Y.cross(push).normalize_or(Vec3::X);
        if c.weapon == Weapon::Melee {
            let k = lod(c.at.distance(eye.pos));
            blow(
                &mut fx,
                &mut debris,
                &world,
                &n,
                c.at,
                c.away,
                k,
                &mesh.0,
                &mat.0,
            );
        }
    }
}

/// Draw every node that is moving: easing onto a stage, ringing from a
/// crumble, shuddering from a blow — or not yet posed at all. The mesh is the
/// stage's cut, swapped on the crumble, under the burst that covers it.
pub fn pose(
    time: Res<Time>,
    cuts: Res<OreCuts>,
    mut nodes: Query<(&mut OreNode, &mut Transform, &mut Mesh3d)>,
) {
    let dt = time.delta_secs().min(0.1);
    // The cuts arriving re-draws every worn node with them.
    let fresh = cuts.is_changed();
    for (mut n, mut tf, mut mesh) in &mut nodes {
        if !n.animating() && !fresh {
            continue;
        }
        let target = n.target as f32;
        if n.shown < target {
            n.shown = (n.shown + CRACK_RATE * dt).min(target);
        } else if n.shown > target {
            n.shown = (n.shown - CRACK_RATE * dt).max(target);
        }
        n.since_hit += dt;
        n.since_crack += dt;
        let want = cuts.mesh(&n, n.target);
        let cut = want.is_some();
        let want = want.unwrap_or(&n.base);
        if mesh.0 != *want {
            mesh.0 = want.clone();
        }
        let (rock, squash) = shudder(n.since_hit, n.hit_axis);
        let pop = crack_pop(n.since_crack);
        // A cut node is drawn at its stage outright; a squashed one eases.
        let p = stage_pose(n.kind, n.key, if cut { target } else { n.shown });
        *tf = pose_transform(&n, &p, rock, squash, pop, cut);
        n.posed = true;
    }
}

// ─── The hotspot ─────────────────────────────────────────────────────────────

/// Where the hotspot burns and which way it faces, and what it is on.
#[derive(Default)]
pub struct Hot {
    cell: u32,
    mark8: u8,
    stage: f32,
    settle: u8,
    at: Option<(Vec3, Vec3)>,
    glow_due: f32,
    star_due: f32,
}

/// The shimmer on the ore node the sim marked for this player, and the glint
/// when a blow lands on it. A tree's mark is still `decal::weak_spot`'s X.
#[allow(clippy::too_many_arguments)]
pub fn hotspot(
    mut hot: Local<Hot>,
    net: Option<NonSend<Net>>,
    world: Option<Res<WorldId>>,
    feed: Res<Feed>,
    in_weak: Res<super::verbs::InWeak>,
    eye: Res<Eye>,
    time: Res<Time>,
    mut fx: ResMut<Fx>,
    mut sound: ResMut<Sound>,
    mut shake: Option<ResMut<Shake>>,
    meshes: Res<Assets<Mesh>>,
    nodes: Query<(&OreNode, &Mesh3d, &GlobalTransform)>,
) {
    let (Some(net), Some(world)) = (net, world) else {
        return;
    };
    let core = &net.session.core;

    // A blow on the hotspot: the glint where it was, before it moves.
    if feed.applied & APPLIED_MARK != 0 && core.mark_weak_hit && hot.cell == core.mark_cell {
        if let Some((at, n)) = hot.at {
            glint(&mut fx, at, n, eye.pos);
            sound.play(Request::at(Cue::OreGlint, at.to_array()));
            if let Some(shake) = shake.as_deref_mut() {
                shake.kick(Vec3::new(0.004, 0.0, 0.0));
            }
        }
    }

    if core.mark_cell == NO_CELL {
        hot.cell = NO_CELL;
        hot.at = None;
        return;
    }
    let (cx, cz) = (
        (core.mark_cell >> 16) as i32,
        (core.mark_cell & 0xFFFF) as i32,
    );
    let node = nodes.iter().find(|q| q.0.key == core.mark_cell);
    let stage = node.map_or(0.0, |q| q.0.shown());
    if hot.cell != core.mark_cell || hot.mark8 != core.mark8 || hot.stage != stage {
        hot.cell = core.mark_cell;
        hot.mark8 = core.mark8;
        hot.stage = stage;
        // Solved again for a few frames: a crumble swaps the mesh and the
        // pose this frame, and the drawn transform follows a frame later.
        hot.settle = 3;
    }
    if hot.settle > 0 {
        hot.settle -= 1;
        let slot = terrain::scatter(world.seed, &world.table, &world.haven, cx, cz);
        hot.at = if OreKind::of(slot.occupant).is_some() {
            super::decal::weak_spot_pose(&slot, core.mark8).map(|(at, n)| {
                let mesh = node.and_then(|q| meshes.get(&q.1 .0).map(|m| (m, q.2)));
                hot_on(&slot, stage, at, n, mesh).unwrap_or((at, n))
            })
        } else {
            None
        };
    }
    let Some((at, n)) = hot.at else { return };
    if core.harvested.contains(core.mark_cell) || lod(at.distance(eye.pos)) <= 0.0 {
        return;
    }
    let bright = if in_weak.0 { 1.6 } else { 1.0 };
    let dt = time.delta_secs().min(0.1);
    let t = time.elapsed_secs();
    hot.glow_due += dt * SHIMMER_GLOW_HZ;
    hot.star_due += dt * SHIMMER_STAR_HZ * bright;
    let centre = at + n * SHIMMER_LIFT_M;
    // A slow breath under the flicker, so it reads as alive from across a
    // clearing.
    let breath = 0.75 + 0.25 * (t * 2.6).sin();
    while hot.glow_due >= 1.0 {
        hot.glow_due -= 1.0;
        let jitter = Vec3::new(fx.glow.signed(), fx.glow.signed(), fx.glow.signed()) * 0.012;
        let k = bright * breath * (0.8 + 0.4 * fx.glow.roll());
        let size = 0.09 + 0.04 * fx.glow.roll();
        spark(
            &mut fx.glow,
            Particle {
                left: 0.14,
                life: 0.14,
                pos: centre + jitter,
                vel: n * 0.05,
                size0: size,
                size1: size * 1.3,
                c0: [6.0 * k, 2.6 * k, 0.5 * k, 1.0],
                c1: [2.0 * k, 0.6 * k, 0.05 * k, 0.0],
                cell: GLOW,
                ..default()
            },
        );
        // A stain of orange under the light, matter rather than glow, so the
        // spot reads on pale granite in full sun, where light added to a white
        // rock is only more white.
        let size = 0.07 + 0.02 * fx.soft.roll();
        let roll = fx.soft.roll() * std::f32::consts::TAU;
        spark(
            &mut fx.soft,
            Particle {
                left: 0.14,
                life: 0.14,
                pos: at + n * 0.02,
                size0: size,
                size1: size,
                c0: [0.95, 0.42, 0.06, 0.55 * breath],
                c1: [0.95, 0.42, 0.06, 0.0],
                roll,
                cell: PUFF,
                ..default()
            },
        );
    }
    while hot.star_due >= 1.0 {
        hot.star_due -= 1.0;
        let t1 = Vec3::Y.cross(n).normalize_or(Vec3::X);
        let t2 = n.cross(t1);
        let (a, b) = (fx.glow.signed(), fx.glow.signed());
        let pos = centre + (t1 * a + t2 * b) * SHIMMER_R_M;
        let size = 0.012 + 0.018 * fx.glow.roll();
        let roll = fx.glow.roll() * std::f32::consts::TAU;
        let cell = STAR + ((fx.glow.roll() * 3.0) as u8).min(2);
        spark(
            &mut fx.glow,
            Particle {
                left: 0.28,
                life: 0.28,
                pos,
                vel: n * 0.12 + Vec3::Y * 0.08,
                size0: size,
                size1: size * 1.8,
                c0: [9.0 * bright, 7.0 * bright, 3.5 * bright, 1.0],
                c1: [2.0, 0.9, 0.2, 0.0],
                roll,
                spin: 2.0,
                cell,
                ..default()
            },
        );
    }
}

/// The hotspot on the drawn rock: a ray in from the sector's side, level, at
/// heights inside what is left of the node (the strike height of a whole one
/// floats over a stage-3 lump), onto its own mesh. `None` when the mesh is
/// not in or nothing is struck.
fn hot_on(
    slot: &terrain::Slot,
    stage: f32,
    at: Vec3,
    n: Vec3,
    mesh: Option<(&Mesh, &GlobalTransform)>,
) -> Option<(Vec3, Vec3)> {
    let (mesh, tf) = mesh?;
    let (r, top) = terrain::occupant_volume(slot.occupant);
    let i = (stage.round() as usize).min(ORE_STAGES as usize - 1);
    let left = top * slot.scale * ORE_STAGE_TOP_PM[i] as f32 * 0.001;
    let out = r * slot.scale + 0.5;
    [0.55, 0.4, 0.25].iter().find_map(|k| {
        let y = (slot.y + left * k).min(at.y);
        let o = Vec3::new(slot.x, y, slot.z) + n * out;
        super::skin::ray_mesh(mesh, tf, o, -n, out).map(|h| (h.at, h.normal))
    })
}

fn spark(pool: &mut Pool, p: Particle) {
    pool.spawn(Particle {
        orient: Orient::Billboard,
        ..p
    });
}

/// The answer to a blow on the hotspot: a flash, a fan of twinkles and a
/// shower of gold sparks off the face.
fn glint(fx: &mut Fx, at: Vec3, n: Vec3, eye: Vec3) {
    let k = lod(at.distance(eye));
    if k <= 0.0 {
        return;
    }
    let centre = at + n * SHIMMER_LIFT_M;
    emit(&mut fx.glow, Layer::Flash, 1, centre, n, Matter::Metal);
    emit(
        &mut fx.glow,
        Layer::Sparks,
        scaled(14, k),
        centre,
        n,
        Matter::Metal,
    );
    for _ in 0..scaled(12, k) {
        let scatter =
            Vec3::new(fx.glow.signed(), fx.glow.signed(), fx.glow.signed()).normalize_or(n);
        let dir = (n * 0.6 + scatter * 0.4).normalize_or(n);
        let speed = 1.2 + 2.0 * fx.glow.roll();
        let size = 0.02 + 0.02 * fx.glow.roll();
        let cell = STAR + ((fx.glow.roll() * 3.0) as u8).min(2);
        let roll = fx.glow.roll() * std::f32::consts::TAU;
        spark(
            &mut fx.glow,
            Particle {
                left: 0.45,
                life: 0.45,
                pos: centre,
                vel: dir * speed,
                size0: size,
                size1: size * 0.4,
                c0: [12.0, 9.0, 4.0, 1.0],
                c1: [3.0, 1.2, 0.2, 0.0],
                gravity: 3.0,
                drag: 2.5,
                roll,
                spin: 6.0,
                cell,
                ..default()
            },
        );
    }
}
