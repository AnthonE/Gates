//! Gate: every `assets/shaders/*.wgsl` composes and validates (NOW §0gp).
//!
//! Before this, nothing in CI compiled a shader. `tests/ground_splat.rs`,
//! `tests/ground_tiling.rs` and `tests/water.rs` read the files as text, so a
//! missing brace or a `vec3` handed to an `f32` was green in CI and a dead
//! pipeline at boot.
//!
//! This is the game's own path, short of the GPU: Bevy's `ShaderCache`, with
//! naga_oil composing each file against the real `bevy_pbr` / `bevy_ui`
//! libraries and naga validating the result. The libraries are Bevy's embedded
//! ones, loaded by a headless `DefaultPlugins` with no render backend: every
//! plugin's `load_shader_library!` still runs and no device is opened. The
//! cache's module loader is a no-op, which is where wgpu would take over.
//!
//! Not covered: bind-group layouts against the Rust side (wgpu checks those at
//! pipeline creation), the GLSL a WebGL2 build translates to, and `#ifdef`
//! branches that no def set below turns on. The sets are the two ends of the
//! quality tiers for the main pass and the depth, normal and shadow prepasses
//! for `*_prepass.wgsl`, each on a plain mesh and on a full one (`def_sets`).
//!
//! Proven red, each naming the file and line: a `vec3` assigned to an `f32` in
//! `ground_splat.wgsl`; a stray `fn broken( {` appended to it; a `dpdx` in
//! `ground_vertex.wgsl`'s vertex stage, which parses and only the validator
//! refuses; a misspelt `#import` in the same file.

#![cfg(all(feature = "render", not(target_arch = "wasm32")))]

use std::time::{Duration, Instant};

use bevy::pbr::{
    MATERIAL_BIND_GROUP_INDEX, TONEMAPPING_LUT_SAMPLER_BINDING_INDEX,
    TONEMAPPING_LUT_TEXTURE_BINDING_INDEX,
};
use bevy::prelude::*;
use bevy::render::render_resource::{DownlevelFlags, WgpuFeatures};
use bevy::render::settings::WgpuSettings;
use bevy::render::RenderPlugin;
use bevy::shader::{
    PipelineCacheError, ShaderCache, ShaderCacheSource, ShaderDefVal, ShaderImport, ValidateShader,
};
use bevy::window::ExitCondition;

use client::render::render_scale::OpaqueFrame;

const SHADERS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/shaders");

/// A headless app whose only job is to load Bevy's shader libraries.
/// `backends: None` creates no render sub-app, so nothing touches a GPU, but
/// each plugin's `build`/`finish` still registers its libraries.
fn bevy_libraries() -> App {
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .build()
            .disable::<bevy::winit::WinitPlugin>()
            .disable::<bevy::audio::AudioPlugin>()
            .disable::<bevy::gilrs::GilrsPlugin>()
            .disable::<bevy::log::LogPlugin>()
            .disable::<bevy::app::TerminalCtrlCHandlerPlugin>()
            .set(WindowPlugin {
                primary_window: None,
                exit_condition: ExitCondition::DontExit,
                close_when_requested: false,
                ..default()
            })
            .set(RenderPlugin {
                render_creation: WgpuSettings {
                    backends: None,
                    ..default()
                }
                .into(),
                ..default()
            }),
    );
    // `render_scale.wgsl` imports `bevy_ui::ui_vertex_output`, which only a
    // `UiMaterialPlugin` loads; the game adds this one in `render/mod.rs`.
    app.add_plugins(UiMaterialPlugin::<OpaqueFrame>::default());
    app.finish();
    app.cleanup();
    app
}

/// Every file in `assets/shaders`, added under the asset path the game loads
/// it by (`shaders/<name>`), which is also what a quoted `#import` resolves.
/// The handles are strong: drop one and the next frame frees the shader.
fn ours(app: &mut App) -> Vec<(String, Handle<Shader>)> {
    let mut names: Vec<String> = std::fs::read_dir(SHADERS)
        .expect("assets/shaders")
        .map(|e| {
            e.expect("dir entry")
                .file_name()
                .into_string()
                .expect("utf-8 name")
        })
        .filter(|n| n.ends_with(".wgsl"))
        .collect();
    names.sort();
    let mut shaders = app.world_mut().resource_mut::<Assets<Shader>>();
    names
        .into_iter()
        .map(|name| {
            let src = std::fs::read_to_string(format!("{SHADERS}/{name}")).expect("shader");
            let handle = shaders.add(Shader::from_wgsl(src, format!("shaders/{name}")));
            (name, handle)
        })
        .collect()
}

/// The `#import`s these files reach, followed through Bevy's libraries, that
/// nothing has loaded. Every branch's imports count, `#ifdef`'d or not, which
/// is everything a compose below can ask for.
fn missing(app: &App, files: &[(String, Handle<Shader>)]) -> Vec<String> {
    let shaders = app.world().resource::<Assets<Shader>>();
    let by_path = |import: &ShaderImport| {
        shaders
            .iter()
            .find(|(_, s)| s.import_path() == import)
            .map(|(_, s)| s)
    };
    let mut todo: Vec<&Shader> = files.iter().filter_map(|(_, h)| shaders.get(h)).collect();
    let mut seen: Vec<&ShaderImport> = Vec::new();
    let mut missing = Vec::new();
    while let Some(shader) = todo.pop() {
        for import in shader.imports() {
            if seen.contains(&import) {
                continue;
            }
            seen.push(import);
            match by_path(import) {
                Some(s) => todo.push(s),
                None => missing.push(import.module_name().into_owned()),
            }
        }
    }
    missing
}

/// Embedded libraries load on the IO pool, so pump frames until every import
/// our files reach is in `Assets<Shader>`. An import nothing provides (a typo,
/// or a library a Bevy upgrade moved) never arrives: after 5 s without a new
/// shader the wait gives up and the compose reports that file.
fn settle(app: &mut App, ours: &[(String, Handle<Shader>)]) {
    let start = Instant::now();
    let (mut count, mut changed) = (0, start);
    loop {
        app.update();
        if missing(app, ours).is_empty() {
            return;
        }
        let n = app.world().resource::<Assets<Shader>>().len();
        if n != count {
            (count, changed) = (n, Instant::now());
        }
        if changed.elapsed() > Duration::from_secs(5) {
            return;
        }
        assert!(
            start.elapsed() < Duration::from_secs(60),
            "Bevy's shader libraries were still arriving after 60 s"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn defs(names: &[&str]) -> Vec<ShaderDefVal> {
    names.iter().map(|&n| n.into()).collect()
}

/// The files the terrain mesh draws with. That mesh always carries a second
/// UV, tangents and colours (`terrain_mesh.rs`), and both read them without
/// an `#ifdef`, so they are composed with that layout only.
const TERRAIN: [&str; 2] = ["ground_splat.wgsl", "ground_vertex.wgsl"];

/// What every pipeline gets: the material group, the device's storage-buffer
/// count (`PipelineCache::new`'s global def, at wgpu's default limit), and
/// the mesh attributes `specialize` reads off the vertex layout. A plain mesh
/// is position, normal and UV; a full one adds UV B, tangents and colours.
fn base(full_mesh: bool) -> Vec<ShaderDefVal> {
    let mut d = vec![
        ShaderDefVal::UInt(
            "MATERIAL_BIND_GROUP".into(),
            MATERIAL_BIND_GROUP_INDEX as u32,
        ),
        ShaderDefVal::UInt("AVAILABLE_STORAGE_BUFFER_BINDINGS".into(), 8),
    ];
    d.extend(defs(&[
        "VERTEX_OUTPUT_INSTANCE_INDEX",
        "VERTEX_POSITIONS",
        "VERTEX_UVS",
        "VERTEX_UVS_A",
    ]));
    if full_mesh {
        d.extend(defs(&["VERTEX_UVS_B", "VERTEX_COLORS"]));
    }
    d
}

/// The main pass, as `MeshPipeline::specialize` writes it, for the two ends
/// of `render/quality.rs`: low (no prepass, tonemapping in the shader) and
/// high (HDR, prepasses, SSAO, TAA, the atmosphere, dithered fades).
fn main_pass(high: bool, full_mesh: bool) -> Vec<ShaderDefVal> {
    let mut d = base(full_mesh);
    d.extend(defs(&[
        "MESH_PIPELINE",
        "VERTEX_NORMALS",
        "VIEW_PROJECTION_PERSPECTIVE",
        "SHADOW_FILTER_METHOD_HARDWARE_2X2",
        "IRRADIANCE_VOLUMES_ARE_USABLE",
    ]));
    d.push(ShaderDefVal::Int(
        "SCREEN_SPACE_SPECULAR_TRANSMISSION_BLUR_TAPS".into(),
        4,
    ));
    if full_mesh {
        d.extend(defs(&["VERTEX_TANGENTS"]));
    }
    if high {
        d.extend(defs(&[
            "DEPTH_PREPASS",
            "NORMAL_PREPASS",
            "MOTION_VECTOR_PREPASS",
            "SCREEN_SPACE_AMBIENT_OCCLUSION",
            "TEMPORAL_JITTER",
            "VISIBILITY_RANGE_DITHER",
            "ATMOSPHERE",
            "MAY_DISCARD",
        ]));
    } else {
        d.extend(defs(&[
            "TONEMAP_IN_SHADER",
            "TONEMAP_METHOD_TONY_MC_MAPFACE",
            "DEBAND_DITHER",
        ]));
        d.push(ShaderDefVal::UInt(
            "TONEMAPPING_LUT_TEXTURE_BINDING_INDEX".into(),
            TONEMAPPING_LUT_TEXTURE_BINDING_INDEX,
        ));
        d.push(ShaderDefVal::UInt(
            "TONEMAPPING_LUT_SAMPLER_BINDING_INDEX".into(),
            TONEMAPPING_LUT_SAMPLER_BINDING_INDEX,
        ));
    }
    d
}

/// The prepass, as `PrepassPipeline::specialize` writes it: alpha-masked
/// depth only (no fragment outputs), the high tier's normals and motion
/// vectors, and a directional shadow on a GPU that emulates unclipped depth.
fn prepass(kind: &str, full_mesh: bool) -> Vec<ShaderDefVal> {
    let mut d = base(full_mesh);
    d.extend(defs(&["PREPASS_PIPELINE", "DEPTH_PREPASS", "MAY_DISCARD"]));
    match kind {
        "depth" => {}
        "normals" => {
            d.extend(defs(&[
                "NORMAL_PREPASS",
                "NORMAL_PREPASS_OR_DEFERRED_PREPASS",
                "VERTEX_NORMALS",
                "MOTION_VECTOR_PREPASS",
                "MOTION_VECTOR_PREPASS_OR_DEFERRED_PREPASS",
                "VISIBILITY_RANGE_DITHER",
                "PREPASS_FRAGMENT",
            ]));
            if full_mesh {
                d.extend(defs(&["VERTEX_TANGENTS"]));
            }
        }
        "shadow" => d.extend(defs(&[
            "UNCLIPPED_DEPTH_ORTHO_EMULATION",
            "PREPASS_FRAGMENT",
        ])),
        _ => unreachable!(),
    }
    d
}

/// Each pass the file can be specialized for, on each mesh layout it can be
/// drawn on.
fn def_sets(name: &str) -> Vec<(String, Vec<ShaderDefVal>)> {
    let meshes: &[bool] = if TERRAIN.contains(&name) {
        &[true]
    } else {
        &[false, true]
    };
    let mut sets = Vec::new();
    for &full in meshes {
        let mesh = if full { "full mesh" } else { "plain mesh" };
        if name.ends_with("_prepass.wgsl") {
            for kind in ["depth", "normals", "shadow"] {
                sets.push((format!("{kind} prepass, {mesh}"), prepass(kind, full)));
            }
        } else {
            sets.push((format!("low tier, {mesh}"), main_pass(false, full)));
            sets.push((format!("high tier, {mesh}"), main_pass(true, full)));
        }
    }
    sets
}

/// Where wgpu would build the module from the composed, validated naga IR.
#[allow(clippy::result_large_err)] // the signature is `ShaderCache::new`'s
fn no_device(_: &(), _: ShaderCacheSource, _: &ValidateShader) -> Result<(), PipelineCacheError> {
    Ok(())
}

#[test]
fn every_shader_composes_and_validates() {
    let mut app = bevy_libraries();
    let ours = ours(&mut app);
    assert!(
        ours.iter().any(|(n, _)| n == "ground_splat.wgsl") && ours.len() >= 10,
        "found {} shaders in {SHADERS}; the listing broke rather than the tree \
         shrinking, which is a gate that passes by composing nothing",
        ours.len()
    );
    settle(&mut app, &ours);

    let mut cache = ShaderCache::new(WgpuFeatures::empty(), DownlevelFlags::all(), no_device);
    // A release build hands the cache a non-validating composer; this gate is
    // the validation, whatever profile it runs under.
    cache.composer.validate = true;
    for (id, shader) in app.world().resource::<Assets<Shader>>().iter() {
        cache.set_shader(id, shader.clone());
    }

    let mut failures = Vec::new();
    let mut composed = 0;
    for (name, handle) in &ours {
        for (set, defs) in def_sets(name) {
            match cache.get(&(), 0, handle.id(), &defs) {
                Ok(_) => composed += 1,
                Err(PipelineCacheError::ProcessShaderError(e)) => failures.push(format!(
                    "{name} ({set}):\n{}",
                    e.emit_to_string(&cache.composer)
                )),
                Err(PipelineCacheError::ShaderImportNotYetAvailable) => failures.push(format!(
                    "{name} ({set}): imports that nothing loads: {:?}",
                    missing(&app, &[(name.clone(), handle.clone())])
                )),
                Err(e) => failures.push(format!("{name} ({set}): {e}")),
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} shader compositions failed; the game would refuse each of \
         these pipelines at boot:\n\n{}",
        failures.len(),
        failures.len() + composed,
        failures.join("\n\n")
    );
}
