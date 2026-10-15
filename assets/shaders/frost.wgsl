// Frost on the frame while the cold bites (`frost.rs`): ice that grows in
// from the edges, the corners first and deepest, a milky rime at the rim
// and crystals running ahead of it grain by grain. Procedural, so it is
// sharp at any resolution and costs nothing on disk.
//
// What it costs: a pixel past the farthest the front can reach leaves at
// the first test, but a full frost reaches most of a wide frame, and every
// pixel short of that pays the front's noise (16 hashes), every one the
// crystals could reach two grain searches (9 hashes each), and only then,
// where a grain or the rime has really arrived, the needles and the rime's
// noise. `frost.rs` draws it on the eye's target, so the render scale cuts
// this with the world's own cost.
#import bevy_ui::ui_vertex_output::UiVertexOutput

// Field for field `frost::FrostMaterial::params`: x how far the frost has
// crept in, 0..1; y its brightness (the light on the world); z and w
// unused — one whole vec4 because WebGL2 wants a uniform of at least 16
// bytes.
@group(1) @binding(0) var<uniform> frost: vec4<f32>;

// How far in the rime reaches at full strength, in the rim measure below
// (short-side units along a side, twice that down a corner's diagonal).
const REACH: f32 = 0.13;
// How ragged its front is, as a share of how far it has reached, and the
// size of the front's lobes, per short side.
const RAGGED: f32 = 0.5;
const FRONT_FREQ: f32 = 4.0;
// How far past the rime's front the crystals can run, as a share of its
// reach, and how unevenly: each grain waits for its own share of that.
const AHEAD: f32 = 1.3;
const GRAIN_SPREAD: f32 = 0.5;
const CRYSTAL_ALPHA: f32 = 0.65;
const RIME_ALPHA: f32 = 0.6;
// The crosshair's clear disc and its soft edge, short-side units: no level
// of frost reaches it, whatever the window's shape.
const CLEAR: f32 = 0.15;
const CLEAR_SOFT: f32 = 0.08;
const PI: f32 = 3.14159265;

// Dave Hoskins' hashes without sine: a `sin` of a large argument is
// rounded differently by every GPU, and the crystals should be the same
// crystals in a browser as on the desktop.
fn hash21(p: vec2<f32>) -> f32 {
    var p3 = fract(vec3<f32>(p.x, p.y, p.x) * 0.1031);
    p3 = p3 + dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

fn hash22(p: vec2<f32>) -> vec2<f32> {
    var p3 = fract(vec3<f32>(p.x, p.y, p.x) * vec3<f32>(0.1031, 0.1030, 0.0973));
    p3 = p3 + dot(p3, p3.yzx + 33.33);
    return fract((p3.xx + p3.yz) * p3.zy);
}

fn value_noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = hash21(i);
    let b = hash21(i + vec2<f32>(1.0, 0.0));
    let c = hash21(i + vec2<f32>(0.0, 1.0));
    let e = hash21(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, e, u.x), u.y);
}

// Four octaves, 0..1.
fn fbm(p: vec2<f32>) -> f32 {
    var sum = 0.0;
    var amp = 0.5;
    var q = p;
    for (var k = 0; k < 4; k = k + 1) {
        sum = sum + amp * value_noise(q);
        q = q * 2.03 + vec2<f32>(17.1, 9.7);
        amp = amp * 0.5;
    }
    return sum / 0.9375;
}

// The ice grain a point is in: the nearest of one jittered nucleus a cell.
struct Grain {
    // Distances to the nearest nucleus and to the next one, cell units.
    near: f32,
    next: f32,
    // The nearest nucleus's cell, which is the grain's identity.
    cell: vec2<f32>,
}

fn grain_at(q: vec2<f32>) -> Grain {
    let iq = floor(q);
    let fq = q - iq;
    var nearest = 9.0;
    var second = 9.0;
    var cell = iq;
    for (var j = -1; j <= 1; j = j + 1) {
        for (var i = -1; i <= 1; i = i + 1) {
            let g = vec2<f32>(f32(i), f32(j));
            let r = g + hash22(iq + g) - fq;
            let dd = dot(r, r);
            if (dd < nearest) {
                second = nearest;
                nearest = dd;
                cell = iq + g;
            } else if (dd < second) {
                second = dd;
            }
        }
    }
    var out: Grain;
    out.near = sqrt(nearest);
    out.next = sqrt(second);
    out.cell = cell;
    return out;
}

// One scale of crystals: each grain's needles run one way of its own, the
// edges between grains catch the light, and a grain shows once the front
// has passed far enough over it (`ahead`, 0 at the crystals' reach and 1 at
// the rim), so the frost advances grain by grain rather than as a line.
fn crystals(p: vec2<f32>, cells: f32, stretch: f32, edge_w: f32, facet: f32, ahead: f32) -> f32 {
    let q = p * cells;
    let g = grain_at(q);
    // A grain the front has not reached yet shows nothing: leave before the
    // needles are worked out.
    let wait = hash21(g.cell + vec2<f32>(5.1, 7.7)) * GRAIN_SPREAD;
    let shown = smoothstep(0.0, 0.12, ahead - wait);
    if (shown <= 0.0) {
        return 0.0;
    }
    let th = hash21(g.cell + vec2<f32>(31.7, 11.3)) * PI;
    let c = cos(th);
    let s = sin(th);
    let a = vec2<f32>(q.x * c + q.y * s, q.y * c - q.x * s);
    let n = value_noise(vec2<f32>(a.x * 1.5 + g.cell.x * 7.0, a.y * stretch + g.cell.y * 3.0));
    let needles = smoothstep(0.55, 0.85, n) * 0.85;
    let edges = (1.0 - smoothstep(0.0, edge_w, g.next - g.near)) * facet;
    return shown * max(needles, edges);
}

@fragment
fn fragment(in: UiVertexOutput) -> @location(0) vec4<f32> {
    let level = clamp(frost.x, 0.0, 1.0);
    // Centred, in units of the short side: the frost reaches as far into a
    // phone held upright as into a wide monitor.
    let short_px = max(min(in.size.x, in.size.y), 1.0);
    let p = (in.uv - 0.5) * in.size / short_px;
    let half_extent = 0.5 * in.size / short_px;
    let edge = max(half_extent - abs(p), vec2<f32>(0.0));
    // The two edge distances' harmonic blend: a side's own distance near
    // its middle, half of it down a corner's diagonal, so the corners frost
    // first and deepest.
    let rim = edge.x * edge.y / (edge.x + edge.y + 1e-4);
    let reach = REACH * level;
    if (rim > reach * (AHEAD + RAGGED) + 1e-4) {
        return vec4<f32>(0.0);
    }

    // The front, ragged in broad lobes, and grown self-similar: the same
    // shape at every reach, so a creeping frost looks like one frost growing.
    let ragged = (fbm(p * FRONT_FREQ) - 0.5) * 2.0 * RAGGED * reach;
    let front = (rim + ragged) / max(reach, 1e-4);
    let ahead = clamp(1.0 - front / AHEAD, 0.0, 1.0);
    if (ahead <= 0.0) {
        return vec4<f32>(0.0);
    }
    let cover = clamp(1.0 - front, 0.0, 1.0);

    // Two scales of crystal, the coarse one with the fainter edges so the
    // big grains do not read as cracked glass.
    let coarse = crystals(p, 9.0, 14.0, 0.04, 0.25, ahead) * 0.9;
    let fine = crystals(p, 21.0, 14.0, 0.05, 0.45, ahead) * 0.8;
    let crystal = clamp(max(coarse, fine), 0.0, 1.0);
    // Under them a milky rime, thickest at the very rim; its noise only
    // where the rime has arrived.
    var rime = 0.0;
    if (cover > 0.0) {
        rime = cover * sqrt(cover) * (0.5 + 0.5 * fbm(p * 11.0));
    }

    var alpha = max(rime * RIME_ALPHA, crystal * CRYSTAL_ALPHA) + crystal * rime * 0.25;
    // Faint while it is only starting, fuller as it bites.
    alpha = alpha * (0.6 + 0.4 * level);
    alpha = alpha * smoothstep(CLEAR, CLEAR + CLEAR_SOFT, length(p));

    // Linear colour, as every UI colour reaches the pass: a pale blue rime,
    // the crystals nearly white.
    let ice = mix(vec3<f32>(0.60, 0.74, 0.90), vec3<f32>(0.92, 0.96, 1.0), crystal);
    return vec4<f32>(ice * frost.y, clamp(alpha, 0.0, 0.92));
}
