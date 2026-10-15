//! The Verdigris as this client draws it (`sim_core::blight`): this wipe's
//! outbreaks, drawn once from the welcome's salt, and the spread at the arc's
//! own tick — never the capture's pinned hour of the day.
//!
//! **One moment for every reader.** The ground's uniform, the trees that
//! wither, the map's overlay and the counter in your ear all read [`Blight::now`],
//! quantised to [`STEP_TICKS`], so they agree with one another and nothing
//! re-uploads or re-streams more often than the front actually moves.
//!
//! **The tier is held while the works drip in.** A join receives each work's
//! row and then its state; over a partial set `fel_tier_of` can read a tier
//! the island is not in. The last whole reading stands until every row the
//! shard named has had its first state (`ArcView::synced`).

use bevy::prelude::*;
use sim_core::blight as spread;
use sim_core::terrain;

use super::feed::Feed;
use super::ground_splat::{BlightLive, GroundMaterial};
use super::terrain_mesh::Ring;
use super::{Net, WorldId};

/// The step the spread is read on, ticks: three minutes of the arc clock, in
/// which the heart's front walks about a fifth of a metre.
pub const STEP_TICKS: u64 = spread::TICKS_PER_HOUR / 20;

/// The spread as this client last read it.
#[derive(Resource, Default)]
pub struct Blight {
    key: Option<(u64, u32)>,
    /// This wipe's outbreaks (`WorldId::blight_salt`).
    pub plan: spread::Plan,
    /// The island's fel tier, held over a join's partial drip.
    pub tier: u8,
    /// The arc tick [`Blight::now`] was read at, a multiple of [`STEP_TICKS`].
    pub tick: u64,
    /// The spread at `tick`; `None` off a world.
    pub now: Option<spread::Now>,
    /// Moves whenever `now` does: what a cached painting compares.
    pub gen: u32,
}

impl Blight {
    /// How deep the green is at (`x`, `z`) on this world's ground: the
    /// spread now, or the static heart before the first moment is read. The
    /// ground height is sampled only within the spread's reach.
    pub fn at(&self, world: &WorldId, x: f32, z: f32) -> spread::Depth {
        match &self.now {
            Some(now) if spread::within_reach(now, x, z) => {
                let h = terrain::ground(world.seed, &world.haven, x, z);
                spread::depth(now, world.seed, x, z, h)
            }
            Some(_) => spread::Depth::default(),
            None => spread::Depth {
                fel: terrain::fel(world.seed, x, z).max(0.0),
                hot: 0.0,
            },
        }
    }
}

/// Read the spread for this step, and hand it to the ground.
pub fn update(
    world: Option<Res<WorldId>>,
    feed: Res<Feed>,
    net: Option<NonSend<Net>>,
    ring: Option<Res<Ring>>,
    mut ground: ResMut<Assets<GroundMaterial>>,
    mut b: ResMut<Blight>,
) {
    let Some(world) = world else {
        if b.now.is_some() {
            *b = Blight::default();
        }
        return;
    };
    let key = (world.seed, world.blight_salt);
    if b.key != Some(key) {
        b.plan = spread::Plan::new(world.seed, world.blight_salt, &world.haven);
        b.key = Some(key);
        b.now = None;
    }
    // The raw arc clock: `DayPin` pins a capture's hour of the day, and the
    // blight must read the wipe's age.
    let tick = feed.server_tick_est.max(0.0) as u64;
    let tick = tick - tick % STEP_TICKS;
    if let Some(net) = net {
        let arc = &net.session.core.arc;
        if arc.synced() {
            b.tier = arc.fel_tier();
        }
    }
    let now = spread::now(&b.plan, &world.haven, tick, b.tier);
    if b.now != Some(now) {
        b.now = Some(now);
        b.tick = tick;
        b.gen = b.gen.wrapping_add(1);
    }
    if let Some(ring) = ring {
        ring.set_blight(&mut ground, &BlightLive::of(&now));
    }
}

/// The live depth at a cached point (a tree's slot, a map texel), with the
/// heart's wander noise `n` already in hand: the spread now, or the static
/// heart bit for bit when no moment has been read (headless suites).
pub fn live_fel(now: Option<&spread::Now>, x: f32, z: f32, h: f32, n: f32) -> f32 {
    match now {
        Some(now) if spread::within_reach(now, x, z) => spread::depth_n(now, x, z, h, n).fel,
        Some(_) => 0.0,
        None => terrain::fel_of(terrain::fel_d2(x, z), n).max(0.0),
    }
}
