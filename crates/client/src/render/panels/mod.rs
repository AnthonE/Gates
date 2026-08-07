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
//! | inventory + crafting | `Tab` | `Rust Images/inventory.jpeg`, `crafting.png` |
//! | container | opens itself when the sim says one is open | `storageandtoolchest.jpeg` |
//! | build wheel | hold `B` | the radial in the operator's second frame |
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

use crate::ui::build::Hover;
use crate::ui::craft::{Cat, Facts};
use crate::ui::slots::Drag;

pub mod craft;
pub mod inv;
pub mod wheel;

/// Which menu is up. One at a time: the wheel is a hold and the inventory
/// is a toggle, so they cannot both be open without a rule about which one
/// the pointer belongs to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Panel {
    #[default]
    None,
    /// The `Tab` screen: crafting on the left, your inventory below, the
    /// open container beside it when there is one.
    Inventory,
    /// The build wheel, up for as long as its key is held.
    Wheel,
}

impl Panel {
    /// Does this panel own the pointer? A panel that does releases the
    /// cursor and takes look and movement away from `input::gather` — a
    /// player dragging an item is not also turning around.
    pub fn grabs_pointer(self) -> bool {
        !matches!(self, Panel::None)
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
    /// Starred recipes. A local latch — the reference's FAVOURITE is one
    /// too, and nothing on our wire carries a favourite.
    pub favs: Vec<u16>,
    /// The recipe the detail pane is showing.
    pub selected: Option<u16>,
    /// The quantity stepper, always ≥ 1.
    pub count: u16,
    /// The one line under the title that says what just happened — a
    /// refusal, a full action lane, a craft that went in. Never empty for
    /// long, and never silently empty: a panel that cannot say why it did
    /// nothing is the dark-panel defect.
    pub status: String,
    /// Derived category facts, rebuilt when the content tables drip in.
    pub facts: Facts,
    /// The wheel's latched choice: indices into `ui::build::SHAPES` and
    /// `MATERIALS`. Latched rather than momentary, so releasing the wheel
    /// over nothing keeps what was chosen last.
    pub shape: usize,
    pub material: usize,
    /// What the wheel's pointer is over this frame.
    pub hover: Option<Hover>,
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
    pub inv: [(u16, u16); sim_core::limits::INV_SLOTS],
    pub cont: [(u16, u16); sim_core::limits::INV_SLOTS],
    pub cont_kind: u8,
    pub cont_handle: u32,
    pub jobs: [(u8, u8); sim_core::limits::CRAFT_QUEUE],
    pub jobs_count: u8,
    pub recipes_have: u16,
    pub pieces_have: u16,
}

impl Default for Ui {
    fn default() -> Self {
        Self {
            panel: Panel::None,
            drag: None,
            cat: Cat::All,
            query: String::new(),
            favs: Vec::new(),
            selected: None,
            count: 1,
            status: String::new(),
            facts: Facts::default(),
            shape: 0,
            material: 0,
            hover: None,
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

/// Grid cell edge, px. Proposed default, same `DECISIONS.md` row.
///
/// **Sized against 720p, which is the constraint that decides it.** Bevy's
/// default window is 1280×720 and the whole screen is one column: title,
/// browser, queue, your thirty slots, hint. At 54 px that column measured
/// ~830 px tall and a centred overflow clips at BOTH ends — the first cut
/// lost the title off the top and the last two inventory rows off the
/// bottom, and neither is visible from the code. [`PANEL_H`] and
/// [`BROWSER_COLS`] are the rest of the same budget.
pub const CELL_PX: f32 = 44.0;
pub const CELL_GAP_PX: f32 = 4.0;

/// Height of the browser and the detail pane, px — the two tall things, and
/// therefore the ones that pay for the rest of the budget above. What is
/// left after the fixed rows at 720p: title 30, status 16, queue 46, your
/// thirty slots 285, hint 16, four 8 px gaps.
pub const PANEL_H: f32 = 276.0;

/// Columns in the recipe browser. Eight rather than the inventory's six —
/// the recipe list is longer than an inventory and is read by name, not by
/// slot number.
pub const BROWSER_COLS: u16 = 8;

/// The recipe grid's own height inside [`PANEL_H`], leaving room for the
/// search box under it. **The grid scrolls**: content grows with
/// `content/recipes.toml` and a browser sized to today's 36 recipes is a
/// browser that silently hides the 37th.
pub const BROWSER_GRID_H: f32 = 218.0;

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
                craft::clicks,
                craft::scroll,
                wheel::track,
                sync_refusals,
                rebuild,
                inv::ghost_follow,
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
pub fn keys(
    mut ui: ResMut<Ui>,
    net: NonSend<super::Net>,
    mut toast: ResMut<super::hud::Toast>,
    mut keyboard: ResMut<ButtonInput<KeyCode>>,
    mut chars: MessageReader<bevy::input::keyboard::KeyboardInput>,
) {
    // The wheel is a hold, so it is decided every frame rather than latched.
    let holding_wheel = keyboard.pressed(KeyCode::KeyB);
    let was_inventory = ui.panel == Panel::Inventory;

    if keyboard.just_pressed(KeyCode::Tab) {
        ui.panel = match ui.panel {
            Panel::Inventory => Panel::None,
            _ => Panel::Inventory,
        };
        ui.drag = None;
        ui.dirty = true;
    }

    if keyboard.just_pressed(KeyCode::Escape) && ui.panel != Panel::None {
        ui.panel = Panel::None;
        ui.drag = None;
        ui.dirty = true;
        // Consumed: `pause::open` runs after this and would otherwise read
        // the same press and open the Esc menu behind the panel that just
        // closed.
        keyboard.clear_just_pressed(KeyCode::Escape);
    }

    // Closing the inventory closes whatever container was open beside it.
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

    // The wheel wins over nothing and loses to the inventory screen: a
    // player with the inventory open who brushes B is not asking for a
    // wheel on top of it.
    if ui.panel != Panel::Inventory {
        let want = if holding_wheel {
            Panel::Wheel
        } else {
            Panel::None
        };
        if want != ui.panel {
            // Releasing the wheel commits whatever it was over — the latch
            // lives in `ui.shape`/`ui.material`, which `wheel::track` has
            // already written, so there is nothing to resolve here.
            ui.panel = want;
            ui.hover = None;
            ui.dirty = true;
        }
    }

    // Typing into the search box. Only while the inventory screen is up, so
    // the world's own binds are untouched everywhere else.
    if ui.panel == Panel::Inventory {
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
            ui.dirty = true;
        }
    } else {
        // Drain, so a keystroke pressed with the panel shut does not arrive
        // in the search box the moment it opens.
        chars.clear();
    }

    // The sim can close a container out from under an open panel — it
    // despawned, or the player walked out of reach — and the panel is never
    // authoritative about its own visibility.
    let _ = &net;
}

/// The longest search a player can type. Not a knob worth a row: it exists
/// because an unbounded string fed by a keyboard is an unbounded string.
pub const MAX_QUERY_CHARS: usize = 32;

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
) {
    let core = &net.session.core;

    // Change detection against the core's authoritative view.
    if ui.panel != Panel::None {
        let inv: [(u16, u16); sim_core::limits::INV_SLOTS] =
            std::array::from_fn(|i| (core.inv[i].item, core.inv[i].count));
        let cont: [(u16, u16); sim_core::limits::INV_SLOTS] =
            std::array::from_fn(|i| (core.cont[i].item, core.cont[i].count));
        if inv != ui.seen.inv
            || cont != ui.seen.cont
            || core.cont_kind != ui.seen.cont_kind
            || core.cont_handle != ui.seen.cont_handle
            || core.jobs != ui.seen.jobs
            || core.jobs_count != ui.seen.jobs_count
            || core.recipes_have != ui.seen.recipes_have
            || core.piece_defs_have != ui.seen.pieces_have
        {
            // The def tables drip in over the first seconds of a session, so
            // the derived category facts are rebuilt with them.
            if core.recipes_have != ui.seen.recipes_have {
                ui.facts = Facts::build(&core.recipes, &core.deploy_defs);
            }
            ui.seen.inv = inv;
            ui.seen.cont = cont;
            ui.seen.cont_kind = core.cont_kind;
            ui.seen.cont_handle = core.cont_handle;
            ui.seen.jobs = core.jobs;
            ui.seen.jobs_count = core.jobs_count;
            ui.seen.recipes_have = core.recipes_have;
            ui.seen.pieces_have = core.piece_defs_have;
            ui.dirty = true;
        }
    }

    if !ui.dirty {
        return;
    }
    ui.dirty = false;

    for e in roots.iter() {
        commands.entity(e).despawn();
    }

    match ui.panel {
        Panel::None => {}
        Panel::Inventory => inv::build_screen(&mut commands, &ui, core),
        Panel::Wheel => wheel::build_screen(&mut commands, &ui, core),
    }
}
