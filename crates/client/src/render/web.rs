//! What a browser build does where the desktop has a window and a menu.
//!
//! Two things, and both are wasm-only in EFFECT while being compiled on every
//! target — the arithmetic is gated natively (`tests/web.rs`) and the systems
//! are registered under `run_if(|| cfg!(target_arch = "wasm32"))` in
//! `render/mod.rs`, the same shape as `tree::swap_by_distance`, so a native
//! build carries a no-op body rather than a `cfg` hole a test cannot see.
//!
//! ## The surface, and the number a page cannot exceed
//!
//! WebGL2's `max_texture_dimension_2d` is **2048** (`downlevel_webgl2_
//! defaults`), and wgpu REFUSES `configure_surface` above it — a 1080p
//! display at devicePixelRatio 2 asks for 3840 and gets no frame and no error
//! a player can read (`findings/web-build-20260909.md` §15.8). Bevy passes
//! the window's resolution straight through with only `.max(1)`.
//!
//! The first browser build pinned the canvas at 1280 × 720 with a scale
//! override of 1.0, which never exceeds the cap and never fills a window
//! either; on a retina display winit then drew it at 640 CSS pixels. What
//! replaces it is one function, [`fit`], and a page-side stretch:
//!
//!   1. The backing store is the viewport in device pixels, scaled DOWN
//!      uniformly until both axes are inside the cap.
//!   2. winit sizes the canvas's CSS box at `backing / devicePixelRatio`
//!      (its `ResizeObserver` on the device-pixel content box is what sets
//!      the backing store, so the CSS box has to be exactly that), and the
//!      canvas is then stretched back over the viewport with a CSS
//!      `transform: scale(stretch)`.
//!
//! **Why a transform and not a bigger CSS box.** `MouseEvent.offsetX` is in
//! the element's OWN coordinate space — a browser maps the pointer through
//! the inverse transform — so winit's pointer arithmetic (`offset × dpr`)
//! keeps landing inside the backing store, and Bevy's UI, laid out at
//! `backing / dpr` logical pixels, is hit where it is drawn. A CSS box wider
//! than the backing store would break exactly that: the observer would grow
//! the backing store past the cap, and the pointer would land off by the
//! ratio. A transform is invisible to the observer and to `offsetX` both.
//!
//! What it costs, said out loud: on a display where the cap binds, the frame
//! is drawn at fewer device pixels than the display has and the UI is
//! magnified by `stretch` along with it. On every display under 2048 device
//! pixels a side, `stretch` is exactly 1.0 and nothing here changes a pixel.
//!
//! ## The page is the menu
//!
//! `Screen::Menu` is a dead end in a browser: nothing draws it (`menu` is
//! `cfg`'d off wasm32), and the `Session` is one-shot there —
//! `world_teardown` drops `Net`, which drops the `WebTransport`, and a page
//! cannot re-dial without the wallet handshake it did before Bevy started.
//! `AppExit` is no better: Bevy's wasm runner simply stops scheduling frames,
//! which is a frozen canvas. Both were written up at `render/mod.rs` as the
//! two runtime gaps the desktop-front-end cut created. [`hand_back`] is the
//! answer the architecture already implied: leaving the world hands control
//! to the PAGE — `window.gatesLeft(status)` if the page installed one, else a
//! reload — and the page draws whatever it wants a player to see next.

use bevy::prelude::*;
use bevy::window::PrimaryWindow;

/// WebGL2's largest surface on either axis, device pixels.
///
/// `downlevel_webgl2_defaults().max_texture_dimension_2d` — the swapchain is
/// a texture and this is its ceiling. Not a knob: it is the backend's, and
/// a page that asks for more draws nothing.
pub const SURFACE_CAP_PX: u32 = 2048;

/// The canvas the page handed Bevy, as the CSS selector it was named by.
///
/// Inserted by `client-web`'s `Gates::play` so [`follow_viewport`] can find
/// the element winit is drawing into and set its transform; absent on the
/// desktop, where the system never runs.
#[derive(Resource, Clone, Debug)]
pub struct Canvas(pub String);

/// How a viewport fits inside the surface cap.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fit {
    /// The backing store, device pixels — never over [`SURFACE_CAP_PX`] on
    /// either axis, and the viewport's own aspect.
    pub physical: (u32, u32),
    /// Device pixels per CSS pixel of the VIEWPORT this backing store gives:
    /// `devicePixelRatio` while the cap does not bind, less once it does.
    pub scale: f32,
    /// The CSS `transform: scale()` that takes the canvas winit sizes at
    /// `physical / devicePixelRatio` back over the whole viewport:
    /// `devicePixelRatio / scale`, so exactly `1.0` while the cap does not
    /// bind.
    pub stretch: f32,
}

/// Fit a viewport of `css_w × css_h` CSS pixels at `dpr` inside the cap.
///
/// Uniform: one scale for both axes, the smaller of `dpr` and what each axis
/// allows, so the aspect is the viewport's. Rounded to whole device pixels
/// and clamped once more after rounding, so a viewport whose long side is
/// exactly the cap's worth cannot round past it. A non-finite or
/// non-positive `dpr` is taken as 1.0 — a page can report one before layout.
pub fn fit(css_w: f32, css_h: f32, dpr: f32) -> Fit {
    let css_w = if css_w.is_finite() {
        css_w.max(1.0)
    } else {
        1.0
    };
    let css_h = if css_h.is_finite() {
        css_h.max(1.0)
    } else {
        1.0
    };
    let dpr = if dpr.is_finite() && dpr > 0.0 {
        dpr
    } else {
        1.0
    };
    let cap = SURFACE_CAP_PX as f32;
    let scale = dpr.min(cap / css_w).min(cap / css_h);
    let px = |css: f32| ((css * scale).round() as u32).clamp(1, SURFACE_CAP_PX);
    Fit {
        physical: (px(css_w), px(css_h)),
        scale,
        stretch: dpr / scale,
    }
}

/// The page's viewport this frame: `(css_w, css_h, devicePixelRatio)`.
#[cfg(target_arch = "wasm32")]
pub fn viewport() -> Option<(f32, f32, f32)> {
    let w = web_sys::window()?;
    let num = |v: Result<wasm_bindgen::JsValue, wasm_bindgen::JsValue>| {
        v.ok().and_then(|v| v.as_f64()).map(|f| f as f32)
    };
    Some((
        num(w.inner_width())?,
        num(w.inner_height())?,
        w.device_pixel_ratio() as f32,
    ))
}

/// Keep the backing store fitted to the viewport and the canvas stretched
/// over it. Runs every frame on wasm32; a DOM read of three numbers, and a
/// write only when the fit moved.
///
/// The scale factor override Bevy was given at startup (`devicePixelRatio`
/// then) is deliberately NOT moved here: `bevy_winit` re-derives the
/// physical size from the logical one when the factor changes, in the same
/// pass that applies a new physical size, and the two together land on the
/// wrong number for a frame. A DPR that changes mid-session (a window dragged
/// between displays) therefore costs a UI-scale change and nothing else —
/// the backing store is corrected by this system on the next frame, and the
/// pointer stays consistent because winit and Bevy scale by the same
/// factor whichever it is.
pub fn follow_viewport(
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
    canvas: Option<Res<Canvas>>,
    mut last: Local<Option<Fit>>,
) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (&mut windows, &canvas, &mut last);
    }
    #[cfg(target_arch = "wasm32")]
    {
        let Some((w, h, dpr)) = viewport() else {
            return;
        };
        let want = fit(w, h, dpr);
        if *last == Some(want) {
            return;
        }
        let Ok(mut window) = windows.single_mut() else {
            return;
        };
        let (pw, ph) = want.physical;
        let had = (
            window.resolution.physical_width(),
            window.resolution.physical_height(),
        );
        if had != (pw, ph) {
            window.resolution.set_physical_resolution(pw, ph);
        }
        if let Some(canvas) = canvas.as_deref() {
            stretch_canvas(&canvas.0, want.stretch);
        }
        // Once per change, so a console can say what the surface did when a
        // frame does not look like the window.
        bevy::log::info!(
            "web: viewport {w}x{h} @ {dpr} -> store {pw}x{ph} (was {}x{}), stretch {}",
            had.0,
            had.1,
            want.stretch
        );
        *last = Some(want);
    }
}

/// Set the canvas's CSS transform so the box winit sized at
/// `physical / devicePixelRatio` covers the viewport again.
#[cfg(target_arch = "wasm32")]
pub fn stretch_canvas(selector: &str, stretch: f32) {
    use wasm_bindgen::JsCast;
    let Some(el) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.query_selector(selector).ok().flatten())
        .and_then(|e| e.dyn_into::<web_sys::HtmlElement>().ok())
    else {
        return;
    };
    let style = el.style();
    let _ = style.set_property("transform-origin", "0 0");
    let _ = style.set_property("transform", &format!("scale({stretch})"));
}

/// Hand control back to the page: `window.gatesLeft(status)` if the page
/// installed one, else a reload.
///
/// The page owns what a player sees after leaving — it is the menu on this
/// target — and a reload is the honest fallback because the `Session` is
/// one-shot and a second `play` on the same canvas would be a second `App`
/// over the first's context.
#[cfg(target_arch = "wasm32")]
pub fn hand_back(status: &str) {
    use wasm_bindgen::{JsCast, JsValue};
    let Some(window) = web_sys::window() else {
        return;
    };
    let hook = js_sys::Reflect::get(&window, &JsValue::from_str("gatesLeft"))
        .ok()
        .and_then(|v| v.dyn_into::<js_sys::Function>().ok());
    match hook {
        Some(f) => {
            let _ = f.call1(&window, &JsValue::from_str(status));
        }
        None => {
            let _ = window.location().reload();
        }
    }
}

/// `OnEnter(Screen::Menu)` on wasm32: there is no menu to draw, so the page
/// takes over, told why (`Menu::status` — "left <shard>", "left while the
/// world was loading", the disconnect's reason).
pub fn leave_to_page(menu: Res<super::Menu>) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = &menu;
    }
    #[cfg(target_arch = "wasm32")]
    hand_back(&menu.status);
}

/// `Last` on wasm32: an `AppExit` would freeze the canvas, so it hands back
/// instead. Reads the message rather than consuming it — Bevy's runner reads
/// the same buffer through its own cursor — and fires once.
pub fn exit_to_page(mut exits: MessageReader<AppExit>, mut done: Local<bool>) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (&mut exits, &mut done);
    }
    #[cfg(target_arch = "wasm32")]
    {
        if *done {
            return;
        }
        if exits.read().next().is_some() {
            *done = true;
            hand_back("quit");
        }
    }
}

/// One line every `HEAP_REPORT_S` seconds on wasm32: the wasm heap, the
/// asset counts and the entity count. Seconds and not frames, because a
/// software rasterizer draws well under a frame a second and a count of
/// frames then says nothing for minutes.
///
/// **The number a tab dies on, kept in the console.** A page has one 32-bit
/// heap that never shrinks, and an allocation that fails there is a bare
/// `RuntimeError: unreachable` with no message (`std::alloc::rust_oom` →
/// abort), which the first browser runs read as a winit bug because winit's
/// `RefCell` was what the trap left borrowed. The heap is readable from the
/// page (`gatesWasm.memory.buffer.byteLength`) but not what is IN it; the
/// counts here are the cheapest split of that, and the memory size comes
/// off `memory_size` rather than the page so the line is one source.
pub const HEAP_REPORT_S: f32 = 2.0;

/// The report, `Update` on wasm32 (`render/mod.rs`).
#[allow(clippy::too_many_arguments)]
pub fn heap_report(
    images: Res<Assets<Image>>,
    meshes: Res<Assets<Mesh>>,
    materials: Res<Assets<StandardMaterial>>,
    sources: Res<Assets<bevy::audio::AudioSource>>,
    entities: Query<Entity>,
    time: Res<Time>,
    // The input-side message buffers, whose length is this frame's and the
    // last frame's arrivals: a source that spams them shows here as a count.
    motion: Res<Messages<bevy::input::mouse::MouseMotion>>,
    cursor: Res<Messages<bevy::window::CursorMoved>>,
    raw: Res<Messages<bevy::winit::RawWinitWindowEvent>>,
    mut due: Local<f32>,
) {
    *due += time.delta_secs();
    if *due < HEAP_REPORT_S {
        return;
    }
    *due = 0.0;
    #[cfg(target_arch = "wasm32")]
    let heap_mb = core::arch::wasm32::memory_size(0) as f64 * 65536.0 / 1e6;
    #[cfg(not(target_arch = "wasm32"))]
    let heap_mb = 0.0;
    bevy::log::info!(
        "web: heap {heap_mb:.0} MB · images {} · meshes {} · materials {} · audio {} · entities {} · \
         motion {} · cursor {} · raw {} · dt {:.0} ms",
        images.len(),
        meshes.len(),
        materials.len(),
        sources.len(),
        entities.iter().count(),
        motion.len(),
        cursor.len(),
        raw.len(),
        time.delta_secs() * 1000.0
    );
}
