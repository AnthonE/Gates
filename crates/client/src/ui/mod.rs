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

//! [`interact`] is the fourth and is not a panel: it is what the crosshair
//! is pointed at, and it lives here because a pick is arithmetic too — the
//! browser learned its `t > 0` guard by probing the resolver, which is only
//! possible when the resolver can be called without a window.
//!
//! [`load`] is not a panel either, and it is here for the same reason as
//! both: it decides *when a player enters the world*, and the version of
//! that decision that lived inside a Bevy system could only be tested by a
//! windowed run against a live shard.

pub mod build;
pub mod chat;
pub mod craft;
pub mod death;
pub mod interact;
pub mod load;
pub mod map;
pub mod place;
pub mod refusals;
pub mod slots;
pub mod structure;
