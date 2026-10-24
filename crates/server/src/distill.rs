//! A decision source distilled into a small classifier
//! (`ops/distill/train.py`).
//!
//! `--record DIR` wraps whatever decides (scripted, Jev, an external agent
//! such as Claude, an MCP harness) and appends one JSON line per accepted
//! decision: the observation an external agent is shown, its flattened
//! features, the goals offered and the one picked. `--student FILE` loads
//! what the trainer wrote from those lines and picks among the offered goals
//! itself: no model call and no spend, at any number of bots.
//!
//! The features are a flattening of [`Summary::to_json`], the documented
//! observation, so a recording keeps everything needed to re-featurize it,
//! and the student reads exactly what a teacher was shown. No chat text and
//! no player names are in it. Each row names the harness and model that
//! played (`--agent-harness`, `--agent-model`), null when not given.

use crate::mind::{Choice, DecisionSource, Goal, Reason, Say, SourceKind, Summary};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

/// What every recorded line says it is.
pub const ROW_FORMAT: &str = "gates-decision-1";
/// What a student file must say it is.
pub const STUDENT_FORMAT: &str = "gates-student-1";
/// Elements of a list of objects featurized by position (threats, works).
const LIST_HEAD: usize = 3;
/// Recordings this process opened, so every bot of a `--bots` fleet writes
/// its own file.
static OPENED: AtomicU32 = AtomicU32::new(0);

fn squash(v: f64) -> f32 {
    (v.signum() * v.abs().ln_1p()) as f32
}

/// The observation as named numbers. A number is squashed (signed
/// `ln(1+|v|)`), a `[now, max]` meter is the fraction, a string is a one-hot
/// `key=value`, a `{name, count}` list is one squashed count per name, a list
/// of strings is one flag per string, and the first few objects of any other
/// list are walked by position. Absent and `false` are zero, so not emitted.
pub fn features(obs: &Value) -> Vec<(String, f32)> {
    let mut out = Vec::new();
    walk("", obs, &mut out);
    out
}

fn walk(key: &str, v: &Value, out: &mut Vec<(String, f32)>) {
    match v {
        Value::Null | Value::Bool(false) => {}
        Value::Bool(true) => out.push((key.to_owned(), 1.0)),
        Value::Number(n) => out.push((key.to_owned(), squash(n.as_f64().unwrap_or(0.0)))),
        Value::String(s) => out.push((format!("{key}={s}"), 1.0)),
        Value::Object(m) => {
            for (k, v) in m {
                let at = if key.is_empty() {
                    k.clone()
                } else {
                    format!("{key}.{k}")
                };
                walk(&at, v, out);
            }
        }
        Value::Array(a) => {
            if let [Value::Number(now), Value::Number(max)] = a.as_slice() {
                let max = max.as_f64().unwrap_or(0.0);
                if max > 0.0 {
                    out.push((key.to_owned(), (now.as_f64().unwrap_or(0.0) / max) as f32));
                }
                return;
            }
            out.push((format!("{key}#"), squash(a.len() as f64)));
            // A pack can hold one name in several stacks: summed, then squashed.
            let mut counted: BTreeMap<&str, f64> = BTreeMap::new();
            for (i, e) in a.iter().enumerate() {
                let named = e.get("name").and_then(Value::as_str);
                let count = e.get("count").and_then(Value::as_f64);
                match (e, named, count) {
                    (Value::String(s), _, _) => out.push((format!("{key}[{s}]"), 1.0)),
                    (_, Some(name), Some(count)) => *counted.entry(name).or_default() += count,
                    _ if i < LIST_HEAD => walk(&format!("{key}.{i}"), e, out),
                    _ => {}
                }
            }
            for (name, count) in counted {
                out.push((format!("{key}[{name}]"), squash(count)));
            }
        }
    }
}

/// [`DecisionSource`] that records every answer its source gives, then
/// passes it on unchanged. Writes on the decision worker, never the input
/// loop; a recording that cannot be written stops, and the bot plays on.
pub struct Recorder<S> {
    inner: S,
    out: Option<std::io::BufWriter<std::fs::File>>,
    path: PathBuf,
    rows: u64,
    harness: Option<String>,
    model: Option<String>,
}

impl<S: DecisionSource> Recorder<S> {
    /// A new file in `dir` (made if missing), named so that bots, runs and
    /// processes never share one.
    pub fn open(
        inner: S,
        dir: &Path,
        harness: Option<&str>,
        model: Option<&str>,
    ) -> Result<Self, String> {
        std::fs::create_dir_all(dir).map_err(|e| format!("--record {}: {e}", dir.display()))?;
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let n = OPENED.fetch_add(1, Ordering::Relaxed);
        let path = dir.join(format!("{secs}-{}-{n}.jsonl", std::process::id()));
        let file = std::fs::OpenOptions::new()
            .create_new(true)
            .append(true)
            .open(&path)
            .map_err(|e| format!("--record {}: {e}", path.display()))?;
        eprintln!("record: decisions to {}", path.display());
        Ok(Self {
            inner,
            out: Some(std::io::BufWriter::new(file)),
            path,
            rows: 0,
            harness: harness.map(str::to_owned),
            model: model.map(str::to_owned),
        })
    }
}

/// One recorded decision: what [`ROW_FORMAT`] means.
pub fn row(
    summary: &Summary,
    choice: &Choice,
    source: SourceKind,
    n: u64,
    harness: Option<&str>,
    model: Option<&str>,
) -> Value {
    let obs = summary.to_json();
    let x: Map<String, Value> = features(&obs)
        .into_iter()
        .map(|(k, v)| (k, json!(v)))
        .collect();
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64());
    json!({
        "format": ROW_FORMAT,
        "t": (t * 1000.0).round() / 1000.0,
        "n": n,
        "source": source.label(),
        "harness": harness,
        "model": model,
        "options": summary.options().iter().map(|g| g.label().as_str().to_owned()).collect::<Vec<_>>(),
        "goal": choice.goal.label().as_str(),
        "confidence": choice.confidence,
        "reason": choice.reason.as_str(),
        "x": x,
        "obs": obs,
    })
}

impl<S: DecisionSource> DecisionSource for Recorder<S> {
    fn kind(&self) -> SourceKind {
        self.inner.kind()
    }

    fn decide(&mut self, summary: &Summary) -> Result<Choice, String> {
        self.decide_fresh(summary, &mut || None)
    }

    /// The row pairs the answer with the newest look it was given.
    fn decide_fresh(
        &mut self,
        summary: &Summary,
        fresher: &mut dyn FnMut() -> Option<Summary>,
    ) -> Result<Choice, String> {
        let mut newest = *summary;
        let answer = self.inner.decide_fresh(summary, &mut || {
            let newer = fresher();
            if let Some(n) = newer {
                newest = n;
            }
            newer
        });
        if let (Ok(choice), Some(out)) = (&answer, self.out.as_mut()) {
            let line = row(
                &newest,
                choice,
                self.inner.kind(),
                self.rows,
                self.harness.as_deref(),
                self.model.as_deref(),
            );
            if writeln!(out, "{line}").and_then(|()| out.flush()).is_err() {
                eprintln!(
                    "record: {} could not be written; recording stopped",
                    self.path.display()
                );
                self.out = None;
            }
            self.rows += 1;
        }
        answer
    }

    fn take_note(&mut self) -> Option<crate::bugs::Note> {
        self.inner.take_note()
    }

    /// An MCP mind's standing plan rides through the recording (it was
    /// dropped here until 2026-10-24, so `--record` cost a harness its plans).
    fn take_plan(&mut self) -> Option<crate::mind::Plan> {
        self.inner.take_plan()
    }
}

/// [`DecisionSource`] that keeps the body's journal (`--journal FILE`; each
/// body of a fleet keeps its own, [`journal_path`]):
/// one JSON line per answered decision, in the shape the house's runner reads
/// for the bodies it hosts (`ops/buddy/runner.py`): when, the goal, the
/// reason, a line it said and to whom, what answered, what plays it and its
/// wallet. A status, not a dataset (that is `--record`): nothing of what it
/// saw. Appended and flushed per row, and kept small: past
/// [`JOURNAL_MAX_BYTES`] the file moves to `<FILE>.1` (one older copy) and a
/// new one starts. A journal that cannot be written stops, and the bot plays on.
pub struct Journal<S> {
    inner: S,
    out: Option<std::fs::File>,
    path: PathBuf,
    rows: u64,
    harness: Option<String>,
    model: Option<String>,
    wallet: Option<String>,
}

impl<S: DecisionSource> Journal<S> {
    /// Appends to `path` (made, with its folder, if missing), so a restart
    /// carries on the same file.
    pub fn open(
        inner: S,
        path: &Path,
        harness: Option<&str>,
        model: Option<&str>,
        wallet: Option<&str>,
    ) -> Result<Self, String> {
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir)
                .map_err(|e| format!("--journal {}: {e}", dir.display()))?;
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|e| format!("--journal {}: {e}", path.display()))?;
        eprintln!("journal: what it does, to {}", path.display());
        Ok(Self {
            inner,
            out: Some(file),
            path: path.to_owned(),
            rows: 0,
            harness: harness.map(str::to_owned),
            model: model.map(str::to_owned),
            wallet: wallet.map(str::to_owned),
        })
    }

    /// Past [`JOURNAL_MAX_BYTES`], `<FILE>` becomes `<FILE>.1` and a new one
    /// starts: a body deciding once a second all day stays a few megabytes.
    fn rotate_if_big(&mut self) {
        let big = self
            .out
            .as_ref()
            .and_then(|f| f.metadata().ok())
            .is_some_and(|m| m.len() > JOURNAL_MAX_BYTES);
        if !big {
            return;
        }
        let mut old = self.path.clone().into_os_string();
        old.push(".1");
        self.out = std::fs::rename(&self.path, &old)
            .and_then(|()| {
                std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&self.path)
            })
            .map_err(|e| {
                eprintln!(
                    "journal: {} could not rotate ({e}); journal stopped",
                    self.path.display()
                )
            })
            .ok();
    }
}

/// How big a journal grows before it rotates (`Journal`).
pub const JOURNAL_MAX_BYTES: u64 = 4 << 20;

/// Where body `index` of `bots` keeps its journal: `FILE` itself for one
/// body, `<stem>-<name>.<ext>` beside it for each of a fleet's
/// (`journal-Miller-3.jsonl`), so no two bodies share a file.
pub fn journal_path(file: &Path, name: &str, bots: usize) -> PathBuf {
    if bots <= 1 {
        return file.to_owned();
    }
    let stem = file
        .file_stem()
        .map_or_else(|| "journal".into(), |s| s.to_string_lossy().into_owned());
    let ext = file.extension().map(|e| e.to_string_lossy().into_owned());
    let leaf = match ext {
        Some(e) => format!("{stem}-{name}.{e}"),
        None => format!("{stem}-{name}"),
    };
    file.with_file_name(leaf)
}

/// One journal line: what [`Journal`] writes.
pub fn journal_row(
    choice: &Choice,
    source: SourceKind,
    harness: Option<&str>,
    model: Option<&str>,
    wallet: Option<&str>,
) -> Value {
    let (said, to) = match &choice.say {
        Say::Line(l) => (Some(l), Some("local")),
        Say::Team(l) => (Some(l), Some("team")),
        Say::Global(l) => (Some(l), Some("global")),
        Say::Quiet | Say::Refused => (None, None),
    };
    let said = said.map(|l| {
        l.iter()
            .map(|c| String::from_utf8_lossy(c.as_bytes()).into_owned())
            .collect::<Vec<_>>()
            .join(" ")
    });
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64());
    json!({
        "t": (t * 1000.0).round() / 1000.0,
        "goal": choice.goal.label().as_str(),
        "reason": choice.reason.as_str(),
        "say": said,
        "to": to,
        "answered_by": source.label(),
        "harness": harness,
        "model": model,
        "wallet": wallet,
    })
}

impl<S: DecisionSource> DecisionSource for Journal<S> {
    fn kind(&self) -> SourceKind {
        self.inner.kind()
    }

    fn decide(&mut self, summary: &Summary) -> Result<Choice, String> {
        self.decide_fresh(summary, &mut || None)
    }

    fn decide_fresh(
        &mut self,
        summary: &Summary,
        fresher: &mut dyn FnMut() -> Option<Summary>,
    ) -> Result<Choice, String> {
        let answer = self.inner.decide_fresh(summary, fresher);
        if let (Ok(choice), Some(out)) = (&answer, self.out.as_mut()) {
            let line = journal_row(
                choice,
                self.inner.kind(),
                self.harness.as_deref(),
                self.model.as_deref(),
                self.wallet.as_deref(),
            );
            // One write per row: a reader tailing the file never sees half.
            let mut line = line.to_string();
            line.push('\n');
            if out.write_all(line.as_bytes()).is_err() {
                eprintln!(
                    "journal: {} could not be written; journal stopped",
                    self.path.display()
                );
                self.out = None;
            }
            self.rows += 1;
            if self.rows.is_multiple_of(256) {
                self.rotate_if_big();
            }
        }
        answer
    }

    fn take_note(&mut self) -> Option<crate::bugs::Note> {
        self.inner.take_note()
    }

    fn take_plan(&mut self) -> Option<crate::mind::Plan> {
        self.inner.take_plan()
    }
}

#[derive(Clone)]
struct Layer {
    inputs: usize,
    outputs: usize,
    /// Row-major `[inputs][outputs]`.
    w: Vec<f32>,
    b: Vec<f32>,
}

/// The distilled classifier: features through a small ReLU network to one
/// score per goal label it was trained on, read only for the goals offered.
#[derive(Clone)]
pub struct Student {
    features: BTreeMap<String, usize>,
    mean: Vec<f32>,
    scale: Vec<f32>,
    layers: Vec<Layer>,
    labels: BTreeMap<String, usize>,
}

impl Student {
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("--student {}: {e}", path.display()))?;
        let v: Value = serde_json::from_str(&text)
            .map_err(|e| format!("--student {}: {e}", path.display()))?;
        Self::from_json(&v).map_err(|e| format!("--student {}: {e}", path.display()))
    }

    pub fn from_json(v: &Value) -> Result<Self, String> {
        if v["format"] != STUDENT_FORMAT {
            return Err(format!("not a {STUDENT_FORMAT} file"));
        }
        let strings = |k: &str| -> Result<Vec<String>, String> {
            v[k].as_array()
                .ok_or(format!("no {k}"))?
                .iter()
                .map(|s| {
                    s.as_str()
                        .map(str::to_owned)
                        .ok_or(format!("{k} holds a non-string"))
                })
                .collect()
        };
        let floats = |x: &Value, k: &str| -> Result<Vec<f32>, String> {
            x[k].as_array()
                .ok_or(format!("no {k}"))?
                .iter()
                .map(|f| {
                    f.as_f64()
                        .map(|f| f as f32)
                        .filter(|f| f.is_finite())
                        .ok_or(format!("{k} holds a non-number"))
                })
                .collect()
        };
        let features = strings("features")?;
        let names = strings("labels")?;
        let mean = floats(v, "mean")?;
        let scale = floats(v, "scale")?;
        if mean.len() != features.len() || scale.len() != features.len() {
            return Err("mean and scale must match the features".into());
        }
        let mut layers = Vec::new();
        let mut width = features.len();
        for l in v["layers"].as_array().ok_or("no layers")? {
            let inputs = l["in"].as_u64().ok_or("a layer has no in")? as usize;
            let outputs = l["out"].as_u64().ok_or("a layer has no out")? as usize;
            let (w, b) = (floats(l, "w")?, floats(l, "b")?);
            if inputs != width || w.len() != inputs * outputs || b.len() != outputs {
                return Err("layer shapes do not chain".into());
            }
            width = outputs;
            layers.push(Layer {
                inputs,
                outputs,
                w,
                b,
            });
        }
        if layers.is_empty() || width != names.len() {
            return Err("the last layer must score every label".into());
        }
        let student = Self {
            features: features
                .into_iter()
                .enumerate()
                .map(|(i, k)| (k, i))
                .collect(),
            mean,
            scale,
            layers,
            labels: names.into_iter().enumerate().map(|(i, k)| (k, i)).collect(),
        };
        // The trainer's own scores for one observation: a student read
        // through a different featurizer than it was trained on scores it
        // differently, and is refused rather than played badly.
        if v.get("check").is_some() {
            let want = floats(&v["check"], "scores")?;
            let got = student.scores(&v["check"]["obs"]);
            let off = |(g, w): (&f32, &f32)| (g - w).abs() > 1e-2 * w.abs().max(1.0);
            if got.len() != want.len() || got.iter().zip(&want).any(off) {
                return Err("its check observation scores differently here: \
                            trained on another featurizer"
                    .into());
            }
        }
        Ok(student)
    }

    /// FNV-1a over every weight's bits, layer by layer: what a bred
    /// student's genome takes for its base (`examples/school.rs`), so the
    /// same trained student always breeds by the same seeds.
    pub fn fingerprint(&self) -> u32 {
        let mut h: u32 = 0x811c_9dc5;
        for l in &self.layers {
            for v in l.w.iter().chain(&l.b) {
                for b in v.to_bits().to_le_bytes() {
                    h ^= b as u32;
                    h = h.wrapping_mul(0x0100_0193);
                }
            }
        }
        h
    }

    /// Mutation number `g` with seed `m` on a genome whose base is `base`
    /// (`sim_core::genome::perturb`, the wolves' own recipe): every layer's
    /// weights and biases each draw their own stream, scaled by
    /// 1/√fan-in, so a step is the same size whatever a layer's width.
    pub fn mutate(&mut self, base: u32, g: usize, m: u16) {
        for (k, l) in self.layers.iter_mut().enumerate() {
            let scale = 1.0 / ((l.inputs + 1) as f32).sqrt();
            let stream = g * 64 + 2 * k;
            sim_core::genome::perturb(&mut l.w, |_| scale, base, stream, m);
            sim_core::genome::perturb(&mut l.b, |_| scale, base, stream + 1, m);
        }
    }

    /// The student as a `gates-student-1` file, its check recomputed on
    /// `check_obs` (a mutated student scores its check otherwise than the
    /// trainer did, and `from_json` would refuse the stale one).
    pub fn to_json(&self, check_obs: Option<&Value>) -> Value {
        let ordered = |m: &BTreeMap<String, usize>| -> Vec<String> {
            let mut v = vec![String::new(); m.len()];
            for (k, &i) in m {
                v[i] = k.clone();
            }
            v
        };
        let mut out = json!({
            "format": STUDENT_FORMAT,
            "features": ordered(&self.features),
            "mean": self.mean,
            "scale": self.scale,
            "labels": ordered(&self.labels),
            "layers": self.layers.iter().map(|l| json!({
                "in": l.inputs,
                "out": l.outputs,
                "w": l.w,
                "b": l.b,
            })).collect::<Vec<_>>(),
        });
        if let Some(obs) = check_obs {
            out["check"] = json!({"obs": obs, "scores": self.scores(obs)});
        }
        out
    }

    /// One score per label.
    pub fn scores(&self, obs: &Value) -> Vec<f32> {
        let mut x = vec![0.0f32; self.mean.len()];
        for (k, v) in features(obs) {
            if let Some(&i) = self.features.get(&k) {
                x[i] += v;
            }
        }
        for (i, x) in x.iter_mut().enumerate() {
            *x = (*x - self.mean[i]) * self.scale[i];
        }
        let last = self.layers.len() - 1;
        for (n, l) in self.layers.iter().enumerate() {
            let mut y = l.b.clone();
            for (i, &xi) in x.iter().enumerate().take(l.inputs) {
                if xi == 0.0 {
                    continue;
                }
                let row = &l.w[i * l.outputs..(i + 1) * l.outputs];
                for (y, w) in y.iter_mut().zip(row) {
                    *y += xi * w;
                }
            }
            if n < last {
                y.iter_mut().for_each(|y| *y = y.max(0.0));
            }
            x = y;
        }
        x
    }

    /// The label's own unit, else its kind's (`craft:*`, `walk:*`) for a
    /// named goal the teacher never picked — a model teacher walks to
    /// a hundred places and goes to a dozen, and a student that knew only
    /// the ones it saw could never take the rest.
    fn unit(&self, goal: Goal) -> Option<usize> {
        self.labels
            .get(goal.label().as_str())
            .copied()
            .or_else(|| match goal {
                Goal::Craft(_) | Goal::Buy(_) | Goal::Sell(_) | Goal::Go(_) | Goal::Walk(_) => {
                    self.labels.get(&format!("{}:*", goal.key())).copied()
                }
                _ => None,
            })
    }
}

impl DecisionSource for Student {
    fn kind(&self) -> SourceKind {
        SourceKind::Student
    }

    fn decide(&mut self, summary: &Summary) -> Result<Choice, String> {
        let scores = self.scores(&summary.to_json());
        let mut known: Vec<(Goal, f32)> = summary
            .options()
            .iter()
            .filter_map(|&g| Some((g, scores[self.unit(g)?])))
            .collect();
        if known.is_empty() {
            return Err("the student knows none of the offered goals".into());
        }
        known.sort_by(|a, b| b.1.total_cmp(&a.1));
        let top = known[0].1;
        let total: f32 = known.iter().map(|(_, s)| (s - top).exp()).sum();
        let p = |s: f32| (s - top).exp() / total;
        let next = known
            .get(1)
            .map(|&(g, s)| format!(", next {} {:.0}%", g.label().as_str(), p(s) * 100.0))
            .unwrap_or_default();
        let (goal, _) = known[0];
        Ok(Choice {
            goal,
            confidence: f64::from(p(top)),
            reason: Reason::from_text(&format!(
                "student: {} {:.0}%{next}",
                goal.label().as_str(),
                p(top) * 100.0
            )),
            input_tokens: 0,
            output_tokens: 0,
            say: Say::Quiet,
            code: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mind::{Lines, Name, Scripted};

    fn get(f: &[(String, f32)], k: &str) -> Option<f32> {
        f.iter().find(|(key, _)| key == k).map(|(_, v)| *v)
    }

    #[test]
    fn features_flatten_the_observation_the_way_the_trainer_reads_it() {
        let obs = json!({
            "body": "alive",
            "health": [75, 100],
            "night": true,
            "a_sleeping_bag_is_ready": false,
            "loot_place": null,
            "pack": {"items": [{"name": "Wood", "count": 300}, {"name": "Wood", "count": 200}], "free_slots": 4},
            "craftable_now": ["Stone Hatchet"],
            "threats": [{"range": "near"}, {"range": "far"}, {"range": "mid"}, {"range": "here"}],
        });
        let f = features(&obs);
        assert_eq!(get(&f, "body=alive"), Some(1.0));
        assert_eq!(get(&f, "health"), Some(0.75), "a meter is its fraction");
        assert_eq!(get(&f, "night"), Some(1.0));
        assert_eq!(get(&f, "a_sleeping_bag_is_ready"), None, "false is zero");
        assert_eq!(get(&f, "loot_place"), None);
        assert_eq!(
            get(&f, "pack.items[Wood]"),
            Some(squash(500.0)),
            "stacks summed"
        );
        assert_eq!(get(&f, "pack.free_slots"), Some(squash(4.0)));
        assert_eq!(get(&f, "craftable_now[Stone Hatchet]"), Some(1.0));
        assert_eq!(get(&f, "threats#"), Some(squash(4.0)));
        assert_eq!(get(&f, "threats.2.range=mid"), Some(1.0));
        assert_eq!(
            get(&f, "threats.3.range=here"),
            None,
            "only the list's head"
        );
    }

    /// A row names what played it, and says null when nobody said.
    #[test]
    fn a_row_names_its_harness_and_model_or_null() {
        let mut sum = Summary::EMPTY;
        sum.offer(Goal::Explore);
        let choice = Scripted::default().decide(&sum).unwrap();
        let r = row(
            &sum,
            &choice,
            SourceKind::External,
            0,
            Some("claude-code/2.1"),
            Some("m-1"),
        );
        assert_eq!(r["format"], ROW_FORMAT);
        assert_eq!(
            (r["harness"].as_str(), r["model"].as_str()),
            (Some("claude-code/2.1"), Some("m-1"))
        );
        let r = row(&sum, &choice, SourceKind::Scripted, 1, None, None);
        assert!(r["harness"].is_null() && r["model"].is_null());
    }

    /// The journal keeps one line per answer, appended across restarts, in
    /// the runner's shape: the goal, the reason, a line said and to whom,
    /// what answered and the body's wallet; nothing of what it saw.
    #[test]
    fn the_journal_appends_what_it_does_and_says_in_the_runners_shape() {
        let dir = std::env::temp_dir().join(format!("journal-{}", std::process::id()));
        let path = dir.join("rook.jsonl");
        let _ = std::fs::remove_dir_all(&dir);
        let mut sum = Summary::EMPTY;
        sum.offer(Goal::Explore);
        let wallet = Some("0x00000000000000000000000000000000000000aa");
        for _ in 0..2 {
            let mut j = Journal::open(Scripted::default(), &path, Some("h/1"), Some("m-1"), wallet)
                .unwrap();
            assert_eq!(j.decide(&sum).unwrap().goal, Goal::Explore);
        }
        let text = std::fs::read_to_string(&path).unwrap();
        let rows: Vec<Value> = text
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(rows.len(), 2, "appended, never truncated: {text}");
        let r = &rows[1];
        assert_eq!(r["goal"], "explore");
        assert_eq!(
            (r["answered_by"].as_str(), r["wallet"].as_str()),
            (Some("Scripted"), wallet)
        );
        assert_eq!(
            (r["harness"].as_str(), r["model"].as_str()),
            (Some("h/1"), Some("m-1"))
        );
        assert!(r["say"].is_null() && r["to"].is_null() && r["t"].as_f64().unwrap() > 0.0);
        assert!(
            r.get("obs").is_none() && r.get("options").is_none(),
            "a status, not a dataset"
        );
        let mut said = Scripted::default().decide(&sum).unwrap();
        said.say = Say::Team(Lines::split("on my way").unwrap());
        let r = journal_row(&said, SourceKind::External, None, None, None);
        assert_eq!(
            (r["say"].as_str(), r["to"].as_str()),
            (Some("on my way"), Some("team"))
        );

        // Past its cap it rotates to FILE.1 and starts again, so it stays small.
        std::fs::write(&path, vec![b'x'; JOURNAL_MAX_BYTES as usize + 1]).unwrap();
        let mut j = Journal::open(Scripted::default(), &path, None, None, wallet).unwrap();
        for _ in 0..256 {
            j.decide(&sum).unwrap();
        }
        let mut old = path.clone().into_os_string();
        old.push(".1");
        assert!(std::fs::metadata(&old).unwrap().len() > JOURNAL_MAX_BYTES);
        assert!(std::fs::metadata(&path).unwrap().len() < JOURNAL_MAX_BYTES / 4);
        j.decide(&sum).unwrap();
        assert!(
            std::fs::read_to_string(&path).unwrap().lines().count() == 1,
            "it writes to the new file"
        );
        let _ = std::fs::remove_dir_all(&dir);

        // One body keeps FILE; each of a fleet's keeps its own beside it.
        let f = Path::new("/var/lib/v/journal.jsonl");
        assert_eq!(journal_path(f, "Rook", 1), f);
        assert_eq!(
            journal_path(f, "Miller-3", 12),
            Path::new("/var/lib/v/journal-Miller-3.jsonl")
        );
    }

    /// Bias-only weights: `gather_wood` over `explore`, `craft:*` over both.
    fn student(labels: &[&str], bias: &[f32]) -> Result<Student, String> {
        Student::from_json(&json!({
            "format": STUDENT_FORMAT,
            "features": ["night"],
            "mean": [0.0],
            "scale": [1.0],
            "labels": labels,
            "layers": [{"in": 1, "out": labels.len(), "w": vec![0.0; labels.len()], "b": bias}],
        }))
    }

    #[test]
    fn the_student_picks_only_among_offered_goals_it_knows() {
        let mut s = student(&["explore", "gather_wood", "craft:*"], &[0.0, 1.0, 2.0]).unwrap();
        let mut sum = Summary::EMPTY;
        sum.offer(Goal::Explore);
        sum.offer(Goal::GatherWood);
        let c = s.decide(&sum).unwrap();
        assert_eq!(c.goal, Goal::GatherWood);
        assert!(c.confidence > 0.5 && c.reason.as_str().starts_with("student: gather_wood"));
        let hatchet = Goal::Craft(Name::new(b"Stone Hatchet").unwrap());
        sum.offer(hatchet);
        assert_eq!(
            s.decide(&sum).unwrap().goal,
            hatchet,
            "named goals share their kind's unit"
        );
        let mut wait = Summary::EMPTY;
        wait.offer(Goal::Wait);
        assert!(s.decide(&wait).is_err(), "never a goal it was not offered");
        assert!(
            student(&["explore"], &[0.0, 1.0]).is_err(),
            "shapes are checked"
        );
    }

    /// A bred student (`examples/school.rs`): a mutation moves its weights
    /// the same way every time and a different seed moves them otherwise,
    /// and the file it writes reads back with its recomputed check — the
    /// trainer's stale one would refuse it.
    #[test]
    fn a_mutated_student_is_reproducible_and_writes_a_file_that_reads_back() {
        let base = student(&["explore", "gather_wood"], &[0.0, 1.0]).unwrap();
        let obs = json!({"night": true});
        let bred = |m: u16| {
            let mut s = base.clone();
            s.mutate(base.fingerprint(), 0, m);
            s
        };
        assert_eq!(bred(7).scores(&obs), bred(7).scores(&obs));
        assert_ne!(
            bred(7).scores(&obs),
            base.scores(&obs),
            "a step moved nothing"
        );
        assert_ne!(bred(7).scores(&obs), bred(8).scores(&obs));
        let written = bred(7).to_json(Some(&obs));
        let back = Student::from_json(&written).expect("its own file reads back");
        assert_eq!(back.scores(&obs), bred(7).scores(&obs));
        assert_eq!(back.fingerprint(), bred(7).fingerprint());
        let mut stale = written.clone();
        stale["check"]["scores"] = json!(base.scores(&obs));
        assert!(
            Student::from_json(&stale).is_err(),
            "a stale check is refused"
        );
    }
}
