//! Offline coastal-routing experiment, not production world generation.
//!
//! `cargo run --release -p sim-core --example road_route -- [seed] [audit_m] [--geometry]`
//!
//! The graph is bounded by the existing island grid and ring bracket. BFS
//! visits each node once per anchor pair, checks full-width traversal edges, and
//! refuses a seed if any exact site/junction anchor cannot be connected.
//! No terrain, site, scatter, economy, or replay behavior changes. Host heap
//! allocation here is NOT evidence of a production init/tick budget.
#![allow(clippy::disallowed_macros)]

use sim_core::terrain::{self, Haven, Lattice, RoadBand};
use std::collections::VecDeque;

const GRID: f32 = terrain::CELL_SIZE;
const N: usize = (terrain::ISLAND_SIZE / GRID) as usize + 1;
const ANCHORS: usize = terrain::SIDE_ROAD_BEARINGS as usize;
const SOLVE_SAMPLE: f32 = terrain::ROAD_HALF_W / 4.0;
const WIDTH_STEPS: i32 = (terrain::ROAD_HALF_W / SOLVE_SAMPLE) as i32;
const AUDIT_SAMPLE: f32 = SOLVE_SAMPLE / 2.0;
const CENTER: f32 = terrain::ISLAND_SIZE / 2.0;

type Point = (f32, f32);

fn distance(a: Point, b: Point) -> f32 {
    ((a.0 - b.0) * (a.0 - b.0) + (a.1 - b.1) * (a.1 - b.1)).sqrt()
}

fn grid_point(i: usize) -> Point {
    ((i % N) as f32 * GRID, (i / N) as f32 * GRID)
}

/// Interpolate the existing yaw table; never use platform trig. Unlike
/// yaw_dir alone, this produces distinct directions between its 256 entries.
fn direction(phase: f32) -> Point {
    let index = phase as usize;
    let t = phase - index as f32;
    let a = sim_core::yaw_dir(((index % 256) as u16) << 8);
    let b = sim_core::yaw_dir((((index + 1) % 256) as u16) << 8);
    let p = (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
    let len = distance(p, (0.0, 0.0));
    (p.0 / len, p.1 / len)
}

fn bearing(p: Point) -> usize {
    (0..256)
        .max_by(|&a, &b| {
            let da = direction(a as f32);
            let db = direction(b as f32);
            ((p.0 - CENTER) * da.0 + (p.1 - CENTER) * da.1)
                .total_cmp(&((p.0 - CENTER) * db.0 + (p.1 - CENTER) * db.1))
        })
        .unwrap()
}

struct Probe {
    seed: u64,
    haven: Haven,
    lattice: Lattice,
}

impl Probe {
    fn new(seed: u64) -> Self {
        Self {
            seed,
            haven: terrain::haven(seed),
            lattice: Lattice::new(),
        }
    }

    fn sample(&mut self, p: Point, carved: bool) -> (f32, f32) {
        if carved {
            (
                terrain::ground_memo(&mut self.lattice, self.seed, &self.haven, p.0, p.1),
                terrain::ground_slope_memo(&mut self.lattice, self.seed, &self.haven, p.0, p.1),
            )
        } else {
            (
                terrain::height_memo(&mut self.lattice, self.seed, p.0, p.1),
                terrain::slope_memo(&mut self.lattice, self.seed, p.0, p.1),
            )
        }
    }

    fn walkable(&mut self, p: Point) -> bool {
        if !(terrain::ROAD_R_MIN..=terrain::ROAD_R_MAX).contains(&distance(p, (CENTER, CENTER))) {
            return false;
        }
        let (height, slope) = self.sample(p, true);
        height >= terrain::LAND_MIN_H && slope <= terrain::CLIFF_SLOPE_RATIO
    }

    /// Conservative square enclosing the 2 m-radius joint. Checks around
    /// bends as well as the perpendicular strip on each traversal edge.
    fn joint(&mut self, p: Point) -> bool {
        for dz in -WIDTH_STEPS..=WIDTH_STEPS {
            for dx in -WIDTH_STEPS..=WIDTH_STEPS {
                if !self.walkable((
                    p.0 + dx as f32 * SOLVE_SAMPLE,
                    p.1 + dz as f32 * SOLVE_SAMPLE,
                )) {
                    return false;
                }
            }
        }
        true
    }

    fn segment(&mut self, a: Point, b: Point) -> bool {
        let len = distance(a, b);
        if len == 0.0 {
            return self.joint(a);
        }
        let normal = (-(b.1 - a.1) / len, (b.0 - a.0) / len);
        let steps = (len / SOLVE_SAMPLE) as usize + 1;
        for k in 0..=steps {
            let t = k as f32 / steps as f32;
            for side in -WIDTH_STEPS..=WIDTH_STEPS {
                let offset = side as f32 * SOLVE_SAMPLE;
                let p = (
                    a.0 + (b.0 - a.0) * t + normal.0 * offset,
                    a.1 + (b.1 - a.1) * t + normal.1 * offset,
                );
                if !self.walkable(p) {
                    return false;
                }
            }
        }
        true
    }

    /// Outermost shoreline crossing, then the existing inland offset.
    /// This is a diagnostic trace, not an exact replacement of ring_band:
    /// coves can have multiple crossings, so the baseline reports misses.
    fn coast(&mut self, phase: f32) -> Option<Point> {
        let d = direction(phase);
        let at = |r: f32| (CENTER + d.0 * r, CENTER + d.1 * r);
        let mut outer = terrain::ROAD_R_MAX + terrain::ROAD_INLAND_M;
        while outer > terrain::ROAD_R_MIN + terrain::ROAD_INLAND_M {
            let inner = outer - GRID;
            let p = at(inner);
            if terrain::height_memo(&mut self.lattice, self.seed, p.0, p.1) > terrain::SEA_LEVEL {
                let mut lo = inner;
                let mut hi = outer;
                // Fixed bisection count derives millimetre resolution from
                // an 8 m bracket; it is numerical accuracy, not road width.
                while hi - lo > SOLVE_SAMPLE / 1024.0 {
                    let mid = (lo + hi) * 0.5;
                    let p = at(mid);
                    if terrain::height_memo(&mut self.lattice, self.seed, p.0, p.1)
                        > terrain::SEA_LEVEL
                    {
                        lo = mid;
                    } else {
                        hi = mid;
                    }
                }
                return Some(at((lo + hi) * 0.5 - terrain::ROAD_INLAND_M));
            }
            outer = inner;
        }
        None
    }
}

struct Router {
    probe: Probe,
    // Unknown / rejected / accepted nodes and edges. Undirected edges are
    // cached in both directions; graph topology and traversal order are fixed.
    nodes: Vec<u8>,
    edges: Vec<[u8; 4]>,
    expanded: usize,
}

impl Router {
    fn new(seed: u64) -> Self {
        Self {
            probe: Probe::new(seed),
            nodes: vec![0; N * N],
            edges: vec![[0; 4]; N * N],
            expanded: 0,
        }
    }

    fn node(&mut self, i: usize) -> bool {
        if self.nodes[i] == 0 {
            let p = grid_point(i);
            let r = distance(p, (CENTER, CENTER));
            self.nodes[i] = if (terrain::ROAD_R_MIN..=terrain::ROAD_R_MAX).contains(&r)
                && self.probe.joint(p)
            {
                2
            } else {
                1
            };
        }
        self.nodes[i] == 2
    }

    fn snap(&mut self, p: Point) -> Option<usize> {
        // Bounded search, sorted by (squared distance, node id). A snap must
        // itself be a valid full-width segment, not a jump over bad ground.
        let radius = (terrain::ROAD_INLAND_M / GRID) as i32;
        let x = (p.0 / GRID) as i32;
        let z = (p.1 / GRID) as i32;
        let mut candidates = Vec::new();
        for dz in -radius..=radius {
            for dx in -radius..=radius {
                let (gx, gz) = (x + dx, z + dz);
                if gx >= 0 && gz >= 0 && gx < N as i32 && gz < N as i32 {
                    let i = gz as usize * N + gx as usize;
                    candidates.push((distance(p, grid_point(i)), i));
                }
            }
        }
        candidates.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        candidates
            .into_iter()
            .find_map(|(_, i)| (self.node(i) && self.probe.segment(p, grid_point(i))).then_some(i))
    }

    fn route(&mut self, a: Point, b: Point) -> Option<Vec<Point>> {
        if self.probe.segment(a, b) {
            return Some(vec![a, b]);
        }
        let start = self.snap(a)?;
        let end = self.snap(b)?;
        let mut parent = vec![usize::MAX; N * N];
        let mut queue = VecDeque::with_capacity(N * N);
        queue.push_back(start);
        parent[start] = start;
        while let Some(i) = queue.pop_front() {
            self.expanded += 1;
            if i == end {
                break;
            }
            let (x, z) = (i % N, i / N);
            let adjacent = [
                (x + 1 < N).then_some(i + 1),
                (x > 0).then(|| i - 1),
                (z + 1 < N).then_some(i + N),
                (z > 0).then(|| i - N),
            ];
            for (direction, j) in adjacent.into_iter().enumerate() {
                let Some(j) = j else { continue };
                if parent[j] != usize::MAX || !self.node(j) {
                    continue;
                }
                if self.edges[i][direction] == 0 {
                    let pass = self.probe.segment(grid_point(i), grid_point(j));
                    let value = if pass { 2 } else { 1 };
                    self.edges[i][direction] = value;
                    self.edges[j][direction ^ 1] = value;
                }
                if self.edges[i][direction] == 2 {
                    parent[j] = i;
                    queue.push_back(j);
                }
            }
        }
        if parent[end] == usize::MAX {
            return None;
        }
        let mut path = vec![b];
        let mut i = end;
        loop {
            path.push(grid_point(i));
            if i == start {
                break;
            }
            i = parent[i];
        }
        path.push(a);
        path.reverse();
        // Greedy visibility simplification, checked with the same full-width
        // traversal rule; total attempts bounded by the unsimplified path².
        let mut simple = vec![a];
        let mut current = 0;
        while current + 1 < path.len() {
            let next = ((current + 1)..path.len())
                .rev()
                .find(|&j| self.probe.segment(path[current], path[j]))?;
            simple.push(path[next]);
            current = next;
        }
        Some(simple)
    }
}

#[derive(Default)]
struct Audit {
    samples: usize,
    bad: usize,
    low: f32,
    steep: f32,
    length: f32,
    longest_bad_run: f32,
    joint_bad: usize,
    joint_samples: usize,
    center_samples: usize,
    center_bad: usize,
    first_bad_center: Option<Point>,
}

fn audit(probe: &mut Probe, path: &[Point], step: f32, carved: bool, ring_only: bool) -> Audit {
    let mut out = Audit {
        low: f32::MAX,
        ..Audit::default()
    };
    let mut bad_run: f32 = 0.0;
    let mut leading_bad: f32 = 0.0;
    let mut at_start = true;
    for edge in path.windows(2) {
        let (a, b) = (edge[0], edge[1]);
        let len = distance(a, b);
        if len == 0.0 {
            continue;
        }
        out.length += len;
        let normal = (-(b.1 - a.1) / len, (b.0 - a.0) / len);
        let along = (len / step) as usize + 1;
        let across = (2.0 * terrain::ROAD_HALF_W / step) as usize + 1;
        for k in 0..=along {
            let t = k as f32 / along as f32;
            let mut blocked = false;
            let center = (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
            // Baseline centre samples count only positions independently
            // recognized by the shipped predicate. Multi-crossing coast
            // traces can jump between branches, so a bad trace alone cannot
            // establish a physical defect in the actual carriageway.
            if !ring_only
                || terrain::ring_band(probe.seed, center.0, center.1) == RoadBand::Carriageway
            {
                let (h, slope) = probe.sample(center, carved);
                out.center_samples += 1;
                let bad = h < terrain::LAND_MIN_H || slope > terrain::CLIFF_SLOPE_RATIO;
                out.center_bad += usize::from(bad);
                if bad && out.first_bad_center.is_none() {
                    out.first_bad_center = Some(center);
                }
                blocked |= bad;
            }
            for s in 0..=across {
                let offset =
                    -terrain::ROAD_HALF_W + 2.0 * terrain::ROAD_HALF_W * s as f32 / across as f32;
                let p = (
                    a.0 + (b.0 - a.0) * t + normal.0 * offset,
                    a.1 + (b.1 - a.1) * t + normal.1 * offset,
                );
                let (h, slope) = probe.sample(p, carved);
                out.samples += 1;
                let bad = h < terrain::LAND_MIN_H || slope > terrain::CLIFF_SLOPE_RATIO;
                out.bad += usize::from(bad);
                blocked |= bad;
                out.low = out.low.min(h);
                out.steep = out.steep.max(slope);
            }
            if blocked {
                if k > 0 {
                    bad_run += len / along as f32;
                    if at_start {
                        leading_bad += len / along as f32;
                    }
                }
                out.longest_bad_run = out.longest_bad_run.max(bad_run);
            } else {
                at_start = false;
                bad_run = 0.0;
            }
        }
    }
    if path.first() == path.last() {
        out.longest_bad_run = out
            .longest_bad_run
            .max((leading_bad + bad_run).min(out.length));
    }
    // Square enclosing each round endpoint/joint, independently sampled at
    // the requested audit spacing. Perpendicular strips alone leave wedges
    // at bends untested. This is conservative around a round join.
    let steps = (2.0 * terrain::ROAD_HALF_W / step) as usize + 1;
    for &p in path {
        for x in 0..=steps {
            for z in 0..=steps {
                let dx =
                    -terrain::ROAD_HALF_W + 2.0 * terrain::ROAD_HALF_W * x as f32 / steps as f32;
                let dz =
                    -terrain::ROAD_HALF_W + 2.0 * terrain::ROAD_HALF_W * z as f32 / steps as f32;
                let (h, slope) = probe.sample((p.0 + dx, p.1 + dz), carved);
                out.joint_samples += 1;
                out.joint_bad +=
                    usize::from(h < terrain::LAND_MIN_H || slope > terrain::CLIFF_SLOPE_RATIO);
            }
        }
    }
    out
}

/// Inclusive intersection: nonadjacent contacts and collinear backtracking
/// are failures too. Adjacent endpoint joins and the final closure are legal.
fn intersections(path: &[Point]) -> usize {
    let turn = |a: Point, b: Point, c: Point| (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0);
    let mut count = 0;
    for i in 0..path.len() - 1 {
        let a = path[i];
        let b = path[(i + 1) % (path.len() - 1)];
        let c = path[(i + 2) % (path.len() - 1)];
        if turn(a, b, c) == 0.0 && (b.0 - a.0) * (c.0 - b.0) + (b.1 - a.1) * (c.1 - b.1) < 0.0 {
            count += 1;
        }
    }
    for (i, a) in path.windows(2).enumerate() {
        for (j, b) in path.windows(2).enumerate().skip(i + 2) {
            if i == 0 && j == path.len() - 2 {
                continue;
            }
            let overlap = a[0].0.min(a[1].0) <= b[0].0.max(b[1].0)
                && b[0].0.min(b[1].0) <= a[0].0.max(a[1].0)
                && a[0].1.min(a[1].1) <= b[0].1.max(b[1].1)
                && b[0].1.min(b[1].1) <= a[0].1.max(a[1].1);
            if overlap
                && turn(a[0], a[1], b[0]) * turn(a[0], a[1], b[1]) <= 0.0
                && turn(b[0], b[1], a[0]) * turn(b[0], b[1], a[1]) <= 0.0
            {
                count += 1;
            }
        }
    }
    count
}

fn encloses_center(path: &[Point]) -> bool {
    let mut inside = false;
    for edge in path.windows(2) {
        let (a, b) = (edge[0], edge[1]);
        if (a.1 > CENTER) != (b.1 > CENTER)
            && CENTER < a.0 + (b.0 - a.0) * (CENTER - a.1) / (b.1 - a.1)
        {
            inside = !inside;
        }
    }
    inside
}

fn run(seed: u64, step: f32, geometry: bool) -> Result<(), &'static str> {
    let mut router = Router::new(seed);
    let h = router.probe.haven;
    println!("seed={seed} haven=({:.2},{:.2})", h.x, h.z);
    for r in h.roads.iter().filter(|r| r.live) {
        println!(
            "side port=({:.2},{:.2}) junction=({:.2},{:.2}) path={:.1}m bend={:.1}m",
            r.px,
            r.pz,
            r.rx,
            r.rz,
            r.path_len(),
            r.bend_m()
        );
    }
    let mut baseline = Vec::new();
    let mut misses = 0;
    // Dense trace avoids the 2/4/8 m raster adjacency question altogether.
    for i in 0..=4096 {
        let p = router
            .probe
            .coast((i % 4096) as f32 / 16.0)
            .ok_or("no coast crossing")?;
        misses += usize::from(terrain::ring_band(seed, p.0, p.1) != RoadBand::Carriageway);
        baseline.push(p);
    }
    for carved in [false, true] {
        let a = audit(&mut router.probe, &baseline, step, carved, true);
        println!("baseline carved={carved} length={:.1}m bad={}/{} low={:.3} slope={:.3} trace_off_ring={misses} longest_bad_width={:.1}m joint_bad={}/{}", a.length,a.bad,a.samples,a.low,a.steep,a.longest_bad_run,a.joint_bad,a.joint_samples);
        println!(
            "center_bad={}/{} (actual ring only for baseline) first_bad={:?}",
            a.center_bad, a.center_samples, a.first_bad_center
        );
    }
    let mut anchors = Vec::new();
    for b in 0..ANCHORS {
        let phase = b * 256 / ANCHORS;
        let p = router.probe.coast(phase as f32).ok_or("no coast anchor")?;
        let dir = direction(phase as f32);
        let mut found = None;
        for k in 0..=(terrain::ROAD_INLAND_M / SOLVE_SAMPLE) as usize {
            for sign in [-1.0, 1.0] {
                let q = (
                    p.0 + dir.0 * k as f32 * SOLVE_SAMPLE * sign,
                    p.1 + dir.1 * k as f32 * SOLVE_SAMPLE * sign,
                );
                if router.probe.joint(q) {
                    found = Some(q);
                    break;
                }
            }
            if found.is_some() {
                break;
            }
        }
        anchors.push((phase, found.ok_or("no walkable coastal anchor")?));
    }
    let mut fixed = vec![(h.x, h.z)];
    fixed.extend(
        h.minor
            .iter()
            .filter(|s| s.live && s.kind == terrain::SiteKind::Waystation)
            .map(|s| (s.x, s.z)),
    );
    fixed.extend(h.roads.iter().filter(|r| r.live).map(|r| (r.rx, r.rz)));
    for &p in &fixed {
        if !router.probe.joint(p) {
            return Err("existing site/junction fails full-width joint audit");
        }
        anchors.push((bearing(p), p));
    }
    anchors.sort_by_key(|a| a.0);
    let mut candidate = Vec::new();
    for i in 0..anchors.len() {
        let a = anchors[i].1;
        let b = anchors[(i + 1) % anchors.len()].1;
        let path = router
            .route(a, b)
            .ok_or("no full-width route between anchors")?;
        candidate.extend_from_slice(&path[..path.len() - 1]);
    }
    candidate.push(candidate[0]);
    assert!(fixed.iter().all(|p| candidate.contains(p)));
    assert_eq!(candidate.first(), candidate.last());
    let crossings = intersections(&candidate);
    println!("candidate nonadjacent_intersections={crossings}");
    if !encloses_center(&candidate) {
        return Err("candidate does not enclose island centre");
    }
    if crossings != 0 {
        return Err("candidate intersects or backtracks");
    }
    for carved in [false, true] {
        let a = audit(&mut router.probe, &candidate, step, carved, false);
        println!("candidate carved={carved} length={:.1}m bad={}/{} low={:.3} slope={:.3} nodes={} expanded={} fixed={}/{} longest_bad_width={:.1}m joint_bad={}/{}", a.length,a.bad,a.samples,a.low,a.steep,candidate.len(),router.expanded,fixed.len(),fixed.len(),a.longest_bad_run,a.joint_bad,a.joint_samples);
        println!(
            "center_bad={}/{} (actual ring only for baseline) first_bad={:?}",
            a.center_bad, a.center_samples, a.first_bad_center
        );
        if carved && (a.bad != 0 || a.joint_bad != 0 || a.center_bad != 0) {
            return Err("candidate failed independent fine carriageway audit");
        }
    }
    if geometry {
        for (name, points) in [
            ("baseline", &baseline),
            ("candidate", &candidate),
            ("anchor", &fixed),
        ] {
            for (i, p) in points.iter().enumerate() {
                println!("{name},{seed},{i},{:.3},{:.3}", p.0, p.1);
            }
        }
    }
    Ok(())
}

fn main() {
    let mut args = std::env::args().skip(1);
    let seed = args
        .next()
        .map(|s| s.parse::<u64>().expect("seed must be u64"));
    let step = args
        .next()
        .map(|s| s.parse::<f32>().expect("audit step must be metres"))
        .unwrap_or(AUDIT_SAMPLE);
    assert!(
        step.is_finite() && (AUDIT_SAMPLE / 2.0..=SOLVE_SAMPLE).contains(&step),
        "audit step must be 0.125..=0.5 m"
    );
    let geometry = match args.next().as_deref() {
        None => false,
        Some("--geometry") => true,
        _ => panic!("expected --geometry"),
    };
    assert!(args.next().is_none(), "unexpected argument");
    let seeds = seed.map_or_else(|| vec![20_260_731, 42, 0xDEAD_BEEF], |s| vec![s]);
    let mut failed = false;
    for seed in seeds {
        if let Err(reason) = run(seed, step, geometry) {
            println!("REFUSED seed={seed}: {reason}");
            failed = true;
        }
    }
    if failed {
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::intersections;

    #[test]
    fn crossing_and_backtracking_are_not_a_simple_loop() {
        let square = [(0.0, 0.0), (8.0, 0.0), (8.0, 8.0), (0.0, 8.0), (0.0, 0.0)];
        assert_eq!(intersections(&square), 0);
        let crossed = [(0.0, 0.0), (8.0, 8.0), (8.0, 0.0), (0.0, 8.0), (0.0, 0.0)];
        assert_eq!(intersections(&crossed), 1);
        let backtracked = [(0.0, 0.0), (8.0, 0.0), (4.0, 0.0), (4.0, 8.0), (0.0, 0.0)];
        assert!(intersections(&backtracked) > 0);
    }
}
