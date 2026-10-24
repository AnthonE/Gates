//! A player's auto turret (`NOW.md` §0aa 1): Rust's `AutoTurret`, unpowered
//! (there is no electricity here). A deployable with a box and a gun on top
//! that shoots anybody in reach and in sight who is not **on its own
//! authorize list**.
//!
//! ## Its gun is the one in its box
//!
//! As in Rust, the turret fires the gun it is given: the **first gun in its
//! box** (slot order) that `[turret]` `guns` names, at that gun's own
//! numbers ([`TurretGun`]), spending that gun's round out of the same box.
//! No gun, or no round for it, and it holds fire. The box is already saved,
//! so this needs no save field; the gun it is firing rides the wire as its
//! record's held item ([`Turret::carried`]), which is what the client
//! mounts on it.
//!
//! ## What it reuses
//!
//! The gun is the town sentry's (`sentry.rs`): the same look, lock beep,
//! burst and bullet solve, through `sentry::look` and `sentry::fire`. And
//! like a sentry it rides the animal roster: turret `k` is roster slot
//! [`TURRET_SLOT0`]` + k`, species `mob::MOB_SENTRY`, a mob record whose yaw
//! is where the gun points. So the wire, the client's turret mesh, the
//! tracer and the lock-on beep all exist already.
//!
//! ## Who it spares: its own list
//!
//! Rust's model: the turret carries its own authorize list
//! ([`TurretRec::auth`], a `roster::Roster`), and spares exactly who is on
//! it. Placing it puts its placer on; nobody else is on until they say so.
//! A cupboard's crew is not on it: a crewmate is shot until they authorize.
//!
//! **Who may change the list** is the turret's building privilege,
//! [`spares`]: its owner and whoever is on every crew its owner is on, of
//! the hearths whose claim covers it (`claim::shares_crew`). That is what
//! opening its box already asked (`World`'s `CONT_BOX` path), and what Rust
//! asks: building privilege where it stands. So a crewmate may put
//! themselves on ([`auth_op`]), and a stranger may not; anybody may take
//! themselves off, and a clear empties it, owner and all (Rust's
//! `ClearList`).
//!
//! ## Peacekeeper and attack all
//!
//! Rust's two modes, set by the same privilege ([`auth_op`]). **Attack
//! all**, the one it is placed in, shoots anybody off its list.
//! **Peacekeeper** shoots only the hostile among them: a body whose
//! `Player::hostile` clock runs, which any attack on a player sets for five
//! minutes (`combat::HOSTILE_TICKS`) and so does any blow on a turret
//! (`World::chip`), so a peacekeeper turret answers whoever shoots it. Its
//! list is spared in either.
//!
//! A turret whose list is still its owner alone, attacking all, keeps no
//! record: the store ([`TurretRecs`]) holds only the turrets somebody
//! changed, keyed by box address like the vending machine's offers, and
//! dropped with the turret in `Deploys::remove_at`.
//!
//! ## The table is derived, not kept
//!
//! [`reconcile`] walks the deploy store every look and gives each turret
//! record an entry (and a roster slot); an entry whose record is gone is
//! freed. Placement, a raid, decay, a pickup and a world load all arrive the
//! same way, with no hook in any of them. Past [`MAX_TURRETS`] a placed
//! turret stands inert until one comes down or a dry one gives its entry up
//! to it ([`reconcile`]), and one hand places at most
//! `limits::MAX_TURRETS_PER_OWNER`. Not saved, like a sentry's brain: a
//! restart forgets who it was after.
//!
//! ## Interference
//!
//! Rust's rule rather than a shard-wide one: of the loaded turrets within
//! [`TURRET_INTERFERENCE_M`] of each other, the first
//! [`TURRET_INTERFERENCE`] in table order shoot and the rest stand idle,
//! guns up. A wall of turrets is a wall of twelve.

use crate::build::Pieces;
use crate::collide::ColIndex;
use crate::deploy::{box_key, DeployContent, Deploys, ARCH_TURRET};
use crate::gather::ItemStack;
use crate::heli::Round;
use crate::limits::{
    BOX_SLOTS, MAX_PLAYERS, MAX_TURRETS, MAX_TURRET_GUNS, MAX_TURRET_RECS, TURRET_AUTH_CAP,
    TURRET_INTERFERENCE, TURRET_INTERFERENCE_M,
};
use crate::mob::Mob;
use crate::movement::{Body, POS_XZ_Q, POS_Y_Q};
use crate::nav;
use crate::occupy::Occupants;
use crate::ranged::{ARROW_EYE_MM, MM_PER_M};
use crate::sentry::{self, Sentry, SentryDef, LOOK_TICKS, SENTRY_SLOT0};
use crate::terrain::Haven;
use crate::world::{EventQueue, Player};

/// Turret `k`'s roster slot is `TURRET_SLOT0 + k`, just below the sentries.
pub const TURRET_SLOT0: usize = SENTRY_SLOT0 - MAX_TURRETS;

/// Whether a roster slot is a player turret's.
#[inline]
pub const fn is_turret_slot(slot: usize) -> bool {
    slot >= TURRET_SLOT0 && slot < SENTRY_SLOT0
}

/// The gun's height over the turret's base, metres.
pub const GUN_M: f32 = 0.9;

/// **Who a player's machine answers to**: its owner, and whoever is on
/// every crew its owner is on, of the hearths whose claim covers `(x, z)`
/// (`claim::shares_crew`). Outside its owner's claims, the owner alone.
/// These may open a turret, put themselves on its list and clear it
/// ([`auth_op`]); the vending machine (`shop.rs`) lets exactly these stock
/// it and set its offers. Whom the turret spares is its own list
/// ([`TurretRecs::authorized`]).
pub fn spares(pieces: &Pieces, deploys: &Deploys, owner: u32, x: f32, z: f32, id: u32) -> bool {
    id == owner || crate::claim::shares_crew(pieces, deploys, x, z, owner, id)
}

/// One gun a turret can carry: the item, its numbers on a turret, and the
/// round it spends out of the turret's box.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TurretGun {
    pub item: u16,
    pub def: SentryDef,
    pub ammo: u16,
}

impl TurretGun {
    pub const NONE: Self = Self {
        item: 0,
        def: SentryDef::INERT,
        ammo: 0,
    };
}

/// The baked `[turret]` table: every gun a turret fires, the first `n` of
/// [`Self::guns`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TurretDef {
    pub guns: [TurretGun; MAX_TURRET_GUNS],
    pub n: u8,
}

impl TurretDef {
    pub const INERT: Self = Self {
        guns: [TurretGun::NONE; MAX_TURRET_GUNS],
        n: 0,
    };

    #[inline]
    pub fn armed(&self) -> bool {
        self.n > 0
    }

    /// The guns that are live, in table order.
    #[inline]
    pub fn live(&self) -> &[TurretGun] {
        &self.guns[..self.n as usize]
    }
}

/// A turret's authorize list (`limits::TURRET_AUTH_CAP`).
pub type TurretAuth = crate::roster::Roster<TURRET_AUTH_CAP>;

/// What one turret keeps beyond its box, keyed by its packed box address
/// (`deploy::box_key`). Saved (world format 34).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TurretRec {
    pub key: u32,
    /// Whom it spares.
    pub auth: TurretAuth,
    /// Peacekeeper (shoots only the hostile) rather than attack all.
    pub peacekeeper: bool,
}

/// Bytes of one [`TurretRec`] in `state_hash` and the world save: the key,
/// the list's count, its whole array and the mode (format 35).
pub const TURRET_REC_BYTES: usize = 4 + 1 + TURRET_AUTH_CAP * 4 + 1;

impl TurretRec {
    /// The record a turret owned by `owner` has without one: its owner alone
    /// on the list, attacking all. The store never holds this shape
    /// ([`TurretRecs`]).
    pub fn fresh(key: u32, owner: u32) -> Self {
        Self {
            key,
            auth: TurretAuth::of(owner),
            peacekeeper: false,
        }
    }

    /// Whether it shoots `p`, a body in reach and in sight it could: off its
    /// list, and hostile too when it keeps the peace.
    pub fn shoots(&self, p: &Player) -> bool {
        !self.auth.contains(p.id) && (!self.peacekeeper || p.hostile > 0)
    }

    /// Every field, little-endian: the key, the count, the whole list, the
    /// mode.
    pub fn hash_bytes(&self) -> [u8; TURRET_REC_BYTES] {
        let mut b = [0u8; TURRET_REC_BYTES];
        b[0..4].copy_from_slice(&self.key.to_le_bytes());
        b[4] = self.auth.len() as u8;
        for (k, id) in self.auth.raw().iter().enumerate() {
            b[5 + k * 4..9 + k * 4].copy_from_slice(&id.to_le_bytes());
        }
        b[TURRET_REC_BYTES - 1] = self.peacekeeper as u8;
        b
    }
}

/// Every turret's record that differs from [`TurretRec::fresh`]: a dense
/// list, swap-removed, boxed like the shops.
pub struct TurretRecs {
    entries: Box<[TurretRec; MAX_TURRET_RECS]>,
    len: usize,
}

impl Default for TurretRecs {
    fn default() -> Self {
        Self::new()
    }
}

impl TurretRecs {
    pub fn new() -> Self {
        Self {
            entries: crate::boxed_array(TurretRec::default()),
            len: 0,
        }
    }

    pub fn entries(&self) -> &[TurretRec] {
        &self.entries[..self.len]
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn index_of(&self, key: u32) -> Option<usize> {
        if key == 0 {
            return None;
        }
        self.entries().iter().position(|r| r.key == key)
    }

    /// The turret at `key`'s record, owned by `owner`: its own, or
    /// [`TurretRec::fresh`].
    pub fn get(&self, key: u32, owner: u32) -> TurretRec {
        self.index_of(key)
            .map_or(TurretRec::fresh(key, owner), |i| self.entries[i])
    }

    /// Whether the turret at `key`, owned by `owner`, spares `id`.
    pub fn authorized(&self, key: u32, owner: u32, id: u32) -> bool {
        match self.index_of(key) {
            Some(i) => self.entries[i].auth.contains(id),
            None => id == owner,
        }
    }

    /// Store `rec`, dropping it instead when it is back to fresh. False when
    /// it needs a record and the store is full.
    fn put(&mut self, rec: TurretRec, owner: u32) -> bool {
        let fresh = rec == TurretRec::fresh(rec.key, owner);
        match self.index_of(rec.key) {
            Some(_) if fresh => {
                self.drop_key(rec.key);
                true
            }
            Some(i) => {
                self.entries[i] = rec;
                true
            }
            None if fresh => true,
            None if self.len == MAX_TURRET_RECS => false,
            None => {
                self.entries[self.len] = rec;
                self.len += 1;
                true
            }
        }
    }

    /// Forget a turret's record (it is gone). A key with none is a no-op.
    pub(crate) fn drop_key(&mut self, key: u32) {
        if let Some(i) = self.index_of(key) {
            self.len -= 1;
            self.entries[i] = self.entries[self.len];
            self.entries[self.len] = TurretRec::default();
        }
    }

    /// Every listed id through `f` (`World::remap_player_ids`).
    pub(crate) fn map_ids(&mut self, f: &mut impl FnMut(u32) -> u32) {
        for r in self.entries[..self.len].iter_mut() {
            r.auth.map_ids(f);
        }
    }

    /// Replace every record from a decoded world save. Boot-only.
    pub(crate) fn restore(&mut self, recs: &[TurretRec]) {
        self.len = recs.len().min(MAX_TURRET_RECS);
        self.entries[..self.len].copy_from_slice(&recs[..self.len]);
        for r in self.entries[self.len..].iter_mut() {
            *r = TurretRec::default();
        }
    }
}

/// Apply one turret op (`deploy::op_is_turret`) from `p` at turret `i`
/// (an index into the box store, already found to be a turret the presser
/// reaches). Putting yourself on, clearing and setting the mode take the
/// turret's building privilege ([`spares`]); taking yourself off takes
/// nothing, the crew leave's rule, so a refusal never tells a stranger
/// whether they were on.
/// Refusals are `EV_DEPLOY_REFUSED`; the list itself reaches the client in
/// its turret view while the box is open.
pub fn auth_op(
    pieces: &Pieces,
    deploys: &mut Deploys,
    i: usize,
    p: &Player,
    op: u8,
    events: &mut EventQueue,
) {
    use crate::deploy::{
        ACCESS_OP_TURRET_ATTACK_ALL, ACCESS_OP_TURRET_AUTH, ACCESS_OP_TURRET_CLEAR,
        ACCESS_OP_TURRET_DEAUTH, ACCESS_OP_TURRET_PEACEKEEPER, REFUSE_D_AUTH_FULL, REFUSE_D_FULL,
        REFUSE_D_KIND, REFUSE_D_OWNER,
    };
    use crate::roster::Added;
    use crate::world::EV_DEPLOY_REFUSED;
    let Some(b) = deploys.boxes().get(i).copied() else {
        events.push(EV_DEPLOY_REFUSED, p.id, REFUSE_D_KIND, 0);
        return;
    };
    let key = box_key(b.cx, b.cz, b.level, b.loc);
    let (x, z) = b.xz();
    let privileged = spares(pieces, deploys, b.owner, x, z, p.id);
    let mut rec = deploys.turret_recs().get(key, b.owner);
    match op {
        ACCESS_OP_TURRET_DEAUTH => {
            rec.auth.remove(p.id);
        }
        _ if !privileged => {
            events.push(EV_DEPLOY_REFUSED, p.id, REFUSE_D_OWNER, 0);
            return;
        }
        ACCESS_OP_TURRET_AUTH => {
            if rec.auth.add(p.id) == Added::Full {
                events.push(EV_DEPLOY_REFUSED, p.id, REFUSE_D_AUTH_FULL, 0);
                return;
            }
        }
        ACCESS_OP_TURRET_CLEAR => rec.auth.clear(),
        ACCESS_OP_TURRET_PEACEKEEPER => rec.peacekeeper = true,
        ACCESS_OP_TURRET_ATTACK_ALL => rec.peacekeeper = false,
        _ => {
            events.push(EV_DEPLOY_REFUSED, p.id, REFUSE_D_KIND, 0);
            return;
        }
    }
    if !deploys.turret_recs_mut().put(rec, b.owner) {
        events.push(EV_DEPLOY_REFUSED, p.id, REFUSE_D_FULL, 0);
    }
}

/// One table entry: which deployable it is, and its gun's brain.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Turret {
    pub live: bool,
    pub cx: u16,
    pub cz: u16,
    pub level: u8,
    pub loc: u8,
    pub owner: u32,
    pub gun: Sentry,
    /// The gun item it is firing ([`gun_in`]), for the wire to mount on it;
    /// `None` with no gun in its box.
    pub carried: Option<u16>,
}

impl Turret {
    /// Every field, little-endian, in declaration order.
    pub fn hash_bytes(&self) -> [u8; 37] {
        let mut b = [0u8; 37];
        b[0] = self.live as u8;
        b[1..3].copy_from_slice(&self.cx.to_le_bytes());
        b[3..5].copy_from_slice(&self.cz.to_le_bytes());
        b[5] = self.level;
        b[6] = self.loc;
        b[7..11].copy_from_slice(&self.owner.to_le_bytes());
        b[11..34].copy_from_slice(&self.gun.hash_bytes());
        b[34] = self.carried.is_some() as u8;
        b[35..37].copy_from_slice(&self.carried.unwrap_or(0).to_le_bytes());
        b
    }

    fn holds(&self, cx: u16, cz: u16, level: u8, loc: u8) -> bool {
        self.live && self.cx == cx && self.cz == cz && self.level == level && self.loc == loc
    }
}

/// Bring the table in line with the deploy store: free an entry whose
/// turret is gone, then give every turret record without one the first free
/// entry, in store order. Bounded: `MAX_TURRETS` finds and one store walk.
///
/// **A full table gives way to a loaded gun.** First come held an entry
/// whether or not the gun had a round in it, so turrets parked dry anywhere
/// on the shard silenced every turret placed after them. Once no entry is
/// free, a turret with `ammo` in its box takes the first entry whose own box
/// has none; the dry one stands inert until there is room again. The box
/// probes run only on that full-table path, and stop once no dry entry is
/// left.
pub fn reconcile(
    dc: &DeployContent,
    deploys: &Deploys,
    def: &TurretDef,
    table: &mut [Turret; MAX_TURRETS],
    slots: &mut [Mob],
) {
    let is_turret =
        |row: u8| (row as usize) < dc.defs.len() && dc.defs[row as usize].arch == ARCH_TURRET;
    for (t, m) in table.iter_mut().zip(slots.iter_mut()) {
        if t.live
            && !deploys
                .find(t.cx, t.cz, t.level, t.loc)
                .is_some_and(|d| is_turret(d.row))
        {
            *t = Turret::default();
            m.alive = false;
        }
    }
    let mut dry: Option<[bool; MAX_TURRETS]> = None;
    for d in deploys.entries() {
        if !is_turret(d.row) || table.iter().any(|t| t.holds(d.cx, d.cz, d.level, d.loc)) {
            continue;
        }
        let k = match table.iter().position(|t| !t.live) {
            Some(k) => k,
            None => {
                let dry = dry.get_or_insert_with(|| {
                    let mut dry = [false; MAX_TURRETS];
                    for (d, t) in dry.iter_mut().zip(table.iter()) {
                        *d = t.live
                            && loaded(def, deploys, box_key(t.cx, t.cz, t.level, t.loc)).is_none();
                    }
                    dry
                });
                let Some(j) = dry.iter().position(|&d| d) else {
                    return;
                };
                if loaded(def, deploys, box_key(d.cx, d.cz, d.level, d.loc)).is_none() {
                    continue;
                }
                dry[j] = false;
                slots[j].alive = false;
                j
            }
        };
        table[k] = Turret {
            live: true,
            cx: d.cx,
            cz: d.cz,
            level: d.level,
            loc: d.loc,
            owner: d.owner,
            gun: Sentry::default(),
            carried: None,
        };
        // It starts facing the way it was placed.
        slots[k].yaw = (d.pose.yaw as u16) << 8;
    }
}

/// One tick of every player turret. A round that landed on somebody is
/// written to `rounds` for `World::tick` to land (`sentry::step`'s split);
/// each round fired, hit or miss, spends one of its gun's round from the box.
#[allow(clippy::too_many_arguments)]
pub fn step(
    seed: u64,
    haven: &Haven,
    tick: u64,
    def: &TurretDef,
    dc: &DeployContent,
    pieces: &Pieces,
    occ: &mut Occupants,
    deploys: &mut Deploys,
    table: &mut [Turret; MAX_TURRETS],
    slots: &mut [Mob],
    players: &[Player; MAX_PLAYERS],
    events: &mut EventQueue,
    rounds: &mut [Option<Round>; MAX_TURRETS],
) {
    *rounds = [None; MAX_TURRETS];
    if !def.armed() {
        for (t, m) in table.iter_mut().zip(slots.iter_mut()) {
            *t = Turret::default();
            m.alive = false;
        }
        return;
    }
    let cols: &ColIndex = pieces.cols();
    if tick.is_multiple_of(LOOK_TICKS) {
        reconcile(dc, deploys, def, table, slots);
    }
    // Each entry's record and box, the gun in that box, and whether it has a
    // round for it: a dry gun neither shoots nor jams the guns around it.
    let mut recs = [None; MAX_TURRETS];
    let mut boxes = [None; MAX_TURRETS];
    let mut hot = [false; MAX_TURRETS];
    for k in 0..MAX_TURRETS {
        let t = table[k];
        if t.live {
            recs[k] = deploys.find(t.cx, t.cz, t.level, t.loc).copied();
            boxes[k] = deploys.box_index(box_key(t.cx, t.cz, t.level, t.loc));
            let g = gun_in(def, deploys, boxes[k]);
            table[k].carried = g.map(|g| def.guns[g].item);
            hot[k] = recs[k].is_some()
                && g.is_some_and(|g| rounds_in(deploys, boxes[k], def.guns[g].ammo) > 0);
        }
    }
    for k in 0..MAX_TURRETS {
        let t = table[k];
        if !t.live {
            continue;
        }
        let Some(rec) = recs[k] else {
            continue;
        };
        let (x, z) = rec.xz();
        let gun_y = crate::deploy::body_base_y(seed, haven, cols, &rec, ARCH_TURRET) + GUN_M;
        let slot = &mut slots[k];
        // The record's feet sit `ARROW_EYE_MM` under the muzzle, the
        // sentry's convention, so the client draws the round from the gun.
        slot.body = Body {
            qx: (x / POS_XZ_Q) as i32,
            qy: ((gun_y * MM_PER_M - ARROW_EYE_MM as f32) / (POS_Y_Q * MM_PER_M)) as i32,
            qz: (z / POS_XZ_Q) as i32,
            qvy: 0,
            grounded: true,
        };
        slot.alive = true;
        let gun = (
            (slot.body.qx * (POS_XZ_Q * MM_PER_M) as i32) as f32,
            (slot.body.qy * (POS_Y_Q * MM_PER_M) as i32 + ARROW_EYE_MM) as f32,
            (slot.body.qz * (POS_XZ_Q * MM_PER_M) as i32) as f32,
        );
        let bi = boxes[k];
        let s = &mut table[k].gun;
        // No gun, or a dry one, lets its target go and looks at nobody; so
        // does one jammed by the turrets around it (module doc).
        let Some(g) = gun_in(def, deploys, bi).filter(|_| hot[k]) else {
            s.drop_target();
            continue;
        };
        if jammed(&recs, &hot, k, (x, z)) {
            s.drop_target();
            continue;
        }
        let g = &def.guns[g];
        // Looks are spread over the cadence by index, so the turrets do not
        // all walk their sight lines on one tick.
        if (tick + k as u64).is_multiple_of(LOOK_TICKS) {
            // Its own list and mode, read once for every player asked.
            let rec = deploys
                .turret_recs()
                .get(box_key(t.cx, t.cz, t.level, t.loc), t.owner);
            let wanted = |p: &Player| {
                p.active
                    && !p.dead
                    && !p.sleeping
                    && p.hp > 0
                    && !crate::combat::protected(p)
                    && rec.shoots(p)
            };
            sentry::look(
                seed,
                haven,
                tick,
                &g.def,
                cols,
                occ,
                s,
                TURRET_SLOT0 + k,
                gun,
                players,
                events,
                &wanted,
            );
        }
        if s.engaged() {
            let p = &players[s.target as usize];
            let (px, pz) = (
                p.body.qx as f32 * POS_XZ_Q * MM_PER_M,
                p.body.qz as f32 * POS_XZ_Q * MM_PER_M,
            );
            let want = nav::yaw_toward(px - gun.0, pz - gun.2, slot.yaw);
            slot.yaw = nav::turn_toward(slot.yaw, want, sentry::TURN_STEP);
        }
        let before = s.next_shot;
        rounds[k] = sentry::fire(
            seed,
            haven,
            tick,
            &g.def,
            cols,
            occ,
            s,
            (TURRET_SLOT0 + k, (TURRET_SLOT0 + k) as i32),
            gun,
            slot,
            players,
            events,
        );
        if s.next_shot != before {
            if let Some(i) = bi {
                spend_one(deploys, i, g.ammo);
            }
        }
    }
}

/// Whether turret `k` is past [`TURRET_INTERFERENCE`] among the loaded
/// (`hot`) turrets within [`TURRET_INTERFERENCE_M`] of it: the ones before
/// it in table order win, which is stable for as long as they all stand. A
/// dry turret counts for nothing: twelve parked empty beside a base would
/// otherwise silence its guns.
fn jammed(
    recs: &[Option<crate::deploy::DeployRec>; MAX_TURRETS],
    hot: &[bool; MAX_TURRETS],
    k: usize,
    at: (f32, f32),
) -> bool {
    let r2 = TURRET_INTERFERENCE_M * TURRET_INTERFERENCE_M;
    recs[..k]
        .iter()
        .zip(hot.iter())
        .filter_map(|(r, &h)| r.filter(|_| h))
        .filter(|r| {
            let (x, z) = r.xz();
            let (dx, dz) = (x - at.0, z - at.1);
            dx * dx + dz * dz <= r2
        })
        .count()
        >= TURRET_INTERFERENCE
}

/// The first gun in box `bi` a turret fires, as its index in `def.guns`.
pub fn gun_in(def: &TurretDef, deploys: &Deploys, bi: Option<usize>) -> Option<usize> {
    let i = bi?;
    (0..BOX_SLOTS).find_map(|s| {
        let st = deploys.box_slot(i, s);
        if st.count == 0 {
            return None;
        }
        def.live().iter().position(|g| g.item == st.item)
    })
}

/// The gun in the box at `key`, if the box also holds a round for it.
fn loaded(def: &TurretDef, deploys: &Deploys, key: u32) -> Option<usize> {
    let bi = deploys.box_index(key);
    gun_in(def, deploys, bi).filter(|&g| rounds_in(deploys, bi, def.guns[g].ammo) > 0)
}

/// Rounds of `ammo` in box `bi`.
fn rounds_in(deploys: &Deploys, bi: Option<usize>, ammo: u16) -> u32 {
    bi.map_or(0u32, |i| {
        (0..BOX_SLOTS)
            .map(|s| deploys.box_slot(i, s))
            .filter(|s| s.count > 0 && s.item == ammo)
            .map(|s| s.count as u32)
            .sum()
    })
}

/// Take one `item` out of box `i`, from the first stack holding it.
fn spend_one(deploys: &mut Deploys, i: usize, item: u16) {
    for s in 0..BOX_SLOTS {
        let mut st = deploys.box_slot(i, s);
        if st.count > 0 && st.item == item {
            st.count -= 1;
            if st.count == 0 {
                st = ItemStack::default();
            }
            deploys.set_box_slot(i, s, st);
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_turret_slots_sit_under_the_sentries_and_are_guns() {
        for k in 0..MAX_TURRETS {
            let slot = TURRET_SLOT0 + k;
            assert!(is_turret_slot(slot));
            assert!(!sentry::is_sentry_slot(slot));
            assert_eq!(crate::mob::kind_of(slot), crate::mob::MOB_SENTRY);
            assert_eq!(crate::mob::ordinal(slot), None);
        }
        assert!(!is_turret_slot(TURRET_SLOT0 - 1));
        assert!(!is_turret_slot(SENTRY_SLOT0));
    }

    /// Rust's interference: fourteen turrets in one base, and the first
    /// twelve in table order shoot; a fifteenth across the island is its
    /// own base and shoots too.
    #[test]
    fn past_twelve_turrets_within_forty_metres_the_rest_stand_idle() {
        let mut recs = [None; MAX_TURRETS];
        for (k, r) in recs.iter_mut().take(14).enumerate() {
            *r = Some(crate::deploy::DeployRec {
                cx: 500 + (k % 4) as u16,
                cz: 500 + (k / 4) as u16,
                ..Default::default()
            });
        }
        recs[20] = Some(crate::deploy::DeployRec {
            cx: 900,
            cz: 900,
            ..Default::default()
        });
        let hot = [true; MAX_TURRETS];
        let at = |k: usize| recs[k].unwrap().xz();
        for k in 0..14 {
            assert_eq!(
                jammed(&recs, &hot, k, at(k)),
                k >= TURRET_INTERFERENCE,
                "turret {k}"
            );
        }
        assert!(!jammed(&recs, &hot, 20, at(20)), "a far base is its own");
    }

    /// Twelve dry turrets ahead of a loaded one in the table, beside it, do
    /// not jam it (exploit sweep 2, pve#4).
    #[test]
    fn dry_turrets_do_not_jam_a_loaded_one() {
        let mut recs = [None; MAX_TURRETS];
        let mut hot = [false; MAX_TURRETS];
        for (k, r) in recs.iter_mut().take(TURRET_INTERFERENCE + 1).enumerate() {
            *r = Some(crate::deploy::DeployRec {
                cx: 500 + (k % 4) as u16,
                cz: 500 + (k / 4) as u16,
                ..Default::default()
            });
        }
        let k = TURRET_INTERFERENCE;
        hot[k] = true;
        let at = recs[k].unwrap().xz();
        assert!(!jammed(&recs, &hot, k, at), "dry guns jammed a loaded one");
        hot[..k].fill(true);
        assert!(jammed(&recs, &hot, k, at), "twelve loaded ones still do");
    }
}
