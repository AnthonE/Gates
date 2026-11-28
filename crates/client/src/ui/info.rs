//! The item card on the inventory page — Rust's item info panel. Click a
//! slot and the card over your pack says what the stack is: a name, a class
//! strip, its description, an INFORMATION column of numbers with bars, and
//! the ACTIONS it takes (eat, wear, unload, split, drop).
//!
//! Everything a card shows is already on the client: the catalog row
//! (`protocol::event::ItemRow`) carries the damage (v131), the gun handling,
//! the armor, the eat columns and the condition ceiling.
//!
//! The bars are drawn against fixed presentation ceilings ([`RPM_CAP`] and
//! friends), not against the strongest item in the catalog, so a bar means
//! the same length on every card and a new gun does not shrink the others.

use protocol::event::{ItemCatalog, ItemRow};
use sim_core::combat::{ARMOR_MAX_PCT, WEAR_NONE};
use sim_core::craft::{repair_cost, repair_recipe, repair_station, CraftContent};
use sim_core::gather::ItemStack;
use sim_core::inventory::{CONT_SELF, CONT_WEAR};
use sim_core::limits::{INV_SLOTS, MAX_RECIPE_INPUTS, TICK_HZ};

/// One INFORMATION row: a label, a value and, for a number with a sensible
/// ceiling, how full its bar is (0..=1).
#[derive(Clone, Debug, PartialEq)]
pub struct Stat {
    pub label: &'static str,
    pub value: String,
    pub frac: Option<f32>,
}

/// Fire rate a full bar stands for, rounds a minute.
pub const RPM_CAP: f32 = 900.0;
/// An aim or hip cone a full bar stands for, hundredths of a degree (5°).
pub const CONE_CAP: f32 = 500.0;
/// A recoil kick a full bar stands for, hundredths of a degree (8°).
pub const RECOIL_CAP: f32 = 800.0;
/// A reload or draw a full bar stands for, seconds.
pub const WAIT_CAP_S: f32 = 6.0;
/// What one bite a full bar stands for (food, water or health).
pub const EAT_CAP: f32 = 100.0;
/// The damage of one shot or swing a full bar stands for — a shotgun's
/// whole spread (14 × 15) sits near the top, a bat (20) near the foot.
pub const DAMAGE_CAP: f32 = 250.0;
/// The warmth one piece fills its bar at: the snow jacket's (clothing v1).
pub const WARMTH_CAP: f32 = 90.0;

fn frac(v: f32, cap: f32) -> Option<f32> {
    Some((v / cap).clamp(0.0, 1.0))
}

fn secs(ticks: u32) -> f32 {
    ticks as f32 / TICK_HZ as f32
}

/// The worn slot's word, by `combat::WEAR_*`.
pub fn worn_on(wear_slot: u8) -> Option<&'static str> {
    match wear_slot {
        sim_core::combat::WEAR_NONE => None,
        w => Some(crate::ui::slots::wear_slot_label(w as usize - 1)).filter(|l| *l != "-"),
    }
}

/// The INFORMATION column for `stack`, whose catalog row is `row`.
/// `loaded` is the rounds in the gun when the card is the gun in your hand.
pub fn stats(row: ItemRow, stack: ItemStack, loaded: Option<u16>) -> Vec<Stat> {
    let mut out = Vec::new();
    if row.damage > 0 {
        // A shot of pellets is that many hits: said as both numbers, and
        // its bar the whole spread landing.
        let pellets = row.gun.map_or(1, |g| g.pellets.max(1));
        out.push(Stat {
            label: "DAMAGE",
            value: if pellets > 1 {
                format!("{} × {pellets}", row.damage)
            } else {
                row.damage.to_string()
            },
            frac: frac(row.damage as f32 * pellets as f32, DAMAGE_CAP),
        });
    }
    if let Some(g) = row.gun {
        // The exact cadence when the row has one (`rate_sub`, 256ths of a
        // tick), as the sim fires it; whole ticks overstated the Thompson
        // at 600 rpm against its 462.
        let ticks = if g.rate_sub > 0 {
            g.rate_sub as f32 / 256.0
        } else {
            g.rate_ticks.max(1) as f32
        };
        let rpm = (60 * TICK_HZ) as f32 / ticks;
        out.push(Stat {
            label: "FIRE RATE",
            value: format!("{rpm:.0} rpm"),
            frac: frac(rpm, RPM_CAP),
        });
        out.push(Stat {
            label: "AIM CONE",
            value: format!("{:.2}°", g.aim as f32 / 100.0),
            frac: frac(g.aim as f32, CONE_CAP),
        });
        out.push(Stat {
            label: "HIP CONE",
            value: format!("{:.2}°", g.hip as f32 / 100.0),
            frac: frac(g.hip as f32, CONE_CAP),
        });
        let kick = (g.recoil_pitch[0] as f32 + g.recoil_pitch[1] as f32) / 2.0;
        out.push(Stat {
            label: "RECOIL",
            value: format!("{:.1}°", kick / 100.0),
            frac: frac(kick, RECOIL_CAP),
        });
        if g.reload_ticks > 0 {
            let s = secs(g.reload_ticks as u32);
            out.push(Stat {
                label: "RELOAD",
                value: format!("{s:.1} s"),
                frac: frac(s, WAIT_CAP_S),
            });
        }
        if let Some(n) = loaded {
            out.push(Stat {
                label: "LOADED",
                value: n.to_string(),
                frac: None,
            });
        }
    } else if row.draws() {
        let s = secs(row.draw_ticks as u32);
        out.push(Stat {
            label: "DRAW",
            value: format!("{s:.1} s"),
            frac: frac(s, WAIT_CAP_S),
        });
    }
    if row.wear_slot != WEAR_NONE {
        out.push(Stat {
            label: "PROTECTION",
            value: format!("{}%", row.armor_pct),
            frac: frac(row.armor_pct as f32, ARMOR_MAX_PCT as f32),
        });
        // Rust's Cold row (clothing v1): the share of the cold it keeps
        // out, or lets in for a sheet of metal; its bar is a third of a
        // full winter kit.
        if row.warmth_pct != 0 {
            out.push(Stat {
                label: "WARMTH",
                value: format!("{:+}%", row.warmth_pct),
                frac: frac(row.warmth_pct.max(0) as f32, WARMTH_CAP),
            });
        }
        if row.ward_pct > 0 {
            out.push(Stat {
                label: "VERDIGRIS",
                value: format!("{}%", row.ward_pct),
                frac: frac(row.ward_pct as f32, 100.0),
            });
        }
        if let Some(on) = worn_on(row.wear_slot) {
            out.push(Stat {
                label: "WORN ON",
                value: on.to_string(),
                frac: None,
            });
        }
    }
    for (n, label) in [
        (row.food, "CALORIES"),
        (row.water, "HYDRATION"),
        (row.health, "HEALING"),
    ] {
        if n > 0 {
            out.push(Stat {
                label,
                value: format!("+{n}"),
                frac: frac(n as f32, EAT_CAP),
            });
        }
    }
    if row.cond_max > 0 {
        let pct = (stack.cond as u32 * 100 / row.cond_max as u32).min(100);
        out.push(Stat {
            label: "CONDITION",
            value: format!("{pct}%"),
            frac: Some(pct as f32 / 100.0),
        });
    }
    if row.stack_max > 1 {
        out.push(Stat {
            label: "STACK",
            value: format!("{} / {}", stack.count, row.stack_max),
            frac: frac(stack.count as f32, row.stack_max as f32),
        });
    }
    out
}

/// A verb on the card. `Quick` is the slot's right-click, whatever that is
/// where the player stands (eat, wear, take off, read, or move across), so
/// the card and the gesture can never disagree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    Quick,
    Unload,
    /// Put this attachment on the gun in hand.
    Attach,
    /// Take the gun in hand's attachment `bit` off.
    Detach(u8),
    /// Mend this worn item at a workbench (`craft::repair_item`).
    Repair,
    Split,
    Drop,
}

/// What a gun's attachment slot is called on the card, by
/// `combat::MOD_*` (Rust's mod slots).
pub const MOD_SLOT_NAMES: [&str; 4] = ["SIGHT", "MUZZLE", "UNDERBARREL", "MAGAZINE"];
/// The verb that takes each slot's attachment off.
const DETACH_LABELS: [&str; 4] = [
    "Take off sight",
    "Take off muzzle",
    "Take off underbarrel",
    "Take off magazine",
];

/// What the card's ACTIONS column needs to know about the picked slot.
#[derive(Clone, Copy, Debug)]
pub struct Ctx {
    /// The slot's container (`CONT_*`).
    pub kind: u8,
    /// A ground container is open beside the pack.
    pub looting: bool,
    /// The stack is a blueprint the player can read.
    pub paper: bool,
    /// The stack is the gun in your hand and it has rounds in it.
    pub loaded_in_hand: bool,
    /// The pack has an empty slot for a split.
    pub room: bool,
    /// The stack is an attachment the gun in hand takes and is not wearing.
    pub attachable: bool,
    /// The stack is the gun in hand, and the attachment bit in each of its
    /// slots (`combat::MOD_*`).
    pub fitted: [Option<u8>; 4],
    /// The stack is worn and a recipe prices its mend ([`Repair::Quote`]).
    pub repair: bool,
}

/// The ACTIONS column, top to bottom: the right-click's verb first, Drop
/// last (Rust's order).
pub fn actions(row: ItemRow, stack: ItemStack, ctx: Ctx) -> Vec<(Act, &'static str)> {
    let mut out = Vec::new();
    if stack.count == 0 {
        return out;
    }
    let quick = match ctx.kind {
        CONT_SELF if ctx.looting => Some("Move across"),
        CONT_SELF if ctx.paper => Some("Learn blueprint"),
        CONT_SELF if row.wear_slot != WEAR_NONE => Some("Wear"),
        CONT_SELF if row.food > 0 || row.water > 0 => Some("Eat"),
        CONT_SELF if row.health > 0 => Some("Use"),
        CONT_SELF => None,
        CONT_WEAR if ctx.looting => Some("Move across"),
        CONT_WEAR => Some("Take off"),
        _ => Some("Take"),
    };
    if let Some(label) = quick {
        out.push((Act::Quick, label));
    }
    if ctx.kind == CONT_SELF {
        if ctx.attachable {
            out.push((Act::Attach, "Attach to gun"));
        }
        for (group, bit) in ctx.fitted.iter().enumerate() {
            if let Some(bit) = bit {
                out.push((Act::Detach(*bit), DETACH_LABELS[group]));
            }
        }
        if ctx.loaded_in_hand {
            out.push((Act::Unload, "Unload ammo"));
        }
        if ctx.repair {
            out.push((Act::Repair, "Repair"));
        }
        if stack.count > 1 && ctx.room {
            out.push((Act::Split, "Split stack"));
        }
        out.push((Act::Drop, "Drop"));
    }
    out
}

/// What the card says of mending a stack at a workbench.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Repair {
    /// Whole, an item that never wears, or a table not dripped in yet.
    Nothing,
    /// Worn, and no recipe makes it (loot only): it can never be mended.
    Never,
    /// Worn: recipe row `recipe` prices the mend at `rows[..n]`, the sim's
    /// own `craft::repair_cost`.
    Quote {
        recipe: u16,
        rows: [(u16, u32); MAX_RECIPE_INPUTS],
        n: usize,
    },
}

/// The bench repair of `stack`, whose item's ceiling is `cond_max`, priced
/// off the client's recipe table (the drip carries the share, wire v157).
pub fn repair(cc: &CraftContent, cond_max: u16, stack: ItemStack) -> Repair {
    if stack.count == 0 || cond_max == 0 || stack.cond >= cond_max || cc.repair_pct == 0 {
        return Repair::Nothing;
    }
    let Some(recipe) = repair_recipe(cc, stack.item) else {
        return Repair::Never;
    };
    let mut rows = [(0, 0); MAX_RECIPE_INPUTS];
    let def = &cc.recipes[recipe as usize];
    match repair_cost(
        def,
        stack.cond,
        cond_max,
        stack.count,
        cc.repair_pct,
        &mut rows,
    ) {
        0 => Repair::Nothing,
        n => Repair::Quote { recipe, rows, n },
    }
}

/// The card's line under its verbs: what a mend costs, or that there is
/// none to be had.
pub fn repair_line(catalog: &ItemCatalog, repair: Repair) -> Option<String> {
    match repair {
        Repair::Nothing => None,
        Repair::Never => Some("Nothing makes it, so it cannot be repaired.".to_string()),
        Repair::Quote { rows, n, .. } => {
            let cost: Vec<String> = rows[..n]
                .iter()
                .map(|&(item, units)| {
                    format!("{units} {}", crate::ui::craft::item_label(catalog, item))
                })
                .collect();
            Some(format!("Repair at a workbench: {}", cost.join(", ")))
        }
    }
}

/// Why the shard would refuse this mend where the player stands, in its own
/// order (`craft::repair_item`): the blueprint, the bench (`best`, the
/// highest rung in reach), then the pack. `None` when it would go.
pub fn repair_blocked(
    cc: &CraftContent,
    known: u128,
    best: u8,
    inv: &[ItemStack; INV_SLOTS],
    catalog: &ItemCatalog,
    recipe: u16,
    rows: &[(u16, u32)],
) -> Option<String> {
    let def = &cc.recipes[recipe as usize];
    if def.blueprint && !sim_core::research::knows(known, recipe) {
        return Some("learn its blueprint to repair it".to_string());
    }
    let tier = repair_station(def);
    if best < tier {
        return Some(match tier {
            sim_core::craft::STATION_WORKBENCH1 => "repairing needs a workbench nearby".to_string(),
            t => format!("repairing this needs a level {t} workbench nearby"),
        });
    }
    let short = rows
        .iter()
        .find(|&&(item, units)| crate::ui::craft::spendable(inv, item, catalog) < units)?;
    Some(format!(
        "not enough {} to repair it",
        crate::ui::craft::item_label(catalog, short.0)
    ))
}

/// The strip under the card's name: the skin, or else the class and, for a
/// blueprint, that it is one.
pub fn strip(class: &'static str, skin: Option<String>, paper: bool) -> String {
    match (skin, paper) {
        (Some(s), _) => s.to_uppercase(),
        (None, true) => "BLUEPRINT".to_string(),
        (None, false) => class.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::event::GunRow;
    use sim_core::inventory::CONT_BOX;

    fn stack(count: u16, cond: u16) -> ItemStack {
        ItemStack {
            item: 1,
            count,
            cond,
            ..ItemStack::default()
        }
    }

    fn ctx(kind: u8) -> Ctx {
        Ctx {
            kind,
            looting: false,
            paper: false,
            loaded_in_hand: false,
            room: true,
            attachable: false,
            fitted: [None; 4],
            repair: false,
        }
    }

    /// A worn item a recipe makes offers Repair and quotes its price; a
    /// whole one, one that never wears and one no recipe makes do not, and
    /// the last says it never can.
    #[test]
    fn the_card_quotes_a_bench_repair() {
        let mut cc = CraftContent::probe_fixture();
        // Recipe 1 makes item 3 from two of item 1 and one of item 2.
        cc.repair_pct = 20;
        let worn = ItemStack {
            item: 3,
            count: 1,
            cond: 0,
            ..ItemStack::default()
        };
        let Repair::Quote { recipe, rows, n } = repair(&cc, 100, worn) else {
            panic!("a broken item a recipe makes is quoted");
        };
        assert_eq!((recipe, n), (1, 2));
        assert_eq!(&rows[..n], &[(1, 1), (2, 1)], "20% of each, rounded up");
        assert_eq!(
            repair(&cc, 100, ItemStack { cond: 100, ..worn }),
            Repair::Nothing
        );
        assert_eq!(repair(&cc, 0, worn), Repair::Nothing, "never wears");
        assert_eq!(
            repair(&cc, 100, ItemStack { item: 9, ..worn }),
            Repair::Never
        );
        let unbaked = CraftContent {
            repair_pct: 0,
            ..cc
        };
        assert_eq!(repair(&unbaked, 100, worn), Repair::Nothing);

        let acts = actions(
            ItemRow::EMPTY,
            worn,
            Ctx {
                repair: true,
                ..ctx(CONT_SELF)
            },
        );
        assert!(acts.iter().any(|a| a.0 == Act::Repair));
        let acts = actions(
            ItemRow::EMPTY,
            worn,
            Ctx {
                repair: true,
                ..ctx(CONT_BOX)
            },
        );
        assert!(
            acts.iter().all(|a| a.0 != Act::Repair),
            "only from the pack"
        );
    }

    /// The card says why a mend would be refused, in the shard's order.
    #[test]
    fn a_blocked_repair_says_why() {
        let cc = CraftContent::probe_fixture();
        let catalog = ItemCatalog::default();
        let mut inv = [ItemStack::default(); INV_SLOTS];
        inv[0] = ItemStack {
            item: 0,
            count: 5,
            ..ItemStack::default()
        };
        let rows = [(0u16, 1u32)];
        let wb1 = sim_core::craft::STATION_WORKBENCH1;
        // Recipe 2 is a blueprint at a level-1 bench.
        let why = |known, best, inv: &[ItemStack; INV_SLOTS], rows: &[(u16, u32)]| {
            repair_blocked(&cc, known, best, inv, &catalog, 2, rows)
        };
        assert!(why(0, wb1, &inv, &rows).unwrap().contains("blueprint"));
        assert!(why(1 << 2, 0, &inv, &rows).unwrap().contains("workbench"));
        assert!(why(1 << 2, wb1, &inv, &[(0, 6)])
            .unwrap()
            .contains("not enough"));
        assert_eq!(why(1 << 2, wb1, &inv, &rows), None);
    }

    /// An attachment the gun in hand takes offers itself; a gun in hand
    /// offers each fitted attachment off, by its slot.
    #[test]
    fn the_card_offers_to_attach_and_take_off() {
        let scope = ItemRow {
            stack_max: 1,
            ..ItemRow::EMPTY
        };
        let acts = actions(
            scope,
            stack(1, 0),
            Ctx {
                attachable: true,
                ..ctx(CONT_SELF)
            },
        );
        assert!(acts.iter().any(|a| a.0 == Act::Attach));
        let acts = actions(
            scope,
            stack(1, 0),
            Ctx {
                fitted: [Some(1), None, Some(5), None],
                ..ctx(CONT_SELF)
            },
        );
        let off: Vec<_> = acts
            .iter()
            .filter(|a| matches!(a.0, Act::Detach(_)))
            .collect();
        assert_eq!(off.len(), 2);
        assert_eq!(off[0].0, Act::Detach(1));
        assert_eq!(off[1], &(Act::Detach(5), "Take off underbarrel"));
        assert!(!actions(scope, stack(1, 0), ctx(CONT_SELF))
            .iter()
            .any(|a| matches!(a.0, Act::Attach | Act::Detach(_))));
    }

    #[test]
    fn a_gun_card_reads_its_handling_off_the_row() {
        let row = ItemRow {
            cond_max: 10_000,
            stack_max: 1,
            damage: 35,
            gun: Some(GunRow {
                rate_ticks: 5,
                reload_ticks: 102,
                semi: true,
                pellets: 1,
                aim: 75,
                hip: 200,
                recoil_pitch: [220, 260],
                recoil_yaw: [-40, 40],
                ads_recoil_pct: 50,
                ..GunRow::default()
            }),
            ..ItemRow::EMPTY
        };
        let s = stats(row, stack(1, 5_000), Some(8));
        let get = |l: &str| s.iter().find(|x| x.label == l).map(|x| x.value.clone());
        assert_eq!(get("FIRE RATE").as_deref(), Some("360 rpm"));
        assert_eq!(get("AIM CONE").as_deref(), Some("0.75°"));
        assert_eq!(get("RECOIL").as_deref(), Some("2.4°"));
        assert_eq!(get("RELOAD").as_deref(), Some("3.4 s"));
        assert_eq!(get("LOADED").as_deref(), Some("8"));
        assert_eq!(get("CONDITION").as_deref(), Some("50%"));
        // The exact cadence wins over the whole ticks: 3.9 ticks a shot at
        // 30 Hz is 462 rpm, not the 600 that 3 whole ticks would say.
        let exact = ItemRow {
            gun: row.gun.map(|g| GunRow {
                rate_ticks: 3,
                rate_sub: 998,
                ..g
            }),
            ..row
        };
        let s = stats(exact, stack(1, 5_000), Some(8));
        let rpm = s
            .iter()
            .find(|x| x.label == "FIRE RATE")
            .map(|x| x.value.clone());
        assert_eq!(rpm.as_deref(), Some("462 rpm"));
        assert_eq!(get("DAMAGE").as_deref(), Some("35"), "one pellet: no ×");
        assert_eq!(s[0].label, "DAMAGE", "damage heads the card, Rust's order");
        assert!(get("STACK").is_none(), "a gun does not stack");
        assert!(s
            .iter()
            .all(|x| x.frac.is_none_or(|f| (0.0..=1.0).contains(&f))));
        // A shotgun's shot is its pellets, and its bar the whole spread.
        let shotgun = ItemRow {
            damage: 14,
            gun: row.gun.map(|g| GunRow { pellets: 15, ..g }),
            ..row
        };
        let s = stats(shotgun, stack(1, 5_000), None);
        assert_eq!(s[0].value, "14 × 15");
        assert_eq!(s[0].frac, Some(210.0 / DAMAGE_CAP));
        // Nothing that does not hit says DAMAGE.
        assert!(stats(ItemRow::EMPTY, stack(1, 0), None)
            .iter()
            .all(|x| x.label != "DAMAGE"));
    }

    #[test]
    fn the_quick_verb_is_the_right_click_where_you_stand() {
        let food = ItemRow {
            food: 30,
            stack_max: 10,
            ..ItemRow::EMPTY
        };
        let hat = ItemRow {
            wear_slot: sim_core::combat::WEAR_HEAD,
            armor_pct: 10,
            stack_max: 1,
            ..ItemRow::EMPTY
        };
        let first = |row, kind, looting| {
            actions(
                row,
                stack(2, 0),
                Ctx {
                    looting,
                    ..ctx(kind)
                },
            )
            .first()
            .map(|a| a.1)
        };
        assert_eq!(first(food, CONT_SELF, false), Some("Eat"));
        assert_eq!(first(hat, CONT_SELF, false), Some("Wear"));
        assert_eq!(first(hat, CONT_WEAR, false), Some("Take off"));
        assert_eq!(first(food, CONT_SELF, true), Some("Move across"));
        assert_eq!(first(food, CONT_BOX, true), Some("Take"));
        // Wood has no right-click verb with nothing open: Split, then Drop.
        let wood = ItemRow {
            stack_max: 1000,
            ..ItemRow::EMPTY
        };
        let acts = actions(wood, stack(2, 0), ctx(CONT_SELF));
        assert_eq!(
            acts.iter().map(|a| a.0).collect::<Vec<_>>(),
            [Act::Split, Act::Drop]
        );
        // One of a thing does not split, and loot cannot be dropped from here.
        assert!(!actions(wood, stack(1, 0), ctx(CONT_SELF))
            .iter()
            .any(|a| a.0 == Act::Split));
        assert!(!actions(wood, stack(5, 0), ctx(CONT_BOX))
            .iter()
            .any(|a| a.0 == Act::Drop));
    }
}
