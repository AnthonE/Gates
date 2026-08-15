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
//! - [`craft`] — the craft panel behind the reference `crafting.png`:
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
//!
//! [`keypad`] is the code lock's four digits (lock v1). Deliberately
//! pointer-free — a screen that takes your mouse to type four digits is a
//! screen you die in — so the render layer draws it as a small overlay
//! that grabs nothing, and the buffer rules and the key→op map are here
//! where a test can hold them.
//!
//! [`hammer`] is the hammer's wheel — the second radial, whose segments
//! fire on release where [`build`]'s latch.
//!
//! [`hub`] is the third piece of the front door and the one that is not
//! ours: NEWS, ITEM STORE and WORKSHOP are the scry-works launcher's
//! (operator, 2026-08-09), so this owns which state each is in and what the
//! screen says about it — four states, never collapsed, because "no launcher"
//! and "not published yet" have different fixes.
//!
//! [`boot`] and [`servers`] are the two halves of the front door, and they
//! are here for [`load`]'s reason exactly. `boot` decides when the splash
//! lifts and which screen it lifts into — a one-bit answer that used to be a
//! branch in `main` before any window existed. `servers` is the browser
//! behind PLAY GAME: filtering, the busy-first sort and the favourite set,
//! all of which are arithmetic a windowed run is a bad way to check.

/// The longest search a player can type, in either search box.
///
/// **One constant because it is one rule**, and the knob-registry gate is
/// what made that explicit: the server browser landed with its own copy at a
/// different value, and a name that means two things is a name the registry
/// cannot be authoritative about. The value is the craft panel's shipped 32,
/// not the browser's invented 40 — a cap that was already in players' hands
/// does not move because a second screen wanted one.
///
/// Not a knob worth a registry row, in the panel's own words: it exists
/// because an unbounded string fed by a keyboard is an unbounded string
/// (wall 4, applied to a client-driven path).
pub const MAX_QUERY_CHARS: usize = 32;

pub mod boot;
pub mod build;
pub mod chat;
pub mod craft;
pub mod death;
pub mod hammer;
pub mod hold;
pub mod hub;
pub mod icons;
pub mod interact;
pub mod keypad;
pub mod load;
pub mod map;
pub mod place;
pub mod refusals;
pub mod servers;
pub mod slots;
pub mod structure;
pub mod techtree;
