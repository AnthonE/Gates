//! The in-game menus' **model** — every number a panel needs, computed
//! outside Bevy.
//!
//! **Why this is not in `render/`.** `RENDER.md` §1's rule is that Bevy
//! draws and does not decide, and a menu is where that rule is easiest to
//! break: a drag has arithmetic in it, a craft has affordability in it, and
//! a build wheel has an angle in it. Put any of that in a system and the
//! only thing that can test it is a windowed run. Everything here is a pure
//! function of `ClientCore`'s own tables plus the pointer, so
//! `crates/client/tests/ui.rs` drives all of it **headless, with no
//! `render` feature and no GPU** — which is also why this module is not
//! feature-gated.
//!
//! The three panels, and what each owes:
//!
//! - [`load`] — the loading screen's model. Not a panel, and here for the
//!   same reason they are: it decides *when a player enters the world*, and
//!   the version of that decision that lived inside a Bevy system could
//!   only be tested by a windowed run against a live shard.
//! - [`slots`] — the inventory grid and the container panel. Owns the
//!   move verb's marshalling, which `CLAUDE.md` names as the most
//!   bug-prone thing in the reference: three Oxide fixes in 28 minutes on
//!   one day, all landing as *the server disconnecting the client*.
//! - [`craft`] — the craft panel behind `Rust Images/crafting.png`:
//!   category rail, search, the detail pane, and the
//!   AMOUNT/ITEM TYPE/TOTAL/HAVE ingredient table.
//! - [`build`] — the radial build menu. Shape on the outer ring, material
//!   on the inner one, because that is what `content/building.toml`
//!   actually is: 6 shapes × 3 materials.
//!
//! Nothing here reads a clock, opens a socket, or holds a handle to
//! anything. A panel that wants one of those asks the render layer for it.

pub mod build;
pub mod craft;
pub mod load;
pub mod slots;
