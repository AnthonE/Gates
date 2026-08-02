//! Combat — the verb that takes something away (DESIGN.md §2, M1). Until
//! this module, every hp in the world decayed and nothing attacked it: a
//! locked door was furniture and `content/weapons.toml` was data nothing
//! played. Melee v0 is the smallest honest fix — **the swing that already
//! fells a tree also lands on a person.**
//!
//! One verb, one gate: `gather::swing` owns the cadence and the target
//! pick for scatter; a swing that finds no node is handed here, and looks
//! for a player instead. Same reach shape, same aim cone, same tick — a
//! hatchet swung at a neighbour is not a second mechanic.
//!
//! Content reaches the sim only as a baked `CombatContent` table (CLAUDE.md
//! wall 7): per-item melee damage and reach from `content/weapons.toml`,
//! max hp from `content/balance.toml`'s `globals.player_hp` — the same
//! number `test_content`'s TTK anchor divides by, so the band the data
//! declares and the band the sim plays are one number, not two. The inert
//! `EMPTY` default makes combat a no-op (nothing has damage, nobody has
//! hp), and `probe_fixture()` is a synthetic table for the parity/replay/
//! alloc gates.
//!
//! Pure and fixed-capacity like the rest of the crate: the target scan is
//! one pass over the `MAX_PLAYERS` slot array, taken only on a swing tick
//! that missed every node, and it allocates nothing.
//!
//! **What v0 deliberately does not do**, all of it registered in
//! `DECISIONS.md` §open ("melee combat v0"): no headshots (aim is planar
//! until M2's rewound raycasts, so there is no head to hit), no armor
//! reduction, no per-weapon cadence (every swing rides gather's one
//! interval, which is the melee rows' own rate), no structure damage
//! (`weapons.toml` carries no melee-vs-structure column and inventing one
//! would move the raid ratio), and no corpse: death destroys what you
//! carried where the backpack will later drop it.

use crate::fmath::fabs;
use crate::gather::{CONE_COS, DY_MAX_M, NO_ITEM, POINT_BLANK_M2};
use crate::limits::{MAX_ITEM_DEFS, MAX_PLAYERS};
use crate::movement::{POS_XZ_Q, POS_Y_Q};
use crate::world::{EventQueue, Player, EV_DEATH, EV_HEALTH, EV_HIT};
use crate::yaw_lut::yaw_dir;

/// One item's melee row. `damage == 0` ⇒ the item is not a weapon (the
/// whole table starts that way), so a bare hand and a stack of wood are
/// the same swing: nothing. Reach is centimetres so the baked table
/// carries no float rounding of `content/weapons.toml`'s `range_m`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MeleeDef {
    pub damage: u16,
    pub reach_cm: u16,
}

/// The whole combat ruleset the sim knows. Construction input like the
/// seed and the gather table; the WAL pins the content hash it was baked
/// from (CONTENT.md §0).
#[derive(Clone, Copy, Debug)]
pub struct CombatContent {
    /// Indexed by item index (the sorted-rank mapping `bake` owns).
    pub melee: [MeleeDef; MAX_ITEM_DEFS],
    /// Max player hp — `content/balance.toml` `globals.player_hp`. Zero is
    /// the inert default and disarms the module entirely: no hp is granted
    /// at join, so no damage is applied and nobody can die.
    pub player_hp: u16,
}

impl CombatContent {
    pub const EMPTY: Self = Self {
        melee: [MeleeDef {
            damage: 0,
            reach_cm: 0,
        }; MAX_ITEM_DEFS],
        player_hp: 0,
    };

    /// Synthetic table for the parity/replay/alloc gates. Deliberately
    /// unlike game content: every hotbar-reachable fixture item is a
    /// weapon, and item 0 — which the gather fixture also makes a tool —
    /// kills in three hits, so hits, deaths and respawns land inside the
    /// counted windows instead of waiting on a lucky wander. Reach is the
    /// real melee rows' 2 m on purpose: a fixture that could hit across
    /// the island would put every herd gate quietly into a brawl.
    pub fn probe_fixture() -> Self {
        let mut c = Self::EMPTY;
        c.player_hp = 100;
        let rows: [u16; 4] = [34, 12, 25, 50];
        let mut i = 0;
        while i < rows.len() {
            c.melee[i] = MeleeDef {
                damage: rows[i],
                reach_cm: 200,
            };
            i += 1;
        }
        c
    }

    /// The melee row of the item a player is holding, or `None` when the
    /// hand is empty, the held item is off the table, or the item is not
    /// a weapon.
    #[inline]
    pub fn held_melee(&self, held: u16) -> Option<MeleeDef> {
        if held == NO_ITEM || held as usize >= MAX_ITEM_DEFS {
            return None;
        }
        let def = self.melee[held as usize];
        (def.damage > 0).then_some(def)
    }
}

impl Default for CombatContent {
    fn default() -> Self {
        Self::EMPTY
    }
}

/// The item in a player's selected hotbar slot, or `NO_ITEM` for an empty
/// hand — the same read `gather::swing` makes, kept in one place.
#[inline]
pub fn held_item(p: &Player) -> u16 {
    if p.inv[p.frame.sel as usize].count > 0 {
        p.inv[p.frame.sel as usize].item
    } else {
        NO_ITEM
    }
}

/// Resolve one already-taken swing (cadence paid, no node hit) against
/// the other players. Returns the **slot** of a player this strike killed,
/// if any — respawn is the caller's, because the spawn ring needs the
/// whole world and this function only needs the slot array.
///
/// Bounded: one pass over `MAX_PLAYERS`, on a swing tick only. Nearest
/// eligible target inside the weapon's reach and gather's aim cone wins,
/// exactly as a node does; the attacker is never a candidate, so no weapon
/// can ever hit its own holder.
pub fn strike(
    cc: &CombatContent,
    attacker: usize,
    players: &mut [Player; MAX_PLAYERS],
    events: &mut EventQueue,
) -> Option<usize> {
    if cc.player_hp == 0 {
        return None; // inert content: combat is not armed
    }
    let a = &players[attacker];
    if !a.active || a.hp == 0 {
        return None;
    }
    let def = cc.held_melee(held_item(a))?;
    let reach = def.reach_cm as f32 * 0.01;
    let ax = a.body.qx as f32 * POS_XZ_Q;
    let ay = a.body.qy as f32 * POS_Y_Q;
    let az = a.body.qz as f32 * POS_XZ_Q;
    let (fx, fz) = yaw_dir(a.frame.yaw);
    let attacker_id = a.id;

    let mut best: Option<(f32, usize)> = None;
    for (j, t) in players.iter().enumerate() {
        if j == attacker || !t.active || t.hp == 0 {
            continue;
        }
        let dx = t.body.qx as f32 * POS_XZ_Q - ax;
        let dy = t.body.qy as f32 * POS_Y_Q - ay;
        let dz = t.body.qz as f32 * POS_XZ_Q - az;
        let d2 = dx * dx + dz * dz;
        if d2 > reach * reach || fabs(dy) > DY_MAX_M {
            continue;
        }
        // Standing inside someone has no bearing to test, same rule the
        // node scan uses.
        let aimed = d2 <= POINT_BLANK_M2 || dx * fx + dz * fz > CONE_COS * d2.sqrt();
        if aimed && best.is_none_or(|(bd2, _)| d2 < bd2) {
            best = Some((d2, j));
        }
    }
    let (_, victim) = best?;

    let v = &mut players[victim];
    let victim_id = v.id;
    let died = def.damage >= v.hp;
    v.hp -= def.damage.min(v.hp);
    let left = v.hp;
    if died {
        v.deaths = v.deaths.saturating_add(1);
    }
    events.push(EV_HIT, attacker_id, victim_id, def.damage as u32);
    events.push(EV_HEALTH, victim_id, left as u32, cc.player_hp as u32);
    if died {
        events.push(EV_DEATH, victim_id, attacker_id, 0);
        return Some(victim);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gather::ItemStack;

    #[test]
    fn held_melee_refuses_hands_junk_and_the_table_edge() {
        let cc = CombatContent::probe_fixture();
        assert_eq!(cc.held_melee(0).map(|d| d.damage), Some(34));
        assert_eq!(cc.held_melee(NO_ITEM), None, "a bare hand is not a weapon");
        assert_eq!(cc.held_melee(9), None, "an item with no weapon row");
        assert_eq!(
            cc.held_melee(MAX_ITEM_DEFS as u16),
            None,
            "an index past the table"
        );
    }

    #[test]
    fn held_item_reads_the_selected_slot_only() {
        let mut p = Player::default();
        p.inv[0] = ItemStack { item: 3, count: 1 };
        p.inv[2] = ItemStack { item: 7, count: 5 };
        assert_eq!(held_item(&p), 3);
        p.frame.sel = 2;
        assert_eq!(held_item(&p), 7);
        p.frame.sel = 1;
        assert_eq!(held_item(&p), NO_ITEM, "an empty slot is an empty hand");
    }

    #[test]
    fn inert_content_never_hurts_anyone() {
        let cc = CombatContent::EMPTY;
        let mut players = [Player::default(); MAX_PLAYERS];
        for (i, p) in players.iter_mut().take(2).enumerate() {
            p.id = i as u32 + 1;
            p.active = true;
            p.hp = 100;
            p.inv[0] = ItemStack { item: 0, count: 1 };
        }
        let mut ev = EventQueue::default();
        assert_eq!(strike(&cc, 0, &mut players, &mut ev), None);
        assert_eq!(players[1].hp, 100);
        assert!(ev.is_empty());
    }
}
