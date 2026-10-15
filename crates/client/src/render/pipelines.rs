//! How many render pipelines are still compiling, read from the render world.
//!
//! The loading screen used to end the moment the rings were full, while the
//! pipelines the world draws with could still be compiling. Natively a draw
//! whose pipeline is not ready is skipped, so the sky showed as a flat clear
//! colour for a few seconds after the overlay went. This counter lets
//! `loading::update` wait for them as well.
//!
//! `PipelineCache::waiting_pipelines` covers both the pipelines still being
//! created and those waiting on a shader asset that has not loaded yet (Bevy
//! re-queues those every frame). Pipelines that failed for good are not in
//! it, so a broken shader cannot hold the count up; `loading.rs` keeps a stall
//! guard anyway.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use bevy::prelude::*;
use bevy::render::render_resource::PipelineCache;
use bevy::render::{Render, RenderApp, RenderSystems};

/// Pipelines still compiling, as of the render world's last frame. Shared
/// between the two worlds; 0 when there is no render world (headless tests).
#[derive(Resource, Clone, Default)]
pub struct PipelineBacklog(Arc<AtomicU32>);

impl PipelineBacklog {
    pub fn pending(&self) -> u32 {
        self.0.load(Ordering::Relaxed)
    }
}

pub fn plugin(app: &mut App) {
    let backlog = PipelineBacklog::default();
    app.insert_resource(backlog.clone());
    if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
        render_app
            .insert_resource(backlog)
            .add_systems(Render, count.in_set(RenderSystems::Cleanup));
    }
}

/// After `RenderSystems::Render`, which is where the cache processes its
/// queue, so this frame's new pipelines are already counted.
fn count(cache: Res<PipelineCache>, backlog: Res<PipelineBacklog>) {
    let n = cache.waiting_pipelines().count() as u32;
    backlog.0.store(n, Ordering::Relaxed);
}
