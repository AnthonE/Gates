//! Every tick of every agent on a lockstep island, for a model that plays
//! at frame rate (`gates-agent --bench N --warp --ticks DIR`, `ops/ticks/`).
//!
//! The mind picks a goal about once a second (`mind.rs`) and the scripted
//! body plays every tick in between. This records the body's side: on each
//! tick, for each seat, what its client knew (its own state, the coarse
//! sightings its mind is shown, the nearest bodies its eyes track), what the
//! goal's skill asked of the hands (the [`Intent`]), and the input frame and
//! action that went out. That is a teacher for a learned body the way
//! `distill.rs`'s rows are a teacher for a learned mind. Recorded with
//! `--record` beside it (one file per bot, in seat order), each decision
//! row's `tick` is this log's `seen` on the tick it was made, and the goal
//! it picked runs from the next tick: a goal's reason lines up with the
//! ticks it ran.
//!
//! **Only what the client received.** Every column is read off the seat's
//! `ClientView`, its `ClientCore` and the agent's own tracks and summary —
//! the same sources the body decides on — never the island's `World`.
//!
//! **Format `gates-ticks-1`.** `ticks.bin` holds fixed-size little-endian
//! records with no padding, one per seat per tick, in tick order. Its
//! sibling `ticks.json` names the record as a numpy structured dtype
//! (`[name, type, shape]`) and holds the tables its small integers index:
//! goals and actions (0 is none, `n` is the table's row `n − 1`), the
//! sighting kinds and the track species. The header is written when the
//! log opens and again, with the final tables and row count, when it ends.
//!
//! Angles stay on the wire's grid: a yaw is the `u16` the sim reads
//! (`yaw >> 8` of 256 steps, `intent::yaw_toward`'s convention), and a
//! track's `bearing` is the yaw toward it less this body's own, wrapped to
//! `i16`. Positions are metres; a track's `dx`/`dz`/`dy` are from this
//! body's feet.

use crate::agent::intent::{yaw_toward, Intent, Look};
use crate::agent::tracks::{Species, Track};
use crate::explorer::Survivor;
use crate::mind::{Goal, Sighting, Summary};
use crate::view::ClientView;
use protocol::ActionMsg;
use serde_json::{json, Value};
use sim_core::input::InputFrame;
use sim_core::movement::{POS_XZ_Q, POS_Y_Q};
use std::fmt::Write as _;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

pub const FORMAT: &str = "gates-ticks-4";

/// Bodies kept per record, nearest first.
pub const TRACKS: usize = 8;

/// The sightings each record keeps, in column order.
pub const SIGHTINGS: [&str; 9] = [
    "trees", "stone", "ore", "bushes", "hemp", "players", "humans", "animals", "water",
];

/// The resources each record places exactly, in column order: the nearest
/// of each kind at the agent's last census (`Senses::resources`), and the
/// open water its last probe found.
pub const RESOURCES: [&str; 6] = ["tree", "stone", "ore", "bush", "hemp", "water"];

/// "None" for a `u16` column.
pub const NONE16: u16 = u16::MAX;
/// "None" for a `u8` slot column.
pub const NONE8: u8 = u8::MAX;

const S: usize = SIGHTINGS.len();
const R: usize = RESOURCES.len();

/// The record, in order: name, numpy type, count (1 is a scalar).
pub const COLUMNS: &[(&str, &str, usize)] = &[
    // When and who.
    ("tick", "<u4", 1),
    ("seen", "<u4", 1),
    ("seat", "u1", 1),
    ("alive", "u1", 1),
    ("goal", "<u2", 1),
    ("goal_ticks", "<u4", 1),
    // Its own body.
    ("hp", "<u2", 1),
    ("hp_max", "<u2", 1),
    ("food", "<u2", 1),
    ("water", "<u2", 1),
    ("x", "<f4", 1),
    ("y", "<f4", 1),
    ("z", "<f4", 1),
    ("yaw", "<u2", 1),
    ("pitch", "u1", 1),
    ("grounded", "u1", 1),
    // What its mind is shown, coarse: count, nearest metres, bearing index
    // (`mind::BEARINGS`), one per `SIGHTINGS`.
    ("see_count", "u1", S),
    ("see_m", "u1", S),
    ("see_bearing", "u1", S),
    // The nearest bodies its eyes track.
    ("tr_species", "u1", TRACKS),
    ("tr_flags", "u1", TRACKS),
    ("tr_bearing", "<i2", TRACKS),
    ("tr_dist", "<f4", TRACKS),
    ("tr_dx", "<f4", TRACKS),
    ("tr_dz", "<f4", TRACKS),
    ("tr_dy", "<f4", TRACKS),
    ("tr_vx", "<f4", TRACKS),
    ("tr_vz", "<f4", TRACKS),
    ("tr_yaw", "<u2", TRACKS),
    ("tr_held", "<u2", TRACKS),
    // The nearest of each resource: known, its bearing (as a track's) and
    // how far, in metres.
    ("rs_here", "u1", R),
    ("rs_bearing", "<i2", R),
    ("rs_dist", "<f4", R),
    // What the skill asked of the hands.
    ("in_look", "u1", 1),
    ("in_yaw", "<u2", 1),
    ("in_dist", "<f4", 1),
    ("in_dy", "<f4", 1),
    ("in_move_x", "i1", 1),
    ("in_move_z", "i1", 1),
    ("in_travel", "<u2", 1),
    ("in_buttons", "u1", 1),
    ("in_sel", "u1", 1),
    // What went out.
    ("buttons", "u1", 1),
    ("f_yaw", "<u2", 1),
    ("f_pitch", "u1", 1),
    ("f_pitch_fine", "i1", 1),
    ("f_move_x", "i1", 1),
    ("f_move_z", "i1", 1),
    ("f_sel", "u1", 1),
    // What the scripted hands made of the same tick: the frame itself
    // when they drive, the expert's answer when a learned body does
    // (`body.rs`; the label DAgger trains on).
    ("t_buttons", "u1", 1),
    ("t_yaw", "<u2", 1),
    ("t_pitch", "u1", 1),
    ("t_move_x", "i1", 1),
    ("t_move_z", "i1", 1),
    ("t_sel", "u1", 1),
    ("action", "u1", 1),
    // Running counts, across every life: things received (each one's toast,
    // `+5 Wood`), the units of them, and deaths. A tick's reward is what
    // they grew by (`ops/ticks/rl.py`).
    ("awards", "<u4", 1),
    ("got", "<u4", 1),
    ("deaths", "<u2", 1),
];

/// `tr_species`: 0 is an empty row.
pub const SPECIES: [&str; 4] = ["none", "player", "pig", "wolf"];
/// `tr_flags`, by bit.
pub const TRACK_FLAGS: [&str; 8] = [
    "visible",
    "dead",
    "sleeping",
    "crouched",
    "wounded",
    "aiming_at_me",
    "lit",
    "teammate",
];
/// `in_look`.
pub const LOOKS: [&str; 4] = ["keep", "heading", "point", "body"];

fn type_bytes(t: &str) -> usize {
    t.as_bytes()
        .last()
        .and_then(|c| (*c as char).to_digit(10))
        .unwrap_or(0) as usize
}

/// Bytes in one record.
pub fn row_bytes() -> usize {
    COLUMNS.iter().map(|(_, t, n)| type_bytes(t) * n).sum()
}

/// Where column `name` starts in a record, and the width of one of its
/// values in bytes.
pub fn column(name: &str) -> Option<(usize, usize)> {
    let mut at = 0;
    for (n, t, k) in COLUMNS {
        if *n == name {
            return Some((at, type_bytes(t)));
        }
        at += type_bytes(t) * k;
    }
    None
}

/// Little-endian appends onto a reused record buffer.
struct Rec<'a>(&'a mut Vec<u8>);

impl Rec<'_> {
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    fn i8(&mut self, v: i8) {
        self.0.push(v as u8);
    }
    fn u16(&mut self, v: u16) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn i16(&mut self, v: i16) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn f32(&mut self, v: f32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
}

/// One tracked body as a record keeps it.
#[derive(Clone, Copy, Default)]
struct Near {
    species: u8,
    flags: u8,
    bearing: i16,
    dist: f32,
    dx: f32,
    dz: f32,
    dy: f32,
    vx: f32,
    vz: f32,
    yaw: u16,
    held: u16,
}

/// The wire yaw toward `(dx, dz)` less `yaw`, wrapped to signed.
fn bearing(dx: f32, dz: f32, yaw: u16) -> i16 {
    yaw_toward(dx, dz).wrapping_sub(yaw) as i16
}

/// Who wrote the log and on what island: the header's fixed half.
#[derive(Clone, Debug)]
pub struct Meta {
    pub seed: u64,
    pub variant: u64,
    pub arc_hour: u32,
    pub mind: String,
    pub seats: usize,
}

/// A run's tick log (`ticks.bin` + `ticks.json` in one directory).
pub struct TickLog {
    dir: PathBuf,
    out: BufWriter<File>,
    meta: Meta,
    buf: Vec<u8>,
    goals: Vec<String>,
    actions: Vec<String>,
    /// Per seat: the goal last written and its index, so a running goal
    /// costs no lookup.
    last_goal: Vec<Option<(Goal, u16)>>,
    scratch: String,
    rows: u64,
    error: Option<String>,
}

/// What a finished log holds.
#[derive(Clone, Debug)]
pub struct Written {
    pub dir: PathBuf,
    pub rows: u64,
    pub bytes: u64,
}

impl TickLog {
    /// Make `dir` (and its parents) and open a fresh log in it.
    pub fn open(dir: &Path, meta: Meta) -> Result<Self, String> {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let bin = dir.join("ticks.bin");
        let file = File::create(&bin).map_err(|e| format!("{}: {e}", bin.display()))?;
        let seats = meta.seats;
        let log = Self {
            dir: dir.to_path_buf(),
            out: BufWriter::with_capacity(1 << 20, file),
            meta,
            buf: Vec::with_capacity(row_bytes()),
            goals: Vec::new(),
            actions: Vec::new(),
            last_goal: vec![None; seats],
            scratch: String::with_capacity(256),
            rows: 0,
            error: None,
        };
        log.write_header()?;
        Ok(log)
    }

    /// The header as it stands.
    pub fn header(&self) -> Value {
        let dtype: Vec<Value> = COLUMNS
            .iter()
            .map(|(name, t, n)| {
                if *n == 1 {
                    json!([name, t])
                } else {
                    json!([name, t, [n]])
                }
            })
            .collect();
        json!({
            "format": FORMAT,
            "proto": protocol::PROTO_VER,
            "tick_hz": sim_core::limits::TICK_HZ,
            "seed": self.meta.seed,
            "variant": self.meta.variant,
            "arc_hour": self.meta.arc_hour,
            "mind": self.meta.mind,
            "seats": self.meta.seats,
            "rows": self.rows,
            "row_bytes": row_bytes(),
            "dtype": dtype,
            "goals": self.goals,
            "actions": self.actions,
            "sightings": SIGHTINGS,
            "resources": RESOURCES,
            "species": SPECIES,
            "track_flags": TRACK_FLAGS,
            "looks": LOOKS,
            "none16": NONE16,
            "none8": NONE8,
        })
    }

    fn write_header(&self) -> Result<(), String> {
        let path = self.dir.join("ticks.json");
        let tmp = self.dir.join("ticks.json.tmp");
        let text = serde_json::to_string_pretty(&self.header()).map_err(|e| e.to_string())?;
        std::fs::write(&tmp, text).map_err(|e| format!("{}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, &path).map_err(|e| format!("{}: {e}", path.display()))
    }

    fn goal_index(&mut self, seat: usize, goal: Option<Goal>) -> u16 {
        let Some(goal) = goal else {
            return 0;
        };
        if let Some(Some((g, i))) = self.last_goal.get(seat) {
            if *g == goal {
                return *i;
            }
        }
        let label = goal.label();
        let i = intern(&mut self.goals, label.as_str());
        if let Some(slot) = self.last_goal.get_mut(seat) {
            *slot = Some((goal, i));
        }
        i
    }

    fn action_index(&mut self, msg: Option<&ActionMsg>) -> u8 {
        let Some(msg) = msg else {
            return 0;
        };
        self.scratch.clear();
        let _ = write!(self.scratch, "{msg:?}");
        let end = self
            .scratch
            .find(|c: char| !c.is_ascii_alphanumeric() && c != '_')
            .unwrap_or(self.scratch.len());
        let name = self.scratch[..end].to_owned();
        intern(&mut self.actions, &name).min(u16::from(u8::MAX)) as u8
    }

    /// One seat's tick: `bot` and `view` as they stand after `frame` went
    /// out (the frame's perception has run), `teacher` the frame the
    /// scripted hands made of it, `action` if one went with it.
    #[allow(clippy::too_many_arguments)]
    pub fn row(
        &mut self,
        tick: u64,
        seat: usize,
        bot: &Survivor,
        view: &ClientView,
        player: u32,
        frame: &InputFrame,
        teacher: &InputFrame,
        action: Option<&ActionMsg>,
    ) {
        if self.error.is_some() {
            return;
        }
        let goal = self.goal_index(seat, bot.goal());
        let action = self.action_index(action);
        let mut buf = std::mem::take(&mut self.buf);
        fill(
            &mut buf, tick, seat, goal, action, bot, view, player, frame, teacher,
        );
        debug_assert_eq!(buf.len(), row_bytes(), "a record is its columns");
        if let Err(e) = self.out.write_all(&buf) {
            self.error = Some(format!("{}: {e}", self.dir.display()));
        }
        self.buf = buf;
        self.rows += 1;
    }

    /// Records written so far.
    pub fn rows(&self) -> u64 {
        self.rows
    }

    /// Flush the records and write the final header.
    pub fn finish(mut self) -> Result<Written, String> {
        if let Some(e) = self.error.take() {
            return Err(e);
        }
        self.out
            .flush()
            .map_err(|e| format!("{}: {e}", self.dir.display()))?;
        self.write_header()?;
        Ok(Written {
            dir: self.dir.clone(),
            rows: self.rows,
            bytes: self.rows * row_bytes() as u64,
        })
    }
}

/// One record into `buf` (cleared first): `bot` and `view` as they stand
/// after `frame` went out, the goal and action as indices into whatever
/// tables the caller keeps (the log's own, or a learned body's).
#[allow(clippy::too_many_arguments)]
pub fn fill(
    buf: &mut Vec<u8>,
    tick: u64,
    seat: usize,
    goal: u16,
    action: u8,
    bot: &Survivor,
    view: &ClientView,
    player: u32,
    frame: &InputFrame,
    teacher: &InputFrame,
) {
    let summary = bot.summary(view, player);
    let core = bot.core();
    let body = view.get(player);
    let me = body.map_or([0.0; 3], |b| {
        [
            b.qx as f32 * POS_XZ_Q,
            b.qy as f32 * POS_Y_Q,
            b.qz as f32 * POS_XZ_Q,
        ]
    });
    let my_yaw = body.map_or(0, |b| b.yaw);
    let alive = core.is_some_and(|c| !c.dead && c.hp > 0) && body.is_some_and(|b| !b.dead);

    let mut near = [Near::default(); TRACKS];
    let mut n = 0usize;
    if let Some(core) = core {
        let team = core.team();
        let mates = &team.members[..usize::from(team.count).min(team.members.len())];
        for t in bot.tracks().all().filter(|t| t.id != player) {
            let row = near_of(t, me, my_yaw, mates.iter().any(|m| m.id == t.id));
            // Keep the TRACKS nearest, sorted.
            let mut at = n;
            while at > 0 && near[at - 1].dist > row.dist {
                at -= 1;
            }
            if at >= TRACKS {
                continue;
            }
            let end = n.min(TRACKS - 1);
            let mut k = end;
            while k > at {
                near[k] = near[k - 1];
                k -= 1;
            }
            near[at] = row;
            n = (n + 1).min(TRACKS);
        }
    }

    let intent = bot.intent();
    let (look, look_yaw, look_dist, look_dy) = look_of(&intent, bot, me);

    buf.clear();
    let mut r = Rec(buf);
    r.u32(tick as u32);
    r.u32(view.newest_applied.unwrap_or_default());
    r.u8(seat.min(255) as u8);
    r.u8(u8::from(alive));
    r.u16(goal);
    r.u32(bot.goal_ticks().unwrap_or(0));
    r.u16(core.map_or(0, |c| c.hp));
    r.u16(core.map_or(0, |c| c.hp_max));
    r.u16(core.map_or(0, |c| c.food));
    r.u16(core.map_or(0, |c| c.water));
    r.f32(me[0]);
    r.f32(me[1]);
    r.f32(me[2]);
    r.u16(my_yaw);
    r.u8(body.map_or(0, |b| b.pitch));
    r.u8(u8::from(body.is_some_and(|b| b.grounded)));
    let sights = sightings(summary.as_ref());
    for s in &sights {
        r.u8(s.count);
    }
    for s in &sights {
        r.u8(s.nearest_m);
    }
    for s in &sights {
        r.u8(s.bearing);
    }
    for t in &near {
        r.u8(t.species);
    }
    for t in &near {
        r.u8(t.flags);
    }
    for t in &near {
        r.i16(t.bearing);
    }
    for t in &near {
        r.f32(t.dist);
    }
    for t in &near {
        r.f32(t.dx);
    }
    for t in &near {
        r.f32(t.dz);
    }
    for t in &near {
        r.f32(t.dy);
    }
    for t in &near {
        r.f32(t.vx);
    }
    for t in &near {
        r.f32(t.vz);
    }
    for t in &near {
        r.u16(t.yaw);
    }
    for t in &near {
        r.u16(t.held);
    }
    let senses = bot.senses();
    let mut places = [None; R];
    places[..5].copy_from_slice(&senses.resources);
    places[5] = bot.water_point();
    let rel = places.map(|p| {
        p.map(|[x, z]| {
            let (dx, dz) = (x - me[0], z - me[2]);
            (bearing(dx, dz, my_yaw), (dx * dx + dz * dz).sqrt())
        })
    });
    for p in &rel {
        r.u8(u8::from(p.is_some()));
    }
    for p in &rel {
        r.i16(p.map_or(0, |(b, _)| b));
    }
    for p in &rel {
        r.f32(p.map_or(0.0, |(_, d)| d));
    }
    r.u8(look);
    r.u16(look_yaw);
    r.f32(look_dist);
    r.f32(look_dy);
    r.i8(intent.move_x);
    r.i8(intent.move_z);
    r.u16(intent.travel.unwrap_or(NONE16));
    r.u8(intent.buttons);
    r.u8(intent.sel.unwrap_or(NONE8));
    r.u8(frame.buttons);
    r.u16(frame.yaw);
    r.u8(frame.pitch);
    r.i8(frame.pitch_fine);
    r.i8(frame.move_x);
    r.i8(frame.move_z);
    r.u8(frame.sel);
    r.u8(teacher.buttons);
    r.u16(teacher.yaw);
    r.u8(teacher.pitch);
    r.i8(teacher.move_x);
    r.i8(teacher.move_z);
    r.u8(teacher.sel);
    r.u8(action);
    r.u32(bot.stats.gather_awards.min(u64::from(u32::MAX)) as u32);
    r.u32(bot.gathered.iter().fold(0u32, |a, g| a.saturating_add(*g)));
    r.u16(bot.stats.deaths.min(u64::from(u16::MAX)) as u16);
}

/// `name`'s 1-based row in `table`, added if new.
fn intern(table: &mut Vec<String>, name: &str) -> u16 {
    let i = match table.iter().position(|g| g == name) {
        Some(i) => i,
        None => {
            table.push(name.to_owned());
            table.len() - 1
        }
    };
    (i + 1).min(usize::from(u16::MAX)) as u16
}

fn near_of(t: &Track, me: [f32; 3], my_yaw: u16, teammate: bool) -> Near {
    let (dx, dy, dz) = (t.pos[0] - me[0], t.pos[1] - me[1], t.pos[2] - me[2]);
    let flags = [
        t.visible,
        t.dead,
        t.sleeping,
        t.crouched,
        t.wounded,
        t.aiming_at_me,
        t.lit,
        teammate,
    ]
    .iter()
    .enumerate()
    .fold(0u8, |f, (bit, on)| f | (u8::from(*on) << bit));
    Near {
        species: match t.species {
            Species::Player => 1,
            Species::Pig => 2,
            Species::Wolf => 3,
        },
        flags,
        bearing: bearing(dx, dz, my_yaw),
        dist: (dx * dx + dz * dz).sqrt(),
        dx,
        dz,
        dy,
        vx: t.vel[0],
        vz: t.vel[2],
        yaw: t.yaw,
        held: t.held.unwrap_or(NONE16),
    }
}

/// The intent's look as a kind, the yaw it wants, and how far and how high
/// what it looks at is (0 for a heading).
fn look_of(intent: &Intent, bot: &Survivor, me: [f32; 3]) -> (u8, u16, f32, f32) {
    let toward = |p: [f32; 3]| {
        let (dx, dy, dz) = (p[0] - me[0], p[1] - me[1], p[2] - me[2]);
        (yaw_toward(dx, dz), (dx * dx + dz * dz).sqrt(), dy)
    };
    match intent.look {
        Look::Keep => (0, NONE16, 0.0, 0.0),
        Look::Heading(yaw) => (1, yaw, 0.0, 0.0),
        Look::Point(p) => {
            let (yaw, d, dy) = toward(p);
            (2, yaw, d, dy)
        }
        Look::Body { id, .. } => match bot.tracks().get(id) {
            Some(t) => {
                let (yaw, d, dy) = toward(t.pos);
                (3, yaw, d, dy)
            }
            None => (3, NONE16, 0.0, 0.0),
        },
    }
}

fn sightings(s: Option<&Summary>) -> [Sighting; S] {
    let Some(s) = s else {
        return [Sighting::NONE; S];
    };
    [
        s.trees,
        s.stone_nodes,
        s.ore_nodes,
        s.bushes,
        s.hemp,
        s.players,
        s.humans,
        s.animals,
        s.water_near,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_record_is_its_columns() {
        // Every type a column names has a width, and the header says it.
        for (name, t, n) in COLUMNS {
            assert!(type_bytes(t) > 0, "{name}: {t}");
            assert!(*n > 0, "{name}");
        }
        let names: std::collections::BTreeSet<_> = COLUMNS.iter().map(|c| c.0).collect();
        assert_eq!(names.len(), COLUMNS.len(), "column names are unique");
        assert_eq!(SIGHTINGS.len(), S);
    }

    #[test]
    fn interned_rows_are_one_based_and_stable() {
        let mut t = Vec::new();
        assert_eq!(intern(&mut t, "explore"), 1);
        assert_eq!(intern(&mut t, "gather_wood"), 2);
        assert_eq!(intern(&mut t, "explore"), 1);
        assert_eq!(t, ["explore", "gather_wood"]);
    }

    #[test]
    fn a_bearing_is_relative_to_the_body() {
        // Straight ahead is 0 whichever way the body faces.
        let yaw = yaw_toward(1.0, 0.0);
        assert_eq!(bearing(1.0, 0.0, yaw), 0);
        // Behind is half a turn.
        assert_eq!(bearing(-1.0, 0.0, yaw).unsigned_abs(), 0x8000);
    }
}
