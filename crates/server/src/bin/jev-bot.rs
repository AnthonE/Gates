//! `jev-bot`: `gates-agent` under its old name, kept for one release while
//! the units and scripts that start it move over. The same program, built
//! from the same file.

#[path = "gates-agent.rs"]
mod gates_agent;

fn main() {
    gates_agent::main();
}
