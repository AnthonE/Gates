//! The item card on the inventory page — Rust's item info panel. Click a
//! slot and the card over your pack says what the stack is: a name, a class
//! strip, its description, an INFORMATION column of numbers with bars, and
//! the ACTIONS it takes (eat, wear, unload, split, drop).
//!
//! Everything a card shows is already on the client: the catalog row
//! (`protocol::event::ItemRow`) carries the gun handling, the armor, the eat
//! columns and the condition ceiling. Damage is not on the wire, so the card
//! does not invent one.
//!
//! The bars are drawn against fixed presentation ceilings ([`RPM_CAP`] and
//! friends), not against the strongest item in the catalog, so a bar means
//! the same length on every card and a new gun does not shrink the others.

use protocol::event::ItemRow;
use sim_core::combat::{ARMOR_MAX_PCT, WEAR_NONE};
use sim_core::gather::ItemStack;
use sim_core::inventory::{CONT_SELF, CONT_WEAR};
use sim_core::limits::{MAX_PELLETS, TICK_HZ};

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

fn frac(v: f32, cap: f32) -> Option<f32> {
    Some((v / cap).clamp(0.0, 1.0))
}

fn secs(ticks: u32) -> f32 {
    ticks as f32 / TICK_HZ as f32
}

/// The worn slot's word, by `combat::WEAR_*`.
pub fn worn_on(wear_slot: u8) -> Option<&'static str> {
    match wear_slot {
        sim_core::combat::WEAR_HEAD => Some("HEAD"),
        sim_core::combat::WEAR_BODY => Some("BODY"),
        sim_core::combat::WEAR_FEET => Some("FEET"),
        _ => None,
    }
}

/// The INFORMATION column for `stack`, whose catalog row is `row`.
/// `loaded` is the rounds in the gun when the card is the gun in your hand.
pub fn stats(row: ItemRow, stack: ItemStack, loaded: Option<u16>) -> Vec<Stat> {
    let mut out = Vec::new();
    if let Some(g) = row.gun {
        let rpm = (60 * TICK_HZ) as f32 / g.rate_ticks.max(1) as f32;
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
        if g.pellets > 1 {
            out.push(Stat {
                label: "PELLETS",
                value: g.pellets.to_string(),
                frac: frac(g.pellets as f32, MAX_PELLETS as f32),
            });
        }
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
    Split,
    Drop,
}

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
        if ctx.loaded_in_hand {
            out.push((Act::Unload, "Unload ammo"));
        }
        if stack.count > 1 && ctx.room {
            out.push((Act::Split, "Split stack"));
        }
        out.push((Act::Drop, "Drop"));
    }
    out
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
        }
    }

    #[test]
    fn a_gun_card_reads_its_handling_off_the_row() {
        let row = ItemRow {
            cond_max: 10_000,
            stack_max: 1,
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
        assert!(get("PELLETS").is_none(), "one pellet is not a stat");
        assert!(get("STACK").is_none(), "a gun does not stack");
        assert!(s
            .iter()
            .all(|x| x.frac.is_none_or(|f| (0.0..=1.0).contains(&f))));
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
