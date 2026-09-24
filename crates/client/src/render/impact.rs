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
//! is fired from a fact the SIM already announced — an `EV_SWING`, an
//! `EV_HIT`, an `EV_STRUCT_HIT`, an arrow's `EV_IMPACT` — never from the
//! button and never from the client's own reading of what is in reach.
//!
//! ⚠ **The first of those four was `EV_GATHER` for one commit, and that was
//! a defect rather than a looser reading of the same fact.** `EV_GATHER`'s
//! own doc says it announces a *backpack loot* the same way it announces a
//! node paying out, and deliberately — so emptying a corpse's pack while
//! facing a tree threw wood chips off the tree, once per slot moved.
//! [`gather_burst`] carries the correction in full.
//!
//! **One seam is honest to state.** Three of the four facts say what was hit
//! and not *where*: `EV_SWING` carries only the swinger, `EV_HIT` a victim
//! id, `EV_STRUCT_HIT` a build address. So the point a burst comes from is
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
//!
//! # One contact list, three layers and a sound (2026-09-13)
//!
//! The chips were the whole answer to *"connecting looks like missing"* for
//! two weeks and the operator's next note was that they are not enough
//! (*"we dont really have any good FX effect around combat or hitting rocks
//! or hitting woods"*). What was missing is not more chips: it is the two
//! things a chip cannot be — the **hot** fragment a pick strikes off stone
//! and metal (`sparks.rs`) and the **soft** cloud every solid blow knocks
//! loose (`dust.rs`) — plus the sound of the matter struck, which the bank
//! has carried since audio v0 with no producer (`Cue::ImpactWood` and its
//! two siblings).
//!
//! So the resolver and the draw are split. [`contacts`] turns this frame's
//! facts into a bounded list of [`Contact`]s — *a blow met this matter at
//! this point, thrown this way* — and [`strike`], `sparks::fly`, `dust::fly`
//! and `audio::impacts` all read that one list. It is `feed.rs`'s shape one
//! layer down: one resolution of *where and what*, many readers, so the
//! puff, the sparks, the chips and the thock cannot disagree about which
//! blow they are for.
//!
//! **And it de-duplicates a blow the wire reports twice.** A landed node
//! swing arrives as `EV_SWING` (the cadence gate) AND `EV_IMPACT` (the
//! bite's skin point, the same event an arrow's stop uses) in one frame, and
//! the first cut of this file threw a burst for each — sixteen chips at two
//! points for one hatchet blow. [`same_blow`] is the rule: an own swing
//! claims the world-surface impact nearest its pick, and the impact's point
//! — the sim's own, on the ray — is the one the burst comes from; the pick
//! answers only for the swings the sim does not mark (a refused tool, a
//! barrel, a bush).

use bevy::prelude::*;

use super::feed::Feed;
use super::{surface, Eye, Net, WorldId};
use crate::sound::Cue;
use crate::ui::interact::SwingPick;
use sim_core::build::{MAT_METAL, MAT_STONE, MAT_TWIG, MAT_WOOD};
use sim_core::movement::{POS_XZ_Q, POS_Y_Q};
use sim_core::ranged::{SURF_GROUND, SURF_WORLD};
use sim_core::terrain::{self, Occupant};

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

/// What was struck — resolved once per blow by `surface::matter_at`, and the
/// one input every effect layer (particles, decal, sound) keys on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Matter {
    Wood,
    Stone,
    Metal,
    /// A body — players and animals alike. Blood (operator, 2026-09-24).
    Flesh,
    /// A bush, a picked plant — the green half of the scatter.
    Plant,
    /// Forest floor and anything this file cannot name more precisely.
    Dirt,
    Sand,
    Grass,
    /// A shot into the sea: the splash on the surface, never the seabed.
    Water,
}

/// How many [`Matter`]s there are — every per-matter table's length.
pub const MATTER_COUNT: usize = 9;

impl Matter {
    /// Every kind, so `setup` can build one material each and the array
    /// index below cannot drift from the enum.
    pub const ALL: [Matter; MATTER_COUNT] = [
        Matter::Wood,
        Matter::Stone,
        Matter::Metal,
        Matter::Flesh,
        Matter::Plant,
        Matter::Dirt,
        Matter::Sand,
        Matter::Grass,
        Matter::Water,
    ];

    /// The index into every per-matter table — `Matter::ALL`'s order.
    pub fn slot(self) -> usize {
        match self {
            Matter::Wood => 0,
            Matter::Stone => 1,
            Matter::Metal => 2,
            Matter::Flesh => 3,
            Matter::Plant => 4,
            Matter::Dirt => 5,
            Matter::Sand => 6,
            Matter::Grass => 7,
            Matter::Water => 8,
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
            Matter::Sand => Color::srgb(0.74, 0.66, 0.50),
            Matter::Grass => Color::srgb(0.40, 0.46, 0.24),
            Matter::Water => Color::srgb(0.70, 0.78, 0.82),
        }
    }

    /// What a scatter occupant is made of. `Occupant::None` and everything
    /// this file has no opinion about answer `Dirt`, which is the honest
    /// default rather than a refusal to draw.
    ///
    /// The boulder and the two wooden containers joined the table with
    /// `world_matter` (2026-09-13): an arrow that stops on a rock is a
    /// `SURF_WORLD` impact too, and it used to throw dirt off granite.
    pub fn of_occupant(o: u8) -> Matter {
        match o {
            x if x == Occupant::Tree as u8 => Matter::Wood,
            x if x == Occupant::Bush as u8 => Matter::Plant,
            x if x == Occupant::StoneNode as u8 => Matter::Stone,
            x if x == Occupant::MetalNode as u8 => Matter::Metal,
            x if x == Occupant::SulfurNode as u8 => Matter::Stone,
            x if x == Occupant::BarrelSlot as u8 => Matter::Metal,
            x if x == Occupant::Rock as u8 => Matter::Stone,
            x if x == Occupant::CrateSlot as u8 => Matter::Wood,
            x if x == Occupant::CacheSlot as u8 => Matter::Wood,
            _ => Matter::Dirt,
        }
    }

    /// What a built piece is made of, from its baked row's material tier
    /// (`sim_core::build::MAT_*`). Twig is wood — it is sticks.
    pub fn of_piece(material: u8) -> Matter {
        match material {
            MAT_TWIG | MAT_WOOD => Matter::Wood,
            MAT_STONE => Matter::Stone,
            MAT_METAL => Matter::Metal,
            _ => Matter::Dirt,
        }
    }
}

/// The sound a blow on this matter makes at the point it landed, or `None`
/// for a matter whose blow is already voiced another way: a body's is the
/// hitmarker plus the victim's own hurt cue, and a plant has no waveform in
/// the bank worth a wrong one. Dirt takes the stone crunch — turned earth
/// is closer to that than to a wooden thock.
///
/// A pure map so `tests/impact.rs` can hold it: `Cue::ImpactWood` shipped
/// with no producer from audio v0 (2026-08-06) to 2026-09-13, and every
/// gate over the bank was green, because a def, a waveform and a mixer are
/// all correct with nobody asking.
pub fn impact_cue(matter: Matter) -> Option<Cue> {
    match matter {
        Matter::Wood => Some(Cue::ImpactWood),
        Matter::Stone | Matter::Dirt | Matter::Sand | Matter::Grass => Some(Cue::ImpactStone),
        Matter::Metal => Some(Cue::ImpactMetal),
        Matter::Flesh | Matter::Plant | Matter::Water => None,
    }
}

/// What delivered a blow — picks the decal (a bullet hole, a gash) and, once
/// `fx` lands, the size of what it throws.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Weapon {
    Melee,
    Arrow,
    Bullet,
    Blast,
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

/// Where a landed swing's chips come from, or `None` for a swing that hit
/// nothing.
///
/// **A named decision for [`struck`]'s reason**, and it fixes a real defect
/// in the first cut of this file rather than only tidying it. That cut fired
/// the gather burst on `Feed::gathered()` being non-empty — a payout — and
/// `EV_GATHER`'s own doc says what is wrong with that: *"looting a backpack
/// announces its take the same way, and deliberately"*. So emptying a
/// corpse's pack while facing a tree threw wood chips off the tree, once per
/// slot moved.
///
/// What replaces it is the sim's own swing fact. `EV_SWING` is pushed by
/// `gather::swing` **at the cadence gate, before the scan** — *"the only
/// point in the tree that runs exactly once per swing regardless of what the
/// swing goes on to find"* — and the server sends a body event to its own
/// subject unconditionally (`body_event_visible`'s first line), so the
/// swinger receives their own. Paired with the pick, which is the client's
/// mirror of the very scan that swing ran, the two say *you swung, and there
/// was something in reach* — which is the operator's *"when u actually
/// connect with something"* stated exactly.
///
/// **Not gated on a payout**, and that is the other half of the correction: a
/// barrel that pays nothing, a pack too full to take the wood, and a torch
/// the node refuses (`REFUSE_G_*`) are all swings that CONNECTED. Chips are a
/// fact about contact, not about income.
///
/// **On the skin, facing the swinger** (2026-09-13). The first cut put the
/// burst at the slot's own `x`/`z`, which is the node's AXIS: chips were born
/// a hand's width from the centre of a 0.9 m stone node and spent most of
/// their half-second flying out through rock nobody could see them in, and
/// a tree's came out of the middle of the trunk. `from` is the eye, and the
/// point is the occupant's collision skin ([`skin_radius`] × the slot's
/// scale) on the side the player is standing on — the same skin the sim's
/// own `NodeHit::skin` projects an `EV_IMPACT` onto, so the two ways a blow
/// can be placed agree to within the ray's own angle.
pub fn gather_burst(swung_this_tick: bool, pick: &SwingPick, from: Vec3) -> Option<Burst> {
    if !swung_this_tick || pick.occupant == 0 {
        return None;
    }
    let centre = Vec3::new(pick.x, pick.y + strike_height(pick.occupant), pick.z);
    let toward = (from - centre).with_y(0.0).normalize_or(Vec3::Z);
    let r = skin_radius(pick.occupant) * pick.scale;
    Some(Burst {
        at: centre + toward * r,
        // Back toward the player: chips come off the face that was struck,
        // which is the one they are looking at.
        away: toward,
        matter: Matter::of_occupant(pick.occupant),
    })
}

/// The radius of a swingable occupant's collision skin at scale 1.0 — the
/// sim's own `occupant_volume`, read for the archetypes a swing can pick and
/// zero for the bush, which has no skin and is struck at its centre.
pub fn skin_radius(occupant: u8) -> f32 {
    let o = match occupant {
        x if x == Occupant::Tree as u8 => Occupant::Tree,
        x if x == Occupant::StoneNode as u8 => Occupant::StoneNode,
        x if x == Occupant::MetalNode as u8 => Occupant::MetalNode,
        x if x == Occupant::SulfurNode as u8 => Occupant::SulfurNode,
        x if x == Occupant::BarrelSlot as u8 => Occupant::BarrelSlot,
        _ => return 0.0,
    };
    terrain::occupant_volume(o).0
}

/// How far, planar metres, a world-surface impact may stand from the swing
/// pick's slot and still be the same blow — the largest swingable skin
/// (`Occupant::StoneNode`, 0.91 m) at the largest slot scale, plus the
/// ray's own probe, rounded up. An impact further out than this on the same
/// frame is somebody else's arrow or somebody else's swing.
pub const SAME_BLOW_M: f32 = 2.0;

/// Is the impact at `at` the pick's own blow, reported a second time?
///
/// The rule the header names: a landed node swing reaches the client as
/// both `EV_SWING` and `EV_IMPACT` in one frame, and without this every
/// hatchet blow was two bursts and would have been two thocks.
pub fn same_blow(pick: &SwingPick, at: Vec3) -> bool {
    if pick.occupant == 0 {
        return false;
    }
    let (dx, dz) = (at.x - pick.x, at.z - pick.z);
    dx * dx + dz * dz <= SAME_BLOW_M * SAME_BLOW_M
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

/// What kind of fact a [`Contact`] came from — which decides nothing about
/// how it is drawn (the matter does) and everything about what a reader may
/// assume: a `Swing` has the swinger's own pick behind it, a `Flesh` has a
/// hitmarker already voicing it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ContactKind {
    /// The local player's own swing met the thing its pick named.
    Swing,
    /// An `EV_IMPACT`: an arrow's or a bullet's stop, or a swing the sim
    /// marked — anyone's.
    Impact,
    /// A blow landed on a body.
    Flesh,
}

/// One blow's contact with the world this frame: where, which way the surface
/// faces, on what, by what. The whole of what every effect layer needs,
/// resolved once by [`contacts`].
#[derive(Clone, Copy, Debug)]
pub struct Contact {
    pub at: Vec3,
    /// Which way the debris is thrown.
    pub away: Vec3,
    /// Which way the struck surface faces — what a mark lies on.
    pub normal: Vec3,
    /// The wire's `SURF_*` the blow stopped on; a world mark bends to its trunk.
    pub surf: u8,
    pub matter: Matter,
    pub kind: ContactKind,
    pub weapon: Weapon,
    /// Whether the blow leaves a mark (`decal::mark`).
    pub mark: bool,
}

impl Default for Contact {
    fn default() -> Self {
        Self {
            at: Vec3::ZERO,
            away: Vec3::Y,
            normal: Vec3::Y,
            surf: SURF_GROUND,
            matter: Matter::Dirt,
            kind: ContactKind::Impact,
            weapon: Weapon::Bullet,
            mark: false,
        }
    }
}

/// Contacts one frame may carry: the impacts, the hit victims and the own
/// swing, each at most `FEED_CAP` long.
pub const CONTACT_CAP: usize = super::feed::FEED_CAP * 3;

/// This frame's contacts — `Feed`'s shape: one resolver writes it, every
/// layer reads it through `Res<_>`, so no layer can consume a fact another
/// wanted (`CLAUDE.md`'s clean-merge trap).
#[derive(Resource)]
pub struct Contacts {
    list: [Contact; CONTACT_CAP],
    n: usize,
    /// Contacts refused for want of a slot. A number, never a silent cap.
    pub dropped: u32,
}

impl Default for Contacts {
    fn default() -> Self {
        Self {
            list: [Contact::default(); CONTACT_CAP],
            n: 0,
            dropped: 0,
        }
    }
}

impl Contacts {
    pub fn clear(&mut self) {
        self.n = 0;
    }

    /// Add one, drop-newest past the cap and count it.
    pub fn push(&mut self, c: Contact) {
        if self.n >= CONTACT_CAP {
            self.dropped = self.dropped.saturating_add(1);
            return;
        }
        self.list[self.n] = c;
        self.n += 1;
    }

    pub fn iter(&self) -> impl Iterator<Item = &Contact> {
        self.list[..self.n].iter()
    }

    pub fn len(&self) -> usize {
        self.n
    }

    pub fn is_empty(&self) -> bool {
        self.n == 0
    }
}

/// How near a body that swung this frame must stand to an impact for the
/// impact to be that swing's, metres — a melee reach plus the ray's probe.
/// It is how a remote hatchet blow is told from a bullet until the wire
/// names the weapon.
pub const SWING_REACH_M: f32 = 3.5;

/// What delivered an impact nobody claimed: a swing if a swinger stood within
/// [`SWING_REACH_M`] of it this frame, a shot otherwise.
pub fn weapon_of(mine: bool, swinger_near: bool) -> Weapon {
    if mine || swinger_near {
        Weapon::Melee
    } else {
        Weapon::Bullet
    }
}

/// Resolve every blow the feed reports into this frame's [`Contacts`].
///
/// Reads `Res<Feed>` and never `pop_*`, for the single-drain reason
/// `feed.rs`'s header narrates and `tests/sound.rs` greps for. Where and
/// what: the point is the sim's (`EV_IMPACT`) or the pick's or the drawn
/// body's; the facing and the matter are `surface`'s.
#[allow(clippy::too_many_arguments)]
pub fn contacts(
    mut out: ResMut<Contacts>,
    feed: Res<Feed>,
    eye: Res<Eye>,
    world: Option<Res<WorldId>>,
    net: Option<NonSend<Net>>,
    swung: Res<super::verbs::Swung>,
    bodies: Query<(&super::bodies::Body, &GlobalTransform)>,
    herd: Query<(&super::mobs::Animal, &GlobalTransform)>,
) {
    out.clear();
    let Some(net) = net else { return };
    let core = &net.session.core;
    let own_swing = feed.swings().contains(&core.player_id);
    let pick = &swung.0;
    let swinger_near = |at: Vec3| {
        feed.swings().iter().any(|&id| {
            let p = if id == core.player_id {
                Some(eye.pos)
            } else {
                bodies
                    .iter()
                    .find(|(b, _)| b.0 == id)
                    .map(|(_, gt)| gt.translation())
            };
            p.is_some_and(|p| p.distance_squared(at) <= SWING_REACH_M * SWING_REACH_M)
        })
    };

    // ── Every surface the sim says was struck ────────────────────────────
    //
    // The only source whose position is the SIM's rather than recovered, so
    // when this frame's own swing is among them the impact's point is the
    // one the burst takes (`same_blow`).
    let mut claimed = false;
    for &(qx, qy, qz, surf) in feed.impacts() {
        let mut at = Vec3::new(
            qx as f32 * POS_XZ_Q,
            qy as f32 * POS_Y_Q,
            qz as f32 * POS_XZ_Q,
        );
        let mine = own_swing && surf == SURF_WORLD && same_blow(pick, at);
        claimed |= mine;
        let (mut normal, mut matter) = match world.as_deref() {
            Some(w) => {
                let n = surface::normal_at(w, core.pieces.cols(), at.x, at.y, at.z, surf);
                (n, surface::matter_at(w, core, at, surf, n))
            }
            None => (Vec3::Y, Matter::Dirt),
        };
        if mine {
            matter = Matter::of_occupant(pick.occupant);
        }
        // A shot into the sea stops on the seabed; what a player sees is the
        // splash on the surface above it.
        if matter == Matter::Water {
            at.y = terrain::SEA_LEVEL;
            normal = Vec3::Y;
        }
        out.push(Contact {
            at,
            away: normal,
            normal,
            surf,
            matter,
            kind: if mine {
                ContactKind::Swing
            } else {
                ContactKind::Impact
            },
            weapon: weapon_of(mine, swinger_near(at)),
            mark: !matches!(matter, Matter::Water | Matter::Plant),
        });
    }

    // ── A swing that connected with a node the sim did not mark ──────────
    //
    // A refused tool, a barrel, a bush: the sim's own swing fact plus the
    // pick ([`gather_burst`]). Skipped when an impact above already placed
    // this blow. No mark — the sim did not mark it.
    if !claimed {
        if let Some(b) = gather_burst(own_swing, pick, eye.pos) {
            out.push(Contact {
                at: b.at,
                away: b.away,
                normal: b.away,
                surf: SURF_WORLD,
                matter: b.matter,
                kind: ContactKind::Swing,
                weapon: Weapon::Melee,
                mark: false,
            });
        }
    }

    // ── A blow that landed on something alive ────────────────────────────
    //
    // Two stores, split by the wire id (`struck`): players out of
    // `bodies::Bodies`, animals out of `mobs::Herd`. A victim this client
    // cannot place throws nothing rather than guessing at the crosshair.
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
        let Some(at) = at.map(|p| p + Vec3::Y * hit.lift()) else {
            continue;
        };
        // The entry face looks back at the attacker, and that is where the
        // spray is thrown; the mark lands behind the victim.
        let toward = (eye.pos - at).normalize_or(Vec3::Y);
        out.push(Contact {
            at,
            away: toward,
            normal: toward,
            surf: SURF_GROUND,
            matter: Matter::Flesh,
            kind: ContactKind::Flesh,
            weapon: if own_swing {
                Weapon::Melee
            } else {
                Weapon::Bullet
            },
            mark: true,
        });
    }
}

/// Throw the debris for every contact this frame: chips here, sparks and
/// dust in their own pools, all off the one list so a blow's three layers
/// leave the same point in the same direction.
pub fn strike(
    contacts: Res<Contacts>,
    mut chips: ResMut<Chips>,
    mut sparks: ResMut<super::sparks::Sparks>,
    mut dust: ResMut<super::dust::Dust>,
) {
    for c in contacts.iter() {
        let b = Burst {
            at: c.at,
            away: c.away,
            matter: c.matter,
        };
        if c.matter != Matter::Water {
            chips.ignite(&b);
        }
        sparks.ignite(&b);
        dust.ignite(&b);
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
