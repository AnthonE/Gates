//! A layer's recipe: what one kind of particle an effect throws looks like
//! and how it flies, as plain numbers, and [`emit`], which rolls particles
//! from one.
//!
//! A game names its layers (Sparks, Dust, Smoke…) and keeps one recipe per
//! layer in code. **Knobs** retune a recipe at run time without a rebuild, so
//! the fx stage can try a change in seconds: `Sparks.speed=4,12;Flash.life=0.04,0.07`
//! ([`knobs`] parses it, [`Recipe::set`] applies one, and a recipe prints
//! itself back in the same syntax, so a tuned look pastes into code).

use core::fmt;

use crate::pool::{Orient, Particle, Pool};
use crate::v3::{self, V3};

/// One layer's recipe.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Recipe {
    pub orient: Orient,
    /// The first sprite cell, and how many variants from it a particle
    /// rolls: the next cell each, or the next flipbook (`frames` cells on).
    pub cell: u8,
    pub cells: u8,
    /// A flipbook: cells played from the one rolled, once over the
    /// particle's life (row-major on the pool's sheet). 0 or 1: a still.
    pub frames: u8,
    /// A baked sheet's heat: which of the game's emissive ramps it takes,
    /// and how bright.
    pub tone: f32,
    pub glow: f32,
    /// Launch speed range, m/s.
    pub speed: (f32, f32),
    /// Share of the launch along the thrown direction rather than scattered.
    pub spray: f32,
    /// Extra upward speed, m/s.
    pub up: f32,
    /// Downward acceleration, m/s²; negative rises.
    pub gravity: f32,
    /// Velocity decay per second.
    pub drag: f32,
    /// Life range, seconds.
    pub life: (f32, f32),
    /// Half-extent range at birth, metres.
    pub size: (f32, f32),
    /// End size over start size.
    pub grow: f32,
    /// Linear colour and alpha at birth and at death. HDR allowed.
    pub c0: [f32; 4],
    pub c1: [f32; 4],
    /// Multiply the colours by the caller's tint (the struck matter's).
    pub tinted: bool,
    /// Roll speed range, ± rad/s.
    pub spin: f32,
    /// Seconds of velocity a streak is drawn over.
    pub stretch: f32,
    /// The longest a streak may draw, metres; zero is the pool's default.
    pub len_max: f32,
    /// Bounce once off the plane the effect came off.
    pub bounce: bool,
}

impl Recipe {
    /// A recipe to start from: a still white puff in cell 5 for half a second.
    pub const BASE: Recipe = Recipe {
        orient: Orient::Billboard,
        cell: 5,
        cells: 1,
        frames: 0,
        tone: 0.0,
        glow: 1.0,
        speed: (0.0, 0.0),
        spray: 0.5,
        up: 0.0,
        gravity: 0.0,
        drag: 0.0,
        life: (0.5, 0.5),
        size: (0.05, 0.05),
        grow: 1.0,
        c0: [1.0; 4],
        c1: [1.0, 1.0, 1.0, 0.0],
        tinted: false,
        spin: 0.0,
        stretch: 0.0,
        len_max: 0.0,
        bounce: false,
    };

    /// The knob names [`Recipe::set`] takes, in the order a recipe prints.
    pub const KEYS: [&'static str; 21] = [
        "orient", "cell", "cells", "frames", "tone", "glow", "speed", "spray", "up", "gravity",
        "drag", "life", "size", "grow", "c0", "c1", "tinted", "spin", "stretch", "len_max",
        "bounce",
    ];

    /// Set one knob from its text: a number, a pair `a,b`, a colour
    /// `r,g,b,a`, `true`/`false`, or an orientation
    /// (`billboard`/`stretch`/`flat`).
    pub fn set(&mut self, key: &str, val: &str) -> Result<(), String> {
        let bad = |what: &str| format!("{key}={val}: {what}");
        let num = |s: &str| {
            s.trim()
                .parse::<f32>()
                .map_err(|_| bad("not a number"))
                .and_then(|x| {
                    if x.is_finite() {
                        Ok(x)
                    } else {
                        Err(bad("not finite"))
                    }
                })
        };
        let nums = |n: usize| -> Result<Vec<f32>, String> {
            let v = val.split(',').map(num).collect::<Result<Vec<_>, _>>()?;
            // One number for a range is the range `a..a`.
            if n == 2 && v.len() == 1 {
                return Ok(vec![v[0], v[0]]);
            }
            if v.len() == n {
                Ok(v)
            } else {
                Err(bad(&format!("wants {n} numbers")))
            }
        };
        let flag = || match val.trim() {
            "true" | "1" | "yes" => Ok(true),
            "false" | "0" | "no" => Ok(false),
            _ => Err(bad("wants true or false")),
        };
        let byte = |lo: u8| {
            let x = num(val)?;
            if x.fract() == 0.0 && x >= lo as f32 && x <= 255.0 {
                Ok(x as u8)
            } else {
                Err(bad("wants a whole number"))
            }
        };
        match key {
            "orient" => {
                self.orient = match val.trim() {
                    "billboard" => Orient::Billboard,
                    "stretch" => Orient::Stretch,
                    "flat" => Orient::Flat,
                    _ => return Err(bad("wants billboard, stretch or flat")),
                }
            }
            "cell" => self.cell = byte(0)?,
            "cells" => self.cells = byte(1)?,
            "frames" => self.frames = byte(0)?,
            "tone" => self.tone = num(val)?,
            "glow" => self.glow = num(val)?,
            "speed" => self.speed = pair(&nums(2)?),
            "spray" => self.spray = num(val)?,
            "up" => self.up = num(val)?,
            "gravity" => self.gravity = num(val)?,
            "drag" => self.drag = num(val)?,
            "life" => {
                let life = pair(&nums(2)?);
                if life.0 <= 0.0 || life.1 <= 0.0 {
                    return Err(bad("a life is longer than zero"));
                }
                self.life = life;
            }
            "size" => self.size = pair(&nums(2)?),
            "grow" => self.grow = num(val)?,
            "c0" => self.c0 = quad(&nums(4)?),
            "c1" => self.c1 = quad(&nums(4)?),
            "tinted" => self.tinted = flag()?,
            "spin" => self.spin = num(val)?,
            "stretch" => self.stretch = num(val)?,
            "len_max" => self.len_max = num(val)?,
            "bounce" => self.bounce = flag()?,
            _ => {
                return Err(format!(
                    "no knob `{key}` (knobs: {})",
                    Recipe::KEYS.join(", ")
                ))
            }
        }
        Ok(())
    }
}

fn pair(v: &[f32]) -> (f32, f32) {
    (v[0], v[1])
}

fn quad(v: &[f32]) -> [f32; 4] {
    [v[0], v[1], v[2], v[3]]
}

/// A recipe in knob syntax: `orient=stretch;cell=1;…`, every knob, so it
/// pastes back through [`knobs`] (prefixed with its layer) or into code.
impl fmt::Display for Recipe {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let orient = match self.orient {
            Orient::Billboard => "billboard",
            Orient::Stretch => "stretch",
            Orient::Flat => "flat",
        };
        let [a, b, c, d] = self.c0;
        let [e, g, h, i] = self.c1;
        write!(
            f,
            "orient={orient};cell={};cells={};frames={};tone={};glow={};speed={},{};spray={};\
             up={};gravity={};drag={};life={},{};size={},{};grow={};c0={a},{b},{c},{d};\
             c1={e},{g},{h},{i};tinted={};spin={};stretch={};len_max={};bounce={}",
            self.cell,
            self.cells,
            self.frames,
            self.tone,
            self.glow,
            self.speed.0,
            self.speed.1,
            self.spray,
            self.up,
            self.gravity,
            self.drag,
            self.life.0,
            self.life.1,
            self.size.0,
            self.size.1,
            self.grow,
            self.tinted,
            self.spin,
            self.stretch,
            self.len_max,
            self.bounce,
        )
    }
}

/// One knob: `layer.key=val`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Knob<'a> {
    pub layer: &'a str,
    pub key: &'a str,
    pub val: &'a str,
}

/// The knobs in `s`: `Layer.key=val` separated by `;` (or newlines), blanks
/// ignored. A layer's name is matched by the game, case-insensitively by
/// convention.
pub fn knobs(s: &str) -> impl Iterator<Item = Result<Knob<'_>, String>> {
    s.split([';', '\n'])
        .map(str::trim)
        .filter(|k| !k.is_empty())
        .map(|k| {
            let (lhs, val) = k
                .split_once('=')
                .ok_or_else(|| format!("`{k}`: wants Layer.key=value"))?;
            let (layer, key) = lhs
                .trim()
                .split_once('.')
                .ok_or_else(|| format!("`{k}`: wants Layer.key=value"))?;
            Ok(Knob {
                layer: layer.trim(),
                key: key.trim(),
                val: val.trim(),
            })
        })
}

fn mix(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Throw `n` particles of recipe `r` from `at`, around `dir`, into `pool`:
/// `tint` (linear) multiplies a tinted recipe's colours, `scale` its size
/// (and its speed by `√scale`, so a bigger burst carries further without
/// flying apart). A bouncing recipe bounces once off the plane through `at`
/// facing `dir`, the surface it came off.
pub fn emit(pool: &mut Pool, r: &Recipe, n: usize, at: V3, dir: V3, tint: [f32; 3], scale: f32) {
    let tint = if r.tinted { tint } else { [1.0; 3] };
    let paint = |c: [f32; 4]| [c[0] * tint[0], c[1] * tint[1], c[2] * tint[2], c[3]];
    let dir = v3::norm_or(dir, v3::Y);
    let cells = r.cells.max(1);
    // A variant is the next cell, or the next flipbook along.
    let stride = r.frames.max(1);
    for _ in 0..n {
        let scatter = v3::norm_or([pool.signed(), pool.signed(), pool.signed()], v3::Y);
        let v = v3::norm_or(
            v3::add(v3::scale(dir, r.spray), v3::scale(scatter, 1.0 - r.spray)),
            dir,
        );
        let speed = mix(r.speed.0, r.speed.1, pool.roll());
        let life = mix(r.life.0, r.life.1, pool.roll());
        let size = mix(r.size.0, r.size.1, pool.roll()) * scale;
        let cell = r.cell.wrapping_add(
            ((pool.roll() * cells as f32) as u8)
                .min(cells - 1)
                .wrapping_mul(stride),
        );
        let roll = pool.roll() * core::f32::consts::TAU;
        let spin = pool.signed() * r.spin;
        let seed = pool.roll();
        pool.spawn(Particle {
            left: life,
            life,
            // A couple of centimetres off the surface, so nothing is born
            // inside it.
            pos: v3::add(at, v3::scale(v, 0.02)),
            vel: v3::add(
                v3::scale(v3::scale(v, speed), scale.sqrt()),
                v3::scale(v3::Y, r.up),
            ),
            size0: size,
            size1: size * r.grow,
            c0: paint(r.c0),
            c1: paint(r.c1),
            gravity: r.gravity,
            drag: r.drag,
            roll,
            spin,
            cell,
            orient: r.orient,
            stretch: r.stretch,
            len_max: r.len_max,
            plane_p: at,
            plane_n: if r.bounce { dir } else { v3::ZERO },
            frames: r.frames,
            seed,
            tone: r.tone,
            glow: r.glow,
        });
    }
}

/// `count` scaled by `lod`, never rounding a non-empty layer to nothing
/// while it is in range at all.
pub fn scaled(count: u8, lod: f32) -> usize {
    if count == 0 || lod <= 0.0 {
        0
    } else {
        ((count as f32 * lod).ceil() as usize).max(1)
    }
}
