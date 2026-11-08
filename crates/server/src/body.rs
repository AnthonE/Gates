//! A learned body: the transformer `ops/ticks/body.py` trains, run here at
//! frame rate in place of the scripted hands (`gates-agent --bench N --warp
//! --body FILE`).
//!
//! The mind and the skills are unchanged: the mind picks the goal, the
//! skills still decide the actions (craft, eat, place) and still compute
//! their intent. Only the input frame — buttons, the two axes, the turn, the
//! pitch and the hotbar slot — comes from the model, and the agent adopts it
//! as the frame it sent ([`crate::explorer::Survivor::adopt_frame`]).
//!
//! **Its inputs are a tick record** (`ticks.rs`, built by the same
//! [`ticks::fill`] the recorder uses): what the client saw, read off the
//! record's columns exactly as `clone.observation` reads them in Python, and
//! the frames this body itself sent. `tests/body.rs` holds the port to
//! torch's numbers on a fixture `ops/ticks/export.py --fixture` wrote.
//!
//! **The transformer runs one position a tick** with a key/value cache. Its
//! positions are learned and absolute within a window of `ctx` ticks, so a
//! full window starts again from its newest half: the cache is rebuilt for
//! those `ctx / 2` positions and the window grows again. Every frame is
//! decided with between `ctx / 2` and `ctx − 1` ticks behind it, about two
//! positions' work a tick.
//!
//! **The monologue.** An `aux` body also writes its plan `ahead` ticks on
//! (`ops/ticks/monologue.py`); [`LearnedBody::say`] puts it in words.

use crate::explorer::Survivor;
use crate::mind::Goal;
use crate::ticks;
use crate::view::ClientView;
use serde_json::Value;
use sim_core::input::{InputFrame, BTN_MASK};
use sim_core::rng::{splitmix64, Pcg32};
use std::path::Path;
use std::sync::Arc;

pub const FORMAT: &str = "gates-body-1";

const SIGHTINGS: usize = ticks::SIGHTINGS.len();
const TRACKS: usize = ticks::TRACKS;
const RESOURCES: usize = ticks::RESOURCES.len();
const BUTTON_BITS: usize = 7;
const BEARINGS: usize = 8;
const LN_EPS: f32 = 1e-5;

/// `out = W x + b`, `W` row-major `[out][in]` (torch's `nn.Linear`).
struct Linear {
    w: Vec<f32>,
    b: Vec<f32>,
    n_in: usize,
    n_out: usize,
}

impl Linear {
    fn apply(&self, x: &[f32], out: &mut [f32]) {
        for (o, y) in out.iter_mut().enumerate().take(self.n_out) {
            *y = self.b[o] + dot(&self.w[o * self.n_in..(o + 1) * self.n_in], x);
        }
    }

    fn add(&self, x: &[f32], out: &mut [f32]) {
        for (o, y) in out.iter_mut().enumerate().take(self.n_out) {
            *y += self.b[o] + dot(&self.w[o * self.n_in..(o + 1) * self.n_in], x);
        }
    }
}

struct Norm {
    w: Vec<f32>,
    b: Vec<f32>,
}

impl Norm {
    fn apply(&self, x: &[f32], out: &mut [f32]) {
        let n = x.len() as f32;
        let mean = x.iter().sum::<f32>() / n;
        let var = x.iter().map(|v| (v - mean) * (v - mean)).sum::<f32>() / n;
        let inv = 1.0 / (var + LN_EPS).sqrt();
        for i in 0..x.len() {
            out[i] = (x[i] - mean) * inv * self.w[i] + self.b[i];
        }
    }
}

/// One pre-norm encoder layer (`nn.TransformerEncoderLayer`, `norm_first`).
struct Layer {
    norm1: Norm,
    qkv: Linear,
    out: Linear,
    norm2: Norm,
    up: Linear,
    down: Linear,
}

/// Eight running sums, so the compiler can keep them in one vector register.
fn dot(a: &[f32], b: &[f32]) -> f32 {
    let mut acc = [0.0f32; 8];
    let (ca, cb) = (a.chunks_exact(8), b.chunks_exact(8));
    let (ra, rb) = (ca.remainder(), cb.remainder());
    for (x, y) in ca.zip(cb) {
        for l in 0..8 {
            acc[l] += x[l] * y[l];
        }
    }
    let mut s = acc.iter().sum::<f32>();
    for (x, y) in ra.iter().zip(rb) {
        s += x * y;
    }
    s
}

/// Where the columns the inputs read sit in a record.
struct Cols {
    goal_ticks: usize,
    hp: usize,
    hp_max: usize,
    food: usize,
    water: usize,
    yaw: usize,
    pitch: usize,
    grounded: usize,
    see_count: usize,
    see_m: usize,
    see_bearing: usize,
    tr_species: usize,
    tr_flags: usize,
    tr_bearing: usize,
    tr_dist: usize,
    tr_dy: usize,
    tr_yaw: usize,
    rs_here: usize,
    rs_bearing: usize,
    rs_dist: usize,
}

impl Cols {
    fn new() -> Result<Self, String> {
        let at = |name: &str| {
            ticks::column(name)
                .map(|(o, _)| o)
                .ok_or_else(|| format!("the tick record has no {name}"))
        };
        Ok(Self {
            goal_ticks: at("goal_ticks")?,
            hp: at("hp")?,
            hp_max: at("hp_max")?,
            food: at("food")?,
            water: at("water")?,
            yaw: at("yaw")?,
            pitch: at("pitch")?,
            grounded: at("grounded")?,
            see_count: at("see_count")?,
            see_m: at("see_m")?,
            see_bearing: at("see_bearing")?,
            tr_species: at("tr_species")?,
            tr_flags: at("tr_flags")?,
            tr_bearing: at("tr_bearing")?,
            tr_dist: at("tr_dist")?,
            tr_dy: at("tr_dy")?,
            tr_yaw: at("tr_yaw")?,
            rs_here: at("rs_here")?,
            rs_bearing: at("rs_bearing")?,
            rs_dist: at("rs_dist")?,
        })
    }
}

fn u16_at(r: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([r[o], r[o + 1]])
}
fn i16_at(r: &[u8], o: usize) -> i16 {
    i16::from_le_bytes([r[o], r[o + 1]])
}
fn u32_at(r: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([r[o], r[o + 1], r[o + 2], r[o + 3]])
}
fn f32_at(r: &[u8], o: usize) -> f32 {
    f32::from_le_bytes([r[o], r[o + 1], r[o + 2], r[o + 3]])
}

/// A trained body's weights and tables (`gates-body-1`), shared by every
/// seat that plays it.
pub struct Weights {
    pub ctx: usize,
    pub d: usize,
    pub heads: usize,
    pub n_obs: usize,
    pub n_frame: usize,
    pub goals: Vec<String>,
    pub actions: Vec<String>,
    pub ahead: usize,
    scale: [f32; 2],
    move_z: Vec<i8>,
    move_x: Vec<i8>,
    turn_max: usize,
    pitch_max: usize,
    hotbar: usize,
    plan_sizes: Vec<usize>,
    obs: Linear,
    frame: Linear,
    pos: Vec<f32>,
    layers: Vec<Layer>,
    norm: Norm,
    out_frame: Linear,
    out_plan: Option<Linear>,
    cols: Cols,
}

impl Weights {
    pub fn load(path: &Path) -> Result<Arc<Self>, String> {
        let text = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let v: Value =
            serde_json::from_slice(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::from_json(&v)
            .map(Arc::new)
            .map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn from_json(v: &Value) -> Result<Self, String> {
        if v["format"] != FORMAT {
            return Err(format!("not a {FORMAT} body"));
        }
        let n = |k: &str| {
            v[k].as_u64()
                .map(|x| x as usize)
                .ok_or_else(|| format!("no {k}"))
        };
        let strings = |k: &str| -> Result<Vec<String>, String> {
            v[k].as_array()
                .ok_or_else(|| format!("no {k}"))?
                .iter()
                .map(|s| {
                    s.as_str()
                        .map(str::to_owned)
                        .ok_or_else(|| format!("{k}: not text"))
                })
                .collect()
        };
        let ints = |k: &str| -> Result<Vec<i8>, String> {
            v["values"][k]
                .as_array()
                .ok_or_else(|| format!("no values.{k}"))?
                .iter()
                .map(|x| {
                    x.as_i64()
                        .map(|x| x.clamp(-127, 127) as i8)
                        .ok_or("not a number".into())
                })
                .collect()
        };
        let monologue = v["monologue"].as_str().unwrap_or("off");
        if monologue == "feed" {
            return Err("a feed body reads its own plan back; this port runs aux and off".into());
        }
        let (ctx, d, heads) = (n("ctx")?, n("d")?, n("heads")?);
        let (n_obs, n_frame) = (n("n_obs")?, n("n_frame")?);
        let tensor = |name: &str, shape: &[usize]| -> Result<Vec<f32>, String> {
            let t = &v["tensors"][name];
            let got: Vec<usize> = t["shape"]
                .as_array()
                .ok_or_else(|| format!("no tensor {name}"))?
                .iter()
                .map(|x| x.as_u64().unwrap_or(0) as usize)
                .collect();
            if got != shape {
                return Err(format!("{name}: shape {got:?}, wanted {shape:?}"));
            }
            t["data"]
                .as_array()
                .ok_or_else(|| format!("{name}: no data"))?
                .iter()
                .map(|x| {
                    x.as_f64()
                        .map(|x| x as f32)
                        .ok_or(format!("{name}: not a number"))
                })
                .collect()
        };
        let linear = |name: &str, n_out: usize, n_in: usize| -> Result<Linear, String> {
            Ok(Linear {
                w: tensor(&format!("{name}.weight"), &[n_out, n_in])?,
                b: tensor(&format!("{name}.bias"), &[n_out])?,
                n_in,
                n_out,
            })
        };
        let norm = |name: &str| -> Result<Norm, String> {
            Ok(Norm {
                w: tensor(&format!("{name}.weight"), &[d])?,
                b: tensor(&format!("{name}.bias"), &[d])?,
            })
        };
        let ff = v["tensors"]["tr.layers.0.linear1.weight"]["shape"][0]
            .as_u64()
            .ok_or("no tr.layers.0.linear1")? as usize;
        let mut layers = Vec::new();
        for i in 0..n("layers")? {
            let p = format!("tr.layers.{i}");
            layers.push(Layer {
                norm1: norm(&format!("{p}.norm1"))?,
                qkv: Linear {
                    w: tensor(&format!("{p}.self_attn.in_proj_weight"), &[3 * d, d])?,
                    b: tensor(&format!("{p}.self_attn.in_proj_bias"), &[3 * d])?,
                    n_in: d,
                    n_out: 3 * d,
                },
                out: linear(&format!("{p}.self_attn.out_proj"), d, d)?,
                norm2: norm(&format!("{p}.norm2"))?,
                up: linear(&format!("{p}.linear1"), ff, d)?,
                down: linear(&format!("{p}.linear2"), d, ff)?,
            });
        }
        let goals = strings("goals")?;
        let actions = strings("actions")?;
        let plan_sizes: Vec<usize> = v["plan_sizes"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|p| p[1].as_u64())
                    .map(|x| x as usize)
                    .collect()
            })
            .unwrap_or_default();
        let (turn_max, pitch_max, hotbar) = (n("turn_max")?, n("pitch_max")?, n("hotbar")?);
        let frame_out = BUTTON_BITS + 3 + 5 + (2 * turn_max + 1) + (2 * pitch_max + 1) + hotbar;
        let want_obs = goals.len()
            + 1
            + 6
            + SIGHTINGS * (3 + BEARINGS)
            + TRACKS * (1 + 3 + 8 + 6)
            + RESOURCES * 4;
        if n_obs != want_obs {
            return Err(format!(
                "{n_obs} observation inputs; this build reads {want_obs}"
            ));
        }
        let want_frame = BUTTON_BITS + 2 + (2 * turn_max + 1) + 1 + hotbar;
        if n_frame != want_frame {
            return Err(format!(
                "{n_frame} frame inputs; this build makes {want_frame}"
            ));
        }
        if d % heads != 0 {
            return Err(format!("{d} wide in {heads} heads"));
        }
        let out_plan = if monologue == "aux" {
            Some(linear(
                "out_plan",
                plan_sizes.iter().sum::<usize>() + BUTTON_BITS,
                d,
            )?)
        } else {
            None
        };
        Ok(Self {
            ctx,
            d,
            heads,
            n_obs,
            n_frame,
            ahead: n("ahead")?,
            scale: [
                v["scale"]["food"].as_f64().unwrap_or(1.0) as f32,
                v["scale"]["water"].as_f64().unwrap_or(1.0) as f32,
            ],
            move_z: ints("move_z")?,
            move_x: ints("move_x")?,
            turn_max,
            pitch_max,
            hotbar,
            plan_sizes,
            obs: linear("obs", d, n_obs)?,
            frame: linear("frame", d, n_frame)?,
            pos: tensor("pos.weight", &[ctx, d])?,
            layers,
            norm: norm("norm")?,
            out_frame: linear("out_frame", frame_out, d)?,
            out_plan,
            cols: Cols::new()?,
            goals,
            actions,
        })
    }

    /// The goal's row in the goal table: 0 for none or one it never saw.
    pub fn goal_index(&self, label: Option<&str>) -> usize {
        label
            .and_then(|l| self.goals.iter().position(|g| g == l))
            .map_or(0, |i| i + 1)
    }

    /// What the client saw, as the model reads it (`clone.observation`).
    pub fn observation(&self, row: &[u8], goal: usize, out: &mut [f32]) {
        let c = &self.cols;
        out.fill(0.0);
        let g = self.goals.len() + 1;
        out[goal.min(g - 1)] = 1.0;
        let mut i = g;
        let mut put = |v: f32| {
            out[i] = v;
            i += 1;
        };
        put((u32_at(row, c.goal_ticks) as f32).ln_1p() / 8.0);
        put(u16_at(row, c.hp) as f32 / f32::from(u16_at(row, c.hp_max).max(1)));
        put(u16_at(row, c.food) as f32 / self.scale[0]);
        put(u16_at(row, c.water) as f32 / self.scale[1]);
        put(f32::from(row[c.grounded]));
        put(f32::from(row[c.pitch]) / 255.0);
        let count = |s: usize| row[c.see_count + s];
        for s in 0..SIGHTINGS {
            put(f32::from(u8::from(count(s) > 0)));
        }
        for s in 0..SIGHTINGS {
            put(f32::from(count(s).min(10)) / 10.0);
        }
        for s in 0..SIGHTINGS {
            put(f32::from(row[c.see_m + s]) / 255.0);
        }
        for s in 0..SIGHTINGS {
            let b = usize::from(row[c.see_bearing + s].min(7));
            for k in 0..BEARINGS {
                put(f32::from(u8::from(count(s) > 0 && k == b)));
            }
        }
        let species = |t: usize| row[c.tr_species + t];
        let here = |t: usize| f32::from(u8::from(species(t) > 0));
        for t in 0..TRACKS {
            put(here(t));
        }
        for t in 0..TRACKS {
            for k in 1..=3 {
                put(f32::from(u8::from(species(t) == k)));
            }
        }
        for t in 0..TRACKS {
            for b in 0..8 {
                put(f32::from((row[c.tr_flags + t] >> b) & 1));
            }
        }
        let pi = std::f32::consts::PI;
        let ang = |t: usize| i16_at(row, c.tr_bearing + 2 * t) as f32 / 32768.0 * pi;
        let my_yaw = u16_at(row, c.yaw);
        let face = |t: usize| {
            u16_at(row, c.tr_yaw + 2 * t).wrapping_sub(my_yaw) as i16 as f32 / 32768.0 * pi
        };
        for t in 0..TRACKS {
            put(ang(t).sin() * here(t));
        }
        for t in 0..TRACKS {
            put(ang(t).cos() * here(t));
        }
        for t in 0..TRACKS {
            put((f32_at(row, c.tr_dist + 4 * t) / 50.0).min(1.0) * here(t));
        }
        for t in 0..TRACKS {
            put((f32_at(row, c.tr_dy + 4 * t) / 5.0).clamp(-1.0, 1.0) * here(t));
        }
        for t in 0..TRACKS {
            put(face(t).sin() * here(t));
        }
        for t in 0..TRACKS {
            put(face(t).cos() * here(t));
        }
        let known = |k: usize| f32::from(row[c.rs_here + k]);
        let rang = |k: usize| i16_at(row, c.rs_bearing + 2 * k) as f32 / 32768.0 * pi;
        for k in 0..RESOURCES {
            put(known(k));
        }
        for k in 0..RESOURCES {
            put(rang(k).sin() * known(k));
        }
        for k in 0..RESOURCES {
            put(rang(k).cos() * known(k));
        }
        for k in 0..RESOURCES {
            put((f32_at(row, c.rs_dist + 4 * k) / 50.0).min(1.0) * known(k));
        }
        debug_assert_eq!(i, self.n_obs);
    }

    /// The frame `last` as an input (`body.frame_features`): `before` is the
    /// frame sent the tick before it, for its turn; none is no turn. All
    /// zeros when nothing was sent yet.
    pub fn frame_input(
        &self,
        last: Option<&InputFrame>,
        before: Option<&InputFrame>,
        out: &mut [f32],
    ) {
        out.fill(0.0);
        let Some(f) = last else {
            return;
        };
        for (b, o) in out.iter_mut().enumerate().take(BUTTON_BITS) {
            *o = f32::from((f.buttons >> b) & 1);
        }
        out[BUTTON_BITS] = f32::from(f.move_x) / 127.0;
        out[BUTTON_BITS + 1] = f32::from(f.move_z) / 127.0;
        let turn = before.map_or(0, |b| f.yaw.wrapping_sub(b.yaw) as i16);
        let tc = turn_class(turn, self.turn_max);
        let at = BUTTON_BITS + 2;
        out[at + tc] = 1.0;
        let at = at + 2 * self.turn_max + 1;
        out[at] = f32::from(f.pitch) / 255.0;
        out[at + 1 + usize::from(f.sel).min(self.hotbar - 1)] = 1.0;
    }
}

/// A wire-yaw change as its class: 1/256ths of a turn, clipped, centred.
fn turn_class(turn: i16, max: usize) -> usize {
    let steps = (f32::from(turn) / 256.0).round_ties_even() as i64;
    (steps.clamp(-(max as i64), max as i64) + max as i64) as usize
}

/// A class drawn from logits `v` (softmax), `u` uniform in [0, 1).
fn draw(v: &[f32], u: f32) -> usize {
    let top = v.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let sum: f32 = v.iter().map(|x| (x - top).exp()).sum();
    let mut acc = 0.0;
    for (i, x) in v.iter().enumerate() {
        acc += (x - top).exp() / sum;
        if u < acc {
            return i;
        }
    }
    v.len() - 1
}

fn argmax(v: &[f32]) -> usize {
    let mut best = 0;
    for (i, x) in v.iter().enumerate() {
        if *x > v[best] {
            best = i;
        }
    }
    best
}

/// One seat's learned body: the window of ticks it has seen, its cache, and
/// the frames it sent.
pub struct LearnedBody {
    w: Arc<Weights>,
    /// Each position's input before its position is added (`obs + frame`),
    /// kept so a window can start again from its newest half.
    embedded: Vec<f32>,
    len: usize,
    keys: Vec<Vec<f32>>,
    values: Vec<Vec<f32>>,
    sent: [Option<InputFrame>; 2],
    row: Vec<u8>,
    goal: Option<(Goal, usize)>,
    obs_in: Vec<f32>,
    frame_in: Vec<f32>,
    x: Vec<f32>,
    h: Vec<f32>,
    qkv: Vec<f32>,
    att: Vec<f32>,
    ff: Vec<f32>,
    scores: Vec<f32>,
    pub frame_logits: Vec<f32>,
    pub plan_logits: Vec<f32>,
    /// Draws each output from the model's odds when set; takes the likeliest
    /// otherwise.
    rng: Option<Pcg32>,
}

impl LearnedBody {
    pub fn new(w: Arc<Weights>) -> Self {
        let (d, ctx) = (w.d, w.ctx);
        let ff = w.layers.first().map_or(0, |l| l.up.n_out);
        let layers = w.layers.len();
        Self {
            embedded: vec![0.0; ctx * d],
            len: 0,
            keys: vec![vec![0.0; ctx * d]; layers],
            values: vec![vec![0.0; ctx * d]; layers],
            sent: [None, None],
            row: Vec::with_capacity(ticks::row_bytes()),
            goal: None,
            obs_in: vec![0.0; w.n_obs],
            frame_in: vec![0.0; w.n_frame],
            x: vec![0.0; d],
            h: vec![0.0; d],
            qkv: vec![0.0; 3 * d],
            att: vec![0.0; d],
            ff: vec![0.0; ff],
            scores: vec![0.0; ctx],
            frame_logits: vec![0.0; w.out_frame.n_out],
            plan_logits: vec![0.0; w.out_plan.as_ref().map_or(0, |p| p.n_out)],
            rng: None,
            w,
        }
    }

    /// A body that draws its frame from the model's odds instead of taking
    /// the likeliest frame, its draws seeded by `seed` (the same seed plays
    /// the same). Taking the likeliest frame copies the last one: a body
    /// standing still stays standing, since starting to walk is never the
    /// single likeliest thing on any one tick.
    pub fn sampling(w: Arc<Weights>, seed: u64) -> Self {
        let mut b = Self::new(w);
        b.rng = Some(Pcg32::new(splitmix64(seed), 0x626f_6479));
        b
    }

    pub fn weights(&self) -> &Weights {
        &self.w
    }

    /// Push one tick's input and run its position: the logits land in
    /// `frame_logits` and `plan_logits`.
    pub fn step(&mut self, obs: &[f32], frame: &[f32]) {
        let (d, ctx) = (self.w.d, self.w.ctx);
        if self.len == ctx {
            // Start the window again from its newest half.
            let keep = ctx / 2;
            self.embedded.copy_within((ctx - keep) * d..ctx * d, 0);
            self.len = 0;
            for p in 0..keep {
                self.x.copy_from_slice(&self.embedded[p * d..(p + 1) * d]);
                self.position(p, false);
            }
            self.len = keep;
        }
        let p = self.len;
        let mut e = std::mem::take(&mut self.x);
        self.w.obs.apply(obs, &mut e);
        self.w.frame.add(frame, &mut e);
        self.embedded[p * d..(p + 1) * d].copy_from_slice(&e);
        self.x = e;
        self.position(p, true);
        self.len = p + 1;
    }

    /// Position `p` through every layer, `x` holding its embedded input.
    /// Its keys and values go into the cache; with `heads`, the outputs too.
    fn position(&mut self, p: usize, heads: bool) {
        let w = Arc::clone(&self.w);
        let d = w.d;
        let hd = d / w.heads;
        let scale = 1.0 / (hd as f32).sqrt();
        for (i, v) in self.x.iter_mut().enumerate() {
            *v += w.pos[p * d + i];
        }
        for (li, layer) in w.layers.iter().enumerate() {
            layer.norm1.apply(&self.x, &mut self.h);
            layer.qkv.apply(&self.h, &mut self.qkv);
            self.keys[li][p * d..(p + 1) * d].copy_from_slice(&self.qkv[d..2 * d]);
            self.values[li][p * d..(p + 1) * d].copy_from_slice(&self.qkv[2 * d..3 * d]);
            for hh in 0..w.heads {
                let q = &self.qkv[hh * hd..(hh + 1) * hd];
                let mut top = f32::NEG_INFINITY;
                for j in 0..=p {
                    let k = &self.keys[li][j * d + hh * hd..j * d + (hh + 1) * hd];
                    let s = dot(q, k) * scale;
                    self.scores[j] = s;
                    top = top.max(s);
                }
                let mut sum = 0.0;
                for s in &mut self.scores[..=p] {
                    *s = (*s - top).exp();
                    sum += *s;
                }
                let out = &mut self.att[hh * hd..(hh + 1) * hd];
                out.fill(0.0);
                for j in 0..=p {
                    let a = self.scores[j] / sum;
                    let v = &self.values[li][j * d + hh * hd..j * d + (hh + 1) * hd];
                    for (o, x) in out.iter_mut().zip(v) {
                        *o += a * x;
                    }
                }
            }
            layer.out.add(&self.att, &mut self.x);
            layer.norm2.apply(&self.x, &mut self.h);
            layer.up.apply(&self.h, &mut self.ff);
            for v in &mut self.ff {
                *v = v.max(0.0);
            }
            layer.down.add(&self.ff, &mut self.x);
        }
        if heads {
            w.norm.apply(&self.x, &mut self.h);
            w.out_frame.apply(&self.h, &mut self.frame_logits);
            if let Some(plan) = &w.out_plan {
                plan.apply(&self.h, &mut self.plan_logits);
            }
        }
    }

    /// This tick's frame for `bot`, in place of `scripted` (whose `seq` it
    /// keeps): the record of what the client saw, the model on it, the
    /// frame decoded and remembered as sent.
    pub fn drive(
        &mut self,
        bot: &Survivor,
        view: &ClientView,
        player: u32,
        tick: u64,
        seat: usize,
        scripted: &InputFrame,
    ) -> InputFrame {
        let goal = match (bot.goal(), self.goal) {
            (Some(g), Some((last, i))) if g == last => i,
            (Some(g), _) => {
                let i = self.w.goal_index(Some(g.label().as_str()));
                self.goal = Some((g, i));
                i
            }
            (None, _) => 0,
        };
        let mut row = std::mem::take(&mut self.row);
        ticks::fill(&mut row, tick, seat, 0, 0, bot, view, player, scripted);
        let mut obs = std::mem::take(&mut self.obs_in);
        let mut fin = std::mem::take(&mut self.frame_in);
        self.w.observation(&row, goal, &mut obs);
        self.w
            .frame_input(self.sent[0].as_ref(), self.sent[1].as_ref(), &mut fin);
        self.step(&obs, &fin);
        self.obs_in = obs;
        self.frame_in = fin;
        self.row = row;
        // Before its first frame it holds the view the hands start from
        // (the body's yaw, eyes level): before the first snapshot the body
        // has no view at all, and a pitch of 0 is straight down.
        let f = self.decode(scripted.seq, (scripted.yaw, scripted.pitch));
        self.sent = [Some(f), self.sent[0]];
        f
    }

    /// The frame the logits say, turning from the last frame sent (from
    /// `base`, the body's own view, before any).
    fn decode(&mut self, seq: u16, base: (u16, u8)) -> InputFrame {
        let w = Arc::clone(&self.w);
        let z = &self.frame_logits;
        let rng = &mut self.rng;
        let mut uniform = || {
            rng.as_mut()
                .map(|r| (r.next_u32() >> 8) as f32 / 16_777_216.0)
        };
        let mut buttons = 0u8;
        for (b, x) in z.iter().enumerate().take(BUTTON_BITS) {
            let on = match uniform() {
                Some(u) => u < 1.0 / (1.0 + (-x).exp()),
                None => *x > 0.0,
            };
            if on {
                buttons |= 1 << b;
            }
        }
        let mut at = BUTTON_BITS;
        let mut head = |n: usize| {
            let v = &z[at..at + n];
            let k = uniform().map_or_else(|| argmax(v), |u| draw(v, u));
            at += n;
            k
        };
        let mz = head(3);
        let mx = head(5);
        let tc = head(2 * w.turn_max + 1);
        let pc = head(2 * w.pitch_max + 1);
        let sel = head(w.hotbar);
        let (yaw, pitch) = self.sent[0].map_or(base, |f| (f.yaw, f.pitch));
        let turn = (tc as i32 - w.turn_max as i32) * 256;
        let pitch = (i32::from(pitch) + pc as i32 - w.pitch_max as i32).clamp(0, 255) as u8;
        InputFrame {
            seq,
            buttons: buttons & BTN_MASK,
            yaw: yaw.wrapping_add(turn as u16),
            pitch,
            pitch_fine: 0,
            move_x: w.move_x.get(mx).copied().unwrap_or(0),
            move_z: w.move_z.get(mz).copied().unwrap_or(0),
            sel: sel as u8,
        }
    }

    /// The plan it wrote on the last tick, for `ahead` ticks on, in words
    /// (`ops/ticks/monologue.py`'s `say`). `None` for a body with no plan.
    pub fn say(&self, goal: &str) -> Option<String> {
        if self.plan_logits.is_empty() {
            return None;
        }
        let z = &self.plan_logits;
        let mut at = 0;
        let mut pick = Vec::with_capacity(self.w.plan_sizes.len());
        for &n in &self.w.plan_sizes {
            pick.push(argmax(&z[at..at + n]));
            at += n;
        }
        let hands: Vec<bool> = (0..BUTTON_BITS).map(|b| z[at + b] > 0.0).collect();
        let [look, wher, far, mv, next] = pick[..] else {
            return None;
        };
        Some(say(
            goal,
            look,
            wher,
            far,
            mv,
            next,
            &hands,
            &self.w.actions,
        ))
    }
}

const BEARING_WORDS: [&str; 8] = [
    "ahead",
    "ahead-right",
    "right",
    "behind-right",
    "behind",
    "behind-left",
    "left",
    "ahead-left",
];
const FAR_WORDS: [&str; 5] = ["none", "<2m", "2-6m", "6-20m", "20m+"];
const BUTTON_WORDS: [&str; 7] = [
    "sprint", "crouch", "swing", "jump", "light", "assist", "aim",
];

#[allow(clippy::too_many_arguments)]
fn say(
    goal: &str,
    look: usize,
    wher: usize,
    far: usize,
    mv: usize,
    next: usize,
    hands: &[bool],
    actions: &[String],
) -> String {
    let mut held: Vec<&str> = (0..BUTTON_BITS)
        .filter(|&b| hands[b])
        .map(|b| BUTTON_WORDS[b])
        .collect();
    let mut parts: Vec<String> = Vec::new();
    let mut walking = None;
    match mv {
        0 => parts.push("standing".into()),
        1 => parts.push("stepping forward".into()),
        2 => parts.push("backing off".into()),
        3 => parts.push("sidestepping left".into()),
        4 => parts.push("sidestepping right".into()),
        m => {
            let b = BEARING_WORDS[(m - 5).min(7)];
            walking = Some(b);
            let verb = if held.contains(&"sprint") {
                "running"
            } else {
                "walking"
            };
            held.retain(|h| *h != "sprint");
            parts.push(format!("{verb} {b}"));
        }
    }
    if look == 1 && wher > 0 && wher < 8 && Some(BEARING_WORDS[wher]) != walking {
        parts.push(format!("turning {}", BEARING_WORDS[wher]));
    } else if (look == 2 || look == 3) && wher < 8 {
        let what = if look == 3 { "a body" } else { "a spot" };
        let dist = if far > 0 {
            format!(" {}", FAR_WORDS[far.min(4)])
        } else {
            String::new()
        };
        parts.push(format!("eyes on {what}{dist} {}", BEARING_WORDS[wher]));
    }
    if !held.is_empty() {
        parts.push(held.join(" + "));
    }
    if next > 0 {
        if let Some(a) = actions.get(next - 1) {
            parts.push(format!("about to {}", a.to_lowercase()));
        }
    }
    format!("{goal}: {}", parts.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_turn_class_is_centred_and_clipped() {
        assert_eq!(turn_class(0, 8), 8);
        assert_eq!(turn_class(256, 8), 9);
        assert_eq!(turn_class(-512, 8), 6);
        assert_eq!(turn_class(i16::MAX, 8), 16);
        assert_eq!(turn_class(i16::MIN, 8), 0);
    }

    #[test]
    fn the_monologue_reads_like_monologue_py() {
        let none = [false; 7];
        let mut sprint = none;
        sprint[0] = true;
        let acts = ["Craft".to_string()];
        assert_eq!(
            say("gather_wood", 1, 0, 0, 5, 0, &sprint, &acts),
            "gather_wood: running ahead"
        );
        assert_eq!(
            say("build", 1, 7, 0, 6, 1, &none, &acts),
            "build: walking ahead-right, turning ahead-left, about to craft"
        );
        assert_eq!(
            say("hunt", 3, 0, 2, 0, 0, &none, &acts),
            "hunt: standing, eyes on a body 2-6m ahead"
        );
    }
}
