//! The fel beasts: the pig and the wolf as skinned, animated models, in three
//! corruption tiers that climb with the wipe (`ARC.md` §2).
//!
//! `mobs.rs` still owns the animal — its id, its interpolated position, its
//! [`Gait`] (speed, bite, howl, lying) — and still draws it as the rigid
//! body-and-four-legs kit whenever this file has nothing to give it: on a
//! headless build, for the stag, and for the frames before a tier's model has
//! loaded. What this file swaps is what the animal WEARS: a skinned model
//! from `ci/rig_beast.py` and the seven clips that script authors.
//!
//! **Bevy plays; it does not decide** (`RENDER.md` §1, `anim.rs`'s rule one
//! surface over). The clip is a pure function of state the sim already sent:
//! the derived speed, the bite and howl events, the sleeping bit. Deleting
//! every clip here changes how an animal looks and nothing else.
//!
//! **The tier is read off the arc the wire already carries**
//! ([`ArcView`], v95), so the corruption needs no field of its own: A on wipe
//! day, B once the first act-II work opens, C once every act-II work has
//! burned (or the shard reaches act III). Every work has a fallback hour, so
//! a shard nobody feeds still reaches C — by hour 192 with today's rows.
//! An animal keeps the look it was spawned with; the next one out of the
//! fog wears the island's current tier.

use std::time::Duration;

use bevy::gltf::Gltf;
use bevy::prelude::*;
use client_core::arc::ArcView;
use sim_core::mob;
use sim_core::works::WORK_LIT;

use super::mobs::{Animal, Gait};
use super::Net;

/// Corruption tiers: light, medium, heavy.
pub const TIERS: usize = 3;

/// The two species that wear fel models, in this file's own order.
pub const FEL_PIG: usize = 0;
pub const FEL_WOLF: usize = 1;

/// `[species][tier]`, what `ci/rig_beast.py` then `ci/ktx_pack.py` wrote.
pub const BEAST_GLB: [[&str; TIERS]; 2] = [
    [
        "models/mob/fel/pig_a.glb",
        "models/mob/fel/pig_b.glb",
        "models/mob/fel/pig_c.glb",
    ],
    [
        "models/mob/fel/wolf_a.glb",
        "models/mob/fel/wolf_b.glb",
        "models/mob/fel/wolf_c.glb",
    ],
];

/// The same animals killed: static, lying on the right side in
/// `mobs::CARCASS_GLB`'s frame, drawn by `structures.rs` for a dead animal's
/// bag.
pub const BEAST_CARCASS_GLB: [[&str; TIERS]; 2] = [
    [
        "models/mob/fel/pig_a_carcass.glb",
        "models/mob/fel/pig_b_carcass.glb",
        "models/mob/fel/pig_c_carcass.glb",
    ],
    [
        "models/mob/fel/wolf_a_carcass.glb",
        "models/mob/fel/wolf_b_carcass.glb",
        "models/mob/fel/wolf_c_carcass.glb",
    ],
];

/// The fel species a roster slot draws as, if any — `mob::kind_of`, the
/// same pure function `HerdAssets::of` reads.
pub fn fel_species(slot: usize) -> Option<usize> {
    fel_species_of_kind(mob::kind_of(slot))
}

/// The fel species a sim species code draws as (`WireBag::species`).
pub fn fel_species_of_kind(kind: u8) -> Option<usize> {
    match kind {
        mob::MOB_PIG => Some(FEL_PIG),
        mob::MOB_WOLF => Some(FEL_WOLF),
        _ => None,
    }
}

/// The island's corruption tier, off the arc as the client knows it.
pub fn tier_of(arc: &ArcView) -> usize {
    let act = arc.act();
    if act >= 3 {
        return 2;
    }
    if act <= 1 {
        return 0;
    }
    let mut any = false;
    let mut all = true;
    for (_, w) in arc.known() {
        if w.def.act == 2 {
            any = true;
            all &= w.state == WORK_LIT;
        }
    }
    if any && all {
        2
    } else {
        1
    }
}

/// The clips `ci/rig_beast.py` authors, resolved by NAME (`anim.rs` says why
/// an index is a trap).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BeastClip {
    Idle,
    Walk,
    Trot,
    Run,
    Bite,
    Howl,
    Lie,
}

impl BeastClip {
    pub const ALL: [BeastClip; 7] = [
        BeastClip::Idle,
        BeastClip::Walk,
        BeastClip::Trot,
        BeastClip::Run,
        BeastClip::Bite,
        BeastClip::Howl,
        BeastClip::Lie,
    ];

    pub fn name(self) -> &'static str {
        match self {
            BeastClip::Idle => "Idle",
            BeastClip::Walk => "Walk",
            BeastClip::Trot => "Trot",
            BeastClip::Run => "Run",
            BeastClip::Bite => "Bite",
            BeastClip::Howl => "Howl",
            BeastClip::Lie => "Lie",
        }
    }

    fn slot(self) -> usize {
        self as usize
    }

    /// The loops; the other three play once and hold their last frame (the
    /// lying pose stays down until the animal wakes).
    pub fn looped(self) -> bool {
        matches!(
            self,
            BeastClip::Idle | BeastClip::Walk | BeastClip::Trot | BeastClip::Run
        )
    }

    /// How long a cross-fade INTO this clip takes, seconds.
    fn blend(self, from: Option<BeastClip>) -> f32 {
        match (from, self) {
            (Some(BeastClip::Lie), _) => 0.6,
            (_, BeastClip::Bite) => 0.08,
            (_, BeastClip::Lie) => 0.3,
            _ => 0.25,
        }
    }
}

/// Each gait clip's native ground speed, m/s — the speed at which a paw in
/// stance does not slide, `[Walk, Trot, Run]`. **These are
/// `ci/rig_beast.py`'s `SPECIES[..]["gaits"]` numbers and must move with
/// them**; `tests/beasts.rs` reads the script and holds the two together.
pub const CLIP_MPS: [[f32; 3]; 2] = [[1.1, 2.3, 4.0], [1.3, 2.7, 5.0]];

/// Below this the animal stands, m/s.
pub const BEAST_IDLE_MPS: f32 = 0.25;
/// How far past a gait boundary the speed must go before the clip changes,
/// m/s — a speed riding the boundary otherwise flickers between two clips.
pub const BEAST_HYSTERESIS_MPS: f32 = 0.2;
/// The playback rate range a gait clip may be stretched over to plant its
/// feet at the animal's actual speed.
pub const BEAST_RATE_MIN: f32 = 0.55;
pub const BEAST_RATE_MAX: f32 = 1.8;

/// How bright the fel cracks burn, linear multiplier on the model's emissive
/// map (the cracks' own albedo). Between the town's lapis seams by night
/// (1.6) and its bulbs (6.0). **(knob)**
pub const FEL_GLOW: f32 = 3.0;

/// The gait for a speed, with hysteresis against the clip already playing.
pub fn gait_for(speed: f32, mps: [f32; 3], current: Option<BeastClip>) -> BeastClip {
    let bounds = [
        BEAST_IDLE_MPS,
        0.5 * (mps[0] + mps[1]),
        0.5 * (mps[1] + mps[2]),
    ];
    let order = [
        BeastClip::Idle,
        BeastClip::Walk,
        BeastClip::Trot,
        BeastClip::Run,
    ];
    let raw = order[bounds.iter().filter(|b| speed >= **b).count()];
    let Some(cur) = current.filter(|c| c.looped()) else {
        return raw;
    };
    let ci = order.iter().position(|c| *c == cur).unwrap_or(0);
    let ri = order.iter().position(|c| *c == raw).unwrap_or(0);
    // Hold the current gait until the speed is clear of the boundary
    // between it and the raw pick.
    let edge = if ri > ci {
        bounds[ci]
    } else if ri < ci {
        bounds[ri]
    } else {
        return cur;
    };
    if (speed - edge).abs() < BEAST_HYSTERESIS_MPS && ri.abs_diff(ci) == 1 {
        cur
    } else {
        raw
    }
}

/// What a beast should be playing this frame. A lying animal lies; a bite
/// or a howl in progress plays over any gait; otherwise its speed picks.
pub fn wants(gait: &Gait, mps: [f32; 3], current: Option<BeastClip>) -> BeastClip {
    if gait.lie > 0.5 {
        return BeastClip::Lie;
    }
    if gait.bite > 0.0 {
        return BeastClip::Bite;
    }
    if gait.howl > 0.0 {
        return BeastClip::Howl;
    }
    gait_for(gait.speed, mps, current)
}

/// The playback rate of a clip at this speed.
pub fn rate_of(clip: BeastClip, speed: f32, mps: [f32; 3]) -> f32 {
    let native = match clip {
        BeastClip::Walk => mps[0],
        BeastClip::Trot => mps[1],
        BeastClip::Run => mps[2],
        _ => return 1.0,
    };
    (speed / native).clamp(BEAST_RATE_MIN, BEAST_RATE_MAX)
}

/// One species-and-tier model: requested lazily, built once.
#[derive(Default)]
struct BeastRig {
    gltf: Option<Handle<Gltf>>,
    scene: Option<Handle<Scene>>,
    graph: Option<Handle<AnimationGraph>>,
    nodes: [AnimationNodeIndex; 7],
    durations: [f32; 7],
    has: [bool; 7],
}

impl BeastRig {
    fn ready(&self) -> bool {
        self.scene.is_some() && self.graph.is_some()
    }
}

/// Every fel model this client has asked for.
#[derive(Resource, Default)]
pub struct Beasts {
    rigs: [[BeastRig; TIERS]; 2],
    /// Names the files did not have, reported once per model.
    reported: [[bool; TIERS]; 2],
}

impl Beasts {
    /// The scene to spawn for this slot at this tier, once it is built.
    pub fn scene_for(&self, slot: usize, tier: usize) -> Option<(usize, Handle<Scene>)> {
        let sp = fel_species(slot)?;
        let rig = &self.rigs[sp][tier.min(TIERS - 1)];
        rig.scene.clone().filter(|_| rig.ready()).map(|s| (sp, s))
    }
}

/// An animal wearing a fel model: which one, so its player finds its graph.
#[derive(Component, Clone, Copy)]
pub struct Skinned {
    pub species: usize,
    pub tier: usize,
}

/// Request the island's current tier, then build each model's graph once its
/// glTF and clips are in. Lazy so a client never downloads a tier the shard
/// has not reached: on the web every model is a fetch.
pub fn build(
    mut beasts: ResMut<Beasts>,
    assets: Res<AssetServer>,
    net: NonSend<Net>,
    gltfs: Res<Assets<Gltf>>,
    clips: Res<Assets<AnimationClip>>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let tier = tier_of(&net.session.core.arc);
    let Beasts { rigs, reported } = &mut *beasts;
    for sp in [FEL_PIG, FEL_WOLF] {
        let rig = &mut rigs[sp][tier];
        if rig.gltf.is_none() {
            rig.gltf = Some(assets.load(BEAST_GLB[sp][tier]));
        }
    }
    for sp in [FEL_PIG, FEL_WOLF] {
        for (t, rig) in rigs[sp].iter_mut().enumerate() {
            if rig.ready() {
                continue;
            }
            let Some(gltf) = rig.gltf.as_ref().and_then(|h| gltfs.get(h)) else {
                continue;
            };
            // Clips are sub-assets and land after the `Gltf`: wait for all.
            let mut durations = [1.0; 7];
            let mut waiting = false;
            for c in BeastClip::ALL {
                if let Some(h) = gltf.named_animations.get(c.name()) {
                    match clips.get(h) {
                        Some(clip) => durations[c.slot()] = clip.duration(),
                        None => waiting = true,
                    }
                }
            }
            if waiting {
                continue;
            }
            let mut graph = AnimationGraph::new();
            let root = graph.root;
            let mut missing = Vec::new();
            for c in BeastClip::ALL {
                match gltf.named_animations.get(c.name()) {
                    Some(h) => {
                        rig.nodes[c.slot()] = graph.add_clip(h.clone(), 1.0, root);
                        rig.has[c.slot()] = true;
                    }
                    None => missing.push(c.name()),
                }
            }
            if !missing.is_empty() && !reported[sp][t] {
                error!(
                    "beasts: {} lacks clip(s) {:?} — re-run ci/rig_beast.py; the \
                     animal falls back to Idle for those",
                    BEAST_GLB[sp][t], missing
                );
                reported[sp][t] = true;
            }
            // The cracks burn: the model's emissive map is the cracks' own
            // albedo, so the multiplier is the whole of the glow.
            for m in &gltf.materials {
                if let Some(mat) = materials.get_mut(m) {
                    mat.emissive = LinearRgba::rgb(FEL_GLOW, FEL_GLOW, FEL_GLOW);
                }
            }
            rig.durations = durations;
            rig.graph = Some(graphs.add(graph));
            rig.scene = gltf
                .default_scene
                .clone()
                .or_else(|| gltf.scenes.first().cloned());
        }
    }
}

/// What a beast's player is playing, so [`drive`] writes on a change and not
/// every frame (assigning every frame restarts the clip: `anim::Playing`).
#[derive(Component)]
pub struct BeastPlayer {
    root: Entity,
    clip: Option<BeastClip>,
    last_bite: f32,
    last_howl: f32,
}

/// Attach the right graph to every `AnimationPlayer` a beast's scene spawns.
pub fn bind(
    mut commands: Commands,
    beasts: Res<Beasts>,
    parents: Query<&ChildOf>,
    skinned: Query<&Skinned>,
    added: Query<Entity, Added<AnimationPlayer>>,
) {
    for player in &added {
        // Up to the animal root, bounded (`anim::bind`'s walk).
        let mut at = player;
        let mut found = None;
        for _ in 0..16 {
            if let Ok(s) = skinned.get(at) {
                found = Some((at, *s));
                break;
            }
            match parents.get(at) {
                Ok(p) => at = p.0,
                Err(_) => break,
            }
        }
        let Some((root, s)) = found else { continue };
        let Some(graph) = beasts.rigs[s.species][s.tier].graph.clone() else {
            continue;
        };
        commands.entity(player).insert((
            AnimationGraphHandle(graph),
            AnimationTransitions::new(),
            BeastPlayer {
                root,
                clip: None,
                last_bite: 0.0,
                last_howl: 0.0,
            },
        ));
    }
}

/// A stable phase in `[0, 1)` per animal, so a herd standing still does not
/// breathe in unison (`anim::desync`).
fn desync(e: Entity) -> f32 {
    let h = e.to_bits().wrapping_mul(0x9E37_79B9_7F4A_7C15);
    (h >> 40) as f32 / (1u64 << 24) as f32
}

/// Cross-fade each beast to what its gait wants and play its stride at the
/// rate that plants its feet.
pub fn drive(
    beasts: Res<Beasts>,
    animals: Query<(&Gait, &Skinned), With<Animal>>,
    mut players: Query<(
        &mut BeastPlayer,
        &mut AnimationPlayer,
        &mut AnimationTransitions,
    )>,
) {
    for (mut bp, mut player, mut transitions) in &mut players {
        let Ok((gait, s)) = animals.get(bp.root) else {
            continue;
        };
        let rig = &beasts.rigs[s.species][s.tier];
        if !rig.ready() {
            continue;
        }
        let mps = CLIP_MPS[s.species];
        let mut want = wants(gait, mps, bp.clip);
        if !rig.has[want.slot()] {
            want = BeastClip::Idle;
        }
        // A new bite or howl restarts its clip even if one is still playing:
        // the gait's clock jumps back up when the sim says it happened again.
        let rebite = gait.bite > bp.last_bite + 1e-4;
        let rehowl = gait.howl > bp.last_howl + 1e-4;
        bp.last_bite = gait.bite;
        bp.last_howl = gait.howl;
        let restart = (want == BeastClip::Bite && rebite) || (want == BeastClip::Howl && rehowl);
        if bp.clip != Some(want) || restart {
            let from = bp.clip;
            let active = transitions.play(
                &mut player,
                rig.nodes[want.slot()],
                Duration::from_secs_f32(want.blend(from)),
            );
            if want.looped() {
                active.repeat();
                if from.is_none() {
                    active.set_seek_time(desync(bp.root) * rig.durations[want.slot()]);
                }
            }
            bp.clip = Some(want);
        }
        let rate = rate_of(want, gait.speed, mps);
        if let Some(a) = player.animation_mut(rig.nodes[want.slot()]) {
            a.set_speed(rate);
        }
    }
}
