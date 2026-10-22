//! POTATO: the bottom graphics rung, and the game picking it for a player
//! whose machine needs it (operator, 2026-10-22: *"make a new graphics mode
//! called potato for web … try to auto detect when they need it too"*).
//!
//! The rung itself is a column in `quality::preset`. This is the picking. It
//! happens only while `Settings::quality_auto` holds (the player has not
//! touched a graphics row), and it only ever moves the preset DOWN:
//!
//!   * **At boot, the machine** ([`boot_hint`]): a software rasterizer
//!     (SwiftShader, llvmpipe, WARP), a phone or tablet, a page reporting
//!     2 GB or less, or 4 GB on 4 cores or fewer. Straight to POTATO.
//!     `?potato` in the address forces it whatever the player chose.
//!   * **In a browser, the frames** ([`watch`]): after a warmup, a median
//!     frame slower than [`SLOW_FPS`] over a [`WINDOW_S`] window steps the
//!     preset down one rung, or straight to POTATO under [`CRAWL_FPS`]. The
//!     median, not the mean, so a shader compiling or a chunk landing is not
//!     a slow machine. It stands down for the session after
//!     [`SETTLED_WINDOWS`] good windows in a row, so a CPU-bound raid late in
//!     a session cannot walk a good machine down for something graphics
//!     would not fix.
//!
//! Either way the toast says what happened and where to undo it, and the
//! rung is saved, so the next boot opens on it.

use bevy::prelude::*;
use bevy::render::renderer::RenderAdapterInfo;

use crate::config::Quality;

use super::hud::Toast;
use super::quality;
use super::screen::Screen;
use super::Settings;

/// Below this many frames a second (median) the preset steps down a rung.
pub const SLOW_FPS: f32 = 24.0;
/// Below this it goes straight to POTATO.
pub const CRAWL_FPS: f32 = 14.0;
/// In-world seconds before the first window: shaders compile and the rings
/// fill on arrival, and none of that is the machine's frame rate.
pub const WARMUP_S: f32 = 12.0;
/// In-world seconds skipped after a step, for the pipelines it re-specializes.
pub const SETTLE_S: f32 = 6.0;
/// Seconds of frames one verdict is drawn from.
pub const WINDOW_S: f32 = 8.0;
/// Good windows in a row after which the watch stands down for the session.
pub const SETTLED_WINDOWS: u8 = 3;
/// A frame longer than this is a hidden tab or a stall, not a frame rate.
pub const STALL_S: f32 = 0.5;

/// What the boot hint reads about the machine.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Machine {
    /// wgpu's adapter is a CPU rasterizer.
    pub software_gpu: bool,
    pub phone: bool,
    /// `navigator.deviceMemory`, GB (Chromium only; rounded down to a power
    /// of two and capped at 8).
    pub memory_gb: Option<f32>,
    /// `navigator.hardwareConcurrency`.
    pub cores: Option<u32>,
    /// `?potato` in the page's address.
    pub forced: bool,
}

/// Why this machine should start on POTATO, if it should.
pub fn boot_hint(m: &Machine) -> Option<&'static str> {
    if m.forced {
        return Some("asked for in the address");
    }
    if m.software_gpu {
        return Some("no graphics card found, drawing on the CPU");
    }
    if m.phone {
        return Some("phone or tablet");
    }
    let mem = m.memory_gb.unwrap_or(f32::MAX);
    let cores = m.cores.unwrap_or(u32::MAX);
    if mem <= 2.0 || (mem <= 4.0 && cores <= 4) {
        return Some("low-memory machine");
    }
    None
}

/// Whether an adapter is a software rasterizer. wgpu calls it a `Cpu`
/// device; WebGL2 names it in the renderer string instead.
pub fn is_software(device_type_debug: &str, name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    device_type_debug == "Cpu"
        || [
            "swiftshader",
            "llvmpipe",
            "softpipe",
            "software",
            "basic render",
        ]
        .iter()
        .any(|s| name.contains(s))
}

/// The rung a median frame of `median_s` seconds moves `q` to, or `None` to
/// stay.
pub fn verdict(q: Quality, median_s: f32) -> Option<Quality> {
    if median_s <= 1.0 / SLOW_FPS || q == Quality::Potato {
        return None;
    }
    if median_s > 1.0 / CRAWL_FPS {
        return Some(Quality::Potato);
    }
    let at = Quality::LADDER.iter().position(|l| *l == q)?;
    Some(Quality::LADDER[at.saturating_sub(1)])
}

/// The watch's state for the session, and the line owed to the toast.
#[derive(Resource)]
pub struct Watch {
    warm_s: f32,
    frames: Vec<f32>,
    sampled_s: f32,
    good: u8,
    note: Option<String>,
}

impl Default for Watch {
    fn default() -> Self {
        Self {
            warm_s: WARMUP_S,
            frames: Vec::with_capacity(1024),
            sampled_s: 0.0,
            good: 0,
            note: None,
        }
    }
}

/// The machine this build runs on, as far as it can tell.
fn machine(adapter: Option<&RenderAdapterInfo>) -> Machine {
    let software_gpu =
        adapter.is_some_and(|a| is_software(&format!("{:?}", a.device_type), &a.name));
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::JsValue;
        let Some(w) = web_sys::window() else {
            return Machine {
                software_gpu,
                ..default()
            };
        };
        let nav = w.navigator();
        let ua = nav.user_agent().unwrap_or_default();
        let memory_gb = js_sys::Reflect::get(&nav, &JsValue::from_str("deviceMemory"))
            .ok()
            .and_then(|v| v.as_f64())
            .map(|v| v as f32);
        let cores = Some(nav.hardware_concurrency() as u32).filter(|c| *c > 0);
        let forced = w.location().search().is_ok_and(|q| {
            q.trim_start_matches('?')
                .split('&')
                .any(|kv| kv.split('=').next() == Some("potato"))
        });
        Machine {
            software_gpu,
            phone: ["Android", "iPhone", "iPad", "Mobile"]
                .iter()
                .any(|s| ua.contains(s)),
            memory_gb,
            cores,
            forced,
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        Machine {
            software_gpu,
            ..default()
        }
    }
}

/// Once, from `RenderPlugin::finish`: the device exists, the settings are
/// loaded, and no camera has been given a tier yet.
pub fn at_boot(app: &mut App) {
    let m = machine(app.world().get_resource::<RenderAdapterInfo>());
    let Some(why) = boot_hint(&m) else {
        return;
    };
    let world = app.world_mut();
    let Some(mut s) = world.get_resource_mut::<Settings>() else {
        return;
    };
    if s.quality == Quality::Potato && s.preset_name() == Some(Quality::Potato) {
        return;
    }
    if !(s.quality_auto || m.forced) {
        return;
    }
    s.quality = Quality::Potato;
    s.gfx = quality::effective(quality::preset(Quality::Potato));
    info!("gfx: POTATO at boot ({why}); {m:?}");
    let note = format!("graphics set to POTATO ({why}) — SETTINGS > GRAPHICS to change");
    if let Some(mut w) = world.get_resource_mut::<Watch>() {
        w.note = Some(note);
    }
}

/// `Update`: say what the boot hint did once a world is up, and in a browser
/// watch the frame rate and step the preset down while it stays too slow.
pub fn watch(
    time: Res<Time<Real>>,
    state: Res<State<Screen>>,
    mut settings: ResMut<Settings>,
    mut w: ResMut<Watch>,
    toast: Option<ResMut<Toast>>,
    capture: Option<Res<super::capture::Capture>>,
) {
    if capture.is_some() || *state.get() != Screen::InWorld {
        return;
    }
    if let Some(mut toast) = toast {
        if let Some(note) = w.note.take() {
            toast.say(note);
        }
    }
    if !cfg!(target_arch = "wasm32")
        || !settings.quality_auto
        || settings.quality == Quality::Potato
        || settings.background
        || w.good >= SETTLED_WINDOWS
    {
        return;
    }
    let dt = time.delta_secs();
    if w.warm_s > 0.0 {
        w.warm_s -= dt.min(STALL_S);
        return;
    }
    if dt > STALL_S {
        return;
    }
    w.frames.push(dt);
    w.sampled_s += dt;
    if w.sampled_s < WINDOW_S {
        return;
    }
    let w = &mut *w;
    w.frames.sort_unstable_by(f32::total_cmp);
    let median = w.frames[w.frames.len() / 2];
    w.frames.clear();
    w.sampled_s = 0.0;
    let Some(next) = verdict(settings.quality, median) else {
        w.good += 1;
        return;
    };
    w.good = 0;
    w.warm_s = SETTLE_S;
    let fps = (1.0 / median).round();
    info!(
        "gfx: {fps} fps median on {:?}, stepping down to {next:?}",
        settings.quality
    );
    settings.quality = next;
    settings.gfx = quality::effective(quality::preset(next));
    w.note = Some(format!(
        "graphics lowered to {} ({fps} fps) — SETTINGS > GRAPHICS to change",
        next.name().to_uppercase()
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_slow_frame_steps_one_rung_and_a_crawl_goes_to_the_bottom() {
        let at = |fps: f32| 1.0 / fps;
        assert_eq!(verdict(Quality::High, at(60.0)), None);
        assert_eq!(verdict(Quality::High, at(25.0)), None);
        assert_eq!(verdict(Quality::High, at(20.0)), Some(Quality::Medium));
        assert_eq!(verdict(Quality::Medium, at(20.0)), Some(Quality::Low));
        assert_eq!(verdict(Quality::Low, at(20.0)), Some(Quality::Potato));
        assert_eq!(verdict(Quality::High, at(10.0)), Some(Quality::Potato));
        assert_eq!(verdict(Quality::Potato, at(5.0)), None);
    }

    #[test]
    fn the_boot_hint_reads_the_weak_machines_and_leaves_the_rest() {
        let ok = Machine {
            memory_gb: Some(8.0),
            cores: Some(8),
            ..default()
        };
        assert_eq!(boot_hint(&ok), None);
        assert_eq!(boot_hint(&Machine::default()), None, "unknown is fine");
        for weak in [
            Machine {
                software_gpu: true,
                ..ok
            },
            Machine { phone: true, ..ok },
            Machine {
                memory_gb: Some(2.0),
                ..ok
            },
            Machine {
                memory_gb: Some(4.0),
                cores: Some(4),
                ..ok
            },
            Machine { forced: true, ..ok },
        ] {
            assert!(boot_hint(&weak).is_some(), "{weak:?}");
        }
        assert_eq!(
            boot_hint(&Machine {
                memory_gb: Some(4.0),
                ..ok
            }),
            None,
            "4 GB on 8 cores is not a potato"
        );
    }

    #[test]
    fn software_adapters_are_named_either_way() {
        assert!(is_software("Cpu", "llvmpipe (LLVM 15.0.7, 256 bits)"));
        assert!(is_software(
            "Other",
            "ANGLE (Google, Vulkan 1.3.0 (SwiftShader Device (Subzero)), SwiftShader driver)"
        ));
        assert!(!is_software("IntegratedGpu", "Intel(R) UHD Graphics 620"));
        assert!(!is_software("DiscreteGpu", "NVIDIA GeForce RTX 3060"));
    }
}
