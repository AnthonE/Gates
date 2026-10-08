//! The camp fire, as the client reads it — the pure half.
//!
//! The sim sections a fire's slots the way Rust does (`sim_core::oven::
//! layout`: one fuel slot, one on the grill, two for what comes off it) and
//! refuses a stack in the wrong one (`REFUSE_M_OVEN`). What is decided here,
//! headless, is what the panel draws and where a gesture sends a stack, off
//! the roles the item catalog carries (`ItemRow::oven`, wire v89) and the
//! sim's own [`slot_takes`]; `render::panels::inv` only draws it.

use core::ops::Range;

use protocol::event::ItemCatalog;
use sim_core::deploy::{box_key, DeployContent, DeployRec};
use sim_core::gather::ItemStack;
use sim_core::inventory::{is_own, CONT_BOX, CONT_SELF, CONT_WEAR, REFUSE_M_OVEN};
use sim_core::limits::INV_SLOTS;
use sim_core::oven::{
    layout, slot_takes, unpack_roles, OvenLayout, ROLE_FUEL, ROLE_INPUT, ROLE_OUTPUT,
};

use crate::ui::slots::{move_args, quick_move, refusal_text, Grab, Quick};

/// The open container's archetype and sections, if it is a sectioned
/// converter — the camp fire. Resolved off the deploy sync the way
/// `slots::box_item_at` names a box, under the same def watermark.
pub fn fire_open(
    kind: u8,
    handle: u32,
    deploys: &[DeployRec],
    defs: &DeployContent,
    defs_have: u16,
) -> Option<(u8, OvenLayout)> {
    if kind != CONT_BOX || handle == 0 {
        return None;
    }
    deploys.iter().find_map(|d| {
        if box_key(d.cx, d.cz, d.level, d.loc) != handle || u16::from(d.row) >= defs_have {
            return None;
        }
        let arch = defs.defs[d.row as usize].arch;
        layout(arch).map(|l| (arch, l))
    })
}

/// The open container's archetype, if it converts at all — the fire, the
/// furnace, the recycler (`OvenState::arch_converts`), sectioned or not.
pub fn converter_open(
    kind: u8,
    handle: u32,
    deploys: &[DeployRec],
    defs: &DeployContent,
    defs_have: u16,
) -> Option<u8> {
    if kind != CONT_BOX || handle == 0 {
        return None;
    }
    deploys.iter().find_map(|d| {
        if box_key(d.cx, d.cz, d.level, d.loc) != handle || u16::from(d.row) >= defs_have {
            return None;
        }
        let arch = defs.defs[d.row as usize].arch;
        sim_core::oven::OvenState::arch_converts(arch).then_some(arch)
    })
}

/// `REFUSE_M_OVEN` in words, for the converter that said it.
pub fn oven_refusal(arch: u8) -> &'static str {
    match arch {
        sim_core::deploy::ARCH_FURNACE => "a furnace takes wood and ore",
        sim_core::deploy::ARCH_RECYCLER => "the recycler only takes what it can break down",
        _ => refusal_text(REFUSE_M_OVEN as u8),
    }
}

/// One band of the fire panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Fuel,
    Input,
    Output,
}

impl Section {
    /// Top to bottom, the way the fire works: what burns, what cooks, what
    /// comes off.
    pub const ALL: [Section; 3] = [Section::Fuel, Section::Input, Section::Output];

    pub fn label(self) -> &'static str {
        match self {
            Section::Fuel => "FUEL",
            Section::Input => "INPUT",
            Section::Output => "OUTPUT",
        }
    }

    pub fn slots(self, l: OvenLayout) -> Range<usize> {
        match self {
            Section::Fuel => l.fuel_slots(),
            Section::Input => l.input_slots(),
            Section::Output => l.output_slots(),
        }
    }

    /// Does a stack in this band burn or cook while the fire is lit? The
    /// output only sits: what came off the grill stays as it came off.
    pub fn works(self) -> bool {
        self != Section::Output
    }
}

/// The section a slot belongs to, or `None` past the layout.
pub fn section_of(l: OvenLayout, slot: usize) -> Option<Section> {
    Section::ALL
        .into_iter()
        .find(|s| s.slots(l).contains(&slot))
}

/// What `item` is to a converter of `arch`, or `None` while its catalog row
/// has not dripped in — an undelivered row reads as zeroes, which would say
/// "a fire has no use for this" about wood.
pub fn roles_of(catalog: &ItemCatalog, arch: u8, item: u16) -> Option<u8> {
    if catalog.name(item as usize).is_empty() {
        return None;
    }
    Some(unpack_roles(catalog.row(item as usize).oven, arch))
}

/// Will `slot` of the fire take `item` — the sim's own [`slot_takes`],
/// asked of the catalog's roles. `None` when only the sim can say.
pub fn takes(catalog: &ItemCatalog, arch: u8, slot: usize, item: u16) -> Option<bool> {
    roles_of(catalog, arch, item).map(|r| slot_takes(arch, slot, r))
}

/// The band a right-click sends an item to: the fire's fuel to FUEL; what
/// the fire makes to OUTPUT, so a cooked meal goes back beside the others
/// rather than onto the grill to burn; what it cooks to INPUT.
pub fn home(roles: u8) -> Option<Section> {
    if roles & ROLE_FUEL != 0 {
        Some(Section::Fuel)
    } else if roles & ROLE_OUTPUT != 0 {
        Some(Section::Output)
    } else if roles & ROLE_INPUT != 0 {
        Some(Section::Input)
    } else {
        None
    }
}

/// The line a drop into the wrong band says: where the item does go, or
/// the sim's own sentence for something a fire has no use for.
pub fn wrong_band(roles: u8) -> &'static str {
    match home(roles) {
        Some(Section::Fuel) => "that goes in FUEL",
        Some(Section::Input) => "that goes in INPUT",
        Some(Section::Output) => "that goes in OUTPUT",
        None => refusal_text(REFUSE_M_OVEN as u8),
    }
}

/// A drag the fire will refuse, said before the round trip: the stack
/// landing in a fire slot that does not take it — the destination, or the
/// fire slot a whole-stack swap would push the destination's stack into.
/// `None` when the move is fine or only the sim can say.
#[allow(clippy::too_many_arguments)]
pub fn drop_refusal(
    catalog: &ItemCatalog,
    arch: u8,
    from_kind: u8,
    from_slot: usize,
    to_kind: u8,
    to_slot: usize,
    src: ItemStack,
    dst: ItemStack,
    units: u16,
) -> Option<&'static str> {
    let refuses = |slot: usize, item: u16| {
        roles_of(catalog, arch, item)
            .filter(|&r| !slot_takes(arch, slot, r))
            .map(wrong_band)
    };
    if to_kind == CONT_BOX && src.count > 0 {
        if let Some(why) = refuses(to_slot, src.item) {
            return Some(why);
        }
    }
    let swap =
        dst.count > 0 && (dst.item != src.item || dst.skin != src.skin) && units == src.count;
    if from_kind == CONT_BOX && swap {
        return refuses(from_slot, dst.item);
    }
    None
}

/// Where a right-click sends a stack with a camp fire open.
///
/// Out of the fire it is the ordinary quick-move, into the pack. Into it,
/// the stack goes to its own band ([`home`]): onto a stack of the same item
/// with room, else into an empty slot there. Something the fire has no use
/// for is refused in the sim's words.
#[allow(clippy::too_many_arguments)]
pub fn fire_quick_move(
    handle: u32,
    arch: u8,
    l: OvenLayout,
    from_kind: u8,
    from_slot: usize,
    catalog: &ItemCatalog,
    inv: &[ItemStack; INV_SLOTS],
    cont: &[ItemStack; INV_SLOTS],
    worn: &[ItemStack],
) -> Quick {
    if !is_own(from_kind) {
        return quick_move(
            CONT_BOX, handle, from_kind, from_slot, catalog, inv, cont, worn,
        );
    }
    let src = match from_kind {
        CONT_SELF => inv.get(from_slot),
        CONT_WEAR => worn.get(from_slot),
        _ => None,
    }
    .copied()
    .unwrap_or_default();
    if src.count == 0 {
        return Quick::Refused("there is nothing in that slot");
    }
    let Some(roles) = roles_of(catalog, arch, src.item) else {
        return Quick::Refused("that item is not known yet - try again in a moment");
    };
    let Some(band) = home(roles) else {
        return Quick::Refused(refusal_text(REFUSE_M_OVEN as u8));
    };
    let cap = catalog.stack_max(src.item as usize);
    let mut empty = None;
    let mut pick = None;
    for s in band.slots(l) {
        let there = cont[s];
        if there.count == 0 {
            empty = empty.or(Some(s));
        } else if there.item == src.item && there.skin == src.skin {
            let room = cap.saturating_sub(there.count);
            if room > 0 {
                pick = Some((s, Grab::Fit(room)));
                break;
            }
        }
    }
    let Some((to_slot, grab)) = pick.or(empty.map(|s| (s, Grab::All))) else {
        return Quick::Refused(match band {
            Section::Fuel => "the fire's fuel slot is full",
            Section::Input => "the grill is full",
            Section::Output => "the fire's output is full",
        });
    };
    match move_args(
        handle, from_kind, from_slot, CONT_BOX, to_slot, grab, inv, cont, worn,
    ) {
        Some(args) => Quick::Send(args),
        None => Quick::Refused("that move cannot be addressed from here"),
    }
}

/// Does this slot of the fire burn or cook right now — the flame the panel
/// draws on it. Only while lit, only on a stack, and never in the output.
pub fn burning(l: OvenLayout, slot: usize, stack: ItemStack, lit: bool) -> bool {
    lit && stack.count > 0 && section_of(l, slot).is_some_and(Section::works)
}

/// The fire's slots past its layout that still hold something — a save
/// from before the fire had sections. The panel draws them so they can be
/// taken out; nothing can be put back.
pub fn leftovers(l: OvenLayout, cont: &[ItemStack]) -> Range<usize> {
    let end = cont
        .iter()
        .take(sim_core::limits::BOX_SLOTS)
        .rposition(|s| s.count > 0)
        .map_or(0, |i| i + 1);
    l.slots()..end.max(l.slots())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim_core::deploy::ARCH_FIRE;
    use sim_core::oven::FIRE_LAYOUT;

    const WOOD: u16 = 1;
    const RAW: u16 = 2;
    const COOKED: u16 = 3;
    const CHAR: u16 = 4;
    const ROCK: u16 = 5;

    /// The shipped shape of the roles: wood is fuel at both burners, raw
    /// meat cooks at a fire, cooked meat is what a fire makes and what it
    /// burns further, charcoal is made, a rock is nothing.
    fn catalog() -> ItemCatalog {
        let mut c = ItemCatalog::EMPTY;
        c.count = 6;
        let rows: [(&[u8], u16, u16); 5] = [
            (b"Wood", WOOD, 0b001_001),
            (b"Raw Meat", RAW, 0b010),
            (b"Cooked Meat", COOKED, 0b110),
            (b"Charcoal", CHAR, 0b100_100),
            (b"Rock", ROCK, 0),
        ];
        for (name, i, oven) in rows {
            let row = protocol::ItemRow {
                stack_max: 100,
                oven,
                ..protocol::ItemRow::EMPTY
            };
            c.set(i as usize, name, row).unwrap();
        }
        c
    }

    fn stack(item: u16, count: u16) -> ItemStack {
        ItemStack {
            item,
            count,
            ..Default::default()
        }
    }

    fn quick(inv: &[ItemStack; INV_SLOTS], cont: &[ItemStack; INV_SLOTS], from: usize) -> Quick {
        fire_quick_move(
            0x10,
            ARCH_FIRE,
            FIRE_LAYOUT,
            CONT_SELF,
            from,
            &catalog(),
            inv,
            cont,
            &[],
        )
    }

    fn sent_to(q: Quick) -> (u8, u16) {
        match q {
            Quick::Send(a) => (a.to_slot, a.count),
            other => panic!("not sent: {other:?}"),
        }
    }

    /// A right-click puts each thing where it belongs: wood in the fire,
    /// raw meat on the grill, a cooked meal and the charcoal in the output.
    #[test]
    fn a_right_click_sends_each_stack_to_its_band() {
        let mut inv = [ItemStack::default(); INV_SLOTS];
        let cont = [ItemStack::default(); INV_SLOTS];
        inv[6] = stack(WOOD, 50);
        inv[7] = stack(RAW, 4);
        inv[8] = stack(COOKED, 2);
        inv[9] = stack(CHAR, 9);
        inv[10] = stack(ROCK, 3);
        assert_eq!(sent_to(quick(&inv, &cont, 6)), (0, 50), "wood → FUEL");
        assert_eq!(sent_to(quick(&inv, &cont, 7)), (1, 4), "raw → INPUT");
        assert_eq!(sent_to(quick(&inv, &cont, 8)), (2, 2), "cooked → OUTPUT");
        assert_eq!(sent_to(quick(&inv, &cont, 9)), (2, 9), "charcoal → OUTPUT");
        assert_eq!(
            quick(&inv, &cont, 10),
            Quick::Refused(refusal_text(REFUSE_M_OVEN as u8)),
            "a rock is refused in the sim's words"
        );
    }

    /// Onto the stack already there, as much as fits; a full band says so.
    #[test]
    fn a_right_click_tops_up_and_says_when_a_band_is_full() {
        let mut inv = [ItemStack::default(); INV_SLOTS];
        let mut cont = [ItemStack::default(); INV_SLOTS];
        inv[6] = stack(WOOD, 50);
        cont[0] = stack(WOOD, 90);
        assert_eq!(
            sent_to(quick(&inv, &cont, 6)),
            (0, 10),
            "the room, not the stack"
        );
        cont[0] = stack(WOOD, 100);
        assert_eq!(
            quick(&inv, &cont, 6),
            Quick::Refused("the fire's fuel slot is full")
        );
        inv[7] = stack(CHAR, 1);
        cont[2] = stack(COOKED, 1);
        assert_eq!(
            sent_to(quick(&inv, &cont, 7)),
            (3, 1),
            "the second output slot"
        );
    }

    /// The drag says what a release would be refused for, both ways.
    #[test]
    fn a_drop_in_the_wrong_band_names_the_right_one() {
        let c = catalog();
        let at = |to_slot, item| {
            drop_refusal(
                &c,
                ARCH_FIRE,
                CONT_SELF,
                6,
                CONT_BOX,
                to_slot,
                stack(item, 5),
                ItemStack::default(),
                5,
            )
        };
        assert_eq!(at(1, WOOD), Some("that goes in FUEL"));
        assert_eq!(at(0, RAW), Some("that goes in INPUT"));
        assert_eq!(at(2, RAW), Some("that goes in INPUT"));
        assert_eq!(at(1, CHAR), Some("that goes in OUTPUT"));
        assert_eq!(at(4, WOOD), Some("that goes in FUEL"), "past the layout");
        assert_eq!(at(0, WOOD), None);
        assert_eq!(at(1, COOKED), None, "it may go on the grill, to burn");
        // Taking cooked meat out onto a rock: the swap would put the rock in
        // the output.
        assert_eq!(
            drop_refusal(
                &c,
                ARCH_FIRE,
                CONT_BOX,
                2,
                CONT_SELF,
                6,
                stack(COOKED, 2),
                stack(ROCK, 1),
                2,
            ),
            Some(refusal_text(REFUSE_M_OVEN as u8))
        );
        // An undripped row: only the sim can say.
        assert_eq!(at(1, 40), None);
    }

    #[test]
    fn the_flame_is_on_the_fuel_and_the_grill_only_while_lit() {
        let l = FIRE_LAYOUT;
        let s = stack(WOOD, 1);
        assert!(burning(l, 0, s, true));
        assert!(burning(l, 1, s, true));
        assert!(!burning(l, 2, s, true), "the output only sits");
        assert!(!burning(l, 0, s, false), "out is out");
        assert!(!burning(l, 1, ItemStack::default(), true));
    }

    #[test]
    fn leftovers_are_whatever_sits_past_the_layout() {
        let l = FIRE_LAYOUT;
        let mut cont = [ItemStack::default(); INV_SLOTS];
        assert!(leftovers(l, &cont).is_empty());
        cont[1] = stack(RAW, 1);
        assert!(leftovers(l, &cont).is_empty());
        cont[7] = stack(WOOD, 3);
        assert_eq!(leftovers(l, &cont), 4..8);
    }
}
