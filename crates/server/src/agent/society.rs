//! A society (`jev-bot --society`): agent players who live as a clan, the
//! way a Rust clan does. A **crew** is one team in one base, each body with
//! a job: the founder who builds and keeps the base, farmers who fill its
//! boxes, guards who walk a ring round it, a runner who trades the surplus.
//! A **guild** is several crews and the players they name as allies, who
//! never shoot each other and whose crews build their bases side by side.
//!
//! What they share is what a clan's voice channel would carry: who is in
//! it, who does what, the code on the founder's doors, and where the guild
//! builds ([`Board`]). Everything else each body learns the way a player
//! does: who a body is from its nametag ([`Kinship::refresh`]), where home
//! is by being shown it (`agent::supply`), who is about by its own eyes.
//!
//! **Why the guards do not shoot on sight.** A ring of guards that kills
//! everyone makes the compound a death zone: nobody comes to trade, and the
//! farmers' surplus has no buyer. So a guard is a defensive body (it answers
//! a blow, never starts one) and its patrol takes on only an *intruder*
//! ([`intruder`]): someone inside the ward with a weapon in their hands, or
//! who swung or shot near it. A visitor walking up with empty hands is left
//! alone, as THE GATE's own turrets leave alone anyone who has not attacked
//! a player lately.

use crate::agent::combat::Temperament;
use crate::agent::lock::LockSecret;
use crate::agent::tracks::{Species, Track};
use client_core::core::ClientCore;
use sim_core::bots::OpAddr;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

/// The most players one body counts as its guild beyond its own crew:
/// other crews' members and the allies named on the command line.
pub const MAX_ALLIES: usize = 16;
/// The most crews one guild (one `jev-bot` run) holds.
pub const MAX_CREWS: usize = 4;

/// A crew member not yet on the team is walked up to from this far, metres:
/// in sight at THE GATE, where the crew meets.
pub const GATHER_M: f32 = 40.0;
/// How close a member walks to its founder for the invite, metres: inside
/// the agent's invite reach (`explorer::INVITE_REACH_M`).
pub const GATHER_STOP_M: f32 = 2.0;
/// A crewmate keeping up with its team stops this near them, metres: close
/// enough to be shown home, far enough to stay off the building site.
pub const KEEP_UP_M: f32 = 10.0;
/// After a session's welcome the founder waits at THE GATE for its crew
/// at most this long before it starts on the base; a crewmate who comes
/// later walks to wherever its founder says it is ([`Board::founder`]).
pub const FOUNDING_SECS: u32 = 8 * 60;

/// The patrol: a ring of posts this far from home's middle, metres — outside
/// a starter base and its yard, inside the reach of a shout.
pub const PATROL_RING_M: f32 = 22.0;
/// Posts on the ring: a hexagon's corners, so no trigonometry is needed.
pub const PATROL_POSTS: [[f32; 2]; 6] = [
    [1.0, 0.0],
    [0.5, 0.866_025_4],
    [-0.5, 0.866_025_4],
    [-1.0, 0.0],
    [-0.5, -0.866_025_4],
    [0.5, -0.866_025_4],
];
/// How long a guard stands at a post looking out, ticks.
pub const PATROL_DWELL_TICKS: u32 = 3 * sim_core::limits::TICK_HZ;
/// A post is reached within this many metres.
pub const PATROL_STOP_M: f32 = 2.5;
/// One patrol decision is a few laps' worth, seconds.
pub const PATROL_GOAL_SECS: u32 = 120;
/// The ward: an intruder is someone this near home's middle, metres.
pub const WARD_M: f32 = 34.0;
/// A swing or a shot this recent makes its owner an intruder, ticks.
pub const WARD_FOUGHT_TICKS: u32 = 10 * sim_core::limits::TICK_HZ;

/// What a body does for its crew.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Role {
    /// Founds the team, builds the base and keeps it: the cupboard, the
    /// stations, the cooking and the recycling.
    #[default]
    Founder,
    /// Supply runs for the team's boxes; never fights.
    Farmer,
    /// Walks a ring round home and takes on intruders.
    Guard,
    /// Takes the surplus to THE GATE and comes back with tools.
    Runner,
}

impl Role {
    pub const ALL: [Role; 4] = [Role::Founder, Role::Farmer, Role::Guard, Role::Runner];
    /// The jobs a crew hands out when none are named, after its founder:
    /// a farmer, a guard and a runner (who keeps the shop) for a crew of
    /// four, then more hands to the boxes than guards.
    const CYCLE: [Role; 6] = [
        Role::Farmer,
        Role::Guard,
        Role::Runner,
        Role::Farmer,
        Role::Guard,
        Role::Farmer,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Role::Founder => "founder",
            Role::Farmer => "farmer",
            Role::Guard => "guard",
            Role::Runner => "runner",
        }
    }

    pub fn parse(name: &str) -> Option<Role> {
        Self::ALL.into_iter().find(|r| r.name() == name)
    }

    /// The job of crew member `i` when none was named: the founder first.
    pub fn nth(i: usize) -> Role {
        match i {
            0 => Role::Founder,
            _ => Self::CYCLE[(i - 1) % Self::CYCLE.len()],
        }
    }

    /// When it fights. A farmer runs from every fight; everyone else
    /// answers a blow and starts none of their own — a guard's patrol is
    /// what picks its fights, and only with intruders.
    pub fn temperament(self) -> Temperament {
        match self {
            Role::Farmer => Temperament::Passive,
            Role::Founder | Role::Guard | Role::Runner => Temperament::Defensive,
        }
    }
}

/// Someone a society knows: by wallet when they have one (any shard), else
/// by the name on their nametag, else — a guest agent of its own guild, who
/// has no nametag at all — by the post it holds on the guild's board
/// ([`Board::member`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Kin {
    pub name: protocol::Name,
    pub wallet: protocol::Address,
    /// Its post on the guild's board, one past [`post_of`]; 0 for someone
    /// outside the guild's own run.
    pub post: u8,
}

/// Board post of member `me` of crew `crew_no`.
pub fn post_of(crew_no: u8, me: u8) -> usize {
    usize::from(crew_no) * protocol::TEAM_MAX + usize::from(me)
}

/// Members a guild's board has posts for.
pub const BOARD_POSTS: usize = MAX_CREWS * protocol::TEAM_MAX;

impl Kin {
    pub const NONE: Self = Self {
        name: protocol::Name::EMPTY,
        wallet: protocol::Address::GUEST,
        post: 0,
    };

    /// `0x` and forty hex digits is a wallet; anything else is a name.
    pub fn parse(text: &str) -> Option<Kin> {
        let text = text.trim();
        if let Some(wallet) = protocol::Address::from_hex(text.as_bytes()).filter(|a| !a.is_guest())
        {
            return Some(Kin {
                wallet,
                ..Kin::NONE
            });
        }
        protocol::Name::new(text)
            .filter(|n| !n.is_empty())
            .map(|name| Kin { name, ..Kin::NONE })
    }

    /// Member `me` of crew `crew_no` of this run's guild.
    pub fn member(
        name: protocol::Name,
        wallet: Option<protocol::Address>,
        crew_no: u8,
        me: u8,
    ) -> Kin {
        Kin {
            name,
            wallet: wallet.unwrap_or(protocol::Address::GUEST),
            post: (post_of(crew_no, me) + 1) as u8,
        }
    }

    pub fn is_set(&self) -> bool {
        !self.wallet.is_guest() || !self.name.is_empty() || self.post != 0
    }

    /// Is this nametag theirs? A wallet is proven by the shard; a name is
    /// only a name, so a crewmate known by name must also have declared
    /// itself an agent (`agent`) — a person typing a crewmate's name onto
    /// their own tag is not let into the base for it.
    fn matches(&self, tag: &client_core::core::Tag, agent: bool, need_agent: bool) -> bool {
        if !self.wallet.is_guest() {
            return tag.address == self.wallet;
        }
        !self.name.is_empty() && tag.name == self.name && (agent || !need_agent)
    }
}

/// How far apart a guild's bases stand, metres: clear of each other's
/// claim (`site::CLAIM_KEEP_M`), close enough that the guards' rings round
/// them overlap.
pub const SEAT_STEP_M: f32 = 40.0;

/// Where crew `crew_no` of a guild seated at `seat` builds: the senior crew
/// on the seat, the others a step east, west and north of it.
pub fn seat_for(crew_no: u8, seat: [f32; 2]) -> [f32; 2] {
    let [dx, dz] = match crew_no {
        0 => [0.0, 0.0],
        1 => [1.0, 0.0],
        2 => [-1.0, 0.0],
        _ => [0.0, 1.0],
    };
    [seat[0] + dx * SEAT_STEP_M, seat[1] + dz * SEAT_STEP_M]
}

/// What the guild's crews share beyond who is in them, as a clan says it
/// over its voice channel: where the senior founder's base stands, so the
/// other founders build theirs beside it and the compound is a few bases
/// together; where each crew's founder is now, so a crewmate who comes
/// late walks to them rather than to an empty GATE; and which box is each
/// crew's, so a crewmate who never saw its founder at it still knows where
/// home is (walls hide a founder at its box). Lock-free: words written by
/// one body each, read by the others between frames.
#[derive(Debug, Default)]
pub struct Board {
    seat: AtomicU64,
    founders: [AtomicU64; MAX_CREWS],
    chests: [AtomicU64; MAX_CREWS],
    /// Who holds each post on this island now: the player id its own
    /// client was welcomed as. A guest agent has no nametag (the shard tags
    /// wallets), so this is how its guild knows it — what a clan member
    /// would say over voice: "that's me, waving".
    members: [AtomicU32; BOARD_POSTS],
}

/// A place as one word: both coordinates' bits, never zero.
fn pack(at: [f32; 2]) -> u64 {
    ((u64::from(at[0].to_bits()) << 32) | u64::from(at[1].to_bits())).max(1)
}

fn unpack(bits: u64) -> Option<[f32; 2]> {
    (bits != 0).then(|| {
        [
            f32::from_bits((bits >> 32) as u32),
            f32::from_bits(bits as u32),
        ]
    })
}

/// A box's address as one word, with a bit to say there is one.
const CHEST_SET: u64 = 1 << 63;

impl Board {
    pub const fn new() -> Self {
        Self {
            seat: AtomicU64::new(0),
            founders: [const { AtomicU64::new(0) }; MAX_CREWS],
            chests: [const { AtomicU64::new(0) }; MAX_CREWS],
            members: [const { AtomicU32::new(0) }; BOARD_POSTS],
        }
    }

    /// The player id holding `post` (one past [`post_of`]), 0 for none.
    pub fn member(&self, post: u8) -> u32 {
        usize::from(post)
            .checked_sub(1)
            .and_then(|p| self.members.get(p))
            .map_or(0, |w| w.load(Ordering::Relaxed))
    }

    pub fn set_member(&self, post: usize, id: u32) {
        if let Some(w) = self.members.get(post) {
            w.store(id, Ordering::Relaxed);
        }
    }

    /// Where the guild builds, once its senior founder has chosen.
    pub fn seat(&self) -> Option<[f32; 2]> {
        unpack(self.seat.load(Ordering::Relaxed))
    }

    pub fn set_seat(&self, at: [f32; 2]) {
        self.seat.store(pack(at), Ordering::Relaxed);
    }

    /// Where crew `crew`'s founder last said it stands.
    pub fn founder(&self, crew: u8) -> Option<[f32; 2]> {
        unpack(
            self.founders
                .get(usize::from(crew))?
                .load(Ordering::Relaxed),
        )
    }

    pub fn set_founder(&self, crew: u8, at: [f32; 2]) {
        if let Some(w) = self.founders.get(usize::from(crew)) {
            w.store(pack(at), Ordering::Relaxed);
        }
    }

    /// Crew `crew`'s box, as its founder last said.
    pub fn chest(&self, crew: u8) -> Option<OpAddr> {
        let bits = self.chests.get(usize::from(crew))?.load(Ordering::Relaxed);
        (bits & CHEST_SET != 0).then_some(OpAddr {
            cx: bits as u16,
            cz: (bits >> 16) as u16,
            level: (bits >> 32) as u8,
            loc: (bits >> 40) as u8,
        })
    }

    pub fn set_chest(&self, crew: u8, at: Option<OpAddr>) {
        let bits = at.map_or(0, |a| {
            CHEST_SET
                | u64::from(a.cx)
                | (u64::from(a.cz) << 16)
                | (u64::from(a.level) << 32)
                | (u64::from(a.loc) << 40)
        });
        if let Some(w) = self.chests.get(usize::from(crew)) {
            w.store(bits, Ordering::Relaxed);
        }
    }
}

/// A body's place in its society: its job, its crew (founder first, itself
/// among them), its guild beyond the crew, the code on the crew's doors,
/// and the guild's board.
#[derive(Clone, Copy, Debug)]
pub struct Society {
    pub role: Role,
    pub me: u8,
    pub crew: [Kin; protocol::TEAM_MAX],
    pub crew_len: u8,
    pub allies: [Kin; MAX_ALLIES],
    pub allies_len: u8,
    /// The founder's lock secret: what the crew's doors and cupboard are
    /// coded to.
    pub code: Option<LockSecret>,
    /// Which of its guild's crews it is in (the first is the senior crew):
    /// the first crew's founder writes the guild's seat, and every other
    /// founder builds a step from it ([`seat_for`]).
    pub crew_no: u8,
    pub board: Option<&'static Board>,
}

impl Society {
    pub fn founder(&self) -> bool {
        self.role == Role::Founder
    }

    /// Its own post on the guild's board.
    pub fn post(&self) -> usize {
        post_of(self.crew_no, self.me)
    }

    pub fn senior(&self) -> bool {
        self.crew_no == 0
    }

    pub fn crew(&self) -> &[Kin] {
        &self.crew[..usize::from(self.crew_len)]
    }

    pub fn allies(&self) -> &[Kin] {
        &self.allies[..usize::from(self.allies_len)]
    }
}

/// Who its society's members are on this island now, by player id, as the
/// nametags say: refreshed when the tags change and once a second.
#[derive(Clone, Copy, Debug, Default)]
pub struct Kinship {
    pub crew: [u32; protocol::TEAM_MAX],
    pub allies: [u32; MAX_ALLIES],
    gen: Option<u32>,
    at: u32,
}

impl Kinship {
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// Read the nametags again if they changed or a second has gone by.
    /// Allocation-free; a scan of the shard's tag table.
    pub fn refresh(&mut self, soc: &Society, core: &ClientCore, tick: u32) {
        let gen = core.tags_gen;
        if self.gen == Some(gen) && tick.wrapping_sub(self.at) < sim_core::limits::TICK_HZ {
            return;
        }
        self.gen = Some(gen);
        self.at = tick;
        for (i, kin) in soc.crew().iter().enumerate() {
            self.crew[i] = if i == usize::from(soc.me) {
                core.player_id
            } else {
                find(core, soc.board, kin, true)
            };
        }
        for (i, kin) in soc.allies().iter().enumerate() {
            self.allies[i] = find(core, soc.board, kin, false);
        }
    }

    pub fn is_crew(&self, id: u32) -> bool {
        id != 0 && self.crew.contains(&id)
    }

    pub fn is_ally(&self, id: u32) -> bool {
        id != 0 && self.allies.contains(&id)
    }

    /// Crew or ally: never a quarry.
    pub fn is_kin(&self, id: u32) -> bool {
        self.is_crew(id) || self.is_ally(id)
    }

    /// Its founder's player id, once tagged.
    pub fn founder(&self) -> u32 {
        self.crew[0]
    }

    pub fn ids(&self) -> impl Iterator<Item = u32> + '_ {
        self.crew
            .iter()
            .chain(self.allies.iter())
            .copied()
            .filter(|&id| id != 0)
    }
}

/// The newest body on this island whose nametag is `kin`'s; else, for a
/// member of its own guild, whoever holds its post on the board, if that
/// body declared itself an agent.
fn find(core: &ClientCore, board: Option<&Board>, kin: &Kin, crew: bool) -> u32 {
    if !kin.is_set() {
        return 0;
    }
    let tagged = core
        .tags()
        .filter(|t| t.id != core.player_id && kin.matches(t, core.is_agent(t.id), crew))
        .max_by_key(|t| t.id >> 8)
        .map_or(0, |t| t.id);
    if tagged != 0 {
        return tagged;
    }
    board
        .map(|b| b.member(kin.post))
        .filter(|&id| id != 0 && id != core.player_id && core.is_agent(id))
        .unwrap_or(0)
}

/// A patrol under way: the post it is walking to (`None` before the first
/// is chosen), since when it has stood there looking out, and how many
/// posts in a row it could not reach.
#[derive(Clone, Copy, Debug, Default)]
pub struct Patrol {
    pub post: Option<u8>,
    pub dwell_from: Option<u32>,
    pub misses: u8,
}

impl Patrol {
    /// On to the next post round the ring.
    pub fn next(&mut self) {
        self.post = Some(self.post.map_or(0, |p| (p + 1) % PATROL_POSTS.len() as u8));
        self.dwell_from = None;
    }
}

/// Patrol post `k` round `centre`.
pub fn post(centre: [f32; 2], k: usize) -> [f32; 2] {
    let [ux, uz] = PATROL_POSTS[k % PATROL_POSTS.len()];
    [
        centre[0] + ux * PATROL_RING_M,
        centre[1] + uz * PATROL_RING_M,
    ]
}

/// The post nearest `at`, where a patrol starts.
pub fn nearest_post(centre: [f32; 2], at: [f32; 2]) -> usize {
    let mut best = (f32::INFINITY, 0);
    for k in 0..PATROL_POSTS.len() {
        let [x, z] = post(centre, k);
        let d = (x - at[0]).hypot(z - at[1]);
        if d < best.0 {
            best = (d, k);
        }
    }
    best.1
}

/// Is this body an intruder at the home whose middle is `centre`: a player
/// up and in sight, inside the ward, and either holding a weapon or fresh
/// from a swing or a shot. `armed` says whether what it holds is one.
pub fn intruder(t: &Track, centre: [f32; 2], armed: bool, tick: u32) -> bool {
    if t.species != Species::Player || !t.visible || !t.active() {
        return false;
    }
    if (t.pos[0] - centre[0]).hypot(t.pos[2] - centre[1]) > WARD_M {
        return false;
    }
    let fought = [t.last_swing, t.last_shot]
        .into_iter()
        .flatten()
        .any(|when| tick.wrapping_sub(when) < WARD_FOUGHT_TICKS);
    armed || fought || t.aiming_at_me
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roles_cycle_after_the_founder_and_parse_by_name() {
        let crew: Vec<Role> = (0..8).map(Role::nth).collect();
        assert_eq!(crew[0], Role::Founder);
        assert_eq!(crew.iter().filter(|r| **r == Role::Founder).count(), 1);
        assert!(crew.contains(&Role::Guard) && crew.contains(&Role::Runner));
        assert!(
            crew.iter().filter(|r| **r == Role::Farmer).count()
                >= crew.iter().filter(|r| **r == Role::Guard).count()
        );
        for r in Role::ALL {
            assert_eq!(Role::parse(r.name()), Some(r));
        }
        assert_eq!(Role::parse("king"), None);
        assert_eq!(Role::Farmer.temperament(), Temperament::Passive);
        assert_eq!(Role::Guard.temperament(), Temperament::Defensive);
    }

    #[test]
    fn kin_is_a_wallet_or_a_name() {
        let w = Kin::parse("0x00000000000000000000000000000000000000f1").unwrap();
        assert!(!w.wallet.is_guest() && w.name.is_empty());
        let n = Kin::parse(" rook ").unwrap();
        assert_eq!(n.name.as_str(), "rook");
        assert!(n.wallet.is_guest());
        assert!(Kin::parse("").is_none());
        assert!(!Kin::NONE.is_set());
    }

    #[test]
    fn a_name_lets_in_only_an_agent_but_a_wallet_is_proof() {
        let tag = |name: &str, agent_wallet: protocol::Address| client_core::core::Tag {
            id: 300,
            address: agent_wallet,
            name: protocol::Name::new(name).unwrap(),
            pic: 0,
        };
        let rook = Kin::parse("rook").unwrap();
        assert_eq!(rook.post, 0);
        let t = tag("rook", protocol::Address::GUEST);
        assert!(rook.matches(&t, true, true));
        assert!(!rook.matches(&t, false, true), "a person wearing the name");
        assert!(rook.matches(&t, false, false), "an ally by name");
        let wallet =
            protocol::Address::from_hex(b"0x00000000000000000000000000000000000000f1").unwrap();
        let w = Kin::parse("0x00000000000000000000000000000000000000f1").unwrap();
        assert!(w.matches(&tag("anyone", wallet), false, true));
        assert!(!w.matches(&tag("rook", protocol::Address::GUEST), true, true));
    }

    #[test]
    fn the_ring_goes_round_home_and_starts_at_the_nearest_post() {
        let c = [500.0, 700.0];
        for k in 0..PATROL_POSTS.len() {
            let [x, z] = post(c, k);
            let r = (x - c[0]).hypot(z - c[1]);
            assert!((r - PATROL_RING_M).abs() < 0.01, "post {k} at {r} m");
        }
        assert_eq!(nearest_post(c, [c[0] + 40.0, c[1]]), 0);
        assert_eq!(nearest_post(c, [c[0] - 40.0, c[1]]), 3);
    }

    #[test]
    fn an_intruder_is_armed_or_fighting_inside_the_ward() {
        let mut t = Track {
            id: 300,
            species: Species::Player,
            first_seen: 0,
            last_seen: 100,
            pos: [10.0, 0.0, 0.0],
            vel: [0.0; 3],
            yaw: 0,
            pitch: 0,
            held: None,
            lit: false,
            wounded: false,
            dead: false,
            sleeping: false,
            crouched: false,
            dealt: 0,
            last_swing: None,
            last_shot: None,
            aiming_at_me: false,
            visible: true,
        };
        let home = [0.0, 0.0];
        assert!(
            !intruder(&t, home, false, 100),
            "a visitor with empty hands"
        );
        assert!(intruder(&t, home, true, 100), "a spear in hand");
        t.last_swing = Some(90);
        assert!(intruder(&t, home, false, 100), "a swing just now");
        assert!(
            !intruder(&t, home, false, 90 + WARD_FOUGHT_TICKS),
            "a swing long ago"
        );
        t.pos = [WARD_M + 1.0, 0.0, 0.0];
        assert!(!intruder(&t, home, true, 100), "outside the ward");
        t.pos = [1.0, 0.0, 0.0];
        t.species = Species::Wolf;
        assert!(
            !intruder(&t, home, true, 100),
            "a wolf is the fight reflex's"
        );
    }

    #[test]
    fn crews_build_a_step_apart_round_the_seat() {
        let seat = [100.0, 200.0];
        assert_eq!(seat_for(0, seat), seat);
        let mut all: Vec<[f32; 2]> = (0..MAX_CREWS as u8).map(|c| seat_for(c, seat)).collect();
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                assert!((a[0] - b[0]).hypot(a[1] - b[1]) >= SEAT_STEP_M);
            }
        }
        all.dedup();
        assert_eq!(all.len(), MAX_CREWS);
    }

    #[test]
    fn the_board_keeps_the_seat_each_founder_and_each_box() {
        let b = Board::new();
        assert_eq!(b.seat(), None);
        b.set_seat([12.5, -3.0]);
        assert_eq!(b.seat(), Some([12.5, -3.0]));
        assert_eq!(b.founder(1), None);
        b.set_founder(1, [7.0, 8.0]);
        assert_eq!((b.founder(0), b.founder(1)), (None, Some([7.0, 8.0])));
        assert_eq!(b.founder(MAX_CREWS as u8), None, "no such crew");
        let at = OpAddr {
            cx: 600,
            cz: 33,
            level: 2,
            loc: 7,
        };
        b.set_chest(2, Some(at));
        assert_eq!(b.chest(2), Some(at));
        b.set_chest(2, None);
        assert_eq!(b.chest(2), None);
        let kin = Kin::member(protocol::Name::EMPTY, None, 1, 3);
        assert!(kin.is_set(), "a guest crewmate is known by its post");
        assert_eq!(b.member(kin.post), 0);
        b.set_member(post_of(1, 3), 300);
        assert_eq!(b.member(kin.post), 300);
        assert_eq!(b.member(0), 0, "post 0 is nobody's");
    }
}
