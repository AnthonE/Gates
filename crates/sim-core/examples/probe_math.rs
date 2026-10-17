//! The `math` line of the parity probe on its own (`probe::probe_math`), for
//! checking the transcendentals against wasm without the whole probe.
#![allow(clippy::disallowed_macros)]

fn main() {
    let math = sim_core::probe::probe_math();
    println!("math {} {:#010x}", math >> 32, math & 0xFFFF_FFFF);
}
