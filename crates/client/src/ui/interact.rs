//! What the player is aiming at, and what `E` therefore does.
//!
//! A port of `web/src/interact.js`'s `resolveInteract`/`promptFor`, kept pure
//! and in `ui/` for this module tree's stated reason: a pick is arithmetic,
//! and arithmetic inside a Bevy system is arithmetic no headless test can
//! call. `crates/client/tests/ui.rs` drives it in the **code** tier.
//!
//! ## Two ranks, and the first always beats the second
//!
//! - **aimed** — the candidate is in FRONT of the player (its projection on
//!   the look direction is positive) and lies within [`AIM_RADIUS_M`] of the
//!   aim line. Among these the nearest wins, which is what a raycast would
//!   answer: a hearth a metre in front of a box is the thing *between* you
//!   and the box, not a worse version of it.
//! - **nearby** — everything else in reach; nearest wins.
//!
//! Nothing is ever excluded for being off-aim. Being off-aim only means
//! losing to something that is not. Standing between a hearth and a box, look
//! at the box and the box wins however much closer the hearth is; turn round
//! and it reverses.
//!
//! **The `t > 0` half of the aimed test is load-bearing**, and the browser
//! learned it by probing rather than by reasoning: without it a hearth whose
//! cell centre is exactly under the player's feet sits at distance zero from
//! the aim line and trumps every box in the room.
//!
//! ## Reach is not a client knob
//!
//! [`REACH_M`] is `sim_core::build::BUILD_REACH_M` by import, because the sim
//! gates a door use, a hearth feed, a box open (`deploy.rs`) and a bag loot
//! (`backpack.rs`, which aliases it `LOOT_REACH_M`) on exactly that number.
//! Picking a target outside it costs a round trip and a refusal — the
//! quantize-both-sides law (`CLAUDE.md`) applied to reach.

use protocol::event::{ItemCatalog, WireBag};
use sim_core::backpack::LOOT_REACH_M;
use sim_core::deploy::{
    box_key, DeployContent, DeployRec, ARCH_BAG, ARCH_BOX, ARCH_DOOR, ARCH_FIRE, ARCH_FURNACE,
    ARCH_GARAGE_DOOR, ARCH_HEARTH, ARCH_RECYCLER, ARCH_RESEARCH, ARCH_WORKBENCH, ARCH_WORKBENCH2,
    ARCH_WORKBENCH3,
};
use sim_core::footprint::Rect;
use sim_core::movement::POS_XZ_Q;

pub use sim_core::build::BUILD_REACH_M as REACH_M;

/// How far off the aim line a thing may sit and still count as aimed at.
///
/// PROPOSED — `DECISIONS.md` §open ("interact aim radius v0"), carried over
/// from the browser resolver, which is the one number this pick needed that
/// no doc and no Rust constant already fixed. 1.0 m is the deployable's own
/// scale: a box and a hearth are roughly a metre across in a 3 m build cell,
/// so a crosshair within a metre of the centre is a crosshair on the thing.
pub const AIM_RADIUS_M: f32 = 1.0;

/// What `E` would do. `None` never carries a prompt.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Verb {
    #[default]
    None,
    Door,
    Bag,
    Box,
    Hearth,
    /// A fire or a furnace — one verb for both, because they are one
    /// thing in the sim (`sim-core/oven.rs`) and the prompt names a KIND.
    Fire,
    /// A recycler. The same sim class as the two above and deliberately
    /// **not** the same verb: the prompt names a KIND, and "FIRE" over a
    /// machine that burns nothing would teach the wrong noun for the
    /// second key. What it shares with `Fire` is the shape of the
    /// interaction — a container with a switch — and that shows up as the
    /// two arms agreeing everywhere the code asks a question about
    /// containers, never as one variant doing both jobs.
    Recycler,
    /// A planter box (crops v0): a container that grows. `E` opens it.
    Planter,
    /// A research table (research v0). The one verb here that acts on what
    /// is in your HAND rather than on what is at the address — the table
    /// holds nothing — so the prompt names the held item and `E` spends it.
    Research,
    /// An authored world container — the haven pad's crate, a waystation's
    /// cache (`sim-core/worldcont.rs`).
    ///
    /// **The one verb on this enum that is not a deployable.** Every other
    /// variant is born from an `ARCH_*` on a record the deploy sync sent;
    /// this one is born from a terrain `Occupant`, which terrain places as
    /// a pure function of the seed and no packet ever mentions. So it is
    /// resolved by the *second* resolver (`resolve_open`, beside
    /// `resolve_swing`) and folded into the pick by the caller — the
    /// deploy scan cannot see it, because there is nothing there to see.
    ///
    /// One variant for both kinds, `Fire`'s argument exactly: a crate and
    /// a cache are one thing in the sim, differing only in the loot table
    /// their occupant selects, and the prompt names a KIND. What the
    /// player is told apart is what is inside.
    Crate,
    /// A loose stack lying on the ground (`sim-core/grounditem.rs`) — what
    /// a smashed barrel left behind.
    ///
    /// **The second verb here that is not a deployable**, `Crate`'s
    /// company, and it is resolved by a third scan for the same reason:
    /// a loose stack is neither a record the deploy sync sent nor a
    /// terrain occupant, it is a set the server states outright
    /// (`ClientCore::ground_items`).
    ///
    /// ⚠ **And it is the one verb whose pick is NOT aim-weighted.** Every
    /// other prompt here takes the thing you are looking at; this one
    /// takes the nearest in reach, because that is what the sim's own
    /// `take_nearest` does — and two stacks out of one barrel land a
    /// metre apart, so an aimed prompt would routinely name the far sack
    /// and hand you the near one. A prompt that can promise what the key
    /// will do is worth more than one that follows the crosshair.
    Take,
    /// A workbench, any rung (tech tree v0). One verb for three
    /// archetypes, `Fire`'s argument again: the prompt names a KIND, and
    /// which level you are standing at is the sim's business — the tree
    /// panel `E` opens shows every node and the server refuses a rung
    /// you have not built. Before this verb a workbench was pure
    /// proximity token: the first deployable a player could place and
    /// not press.
    TechTree,
    Assist,
    /// A kiosk in the town (THE GATE, `sim_core::vend`): `handle` is the
    /// vendor index. Resolved by nearness like `Take`, from the town's own
    /// layout — there is no record on the wire to aim at.
    Trade,
    /// A card reader or an exit lever at the Black Ziggurat
    /// (`sim_core::monument`): `handle` is the door, `lit` true for the
    /// lever. Resolved by nearness, like `Trade`.
    Swipe,
    /// A berry bush or hemp, picked by hand (`sim_core::gather::pick`):
    /// `handle` is its cell key and `occupant` says which. Resolved by
    /// [`resolve_pick`], beside `Crate`'s [`resolve_open`], and folded into
    /// the pick the same way.
    Pick,
    /// A work's terminal (`sim_core::works`, `ARC.md` F1): `handle` is the
    /// work. Resolved by nearness from the work's own spot (`sim_core::spot`),
    /// like `Trade` — the terminal is a place you walk up to.
    Work,
    /// A speaker (`sim_core::lore`): `handle` is the speaker. By nearness.
    Talk,
    /// An inscription: `handle` is the stone. By nearness.
    Read,
    /// A mechanism's dial (`sim_core::mech`): `handle` is mechanism << 8 |
    /// dial, `item` the notch it shows. By nearness, at the sim's dial reach.
    Turn,
}

impl Verb {
    /// The tiebreak order, and the only place the browser chain's ordering
    /// survives.
    ///
    /// Two candidates can score exactly equal — two boxes placed symmetrically
    /// about the aim ray is the ordinary case, and the floats compare exactly
    /// because both sides came out of the same arithmetic. The pick still has
    /// to be a function of its inputs alone, or the prompt drawn on one frame
    /// and the verb run on the keypress could differ while the world stood
    /// still. So a dead tie falls back to the order `E` has always used: a
    /// door is aimed at, a bag is stood on, a box is the durable one, and a
    /// hearth is what you meant if none of those is there.
    fn tie(self) -> u8 {
        match self {
            Verb::None => 0,
            Verb::Door => 1,
            Verb::Bag => 2,
            Verb::Box => 3,
            Verb::Hearth => 4,
            // Last, and it costs nothing to say why: an oven stands on
            // the plane like a box, so a dead tie between the two is a
            // box and an oven in one cell, which placement already
            // forbids. The rung exists so the order is total, not because
            // anything can reach it.
            Verb::Fire => 5,
            Verb::Recycler => 6,
            Verb::Research => 7,
            // Last, and unlike `Fire`'s rung this one is genuinely
            // unreachable: a world container is not a deployable, so it
            // cannot tie with one — the two resolvers scan different
            // things and the caller only consults this one when the
            // deploy scan came back `None`. The rung exists so the order
            // stays total.
            Verb::Crate => 8,
            Verb::TechTree => 9,
            // Last, and unreachable for `Crate`'s reason twice over: a
            // loose stack is neither a deployable nor an occupant, so it
            // cannot tie with anything this function orders. The rung
            // exists so the order stays total.
            Verb::Take => 10,
            Verb::Assist => 11,
            Verb::Trade => 12,
            Verb::Swipe => 13,
            Verb::Pick => 14,
            Verb::Work => 15,
            Verb::Talk => 16,
            Verb::Read => 17,
            Verb::Turn => 18,
            Verb::Planter => 19,
        }
    }

    /// The noun the prompt names. Generic kinds only — `CONTENT.md` owns
    /// item names and these four name a KIND of thing.
    pub fn label(self) -> &'static str {
        match self {
            Verb::None => "",
            Verb::Door => "DOOR",
            Verb::Bag => "BACKPACK",
            Verb::Box => "BOX",
            Verb::Hearth => "HEARTH",
            Verb::Fire => "FIRE",
            Verb::Recycler => "RECYCLER",
            Verb::Research => "RESEARCH TABLE",
            Verb::Crate => "CRATE",
            Verb::TechTree => "WORKBENCH",
            // The KIND, for the same reason as every other row — and the
            // prompt overrides it with the item's own name, which is the
            // one place a generic word is not enough (the sack on the
            // ground is the same mesh whatever is in it).
            Verb::Take => "ITEM",
            Verb::Assist => "WOUNDED PLAYER",
            Verb::Trade => "VENDOR",
            Verb::Swipe => "CARD READER",
            Verb::Pick => "PLANT",
            Verb::Work => "WORK",
            Verb::Talk => "SPEAKER",
            Verb::Read => "INSCRIPTION",
            Verb::Turn => "DIAL",
            Verb::Planter => "PLANTER",
        }
    }
}

/// One resolved pick.
#[derive(Clone, Copy, Debug, Default)]
pub struct Pick {
    pub verb: Verb,
    /// A bag that is a killed animal's carcass names the species
    /// (`WireBag::species`, v84), so the prompt says what is lying there.
    pub species: Option<u8>,
    /// The archetype of the record this pick resolved, exactly as the
    /// deploy sync named it (`ARCH_BAG` for a bag, which arrives on its
    /// own lane and has no deploy record). Carried so the access verb can
    /// ask `sim_core::deploy::lockable` — the sim's own predicate — rather
    /// than matching verbs, which would be a second copy of the lockable
    /// set waiting to drift (`ui::keypad::lock_target`).
    pub arch: u8,
    /// The container handle: a bag id, or a box's packed `box_key`.
    ///
    /// `Verb::Take` puts the loose stack's id here, which is **not** sent
    /// anywhere: the take is payload-free and the sim picks (`grounditem.rs`).
    /// It is carried so the HUD can tell one stack from another between
    /// frames without re-resolving.
    pub handle: u32,
    /// What a `Verb::Take` pick is a stack OF, and how many. Zero for
    /// every other verb.
    ///
    /// On the `Pick` rather than looked up by the HUD because the prompt
    /// is composed in exactly one place, which is this file's rule for
    /// every other dynamic word in a prompt (`open`, `lit`, `locked`).
    pub item: u16,
    pub count: u16,
    /// What a `Verb::Pick` is picking — the terrain occupant ordinal, so the
    /// prompt can say berries or hemp. Zero for every other verb.
    pub occupant: u8,
    pub cx: u16,
    pub cz: u16,
    pub level: u8,
    pub loc: u8,
    /// Door state, straight off the wire — all three bits (lock v1).
    /// `has_lock` is what lets the prompt tell "bare" from "unlocked",
    /// which are the same to `E` and completely different to `L`.
    pub open: bool,
    pub locked: bool,
    pub has_lock: bool,
    /// Fire state, and the one field of a pick the resolver does not
    /// fill: whether a fire is burning lives in `ClientCore`'s own lit
    /// set rather than on the mirrored deploy record (`core.rs` says
    /// why), so the caller stamps it after resolving. False on every
    /// other verb and on a fire nobody has heard about yet, which reads
    /// as "out" — the honest default, since an unheard fire is one this
    /// client has no news of.
    pub lit: bool,
    /// THE GATE's own station (`town::STATIONS`): anyone's to use, nobody's
    /// to take, and a recycler that pays the safe zone's share. Stamped by
    /// the caller, as `lit` is, from [`town_station`].
    pub public: bool,
    /// Squared distance from the player, and from the aim line. Diagnostics
    /// for the gate; nothing draws them.
    pub d2: f32,
    pub perp2: f32,
    /// True when the pick was AIMED at rather than merely nearest in reach.
    pub aimed: bool,
}

impl Pick {
    pub fn is_none(&self) -> bool {
        self.verb == Verb::None
    }

    /// What the centre prompt says, or `""` for nothing in reach.
    ///
    /// The key is named in the text because that is the whole job: the
    /// reference genre's centre-screen hint is how a player learns the island
    /// has verbs at all. A door additionally reports the two bits of state the
    /// wire carries — `open` says whether `E` closes or opens it, and `locked`
    /// is stated without claiming the press will fail, because the wire
    /// carries the lock bit but never the owner and only the server knows
    /// whether this door is yours.
    /// The line under the crosshair.
    ///
    /// **`catalog` arrived with `Verb::Take`** (ground items v0) and it is
    /// the first dynamic *word* any prompt here needed rather than a
    /// dynamic state: every other row names a KIND, which is a constant,
    /// but a sack on the ground is one mesh whatever is inside it — so the
    /// item's own name is the only thing that tells a player whether to
    /// stop. `ItemCatalog::EMPTY` is a legitimate argument: a name that
    /// has not dripped in yet reads as `#id`, exactly as it does in the
    /// inventory panel, rather than making the prompt disappear.
    pub fn prompt(&self, catalog: &ItemCatalog) -> String {
        match self.verb {
            Verb::None => String::new(),
            Verb::Assist => format!(
                "HOLD [E] HELP UP · {} s · STAY STILL",
                sim_core::assist::ASSIST_TICKS / sim_core::limits::TICK_HZ as u16
            ),
            Verb::Door => format!(
                "[E] {} DOOR{}",
                if self.open { "CLOSE" } else { "OPEN" },
                // Three states, three sentences (lock v1): a bare door
                // says nothing extra, a bolted-but-open one advertises
                // its keypad, and a shut one says so. Naming `[L]` on
                // the middle case is the only place a player learns the
                // key exists.
                match (self.has_lock, self.locked) {
                    (false, _) => "",
                    (true, false) => "  ·  [L] KEYPAD",
                    (true, true) => "  ·  LOCKED  ·  [L] KEYPAD",
                }
            ),
            // The box borrows the door's whole lock grammar (locks on
            // boxes, `DOORS.md` §9.8): a bare lid says nothing extra, a
            // bolted one advertises its keypad, an armed one says LOCKED
            // too. `E` stays the open — whether it succeeds against a
            // locked lid is the sim's verdict, never this line's.
            Verb::Box => format!(
                "[E] OPEN BOX{}",
                match (self.has_lock, self.locked) {
                    (false, _) => "",
                    (true, false) => "  ·  [L] KEYPAD",
                    (true, true) => "  ·  LOCKED  ·  [L] KEYPAD",
                }
            ),
            // The crew keys ride the hearth's prompt because there is
            // nowhere else a player would look for them, and `L` is
            // already the access key at a door (hearth crew v1). A lock
            // bolted on turns `L` into its keypad (hearth lock v0): the
            // code is how the crew invites a hand, so JOIN would be the
            // prompt naming a press the pad answers instead.
            Verb::Hearth => format!(
                "[E] FEED HEARTH  ·  [L] {}  ·  [K] LEAVE  ·  [SHIFT+K] CLEAR CREW",
                if self.has_lock { "KEYPAD" } else { "JOIN CREW" }
            ),
            // Two verbs on one thing, and both named: the panel is where
            // the wood goes and `C` is the match. The state is stated the
            // way a door's is, because it is the same question — which
            // way the second key will move it.
            // A furnace shares the verb and is named for what it is.
            Verb::Fire => format!(
                "[E] OPEN {}  ·  [C] {}",
                if self.arch == ARCH_FURNACE {
                    "FURNACE"
                } else {
                    "FIRE"
                },
                if self.lit { "PUT OUT" } else { "LIGHT" }
            ),
            // The same two keys and a different pair of words, because a
            // recycler is switched rather than lit — "LIGHT" over a
            // machine with no fire in it is the prompt lying about the
            // mechanism.
            Verb::Recycler => format!(
                "[E] OPEN {}  ·  [C] {}",
                if self.public {
                    // Rust's safe-zone recycler: it pays less than your own.
                    format!("PUBLIC RECYCLER ({}%)", sim_core::town::PUBLIC_RECYCLE_PCT)
                } else {
                    "RECYCLER".to_string()
                },
                if self.lit { "STOP" } else { "START" }
            ),
            // The recycler's two keys (research table v1): the table is a
            // container now, so `E` opens it and `C` is the switch that
            // starts a research — and says RESEARCHING while one runs,
            // because the press would then be refused and the prompt should
            // not offer it.
            Verb::Research => format!(
                "[E] OPEN {}RESEARCH TABLE  ·  [C] {}",
                if self.public { "PUBLIC " } else { "" },
                if self.lit { "RESEARCHING" } else { "BEGIN" }
            ),
            // "TECH TREE", not "OPEN WORKBENCH": the bench holds nothing
            // and opens nothing — what `E` does here is show the tree
            // (tech tree v0), so the prompt names the thing you get.
            Verb::TechTree if self.public => "[E] TECH TREE  ·  PUBLIC WORKBENCH".to_string(),
            Verb::TechTree => "[E] TECH TREE".to_string(),
            Verb::Pick => match self.occupant {
                o if o == Occupant::Hemp as u8 => "[E] PICK HEMP",
                o if o == Occupant::StonePile as u8 => "[E] PICK UP STONE",
                o if o == Occupant::WoodPile as u8 => "[E] PICK UP WOOD",
                o if o == Occupant::MetalPile as u8 => "[E] PICK UP METAL ORE",
                o if o == Occupant::SulfurPile as u8 => "[E] PICK UP SULFUR ORE",
                o if o == Occupant::MushroomPatch as u8 => "[E] PICK MUSHROOMS",
                _ => "[E] PICK BERRIES",
            }
            .to_string(),
            Verb::Trade => "[E] TRADE".to_string(),
            Verb::Work => "[E] THE WORK".to_string(),
            Verb::Talk => "[E] TALK".to_string(),
            Verb::Read => "[E] READ THE STONE".to_string(),
            Verb::Turn => format!(
                "[E] TURN DIAL {} · SHOWS {}",
                (self.handle & 0xFF) + 1,
                self.item
            ),
            Verb::Swipe if self.lit => "[E] OPEN DOOR".to_string(),
            Verb::Swipe => format!(
                "[E] SWIPE {} KEYCARD",
                ["GREEN", "BLUE", "RED"][(self.handle as usize).min(2)]
            ),
            // Not "OPEN": a loose stack has nothing to open, and the
            // count is the half of this line a player acts on — a sack
            // holding 4 cloth and a sack holding 300 metal are the same
            // picture. `×` and the same `item_label` the panels use, so
            // one item has one name everywhere.
            Verb::Take => format!(
                "[E] TAKE {} ×{}",
                crate::ui::craft::item_label(catalog, self.item).to_uppercase(),
                self.count
            ),
            // A blade swung at it cuts more out than `E` pulls (`mobs.toml`
            // `[butcher]`); the prompt teaches the verb rather than knowing
            // which tools have a row, which the catalog does not carry.
            Verb::Bag if self.species.is_some() => format!(
                "[E] LOOT {}  ·  SWING A BLADE TO BUTCHER",
                match self.species {
                    Some(sim_core::mob::MOB_WOLF) => "WOLF",
                    Some(sim_core::mob::MOB_STAG) => "STAG",
                    _ => "PIG",
                }
            ),
            v => format!("[E] OPEN {}", v.label()),
        }
    }
}

/// Whether the deployable at cell (`cx`, `cz`) is one of THE GATE's own
/// stations (`town::STATIONS`, seeded on build-cell centres).
pub fn town_station(town: &sim_core::town::Town, cx: u16, cz: u16) -> bool {
    town.live
        && (0..sim_core::town::STATIONS.len()).any(|k| {
            sim_core::town::station_world(town, k).is_some_and(|(_, x, z)| {
                let cell = sim_core::build::BUILD_CELL_M;
                (x / cell) as u16 == cx && (z / cell) as u16 == cz
            })
        })
}

/// The town kiosk `E` would trade at, or a `None` pick: the nearest counter
/// within the sim's own `VEND_REACH_M`.
pub fn resolve_trade(x: f32, z: f32, town: &sim_core::town::Town) -> Pick {
    if !town.live {
        return Pick::default();
    }
    let mut best: Option<(usize, f32)> = None;
    for k in 0..sim_core::town::KIOSKS.len() {
        let Some((kx, kz)) = sim_core::town::kiosk_world(town, k) else {
            continue;
        };
        let d2 = (kx - x) * (kx - x) + (kz - z) * (kz - z);
        if d2 > sim_core::vend::VEND_REACH_M * sim_core::vend::VEND_REACH_M {
            continue;
        }
        if best.is_none_or(|(_, b)| d2 < b) {
            best = Some((k, d2));
        }
    }
    match best {
        Some((k, d2)) => Pick {
            verb: Verb::Trade,
            handle: k as u32,
            d2,
            ..Pick::default()
        },
        None => Pick::default(),
    }
}

/// The work terminal `E` would open, or a `None` pick: the nearest whose row
/// has arrived, within the sim's own `WORK_REACH_M` and `spot::within`'s
/// storey.
pub fn resolve_work(
    x: f32,
    y: f32,
    z: f32,
    haven: &sim_core::terrain::Haven,
    arc: &client_core::arc::ArcView,
) -> Pick {
    let reach = sim_core::works::WORK_REACH_M;
    let mut best: Option<(usize, f32)> = None;
    for (k, w) in arc.known() {
        if !sim_core::spot::within(haven, &w.def.spot, x, y, z, reach) {
            continue;
        }
        let Some((wx, _, wz)) = sim_core::spot::world(haven, &w.def.spot) else {
            continue;
        };
        let d2 = (wx - x) * (wx - x) + (wz - z) * (wz - z);
        if best.is_none_or(|(_, b)| d2 < b) {
            best = Some((k, d2));
        }
    }
    match best {
        Some((k, d2)) => Pick {
            verb: Verb::Work,
            handle: k as u32,
            d2,
            ..Pick::default()
        },
        None => Pick::default(),
    }
}

/// The nearest of `spots` within `reach`, by the sim's own `spot::within`.
fn nearest_spot(
    x: f32,
    y: f32,
    z: f32,
    haven: &sim_core::terrain::Haven,
    reach: f32,
    spots: impl Iterator<Item = (u32, sim_core::spot::Spot)>,
) -> Option<(u32, f32)> {
    let mut best: Option<(u32, f32)> = None;
    for (handle, spot) in spots {
        if !sim_core::spot::within(haven, &spot, x, y, z, reach) {
            continue;
        }
        let Some((wx, _, wz)) = sim_core::spot::world(haven, &spot) else {
            continue;
        };
        let d2 = (wx - x) * (wx - x) + (wz - z) * (wz - z);
        if best.is_none_or(|(_, b)| d2 < b) {
            best = Some((handle, d2));
        }
    }
    best
}

/// The speaker, stone or dial `E` would use, or a `None` pick: the nearest
/// within the sim's reach for each, a dial winning a tie (it is the
/// smallest thing, and you are standing at it on purpose).
pub fn resolve_lore(
    x: f32,
    y: f32,
    z: f32,
    haven: &sim_core::terrain::Haven,
    lore: &client_core::arc::LoreView,
) -> Pick {
    let dials = lore.known_mechs().flat_map(|(k, m)| {
        let def = sim_core::mech::MechDef {
            spot: m.spot,
            dials: m.n_dials,
            ..Default::default()
        };
        (0..m.n_dials as usize).map(move |d| {
            (
                ((k as u32) << 8) | d as u32,
                sim_core::mech::dial_spot(&def, d),
            )
        })
    });
    if let Some((h, d2)) = nearest_spot(x, y, z, haven, sim_core::mech::DIAL_REACH_M, dials) {
        let m = &lore.mechs[(h >> 8) as usize];
        return Pick {
            verb: Verb::Turn,
            handle: h,
            item: m.dials[(h & 0xFF) as usize] as u16,
            d2,
            ..Pick::default()
        };
    }
    let reach = sim_core::lore::LORE_REACH_M;
    let talk = nearest_spot(
        x,
        y,
        z,
        haven,
        reach,
        lore.known_speakers().map(|(k, s)| (k as u32, s.spot)),
    );
    let read = nearest_spot(
        x,
        y,
        z,
        haven,
        reach,
        lore.known_inscriptions().map(|(k, s)| (k as u32, s.spot)),
    );
    match (talk, read) {
        (Some((h, d2)), r) if r.is_none_or(|(_, rd)| d2 <= rd) => Pick {
            verb: Verb::Talk,
            handle: h,
            d2,
            ..Pick::default()
        },
        (_, Some((h, d2))) => Pick {
            verb: Verb::Read,
            handle: h,
            d2,
            ..Pick::default()
        },
        _ => Pick::default(),
    }
}

/// The ziggurat reader or lever `E` would use, or a `None` pick: the
/// nearest within the sim's own `SWIPE_REACH_M`, on its own floor.
pub fn resolve_swipe(x: f32, y: f32, z: f32, zig: &sim_core::monument::Ziggurat) -> Pick {
    if !zig.live {
        return Pick::default();
    }
    let reach = sim_core::monument::SWIPE_REACH_M;
    let mut best: Option<(usize, bool, f32)> = None;
    for d in 0..sim_core::monument::CARD_DOORS {
        for inside in [false, true] {
            let Some((rx, ry, rz)) = sim_core::monument::reader_world(zig, d, inside) else {
                continue;
            };
            let d2 = (rx - x) * (rx - x) + (rz - z) * (rz - z);
            if d2 > reach * reach || (ry - y).abs() > 2.0 {
                continue;
            }
            if best.is_none_or(|(_, _, b)| d2 < b) {
                best = Some((d, inside, d2));
            }
        }
    }
    match best {
        Some((d, lever, d2)) => Pick {
            verb: Verb::Swipe,
            handle: d as u32,
            lit: lever,
            d2,
            ..Pick::default()
        },
        None => Pick::default(),
    }
}

/// The loose stack `E` would take, or a `None` pick.
///
/// **Nearest-in-reach, and deliberately blind to the crosshair** — the one
/// resolver here that does not weight by aim, because
/// `grounditem::take_nearest` does not either. The rule it has to satisfy
/// is *the prompt names what the key will take*, and two stacks out of one
/// barrel land inside a metre of each other: an aimed pick would put
/// `TAKE WOOD ×20` on screen and hand over the stone at your feet.
///
/// Reach is the sim's own `LOOT_REACH_M` — the arm a bag, a box, a door
/// and a hearth feed already share — read from `sim_core` rather than
/// restated, so the prompt cannot promise a take the sim will refuse for
/// distance. Ties go to the lower index, which is `nearest`'s rule too.
pub fn resolve_take(x: f32, z: f32, items: &[protocol::event::WireGItem]) -> Pick {
    let mut best: Option<(usize, f32)> = None;
    for (i, g) in items.iter().enumerate() {
        let dx = g.qx as f32 * POS_XZ_Q - x;
        let dz = g.qz as f32 * POS_XZ_Q - z;
        let d2 = dx * dx + dz * dz;
        if d2 > LOOT_REACH_M * LOOT_REACH_M {
            continue;
        }
        match best {
            Some((_, bd2)) if bd2 <= d2 => {}
            _ => best = Some((i, d2)),
        }
    }
    match best {
        Some((i, d2)) => Pick {
            verb: Verb::Take,
            handle: items[i].id,
            item: items[i].item,
            count: items[i].count,
            d2,
            ..Pick::default()
        },
        None => Pick::default(),
    }
}

/// What `E` takes once an arrow standing in a body may be in reach too: the
/// nearer of [`resolve_take`]'s loose stack and that arrow, ties to the
/// loose stack — the sim's own rule (`World::pull_arrow`). `body_at` puts a
/// body's feet in world XZ, or `None` for one this client is not drawing;
/// the player's own body stands where they do.
pub fn resolve_take_or_pull(
    x: f32,
    z: f32,
    items: &[protocol::event::WireGItem],
    lodged: &[protocol::WireLodged],
    mut body_at: impl FnMut(u32) -> Option<(f32, f32)>,
) -> Pick {
    let take = resolve_take(x, z, items);
    let take_d2 = if take.verb == Verb::Take {
        take.d2
    } else {
        f32::INFINITY
    };
    let mut best: Option<(u16, f32)> = None;
    for a in lodged {
        let Some((bx, bz)) = body_at(a.host) else {
            continue;
        };
        let (dx, dz) = (bx - x, bz - z);
        let d2 = dx * dx + dz * dz;
        if d2 <= LOOT_REACH_M * LOOT_REACH_M && best.is_none_or(|(_, b)| d2 < b) {
            best = Some((a.item, d2));
        }
    }
    match best {
        Some((item, d2)) if d2 < take_d2 => Pick {
            verb: Verb::Take,
            item,
            count: 1,
            d2,
            ..Pick::default()
        },
        _ => take,
    }
}

/// Where the player is and where they are looking, in world XZ.
#[derive(Clone, Copy, Debug)]
pub struct Aim<'a> {
    pub x: f32,
    pub z: f32,
    pub fx: f32,
    pub fz: f32,
    pub reach: f32,
    pub radius: f32,
    /// The eye's own ray, when the caller has one: a deployable placed
    /// freely is then picked only when the crosshair is ON it ([`Sight`]).
    pub sight: Option<Sight<'a>>,
}

impl Aim<'_> {
    /// The ordinary aim: feet position, facing, and both defaults.
    pub fn new(x: f32, z: f32, fx: f32, fz: f32) -> Self {
        Self {
            x,
            z,
            fx,
            fz,
            reach: REACH_M,
            radius: AIM_RADIUS_M,
            sight: None,
        }
    }
}

/// What the eye looks along, for the deployables `E` has to be LOOKED AT
/// rather than merely stood near — every body deployable, since free
/// placement put them anywhere in a cell. The reference genre's prompt is a
/// raycast: the crosshair on the box opens the box, and the crosshair on the
/// ground beside a fire does not offer the fire (the 2026-10-02 playtest
/// read that as the fire being somewhere other than where it was drawn).
#[derive(Clone, Copy)]
pub struct Sight<'a> {
    pub eye: [f32; 3],
    /// Unit look direction.
    pub dir: [f32; 3],
    /// `(bottom, top)` of a body deployable as the renderer stands it — its
    /// floor (`sim_core::deploy::body_base_y`) and that plus its drawn
    /// height. The ui tier holds no render table, so the caller lends it.
    pub span: &'a dyn Fn(&DeployRec, u8) -> (f32, f32),
}

impl core::fmt::Debug for Sight<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Sight")
            .field("eye", &self.eye)
            .field("dir", &self.dir)
            .finish_non_exhaustive()
    }
}

/// How far past a deployable's own box the look ray still counts as on it,
/// metres — a little forgiveness on a bedroll a third of a metre tall.
pub const SIGHT_SLACK_M: f32 = 0.1;

/// Where along the look ray the eye first meets the box `rect × [bottom,
/// top]` (grown by [`SIGHT_SLACK_M`]), or `None`. The ray goes into the
/// rectangle's own frame and meets three slabs — the turned box tested as a
/// box, the way the sim's collision walks test it.
pub fn sight_hit(s: &Sight<'_>, rect: &Rect, bottom: f32, top: f32) -> Option<f32> {
    let m = SIGHT_SLACK_M;
    let (ox, oz) = rect.local(s.eye[0], s.eye[2]);
    let dx = s.dir[0] * rect.c - s.dir[2] * rect.s;
    let dz = s.dir[0] * rect.s + s.dir[2] * rect.c;
    let mut t0 = 0.0f32;
    let mut t1 = f32::INFINITY;
    for (o, d, lo, hi) in [
        (ox, dx, -rect.hw - m, rect.hw + m),
        (s.eye[1], s.dir[1], bottom - m, top + m),
        (oz, dz, -rect.hd - m, rect.hd + m),
    ] {
        if d.abs() < 1e-9 {
            if o < lo || o > hi {
                return None;
            }
            continue;
        }
        let (a, b) = ((lo - o) / d, (hi - o) / d);
        let (a, b) = if a < b { (a, b) } else { (b, a) };
        t0 = t0.max(a);
        t1 = t1.min(b);
        if t0 > t1 {
            return None;
        }
    }
    Some(t0)
}

/// Best-so-far, as scalars rather than a candidate struct, so the sweep
/// allocates nothing.
struct Best {
    aimed: bool,
    d2: f32,
    tie: u8,
}

impl Best {
    /// Score one candidate at world XZ; answer whether it takes the lead,
    /// filling the shared fields of `out` if it does.
    #[allow(clippy::too_many_arguments)]
    fn wins(
        &mut self,
        out: &mut Pick,
        aim: &Aim<'_>,
        f: (f32, f32),
        verb: Verb,
        x: f32,
        z: f32,
        seen: bool,
    ) -> bool {
        let (dx, dz) = (x - aim.x, z - aim.z);
        let d2 = dx * dx + dz * dz;
        if d2 > aim.reach * aim.reach {
            return false; // out of the server's reach
        }
        // Projection onto the look direction. Positive means in front; the
        // perpendicular offset is only meaningful there, which is why `aimed`
        // requires it. A deployable the eye's ray actually meets (`seen`) is
        // aimed at by definition.
        let t = dx * f.0 + dz * f.1;
        let (px, pz) = (dx - t * f.0, dz - t * f.1);
        let perp2 = px * px + pz * pz;
        let aimed = seen || (t > 0.0 && perp2 <= aim.radius * aim.radius);
        let tie = verb.tie();
        // Rank first, then distance inside the rank, then the old order.
        if self.aimed && !aimed {
            return false;
        }
        if aimed == self.aimed {
            if d2 > self.d2 {
                return false;
            }
            if d2 == self.d2 && tie >= self.tie {
                return false;
            }
        }
        self.aimed = aimed;
        self.d2 = d2;
        self.tie = tie;
        out.verb = verb;
        out.d2 = d2;
        out.perp2 = perp2;
        out.aimed = aimed;
        // Only a bag's branch names a species; any other winner clears it.
        out.species = None;
        true
    }
}

/// Resolve what `E` is pointed at.
///
/// `have` is `ClientCore::deploy_defs_have`: a row past it has not dripped in
/// and its archetype is unknown, so it is skipped rather than read as
/// `DeployDef::INERT`'s bag — offering a verb on a guess is how a prompt ends
/// up naming a thing that is not there.
pub fn resolve(
    aim: Aim<'_>,
    deploys: &[DeployRec],
    defs: &DeployContent,
    have: u16,
    bags: &[WireBag],
) -> Pick {
    let mut out = Pick::default();
    let mut best = Best {
        aimed: false,
        d2: f32::INFINITY,
        tie: u8::MAX,
    };
    // Normalise once. A zero-length look direction is not a reason to refuse
    // to answer — the segment collapses to the player's own position and the
    // metric degrades to plain nearest-wins.
    let flen = (aim.fx * aim.fx + aim.fz * aim.fz).sqrt();
    let f = if flen > 0.0 {
        (aim.fx / flen, aim.fz / flen)
    } else {
        (0.0, 0.0)
    };
    for rec in deploys {
        if (rec.row as u16) >= have {
            continue;
        }
        let arch = defs.defs[rec.row as usize].arch;
        let verb = match arch {
            ARCH_DOOR | ARCH_GARAGE_DOOR | sim_core::deploy::ARCH_WINDOW_SHUTTER => Verb::Door,
            ARCH_BOX => Verb::Box,
            ARCH_HEARTH => Verb::Hearth,
            ARCH_FIRE | ARCH_FURNACE => Verb::Fire,
            ARCH_RECYCLER => Verb::Recycler,
            sim_core::deploy::ARCH_PLANTER => Verb::Planter,
            ARCH_RESEARCH => Verb::Research,
            ARCH_WORKBENCH | ARCH_WORKBENCH2 | ARCH_WORKBENCH3 => Verb::TechTree,
            _ => continue,
        };
        // A box is addressed by its packed cell. `box_key(0, 0, 0, 0)` is 0 and
        // 0 is the reserved "no container" handle, which is why the SIM
        // refuses to place a box there (`deploy.rs`). A record carrying it is
        // one this client should not have, so it is not offered at all —
        // better than a prompt for a box the key would decline to open.
        let mut handle = 0u32;
        if verb == Verb::Box
            || verb == Verb::Fire
            || verb == Verb::Recycler
            || verb == Verb::Planter
            || verb == Verb::Research
        {
            handle = box_key(rec.cx, rec.cz, rec.level, rec.loc);
            if handle == 0 {
                continue;
            }
        }
        // Reach is measured to where the thing stands — its own centre for
        // a deployable placed freely, the cell centre for a door (its pose
        // is the centre) — which is the metric `deploy.rs`'s
        // `box_in_reach` and the door's own gate use.
        let (x, z) = rec.xz();
        // A door is aimed at where it hangs, its edge's middle: two doors
        // share a cell (an airlock's front and inner door), and scored at
        // the centre they tie exactly, so the second could never be picked.
        let (x, z) = if verb == Verb::Door {
            let (dx, dz) = (x - aim.x, z - aim.z);
            if dx * dx + dz * dz > aim.reach * aim.reach {
                continue;
            }
            sim_core::build::anchor(rec.cx, rec.cz, rec.loc)
        } else {
            (x, z)
        };
        // Anything placed freely is picked by looking at it, when there is
        // an eye to look with: the box the ray meets, not the nearest box.
        let mut seen = false;
        if verb != Verb::Door {
            if let Some(s) = aim.sight.as_ref() {
                let Some(rect) = rec.rect(arch) else {
                    continue;
                };
                let (bottom, top) = (s.span)(rec, arch);
                if sight_hit(s, &rect, bottom, top).is_none() {
                    continue;
                }
                seen = true;
            }
        }
        if !best.wins(&mut out, &aim, f, verb, x, z, seen) {
            continue;
        }
        out.arch = arch;
        out.handle = handle;
        out.cx = rec.cx;
        out.cz = rec.cz;
        out.level = rec.level;
        out.loc = rec.loc;
        out.open = rec.open;
        out.locked = rec.locked;
        out.has_lock = rec.has_lock;
    }

    for bag in bags {
        // A bag carries a world position, not a grid cell — it is dropped
        // where its owner died.
        let x = bag.qx as f32 * sim_core::movement::POS_XZ_Q;
        let z = bag.qz as f32 * sim_core::movement::POS_XZ_Q;
        if !best.wins(&mut out, &aim, f, Verb::Bag, x, z, false) {
            continue;
        }
        out.arch = ARCH_BAG;
        out.handle = bag.id;
        out.species = bag.species();
        out.cx = 0;
        out.cz = 0;
        out.level = 0;
        out.loc = 0;
        out.open = false;
        out.locked = false;
        out.has_lock = false;
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim_core::build::BUILD_CELL_M;
    use sim_core::deploy::{DeployDef, ARCH_BAG};

    fn defs_with(arches: &[u8]) -> (DeployContent, u16) {
        let mut d = DeployContent::EMPTY;
        for (i, &arch) in arches.iter().enumerate() {
            d.defs[i] = DeployDef {
                arch,
                ..DeployDef::INERT
            };
        }
        d.def_count = arches.len() as u16;
        (d, arches.len() as u16)
    }

    fn rec(cx: u16, cz: u16, row: u8) -> DeployRec {
        DeployRec {
            cx,
            cz,
            level: 0,
            loc: 0,
            pose: Default::default(),
            row,
            owner: 1,
            hp: 100,
            uh: 0,
            open: false,
            locked: false,
            has_lock: false,
            dmg: 0,
        }
    }

    /// Cell centre of (cx, cz), so a test can stand exactly somewhere.
    fn centre(cx: u16, cz: u16) -> (f32, f32) {
        (
            cx as f32 * BUILD_CELL_M + BUILD_CELL_M * 0.5,
            cz as f32 * BUILD_CELL_M + BUILD_CELL_M * 0.5,
        )
    }

    #[test]
    fn nothing_in_reach_is_no_verb_and_no_prompt() {
        let (defs, have) = defs_with(&[ARCH_BOX]);
        let far = [rec(100, 100, 0)];
        let p = resolve(Aim::new(0.0, 0.0, 0.0, 1.0), &far, &defs, have, &[]);
        assert!(p.is_none());
        assert_eq!(p.prompt(&ItemCatalog::EMPTY), "");
    }

    /// An airlock hangs two doors in one cell, on two of its edges: each is
    /// picked by looking at it, from inside the cell or from either side.
    #[test]
    fn two_doors_in_one_cell_are_each_picked_by_looking_at_them() {
        use sim_core::build::{LOC_EDGE_XLO, LOC_EDGE_ZLO};
        let (defs, have) = defs_with(&[ARCH_DOOR]);
        let front = DeployRec {
            loc: LOC_EDGE_XLO,
            ..rec(5, 5, 0)
        };
        let inner = DeployRec {
            loc: LOC_EDGE_ZLO,
            ..rec(5, 5, 0)
        };
        let (x0, z0) = (5.0 * BUILD_CELL_M, 5.0 * BUILD_CELL_M);
        for recs in [[front, inner], [inner, front]] {
            // Inside the cell, facing west at the front door, then north
            // at the inner one.
            let (x, z) = (x0 + 0.65, z0 + 0.65);
            let p = resolve(Aim::new(x, z, -1.0, 0.8), &recs, &defs, have, &[]);
            assert_eq!((p.verb, p.loc), (Verb::Door, LOC_EDGE_XLO));
            let p = resolve(Aim::new(x, z, 0.8, -1.0), &recs, &defs, have, &[]);
            assert_eq!((p.verb, p.loc), (Verb::Door, LOC_EDGE_ZLO));
            // From outside each, facing it.
            let p = resolve(
                Aim::new(x0 - 1.0, z0 + 1.5, 1.0, 0.0),
                &recs,
                &defs,
                have,
                &[],
            );
            assert_eq!(p.loc, LOC_EDGE_XLO);
            let p = resolve(
                Aim::new(x0 + 1.5, z0 - 0.65, 0.0, 1.0),
                &recs,
                &defs,
                have,
                &[],
            );
            assert_eq!(p.loc, LOC_EDGE_ZLO);
        }
    }

    /// The judge's case, and the whole reason for two ranks: standing between
    /// a hearth and a box, whichever you LOOK at wins.
    #[test]
    fn aim_beats_proximity() {
        let (defs, have) = defs_with(&[ARCH_HEARTH, ARCH_BOX]);
        // Hearth one cell north, box two cells south. Stand between them.
        let recs = [rec(0, 1, 0), rec(0, 3, 1)];
        let (x, _) = centre(0, 2);
        let (_, hz) = centre(0, 1);
        let (_, bz) = centre(0, 3);
        let me = (x, (hz + bz) * 0.5);

        let north = resolve(Aim::new(me.0, me.1, 0.0, -1.0), &recs, &defs, have, &[]);
        assert_eq!(north.verb, Verb::Hearth, "looking north");
        let south = resolve(Aim::new(me.0, me.1, 0.0, 1.0), &recs, &defs, have, &[]);
        assert_eq!(south.verb, Verb::Box, "looking south");
    }

    /// A thing behind you with nothing else around still resolves — off-aim
    /// only ever means losing to something that is not off-aim.
    #[test]
    fn a_lone_target_behind_you_still_resolves() {
        let (defs, have) = defs_with(&[ARCH_DOOR]);
        let recs = [rec(0, 0, 0)];
        let (cx, cz) = centre(0, 0);
        let p = resolve(Aim::new(cx, cz + 2.0, 0.0, 1.0), &recs, &defs, have, &[]);
        assert_eq!(p.verb, Verb::Door);
        assert!(!p.aimed, "it is behind us, so it is nearby and not aimed");
    }

    /// The `t > 0` guard: a hearth underfoot must not trump a box you are
    /// looking straight at.
    #[test]
    fn a_target_underfoot_does_not_trump_the_one_you_face() {
        let (defs, have) = defs_with(&[ARCH_HEARTH, ARCH_BOX]);
        let recs = [rec(0, 0, 0), rec(0, 1, 1)];
        let (cx, cz) = centre(0, 0);
        // Standing exactly on the hearth's centre, facing the box.
        let p = resolve(Aim::new(cx, cz, 0.0, 1.0), &recs, &defs, have, &[]);
        assert_eq!(p.verb, Verb::Box);
        assert!(p.aimed);
    }

    #[test]
    fn a_bag_resolves_by_world_position() {
        let (defs, have) = defs_with(&[ARCH_BAG]);
        let bag = WireBag {
            id: 77,
            qx: (2.0 / sim_core::movement::POS_XZ_Q) as i32,
            qy: 0,
            qz: 0,
            kind: 0,
        };
        let p = resolve(Aim::new(0.0, 0.0, 1.0, 0.0), &[], &defs, have, &[bag]);
        assert_eq!(p.verb, Verb::Bag);
        assert_eq!(p.handle, 77);
        assert!(p.aimed);
    }

    /// A killed animal's bag is its carcass (v84): the prompt names it.
    #[test]
    fn a_carcass_prompts_to_loot_the_animal() {
        let (defs, have) = defs_with(&[ARCH_BAG]);
        let at = |kind| WireBag {
            id: 5,
            qx: (2.0 / sim_core::movement::POS_XZ_Q) as i32,
            qy: 0,
            qz: 0,
            kind,
        };
        let prompt = |kind| {
            resolve(Aim::new(0.0, 0.0, 1.0, 0.0), &[], &defs, have, &[at(kind)])
                .prompt(&ItemCatalog::EMPTY)
        };
        assert_eq!(prompt(0), "[E] OPEN BACKPACK");
        assert_eq!(
            prompt(1 + sim_core::mob::MOB_PIG),
            "[E] LOOT PIG  ·  SWING A BLADE TO BUTCHER"
        );
        assert_eq!(
            prompt(1 + sim_core::mob::MOB_WOLF),
            "[E] LOOT WOLF  ·  SWING A BLADE TO BUTCHER"
        );
    }

    /// Out past `BUILD_REACH_M` is the server's refusal, so the client does
    /// not offer it.
    #[test]
    fn reach_is_the_sims_reach() {
        let (defs, have) = defs_with(&[ARCH_BAG]);
        let just_past = REACH_M + 0.5;
        let bag = WireBag {
            id: 1,
            qx: (just_past / sim_core::movement::POS_XZ_Q) as i32,
            qy: 0,
            qz: 0,
            kind: 0,
        };
        let p = resolve(Aim::new(0.0, 0.0, 1.0, 0.0), &[], &defs, have, &[bag]);
        assert!(p.is_none());
    }

    /// A row the def table has not dripped yet is not a bag — it is unknown,
    /// and offering a verb on `INERT` would name a thing that is not there.
    #[test]
    fn an_undripped_row_offers_nothing() {
        let (defs, _) = defs_with(&[ARCH_BOX]);
        let recs = [rec(0, 0, 0)];
        let (cx, cz) = centre(0, 0);
        let p = resolve(Aim::new(cx, cz - 2.0, 0.0, 1.0), &recs, &defs, 0, &[]);
        assert!(p.is_none());
    }

    /// `box_key(0, 0, 0, 0)` is the reserved no-container handle and the sim
    /// refuses to place there. The pick must not offer one either.
    #[test]
    fn the_reserved_box_handle_is_never_offered() {
        let (defs, have) = defs_with(&[ARCH_BOX]);
        let recs = [rec(0, 0, 0)];
        assert_eq!(box_key(0, 0, 0, 0), 0);
        let (cx, cz) = centre(0, 0);
        let p = resolve(Aim::new(cx, cz - 1.0, 0.0, 1.0), &recs, &defs, have, &[]);
        assert!(p.is_none());
    }

    #[test]
    fn a_door_prompt_reports_the_state_the_wire_carries() {
        let mut p = Pick {
            verb: Verb::Door,
            ..Pick::default()
        };
        // Bare: nothing but the verb. A door nobody has secured has no
        // keypad to name and is not locked (lock v1).
        assert_eq!(p.prompt(&ItemCatalog::EMPTY), "[E] OPEN DOOR");
        p.open = true;
        assert_eq!(p.prompt(&ItemCatalog::EMPTY), "[E] CLOSE DOOR");
        // Bolted but not armed: the keypad exists, the door is not shut.
        // This is the state the two-bit prompt could not say, and the
        // reason `has_lock` is on the wire at all.
        p.has_lock = true;
        assert!(
            p.prompt(&ItemCatalog::EMPTY).contains("[L] KEYPAD"),
            "{}",
            p.prompt(&ItemCatalog::EMPTY)
        );
        assert!(
            !p.prompt(&ItemCatalog::EMPTY).contains("LOCKED"),
            "an unarmed lock is not a locked door: {}",
            p.prompt(&ItemCatalog::EMPTY)
        );
        // Armed: both.
        p.locked = true;
        assert!(
            p.prompt(&ItemCatalog::EMPTY).contains("LOCKED"),
            "{}",
            p.prompt(&ItemCatalog::EMPTY)
        );
        assert!(
            p.prompt(&ItemCatalog::EMPTY).contains("[L] KEYPAD"),
            "{}",
            p.prompt(&ItemCatalog::EMPTY)
        );
    }

    #[test]
    fn every_verb_has_a_prompt() {
        for verb in [Verb::Door, Verb::Bag, Verb::Box, Verb::Hearth] {
            let p = Pick {
                verb,
                ..Pick::default()
            };
            assert!(
                !p.prompt(&ItemCatalog::EMPTY).is_empty(),
                "{verb:?} has no prompt"
            );
            assert!(!verb.label().is_empty(), "{verb:?} has no label");
        }
    }

    /// A pick must be a function of its inputs alone — the prompt drawn on
    /// one frame and the verb run on the keypress cannot differ while the
    /// world stands still.
    #[test]
    fn the_pick_is_deterministic_under_a_tie() {
        let (defs, have) = defs_with(&[ARCH_BOX]);
        // Two boxes placed symmetrically about the aim ray.
        let recs = [rec(0, 2, 0), rec(2, 2, 0)];
        let (x0, _) = centre(0, 2);
        let (x1, _) = centre(2, 2);
        let (_, z) = centre(0, 0);
        let aim = Aim::new((x0 + x1) * 0.5, z, 0.0, 1.0);
        let first = resolve(aim, &recs, &defs, have, &[]);
        for _ in 0..8 {
            let again = resolve(aim, &recs, &defs, have, &[]);
            assert_eq!(first.handle, again.handle);
        }
    }
}

// ---------------------------------------------------------------------------
// The second resolver: what a SWING would hit.
//
// `E` above answers for deployables and bags. The left button answers for the
// scatter — a tree, an ore node, a bush, a barrel — and until this landed the
// crosshair named what `E` would do and never what a swing would hit, so the
// most common verb in the game was the one with no prompt.
//
// **It is the sim's own cast, not a port of it** (melee aim v1, 2026-09-05).
// The deleted `web/src/interact.js` had a `resolveSwing` that hand-mirrored
// the sim's planar scan, this file's first version ported that mirror from
// `gather::swing` and pinned four constants to keep the two in step, and the
// swing then became a ray along the look — yaw AND pitch, entering real
// volumes. Rather than mirror that, the prompt calls `melee::node_cast` on
// the client's own memo: one function on both sides of the wire, so nothing
// here can drift from the swing it invites. The only numbers this file still
// names are the two reaches, and both are imports.
//
// **One difference from the browser, and it is a simplification the native
// client earned.** `resolveSwing` skipped a cell that was `hidden || fellAt`,
// because three.js animated the fall over `FELL_TICKS + FELL_SINK_TICKS` (93
// ticks, 3.1 s) and a tree the sim had already taken still stood on screen —
// so reading `hidden` alone offered "CHOP TREE" over a stump-to-be and the
// swing whiffed. This client has no such window: `render::props::apply_fell`
// is driven directly off `harvested(key)` and swaps the mesh on the event, so
// the harvested set is the whole truth and there is no second state to test.

use sim_core::backpack::LOOT_REACH_M as OPEN_REACH_M;
use sim_core::gather::{POINT_BLANK_M2, REACH_M as SWING_REACH_M};
use sim_core::melee::{self, Ray};
use sim_core::movement::{quant_xz, quant_y, Body};
use sim_core::occupy::{Harvested, Occupants, SlotCache};
use sim_core::ranged::MM_PER_M;
// `scatter` itself is deliberately NOT imported: every read on this path goes
// through `Island::slot`, and `tests/ui.rs`'s call-site gate says so.
use sim_core::terrain::{Haven, Occupant, ScatterTable, Slot};

/// What a swing would land on, or `Occupant::None` for a whiff.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SwingPick {
    /// The terrain occupant ordinal — the same value `EV_SLOT_HARVESTED`
    /// names in field `b`, so the prompt and the event speak one vocabulary.
    pub occupant: u8,
    pub cx: u16,
    pub cz: u16,
    /// Planar squared distance from the feet to the picked slot's centre.
    /// Diagnostics for the gate and the `E` pick's own `d2`; nothing draws
    /// it, and it is not what decided the pick — the ray did.
    pub d2: f32,
    /// Where the picked slot stands, world metres — the scatter's own
    /// `Slot { x, y, z }`, carried out rather than re-derived.
    ///
    /// **Additive and cosmetic**: no verb reads it and the wire never sees
    /// it. It exists because `render::impact` needs a point to throw chips
    /// from, and the alternative — a second walk of the scatter from the
    /// cell key — is the two-copies-of-one-rule failure that
    /// `column_floor_y` was written to end. Zero when `occupant` is 0,
    /// which is the same "nothing picked" the other fields already say.
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// The slot's own scale, carried out for the same reason `x`/`y`/`z`
    /// are: `render::impact` places a burst on the occupant's collision
    /// skin, which is `occupant_volume` × this. Zero when nothing is picked.
    pub scale: f32,
}

/// The noun the prompt names for a swing pick, or `""` for a whiff.
///
/// `[LMB]` rather than a verb name is the caller's job — these are the labels
/// only, and they name a KIND of thing (`CONTENT.md` owns item names).
pub fn swing_label(occupant: u8) -> &'static str {
    match occupant {
        o if o == Occupant::Tree as u8 => "CHOP TREE",
        o if o == Occupant::StoneNode as u8 => "MINE STONE",
        o if o == Occupant::MetalNode as u8 => "MINE METAL",
        o if o == Occupant::SulfurNode as u8 => "MINE SULFUR",
        // No bush: it is picked with `E` (`resolve_pick`), never swung at.
        o if o == Occupant::BarrelSlot as u8 => "SMASH BARREL",
        o if o == Occupant::OilBarrel as u8 => "SMASH OIL BARREL",
        o if o == Occupant::RoadSign as u8 => "SMASH ROAD SIGN",
        _ => "",
    }
}

/// Where the swing is taken from: the feet, and the look **as the wire
/// carries it**.
///
/// `yaw` and `pitch` are `InputFrame`'s own two fields — the 256-entry LUT
/// heading and the 128-level pitch byte — rather than a float direction,
/// because the sim resolves the swing from exactly those bytes and nothing
/// finer. A prompt computed from the unquantized camera would name a node
/// the quantized ray misses by a LUT step, which is the drift the
/// quantize-both-sides law exists for (`CLAUDE.md` §traps). `render/verbs.rs`
/// converts through the same `look::{yaw_u16, pitch_u8}` the frame does.
///
/// A struct rather than five scalars because the five are one fact.
#[derive(Clone, Copy, Debug)]
pub struct SwingAim {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub yaw: u16,
    pub pitch: u8,
    /// The stance the sim will swing from (`ClientCore::crouched`, v83):
    /// a crouched eye is `ranged::CROUCH_EYE_MM` over the feet.
    pub crouched: bool,
}

impl SwingAim {
    /// The sim's own ray for this aim, `reach_m` long: the stance's eye
    /// (`ranged::eye_mm`) over the feet, along the look — `melee::ray`,
    /// from a body quantized the way the server holds one.
    fn ray(&self, reach_m: f32) -> Ray {
        let body = Body {
            qx: quant_xz(self.x),
            qy: quant_y(self.y),
            qz: quant_xz(self.z),
            ..Body::default()
        };
        melee::ray(
            &body,
            self.crouched,
            self.yaw,
            self.pitch,
            reach_m * MM_PER_M,
        )
    }
}

/// The island the swing lands on, borrowed rather than copied.
///
/// `harvested` is a trait object for the reason `render::props::apply_fell`
/// takes `&dyn Fn`: the caller's harvest state is `ClientCore`'s in the game
/// and a fixture in the gate, and neither should force a generic through
/// every signature between here and the HUD.
///
/// This is `sim_core::occupy::Occupants` plus the seed, deliberately — the
/// sim bundles the identical four borrows for the identical scan, and
/// `ClientCore::island` hands both halves over in one call so the client's
/// crosshair and the client's predictor share one set of cache lines.
pub struct Island<'a> {
    pub seed: u64,
    pub table: &'a ScatterTable,
    pub haven: &'a Haven,
    pub harvested: &'a dyn Harvested,
    /// The card doors open now (`occupy::Occupants::doors`).
    pub doors: u32,
    /// **The memo, and the only door to `terrain::scatter` on this path.**
    /// A cold `scatter` costs ~60 `noise2` taps, a 3×3 window nine of them,
    /// and the crosshair resolves that window every frame — so the resolver
    /// reads through here and `tests/ui.rs`'s call-site gate refuses a
    /// direct call that would walk past it.
    pub cache: &'a mut SlotCache,
}

impl Island<'_> {
    /// The slot in cell (`cx`, `cz`), memoized.
    ///
    /// Identical bits to `terrain::scatter(seed, table, haven, cx, cz)` — the
    /// cache is a pure function's memo, so a hit and a miss return the same
    /// answer and eviction can only change how long it took
    /// (`occupy::SlotCache`'s own argument). `tests/ui.rs` proves that
    /// equality over a walk of the island rather than asserting it here.
    pub fn slot(&mut self, cx: i32, cz: i32) -> Slot {
        self.cache.slot(self.seed, self.table, self.haven, cx, cz)
    }

    /// The same four borrows as the sim's `Occupants`, so `melee`'s casts
    /// run over the client's memo exactly as they run over the server's.
    fn occupants(&mut self) -> Occupants<'_> {
        Occupants {
            doors: self.doors,
            table: self.table,
            haven: self.haven,
            harvested: self.harvested,
            cache: self.cache,
        }
    }
}

/// A cast's answer as the prompt carries it.
fn pick_of(at: SwingAim, cx: u16, cz: u16, s: Slot) -> SwingPick {
    let (dx, dz) = (s.x - at.x, s.z - at.z);
    SwingPick {
        occupant: s.occupant as u8,
        cx,
        cz,
        d2: dx * dx + dz * dz,
        x: s.x,
        y: s.y,
        z: s.z,
        scale: s.scale,
    }
}

/// What a swing from `at` would land on: **the sim's own cast**, over the
/// client's memo.
///
/// This is `sim_core::melee::node_cast` and not a mirror of it — the one
/// function, called on both sides of the wire, so the prompt and the swing
/// it invites cannot disagree about which node the ray enters first, how far
/// the arm reaches, or whether the eye's pitch carries the ray over a
/// knee-high node (it does: a level look at a stone node from a 1.6 m eye is
/// a whiff, and the prompt says so before the click). Until melee aim v1 this
/// was a 3×3 planar walk copied from `gather::swing`, and the two had to be
/// kept in step by a test that pinned four constants.
///
/// Allocates nothing and is **idempotent**: the caller may run it every frame
/// for the prompt and again on the click without the two disagreeing.
/// `island.cache` is written, but only with answers `terrain::scatter` would
/// have recomputed, so nothing observable moves.
pub fn resolve_swing(at: SwingAim, island: &mut Island<'_>) -> SwingPick {
    let ray = at.ray(SWING_REACH_M);
    let seed = island.seed;
    match melee::node_cast(seed, &mut island.occupants(), &ray) {
        Some(hit) => pick_of(at, hit.cx, hit.cz, hit.slot),
        None => SwingPick::default(),
    }
}

/// Whether an occupant is something `E` OPENS.
///
/// The mirror of `swingable`, and its complement rather than a subset:
/// nothing on the island is both smashed and opened. A barrel is hit until
/// it bursts and pays into a bag on the ground; a crate is furniture with
/// a lid. That split is the sim's — `gather::target_index` claims the
/// barrel and `worldcont::table_of` claims these two — and this is those
/// two predicates read from the client side, never a third opinion.
fn openable(o: Occupant) -> bool {
    sim_core::worldcont::table_of(o).is_some()
}

/// What `E` would OPEN in the scatter, or `Occupant::None` for nothing.
///
/// Same cast, same memo, same eye as `resolve_swing` — a different predicate
/// and a different reach, because an open is priced at the arm every other
/// container uses (`backpack::LOOT_REACH_M`) and a swing at the shorter
/// `gather::REACH_M`. Sharing `melee::occupant_cast` rather than copying it
/// is the point: the two prompts must never disagree about where the eye is
/// or which way it looks, and two hand-written walks would eventually do
/// exactly that.
///
/// ⚠ **It is narrower than the verb it prompts for, and it always was.**
/// `worldcont::open` asks for planar proximity inside `LOOT_REACH_M` and
/// nothing else — no aim at any distance — so the prompt has been the
/// stricter of the two since it was a 30° cone, and a ray is stricter again:
/// a crate is 0.68 m across and 0.8 m tall, so from a 1.6 m eye it wants the
/// look dropped 44° at 1.5 m and 13° at 4 m. Whether stooping to open a
/// barrel reads as aiming or as fumbling is a frame question and it is
/// unanswered (`NOW.md` §0ray item 6); padding the cast is the fix if it
/// reads badly, and inventing the pad before anybody has looked would be
/// inventing a knob.
pub fn resolve_open(at: SwingAim, island: &mut Island<'_>) -> SwingPick {
    let ray = at.ray(OPEN_REACH_M);
    let seed = island.seed;
    match melee::occupant_cast(seed, &mut island.occupants(), &ray, openable) {
        Some(hit) => pick_of(at, hit.cx, hit.cz, hit.slot),
        None => SwingPick::default(),
    }
}

/// What `E` would PICK in the scatter — a standing berry bush or hemp — or
/// `Occupant::None` for nothing. A shrub is scenery and is never offered.
///
/// `melee::pick_cast` — [`resolve_open`]'s cast, taking the nearest
/// occupant only if a hand picks it — at the reach the sim checks
/// (`gather::PICK_REACH_M`, measured from the same eye to the same swing
/// volume), so the prompt never offers a pick `gather::pick` refuses.
pub fn resolve_pick(at: SwingAim, island: &mut Island<'_>) -> SwingPick {
    let ray = at.ray(sim_core::gather::PICK_REACH_M);
    let seed = island.seed;
    match melee::pick_cast(seed, &mut island.occupants(), &ray) {
        Some(hit) => pick_of(at, hit.cx, hit.cz, hit.slot),
        None => SwingPick::default(),
    }
}

// ---------------------------------------------------------------------------
// The weak spot.
//
// `EV_WEAK_MARK` has been decoded into `ClientCore::{mark_cell, mark8,
// mark_weak_hit}` for a long time and read by nothing, so the one mechanic in
// this game that rewards *where you stand* was invisible: the server was
// announcing a bearing every hit and the player had no way to learn it existed
// (`NOW.md` §0x item 6 — the reference's own `OnDispenserBonus`).
//
// **This mirrors `gather::swing`'s sector test and does not restate it.**
// `WEAK_COS` is imported, the bearing goes through the same 256-entry yaw LUT
// the sim uses, and the point-blank exemption is the sim's own — a hit inside
// `POINT_BLANK_M2` has no bearing to judge, so it never bonuses and the HUD
// must not promise that it will. The client cannot *decide* a weak hit (the
// server does, and says so in `mark_weak_hit`); what it can do is tell the
// player whether they are standing where the next one would land, which is
// the whole skill.

use sim_core::gather::{cell_key, NO_CELL, WEAK_COS};
use sim_core::yaw_dir;

/// Whether the player at `(px, pz)` stands in the announced weak sector of
/// the node at `(nx, nz)`.
///
/// `mark8` is the sector's heading over the sim's 256-entry LUT. The offset
/// is node→player, matching `gather::swing`'s `ox`/`oz`, so a mark of 0
/// (north) means *stand north of the node*.
pub fn in_weak_sector(px: f32, pz: f32, nx: f32, nz: f32, mark8: u8) -> bool {
    let ox = px - nx;
    let oz = pz - nz;
    let d2 = ox * ox + oz * oz;
    // Point blank has no bearing to judge. The sim exempts it, so this must
    // too — a prompt that lit up while standing inside the trunk would
    // promise a bonus the server refuses.
    if d2 <= POINT_BLANK_M2 {
        return false;
    }
    let (wx, wz) = yaw_dir((mark8 as u16) << 8);
    ox * wx + oz * wz > WEAK_COS * d2.sqrt()
}

/// Whether the announced mark belongs to the cell this swing would hit.
///
/// The chase is per-node and the server restarts it when the player switches
/// targets, so a mark for the tree behind you says nothing about the one in
/// front. `NO_CELL` is "no chase in progress" and is the state between the
/// first swing at a fresh node and the hit that announces its mark.
pub fn mark_is_for(mark_cell: u32, pick: &SwingPick) -> bool {
    mark_cell != NO_CELL && pick.occupant != 0 && mark_cell == cell_key(pick.cx, pick.cz)
}

/// A downed body is an aimed E target. A nearby loose stack is still the
/// fallback; it must not absorb the help gesture beside a dropped weapon.
pub fn resolve_assist(aim: SwingAim, own: u32, entities: &[(u32, protocol::EntityState)]) -> Pick {
    use sim_core::movement::{quant_xz, quant_y, Body};
    let body = Body {
        qx: quant_xz(aim.x),
        qy: quant_y(aim.y),
        qz: quant_xz(aim.z),
        ..Default::default()
    };
    let ray = sim_core::assist::ray(
        &body,
        &sim_core::input::InputFrame {
            yaw: aim.yaw,
            pitch: aim.pitch,
            ..Default::default()
        },
        aim.crouched,
    );
    let mut best = Pick::default();
    let mut distance = f32::MAX;
    for &(id, e) in entities {
        if id == own || !e.wounded || e.dead || e.sleeping {
            continue;
        }
        let target = Body {
            qx: e.qx,
            qy: e.qy,
            qz: e.qz,
            ..Default::default()
        };
        if let Some(t) = sim_core::assist::aimed(&ray, &target) {
            if t < distance || (t == distance && id < best.handle) {
                distance = t;
                best = Pick {
                    verb: Verb::Assist,
                    handle: id,
                    aimed: true,
                    d2: (t * sim_core::assist::ASSIST_REACH_M).powi(2),
                    ..Default::default()
                };
            }
        }
    }
    best
}

/// A nametag's target: the player id, the eye the aim leaves from, and
/// their head.
pub type NametagHit = (u32, (f32, f32, f32), (f32, f32, f32));

/// The player a nametag belongs to: the nearest one whose capsule the aim
/// ray enters within `ui::names::NAMETAG_REACH_M` (aim-only, `DECISIONS.md`),
/// skipping yourself, the dead, animals and anyone the shard has not tagged.
/// Returns the id, the eye the ray leaves from and the head it reaches —
/// the two ends a caller checks for a clear line before drawing a name.
pub fn resolve_nametag(
    aim: SwingAim,
    own: u32,
    entities: &[(u32, protocol::EntityState)],
    tagged: impl Fn(u32) -> bool,
) -> Option<NametagHit> {
    use sim_core::movement::POS_Y_Q;
    let ray = aim.ray(super::names::NAMETAG_REACH_M);
    let mut best: Option<(f32, u32, (f32, f32, f32))> = None;
    for &(id, e) in entities {
        if id == own || e.dead || id & sim_core::limits::MOB_ID_TAG != 0 || !tagged(id) {
            continue;
        }
        let target = Body {
            qx: e.qx,
            qy: e.qy,
            qz: e.qz,
            ..Default::default()
        };
        let Some(t) = sim_core::assist::aimed(&ray, &target) else {
            continue;
        };
        if best.is_none_or(|(bt, bid, _)| t < bt || (t == bt && id < bid)) {
            let head = (
                e.qx as f32 * POS_XZ_Q,
                e.qy as f32 * POS_Y_Q + sim_core::collide::CAPSULE_HEIGHT_M * 0.9,
                e.qz as f32 * POS_XZ_Q,
            );
            best = Some((t, id, head));
        }
    }
    best.map(|(_, id, head)| (id, ray.at_m(0.0), head))
}

#[cfg(test)]
mod assist_tests {
    use super::*;
    use sim_core::movement::{quant_xz, quant_y};

    #[test]
    fn hand_help_requires_aim_and_live_wounded_presence() {
        let aim = SwingAim {
            x: 10.0,
            y: 0.0,
            z: 10.0,
            yaw: 0,
            pitch: 128,
            crouched: false,
        };
        let target = protocol::EntityState {
            qx: quant_xz(10.0),
            qy: quant_y(0.0),
            qz: quant_xz(11.5),
            wounded: true,
            ..Default::default()
        };
        let pick = resolve_assist(aim, 1, &[(2, target)]);
        assert_eq!(pick.verb, Verb::Assist);
        assert_eq!(pick.handle, 2);
        assert!(pick.aimed);
        assert!(pick.prompt(&ItemCatalog::EMPTY).contains("HOLD [E]"));
        assert!(resolve_assist(aim, 2, &[(2, target)]).is_none());
        for absent in [
            protocol::EntityState {
                wounded: false,
                ..target
            },
            protocol::EntityState {
                dead: true,
                ..target
            },
            protocol::EntityState {
                sleeping: true,
                ..target
            },
            protocol::EntityState {
                qz: quant_xz(14.0),
                ..target
            },
        ] {
            assert!(resolve_assist(aim, 1, &[(2, absent)]).is_none());
        }
        assert!(resolve_assist(SwingAim { yaw: 32768, ..aim }, 1, &[(2, target)]).is_none());
        // Identity breaks an exact geometric tie, independent of arrival order.
        assert_eq!(
            resolve_assist(aim, 1, &[(3, target), (2, target)]).handle,
            2
        );
    }
}
