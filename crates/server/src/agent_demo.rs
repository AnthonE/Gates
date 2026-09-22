//! Shared setup for the headless explorer and its rendered local broadcast.
use crate::jev::{Action, Decision, DecisionSource, Observation};

/// Explicit offline movement, never a fallback for a failed model request.
pub struct Scripted;
impl DecisionSource for Scripted {
    fn decide(&mut self, observation: Observation) -> Result<Decision, String> {
        Ok(Decision {
            action: if observation.moved_m() == Some(0.0)
                && observation.previous_action != Action::Wait
            {
                Action::Right
            } else {
                Action::Forward
            },
            confidence: 1.0,
            input_tokens: 0,
        })
    }
}

pub async fn spawn_local() -> Result<crate::net::ShardHandle, String> {
    let mut cfg = crate::config::parse_shard_toml(include_str!("../../../shard.toml.example"))?;
    cfg.bind = "127.0.0.1:0".parse().expect("loopback");
    cfg.require_auth = false;
    cfg.population = 0;
    cfg.dev_spawn = Some(sim_core::world::World::new(cfg.seed).spawn_pos(0));
    let content = content::Content::load_dir(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content"),
    )
    .map_err(|e| format!("content: {e}"))?;
    crate::net::spawn_shard(
        cfg,
        crate::net::bake_all(&content)?,
        crate::store::Saves::off(),
        crate::worldfile::WorldBoot::off(),
    )
    .await
}
