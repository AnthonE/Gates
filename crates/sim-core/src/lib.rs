//! sim-core — the deterministic heart (DESIGN.md §4). World state, terrain,
//! movement, the command buffer, `state_hash`. No I/O, no clock, no
//! threads, no nondeterministic iteration, floats restricted to
//! `+ − × ÷ sqrt min max clamp floor-by-cast` (CLAUDE.md wall 1). Compiles
//! native (server truth) and wasm32 (client prediction + shared worldgen).
//! The clippy walls for this crate live in `clippy.toml` next door.

pub mod backpack;
pub mod bots;
pub mod build;
pub mod collide;
pub mod combat;
pub mod craft;
pub mod deploy;
pub mod fmath;
pub mod gather;
pub mod input;
pub mod inventory;
pub mod limits;
pub mod movement;
pub mod probe;
pub mod rng;
pub mod survival;
pub mod terrain;
pub mod world;
mod yaw_lut;

pub use yaw_lut::yaw_dir;
