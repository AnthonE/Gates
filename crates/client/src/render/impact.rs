//! What a landed blow throws into the air.
//!
//! Until this landed, **connecting with something looked exactly like missing
//! it**: the sound played, the number moved, and the frame was unchanged. The
//! operator reported it as *"we need some kinda effect particle wise when u
//! actually connect with something"* (2026-08-30), and the gap is wider than
//! it sounds — a melee game reads its own feedback off the frame, and `decal`
//! (arrows) plus `tracer` (arrows) were the only two things in this tree that
//! drew a consequence anywhere but the HUD.
//!
//! # It decides nothing
//!
//! `RENDER.md` §1, and this is `decal.rs`'s posture one verb over. Every burst
//! is fired from a fact the SIM already announced — a gather payout, an
//! `EV_HIT`, an `EV_STRUCT_HIT`, an arrow's `EV_IMPACT` — never from the
//! button and never from the client's own reading of what is in reach. A
//! whiff throws nothing because the sim announced nothing, which is the
//! correct picture and is also why this file has no reach test in it.
//!
//! **One seam is honest to state.** Three of the four facts say what was hit
//! and not *where*: `EV_GATHER` carries an item, `EV_HIT` a victim id,
//! `EV_STRUCT_HIT` a build address. So the point a burst comes from is
//! recovered from something the client already holds and already trusts for
//! the same question — the swing pick (`ui::interact::resolve_swing`, the
//! client's mirror of the sim's own scan, which the prompt has drawn off
//! since it existed), the drawn body's transform, and the build grid. None
//! of that is a second opinion about whether the blow landed; it is only
//! about where to draw the answer.
//!
//! # A fixed pool, never a per-frame spawn
//!
//! [`CHIP_POOL`] entities are spawned once and hidden; a burst claims
//! [`CHIP_BURST`] of them and the fade releases them. `decal.rs` and `tracer.rs`
//! make the same call for the same reason (`CLAUDE.md`: no per-frame
//! allocations on the client, and a Bevy spawn is a structural archetype
//! move on top of that).
//!
//! **Overflow is drop-OLDEST**, `decal`'s policy rather than `tracer`'s. A
//! tracer that vanishes mid-flight reads as a bug; a chip has half a second
//! of life and the oldest is the faintest, so recycling it is invisible while
//! refusing the newest would drop the burst from the blow the player just
//! landed — the one they are watching for.

use bevy::prelude::*;

use super::feed::Feed;
use super::{Eye, Net, WorldId};
use sim_core::build::{BUILD_CELL_M, LEVEL_H_M};
use sim_core::movement::{POS_XZ_Q, POS_Y_Q};
use sim_core::ranged::{SURF_BUILT, SURF_GROUND, SURF_WORLD};
use sim_core::terrain::Occupant;

/// Chips drawable at once. A *view* cap, not a world one — `decal::MARKS`'s
/// split, and wall 4's bound on a client-driven path: a fight with eight
/// people in it cannot make this grow.
///
/// [`CHIP_BURST`] per blow, so this is twelve simultaneous blows' worth. A melee
/// exchange lands one blow per player per 1.267 s and a chip lives
/// [`CHIP_LIFE_S`], so twelve is far past what a crowded clearing can produce and
/// still one array.
pub const CHIP_POOL: usize = 96;
/// How many chips one landed blow throws.
pub const CHIP_BURST: usize = 8;
/// How long a chip lives, seconds.
///
/// **`CHIP_`-prefixed, and the prefix is the knob registry's** — not a style
/// choice. `decal.rs` publishes its own `LIFE_S` (45 s, an arrow's mark) and
/// its own `SIZE_M`, both declared in `DECISIONS.md`, and
/// `ci/knob_registry.mjs` refuses one name meaning two things: *"the registry
/// cannot be authoritative about a name that means two things"*. It caught
/// this file on its first run through the gates, which is the gate working.
/// Same fix `RIG_SUN_ELEVATION` took for `SUN_ELEVATION`.
/// Short on purpose: this is a punctuation
/// mark on an impact, and debris that outlives the blow reads as litter.
pub const CHIP_LIFE_S: f32 = 0.55;
/// A chip's edge, metres.
pub const CHIP_SIZE_M: f32 = 0.045;
/// How fast a chip leaves the impact, m/s — the mean; each one is rolled
/// between half and one and a half of it.
pub const CHIP_SPEED_MPS: f32 = 3.2;
/// The downward acceleration on a chip, m/s². Earth's, because the sim's
/// own `GRAVITY_MM_PER_TICK2` is the same number in different units and a
/// chip that fell at a different rate from the player would read as wrong
/// without anybody being able to say why.
pub const CHIP_GRAVITY_MPS2: f32 = 9.81;
/// How fast a chip tumbles, rad/s.
pub const CHIP_SPIN_RAD_S: f32 = 11.0;
/// The share of a chip's launch that is **along the surface normal** rather
/// than scattered. At 1.0 every chip flies straight out and the burst is a
/// spike; at 0 it is a sphere and half of it goes into the wall. This is the
/// cone.
pub const CHIP_SPRAY: f32 = 0.55;

/// What was struck. Picks the colour and nothing else — the motion is one
/// law for every burst.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Matter {
    Wood,
    Stone,
    Metal,
    /// A body. `ART.md` has no blood in it and this is deliberately not
    /// blood: a dark red mote that says *you hit a person* and costs the
    /// game nothing it would have to defend.
    Flesh,
    /// A bush, a picked plant — the green half of the scatter.
    Plant,
    /// The island itself, and anything built out of it that this file
    /// cannot name more precisely.
    Dirt,
}

impl Matter {
    /// Every kind, so `setup` can build one material each and the array
    /// index below cannot drift from the enum.
    pub const ALL: [Matter; 6] = [
        Matter::Wood,
        Matter::Stone,
        Matter::Metal,
        Matter::Flesh,
        Matter::Plant,
        Matter::Dirt,
    ];

    fn slot(self) -> usize {
        match self {
            Matter::Wood => 0,
            Matter::Stone => 1,
            Matter::Metal => 2,
            Matter::Flesh => 3,
            Matter::Plant => 4,
            Matter::Dirt => 5,
        }
    }

    /// The chip's colour.
    ///
    /// **Values, not hues, are what carry this** (`ART.md` rule 3's habit):
    /// a chip is 4.5 cm at arm's length and its hue is nearly unreadable at
    /// that size, so each kind is picked to sit clear of the surface it
    /// comes off — pale splinters against dark bark, dark grit against pale
    /// granite — rather than to match it.
    pub fn color(self) -> Color {
        match self {
            Matter::Wood => Color::srgb(0.72, 0.55, 0.33),
            Matter::Stone => Color::srgb(0.55, 0.54, 0.51),
            Matter::Metal => Color::srgb(0.86, 0.78, 0.55),
            Matter::Flesh => Color::srgb(0.46, 0.10, 0.10),
            Matter::Plant => Color::srgb(0.38, 0.52, 0.22),
            Matter::Dirt => Color::srgb(0.44, 0.37, 0.28),
        }
    }

    /// What a scatter occupant is made of. `Occupant::None` and everything
    /// this file has no opinion about answer `Dirt`, which is the honest
    /// default rather than a refusal to draw.
    pub fn of_occupant(o: u8) -> Matter {
        match o {
            x if x == Occupant::Tree as u8 => Matter::Wood,
            x if x == Occupant::Bush as u8 => Matter::Plant,
            x if x == Occupant::StoneNode as u8 => Matter::Stone,
            x if x == Occupant::MetalNode as u8 => Matter::Metal,
            x if x == Occupant::SulfurNode as u8 => Matter::Stone,
            x if x == Occupant::BarrelSlot as u8 => Matter::Metal,
            _ => Matter::Dirt,
        }
    }

    /// What an arrow's stop surface is made of (`sim_core::ranged::SURF_*`).
    pub fn of_surface(surf: u8) -> Matter {
        match surf {
            SURF_GROUND => Matter::Dirt,
            SURF_WORLD => Matter::Wood,
            SURF_BUILT => Matter::Stone,
            _ => Matter::Dirt,
        }
    }
}

/// How far up a swung node's own ground the chips come from, metres.
///
/// A `Slot`'s `y` is where it stands, and a swing lands where a person can
/// reach — so a burst at the slot's foot throws chips out of the grass under
/// a tree rather than off the trunk. Per occupant, because a stone node is
/// knee-high and a pine is not.
pub fn strike_height(occupant: u8) -> f32 {
    match occupant {
        x if x == Occupant::Tree as u8 => 1.20,
        x if x == Occupant::Bush as u8 => 0.55,
        x if x == Occupant::BarrelSlot as u8 => 0.80,
        _ => 0.45,
    }
}

/// A pool entity. Both queries below filter on it, and that is not tidiness:
/// without it `strike`'s `(&mut Transform, &mut Visibility, &mut
/// MeshMaterial3d<_>)` matches every drawn thing on the island — every tree,
/// piece and prop — so the system would declare write access to the whole
/// world's transforms to touch ninety-six entities it reaches by id anyway.
/// Correct either way; the marker is what keeps the scheduler's picture of
/// it honest.
#[derive(Component)]
pub struct ChipOf;

/// One chip in flight. `left == 0` is a free slot — [`Chips::claim`]'s only
/// test, and the reason there is no separate liveness flag to keep in step.
///
/// **The chip's whole state lives here and not on its entity**, which is
/// `RENDER.md` §1's rule applied inside the renderer: [`Chips::step`] is the
/// motion and it takes a `&mut self` and a `f32`, so the cap, the overflow
/// policy, the spread and the arc are all drivable from a test with no
/// `World`, no GPU and no shard. `tests/impact.rs` does exactly that.
/// [`fly`] is then a copy into transforms — the thinnest a Bevy system in
/// this file can be, and `tracer.rs`'s own stated lesson: a law that only a
/// system can reach is a law nothing holds.
#[derive(Clone, Copy)]
struct Chip {
    left: f32,
    pos: Vec3,
    vel: Vec3,
    spin: Vec3,
    rot: Quat,
    matter: Matter,
}

impl Default for Chip {
    fn default() -> Self {
        Self {
            left: 0.0,
            pos: Vec3::ZERO,
            vel: Vec3::ZERO,
            spin: Vec3::ZERO,
            rot: Quat::IDENTITY,
            matter: Matter::Dirt,
        }
    }
}

#[derive(Resource)]
pub struct Chips {
    slots: [Chip; CHIP_POOL],
    entities: Vec<Entity>,
    /// Next slot to steal when everything is busy — the drop-oldest cursor.
    /// A cursor rather than a scan for the oldest, which is the same answer
    /// whenever bursts arrive in order and is O(1) when they do not.
    next: usize,
    /// Bursts thrown since the world was entered, and chips refused for want
    /// of a free slot.
    ///
    /// **The observable, and it is here for `CLAUDE.md`'s water-carry
    /// reason**: a test that can only read a return value is checking the
    /// branch it just read. `tests/impact.rs` asserts on these.
    pub bursts: u64,
    pub stolen: u64,
    /// The roll, advanced per chip. Seeded once and never re-seeded, so two
    /// bursts from the same point are not the same eight chips — which is
    /// the tell of a canned effect.
    rng: u32,
}

impl Default for Chips {
    // Hand-written because `[T; 96]` has no derived `Default` past 32 — and
    // the array is fixed at `CHIP_POOL` rather than a `Vec` for wall 4's reason,
    // so this is the cost of the bound rather than an oversight.
    fn default() -> Self {
        Self {
            slots: [Chip::default(); CHIP_POOL],
            entities: Vec::new(),
            next: 0,
            bursts: 0,
            stolen: 0,
            rng: 0,
        }
    }
}

impl Chips {
    /// A free slot, or the oldest one. Never `None`: see the header's
    /// overflow policy.
    fn claim(&mut self) -> usize {
        if let Some(i) = self.slots.iter().position(|c| c.left <= 0.0) {
            return i;
        }
        self.stolen += 1;
        let i = self.next;
        self.next = (self.next + 1) % CHIP_POOL;
        i
    }

    /// How many chips are drawing right now. Public for the gate's sake, as
    /// `Tracers::live` is; nothing in the frame path reads it.
    pub fn live(&self) -> usize {
        self.slots.iter().filter(|c| c.left > 0.0).count()
    }

    /// One roll in `[0, 1)`. A 32-bit xorshift — no allocation, no clock, and
    /// deterministic for a given sequence of bursts, which is what makes the
    /// spread gate reproducible.
    fn roll(&mut self) -> f32 {
        // Seeded on first use rather than in `Default`, so a `Chips::default()`
        // in a test is not a degenerate generator that returns zero forever.
        if self.rng == 0 {
            self.rng = 0x9E37_79B9;
        }
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng >> 8) as f32 / (1 << 24) as f32
    }

    /// A roll in `[-1, 1)`.
    fn signed(&mut self) -> f32 {
        self.roll() * 2.0 - 1.0
    }
}

/// Which store a landed blow's victim is drawn out of, and how high on it
/// the blow lands.
///
/// **A named decision rather than a `match` inside the system**, which is
/// `tests/tracer.rs`' whole lesson written down: a rule only a Bevy system
/// can reach is a rule nothing holds, and the first cut of this rule was
/// exactly that — an inline lookup in `Bodies` alone, so a wolf took a blow
/// and the frame was unchanged. It failed silently because a miss and an
/// unrecognised victim are the same `continue`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Struck {
    /// A player, drawn by `bodies::stream`. `lift` is chest height on the
    /// volume the sim shoots at (`anim::ANIM_BODY_H_M`) rather than a guess
    /// about the mesh.
    Player { lift: f32 },
    /// An animal, drawn by `mobs::stream` — a different store, a different
    /// component, and the same id space split by `mob::slot_of_id`.
    Animal { slot: usize, lift: f32 },
}

impl Struck {
    pub fn lift(self) -> f32 {
        match self {
            Struck::Player { lift } | Struck::Animal { lift, .. } => lift,
        }
    }
}

/// Where a blow on `victim` lands, from the wire id alone.
pub fn struck(victim: u32) -> Struck {
    match sim_core::mob::slot_of_id(victim) {
        Some(slot) => Struck::Animal {
            slot,
            lift: super::mobs::flank_h_of(slot),
        },
        None => Struck::Player {
            lift: super::anim::ANIM_BODY_H_M * PLAYER_CHEST_FRAC,
        },
    }
}

/// Where a blow lands up a standing player, as a fraction of their height.
/// A chest on a 1.8 m figure is 1.12 m, which is the rung `combat`'s own
/// bands put between the head and the limbs.
pub const PLAYER_CHEST_FRAC: f32 = 0.62;

/// Everything a burst needs, so the four callers below read as four facts
/// rather than as four copies of the same six arguments.
pub struct Burst {
    pub at: Vec3,
    /// Which way the debris is thrown. Normalised by [`throw`]; a zero
    /// vector is taken as straight up, which is the right answer for a blow
    /// whose direction nothing recorded.
    pub away: Vec3,
    pub matter: Matter,
}

impl Chips {
    /// Throw one burst — the one place a chip is given its motion, and the
    /// one place the pool's bound is spent.
    ///
    /// Pure: no `World`, no assets, no clock. See [`Chip`].
    pub fn ignite(&mut self, b: &Burst) {
        let away = b.away.normalize_or(Vec3::Y);
        self.bursts += 1;
        for _ in 0..CHIP_BURST {
            // A cone about `away`: a scattered unit vector blended toward the
            // normal by CHIP_SPRAY, so nothing is thrown into the surface it came
            // off and the burst still has a shape.
            let scatter =
                Vec3::new(self.signed(), self.signed(), self.signed()).normalize_or(Vec3::Y);
            let dir = (away * CHIP_SPRAY + scatter * (1.0 - CHIP_SPRAY)).normalize_or(away);
            // Half to one and a half of the nominal speed, plus a little lift
            // so the burst arcs instead of skidding along the surface.
            let speed = CHIP_SPEED_MPS * (0.5 + self.roll());
            let spin = Vec3::new(self.signed(), self.signed(), self.signed()) * CHIP_SPIN_RAD_S;
            let rot = Quat::from_euler(
                EulerRot::YXZ,
                self.signed() * std::f32::consts::PI,
                self.signed() * std::f32::consts::PI,
                self.signed() * std::f32::consts::PI,
            );
            let life = CHIP_LIFE_S * (0.7 + 0.6 * self.roll());
            let ix = self.claim();
            self.slots[ix] = Chip {
                left: life,
                // Started a few centimetres off the surface for the decal's
                // reason — a chip born exactly on a wall z-fights it for its
                // first frame.
                pos: b.at + dir * (CHIP_SIZE_M * 1.5),
                vel: dir * speed + Vec3::Y * (CHIP_SPEED_MPS * 0.25),
                spin,
                rot,
                matter: b.matter,
            };
        }
    }

    /// Advance every live chip by `dt` and retire the ones whose time is up.
    pub fn step(&mut self, dt: f32) {
        if dt <= 0.0 {
            return;
        }
        for c in &mut self.slots {
            if c.left <= 0.0 {
                continue;
            }
            c.left -= dt;
            if c.left <= 0.0 {
                c.left = 0.0;
                continue;
            }
            c.vel.y -= CHIP_GRAVITY_MPS2 * dt;
            c.pos += c.vel * dt;
            c.rot *= Quat::from_euler(EulerRot::YXZ, c.spin.x * dt, c.spin.y * dt, c.spin.z * dt);
        }
    }

    /// What slot `i` should be drawn as, or `None` when it is free.
    ///
    /// The scale shrinks out over the last third of a chip's life rather
    /// than fading: an alpha fade wants a transparent material, which is a
    /// second pipeline and a sort — shrinking to nothing costs a multiply.
    pub fn draw(&self, i: usize) -> Option<(Vec3, Quat, f32, Matter)> {
        let c = self.slots.get(i)?;
        if c.left <= 0.0 {
            return None;
        }
        let t = (c.left / (CHIP_LIFE_S * 0.34)).min(1.0);
        Some((c.pos, c.rot, CHIP_SIZE_M * t, c.matter))
    }
}

/// The pool's shared meshes and materials. One handle per [`Matter`], built
/// at startup so a burst never touches `Assets`.
#[derive(Resource)]
pub struct ChipAssets {
    mats: Vec<Handle<StandardMaterial>>,
}

pub fn setup(
    mut commands: Commands,
    mut pool: ResMut<Chips>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // A cuboid rather than a billboard: chips tumble, and a tumbling solid
    // needs no per-frame facing pass — 96 quads turned toward the camera is
    // 96 quaternions a frame for a worse read at this size.
    let mesh = meshes.add(Cuboid::new(1.0, 1.0, 0.42));
    let mats: Vec<Handle<StandardMaterial>> = Matter::ALL
        .iter()
        .map(|m| {
            materials.add(StandardMaterial {
                base_color: m.color(),
                // **Lit, unlike `tracer`.** A tracer is a readability
                // affordance drawn over the world; a chip is a thing in the
                // world, and one that ignored the sun would read as a decal
                // pasted on the frame — which is the exact note the browser
                // client's viewmodel earned.
                perceptual_roughness: 0.85,
                reflectance: super::fresnel::DIELECTRIC,
                ..default()
            })
        })
        .collect();
    pool.entities = (0..CHIP_POOL)
        .map(|_| {
            commands
                .spawn((
                    ChipOf,
                    Mesh3d(mesh.clone()),
                    MeshMaterial3d(mats[0].clone()),
                    Transform::from_scale(Vec3::splat(CHIP_SIZE_M)),
                    Visibility::Hidden,
                ))
                .id()
        })
        .collect();
    commands.insert_resource(ChipAssets { mats });
}

/// Advance the pool and copy it onto the entities.
///
/// The whole of the motion is [`Chips::step`]; this is the draw.
pub fn fly(
    time: Res<Time>,
    mut pool: ResMut<Chips>,
    assets: Option<Res<ChipAssets>>,
    mut q: Query<
        (
            &mut Transform,
            &mut Visibility,
            &mut MeshMaterial3d<StandardMaterial>,
        ),
        With<ChipOf>,
    >,
) {
    pool.step(time.delta_secs());
    let Some(assets) = assets else { return };
    for i in 0..CHIP_POOL {
        let Some(&entity) = pool.entities.get(i) else {
            continue;
        };
        let Ok((mut tf, mut vis, mut m)) = q.get_mut(entity) else {
            continue;
        };
        match pool.draw(i) {
            None => {
                if *vis != Visibility::Hidden {
                    *vis = Visibility::Hidden;
                }
            }
            Some((pos, rot, scale, matter)) => {
                tf.translation = pos;
                tf.rotation = rot;
                tf.scale = Vec3::splat(scale);
                *vis = Visibility::Visible;
                // A handle write, not an asset mutation: the six materials
                // are built once and shared, so a burst costs a pointer copy
                // rather than a per-chip material in the asset store
                // (`decal.rs` pays that because its alpha is per mark;
                // nothing here fades by colour).
                if let Some(want) = assets.mats.get(matter.slot()) {
                    if m.0 != *want {
                        m.0 = want.clone();
                    }
                }
            }
        }
    }
}

/// Fire a burst for every landed blow the feed reports.
///
/// Reads `Res<Feed>` and never `pop_*`, for the single-drain reason
/// `feed.rs`'s header narrates and `tests/sound.rs` greps for.
/// Eight parameters, which clippy counts — `bodies::stream` carries the same
/// allow and the same argument. A `SystemParam` struct would exist only to
/// satisfy the count: the two victim stores are the eighth and they are as
/// distinct from each other as `feed` is from `eye`, and bundling any two
/// would hide which of the eight a future reader has to think about.
#[allow(clippy::too_many_arguments)]
pub fn strike(
    mut pool: ResMut<Chips>,
    feed: Res<Feed>,
    eye: Res<Eye>,
    world: Option<Res<WorldId>>,
    net: Option<NonSend<Net>>,
    swung: Res<super::verbs::Swung>,
    bodies: Query<(&super::bodies::Body, &GlobalTransform)>,
    herd: Query<(&super::mobs::Animal, &GlobalTransform)>,
) {
    let Some(net) = net else { return };
    let core = &net.session.core;

    // ── A gather that paid out ───────────────────────────────────────────
    //
    // The sim says a node gave something up; the pick says which node the
    // crosshair is on. It is the same scan the sim ran (`interact::
    // resolve_swing` mirrors `gather`'s), and the player is standing still
    // swinging at it, so the two agree — see the header's one seam.
    if !feed.gathered().is_empty() && swung.0.occupant != 0 {
        let s = &swung.0;
        let at = Vec3::new(s.x, s.y + strike_height(s.occupant), s.z);
        pool.ignite(&Burst {
            at,
            // Back toward the player: chips come off the face that was
            // struck, which is the one they are looking at.
            away: (eye.pos - at).with_y(0.0),
            matter: Matter::of_occupant(s.occupant),
        });
    }

    // ── A blow that landed on something alive ────────────────────────────
    //
    // **Two stores, and the wire id is what says which.** `EV_HIT` names a
    // victim and nothing else, and this client draws players out of
    // `bodies::Bodies` and animals out of `mobs::Herd` — two systems, two
    // component types, one id space split by `mob::slot_of_id`. The first
    // cut of this file looked only in `Bodies`, so a wolf took a blow and
    // the frame was unchanged: the exact gap the whole slice was opened for,
    // one victim class over, and invisible because a miss and an
    // unrecognised victim are the same `continue`.
    for &victim in feed.hit_victims() {
        let hit = struck(victim);
        let at = match hit {
            Struck::Animal { .. } => herd
                .iter()
                .find(|(a, _)| a.0 == victim)
                .map(|(_, gt)| gt.translation()),
            Struck::Player { .. } => bodies
                .iter()
                .find(|(b, _)| b.0 == victim)
                .map(|(_, gt)| gt.translation()),
        };
        // Not in the interest set, or not drawn yet. A blow whose victim
        // this client cannot place throws nothing rather than guessing at
        // the crosshair — `RENDER.md` §1, and the same posture the gather
        // burst takes when the pick is empty.
        let Some(at) = at.map(|p| p + Vec3::Y * hit.lift()) else {
            continue;
        };
        pool.ignite(&Burst {
            at,
            away: (eye.pos - at).normalize_or(Vec3::Y),
            matter: Matter::Flesh,
        });
    }

    // ── A blow that landed on a wall ─────────────────────────────────────
    //
    // Latched, so the freshness bit decides and not the field — `Feed::
    // applied`'s doc, and the failure without it is a wall that sprays chips
    // forever after one hit.
    if feed.applied & client_core::core::APPLIED_STRUCT_HIT != 0 {
        let (cx, cz, level, _, _, _) = core.struct_hit;
        // The build grid's own floor for that column — `build::
        // column_floor_y`, the sim's function rather than a second opinion
        // about where a storey sits (the failure that function exists to
        // end). `plate` comes off the piece mirror the same way
        // `deploy::floor_of` takes it.
        let plate = core.pieces.cols().plate(cx, cz).unwrap_or(0);
        let floor = match world.as_deref() {
            Some(w) => sim_core::build::column_floor_y(w.seed, &w.haven, cx, cz, plate),
            // No world means no island to ask; the eye's own feet are the
            // honest fallback and the burst is cosmetic either way.
            None => eye.pos.y - super::EYE_HEIGHT,
        };
        let at = Vec3::new(
            (cx as f32 + 0.5) * BUILD_CELL_M,
            floor + level as f32 * LEVEL_H_M + LEVEL_H_M * 0.5,
            (cz as f32 + 0.5) * BUILD_CELL_M,
        );
        pool.ignite(&Burst {
            at,
            away: (eye.pos - at).with_y(0.0),
            matter: Matter::Dirt,
        });
    }

    // ── An arrow that stopped ────────────────────────────────────────────
    //
    // The only one of the four whose position is the SIM's rather than
    // recovered: `EV_IMPACT` carries the stop point in the wire's own
    // quanta, which is why `decal.rs` can lay a mark there.
    for &(qx, qy, qz, surf) in feed.impacts() {
        let at = Vec3::new(
            qx as f32 * POS_XZ_Q,
            qy as f32 * POS_Y_Q,
            qz as f32 * POS_XZ_Q,
        );
        pool.ignite(&Burst {
            at,
            away: (eye.pos - at).normalize_or(Vec3::Y),
            matter: Matter::of_surface(surf),
        });
    }
}

/// Retire every chip on the way out of a world, so a reconnect does not open
/// on the last fight's debris hanging in the air.
///
/// `map::forget`'s shape and `decal::forget_in`'s reason: the pool outlives
/// the world because its entities do.
pub fn forget(mut pool: ResMut<Chips>, mut q: Query<&mut Visibility, With<ChipOf>>) {
    for i in 0..CHIP_POOL {
        pool.slots[i] = Chip::default();
        if let Some(&e) = pool.entities.get(i) {
            if let Ok(mut v) = q.get_mut(e) {
                *v = Visibility::Hidden;
            }
        }
    }
    pool.bursts = 0;
    pool.stolen = 0;
}
