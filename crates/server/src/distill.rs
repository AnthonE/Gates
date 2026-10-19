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
}

struct Layer {
    inputs: usize,
    outputs: usize,
    /// Row-major `[inputs][outputs]`.
    w: Vec<f32>,
    b: Vec<f32>,
}

/// The distilled classifier: features through a small ReLU network to one
/// score per goal label it was trained on, read only for the goals offered.
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

    /// The label's own unit, else its kind's (`craft:*`) for a named goal
    /// the teacher never picked.
    fn unit(&self, goal: Goal) -> Option<usize> {
        self.labels
            .get(goal.label().as_str())
            .copied()
            .or_else(|| match goal {
                Goal::Craft(_) | Goal::Buy(_) | Goal::Sell(_) => {
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
    use crate::mind::{Name, Scripted};

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
}
