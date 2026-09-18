//! A bounded visual chart for paint, never a replacement for the sim's road.
//!
//! Only the first live outward shoreline crossing on each bearing is charted.
//! Secondary crossings remain paved but unmarked. Every connecting interval
//! is checked against the authoritative ring; ambiguous intervals stay blank.

use sim_core::terrain::{self, RoadBand};
use std::f32::consts::TAU;

/// Proposed visual defaults, DECISIONS.md road markings v1.
pub const ROAD_CHART_SAMPLES: usize = 8192;
/// Cooperative construction budget; keeps the browser session pump responsive.
pub const ROAD_CHART_BATCH: usize = 256;
pub const ROAD_CHART_ERROR_M: f32 = 0.04;
pub const ROAD_PAINT_WIDTH_M: f32 = 0.12;
pub const ROAD_EDGE_INSET_M: f32 = 0.4;
pub const ROAD_DASH_PERIOD_M: f32 = 6.0;
/// Equal painted and empty lengths. Paint lives inside the sim's radial width.
pub const ROAD_EDGE_OFFSET_M: f32 = terrain::ROAD_HALF_W - ROAD_EDGE_INSET_M;
/// At least a whole terrain triangle stays unpainted at each disconnected end.
pub const ROAD_END_CLEAR_M: f32 = terrain::ROAD_HALF_W * 2.0;

#[derive(Clone, Copy, Default, Debug)]
struct Node {
    radius: f32,
    along: f32,
    length: f32,
    run_length: f32,
    period: f32,
    live: bool,
    paintable: bool,
}

/// One immutable cache per rendered world, shared by terrain workers.
#[derive(Debug)]
pub struct RoadChart {
    nodes: Vec<Node>,
}

/// Interpolated by the terrain mesh independently from road coverage.
/// The phase vector avoids wrapping a scalar dash coordinate across a triangle.
#[derive(Clone, Copy, Debug)]
pub struct PaintCoord {
    pub across: f32,
    pub phase: [f32; 2],
}

fn direction(i: usize) -> (f32, f32) {
    let a = (i % ROAD_CHART_SAMPLES) as f32 * TAU / ROAD_CHART_SAMPLES as f32;
    let (z, x) = a.sin_cos();
    (x, z)
}

fn point(ux: f32, uz: f32, r: f32) -> (f32, f32) {
    let c = terrain::ISLAND_SIZE * 0.5;
    (c + ux * r, c + uz * r)
}

/// The road's radius on one chart bearing, read off the solved ring.
///
/// ⚠ **This used to re-derive it**, marching and bisecting the shoreline the
/// way `haven` did and taking `shore - ROAD_INLAND_M`, because that WAS the
/// road's centre line. Since ring path v0 it is not, and a chart built the
/// old way paints stripes down a road the sim does not have —
/// `tests/road_markings.rs` caught exactly that, which is the whole reason
/// its validity check asks the authoritative ring rather than trusting this.
///
/// `ROAD_CHART_SAMPLES` is a multiple of `RING_BEARINGS`, so a chart bearing
/// falls between two nodes and the radius is the polyline's own linear
/// interpolation — the same shape `ring_band` measures against, and no
/// `height` tap at all where this was a bisection per bearing.
fn radius_at(ring: &terrain::RingPath, i: usize) -> f32 {
    // ⚠ **The two indices run in OPPOSITE directions and the first draft of
    // this assumed they did not** — which produced a chart whose every
    // interval failed its own validity check, so the road came out with no
    // paint on it at all. `direction(i)` here is `(cos a, sin a)` in `(x, z)`
    // and turns +X toward +Z; the yaw LUT's index 0 is +Z and turns toward
    // +X. Matching them gives `theta = pi/2 - a`, so the ring index is
    // `64 - 256 * i / SAMPLES`, decreasing.
    let n = terrain::RING_BEARINGS as f32;
    let f = (64.0 - n * i as f32 / ROAD_CHART_SAMPLES as f32).rem_euclid(n);
    let a = f as usize % terrain::RING_BEARINGS;
    let b = (a + 1) % terrain::RING_BEARINGS;
    let t = f - f.floor();
    ring.r[a] + (ring.r[b] - ring.r[a]) * t
}

/// Built in bounded batches; native workers and browser cooperative tasks use
/// the same arithmetic, with no global cache or per-chunk reconstruction.
pub struct RoadChartBuilder {
    /// The solved coast ring. Resolved once here rather than threaded from
    /// the caller: it is a pure function of the seed, so a second copy cannot
    /// disagree with the sim's, and the chart is built once per world.
    ring: terrain::RingPath,
    nodes: Vec<Node>,
    cursor: usize,
    mesh_step: f32,
}

impl RoadChartBuilder {
    pub fn new(seed: u64, mesh_step: f32) -> Self {
        Self {
            ring: terrain::solve_ring(seed),
            nodes: vec![Node::default(); ROAD_CHART_SAMPLES],
            cursor: 0,
            mesh_step,
        }
    }

    pub fn advance(&mut self) -> Option<RoadChart> {
        let end = (self.cursor + ROAD_CHART_BATCH).min(ROAD_CHART_SAMPLES * 2);
        while self.cursor < end {
            let i = self.cursor;
            if i < ROAD_CHART_SAMPLES {
                self.nodes[i].radius = radius_at(&self.ring, i);
            } else {
                self.sample_edge(i - ROAD_CHART_SAMPLES);
            }
            self.cursor += 1;
        }
        if self.cursor < ROAD_CHART_SAMPLES * 2 {
            return None;
        }
        assign_runs(&mut self.nodes);
        // A closed loop fits an integer dash count, so its actual period can
        // be slightly shorter than the nominal one used during construction.
        // Recheck that period before accepting the chart; a refusal opens the
        // loop and ordinary nominal-period runs are then assigned once more.
        let mut opened = false;
        for i in 0..self.nodes.len() {
            let n = self.nodes[i];
            let next = self.nodes[(i + 1) % self.nodes.len()];
            if n.live
                && n.length >= phase_arc_limit(n.radius, next.radius, self.mesh_step, n.period)
            {
                self.nodes[i].live = false;
                opened = true;
            }
        }
        if opened {
            assign_runs(&mut self.nodes);
        }
        // Arc distance alone is insufficient at steep radial turns. Also
        // leave a complete maximum-resolution triangle's angular support
        // empty on BOTH sides of every invalid interval.
        let guard = ((self.mesh_step * 2.0_f32.sqrt() / terrain::ROAD_R_MIN)
            * ROAD_CHART_SAMPLES as f32
            / TAU)
            .ceil() as usize
            + 1;
        for i in 0..self.nodes.len() {
            self.nodes[i].paintable = (0..=guard * 2).all(|j| {
                self.nodes[(i + ROAD_CHART_SAMPLES + j - guard) % ROAD_CHART_SAMPLES].live
            });
        }
        Some(RoadChart {
            nodes: std::mem::take(&mut self.nodes),
        })
    }

    fn sample_edge(&mut self, i: usize) {
        let nodes = &mut self.nodes;
        let ring = &self.ring;
        let next = (i + 1) % nodes.len();
        let (a, b) = (nodes[i].radius, nodes[next].radius);
        if a == 0.0 || b == 0.0 {
            return;
        }
        let (ux, uz) = direction(i);
        let (vx, vz) = direction(next);
        let length = ((ux * a - vx * b).powi(2) + (uz * a - vz * b).powi(2)).sqrt();
        // A fine chart can still alias on the coarser terrain triangles.
        // Bound the phase rate so any mesh diagonal in this support band
        // advances by less than HALF a dash cycle. The invalid-neighbour
        // dilation below extends this refusal across the entire triangle.
        let max_phase_arc = phase_arc_limit(a, b, self.mesh_step, ROAD_DASH_PERIOD_M);
        if length >= max_phase_arc || length > terrain::ROAD_HALF_W * 2.0 {
            return;
        }
        // The chart interpolates RADIUS, as does the sim's width test.
        // Check the actual crossing to a fraction of the stripe width,
        // plus both stripe outer edges, including the interval interior.
        let mut valid = true;
        for k in 0..=4 {
            let t = k as f32 / 4.0;
            let a = (i as f32 + t) * TAU / ROAD_CHART_SAMPLES as f32;
            let (uz, ux) = a.sin_cos();
            let r = nodes[i].radius + (nodes[next].radius - nodes[i].radius) * t;
            // ⚠ **A shoreline bracket used to sit here and it is gone.** It
            // asserted the crossing lay within `ROAD_CHART_ERROR_M` of
            // `r + ROAD_INLAND_M` — i.e. that this interval really was 40 m
            // inland — which was the road's definition and is not any more.
            // Against the solved ring it fails on every interval, and the
            // symptom is a road with no paint on it at all rather than paint
            // in the wrong place. What replaces it is the check below, which
            // was always the stronger of the two: ask the authoritative ring
            // whether the paint's own three radii are carriageway.
            for offset in [-1.0, 0.0, 1.0] {
                let q = r + offset * (ROAD_EDGE_OFFSET_M + ROAD_PAINT_WIDTH_M * 0.5);
                let (x, z) = point(ux, uz, q);
                if terrain::ring_band(ring, x, z) != RoadBand::Carriageway {
                    valid = false;
                }
            }
        }
        nodes[i].live = valid;
        nodes[i].length = length;
    }
}

impl RoadChart {
    pub fn build(seed: u64, mesh_step: f32) -> Self {
        let mut builder = RoadChartBuilder::new(seed, mesh_step);
        loop {
            if let Some(chart) = builder.advance() {
                return chart;
            }
        }
    }

    /// O(1), no terrain calls: radial bucket then one interpolation. Return
    /// coordinates on the shoulder too, so no sentinel enters a paint edge.
    /// Invalid intervals and a full triangle's support at each end stay blank.
    pub fn at(&self, x: f32, z: f32) -> Option<PaintCoord> {
        let c = terrain::ISLAND_SIZE * 0.5;
        let (dx, dz) = (x - c, z - c);
        let r = (dx * dx + dz * dz).sqrt();
        if !(terrain::ROAD_R_MIN..=terrain::ROAD_R_MAX).contains(&r) {
            return None;
        }
        let bucket = dz.atan2(dx).rem_euclid(TAU) * ROAD_CHART_SAMPLES as f32 / TAU;
        let i = (bucket as usize).min(ROAD_CHART_SAMPLES - 1);
        let t = bucket - i as f32;
        let n = self.nodes[i];
        if !n.live || !n.paintable {
            return None;
        }
        let along = n.along + t * n.length;
        if n.run_length > 0.0
            && (along < ROAD_END_CLEAR_M || n.run_length - along < ROAD_END_CLEAR_M)
        {
            return None;
        }
        let next = self.nodes[(i + 1) % self.nodes.len()];
        let across = r - (n.radius + (next.radius - n.radius) * t);
        if across.abs() > terrain::ROAD_SHOULDER_HALF_W + terrain::ROAD_HALF_W {
            return None;
        }
        let (sin, cos) = (along * TAU / n.period).sin_cos();
        Some(PaintCoord {
            across,
            phase: [sin, cos],
        })
    }

    pub fn live_intervals(&self) -> usize {
        self.nodes.iter().filter(|n| n.live).count()
    }

    pub fn storage_bytes(&self) -> usize {
        self.nodes.len() * std::mem::size_of::<Node>()
    }
}

fn phase_arc_limit(a: f32, b: f32, mesh_step: f32, period: f32) -> f32 {
    let diagonal = mesh_step * 2.0_f32.sqrt();
    let support = terrain::ROAD_SHOULDER_HALF_W + terrain::ROAD_HALF_W;
    let min_radius = (a.min(b) - support - diagonal).max(terrain::ROAD_R_MIN);
    let triangle_angle = 2.0 * (diagonal / (2.0 * min_radius)).asin();
    period * 0.5 * (TAU / ROAD_CHART_SAMPLES as f32) / triangle_angle
}

fn assign_runs(nodes: &mut [Node]) {
    for n in nodes.iter_mut() {
        n.along = 0.0;
        n.run_length = 0.0;
        n.period = 0.0;
    }
    let count = nodes.len();
    let Some(gap) = nodes.iter().position(|n| !n.live) else {
        let total: f32 = nodes.iter().map(|n| n.length).sum();
        let period = total / (total / ROAD_DASH_PERIOD_M).round().max(1.0);
        let mut along = 0.0;
        for n in nodes {
            n.along = along;
            n.period = period;
            along += n.length;
        }
        return;
    };
    let start = (gap + 1) % count;
    let mut j = 0;
    while j < count {
        if !nodes[(start + j) % count].live {
            j += 1;
            continue;
        }
        let begin = j;
        let mut length = 0.0;
        while j < count && nodes[(start + j) % count].live {
            let n = &mut nodes[(start + j) % count];
            n.along = length;
            n.period = ROAD_DASH_PERIOD_M;
            length += n.length;
            j += 1;
        }
        for k in begin..j {
            nodes[(start + k) % count].run_length = length;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ring_nodes() -> Vec<Node> {
        vec![
            Node {
                radius: 800.0,
                length: 1.0,
                live: true,
                paintable: true,
                ..Node::default()
            };
            ROAD_CHART_SAMPLES
        ]
    }

    #[test]
    fn complete_loop_matches_its_dash_phase_at_the_seam() {
        let mut nodes = ring_nodes();
        assign_runs(&mut nodes);
        let last = nodes.last().unwrap();
        let end = (last.along + last.length) * TAU / last.period;
        assert!(end.sin().abs() < 0.002);
        assert!(end.cos() > 0.999);
        let chart = RoadChart { nodes };
        let a = chart.at(1824.0, 1024.001).unwrap();
        let b = chart.at(1824.0, 1023.999).unwrap();
        assert!((a.phase[0] - b.phase[0]).abs() < 0.01);
        assert!((a.phase[1] - b.phase[1]).abs() < 0.01);
    }

    #[test]
    fn disconnected_run_can_cross_zero_without_resetting_phase() {
        let mut nodes = ring_nodes();
        nodes[ROAD_CHART_SAMPLES / 2].live = false;
        assign_runs(&mut nodes);
        assert_eq!(nodes[0].along, nodes.last().unwrap().along + 1.0);
        assert_eq!(nodes[0].run_length, (ROAD_CHART_SAMPLES - 1) as f32);
        let chart = RoadChart { nodes };
        assert!(chart.at(224.0, 1024.0).is_none());
        assert!(chart.at(1824.0, 1024.0).is_some());
    }

    #[test]
    fn steep_triangle_cannot_skip_more_than_half_a_dash_cycle() {
        let chart = RoadChart::build(20260731, 1.0);
        // Before the phase-rate bound this actual 1 m mesh diagonal advanced
        // 4.238 m along a 6 m dash cycle, reversing interpolated dash/gap.
        assert!(chart.at(346.0, 562.0).is_none() || chart.at(347.0, 561.0).is_none());
        // The conservative gap does not remove the ordinary seam fixture.
        assert!(chart.at(1859.276, 1024.0).is_some());
    }

    #[test]
    fn chart_centres_and_edges_remain_in_authoritative_ring() {
        for seed in [20260731, 42, 0xDEAD_BEEF] {
            let chart = RoadChart::build(seed, 1.0);
            // The AUTHORITATIVE ring is the solved path — the raw predicate
            // was it until ring path v0, and the two disagree by design.
            let ring = terrain::solve_ring(seed);
            let mut checked = 0;
            for i in 0..ROAD_CHART_SAMPLES {
                let n = chart.nodes[i];
                if !n.live || !n.paintable {
                    continue;
                }
                let a = (i as f32 + 0.375) * TAU / ROAD_CHART_SAMPLES as f32;
                let (uz, ux) = a.sin_cos();
                let r = n.radius
                    + (chart.nodes[(i + 1) % ROAD_CHART_SAMPLES].radius - n.radius) * 0.375;
                let (x, z) = point(ux, uz, r);
                if chart.at(x, z).is_none() {
                    continue;
                }
                for offset in [
                    -ROAD_EDGE_OFFSET_M - ROAD_PAINT_WIDTH_M * 0.5,
                    0.0,
                    ROAD_EDGE_OFFSET_M + ROAD_PAINT_WIDTH_M * 0.5,
                ] {
                    let (x, z) = point(ux, uz, r + offset);
                    assert_eq!(
                        terrain::ring_band(&ring, x, z),
                        RoadBand::Carriageway,
                        "seed={seed} at {x},{z}"
                    );
                    checked += 1;
                }
            }
            assert!(
                checked > ROAD_CHART_SAMPLES * 2,
                "fixture lost most of its chart"
            );
        }
    }
}
