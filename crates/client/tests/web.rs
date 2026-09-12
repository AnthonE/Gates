//! The browser's surface and its exit — `render/web.rs` (`NOW.md` §0web
//! items 4 and 5).
//!
//! The DOM half runs nowhere a test can reach; what is gated is the
//! arithmetic that decides the backing store, and the two registrations in
//! `render/mod.rs` that make leaving the world hand control to the page.
//! The mutants run: dropping the per-axis clamp (the retina 1080p case goes
//! red), scaling the axes separately (the aspect case goes red), and
//! deleting either registration (the source scan goes red).
#![cfg(feature = "render")]

use client::render::web::{fit, SURFACE_CAP_PX};

/// A display under the cap is untouched: the backing store is the viewport
/// in device pixels, the scale is the ratio, and the canvas is not stretched.
#[test]
fn a_viewport_inside_the_cap_is_drawn_at_its_own_device_pixels() {
    let f = fit(1280.0, 720.0, 1.0);
    assert_eq!(f.physical, (1280, 720));
    assert_eq!(f.scale, 1.0);
    assert_eq!(f.stretch, 1.0);

    // 1024 × 768 at devicePixelRatio 2 is 2048 wide — exactly the cap.
    let f = fit(1024.0, 768.0, 2.0);
    assert_eq!(f.physical, (2048, 1536));
    assert_eq!(f.scale, 2.0);
    assert_eq!(f.stretch, 1.0);
}

/// The case that drew nothing: 1080p at devicePixelRatio 2 asks for 3840 and
/// is refused outright by WebGL2. The store is scaled down uniformly to the
/// cap on the long side and the canvas stretched back by the same factor.
#[test]
fn a_retina_1080p_display_lands_on_the_cap_and_is_stretched_back() {
    let f = fit(1920.0, 1080.0, 2.0);
    assert_eq!(f.physical.0, SURFACE_CAP_PX);
    assert_eq!(f.physical, (2048, 1152));
    assert!((f.scale - 2048.0 / 1920.0).abs() < 1e-6);
    assert!(
        (f.stretch - 2.0 / f.scale).abs() < 1e-6,
        "stretch is not dpr / scale"
    );
    assert!((f.stretch - 1.875).abs() < 1e-6);
}

/// One scale for both axes: the aspect is the viewport's, whichever side
/// binds — including a portrait phone, where the height is the long side.
#[test]
fn the_aspect_is_the_viewports_whichever_axis_binds() {
    for (w, h, dpr) in [
        (1920.0, 1080.0, 2.0),
        (390.0, 844.0, 3.0),
        (2560.0, 1440.0, 1.0),
        (3440.0, 1440.0, 1.0),
        (800.0, 600.0, 1.5),
    ] {
        let f = fit(w, h, dpr);
        let (pw, ph) = f.physical;
        assert!(
            pw <= SURFACE_CAP_PX && ph <= SURFACE_CAP_PX,
            "{w}x{h}@{dpr}: {pw}x{ph}"
        );
        let want = w / h;
        let got = pw as f32 / ph as f32;
        assert!(
            (got - want).abs() / want < 0.01,
            "{w}x{h}@{dpr}: aspect {got} against the viewport's {want}"
        );
        // The store times the stretch is the viewport in device pixels: the
        // canvas covers the page exactly.
        assert!(
            ((pw as f32 * f.stretch) - w * dpr).abs() < 2.0 * f.stretch,
            "{w}x{h}@{dpr}: stretched width {} against {}",
            pw as f32 * f.stretch,
            w * dpr
        );
    }
    let f = fit(390.0, 844.0, 3.0);
    assert_eq!(
        f.physical.1, SURFACE_CAP_PX,
        "the portrait long side is the height"
    );
}

/// Never over the cap, for any viewport and ratio a page can report, and
/// never a zero-sized store — the two refusals wgpu has for a surface.
#[test]
fn no_viewport_and_no_ratio_ever_asks_for_more_than_the_cap() {
    for w in [
        1.0f32, 320.0, 1280.0, 1920.0, 2048.0, 2049.0, 3840.0, 7680.0,
    ] {
        for h in [1.0f32, 200.0, 720.0, 1080.0, 2048.0, 2160.0, 4320.0] {
            for dpr in [0.5f32, 1.0, 1.25, 1.5, 2.0, 3.0, 4.0] {
                let f = fit(w, h, dpr);
                let (pw, ph) = f.physical;
                assert!(
                    (1..=SURFACE_CAP_PX).contains(&pw),
                    "{w}x{h}@{dpr}: width {pw}"
                );
                assert!(
                    (1..=SURFACE_CAP_PX).contains(&ph),
                    "{w}x{h}@{dpr}: height {ph}"
                );
                assert!(
                    f.scale > 0.0 && f.scale <= dpr,
                    "{w}x{h}@{dpr}: scale {}",
                    f.scale
                );
                assert!(
                    f.stretch >= 1.0 - 1e-6,
                    "{w}x{h}@{dpr}: stretch {} shrinks",
                    f.stretch
                );
            }
        }
    }
    // A page can report nonsense before layout; nothing here may panic on it.
    let f = fit(0.0, 0.0, 0.0);
    assert_eq!(f.physical, (1, 1));
    let f = fit(f32::NAN, f32::INFINITY, f32::NAN);
    assert!(f.physical.0 >= 1 && f.physical.1 >= 1);
}

/// The page is the menu: entering `Screen::Menu` and an `AppExit` both hand
/// control back to the page on wasm32, registered in `render/mod.rs` where
/// the desktop front end is cut. Read off the source, because the systems'
/// bodies are `cfg`'d and a native run cannot reach them.
#[test]
fn leaving_the_world_hands_control_to_the_page_on_wasm() {
    let src =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/render/mod.rs")).unwrap();
    let code: String = src
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    let menu = code
        .find("web::leave_to_page")
        .expect("`web::leave_to_page` is not registered");
    let before = &code[menu.saturating_sub(200)..menu];
    assert!(
        before.contains("OnEnter(Screen::Menu)"),
        "`web::leave_to_page` is registered, but not on `OnEnter(Screen::Menu)`"
    );
    let exit = code
        .find("web::exit_to_page")
        .expect("`web::exit_to_page` is not registered");
    let before = &code[exit.saturating_sub(200)..exit];
    assert!(
        before.contains("Last"),
        "`web::exit_to_page` is registered, but not in `Last`"
    );
    assert!(
        code.contains("web::follow_viewport"),
        "`web::follow_viewport` is not registered — the surface would not follow the window"
    );
    // The web entry point sizes the window through `fit` and names the
    // canvas for the stretch; a page that pinned 1280 × 720 again would draw
    // a retina canvas at half size.
    let play = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../client-web/src/lib.rs"
    ))
    .unwrap();
    assert!(
        play.contains("web::fit("),
        "`Gates::play` does not size the window through `web::fit`"
    );
    assert!(
        play.contains("web::Canvas("),
        "`Gates::play` does not name the canvas for the stretch"
    );
    assert!(
        !play.contains("WindowResolution::new(1280, 720)"),
        "`Gates::play` pins the canvas at 1280 x 720 again"
    );
}
