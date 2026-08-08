//! sim-core — the deterministic heart (DESIGN.md §4). World state, terrain,
//! movement, the command buffer, `state_hash`. No I/O, no clock, no
//! threads, no nondeterministic iteration, floats restricted to
//! `+ − × ÷ sqrt min max clamp floor-by-cast` (CLAUDE.md wall 1). Compiles
//! native (server truth) and wasm32 (client prediction + shared worldgen).
//! The clippy walls for this crate live in `clippy.toml` next door.

pub mod backpack;
pub mod bots;
pub mod build;
pub mod charge;
pub mod collide;
pub mod combat;
pub mod craft;
pub mod deploy;
pub mod fmath;
pub mod gather;
pub mod input;
pub mod inventory;
pub mod limits;
pub mod lock;
pub mod loot;
pub mod movement;
pub mod occupy;
pub mod persist;
mod pitch_lut;
pub mod probe;
pub mod ranged;
pub mod rng;
pub mod roster;
pub mod survival;
pub mod terrain;
pub mod world;
pub mod worldsave;
mod yaw_lut;

pub use pitch_lut::pitch_dir;
pub use yaw_lut::yaw_dir;

/// A big fixed array, built **on the heap** and never in a stack frame.
///
/// `Box::new([v; N])` materialises the whole array in the caller's frame
/// before moving it, and `World::new` builds a dozen of these back to
/// back. **wasm32's shadow stack is 1 MiB with no guard page**, so the
/// symptom of crossing it is `RuntimeError: memory access out of bounds`
/// from inside a constructor, on a tree whose every native test is green
/// — wall 1's own gate failing for a reason that has nothing to do with
/// determinism (`CLAUDE.md` §traps, measured 2026-08-08 twice: the lock
/// store found it, the hearth crew hit it again three hours later).
///
/// `vec![]` allocates and fills on the heap, so nothing large is ever in
/// a frame. Allocation is fine here for the ordinary reason: wall 2 bans
/// it **in the tick**, and every caller of this is construction.
pub(crate) fn boxed_array<T: Clone, const N: usize>(fill: T) -> Box<[T; N]> {
    let Ok(b) = vec![fill; N].into_boxed_slice().try_into() else {
        unreachable!("the vec is N long by construction")
    };
    b
}
