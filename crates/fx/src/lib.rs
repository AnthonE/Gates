//! `fx`: effects with no engine in them.
//!
//! The particle pool and the vertex arithmetic that draws it ([`pool`]), the
//! generated sprite sheet ([`atlas`]), a layer's recipe and the knobs that
//! retune one without a rebuild ([`recipe`]), and the contract every game's
//! fx stage keeps ([`stage`]): one effect, two cards, a clock that steps.
//!
//! Plain numbers in, plain numbers out: no Bevy, no glam, no allocation after
//! a pool is made. Each game keeps a thin adapter that copies [`pool::Pool`]
//! into its own mesh and decides what a blow throws.
//!
//! **Canonical in Gates** (`crates/fx`). oneshot and Before Colony vendor this
//! directory with their `fx-sync.sh`, and `tests/vendored.rs` fails there if
//! it was edited in place: change it in Gates, then sync.

pub mod atlas;
pub mod pool;
pub mod recipe;
pub mod stage;
pub mod v3;

pub use v3::V3;
