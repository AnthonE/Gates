//! The agent player's parts below the mind: what a skill asks of the body
//! ([`intent`]), the hands that turn it into input at a person's speed
//! ([`hands`]), what its eyes and ears know of other bodies ([`tracks`]),
//! what a player knows of the game's rules ([`wiki`]), the legs that find
//! a way somewhere ([`route`]), the reflex that answers a fight
//! ([`combat`]), where a shot must go to land ([`aim`]), somewhere to
//! hide from one ([`cover`]) and what goes on the belt for it
//! ([`loadout`]).
//! `explorer.rs` stays the orchestrator; these modules never drain
//! `ClientCore`'s event rings — `Survivor::event_with` is the one reader
//! and hands them facts.
//!
//! Everything under here is held to the agent walls
//! (`tests/agent_walls.rs`): human verbs and buttons only, no server-side
//! state by name, and nothing allocated on the frame path.

pub mod aim;
pub mod combat;
pub mod cover;
pub mod hands;
pub mod intent;
pub mod loadout;
pub mod route;
pub mod tracks;
pub mod wiki;
