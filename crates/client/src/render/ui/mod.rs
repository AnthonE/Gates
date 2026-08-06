//! The in-game menus, drawn.
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
//! **It does not open under `--capture`.** The UI systems are registered
//! only on a non-capture run. A probe harness that could open a panel is a
//! visual gate whose frames depend on which key was last pressed, and the
//! capture path enters the world on frame one specifically so that nothing
//! is halfway through anything. The cost is that no gate photographs these
//! panels yet — `NOW.md` carries that, and it is the same missing menu
//! vantage §0v already names.
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

// ---- the shared palette --------------------------------------------------
// `ART.md` is the visual bar and it is about the world, not the HUD; these
// are the browser client's menu values carried over so the two clients read
// as the same game. Proposed defaults, `DECISIONS.md` §open "menu skin v0".

/// Panel body: nearly opaque, so text over a lit world stays readable.
pub const PANEL_BG: Color = Color::srgba(0.086, 0.086, 0.094, 0.97);
/// The screen-wide scrim behind a panel.
pub const SCRIM: Color = Color::srgba(0.02, 0.02, 0.025, 0.72);
/// An empty cell.
pub const CELL_BG: Color = Color::srgba(0.15, 0.15, 0.16, 0.9);
/// A cell holding something.
pub const CELL_FULL: Color = Color::srgba(0.22, 0.21, 0.19, 0.95);
/// The cell the pointer is over, and the drag's source.
pub const CELL_HOVER: Color = Color::srgba(0.34, 0.32, 0.26, 0.95);
pub const LINE: Color = Color::srgba(0.75, 0.72, 0.62, 0.22);
pub const LINE_HOT: Color = Color::srgba(0.98, 0.86, 0.55, 0.95);
pub const TEXT: Color = Color::srgb(0.92, 0.90, 0.85);
pub const TEXT_DIM: Color = Color::srgba(0.70, 0.68, 0.62, 0.85);
/// A price the player cannot pay, and the reference's own colour for it.
pub const TEXT_SHORT: Color = Color::srgb(0.86, 0.36, 0.30);
/// The station badge.
pub const BADGE: Color = Color::srgb(0.83, 0.74, 0.28);

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
    app.init_resource::<Ui>().add_systems(
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
            .run_if(in_state(super::Screen::InWorld)),
    );
}

/// Open, close, and the keys that belong to a panel rather than to the
/// world. Runs before `input::gather`'s own key reads by being earlier in
/// the chain; `gather` then skips look and movement while a panel is up.
pub fn keys(
    mut ui: ResMut<Ui>,
    net: NonSend<super::Net>,
    keyboard: Res<ButtonInput<KeyCode>>,
    mut chars: MessageReader<bevy::input::keyboard::KeyboardInput>,
) {
    // The wheel is a hold, so it is decided every frame rather than latched.
    let holding_wheel = keyboard.pressed(KeyCode::KeyB);

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
