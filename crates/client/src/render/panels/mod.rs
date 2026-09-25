//! The in-game menus, drawn.
//!
//! Distinct from `render::ui`, which is the chrome the full-screen MENU
//! screens share: `ui` is what a player sees *instead of* the world, this is
//! what they see *on top of* it. The palette below borrows `ui`'s type and
//! rules so the two read as one product.
//!
//! Three screens, one rule: **everything with arithmetic in it lives in
//! `crate::ui`** (pure, headless, gated by `crates/client/tests/ui.rs`) and
//! everything here is nodes, colours and pointer plumbing. That split is
//! `RENDER.md` §1's "Bevy draws, it does not decide" applied to the one
//! surface where it is easiest to break — a menu has a drag, a price and an
//! angle in it, and all three are testable only if they are not inside a
//! system.
//!
//! | screen | key | reference frame |
//! |---|---|---|
//! | inventory page | `Tab` (or `I`) toggles, `Q` switches, `Esc` closes | the reference `inventory.jpeg` |
//! | crafting page | `Q` toggles, `Tab` switches, `Esc` closes | the reference `crafting.png` |
//! | container | the inventory page, opened by the sim | `storageandtoolchest.jpeg` |
//! | build wheel | hold right, building plan in hand | the radial in the operator's second frame |
//! | hammer wheel | hold right, hammer in hand | the reference's second radial ("right click when equipped for more options") |
//!
//! ## Two things this deliberately does not do
//!
//! **It does not open under `--capture`.** These systems are registered only
//! on a non-capture run. A probe harness that could open a panel is a visual
//! gate whose frames depend on which key was last pressed, and the capture
//! path drives itself specifically so that nothing is ever halfway through
//! anything. The cost is that no gate photographs these panels — which is
//! the same hole `NOW.md` §0v item 3 names from the other side, since
//! `ci/gates.sh` does not build `--features render` at all.
//!
//! **It does not predict.** A drag draws a ghost under the cursor and sends
//! a move; the grids redraw from `ClientCore`'s authoritative view and never
//! from what the drag hoped would happen. The reference's own worst bug on
//! this verb is a container state that diverged from the server's, and the
//! cheapest way not to diverge is to have no second copy at all.

use bevy::prelude::*;

// Both search boxes share one cap — see `crate::ui::MAX_QUERY_CHARS`.
use crate::ui::MAX_QUERY_CHARS;

use crate::ui::craft::{Cat, Facts};
use crate::ui::slots::Drag;
use sim_core::gather::ItemStack;

pub mod craft;
pub mod inv;
pub mod ring;
pub mod tech;
pub mod wheel;

/// Which menu is up. One at a time: the wheel is a hold and the inventory
/// is a toggle, so they cannot both be open without a rule about which one
/// the pointer belongs to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Panel {
    #[default]
    None,
    /// The `Tab` page: your pack and belt, your body, and on the right
    /// either quick craft or whatever you are looting (Rust's inventory).
    Inventory,
    /// The `Q` page: the crafting menu on its own — the category rail,
    /// the recipe grid, the detail pane and the queue (Rust's crafting).
    /// It was the top half of the inventory screen until 2026-09-25.
    Craft,
    /// The build wheel, up while right is held with the building plan.
    /// Latches its choice; releasing keeps it.
    Wheel,
    /// The hammer's wheel, up while right is held with the hammer.
    /// Latches nothing: releasing **fires** the hovered verb (`keys`),
    /// which is the two wheels' one deliberate difference.
    Hammer,
    /// The tech tree, opened by `E` at a workbench (tech tree v0). A
    /// toggle like the inventory, closed by Escape; Tab swaps to the
    /// inventory rather than stacking on it.
    Tech,
}

impl Panel {
    /// Does this panel own the pointer? A panel that does releases the
    /// cursor and takes look and movement away from `input::gather` — a
    /// player dragging an item is not also turning around.
    pub fn grabs_pointer(self) -> bool {
        !matches!(self, Panel::None)
    }

    /// This panel as `ui::nav` sees it.
    pub fn page(self) -> crate::ui::nav::Page {
        use crate::ui::nav::Page;
        match self {
            Panel::None => Page::Closed,
            Panel::Inventory => Page::Inventory,
            Panel::Craft => Page::Crafting,
            Panel::Wheel | Panel::Hammer | Panel::Tech => Page::Other,
        }
    }

    /// The panel a `ui::nav` answer lands on; `None` for `Other`, which no
    /// key or tab ever answers.
    pub fn of_page(page: crate::ui::nav::Page) -> Option<Panel> {
        use crate::ui::nav::Page;
        match page {
            Page::Closed => Some(Panel::None),
            Page::Inventory => Some(Panel::Inventory),
            Page::Crafting => Some(Panel::Craft),
            Page::Other => None,
        }
    }
}

/// Everything the menus hold that is not on the wire.
///
/// Note what is **not** here: no inventory, no container, no recipe list, no
/// queue. Those live in `ClientCore` and are read fresh every rebuild. What
/// is here is the player's own view state — which filter, which row, how
/// many — plus the change-detection snapshots that decide when to rebuild.
#[derive(Resource)]
pub struct Ui {
    pub panel: Panel,
    /// The drag in flight, if the pointer is down on a slot.
    pub drag: Option<Drag>,
    /// Left rail selection.
    pub cat: Cat,
    /// The search box's contents.
    pub query: String,
    /// Whether the search box has the keyboard. **Only while it does is a
    /// letter a letter**: Rust's search field is clicked into (and goes
    /// amber), and every other letter on these pages is a key — `Q` shuts
    /// the crafting page, `P` re-skins, `I` opens the pack. The box used to
    /// take every printable key the whole time the screen was up, which is
    /// why no letter could close anything.
    pub search_focus: bool,
    /// Starred recipes. A local latch — the reference's FAVOURITE is one
    /// too, and nothing on our wire carries a favourite.
    pub favs: Vec<u16>,
    /// The recipe the detail pane is showing.
    pub selected: Option<u16>,
    /// The quantity stepper, always ≥ 1.
    pub count: u16,
    /// How far the recipe grid is scrolled, px. Kept here because the grid
    /// is respawned on every redraw, and a fresh node scrolls to the top: a
    /// player who scrolled down and clicked a recipe was thrown back up to
    /// the first row on every click. Zeroed when the list itself changes
    /// (a new bucket or a new search), where the old offset means nothing.
    pub browser_scroll: f32,
    /// The skin the next craft of `selected` is minted in (skins v0): a
    /// catalog id, 0 for the item's own look. Reset with every new pick,
    /// the stepper's reason.
    pub skin: u16,
    /// The one line under the title that says what just happened — a
    /// refusal, a full action lane, a craft that went in. Never empty for
    /// long, and never silently empty: a panel that cannot say why it did
    /// nothing is the dark-panel defect.
    pub status: String,
    /// Derived category facts, rebuilt when the content tables drip in.
    pub facts: Facts,
    /// The wheel's latched choice: an index into `ui::build::SHAPES`.
    /// Latched rather than momentary, so releasing the wheel over nothing
    /// keeps what was chosen last.
    ///
    /// **There is no `material` beside it since 2026-08-07.** The blueprint
    /// places one rung and the hammer climbs the ladder, which is the
    /// reference's split — `ui::build::PLACE_MATERIAL` has the argument.
    pub shape: usize,
    /// Which segment the open wheel's pointer is over this frame — the
    /// shape wheel's (`ui::build::SHAPES`) or the hammer's
    /// (`ui::hammer::VERBS`), whichever is up. For the hammer's it is also
    /// what the release fires, which is why `keys` reads it before
    /// `wheel::track` clears it.
    pub hover: Option<usize>,
    /// The tech tree's selected node — a recipe index, the sidebar's
    /// subject (tech tree v0).
    pub tech_sel: Option<u16>,
    /// The rung of the bench the tree was opened at: the highest tab, and
    /// the rung the panel's reach check holds it to (`keys` closes the tree
    /// when no bench that high stands within the station radius). The sim
    /// still re-derives the demanded rung per node.
    pub tech_tier: u8,
    /// The tab on show — one bench tier's tree, `1..=tech_tier`
    /// (`ui::techtree::tabs`). Opens on the bench's own tier.
    pub tech_tab: u8,
    /// When this client saw the open research table start (research table
    /// v1) — the wait bar's clock, fed every frame by `inv::table_clock`.
    /// `ui::research::TableClock` says why a start has to be SEEN.
    pub table_clock: crate::ui::research::TableClock,
    /// Rebuild the panel's node tree on the next frame.
    pub dirty: bool,
    /// Change detection against the core. A menu that rebuilt every frame
    /// would allocate a node tree per frame for a screen that changes when
    /// a player clicks — so the rebuild is driven by these snapshots, and
    /// a still screen costs one comparison.
    pub(crate) seen: Seen,
}

/// The core state the open panel last drew, so a rebuild happens when it
/// changes and not otherwise.
#[derive(Default)]
pub(crate) struct Seen {
    /// `(item, count, cond)` per slot. **Condition is in the key**, and it
    /// was not until the durability pip landed: a panel that redraws on
    /// `(item, count)` alone cannot see a tool wear, so the bar it drew
    /// would keep claiming a condition the stack no longer has. Nothing
    /// wears while this screen is open in v0 — you cannot swing through it
    /// — so this is not a defect being fixed but the door being shut before
    /// repair or wear-on-hit walks through it.
    ///
    /// **And the skin** (skins v0), for condition's reason one field on: a
    /// re-skin changes nothing else about a slot, so a key without it would
    /// leave the old look drawn.
    pub inv: [ItemStack; sim_core::limits::INV_SLOTS],
    pub cont: [ItemStack; sim_core::limits::INV_SLOTS],
    /// The **body**, watched separately from `cont` since the two views
    /// split (`NOW.md` §0eq item 4). It has to be here rather than
    /// folded into `cont`: the wear panel is drawn on every inventory
    /// screen now, so a helmet arriving on a head while no ground
    /// container is open changes nothing else on this list, and the
    /// paperdoll would keep drawing the slot it had before.
    pub worn: [ItemStack; sim_core::limits::WEAR_SLOTS],
    /// The owned skin set and how much of the skin catalog has dripped: the
    /// craft panel's picker draws from both.
    pub skins_owned: sim_core::skin::SkinSet,
    /// `ClientCore::skins_gen` at the last redraw: any catalog drip,
    /// a reprice included.
    pub skins_have: u32,
    pub cont_kind: u8,
    pub cont_handle: u32,
    pub jobs: [(u8, u8); sim_core::limits::CRAFT_QUEUE],
    pub jobs_count: u8,
    pub recipes_have: u16,
    pub pieces_have: u16,
    pub deploys_have: u16,
    pub hammer_target: Option<crate::ui::structure::Target>,
    /// The blueprint mask (tech tree v0). Without it a node clicked to
    /// `Known` only redraws because the junk left `inv` — and a FREE
    /// node, which content permits, would not redraw at all.
    pub known: u64,
    /// The research drip's watermark, `recipes_have`'s reason exactly.
    pub research_have: u16,
    /// Whether the open research table is running (research table v1).
    /// Its slots do not change when a research STARTS — only the lit bit
    /// does — so without this the line under them would keep saying PRESS
    /// BEGIN over a table that had begun.
    pub table_lit: bool,
}

impl Default for Ui {
    fn default() -> Self {
        Self {
            panel: Panel::None,
            drag: None,
            cat: Cat::All,
            query: String::new(),
            search_focus: false,
            favs: Vec::new(),
            selected: None,
            count: 1,
            browser_scroll: 0.0,
            skin: 0,
            status: String::new(),
            facts: Facts::default(),
            shape: 0,
            hover: None,
            tech_sel: None,
            tech_tier: 1,
            tech_tab: 1,
            table_clock: crate::ui::research::TableClock::default(),
            dirty: false,
            seen: Seen::default(),
        }
    }
}

impl Ui {
    /// Say something on the status line and redraw it.
    pub fn say(&mut self, what: impl Into<String>) {
        self.status = what.into();
        self.dirty = true;
    }
}

/// A button that lights while the pointer is on it.
///
/// **The panels had no hover state at all**, so nothing on them answered the
/// pointer until a click round-tripped through a redraw, which is a large
/// part of why the craft screen felt dead. `rest` is the fill the build
/// drew, `hot` the one under the pointer; [`hover`] swaps them on the frame
/// the pointer arrives or leaves, with no redraw. Slot cells are not these:
/// `inv::drag_pointer` paints their hover with the drag's other states.
#[derive(Component, Clone, Copy)]
pub struct Hover {
    pub rest: Color,
    pub hot: Color,
}

impl Hover {
    /// The usual pair: whatever it rests on, lit to [`CELL_HOVER`].
    pub fn on(rest: Color) -> Self {
        Self {
            rest,
            hot: CELL_HOVER,
        }
    }
}

/// Light the button under the pointer.
pub fn hover(mut q: Query<(&Interaction, &Hover, &mut BackgroundColor), Changed<Interaction>>) {
    for (interaction, h, mut bg) in q.iter_mut() {
        let want = match interaction {
            Interaction::None => h.rest,
            _ => h.hot,
        };
        if bg.0 != want {
            bg.0 = want;
        }
    }
}

/// What a cell is called, for [`tooltip`]: set on a filled inventory slot,
/// a recipe and a queue job when they are built.
///
/// **A cell is a picture, and a picture you do not recognise is a
/// question** a 44 px cell has no room to answer. Rust answers it with the
/// item's name on its info panel; this answers it under the pointer.
#[derive(Component, Clone)]
pub struct Tip(pub String);

/// The tooltip's box and its line — one of each, spawned with the
/// inventory screen and moved to the pointer by [`tooltip`].
#[derive(Component)]
pub struct TipBox;
#[derive(Component)]
pub struct TipText;

/// Put the hovered cell's name beside the pointer, or hide it. Not during a
/// drag: the thing in your hand is already captioned (`inv::spawn_ghost`).
#[allow(clippy::type_complexity)]
pub fn tooltip(
    ui: Res<Ui>,
    window: Query<&Window, With<bevy::window::PrimaryWindow>>,
    tips: Query<(&Interaction, &Tip)>,
    mut boxes: Query<(&mut Node, &ComputedNode), With<TipBox>>,
    mut texts: Query<&mut Text, With<TipText>>,
) {
    let Ok((mut node, computed)) = boxes.single_mut() else {
        return;
    };
    let tip = if ui.drag.is_some() {
        None
    } else {
        tips.iter()
            .find(|(i, _)| matches!(i, Interaction::Hovered))
            .map(|(_, t)| t)
    };
    let win = window.single().ok();
    let at = win.and_then(|w| w.cursor_position().map(|p| (p, w.width(), w.height())));
    let (Some(tip), Some((p, w, h))) = (tip, at) else {
        if node.display != Display::None {
            node.display = Display::None;
        }
        return;
    };
    if let Ok(mut text) = texts.single_mut() {
        if text.0 != tip.0 {
            text.0.clone_from(&tip.0);
        }
    }
    // Below and right of the pointer, flipped to the other side of it where
    // that would run off the window — measured off last frame's layout.
    let size = computed.size() * computed.inverse_scale_factor();
    let x = if p.x + 14.0 + size.x > w - 4.0 {
        p.x - 10.0 - size.x
    } else {
        p.x + 14.0
    };
    let y = if p.y + 18.0 + size.y > h - 4.0 {
        p.y - 8.0 - size.y
    } else {
        p.y + 18.0
    };
    if node.left != Val::Px(x) {
        node.left = Val::Px(x);
    }
    if node.top != Val::Px(y) {
        node.top = Val::Px(y);
    }
    if node.display != Display::Flex {
        node.display = Display::Flex;
    }
}

/// The tooltip's nodes, hidden until [`tooltip`] has something to say.
pub fn spawn_tip(root: &mut ChildSpawnerCommands) {
    root.spawn((
        TipBox,
        Node {
            position_type: PositionType::Absolute,
            padding: UiRect::axes(Val::Px(7.0), Val::Px(3.0)),
            border: UiRect::all(Val::Px(1.0)),
            display: Display::None,
            ..default()
        },
        BackgroundColor(Color::srgba(0.09, 0.085, 0.075, 0.96)),
        BorderColor::all(LINE),
        GlobalZIndex(45),
        Pickable::IGNORE,
    ))
    .with_children(|b| {
        b.spawn((
            TipText,
            Text::new(""),
            font_bold(12.0),
            TextColor(TEXT),
            Pickable::IGNORE,
        ));
    });
}

/// A button in the strip across the top of both pages: the page it opens.
#[derive(Component)]
pub struct TabGo(pub Panel);

/// The strip across the top of the inventory and crafting pages — Rust's:
/// a button to the other page, never one for the page you are on
/// (`ui::nav::strip`), and its key in the tooltip rather than on it.
pub fn page_tabs(root: &mut ChildSpawnerCommands, ui: &Ui) {
    use crate::ui::nav;
    root.spawn(Node {
        flex_direction: FlexDirection::Row,
        column_gap: Val::Px(6.0),
        ..default()
    })
    .with_children(|row| {
        for page in nav::strip(ui.panel.page()) {
            let Some(panel) = Panel::of_page(*page) else {
                continue;
            };
            row.spawn((
                Button,
                TabGo(panel),
                Tip(format!("or press {}", nav::key_hint(*page))),
                Node {
                    min_width: Val::Px(200.0),
                    padding: UiRect::axes(Val::Px(18.0), Val::Px(6.0)),
                    justify_content: JustifyContent::Center,
                    border: UiRect::all(Val::Px(1.0)),
                    ..default()
                },
                BackgroundColor(CELL_BG),
                Hover::on(CELL_BG),
                BorderColor::all(LINE),
            ))
            .with_children(|t| {
                t.spawn((
                    Text::new(nav::label(*page)),
                    font_bold(17.0),
                    TextColor(TEXT),
                    Pickable::IGNORE,
                ));
            });
        }
    });
}

/// Where the strip across the top of both pages sits, px from the top of the
/// window. **Pinned**, so the button to the other page is where the last
/// page's was: the crafting page is taller than the inventory page, and
/// centring each page whole put the button 60 px apart on the two, so it
/// jumped out from under the pointer that had just clicked it. Rust's strip
/// does not move either.
pub const STRIP_TOP_PX: f32 = 64.0;
/// The foot of the window the HUD's hotbar owns, px. A page's body is
/// centred in the room between the strip and it.
pub const HOTBAR_CLEAR_PX: f32 = 64.0;

/// The frame both pages share: the scrim, the strip pinned at the top, and
/// under it the status line over the page's `body`, centred in the room
/// that is left.
pub fn page_root(commands: &mut Commands, ui: &Ui, body: impl FnOnce(&mut ChildSpawnerCommands)) {
    commands
        .spawn((
            PanelRoot,
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                padding: UiRect {
                    top: Val::Px(STRIP_TOP_PX),
                    bottom: Val::Px(HOTBAR_CLEAR_PX),
                    ..default()
                },
                row_gap: Val::Px(8.0),
                ..default()
            },
            BackgroundColor(SCRIM),
        ))
        .with_children(|root| {
            page_tabs(root, ui);
            root.spawn(Node {
                flex_grow: 1.0,
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                row_gap: Val::Px(8.0),
                ..default()
            })
            .with_children(|zone| {
                status_line(zone, ui);
                body(zone);
            });
            spawn_tip(root);
        });
}

/// The one line under the tabs that says what just happened (`Ui::status`).
/// Always drawn, even empty: a line that appears and disappears makes the
/// page jump when it does.
pub fn status_line(root: &mut ChildSpawnerCommands, ui: &Ui) {
    root.spawn((
        Text::new(if ui.status.is_empty() {
            " ".into()
        } else {
            ui.status.clone()
        }),
        font(13.0),
        TextColor(BADGE),
    ));
}

/// The root of whichever panel is open. Despawned wholesale on a rebuild —
/// the same shape `menu::rebuild_on_new_rows` uses, and for the same reason:
/// a screen rebuilt from one function cannot drift from itself.
#[derive(Component)]
pub struct PanelRoot;

/// The stack under the cursor while a drag is in flight. Its own root, not
/// part of `PanelRoot`, so a rebuild underneath it does not make the thing
/// in your hand flicker.
#[derive(Component)]
pub struct GhostRoot;

// ---- the palette ---------------------------------------------------------
// **Type and rules come from `render::ui`, the menu chrome's palette**, so a
// player who opens the Esc menu and then their inventory is reading the same
// product. Only what a panel needs and a full-screen menu does not is
// declared here — a panel is drawn OVER a lit world, so it owns the
// translucency and the cell states, and nothing else. Proposed defaults,
// `DECISIONS.md` §open "menu skin v0".

// The FACE comes from there too, and for a stronger reason than the colours:
// a panel drawn in a different typeface from the Esc menu behind it does not
// read as the same product at all. `ui::font_bold` is the common case — the
// reference game's own UI default is `RobotoCondensed-Bold.ttf` — and
// `ui::font` is for prose.
pub use super::ui::{font, font_bold, ACCENT, DIM as TEXT_DIM, RULE as LINE, TEXT};

/// Panel body — `#2b2723` off `crafting.png`'s recipe grid. Nearly opaque,
/// so text over a lit world stays readable; the reference's is translucent
/// over a *blurred* world, which we do not have and which is what lets it
/// sit lower.
pub const PANEL_BG: Color = Color::srgba(0.169, 0.153, 0.137, 0.97);
/// The screen-wide scrim behind a panel.
pub const SCRIM: Color = Color::srgba(0.055, 0.047, 0.039, 0.72);
/// An empty cell. The reference's grid cells are the panel with a hairline,
/// not a lighter block — the ITEM is what carries the value there, which is
/// why our text-only cells need more separation than its do.
pub const CELL_BG: Color = Color::srgba(0.220, 0.204, 0.184, 0.92);
/// A cell holding something — `#47433c`, the detail pane's value.
pub const CELL_FULL: Color = Color::srgba(0.278, 0.263, 0.235, 0.96);
/// The cell the pointer is over, and the drag's source.
pub const CELL_HOVER: Color = Color::srgba(0.369, 0.353, 0.329, 0.98);
/// The belt slot in your hand — Rust's selection blue (≈#1F5D8D), filled,
/// on the HUD's hotbar and on the inventory's belt row alike.
pub const CELL_SEL: Color = Color::srgba(0.122, 0.365, 0.553, 0.88);
/// A filled cell holding a **blueprint** (research table v1): the paper's
/// blue, under the picture of the thing it teaches — the reference draws a
/// blueprint as its item on blueprint paper, and a sheet on the stone-grey
/// every other stack wears would read as the item itself.
pub const PAPER_BG: Color = Color::srgba(0.141, 0.259, 0.412, 0.96);
/// The one hot line: a selected cell, the head of the queue, an armed button.
pub const LINE_HOT: Color = Color::srgba(0.98, 0.86, 0.55, 0.95);
/// A price the player cannot pay, and the reference's own colour for it.
pub const TEXT_SHORT: Color = Color::srgb(0.86, 0.36, 0.30);
/// The station badge — `#9abc5c`, the green of `WORKBENCH LEVEL 1 REQUIRED`.
/// It was a mustard yellow, which is not a colour on the reference panel.
pub const BADGE: Color = Color::srgb(0.604, 0.737, 0.361);

// ---- the three vitals, measured off `crafting.png`'s bottom-right stack --
//
// Health, water, food. The reference draws them as **filled bars with an
// icon**, not as text, and these are the fills.
/// Health — `#8cb640`.
pub const VITAL_HP: Color = Color::srgb(0.549, 0.714, 0.251);
/// Water — `#4e97d0`.
pub const VITAL_WATER: Color = Color::srgb(0.306, 0.592, 0.816);
/// Food — `#c36f36`.
pub const VITAL_FOOD: Color = Color::srgb(0.765, 0.435, 0.212);
/// The trough a vital bar sits in.
pub const VITAL_TROUGH: Color = Color::srgba(0.106, 0.098, 0.086, 0.72);

// ---- the durability pip (`NOW.md` §0dur item 1) ----------------------------
//
// A cell's second number, drawn as a bar rather than a digit: a fraction of a
// maximum, which is the same statement the vitals stack makes, so it is drawn
// in the same two colours rather than in a third pair nobody measured.
// `ui::slots::pip_fraction` owns *whether* there is a bar; these own what it
// looks like. Proposed defaults (`DECISIONS.md` §open, "durability pip v0").
//
// **One colour, no threshold tier.** A green-to-red ramp at some fraction is
// the obvious embellishment and it would be an invented knob: nothing in
// `reference/DURABILITY.md` sources a warning band, and the state that
// actually matters — a dead tool — is already distinct without one, because a
// pristine item draws no bar at all and a dead one draws the trough empty.
/// The durability pip's fill.
pub const PIP_FILL: Color = VITAL_HP;
/// The trough it sits in — the vitals' own, so an empty bar reads as empty
/// rather than as a cell with a dark edge.
pub const PIP_TROUGH: Color = VITAL_TROUGH;
/// The pip's height, px. Thin on purpose: at [`CELL_PX`] the bar shares the
/// bottom edge with the count badge, and anything taller starts competing
/// with the icon for the cell it is annotating.
pub const PIP_H_PX: f32 = 3.0;

/// A recipe cell on the crafting page, px. Proposed default, same
/// `DECISIONS.md` row.
///
/// **Sized against 720p, which is the constraint that decides it** — Bevy's
/// default window is 1280×720. It was 44 px while the crafting menu shared
/// one column with your thirty slots; on its own page (Rust's split,
/// 2026-09-25) the browser and the detail pane get the height [`PANEL_H`]
/// states, and the cell grew with them.
pub const CELL_PX: f32 = 50.0;
pub const CELL_GAP_PX: f32 = 4.0;

/// A slot on the inventory page — your pack, your belt, your body and the
/// container you are looting. Bigger than a recipe cell because the page is
/// only slots: Rust's slots are ~80 px at 1080p, and 58 at 720p keeps a
/// picture 48 px across, where a thin tool still reads.
pub const SLOT_PX: f32 = 58.0;

/// Height of the crafting page's browser and detail pane, px — the two tall
/// things, and so the ones that pay for the rest of the 720p budget. The
/// strip is pinned [`STRIP_TOP_PX`] down and the HUD's hotbar owns the
/// bottom [`HOTBAR_CLEAR_PX`], which leaves ~590: the strip 34, the status
/// line 16, the queue 56, the hint 16 and five 8 px gaps leave this. At 470
/// the hint line sat on the hotbar (measured off a 720p frame, 2026-09-25).
pub const PANEL_H: f32 = 424.0;

/// Columns in the recipe browser. Eight rather than the inventory's six —
/// the recipe list is longer than an inventory and is read by name, not by
/// slot number.
pub const BROWSER_COLS: u16 = 8;

/// The recipe grid's own height inside [`PANEL_H`], leaving room for the
/// search box under it. **The grid scrolls**: content grows with
/// `content/recipes.toml` and a browser sized to today's recipes is a
/// browser that silently hides the next one.
pub const BROWSER_GRID_H: f32 = PANEL_H - 62.0;

/// Pixels of scroll per wheel line. Proposed default, same `DECISIONS.md`
/// row.
pub const SCROLL_PX_PER_LINE: f32 = 26.0;

/// Register the menus. Called from `GatesRenderPlugin` on a non-capture run
/// only — see the module note.
pub fn register(app: &mut App) {
    app.init_resource::<Ui>()
        .add_systems(
            Update,
            (
                keys,
                inv::drag_pointer,
                inv::skin_keys,
                inv::table_clicks,
                craft::clicks,
                craft::scroll,
                tech::clicks,
                tech::scroll,
                wheel::track,
                sync_refusals,
                inv::table_clock,
                rebuild,
                inv::ghost_follow,
                hover,
                tooltip,
            )
                .chain()
                // **Before `pause::open`, and that ordering is load-bearing.**
                // Escape means "close what is on top of the world" before it
                // means "open the Esc menu", and both systems read the same
                // `just_pressed`. `keys` clears the press when it consumed it, so
                // one Escape is one action — without the ordering, closing a
                // panel and opening the pause screen would happen on the same
                // key, in the same frame.
                .before(super::pause::open)
                .after(super::verbs::resolve)
                .run_if(in_state(super::Screen::InWorld)),
        )
        // The tree's status line hears the sim — a node learned, or why not.
        // After the drain, because it reads this frame's `Feed`; before the
        // rebuild, so the sentence is on the board drawn this frame.
        .add_systems(
            Update,
            (tech::sync_status, craft::sync_status)
                .after(super::feed::drain)
                .before(rebuild)
                .run_if(in_state(super::Screen::InWorld)),
        )
        // The queue strip's countdown and progress, every frame and in place
        // — they move every frame and a redraw is for things that do not.
        // After the clock, so the strip and the HUD bar agree to the second.
        .add_systems(
            Update,
            craft::queue_tick
                .after(super::hud::craft_clock)
                .after(rebuild)
                .run_if(in_state(super::Screen::InWorld)),
        )
        // A panel is only ever drawn over a running world, so leaving `InWorld`
        // takes its nodes with it. Nothing here is a `WorldEntity` — these are
        // menu nodes, not world ones — so `world_teardown` would not have.
        .add_systems(OnExit(super::Screen::InWorld), close)
        // Leaving the shard resets the whole view state, and that is not
        // tidiness: `selected` and `favs` are RECIPE INDICES, and the next shard
        // bakes its own content. Carrying them across would silently point a
        // favourite at a different recipe.
        .add_systems(OnEnter(super::Screen::Menu), forget);
}

/// Shut the panels and drop their nodes.
#[allow(clippy::type_complexity)]
pub fn close(
    mut commands: Commands,
    mut ui: ResMut<Ui>,
    roots: Query<Entity, Or<(With<PanelRoot>, With<GhostRoot>)>>,
) {
    ui.panel = Panel::None;
    ui.drag = None;
    ui.dirty = false;
    for e in roots.iter() {
        commands.entity(e).despawn();
    }
}

/// Forget everything that was true of the shard we just left.
pub fn forget(mut ui: ResMut<Ui>) {
    *ui = Ui::default();
}

/// Open, close, and the keys that belong to a panel rather than to the
/// world. Runs before `input::gather`'s own key reads by being earlier in
/// the chain; `gather` then skips look and movement while a panel is up.
///
/// `keyboard` is `ResMut` for one reason: **a key this consumed must not
/// reach the system after it.** Escape closes an open panel and is cleared;
/// Escape with nothing open is left alone and `pause::open` takes it.
#[allow(clippy::too_many_arguments)]
pub fn keys(
    mut ui: ResMut<Ui>,
    net: NonSend<super::Net>,
    mut toast: ResMut<super::hud::Toast>,
    mut keyboard: ResMut<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    // The hammer wheel's release fires at the nearest structure — the same
    // `Near` the keyboard verbs read, resolved before this system so a
    // removed or upgraded structure cannot leave a stale wheel target.
    near: Res<super::verbs::Near>,
    mut chars: MessageReader<bevy::input::keyboard::KeyboardInput>,
    tabs: Query<(&Interaction, &TabGo), Changed<Interaction>>,
) {
    // **The wheel is held RIGHT, and only by an item that owns one.**
    //
    // It was `B`, held, whatever was in your hand. Both halves were wrong
    // against the reference, whose wiki says it for each item in as many
    // words — *"right click when equipped for more options"* — and whose
    // building interface is entirely held-item modal. `crate::ui::hold` has
    // the argument and the reason binding away from the swing is free
    // (neither item has an attack: the hammer's damage total is 0).
    let core = &net.session.core;
    let hand = crate::ui::hold::held_in_hand(&core.catalog, &core.inv, net.sel);
    let holding_wheel = hand.opens_a_wheel() && mouse.pressed(MouseButton::Right);
    let was_inventory = ui.panel == Panel::Inventory;

    // **Two pages, Rust's two keys.** `Tab` is the inventory and `Q` is the
    // crafting menu; each key shuts its own page and switches from the
    // other (`ui::nav::press`), and a tab in the strip across the top of
    // both is a click that switches and never shuts (`ui::nav::click`).
    // `I` is `Tab`'s alias, as it always was here.
    //
    // **The letters can do this now because the search box stopped taking
    // them.** It used to capture every printable key while the screen was
    // up, so a letter that closed the screen would have closed it mid-word
    // and only `Tab` and `Esc` could close anything. The box takes the
    // keyboard only while it is clicked into (`Ui::search_focus`, Rust's
    // amber field), and while it does, `Q` and `I` are letters.
    let typing = ui.panel == Panel::Craft && ui.search_focus;
    let page = ui.panel.page();
    let mut want = tabs
        .iter()
        .find(|(i, _)| **i == Interaction::Pressed)
        .map(|(_, t)| crate::ui::nav::click(t.0.page()));
    if keyboard.just_pressed(KeyCode::Tab) || (!typing && keyboard.just_pressed(KeyCode::KeyI)) {
        want = Some(crate::ui::nav::press(page, crate::ui::nav::Page::Inventory));
    } else if !typing && keyboard.just_pressed(KeyCode::KeyQ) {
        want = Some(crate::ui::nav::press(page, crate::ui::nav::Page::Crafting));
    }
    if let Some(to) = want.and_then(Panel::of_page) {
        if to != ui.panel {
            ui.panel = to;
            ui.drag = None;
            ui.search_focus = false;
            ui.dirty = true;
            // The crafting page opens on a recipe, never on an empty pane.
            if to == Panel::Craft && ui.selected.is_none() {
                let mut shown = Vec::new();
                crate::ui::craft::rows(
                    &core.recipes,
                    &core.inv,
                    &core.catalog,
                    &ui.facts,
                    &ui.favs,
                    core.known(),
                    ui.cat,
                    &ui.query,
                    &mut shown,
                );
                ui.selected =
                    crate::ui::craft::first_pick(&shown, |r| craft::makeable_here(core, r));
                ui.count = 1;
                ui.skin = 0;
            }
        }
    }

    if keyboard.just_pressed(KeyCode::Escape) && ui.panel != Panel::None {
        // Out of the search box first: Esc there means "stop typing", and
        // a second one shuts the page.
        if ui.search_focus {
            ui.search_focus = false;
        } else {
            ui.panel = Panel::None;
            ui.drag = None;
        }
        ui.dirty = true;
        // Consumed: `pause::open` runs after this and would otherwise read
        // the same press and open the Esc menu behind the panel that just
        // closed.
        keyboard.clear_just_pressed(KeyCode::Escape);
    }

    // Leaving the inventory page — for the crafting page, or shut — closes
    // whatever container was open on it.
    //
    // **The server's idea of an open container outlives the panel drawing
    // it.** A container left open is one the sim keeps syncing to a screen
    // nobody is looking at, and — worse — the next `E` on a different box
    // arrives while the old one is still the open one, which is exactly the
    // container-state divergence `CLAUDE.md` names as the reference's own
    // worst bug on this verb. The close is sent when the panel that owned it
    // goes away, whichever key did it.
    if was_inventory && ui.panel != Panel::Inventory {
        super::verbs::close_container(&net, &mut toast);
    }

    // **Opening the panel no longer opens the body, because the body is
    // never shut.** Armor v1 sent an `ACT_CONTAINER(CONT_WEAR, 0)` here,
    // gated on the key rather than on the panel becoming visible, so that
    // `E` on a box — which also raises this panel — did not put two
    // container opens in one tick and let the box win or lose by
    // ordering. That whole hazard is gone with the shared slot: the wear
    // view has its own stream and is dripped unconditionally, so there is
    // nothing to open, nothing to race, and no ordering to depend on
    // (`NOW.md` §0eq item 4). The reference's arrangement is unchanged —
    // `inventory.jpeg` draws the worn slots as part of the inventory
    // screen rather than as a panel of their own, so there is still no
    // key to bind and none is invented.

    // The wheel wins over nothing and loses to the two toggle screens: a
    // player with the inventory (or the tree) open who brushes the button
    // is not asking for a wheel on top of it.
    if !matches!(ui.panel, Panel::Inventory | Panel::Craft | Panel::Tech) {
        let want = if holding_wheel {
            // One wheel per item (`crate::ui::hold`'s table). Opening the
            // OTHER item's wheel would place with the wrong verb, which is
            // why the hammer opened nothing until it had its own.
            match hand {
                crate::ui::hold::Held::Plan => Panel::Wheel,
                crate::ui::hold::Held::Hammer => Panel::Hammer,
                crate::ui::hold::Held::Other => Panel::None,
            }
        } else {
            Panel::None
        };
        if want != ui.panel {
            // Releasing the shape wheel commits whatever it was over — the
            // latch lives in `ui.shape`, which `wheel::track` has already
            // written, so there is nothing to resolve here.
            //
            // **Releasing the hammer's wheel FIRES what it was over** —
            // the deliberate difference (`NOW.md` §0p2 item 1): an action
            // is done once, not kept. The hover is last frame's
            // `wheel::track` answer, i.e. the wedge the highlight is
            // showing; `None` (dead centre, past the rim, or Escape's
            // close re-opening on the held button) fires nothing, which is
            // how a player backs out. Only the released wheel fires —
            // swapping hotbar slots mid-hold transitions Hammer→Wheel and
            // is not a release.
            if ui.panel == Panel::Hammer && want == Panel::None {
                if let Some(seg) = ui.hover {
                    super::verbs::hammer_fire(&net, &near.0, seg, &mut toast);
                }
            }
            ui.panel = want;
            ui.hover = None;
            ui.dirty = true;
        }
    }

    // Typing into the search box — only while it has the keyboard, which is
    // only on the crafting page. Enter hands the keyboard back, as Esc does
    // above.
    if typing && ui.panel == Panel::Craft {
        let mut changed = false;
        for ev in chars.read() {
            if !ev.state.is_pressed() {
                continue;
            }
            match &ev.logical_key {
                bevy::input::keyboard::Key::Backspace => {
                    ui.query.pop();
                    changed = true;
                }
                bevy::input::keyboard::Key::Enter => {
                    ui.search_focus = false;
                    ui.dirty = true;
                }
                bevy::input::keyboard::Key::Character(s) => {
                    // A bound on a field a player types into: wall 4 is
                    // about client-driven paths, and this is one.
                    for c in s.chars().filter(|c| !c.is_control()) {
                        if ui.query.chars().count() < MAX_QUERY_CHARS {
                            ui.query.push(c);
                            changed = true;
                        }
                    }
                }
                _ => {}
            }
        }
        if changed {
            ui.browser_scroll = 0.0;
            ui.dirty = true;
        }
    } else {
        // Drain, so a keystroke pressed while the box did not have the
        // keyboard does not arrive in it the moment it does.
        chars.clear();
    }

    // **The tree is a thing you stand at a bench to read.** When no bench of
    // the rung it was opened at stands within the station radius any more —
    // demolished, picked up, burned — it closes, on the sim's own scan at the
    // sim's own radius (`ui::techtree::bench_in_reach`), so it never offers a
    // button `research::unlock` would refuse for want of a bench. Movement is
    // zeroed while a panel is up, so a walk-off is not the case this catches;
    // the bench leaving is.
    if ui.panel == Panel::Tech
        && !crate::ui::techtree::bench_in_reach(
            core.deploys.entries(),
            &core.deploy_defs,
            core.predict.position(),
            ui.tech_tier,
        )
    {
        ui.panel = Panel::None;
        ui.tech_sel = None;
        ui.dirty = true;
        toast.warn("workbench out of reach");
    }
}

/// Put the sim's own refusals on the status line.
///
/// `last_move` is a counter the core bumps on every answered move and
/// `last_move_refused` is the reason latched beside it, so the counter is
/// what makes a repeated identical refusal visible — two failed drags onto a
/// full box are two events, and a panel that compared only the reason would
/// show the second one as nothing happening.
pub fn sync_refusals(mut ui: ResMut<Ui>, net: NonSend<super::Net>, mut seen: Local<u32>) {
    let core = &net.session.core;
    if core.last_move == *seen {
        return;
    }
    *seen = core.last_move;
    if core.last_move_refused > 0 {
        inv::note_refusal(&mut ui, core.last_move_refused);
    }
}

/// Rebuild the open panel when something it draws has changed.
///
/// One despawn and one build, never a diff: the panel is ~120 nodes and it
/// changes when a player clicks or a sync lands, so the simple shape is also
/// the cheap one. What is NOT cheap is rebuilding a still screen, which is
/// what `Seen` exists to prevent.
pub fn rebuild(
    mut commands: Commands,
    mut ui: ResMut<Ui>,
    net: NonSend<super::Net>,
    roots: Query<Entity, With<PanelRoot>>,
    // `Option`, because `icons::load` is a `Startup` system and a panel can
    // in principle be asked for before it has run. A missing registry draws
    // the labels it drew before rather than an empty ring.
    icons: Option<Res<super::icons::Icons>>,
    near: Res<super::verbs::Near>,
) {
    let core = &net.session.core;

    detect_changes(&mut ui, core, near.0);

    if !ui.dirty {
        return;
    }
    ui.dirty = false;

    for e in roots.iter() {
        commands.entity(e).despawn();
    }

    match ui.panel {
        Panel::None => {}
        Panel::Inventory => {
            let fallback = super::icons::Icons::default();
            let icons = icons.as_deref().unwrap_or(&fallback);
            inv::build_screen(&mut commands, &ui, core, icons, net.sel)
        }
        Panel::Craft => {
            let fallback = super::icons::Icons::default();
            let icons = icons.as_deref().unwrap_or(&fallback);
            craft::build_screen(&mut commands, &ui, core, icons)
        }
        Panel::Wheel => {
            let fallback = super::icons::Icons::default();
            let icons = icons.as_deref().unwrap_or(&fallback);
            wheel::build_screen(&mut commands, &ui, core, icons)
        }
        Panel::Hammer => {
            let fallback = super::icons::Icons::default();
            let icons = icons.as_deref().unwrap_or(&fallback);
            wheel::build_hammer_screen(&mut commands, &ui, core, near.0.as_ref(), icons)
        }
        Panel::Tech => {
            let fallback = super::icons::Icons::default();
            let icons = icons.as_deref().unwrap_or(&fallback);
            tech::build_screen(&mut commands, &ui, core, icons)
        }
    }
}

/// Observe only the facts panels draw, including the current hammer target.
fn detect_changes(
    ui: &mut Ui,
    core: &client_core::core::ClientCore,
    near: Option<crate::ui::structure::Target>,
) {
    // Change detection against the core's authoritative view.
    if ui.panel != Panel::None {
        let inv = core.inv;
        let cont = core.cont;
        let worn = core.worn;
        let table_lit = inv::open_table_running(core);
        if inv != ui.seen.inv
            || cont != ui.seen.cont
            || worn != ui.seen.worn
            || core.cont_kind != ui.seen.cont_kind
            || core.cont_handle != ui.seen.cont_handle
            || core.jobs != ui.seen.jobs
            || core.jobs_count != ui.seen.jobs_count
            || core.recipes_have != ui.seen.recipes_have
            || core.piece_defs_have != ui.seen.pieces_have
            || core.deploy_defs_have != ui.seen.deploys_have
            || (ui.panel == Panel::Hammer && near != ui.seen.hammer_target)
            || core.known() != ui.seen.known
            || core.research_have != ui.seen.research_have
            || table_lit != ui.seen.table_lit
            || core.skins_owned != ui.seen.skins_owned
            || core.skins_gen != ui.seen.skins_have
        {
            // The def tables drip in over the first seconds of a session, so
            // the derived category facts are rebuilt with them.
            if core.recipes_have != ui.seen.recipes_have {
                ui.facts = Facts::build(&core.recipes, &core.deploy_defs);
            }
            ui.seen.inv = inv;
            ui.seen.cont = cont;
            ui.seen.worn = worn;
            ui.seen.cont_kind = core.cont_kind;
            ui.seen.cont_handle = core.cont_handle;
            ui.seen.jobs = core.jobs;
            ui.seen.jobs_count = core.jobs_count;
            ui.seen.recipes_have = core.recipes_have;
            ui.seen.pieces_have = core.piece_defs_have;
            ui.seen.deploys_have = core.deploy_defs_have;
            ui.seen.hammer_target = near;
            ui.seen.known = core.known();
            ui.seen.research_have = core.research_have;
            ui.seen.table_lit = table_lit;
            ui.seen.skins_owned = core.skins_owned;
            ui.seen.skins_have = core.skins_gen;
            ui.dirty = true;
        }
    }
}

#[cfg(test)]
mod hammer_refresh_tests {
    use super::*;
    use crate::ui::structure::{Store, Target};
    use client_core::core::ClientCore;

    #[test]
    fn a_stationary_open_hammer_refreshes_on_target_damage_removal_and_new_defs() {
        let mut core = ClientCore::new(1, 0, 0);
        let mut ui = Ui {
            panel: Panel::Hammer,
            ..Ui::default()
        };
        let target = Target {
            store: Store::Piece,
            cx: 7,
            cz: 9,
            level: 1,
            loc: 1,
            row: 0,
            dmg: 0,
            hp_max: 500,
            side: Some(true),
        };
        detect_changes(&mut ui, &core, Some(target));
        assert!(ui.dirty);
        ui.dirty = false;
        detect_changes(&mut ui, &core, Some(target));
        assert!(!ui.dirty, "a still wheel must not rebuild every frame");
        let damaged = Target { dmg: 1, ..target };
        detect_changes(&mut ui, &core, Some(damaged));
        assert!(ui.dirty, "repair becomes available while the wheel is open");
        ui.dirty = false;
        let turned = Target {
            side: Some(false),
            ..damaged
        };
        detect_changes(&mut ui, &core, Some(turned));
        assert!(ui.dirty);
        ui.dirty = false;
        detect_changes(&mut ui, &core, None);
        assert!(ui.dirty, "a removed target must not keep its name or price");
        ui.dirty = false;
        core.deploy_defs_have = 1;
        detect_changes(&mut ui, &core, None);
        assert!(ui.dirty);
    }
}

#[cfg(test)]
mod ordering_tests {
    use super::*;

    #[test]
    fn hammer_target_resolution_and_panel_schedule_have_no_ordering_cycle() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::state::app::StatesPlugin));
        app.init_state::<crate::render::Screen>();
        register(&mut app);
        app.add_systems(
            Update,
            (
                crate::render::input::place_eye,
                crate::render::verbs::resolve.after(crate::render::input::place_eye),
                crate::render::verbs::keys.after(crate::render::verbs::resolve),
                crate::render::pause::open,
                crate::render::chat::keys.before(keys),
            ),
        );
        let world = app.world_mut();
        world.schedule_scope(Update, |world, schedule| {
            schedule
                .initialize(world)
                .expect("target resolution must precede the wheel without a cycle");
        });
    }
}
