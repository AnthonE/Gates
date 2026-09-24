//! Settings — the reference's options screen, at the size ours can be honest
//! about.
//!
//! **The shape is the reference's**: a category rail down the left with the
//! selected one blocked out in olive, a pane of rows on the right, each row a
//! label and a control. That is the frame the operator handed over, and it is
//! worth copying because it scales — the pane holds two rows today and thirty
//! later without the screen changing shape.
//!
//! **What is NOT copied is the row count.** The reference's GRAPHICS tab lists
//! shadow cascades, anisotropic filtering, parallax mapping and a dozen more
//! because it has a renderer with those switches. Ours has ten settings that
//! do something, and this screen shows ten settings — and the tenth is a
//! LADDER rather than a panel: `render/quality.rs` states the case for one
//! knob, which is that six independent toggles is six ways to build a frame
//! nobody has ever looked at. A category with nothing
//! behind it says so in a sentence instead of drawing greyed rows that imply
//! a feature exists — the same rule the HUD already obeys, where "a 0-max
//! meter is undrawn, not drawn empty", and the same rule the intro screen
//! obeys when it says why the shard list is empty rather than showing nothing.
//!
//! **Settings persist.** `crate::config` owns the file — the path (the
//! platform's config dir), the format (hand-rolled `key = value` TOML, the
//! server's `shard.toml` precedent), the `version` field for when a knob is
//! renamed, and the ignore-and-preserve policy for a key this build does not
//! know. This module owns the Bevy half: [`load`] runs once at plugin build,
//! before the first frame that applies anything, and [`save_on_change`]
//! writes on every real change rather than at exit — a crash must not cost
//! the player their settings, and a change is a click, not a hot path. What
//! comes off disk is sanitized HERE, beside the steppers' own bounds: a
//! hand-edited `fov_deg = 500` is clamped exactly where a clicked one would
//! be, so the file cannot reach a state the screen cannot. The footer names
//! the file, or says when nothing is being saved (a capture run, a box with
//! no config dir) — the screen does not quietly forget on the player's
//! behalf, and it does not quietly claim to remember either.
//!
//! **One thing this screen deliberately cannot do is touch the sim.** Every
//! field below changes how the client draws or how the mouse maps to a view
//! angle. None of them reaches `set_input`'s quantization — a "sensitivity"
//! that changed the wire's yaw resolution would be a client tuning what the
//! server predicts against, which is the quantize-both-sides law's exact
//! failure. Sensitivity scales the free-running radian yaw *before* it is
//! quantized, so both sides still agree bit for bit.

use bevy::prelude::*;
use bevy::window::{PresentMode, PrimaryWindow, WindowMode};

use super::quality;
use crate::config::{self, Ao, Persisted, Quality};
use crate::ui::servers::Favourites;

use super::rig::{EyeCam, FOV_DEG};
use super::screen::Screen;
use super::ui;

/// Bounds and steps for the two numeric settings (`DECISIONS.md` §open,
/// "settings v0"). The defaults themselves are not new numbers: the field of
/// view starts at the rig's shipped `FOV_DEG` and sensitivity starts at 1.0,
/// which is the identity against `input::MOUSE_RAD_PER_PX`.
pub const FOV_MIN_DEG: f32 = 60.0;
pub const FOV_MAX_DEG: f32 = 110.0;
pub const FOV_STEP_DEG: f32 = 5.0;
pub const SENS_MIN: f32 = 0.25;
pub const SENS_MAX: f32 = 3.0;
pub const SENS_STEP: f32 = 0.05;

/// Which preset a fresh install lands on, per target.
///
/// Native is the frame that shipped; a browser is what the page has been
/// drawing since it existed. Written beside [`quality::default_gfx`] rather
/// than derived from it because the two are a pair — the preset NAME the
/// screen shows and the VALUES the renderer gets — and `tests/quality.rs`
/// fails if they stop agreeing.
#[cfg(target_arch = "wasm32")]
pub const DEFAULT_PRESET: Quality = Quality::Low;
/// See the wasm32 arm.
#[cfg(not(target_arch = "wasm32"))]
pub const DEFAULT_PRESET: Quality = Quality::High;

/// Volume sliders run 0..1 in tenths — the reference's `audio.master`,
/// `audio.game` and `audio.ambience` are 0..1 convars and its options screen
/// is a row of sliders over the same range. A tenth is the smallest step a
/// player can hear as a step; a hundredth would be twenty clicks of nothing.
pub const VOL_STEP: f32 = 0.1;

/// The categories, in the reference's order. Every one is drawn whether or not
/// it has rows — the rail is a map of the game's surface, and a category that
/// vanished when it was empty would make the screen change shape as features
/// land.
pub const CATEGORIES: [&str; 7] = [
    "GAMEPLAY", "AUDIO", "SCREEN", "GRAPHICS", "CONTROLS", "SOCIAL", "KEYBINDS",
];

/// What the player can change. Client-side every one of them; see the header.
#[derive(Resource)]
pub struct Settings {
    pub fov_deg: f32,
    /// **The preset last picked, which is not the same thing as what the
    /// renderer is doing** — [`Self::gfx`] is. This field survives a player
    /// moving an individual row so that the QUALITY stepper has somewhere to
    /// step FROM, and so that an older build reading the file (which knows
    /// `quality` and none of the rows) lands on a frame somebody designed
    /// rather than on the default.
    pub quality: Quality,
    /// **What the renderer is actually asked for**, one field per row on the
    /// GRAPHICS tab. `render/quality.rs` owns the type, the preset table and
    /// the per-target clamp.
    pub gfx: quality::Gfx,
    /// A multiplier on `input::MOUSE_RAD_PER_PX`, not a replacement for it.
    pub sensitivity: f32,
    pub invert_look: bool,
    pub vsync: bool,
    /// Frames-per-second ceiling; 0 is uncapped. See [`MAX_FPS_LADDER`].
    pub max_fps: u16,
    pub fullscreen: bool,
    /// The audio buses, 0..1. The reference's `audio.master`, `audio.game`,
    /// `audio.ambience` and `audio.musicvolume` — `crate::sound::Mix` is what
    /// reads them, and `render/audio.rs` is the only thing that builds one.
    pub vol_master: f32,
    pub vol_game: f32,
    pub vol_ambience: f32,
    /// The reference's `audio.musicvolume`, which is the one bus that does
    /// not open at full — see [`crate::sound::MUSIC_DEFAULT`].
    pub vol_music: f32,
    /// Tell Discord what the player is doing (`crate::discord`).
    pub discord_presence: bool,
    /// Let that presence carry the shard's name and address, which is what
    /// makes Discord's **Ask to Join** appear. The one row on this screen
    /// that discloses something rather than setting a preference.
    pub discord_share_server: bool,
    /// Which rail row is selected.
    pub cat: usize,
    /// Where Esc returns to. Settings is reachable from the intro screen and
    /// from the Esc menu, and it has to go back where it came from — a screen
    /// that always returned to the menu would drop a player out of a live
    /// world for changing their field of view.
    pub back: Screen,
    /// Set when the pane has to be rebuilt. The same explicit flag `Menu`
    /// carries, for the same reason: `is_changed()` fires on the frame the
    /// resource is inserted, which would rebuild what `setup` just spawned.
    pub dirty: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            fov_deg: FOV_DEG,
            // The frame the client drew before tiers existed — and in a
            // browser, the frame the page has drawn since it existed.
            // Anything else as a default would be a visual change nobody
            // asked for, arriving as a side effect of a performance feature.
            // `quality::default_gfx` is the one owner of that per-target
            // choice; this pair must agree, and `tests/quality.rs` says so.
            quality: DEFAULT_PRESET,
            gfx: quality::default_gfx(),
            sensitivity: 1.0,
            invert_look: false,
            vsync: true,
            // **Uncapped** (operator, 2026-08-17, revising the same day's
            // "cant we cap thigns at like 60 fps?" with *"client can go faster
            // than 60 we cant stop that"*). The cap is a tool on this screen,
            // not a policy: a player who bought a 144 Hz panel did so on
            // purpose, and a default that quietly halves it reads as the game
            // feeling worse than it is.
            //
            // **What made that safe to choose was fixing the real problem
            // underneath it.** The reason a frame cap looked like the answer
            // was that drawing faster genuinely cost you input:
            // `ClientCore::set_input` overwrote, so a press that began and
            // ended between two 30 Hz ticks never reached the sim, and a frame
            // at 400 fps is 2.5 ms against a 33 ms tick. That is latched now
            // (`ClientCore::sticky_buttons`), gated at every framerate from 30
            // to 400 by `client-core/tests/input_sampling.rs`, so frame rate
            // and input fidelity are no longer traded against each other and
            // the ceiling can go back to being the player's choice.
            //
            // What is still true, and is why `limit_frames` stays: with vsync
            // off, a menu holding one still image is redrawn as fast as the
            // hardware allows. That is a real cost and now it has a switch.
            max_fps: 0,
            // **On** (operator, 2026-08-13). A survival game opens filling
            // the screen; the windowed default was Bevy's, never a choice.
            // Borderless rather than exclusive, which is what `apply_window`
            // already resolves this to — an alt-tab must not change a video
            // mode.
            //
            // Safe to move because this is a DEFAULT and not an assignment:
            // `load` only reaches it for a key the settings file does not
            // carry, so a player who already turned fullscreen off keeps it
            // off. The one path that takes the defaults wholesale is a
            // `--capture` run, and `render::mod` pins that one windowed on
            // purpose — a probe's frame size is the visual gate's unit.
            fullscreen: true,
            // The reference opens master and game at 1. Ours opens the bed
            // lower than either of them, but that belongs to the cue's own
            // gain rather than to a bus a player would then have to put back.
            vol_master: 1.0,
            vol_game: 1.0,
            vol_ambience: 1.0,
            // On: the whole path is dark without an application id, and at
            // this level it locates no machine and names no person.
            discord_presence: true,
            // **Off, and this one is a disclosure rather than a
            // preference** — the operator opened the door on condition the
            // player enables it (2026-08-16), and opt-in is what that means.
            discord_share_server: false,
            // **Not 1, and it is the reference's number rather than a
            // taste call**: their `audio.musicvolume` ships at 0.2 while
            // master and game ship at 1. A score at parity with footsteps
            // is a score players turn off.
            vol_music: crate::sound::MUSIC_DEFAULT,
            cat: 0,
            back: Screen::Menu,
            dirty: false,
        }
    }
}

/// Every setting a control can move, as one enum, so the click handler is a
/// match rather than five queries.
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Knob {
    /// The preset. Writes every row below it at once; the rows below then
    /// move off it one at a time, and this row reads CUSTOM when they have.
    Quality,
    Shadows,
    ShadowDistance,
    ShadowCascades,
    ShadowMapPx,
    Ambient,
    Smaa,
    Bloom,
    TreeLod,
    RenderScale,
    DiscordPresence,
    DiscordShareServer,
    Vsync,
    MaxFps,
    Fullscreen,
    Fov,
    Sensitivity,
    InvertLook,
    VolMaster,
    VolGame,
    VolAmbience,
    VolMusic,
}

impl Settings {
    /// Move a setting. `delta` is +1/-1 for a numeric row and ignored by a
    /// toggle, which flips.
    ///
    /// Clamping lives here rather than in the click handler so the bounds are
    /// testable without a window, and so a keyboard path added later cannot
    /// reach a different clamp than the mouse path.
    pub fn adjust(&mut self, knob: Knob, delta: i32) {
        match knob {
            // Steps along `Quality::LADDER` and STOPS at each end rather than
            // wrapping. A wrap would put a player who clicked once too often
            // on the far end of the ladder from where they were looking —
            // which for this knob means the frame changing completely on a
            // click that was meant to nudge it.
            //
            // **Stepping this row REWRITES every row below it**, which is
            // what makes it a preset rather than a label. It steps from the
            // preset the rows currently match if they match one, so a player
            // who moved a single row and then clicks + lands on the rung
            // above the one they were customizing rather than on the rung
            // above whatever they last pressed.
            Knob::Quality => {
                let at = self
                    .preset_name()
                    .or(Some(self.quality))
                    .and_then(|q| Quality::LADDER.iter().position(|l| *l == q))
                    .unwrap_or(Quality::LADDER.len() - 1) as i32;
                let want = (at + delta.signum()).clamp(0, Quality::LADDER.len() as i32 - 1);
                self.quality = Quality::LADDER[want as usize];
                self.gfx = quality::effective(quality::preset(self.quality));
            }
            Knob::RenderScale => {
                use super::render_scale::{RENDER_SCALE_MAX, RENDER_SCALE_MIN, RENDER_SCALE_STEP};
                self.gfx.render_scale = (i32::from(self.gfx.render_scale)
                    + delta.signum() * i32::from(RENDER_SCALE_STEP))
                .clamp(i32::from(RENDER_SCALE_MIN), i32::from(RENDER_SCALE_MAX))
                    as u8;
            }
            Knob::Shadows => self.gfx.shadows = !self.gfx.shadows,
            Knob::ShadowDistance => {
                self.gfx.shadow_m =
                    step_ladder(&quality::SHADOW_M_LADDER, self.gfx.shadow_m, delta);
            }
            // Clamped to what the TARGET can draw, not to a taste range: a
            // browser stops at one (`quality::max_cascades`), and a stepper
            // that let a player click past the ceiling would be offering a
            // number the engine silently discards — which is the exact bug
            // this slice exists to fix.
            Knob::ShadowCascades => {
                let want = self.gfx.cascades as i32 + delta.signum();
                self.gfx.cascades = want.clamp(1, quality::max_cascades() as i32) as usize;
            }
            Knob::ShadowMapPx => {
                let want = step_ladder(&quality::SHADOW_PX_LADDER, self.gfx.shadow_map_px, delta);
                self.gfx.shadow_map_px = want.min(quality::max_shadow_map_px());
            }
            Knob::Ambient => {
                let at = Ao::LADDER
                    .iter()
                    .position(|a| *a == self.gfx.ao)
                    .unwrap_or(0) as i32;
                let want = (at + delta.signum()).clamp(0, Ao::LADDER.len() as i32 - 1);
                self.gfx.ao = Ao::LADDER[want as usize];
            }
            Knob::Smaa => self.gfx.smaa = !self.gfx.smaa,
            Knob::Bloom => self.gfx.bloom = !self.gfx.bloom,
            Knob::TreeLod => {
                self.gfx.tree_lod_swap_m =
                    step_ladder(&quality::TREE_LOD_LADDER, self.gfx.tree_lod_swap_m, delta);
            }
            Knob::Vsync => self.vsync = !self.vsync,
            Knob::MaxFps => self.max_fps = step_fps(self.max_fps, delta),
            Knob::Fullscreen => self.fullscreen = !self.fullscreen,
            // **Turning presence off takes sharing with it**, and that is the
            // behaviour rather than a note beside it. A latent "share my
            // address" left true under an off master is how a player who
            // turns presence back on months later gets a disclosure they
            // last consented to under different circumstances.
            Knob::DiscordPresence => {
                self.discord_presence = !self.discord_presence;
                self.discord_share_server &= self.discord_presence;
            }
            // Sharing implies presence: a player who clicks the row that
            // says "so friends can Ask to Join" has asked for the thing that
            // carries it, and a toggle that silently does nothing because a
            // master switch above it is off is worse than one that turns the
            // master on.
            Knob::DiscordShareServer => {
                self.discord_share_server = !self.discord_share_server;
                self.discord_presence |= self.discord_share_server;
            }
            Knob::InvertLook => self.invert_look = !self.invert_look,
            Knob::Fov => {
                self.fov_deg =
                    (self.fov_deg + delta as f32 * FOV_STEP_DEG).clamp(FOV_MIN_DEG, FOV_MAX_DEG);
            }
            Knob::Sensitivity => {
                // Rounded to the step, not just clamped: repeated float adds
                // of 0.05 drift, and a screen that reads "1.00" while holding
                // 0.9999997 would eventually print "0.95" for a click of +.
                let steps = ((self.sensitivity + delta as f32 * SENS_STEP) / SENS_STEP).round();
                self.sensitivity = (steps * SENS_STEP).clamp(SENS_MIN, SENS_MAX);
            }
            // Same round-to-step as sensitivity, for the same reason: a
            // slider that reads "70%" while holding 0.6999998 eventually
            // prints the wrong tenth for a click of +.
            Knob::VolMaster => self.vol_master = step_vol(self.vol_master, delta),
            Knob::VolGame => self.vol_game = step_vol(self.vol_game, delta),
            Knob::VolAmbience => self.vol_ambience = step_vol(self.vol_ambience, delta),
            Knob::VolMusic => self.vol_music = step_vol(self.vol_music, delta),
        }
        self.dirty = true;
    }

    /// The defaults with one preset applied to **both** graphics halves.
    ///
    /// **It exists because `Settings { quality: Low, ..default() }` is a
    /// footgun now and was not before.** `quality` used to be the whole of
    /// the graphics state; it is a label on [`Self::gfx`] today, so that
    /// struct-update expression builds a settings object claiming LOW and
    /// drawing HIGH. Every caller that means "a player on this rung" wants
    /// this.
    pub fn with_preset(q: Quality) -> Self {
        Self {
            quality: q,
            gfx: quality::effective(quality::preset(q)),
            ..Self::default()
        }
    }

    /// The preset the graphics rows currently spell out exactly, if any.
    ///
    /// **`None` is CUSTOM**, and it is derived rather than tracked by a flag
    /// for the reason this screen already applies to its derived fact rows: a
    /// flag is a second copy of the truth, and the copy is what goes stale. A
    /// player who pulls a row off HIGH and then puts it back is on HIGH
    /// again, with nothing to reset.
    pub fn preset_name(&self) -> Option<Quality> {
        Quality::LADDER
            .into_iter()
            .find(|q| quality::effective(quality::preset(*q)) == self.gfx)
    }

    /// What the control on this row currently reads.
    fn value(&self, knob: Knob) -> String {
        match knob {
            Knob::Quality => match self.preset_name() {
                Some(q) => q.name().to_uppercase(),
                None => "CUSTOM".to_string(),
            },
            Knob::Shadows => on_off(self.gfx.shadows),
            Knob::ShadowDistance => format!("{:.0}", self.gfx.shadow_m),
            Knob::ShadowCascades => format!("{}", self.gfx.cascades),
            Knob::ShadowMapPx => format!("{}", self.gfx.shadow_map_px),
            Knob::Ambient => self.gfx.ao.name().to_uppercase(),
            Knob::Smaa => on_off(self.gfx.smaa),
            Knob::Bloom => on_off(self.gfx.bloom),
            Knob::RenderScale => format!("{}%", self.gfx.render_scale),
            Knob::TreeLod => format!("{:.0}", self.gfx.tree_lod_swap_m),
            Knob::Vsync => on_off(self.vsync),
            Knob::MaxFps => {
                if self.max_fps == 0 {
                    "UNCAPPED".to_string()
                } else {
                    format!("{}", self.max_fps)
                }
            }
            Knob::Fullscreen => on_off(self.fullscreen),
            Knob::DiscordPresence => on_off(self.discord_presence),
            Knob::DiscordShareServer => on_off(self.discord_share_server),
            Knob::InvertLook => on_off(self.invert_look),
            Knob::Fov => format!("{:.0}", self.fov_deg),
            Knob::Sensitivity => format!("{:.2}", self.sensitivity),
            Knob::VolMaster => pct(self.vol_master),
            Knob::VolGame => pct(self.vol_game),
            Knob::VolAmbience => pct(self.vol_ambience),
            Knob::VolMusic => pct(self.vol_music),
        }
    }

    /// The eight values that go to disk — everything a player set, none of
    /// the UI state (`cat`, `back`, `dirty` describe this run's screen, not
    /// the player's choices).
    fn persisted(&self) -> Persisted {
        Persisted {
            fov_deg: self.fov_deg,
            quality: self.quality,
            sensitivity: self.sensitivity,
            invert_look: self.invert_look,
            vsync: self.vsync,
            max_fps: self.max_fps,
            fullscreen: self.fullscreen,
            vol_master: self.vol_master,
            vol_game: self.vol_game,
            vol_ambience: self.vol_ambience,
            vol_music: self.vol_music,
            discord_presence: self.discord_presence,
            discord_share_server: self.discord_share_server,
            // Every row written out, always. `GfxFile`'s `None` means "the
            // file did not say", which is a state only an OLDER file (or a
            // test) can be in — the game always knows what it is drawing, so
            // the game always writes it. That is also what makes the
            // preset/row disagreement survivable: a player on HIGH who pulled
            // the shadows in gets both facts on disk, and neither is inferred
            // at the next boot.
            gfx: config::GfxFile {
                ao: Some(self.gfx.ao),
                smaa: Some(self.gfx.smaa),
                bloom: Some(self.gfx.bloom),
                shadows: Some(self.gfx.shadows),
                shadow_m: Some(self.gfx.shadow_m),
                shadow_cascades: Some(self.gfx.cascades as u8),
                shadow_map_px: Some(self.gfx.shadow_map_px as u16),
                tree_lod_m: Some(self.gfx.tree_lod_swap_m),
                render_scale: Some(self.gfx.render_scale),
            },
        }
    }

    /// A loaded file, sanitized against the SAME bounds the steppers
    /// enforce. `config::parse` already refused non-finite floats; the range
    /// and the step live here, beside the controls, so a hand-edited file
    /// cannot reach a value a hundred clicks cannot — `fov_deg = 500` loads
    /// as 110, and an off-step sensitivity is rounded exactly as a click
    /// would round it, because the screen prints the step's precision.
    fn from_persisted(p: Persisted) -> Self {
        let step = |v: f32, s: f32| (v / s).round() * s;
        Self {
            fov_deg: p.fov_deg.clamp(FOV_MIN_DEG, FOV_MAX_DEG),
            // No clamp owed: the parser only produces a value that is on the
            // ladder (`Quality::from_name` refuses anything else and keeps
            // the default), so unlike every numeric row above there is no
            // out-of-range state to sanitize here.
            quality: p.quality,
            gfx: gfx_from_file(p.quality, p.gfx),
            sensitivity: step(p.sensitivity, SENS_STEP).clamp(SENS_MIN, SENS_MAX),
            invert_look: p.invert_look,
            vsync: p.vsync,
            // **Floored, not snapped to a rung.** The bounds live beside the
            // stepper like every other row's, but the rule differs on purpose:
            // a value off the ladder is honoured as long as it is a rate a
            // person could mean, because unlike fov or a volume there is a
            // legitimate reason to hand-edit this one — a 165 Hz panel is not
            // on any ladder anybody would ship. What is refused is the range
            // below the floor, where a typo'd `max_fps = 1` would present as
            // the game having hung.
            max_fps: if p.max_fps == 0 {
                0
            } else {
                p.max_fps.max(MIN_FPS_CAP)
            },
            fullscreen: p.fullscreen,
            vol_master: step(p.vol_master, VOL_STEP).clamp(0.0, 1.0),
            vol_game: step(p.vol_game, VOL_STEP).clamp(0.0, 1.0),
            vol_ambience: step(p.vol_ambience, VOL_STEP).clamp(0.0, 1.0),
            vol_music: step(p.vol_music, VOL_STEP).clamp(0.0, 1.0),
            discord_presence: p.discord_presence,
            // **Sanitized, not just loaded.** A hand-edited file could carry
            // sharing on under presence off, which no sequence of clicks can
            // reach — and the pair is a consent, so the loader resolves it
            // the same way `adjust` does rather than honouring a state the
            // screen cannot show.
            discord_share_server: p.discord_share_server && p.discord_presence,
            ..Self::default()
        }
    }
}

/// **The preset, then whatever the file overrode** — the one place a settings
/// file's graphics half is resolved.
///
/// The order is what makes an old file upgrade for free: a file written
/// before the rows existed carries none of them, every field is `None`, and
/// the player gets exactly the preset column they had. A file written by this
/// build carries all eight and the preset contributes nothing.
///
/// Sanitizing is here beside the steppers, which is this module's standing
/// rule (`config::parse`'s own header says range work does not live there,
/// because two sanitizers drift). Three different rules, each for a reason:
///
///   * **Clamped to the ladder's ends but honoured in between** — the
///     distances. [`MAX_FPS_LADDER`]'s posture: a hand-edited `shadow_m =
///     120` is a distance a person could mean, and [`step_ladder`] steps off
///     it correctly. What is refused is the range outside, where the builder
///     that consumes it starts asserting.
///   * **Snapped to a rung** — the shadow map size, and this one is not
///     taste. `calculate_cascade` divides an integer cascade diameter by it
///     and notes that a power of two is what keeps the texel size exactly
///     representable; an off-power size makes shadow edges crawl as the
///     camera moves, which is a defect nobody would connect back to the file
///     they edited.
///   * **Clamped to what the TARGET can draw** — the cascade count, from
///     [`quality::max_cascades`] rather than from a number written here.
fn gfx_from_file(preset: Quality, f: config::GfxFile) -> quality::Gfx {
    let base = quality::effective(quality::preset(preset));
    let span = |ladder: &[f32], v: f32| v.clamp(ladder[0], ladder[ladder.len() - 1]);
    quality::Gfx {
        render_scale: f.render_scale.unwrap_or(base.render_scale).clamp(
            super::render_scale::RENDER_SCALE_MIN,
            super::render_scale::RENDER_SCALE_MAX,
        ),
        ao: f.ao.unwrap_or(base.ao),
        smaa: f.smaa.unwrap_or(base.smaa),
        bloom: f.bloom.unwrap_or(base.bloom),
        shadows: f.shadows.unwrap_or(base.shadows),
        shadow_m: span(
            &quality::SHADOW_M_LADDER,
            f.shadow_m.unwrap_or(base.shadow_m),
        ),
        cascades: (f.shadow_cascades.map(usize::from).unwrap_or(base.cascades))
            .clamp(1, quality::max_cascades()),
        shadow_map_px: nearest_px(
            f.shadow_map_px
                .map(usize::from)
                .unwrap_or(base.shadow_map_px),
        ),
        tree_lod_swap_m: span(
            &quality::TREE_LOD_LADDER,
            f.tree_lod_m.unwrap_or(base.tree_lod_swap_m),
        ),
    }
}

/// The rung of [`quality::SHADOW_PX_LADDER`] nearest `px`, then capped at what
/// this target can allocate. See [`gfx_from_file`] for why this one snaps
/// where the distances do not.
fn nearest_px(px: usize) -> usize {
    let best = quality::SHADOW_PX_LADDER
        .into_iter()
        .min_by_key(|rung| rung.abs_diff(px))
        .unwrap_or(1024);
    best.min(quality::max_shadow_map_px())
}

/// What is on the disk right now, held so [`save_on_change`] can tell a real
/// change from a rebuild flag — `dirty` is also set by picking a rail row,
/// which changes nothing worth a write. Only inserted when there is a file
/// to keep: a capture run never gets one (frames must not depend on the
/// box's config, so it loads nothing and saves nothing), and neither does a
/// box whose environment names no config dir.
#[derive(Resource)]
pub struct Disk {
    path: std::path::PathBuf,
    /// The loaded file's version — carried so a save under a newer file
    /// keeps its stamp (`config::serialize` takes the max).
    version: u32,
    /// Another build's keys, preserved verbatim (`crate::config`'s policy).
    unknown: Vec<String>,
    /// The sanitized view of what was loaded, NOT the raw file: an
    /// out-of-range value on disk is corrected in memory at load and on disk
    /// only at the player's next change — a boot must not write.
    written: Persisted,
    /// The starred shards as last written. Compared the same way `written`
    /// is, and for the same reason: a star is a click on a different screen
    /// entirely, so this file has two writers' worth of state in it and
    /// exactly one writer.
    written_favs: Vec<String>,
}

/// Read the settings file once, before the first frame that applies
/// anything. Missing or corrupt is the defaults, silently; no resolvable
/// path is the defaults with persistence off for the run (`None`).
pub fn load() -> (Settings, Favourites, Option<Disk>) {
    let Some(path) = config::settings_path() else {
        return (Settings::default(), Favourites::default(), None);
    };
    let loaded = config::load(&path, Settings::default().persisted());
    let settings = Settings::from_persisted(loaded.values);
    // Bounded and deduplicated where the star is, not where the file is —
    // `config::parse`'s own header says range work does not live there, and a
    // hand-edited list of a thousand ids must reach the browser as the same
    // thing a thousand clicks could have produced.
    let favs = Favourites::from_disk(loaded.favourites);
    let disk = Disk {
        path,
        version: loaded.version,
        unknown: loaded.unknown,
        written: settings.persisted(),
        written_favs: favs.ids().to_vec(),
    };
    (settings, favs, Some(disk))
}

/// Save on change, not on exit: a crash must not cost the player their
/// settings, and a change is a click on a menu screen, not a hot path — the
/// write is a few hundred bytes behind a change-detection early-out and a
/// field compare, so an idle frame pays one branch.
/// **One writer, two watched resources.** The settings screen owns eight
/// knobs and the server browser owns the favourite list, and both live in one
/// file — so the alternative was two systems serializing the whole document,
/// which is the shape where the last one to run silently drops the other's
/// change. `Browse` is read here rather than `Settings` growing a list,
/// because a favourite is not a knob and `Settings::adjust` has no arm that
/// could take one.
pub fn save_on_change(
    settings: Res<Settings>,
    browse: Res<super::screen::Browse>,
    disk: Option<ResMut<Disk>>,
) {
    let Some(mut disk) = disk else {
        return;
    };
    if !settings.is_changed() && !browse.is_changed() {
        return;
    }
    let now = settings.persisted();
    let favs = browse.favourites.ids();
    if now == disk.written && favs == disk.written_favs {
        return;
    }
    let text = config::serialize(&now, disk.version, favs, &disk.unknown);
    match config::save(&disk.path, &text) {
        Ok(()) => {
            disk.written = now;
            disk.written_favs = favs.to_vec();
        }
        // Warn and keep `written` as it was, so the next change retries.
        // Never a panic and never a dialog: a full disk must not cost the
        // player the fov they just picked, only its survival.
        Err(e) => warn!("gates: settings not saved - {e}"),
    }
}

/// The frame-rate ceilings this screen offers, ascending, with **0 last and
/// meaning uncapped**.
///
/// A ladder rather than a `+1 fps` stepper: the values a player actually wants
/// are their monitor's refresh rate or a clean divisor of it, and 210 clicks
/// to get from 30 to 240 is not a setting, it is a punishment. The rungs are
/// the common panel rates plus the two halves that matter (30 for a laptop on
/// battery, 120 for a 240 Hz panel at half).
///
/// **Uncapped is on the ladder and is not the default** (operator,
/// 2026-08-17). It has to be reachable — a player who bought a 360 Hz panel
/// did so on purpose, and vsync alone is the old behaviour — but it is the end
/// of the ladder rather than the start, because the thing it costs is a core
/// and a GPU spinning to redraw a menu that is not moving.
pub const MAX_FPS_LADDER: [u16; 6] = [30, 60, 120, 144, 240, 0];

/// The lowest cap the loader will honour from a file, frames per second.
///
/// It is the ladder's own bottom rung, so the screen cannot reach below it
/// either. Under about this the client stops reading as slow and starts
/// reading as hung — a hand-edited `max_fps = 1` would present as a bug
/// report about the game freezing, which is an expensive way to learn that a
/// setting was honoured too literally.
pub const MIN_FPS_CAP: u16 = MAX_FPS_LADDER[0];

/// One click of the FPS row: step along [`MAX_FPS_LADDER`], clamped at both
/// ends rather than wrapping. Wrapping would put "uncapped" one click below
/// 30, which is the single most surprising place to land by accident.
fn step_fps(v: u16, delta: i32) -> u16 {
    let at = MAX_FPS_LADDER
        .iter()
        .position(|&f| f == v)
        // A value the ladder does not carry (a hand-edited settings file, or
        // a rung removed by a later build) steps from the nearest rung at or
        // above it rather than resetting — the file said something and it is
        // not this function's place to discard it.
        .unwrap_or_else(|| {
            MAX_FPS_LADDER
                .iter()
                .position(|&f| f >= v && f != 0)
                .unwrap_or(MAX_FPS_LADDER.len() - 1)
        });
    let next = (at as i32 + delta).clamp(0, MAX_FPS_LADDER.len() as i32 - 1);
    MAX_FPS_LADDER[next as usize]
}

/// One click along a ladder of rungs, clamped at both ends rather than
/// wrapping — [`step_fps`]'s rule, for [`step_fps`]'s reason: a wrap puts the
/// far end of the ladder one click from where the player was looking.
///
/// **Phrased as "the next rung strictly past `v`" rather than as an index
/// step**, which is what makes a value the ladder does not carry behave. A
/// hand-edited settings file said something and it is not this function's
/// place to discard it, so a `shadow_m = 120` sits between two rungs — and
/// index-stepping from "the nearest rung at or above" would send `+` to 200,
/// skipping the 140 it is sitting just below. Searching past `v` sends it to
/// 140 and to 90, which is what a player watching the number expects.
///
/// The ascending-ladder assumption is the caller's, and `tests/quality.rs`
/// asserts it of every ladder this file steps. (It is why [`MAX_FPS_LADDER`]
/// keeps its own stepper: 0 means *uncapped* and sits LAST, so that ladder is
/// not ordered by value.)
fn step_ladder<T: Copy + PartialOrd>(ladder: &[T], v: T, delta: i32) -> T {
    let last = ladder.len() - 1;
    match delta.signum() {
        1 => ladder[ladder.iter().position(|r| *r > v).unwrap_or(last)],
        -1 => ladder[ladder.iter().rposition(|r| *r < v).unwrap_or(0)],
        _ => v,
    }
}

/// One click of a volume slider, rounded onto the step and clamped to 0..1.
fn step_vol(v: f32, delta: i32) -> f32 {
    let steps = ((v + delta as f32 * VOL_STEP) / VOL_STEP).round();
    (steps * VOL_STEP).clamp(0.0, 1.0)
}

fn pct(v: f32) -> String {
    format!("{:.0}%", v * 100.0)
}

fn on_off(b: bool) -> String {
    if b {
        "ON".to_string()
    } else {
        "OFF".to_string()
    }
}

/// What this target refuses of what the player asked for, in a sentence.
///
/// **Derived by diffing the choice against [`quality::effective`]**, never
/// written out per target — a hand-kept list of "what the browser cannot do"
/// is the mirror-goes-stale defect `CLAUDE.md` names three times, and this
/// screen would be the last place anybody looked for it. So a clamp that is
/// added to `effective` shows up here the same day, and a clamp that is
/// removed stops being claimed.
fn target_note(s: &Settings) -> String {
    let want = s.gfx;
    let got = quality::effective(want);
    let mut notes: Vec<String> = Vec::new();
    if got.cascades != want.cascades {
        notes.push(format!(
            "{} shadow cascade{} - the engine supports no more here",
            got.cascades,
            if got.cascades == 1 { "" } else { "s" }
        ));
    }
    if got.shadow_map_px != want.shadow_map_px {
        notes.push(format!("shadow map held at {}px", got.shadow_map_px));
    }
    if got.ao != want.ao {
        notes.push(format!("ambient occlusion {}", got.ao.name()));
    }
    if got.tree_lod_swap_m != want.tree_lod_swap_m {
        notes.push(format!(
            "far trees held out to {:.0} m",
            got.tree_lod_swap_m
        ));
    }
    if notes.is_empty() {
        "everything above is what the renderer gets".to_string()
    } else {
        notes.join(", ")
    }
}

/// One row of the pane. Three kinds, because three is what the settings we
/// actually have need — a toggle, a number with steppers, and a fact.
enum Row {
    Toggle(&'static str, Knob),
    Number(&'static str, Knob, &'static str),
    /// A read-only row: a label and a value nobody can change here. The
    /// keybind list is all of these, and so is anything the client fixes.
    Fact(&'static str, &'static str),
    /// A read-only row whose value is DERIVED from the settings rather than
    /// written beside them — what a knob one row up actually resolves to.
    /// A function pointer rather than a string so the screen cannot state a
    /// consequence the renderer does not have.
    FactOf(&'static str, fn(&Settings) -> String),
    /// A sentence, for a category with nothing in it.
    Note(&'static str),
}

/// The pane's contents, by category. This is the whole of what settings can
/// do, in one readable place.
fn rows(cat: usize) -> Vec<Row> {
    match CATEGORIES[cat] {
        "GAMEPLAY" => vec![Row::Note(
            "No gameplay options exist yet. Every rule this screen could relax is \
             the server's, and a client that could turn one off would be a client \
             deciding - which is the one thing it may never do.",
        )],
        "AUDIO" => vec![
            Row::Number("MASTER VOLUME", Knob::VolMaster, "of full"),
            Row::Number("GAME VOLUME", Knob::VolGame, "steps, tools, hits"),
            Row::Number("AMBIENCE VOLUME", Knob::VolAmbience, "wind, surf, birds"),
            Row::Number("MUSIC VOLUME", Knob::VolMusic, "the score"),
            // A fact rather than a dead slider, which is the rule this
            // screen already obeys: a category with nothing behind it says
            // so. There is no voice chat, so there is no voice slider.
            // **There is also only ONE music slider** where the reference
            // has two - it ships `audio.musicvolume` and
            // `audio.menumusicvolume` separately, and ours is one number for
            // both until somebody wants the menu louder than the world.
            Row::Fact("SOUND OCCLUSION", "Off - walls do not muffle sound yet"),
        ],
        "SCREEN" => vec![
            Row::Toggle("VSYNC", Knob::Vsync),
            Row::Number("FPS LIMIT", Knob::MaxFps, "frames per second"),
            Row::Toggle("FULLSCREEN", Knob::Fullscreen),
        ],
        // **The readout is DERIVED, not written out beside the knob.** Three
        // of these rows used to be `Row::Fact`s reading "SMAA, always on",
        // "SSAO medium, always on" and a fixed render distance — true when
        // nothing could change them and false the moment a tier could. A
        // screen that states what a setting does has to read the same table
        // the renderer does, or it becomes the most confidently wrong thing
        // in the game.
        "GRAPHICS" => vec![
            Row::Number("QUALITY", Knob::Quality, "a preset for the rows below"),
            Row::Toggle("SHADOWS", Knob::Shadows),
            Row::Number("  distance", Knob::ShadowDistance, "metres"),
            Row::Number("  cascades", Knob::ShadowCascades, "slices of it"),
            Row::Number("  shadow map", Knob::ShadowMapPx, "texels a side"),
            // **The row that answers the question this slice was asked**
            // ("shadow stuff is kinda garbage with distance"), and it is
            // derived rather than written out because the answer is
            // arithmetic on the three rows above it — see
            // `quality::far_texel_m`. A shadow edge cannot be finer than one
            // texel, so this is the size of the stair-step at the far end of
            // the last cascade, in centimetres of world.
            Row::FactOf("  detail at the far edge", |s| {
                let g = quality::effective(s.gfx);
                if !g.shadows {
                    return "no shadows".to_string();
                }
                format!(
                    "{:.0} cm per texel at {:.0} m  -  {:.0} MiB",
                    quality::far_texel_m(g, s.fov_deg) * 100.0,
                    g.shadow_m,
                    quality::shadow_vram_mb(g),
                )
            }),
            Row::Number("AMBIENT OCCLUSION", Knob::Ambient, "contact shading"),
            Row::Toggle("ANTI-ALIASING (SMAA)", Knob::Smaa),
            Row::Toggle("BLOOM", Knob::Bloom),
            Row::Number("FAR TREES", Knob::TreeLod, "metres to the hull"),
            Row::Number(
                "RENDER SCALE",
                Knob::RenderScale,
                "world resolution; HUD stays sharp",
            ),
            // **What this TARGET does with the rows above, where it differs.**
            // A setting that silently does nothing is worse than one that is
            // honestly refused — `CLAUDE.md`'s own trap list says a path
            // switched off for one target reads as handled, and the browser
            // shadow bug this slice fixes was exactly that: two cascades
            // asked for, one drawn, a `warn!` into a console nobody reads.
            // Absent on the desktop, where nothing is clamped.
            Row::FactOf("  on this machine", target_note),
            Row::Number("FIELD OF VIEW", Knob::Fov, "vertical degrees"),
            // Still fixed, and still a fact: the rings decide which tiles
            // EXIST, so moving one is a streaming change rather than a draw
            // change — and `ART.md` rule 4 is a floor a tier may not cross.
            Row::Fact(
                "RENDER DISTANCE",
                "160 m near ring, 2 km far mesh - fixed by the streaming budget",
            ),
        ],
        "CONTROLS" => vec![
            Row::Number("MOUSE SENSITIVITY", Knob::Sensitivity, "x base"),
            Row::Toggle("INVERT LOOK", Knob::InvertLook),
        ],
        "SOCIAL" => vec![
            Row::Toggle("DISCORD STATUS", Knob::DiscordPresence),
            Row::Fact(
                "  what it says",
                "What you are doing and roughly where - never your address",
            ),
            Row::Toggle("SHOW MY SERVER", Knob::DiscordShareServer),
            Row::Fact(
                "  what it says",
                "The shard name and address, so friends can Ask to Join",
            ),
            // Said out loud on the screen that turns it on, because the cost
            // is not obvious from the label: a presence line is public to
            // everyone who can see the profile, not just to friends.
            Row::Fact(
                "  who can see it",
                "Anyone who can see your Discord profile - not only friends",
            ),
        ],
        "KEYBINDS" => BINDS.iter().map(|(k, v)| Row::Fact(k, v)).collect(),
        _ => vec![Row::Note("Nothing here yet.")],
    }
}

/// The binds, read off `input::gather`, `verbs::keys`, `panels::keys`,
/// `map::open`, `chat::keys`, `pause::keys`, `shot::take` and `report::keys`. Read-only: rebinding needs a
/// stored map and a conflict check, which is its own slice. Drawn anyway,
/// because **a bind the player is never told about is a bind that does not
/// exist** — the rule the intro screen's numbered rows already obey.
///
/// ⚠ **This array is a HAND-KEPT MIRROR of six systems and nothing gates it**
/// — `CLAUDE.md`'s own recurring defect, the shape of the `props.js` count and
/// the `pop_*` verb list. It had already drifted before this edit: it named
/// eight binds against roughly twenty in the code, so every in-world verb the
/// client has — `E`, the map, chat, the inventory — was absent from the only
/// screen that tells a player what the keys are. Growing it is the fix for
/// today; deriving it is the fix, and `tests/ui.rs` §H now at least fails if a
/// row here names a key no system reads.
///
/// **CROUCH is a sneak, and says so.** `BTN_CROUCH` changes nothing about how
/// the body moves; what reads it is the animal brain (`sim-core/src/brain.rs`),
/// where a crouched player is silent and is seen only inside an animal's sight
/// cone at half range — the reference's sneak-up-from-behind. The row states
/// that rather than implying a stance the player will go looking for.
pub const BINDS: [(&str, &str); 21] = [
    ("MOVE", "W A S D"),
    ("SPRINT", "Left Shift"),
    (
        "CROUCH",
        "Left Ctrl (sneak: animals cannot hear you, or see you from behind)",
    ),
    ("JUMP", "Space"),
    (
        "FREE LOOK",
        "Hold Left Alt (the head turns, the body does not)",
    ),
    ("LOOK", "Mouse (click to capture the pointer)"),
    ("USE / ATTACK", "Left Mouse"),
    ("INTERACT / OPEN", "E"),
    ("HOTBAR", "1 - 6, or the scroll wheel"),
    ("INVENTORY / CRAFTING", "Tab, I or Q  (Tab or Esc closes)"),
    ("MAP", "Hold G"),
    ("CHAT", "T or Enter"),
    ("EAT / DRINK", "J / H"),
    ("BUILD", "Hold Right Mouse with a plan; Left Mouse places"),
    (
        "LIGHT / SNUFF A TORCH",
        "Right Mouse with a torch in hand (it burns while it is lit)",
    ),
    ("REPAIR / UPGRADE", "R / U, or Left Mouse with a hammer"),
    (
        "CHANGE SKIN",
        "P over an item in the inventory, at a workbench (cycles the skins you own)",
    ),
    ("SCREENSHOT", "F12"),
    (
        "REPORT A BUG",
        "F7 (writes a file next to your screenshots)",
    ),
    ("MENU", "Esc"),
    ("QUIT", "Esc from the server list"),
];
/// Marks everything this screen owns.
#[derive(Component)]
pub struct SettingsRoot;

/// Marks the camera this screen had to spawn for itself, so a rebuild leaves
/// it alone. Respawning a camera on every click drops the frame it was
/// rendering.
#[derive(Component)]
pub struct SettingsCamera;

/// A rail row, by index into `CATEGORIES`.
#[derive(Component)]
pub struct Category(usize);

/// A control. `delta` is what a click applies: 0 for a toggle, +1/-1 for a
/// stepper.
#[derive(Component)]
pub struct Adjust {
    pub knob: Knob,
    pub delta: i32,
}

pub fn setup(
    mut commands: Commands,
    settings: Res<Settings>,
    disk: Option<Res<Disk>>,
    cameras: Query<(), With<Camera>>,
) {
    // Entered from the Esc menu there is a `Camera3d` up and the UI draws
    // against it; entered from the intro screen the menu's own camera went
    // with the menu. Two cameras would fight for the frame and Bevy would
    // warn, so this spawns one only when there is none.
    if cameras.is_empty() {
        commands.spawn((SettingsRoot, SettingsCamera, Camera2d));
    }
    build(&mut commands, &settings, disk.as_deref());
}

/// Rebuild after a click. Both the rail's selection and every drawn value are
/// derived from `Settings`, so the screen is respawned from it rather than
/// patched — a dozen nodes are cheaper to rebuild than to diff, and a patch
/// path is where a drawn value drifts from the held one.
pub fn rebuild(
    mut commands: Commands,
    mut settings: ResMut<Settings>,
    disk: Option<Res<Disk>>,
    roots: Query<Entity, (With<SettingsRoot>, Without<SettingsCamera>)>,
) {
    if !settings.dirty {
        return;
    }
    settings.dirty = false;
    for e in roots.iter() {
        commands.entity(e).despawn();
    }
    build(&mut commands, &settings, disk.as_deref());
}

/// The screen, as a plain function so `setup` and `rebuild` are provably the
/// same drawing — the shape `menu::build` already uses for the same reason.
fn build(commands: &mut Commands, settings: &Settings, disk: Option<&Disk>) {
    let back = match settings.back {
        Screen::Paused => "Esc goes back to the game",
        _ => "Esc goes back to the server list",
    };
    // The footer names the file, or says that nothing is being kept — the
    // honest states are "saved to <path>" and "not saved this run" (a
    // capture run, or a box whose environment names no config dir), and the
    // screen must not claim the wrong one.
    let saved = match disk {
        Some(d) => format!("saved to {}", d.path.display()),
        None => "settings are not being saved this run".to_string(),
    };

    commands
        .spawn((
            SettingsRoot,
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                ..default()
            },
            BackgroundColor(ui::BG),
        ))
        .with_children(|root| {
            root.spawn((
                ui::strong("SETTINGS", 30.0, ui::TITLE),
                Node {
                    margin: UiRect::new(Val::Px(34.0), Val::Px(0.0), Val::Px(26.0), Val::Px(16.0)),
                    ..default()
                },
            ));

            // The rail and the pane, side by side, both full height.
            root.spawn((Node {
                flex_direction: FlexDirection::Row,
                flex_grow: 1.0,
                column_gap: Val::Px(14.0),
                padding: UiRect::new(Val::Px(34.0), Val::Px(34.0), Val::Px(0.0), Val::Px(12.0)),
                ..default()
            },))
                .with_children(|body| {
                    // ---- the category rail -----------------------------------
                    body.spawn((
                        Node {
                            width: Val::Px(250.0),
                            flex_direction: FlexDirection::Column,
                            row_gap: Val::Px(2.0),
                            padding: UiRect::all(Val::Px(8.0)),
                            ..default()
                        },
                        BackgroundColor(ui::PANEL),
                    ))
                    .with_children(|rail| {
                        for (i, name) in CATEGORIES.iter().enumerate() {
                            let on = i == settings.cat;
                            let (idle, over) = if on {
                                (ui::ACCENT, ui::ACCENT_HOVER)
                            } else {
                                (ui::ROW_IDLE, ui::ROW_HOVER)
                            };
                            rail.spawn((
                                Button,
                                Category(i),
                                ui::Hover::new(idle, over),
                                Node {
                                    width: Val::Percent(100.0),
                                    padding: UiRect::axes(Val::Px(14.0), Val::Px(11.0)),
                                    border: UiRect::all(Val::Px(1.0)),
                                    ..default()
                                },
                                BackgroundColor(idle),
                                BorderColor::all(if on { ui::ACCENT } else { ui::RULE }),
                                children![ui::strong(
                                    *name,
                                    16.0,
                                    if on { ui::TEXT } else { ui::DIM }
                                )],
                            ));
                        }
                    });

                    // ---- the pane --------------------------------------------
                    body.spawn((
                        Node {
                            flex_grow: 1.0,
                            flex_direction: FlexDirection::Column,
                            row_gap: Val::Px(3.0),
                            padding: UiRect::all(Val::Px(14.0)),
                            overflow: Overflow::clip(),
                            ..default()
                        },
                        BackgroundColor(ui::PANEL),
                    ))
                    .with_children(|pane| {
                        for row in rows(settings.cat) {
                            spawn_row(pane, &row, settings);
                        }
                    });
                });

            root.spawn((
                ui::label(format!("{back}    -    {saved}"), 12.0, ui::FAINT),
                Node {
                    margin: UiRect::new(Val::Px(34.0), Val::Px(0.0), Val::Px(0.0), Val::Px(16.0)),
                    ..default()
                },
            ));
        });
}

/// One pane row. A function that SPAWNS rather than one that returns a bundle:
/// the four kinds are four different bundle types, and a match cannot return
/// four types from one `impl Bundle`.
fn spawn_row(
    pane: &mut bevy::ecs::relationship::RelatedSpawnerCommands<'_, ChildOf>,
    row: &Row,
    settings: &Settings,
) {
    // The label/value frame every row but a note shares.
    let frame = || Node {
        padding: UiRect::axes(Val::Px(12.0), Val::Px(8.0)),
        justify_content: JustifyContent::SpaceBetween,
        align_items: AlignItems::Center,
        column_gap: Val::Px(20.0),
        ..default()
    };

    match row {
        Row::Note(text) => {
            pane.spawn((
                Node {
                    padding: UiRect::axes(Val::Px(8.0), Val::Px(10.0)),
                    max_width: Val::Px(760.0),
                    ..default()
                },
                children![ui::label(*text, 14.0, ui::DIM)],
            ));
        }
        Row::Fact(label, value) => {
            pane.spawn((
                frame(),
                BackgroundColor(ui::ROW_IDLE),
                children![
                    ui::strong(*label, 15.0, ui::TEXT),
                    ui::label(*value, 13.0, ui::FAINT),
                ],
            ));
        }
        // The same row, with the value resolved against the settings the
        // pane was built from. `rebuild` re-spawns the pane on every change
        // (`Settings::dirty`), so these follow the knob above them.
        Row::FactOf(label, of) => {
            pane.spawn((
                frame(),
                BackgroundColor(ui::ROW_IDLE),
                children![
                    ui::strong(*label, 15.0, ui::TEXT),
                    ui::label(of(settings), 13.0, ui::FAINT),
                ],
            ));
        }
        Row::Toggle(label, knob) => {
            pane.spawn((
                frame(),
                BackgroundColor(ui::ROW_IDLE),
                children![
                    ui::strong(*label, 15.0, ui::TEXT),
                    (
                        Button,
                        Adjust {
                            knob: *knob,
                            delta: 0,
                        },
                        ui::Hover::default(),
                        Node {
                            width: Val::Px(92.0),
                            height: Val::Px(26.0),
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            border: UiRect::all(Val::Px(1.0)),
                            ..default()
                        },
                        BackgroundColor(ui::ROW_IDLE),
                        BorderColor::all(ui::RULE),
                        children![ui::strong(settings.value(*knob), 14.0, ui::TEXT)],
                    ),
                ],
            ));
        }
        Row::Number(label, knob, unit) => {
            pane.spawn((
                frame(),
                BackgroundColor(ui::ROW_IDLE),
                children![
                    ui::strong(*label, 15.0, ui::TEXT),
                    (
                        Node {
                            flex_direction: FlexDirection::Row,
                            align_items: AlignItems::Center,
                            column_gap: Val::Px(8.0),
                            ..default()
                        },
                        children![
                            ui::label(*unit, 12.0, ui::FAINT),
                            (
                                ui::stepper(),
                                Adjust {
                                    knob: *knob,
                                    delta: -1,
                                },
                                children![ui::strong("-", 16.0, ui::TEXT)],
                            ),
                            (
                                Node {
                                    // Wide enough for the longest value any
                                    // stepper produces - "UNCAPPED" on the
                                    // fps row, "MEDIUM" on ambient occlusion
                                    // - because the pane clips and a value
                                    // that overflowed would sit under the
                                    // "+" it belongs to.
                                    width: Val::Px(84.0),
                                    justify_content: JustifyContent::Center,
                                    ..default()
                                },
                                children![ui::strong(settings.value(*knob), 15.0, ui::TEXT)],
                            ),
                            (
                                ui::stepper(),
                                Adjust {
                                    knob: *knob,
                                    delta: 1,
                                },
                                children![ui::strong("+", 16.0, ui::TEXT)],
                            ),
                        ],
                    ),
                ],
            ));
        }
    }
}

pub fn click(
    cats: Query<(&Interaction, &Category), Changed<Interaction>>,
    controls: Query<(&Interaction, &Adjust), Changed<Interaction>>,
    mut settings: ResMut<Settings>,
) {
    for (interaction, cat) in cats.iter() {
        if *interaction == Interaction::Pressed {
            settings.cat = cat.0;
            settings.dirty = true;
        }
    }
    for (interaction, adjust) in controls.iter() {
        if *interaction == Interaction::Pressed {
            settings.adjust(adjust.knob, adjust.delta);
        }
    }
}

pub fn keys(
    keyboard: Res<ButtonInput<KeyCode>>,
    settings: Res<Settings>,
    mut next: ResMut<NextState<Screen>>,
) {
    if keyboard.just_pressed(KeyCode::Escape) {
        next.set(settings.back.clone());
    }
}

pub fn teardown(mut commands: Commands, roots: Query<Entity, With<SettingsRoot>>) {
    for e in roots.iter() {
        commands.entity(e).despawn();
    }
}

/// Field of view onto the camera. Runs in every state: the camera may not
/// exist yet (menu) and may be rebuilt later (a second world), and a setting
/// that only applied while the settings screen was open would be a setting
/// that forgot itself on the way out.
pub fn apply_view(settings: Res<Settings>, mut cam: Query<&mut Projection, With<EyeCam>>) {
    let Ok(mut projection) = cam.single_mut() else {
        return;
    };
    if let Projection::Perspective(p) = &mut *projection {
        let want = settings.fov_deg.to_radians();
        if (p.fov - want).abs() > f32::EPSILON {
            p.fov = want;
        }
    }
}

/// Present mode and window mode. Same reasoning as `apply_view`, and the same
/// change guard: writing `Window` every frame marks it changed every frame,
/// which makes Bevy's window backend do work for nothing.
pub fn apply_window(settings: Res<Settings>, mut window: Query<&mut Window, With<PrimaryWindow>>) {
    let Ok(mut w) = window.single_mut() else {
        return;
    };
    let present = if settings.vsync {
        PresentMode::AutoVsync
    } else {
        PresentMode::AutoNoVsync
    };
    if w.present_mode != present {
        w.present_mode = present;
    }
    let mode = if settings.fullscreen {
        WindowMode::BorderlessFullscreen(MonitorSelection::Current)
    } else {
        WindowMode::Windowed
    };
    if w.mode != mode {
        w.mode = mode;
    }
}

/// The frame deadline the limiter is pacing toward. `None` while uncapped.
///
/// A resource rather than a `Local` so a test can read it and so the reset on
/// a settings change has somewhere to live.
#[derive(Resource, Default)]
pub struct FrameDeadline(pub Option<bevy::platform::time::Instant>);

/// Hold the render loop to `Settings::max_fps`.
///
/// **Why this exists at all.** Bevy's focused update mode is
/// `UpdateMode::Continuous` — "over and over, as fast as it possibly can" —
/// and `WinitSettings::default()` is `game()`, so nothing but vsync stood
/// between this loop and the hardware. Vsync is a row on this very screen, so
/// turning it off uncapped the client, and a menu screen with one still image
/// on it would spin a core and a GPU at four figures. (The *unfocused* mode
/// was already `reactive_low_power(1/60)`, so a backgrounded client was never
/// the waste; a focused one looking at a menu was.)
///
/// **Deadline-based, not sleep-a-fixed-amount**, which is the same shape the
/// shard's tick loop uses (`server/src/net.rs`): advancing from the target
/// rather than from "now" keeps the average rate honest instead of drifting
/// slower by however much each sleep overshoots. `std::thread::sleep` is
/// granular to about a millisecond here, so a 60 cap measures 58-60 rather
/// than exactly 60 — and the alternative, spinning down the last millisecond,
/// burns the core this system exists to give back.
///
/// **It re-bases rather than catching up.** After a stall the deadline is
/// behind by more than a frame, and sprinting through uncapped frames to
/// repay that debt is precisely the behaviour a cap is for preventing. The
/// lost frames stay lost.
///
/// Runs in `Last` because a cap has to be the final thing a frame does; put
/// it earlier and it sleeps *before* the render it is supposed to be pacing.
pub fn limit_frames(settings: Res<Settings>, mut deadline: ResMut<FrameDeadline>) {
    use bevy::platform::time::Instant;
    use core::time::Duration;
    if settings.max_fps == 0 {
        // Uncapped: drop the deadline so re-enabling the cap starts from now
        // rather than from a stale instant, which would otherwise spend one
        // frame thinking it was hours behind.
        deadline.0 = None;
        return;
    }
    let frame = Duration::from_nanos(1_000_000_000 / settings.max_fps as u64);
    let now = Instant::now();
    let target = deadline.0.unwrap_or(now);
    // **The sleep is native-only, and its absence on web is not a gap.**
    // A browser paces the frame itself — the render loop is
    // `requestAnimationFrame`, which fires at the display's rate and cannot
    // be outrun — so there is no core to give back and nothing to cap. The
    // deadline arithmetic below still runs on both, because it is what keeps
    // `FrameDeadline` honest if a player moves the slider; only the blocking
    // call is conditional. Blocking a browser's main thread is also the one
    // thing a page must never do, so this is a refusal rather than a stub.
    #[cfg(not(target_arch = "wasm32"))]
    if now < target {
        std::thread::sleep(target - now);
    }
    let mut next = target + frame;
    let after = Instant::now();
    if next < after {
        next = after;
    }
    deadline.0 = Some(next);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two Discord rows are a **consent**, not two independent knobs,
    /// and the pair has one rule in both directions: sharing implies
    /// presence, and turning presence off withdraws sharing with it.
    ///
    /// The state this forbids is `share = true` under `presence = false` —
    /// a latent "publish my address" that no row on the screen is showing,
    /// waiting for the master to come back on months later and disclose
    /// something the player last agreed to under other circumstances.
    #[test]
    fn the_discord_pair_cannot_reach_a_latent_disclosure() {
        let mut s = Settings::default();
        // Shipped: presence on, sharing off.
        assert!(s.discord_presence);
        assert!(!s.discord_share_server);

        // Sharing on pulls presence with it, from either starting point.
        s.adjust(Knob::DiscordPresence, 0);
        assert!(!s.discord_presence);
        s.adjust(Knob::DiscordShareServer, 0);
        assert!(s.discord_share_server);
        assert!(s.discord_presence, "sharing implies presence");

        // Presence off withdraws sharing.
        s.adjust(Knob::DiscordPresence, 0);
        assert!(!s.discord_presence);
        assert!(!s.discord_share_server, "presence off withdraws sharing");

        // No sequence of clicks reaches the forbidden pair.
        let mut s = Settings::default();
        for i in 0..64 {
            s.adjust(
                if i % 3 == 0 {
                    Knob::DiscordPresence
                } else {
                    Knob::DiscordShareServer
                },
                0,
            );
            assert!(
                !(s.discord_share_server && !s.discord_presence),
                "reached a latent disclosure after {i} clicks"
            );
        }
    }

    /// …and a hand-edited settings file cannot reach it either, which is the
    /// half a click test cannot cover: the file is plain text a player can
    /// open, so the loader resolves the pair the same way `adjust` does.
    #[test]
    fn a_hand_edited_file_cannot_smuggle_the_latent_disclosure() {
        let mut p = Settings::default().persisted();
        p.discord_presence = false;
        p.discord_share_server = true;
        let s = Settings::from_persisted(p);
        assert!(
            !s.discord_share_server,
            "sharing under an off master must not load"
        );
    }

    #[test]
    fn a_stepper_cannot_walk_a_setting_out_of_range() {
        let mut s = Settings::default();
        for _ in 0..100 {
            s.adjust(Knob::Fov, 1);
        }
        assert_eq!(s.fov_deg, FOV_MAX_DEG);
        for _ in 0..100 {
            s.adjust(Knob::Fov, -1);
        }
        assert_eq!(s.fov_deg, FOV_MIN_DEG);
    }

    #[test]
    fn sensitivity_lands_on_its_step_after_a_walk() {
        // The drift this guards: 40 float adds of 0.05 do not sum to 2.0, and
        // the value is PRINTED to two decimals, so the drift would be visible
        // before it was large.
        let mut s = Settings::default();
        for _ in 0..40 {
            s.adjust(Knob::Sensitivity, 1);
        }
        for _ in 0..40 {
            s.adjust(Knob::Sensitivity, -1);
        }
        assert_eq!(s.value(Knob::Sensitivity), "1.00");
        assert!((s.sensitivity - 1.0).abs() < 1e-6, "{}", s.sensitivity);
    }

    #[test]
    fn a_toggle_ignores_the_delta_and_flips() {
        let mut s = Settings::default();
        assert!(s.vsync);
        s.adjust(Knob::Vsync, 0);
        assert!(!s.vsync);
        assert_eq!(s.value(Knob::Vsync), "OFF");
    }

    #[test]
    fn every_category_draws_something() {
        // The failure this catches is a category rail row that opens an empty
        // pane — the "dark panel that cannot say what would light it" this
        // repo has a rule about. An empty category must carry a NOTE.
        for (i, name) in CATEGORIES.iter().enumerate() {
            assert!(!rows(i).is_empty(), "{name} draws nothing");
        }
    }

    /// **Every row actually produces its value**, on every preset.
    ///
    /// `build` is the only thing that calls a `FactOf`'s function, and
    /// `build` needs a `Commands`. So a derived row that panicked — an index
    /// off a ladder, a divide by a zeroed map size — or that quietly resolved
    /// to nothing would reach the player and not a gate: `CLAUDE.md`'s "a
    /// spawn is a claim you have to run", one layer up. This runs the value
    /// side of every row in every category, which is what a settings screen
    /// mostly IS.
    #[test]
    fn every_row_can_state_its_value() {
        for q in Quality::LADDER {
            let mut s = Settings::with_preset(q);
            // …and once with shadows off, which is the branch the far-edge
            // readout short-circuits on.
            for shadows in [true, false] {
                s.gfx.shadows = shadows;
                for (cat, name) in CATEGORIES.iter().enumerate() {
                    s.cat = cat;
                    for row in rows(cat) {
                        let (label, value) = match row {
                            Row::Note(t) => ("note", t.to_string()),
                            Row::Fact(l, v) => (l, v.to_string()),
                            Row::FactOf(l, of) => (l, of(&s)),
                            Row::Toggle(l, k) | Row::Number(l, k, _) => (l, s.value(k)),
                        };
                        assert!(
                            !value.trim().is_empty(),
                            "{name} row {label:?} states nothing"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn the_defaults_are_the_rig_the_client_ships() {
        // A settings screen that opens on a value the renderer is not using
        // is a screen that lies on its first frame. FOV is the one setting
        // whose default is owned somewhere else.
        assert_eq!(Settings::default().fov_deg, FOV_DEG);
        assert_eq!(Settings::default().sensitivity, 1.0);
    }

    #[test]
    fn what_a_player_set_survives_a_round_trip() {
        // The whole slice in one assertion: walk every knob off its default,
        // serialize what would be written, parse it back over the defaults,
        // and the settings that come up are the settings that were set.
        let mut s = Settings::default();
        s.adjust(Knob::Fov, 2);
        s.adjust(Knob::Sensitivity, -3);
        s.adjust(Knob::InvertLook, 0);
        s.adjust(Knob::Vsync, 0);
        s.adjust(Knob::Fullscreen, 0);
        s.adjust(Knob::VolMaster, -2);
        s.adjust(Knob::VolGame, -5);
        s.adjust(Knob::VolAmbience, -10);
        let text = config::serialize(&s.persisted(), config::SETTINGS_VERSION, &[], &[]);
        let back =
            Settings::from_persisted(config::parse(&text, Settings::default().persisted()).values);
        assert_eq!(back.persisted(), s.persisted(), "{text}");
    }

    #[test]
    fn a_hand_edited_file_cannot_reach_what_a_hundred_clicks_cannot() {
        // `config::parse` already refused non-finite floats; this is the
        // range half, applied at load beside the steppers' own bounds. The
        // clamp is the same call `adjust` makes, so the two paths cannot
        // disagree about where the ends are.
        let wild = Persisted {
            fov_deg: 500.0,
            sensitivity: -2.0,
            vol_master: 7.0,
            vol_game: -0.4,
            // Off the step: the screen prints tenths, so the file loads tenths.
            vol_ambience: 0.44,
            ..Settings::default().persisted()
        };
        let s = Settings::from_persisted(wild);
        assert_eq!(s.fov_deg, FOV_MAX_DEG);
        assert_eq!(s.sensitivity, SENS_MIN);
        assert_eq!(s.vol_master, 1.0);
        assert_eq!(s.vol_game, 0.0);
        assert_eq!(s.value(Knob::VolAmbience), "40%");
    }

    /// The ladder clamps at both ends rather than wrapping, and UNCAPPED is
    /// only ever one click from the top rung.
    ///
    /// Wrapping is the tempting implementation and it is the wrong one here:
    /// it puts "uncapped" one click below 30, so a player easing the cap down
    /// on a laptop lands on the single setting that spins the fan hardest.
    #[test]
    fn the_fps_ladder_clamps_and_hides_uncapped_at_the_top() {
        assert_eq!(step_fps(30, -1), 30, "the bottom rung does not wrap");
        assert_eq!(step_fps(30, 1), 60);
        assert_eq!(step_fps(60, -1), 30);
        assert_eq!(step_fps(240, 1), 0, "uncapped is past the top rung");
        assert_eq!(step_fps(0, 1), 0, "and it is the end of the ladder");
        assert_eq!(step_fps(0, -1), 240, "stepping back off uncapped is a rate");
        // A rate no rung carries — a hand-edited file, or a rung a later
        // build dropped — steps from the nearest rung at or above it rather
        // than being discarded.
        assert_eq!(step_fps(144, 0), 144);
        assert_eq!(
            step_fps(100, -1),
            60,
            "an off-ladder rate still steps sanely"
        );
    }

    /// A file may set a rate the ladder does not carry — a 165 Hz panel is on
    /// nobody's ladder — but not one that reads as a hang.
    #[test]
    fn a_hand_edited_cap_is_honoured_unless_it_looks_like_a_freeze() {
        let odd = Persisted {
            max_fps: 165,
            ..Settings::default().persisted()
        };
        assert_eq!(
            Settings::from_persisted(odd).max_fps,
            165,
            "an off-ladder rate a real panel runs at must survive the loader"
        );
        let silly = Persisted {
            max_fps: 1,
            ..Settings::default().persisted()
        };
        assert_eq!(
            Settings::from_persisted(silly).max_fps,
            MIN_FPS_CAP,
            "one frame a second is indistinguishable from a hung client"
        );
        let off = Persisted {
            max_fps: 0,
            ..Settings::default().persisted()
        };
        assert_eq!(
            Settings::from_persisted(off).max_fps,
            0,
            "0 is uncapped, not a rate to be floored"
        );
    }

    /// The default is the operator's spoken UNCAPPED, and it is ON the ladder
    /// — a default off its own ladder would mean the first click of `-` jumped
    /// somewhere unrelated. (It is the ladder's last rung, so `-` steps to 240
    /// and `+` stays put, which is the intended shape: there is nothing above
    /// uncapped to reach for.)
    #[test]
    fn the_default_cap_is_uncapped_and_is_a_rung() {
        assert_eq!(Settings::default().max_fps, 0);
        assert!(MAX_FPS_LADDER.contains(&Settings::default().max_fps));
        assert_eq!(Settings::default().value(Knob::MaxFps), "UNCAPPED");
        let uncapped = Settings {
            max_fps: 0,
            ..Settings::default()
        };
        assert_eq!(uncapped.value(Knob::MaxFps), "UNCAPPED");
    }

    #[test]
    fn loading_the_defaults_changes_nothing() {
        // A fresh boot must be byte-for-byte the old behaviour: no file is
        // the defaults, and the defaults sanitized are still the defaults —
        // a default off its own step would mean every boot "changed" a value
        // and the first click wrote a file the player never asked for.
        let d = Settings::default().persisted();
        assert_eq!(Settings::from_persisted(d).persisted(), d);
    }

    /// The fresh-install pair agrees with itself.
    ///
    /// `DEFAULT_PRESET` is the NAME the QUALITY row shows and
    /// `quality::default_gfx` is what the renderer gets. They are written in
    /// two places because they are two different kinds of thing; if they ever
    /// disagreed, a fresh install would open the settings screen reading
    /// CUSTOM, which is the one value no default should ever show.
    #[test]
    fn a_fresh_install_reads_as_a_preset_and_not_as_custom() {
        let s = Settings::default();
        assert_eq!(
            s.preset_name(),
            Some(DEFAULT_PRESET),
            "the defaults resolve to {:?} while the QUALITY row would say {:?}",
            s.preset_name(),
            DEFAULT_PRESET
        );
        assert_eq!(s.value(Knob::Quality), DEFAULT_PRESET.name().to_uppercase());
    }

    /// **A settings file written before the graphics rows existed gets
    /// exactly the preset it had.**
    ///
    /// The whole upgrade path, in one assertion. `GfxFile::default()` is
    /// eight `None`s, which is what `config::parse` hands back for a file
    /// that never mentioned a row — and the answer has to be the preset
    /// column, not the row defaults, or every player with a settings file
    /// would have booted into a frame nobody chose.
    #[test]
    fn an_older_file_upgrades_to_the_preset_it_named() {
        for q in Quality::LADDER {
            assert_eq!(
                gfx_from_file(q, config::GfxFile::default()),
                quality::effective(quality::preset(q)),
                "a file that said only `quality = \"{}\"` must resolve to \
                 that preset exactly",
                q.name()
            );
        }
    }

    /// The upgrade end to end, down the path the game actually takes.
    ///
    /// `an_older_file_upgrades_to_the_preset_it_named` checks the resolver and
    /// `config`'s own suite checks the parser; this is the two joined, with
    /// the SAME defaults `load` passes — which is the argument that made the
    /// parser's half wrong in the first draft (`config::parse`'s comment).
    #[test]
    fn a_file_from_before_the_rows_boots_into_the_preset_it_named() {
        for q in Quality::LADDER {
            let text = format!("version = 1\nquality = \"{}\"\n", q.name());
            let loaded = config::parse(&text, Settings::default().persisted());
            let s = Settings::from_persisted(loaded.values);
            assert_eq!(
                s.gfx,
                quality::effective(quality::preset(q)),
                "a pre-rows file naming {q:?} booted into something else"
            );
            assert_eq!(
                s.preset_name(),
                Some(q),
                "…and the QUALITY row would not even say {q:?}"
            );
        }
    }

    /// A hand-edited file cannot reach a state the screen cannot.
    ///
    /// The three sanitizing rules, each tested at the value that would have
    /// hurt: a distance past the ladder (which `CascadeShadowConfigBuilder`
    /// would have had opinions about), a shadow map that is not a power of
    /// two (shadow edges crawl under a moving camera), and a cascade count
    /// past what this target draws (silently discarded by the engine, which
    /// is the bug this slice exists to fix).
    #[test]
    fn a_hand_edited_graphics_row_lands_where_a_click_could_have_put_it() {
        let wild = config::GfxFile {
            ao: Some(Ao::Ultra),
            smaa: Some(true),
            bloom: Some(true),
            shadows: Some(true),
            shadow_m: Some(5000.0),
            shadow_cascades: Some(99),
            shadow_map_px: Some(3000),
            tree_lod_m: Some(1.0),
            render_scale: Some(0),
        };
        let g = gfx_from_file(Quality::High, wild);
        assert_eq!(
            g.shadow_m,
            quality::SHADOW_M_LADDER[quality::SHADOW_M_LADDER.len() - 1]
        );
        assert_eq!(
            g.tree_lod_swap_m,
            quality::TREE_LOD_LADDER[0],
            "a tree swap under the ladder must land on its bottom rung"
        );
        assert!(
            quality::SHADOW_PX_LADDER.contains(&g.shadow_map_px)
                && g.shadow_map_px.is_power_of_two(),
            "3000 texels is not a rung and not a power of two; it resolved to \
             {}",
            g.shadow_map_px
        );
        assert!(
            g.cascades >= 1 && g.cascades <= quality::max_cascades(),
            "99 cascades resolved to {}, which this target cannot draw",
            g.cascades
        );
        // And a negative distance — the one a `-` in front of a number gets
        // you — is the bottom rung rather than a builder assertion.
        let low = gfx_from_file(
            Quality::High,
            config::GfxFile {
                shadow_m: Some(-1.0),
                ..config::GfxFile::default()
            },
        );
        assert_eq!(low.shadow_m, quality::SHADOW_M_LADDER[0]);
    }

    /// Every row the screen can move survives a trip through the file.
    ///
    /// Not a round trip of the defaults (`loading_the_defaults_changes_
    /// nothing` is that one) — this moves each row OFF its preset first, so a
    /// key that was never written, or was written and never read back, shows
    /// up as a row that reset itself at the next launch.
    #[test]
    fn every_graphics_row_survives_a_launch() {
        let mut s = Settings::with_preset(Quality::High);
        s.adjust(Knob::ShadowDistance, -1);
        s.adjust(Knob::ShadowCascades, -1);
        s.adjust(Knob::ShadowMapPx, -1);
        s.adjust(Knob::Ambient, 1);
        s.adjust(Knob::Smaa, 0);
        s.adjust(Knob::Bloom, 0);
        s.adjust(Knob::TreeLod, -1);
        s.adjust(Knob::Shadows, 0);
        assert_eq!(s.preset_name(), None, "the fixture moved nothing");

        let text = config::serialize(&s.persisted(), config::SETTINGS_VERSION, &[], &[]);
        let back =
            Settings::from_persisted(config::parse(&text, Settings::default().persisted()).values);
        assert_eq!(
            back.gfx, s.gfx,
            "a row did not survive the file:\nwrote {:?}\nread  {:?}\n{text}",
            s.gfx, back.gfx
        );
        assert_eq!(
            back.quality, s.quality,
            "the preset label must survive too — it is what an older build, \
             which knows `quality` and none of the rows, lands on"
        );
    }

    /// The "on this machine" row is DERIVED from the clamp, never written out.
    ///
    /// A hand-kept list of what a target refuses is the mirror-goes-stale
    /// defect `CLAUDE.md` names three times, and this screen would be the
    /// last place anybody looked for it. So: when nothing is clamped the row
    /// says so, and when something is it names the value the renderer
    /// actually got.
    #[test]
    fn the_target_note_reads_the_clamp_rather_than_a_list() {
        let plain = Settings::default();
        assert_eq!(
            quality::effective(plain.gfx),
            plain.gfx,
            "the shipped default must not be clamped on the target that \
             ships it — if it is, the default is a lie"
        );
        assert_eq!(
            target_note(&plain),
            "everything above is what the renderer gets"
        );

        // A value no stepper can reach, which is the only way to exercise
        // the clamp on the target that does not have one.
        let mut clamped = Settings::default();
        clamped.gfx.cascades = quality::max_cascades() + 1;
        let note = target_note(&clamped);
        assert!(
            note.contains(&format!("{}", quality::max_cascades())),
            "the note must name the count the renderer actually got, got \
             {note:?}"
        );
        assert_ne!(note, "everything above is what the renderer gets");
    }
    #[test]
    fn render_scale_survives_the_settings_file_and_sanitizes_its_bounds() {
        let mut s = Settings::default();
        s.gfx.render_scale = 75;
        let text = config::serialize(&s.persisted(), 1, &[], &[]);
        let loaded = config::parse(&text, Settings::default().persisted());
        let back = Settings::from_persisted(loaded.values);
        assert_eq!(back.gfx.render_scale, 75);
        assert_eq!(back.value(Knob::Quality), "CUSTOM");
        for (input, expected) in [(0, 50), (49, 50), (77, 77), (101, 100), (255, 100)] {
            let loaded = config::parse(&format!("render_scale = {input}\n"), s.persisted());
            assert_eq!(
                Settings::from_persisted(loaded.values).gfx.render_scale,
                expected
            );
        }
        for invalid in ["-1", "300", "NaN", "inf", "0.5", "nope"] {
            let loaded = config::parse(&format!("render_scale = {invalid}\n"), s.persisted());
            assert_eq!(
                Settings::from_persisted(loaded.values).gfx.render_scale,
                100
            );
        }
    }
}
