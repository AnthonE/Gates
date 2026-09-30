//! The agent player's parts below the mind: what a skill asks of the body
//! ([`intent`]) and what a player knows of the game's rules ([`wiki`]).
//! `explorer.rs` stays the orchestrator; these modules never drain
//! `ClientCore`'s event rings — `Survivor::event_with` is the one reader
//! and hands them facts.
//!
//! Everything under here is held to the agent walls
//! (`tests/agent_walls.rs`): human verbs and buttons only, no server-side
//! state by name, and nothing allocated on the frame path.

pub mod intent;
pub mod wiki;
