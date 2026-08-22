//! The music director: when a song plays, which piece plays next, and what
//! the fight has to do before the music notices.
//!
//! **Music here is a gap-and-intensity system, not a soundtrack**, which is
//! the reference game's design and is the most transferable thing in
//! `reference/AUDIO.md` (§8). Five findings, and every one of them is a rule
//! in this file:
//!
//! - **Music is not continuous.** Their `music.songgapmin` /
//!   `music.songgapmax` default to **240 and 480 seconds** — four to eight
//!   minutes of silence between songs. Taken verbatim
//!   ([`SONG_GAP_MIN_S`], [`SONG_GAP_MAX_S`]); `reference/BALANCE.md` §6 says
//!   a case is owed for *differing*, not for taking.
//! - **Themes hold pieces, not tracks.** A theme is divided into sections
//!   (verses and choruses); each section holds clips of differing intensity.
//!   [`PIECES`] is that table, and it is the whole of what a "song" is here:
//!   a walk through [`SECTIONS`], picking one clip per section.
//! - **Each piece carries an intensity level**, and the current in-game
//!   intensity picks which piece plays next. [`Tier`].
//! - **Pieces end with tails rather than looping cleanly**, so the system can
//!   cut from any piece to any other in any order without stopping playback
//!   and without fading — the tail covers the join. That is why a piece is
//!   [`SECTION_S`] of body plus [`TAIL_S`] of ring-out, why the next piece
//!   starts at the end of the *body*, and why `render/audio.rs` never
//!   despawns a music voice: it lets it finish under its successor.
//! - **Intensity changes only at section boundaries.** Bumps land whenever
//!   they land, but the tier is read once per boundary, so the music never
//!   lurches mid-phrase. If nothing happened during a section, intensity
//!   falls at the end of it — a musical boundary, not a timer.
//!
//! ## What is ours rather than theirs
//!
//! Three things, all in `DECISIONS.md` §open ("music v0"), and none of them
//! pretending to be measured:
//!
//! - **[`FIRST_GAP_S`]**, because a player who never hears the music cannot
//!   tell you it is wrong. Four to eight minutes is the gap *between* songs;
//!   opening a session with one is a different question and the reference
//!   does not answer it in anything published.
//! - **[`Mode::Menu`]**, where the gap is zero and the intensity is pinned
//!   calm. The reference ships `audio.menumusicvolume` as a *separate convar*
//!   from `audio.musicvolume`, so the menu having its own music behaviour is
//!   theirs; that it is continuous is ours.
//! - **The bumps** ([`BUMP_SWING`], [`BUMP_HIT`], [`BUMP_HURT`]). Their
//!   published *order* is taken exactly — a weapon in play is a small bump, a
//!   bullet past your head is bigger, taking damage is the biggest — and the
//!   magnitudes are ours because they published none.
//!
//! Pure, allocation-free, no clock of its own: `dt_s` comes in and a piece
//! comes out, so `crates/client/tests/sound.rs` drives a whole session's
//! worth of music headless in microseconds.

use super::Cue;

/// Shortest gap between two songs, seconds. The reference's
/// `music.songgapmin`, verbatim.
pub const SONG_GAP_MIN_S: f32 = 240.0;

/// Longest gap between two songs, seconds. The reference's
/// `music.songgapmax`, verbatim.
///
/// **They tuned these in both directions** across their own devblogs —
/// shortened when music was too rare, lengthened again when it played too
/// often — which is the best evidence available that the pair is a taste
/// knob rather than a derived number, and the reason ours are knobs too.
pub const SONG_GAP_MAX_S: f32 = 480.0;

/// How long after entering a world the first song plays, seconds.
///
/// **Ours, and the one number here that is a deliberate departure**
/// (`DECISIONS.md` §open, "music v0"). Drawing the first gap from the same
/// 240–480 s range would mean a player who joins, plays for three minutes and
/// leaves hears no music at all — including the operator, on the pass that is
/// supposed to decide whether the music is any good. The gap *between* songs
/// is the reference's number and is untouched.
pub const FIRST_GAP_S: f32 = 30.0;

/// How long a section's body is, seconds — and therefore how often the
/// director makes a decision.
///
/// One constant for both because they are one thing: a section IS a piece's
/// body, and the boundary where the next piece starts is the boundary where
/// the intensity is read. Twelve beats at 90 BPM (`synth::music`), so a piece
/// is a phrase rather than a fragment.
pub const SECTION_S: f32 = 8.0;

/// How long a piece rings out past its body, seconds.
///
/// This is the reference's "pieces end with reverb tails and delays rather
/// than looping cleanly", as arithmetic: the generator writes
/// `SECTION_S + TAIL_S` of audio and puts no note in the last [`TAIL_S`], so
/// what is left there is reverb. The next piece starts at the end of the body
/// and the tail plays under it, which is what makes a cut from any piece to
/// any other sound like a join instead of an edit.
pub const TAIL_S: f32 = 2.5;

/// How long one piece's audio is, seconds. What `synth::music` renders and
/// what `tests/sound.rs` checks it against.
pub const PIECE_S: f32 = SECTION_S + TAIL_S;

/// Sections in the theme.
pub const SECTIONS: usize = 3;

/// Intensity levels a section holds a clip for.
pub const TIERS: usize = 3;

/// Shortest song, in sections. Four sections is A-B-C-A: the theme has come
/// round, which is the shortest thing that reads as a song rather than as a
/// sting.
pub const SONG_SECTIONS_MIN: u32 = 4;
/// Longest song, in sections.
pub const SONG_SECTIONS_MAX: u32 = 7;

/// How much intensity falls at the end of a section in which nothing
/// happened.
///
/// Sized against the bumps rather than picked: one [`BUMP_HURT`] decays to
/// calm over two quiet sections (~16 s), and a sustained fight has to *keep*
/// producing events to hold the top tier. A decay that outran the bumps would
/// make the tiers unreachable; one that lagged them would leave the music in
/// combat over an empty forest.
pub const SECTION_DECAY: f32 = 0.34;

/// Intensity at or above which a section plays its [`Tier::Tense`] clip.
pub const TENSE_AT: f32 = 0.30;
/// Intensity at or above which a section plays its [`Tier::Combat`] clip.
pub const COMBAT_AT: f32 = 0.70;

/// A tool swung — the reference's "a weapon equipped or a helicopter around",
/// which is the *small* bump. Ours is the swing rather than the equip because
/// a swing is an event the client already produces (`render/input.rs`) and an
/// equipped-weapon state is not.
pub const BUMP_SWING: f32 = 0.08;
/// You connected with something — their "a bullet passing your head", the
/// middle bump. It is the nearest thing we have to *combat is happening*
/// until a projectile can miss you (`reference/PROJECTILES.md`).
pub const BUMP_HIT: f32 = 0.22;
/// You took damage. **The biggest**, and theirs is too.
pub const BUMP_HURT: f32 = 0.55;

/// Which clip of a section plays.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Tier {
    Calm = 0,
    Tense = 1,
    Combat = 2,
}

impl Tier {
    /// The tier's column in [`PIECES`].
    pub fn idx(self) -> usize {
        self as usize
    }

    /// The tier an intensity reads as.
    pub fn of(intensity: f32) -> Tier {
        if intensity >= COMBAT_AT {
            Tier::Combat
        } else if intensity >= TENSE_AT {
            Tier::Tense
        } else {
            Tier::Calm
        }
    }
}

/// The theme: three sections, three clips each, `[section][tier]`.
///
/// **This is the whole of what a theme is** — the reference's structure at
/// the size ours can honestly be. Their themes hold more sections and more
/// clips per section; the shape is the same and the count is a content
/// question, not an engineering one (`synth::music` generates all nine, and
/// swapping in recorded ones is a change to one function).
pub const PIECES: [[Cue; TIERS]; SECTIONS] = [
    [
        Cue::MusicOpenCalm,
        Cue::MusicOpenTense,
        Cue::MusicOpenCombat,
    ],
    [
        Cue::MusicTurnCalm,
        Cue::MusicTurnTense,
        Cue::MusicTurnCombat,
    ],
    [
        Cue::MusicCloseCalm,
        Cue::MusicCloseTense,
        Cue::MusicCloseCombat,
    ],
];

/// Which section and tier a cue is, if it is a piece at all.
///
/// A scan rather than a second table, so the two cannot drift: [`PIECES`] is
/// the one place the mapping is written and this reads it. Nine comparisons,
/// called once per piece generated at boot.
pub fn piece_of(cue: Cue) -> Option<(usize, Tier)> {
    for (s, row) in PIECES.iter().enumerate() {
        for (t, c) in row.iter().enumerate() {
            if *c == cue {
                return Some((s, [Tier::Calm, Tier::Tense, Tier::Combat][t]));
            }
        }
    }
    None
}

/// Where the music is playing.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Mode {
    /// In a world: gaps between songs, intensity from what happens.
    #[default]
    World,
    /// On the menus: no gaps, and nothing to be tense about. The reference
    /// ships a separate `audio.menumusicvolume`, so the menu having its own
    /// behaviour is theirs; that ours is continuous is ours.
    Menu,
}

/// A piece the director wants started.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Piece {
    pub cue: Cue,
    /// Which section of the theme this is — for tests and for a reader; the
    /// render layer only needs the cue.
    pub section: usize,
    pub tier: Tier,
}

/// The music director.
///
/// Two clocks, and the second one is the interesting half:
///
/// - `next_s` is time to the next *piece*, which is the end of the current
///   section's body while a song plays and the end of the gap while none
///   does. One clock for both because the two never overlap and the question
///   is the same: how long until something starts.
/// - `decay_s` is time to the intensity's next chance to fall, paced by
///   [`SECTION_S`] **whether or not a section is playing**. A fight during a
///   four-minute gap must not open the next song in combat five minutes
///   later, and the gap has no musical boundaries of its own to decay at.
pub struct Director {
    mode: Mode,
    next_s: f32,
    decay_s: f32,
    /// Sections of the current song still to start. **Zero is the gap**, and
    /// it is the only state flag there is.
    left: u32,
    /// Which section of the theme comes next.
    section: usize,
    intensity: f32,
    /// Did anything bump during the section now running? The reference's rule
    /// is that intensity falls only when nothing happened.
    bumped: bool,
    /// The gap and song-length stream. Deterministic and owned here rather
    /// than pulled from the OS — the house style (`mixer::Mixer::rng`,
    /// `synth`'s seeding, `voice::hash01`), and what makes a whole session's
    /// music replayable in a test.
    rng: u32,
}

impl Default for Director {
    fn default() -> Self {
        Self::new(Mode::World)
    }
}

impl Director {
    pub fn new(mode: Mode) -> Self {
        Self {
            mode,
            next_s: first_gap(mode),
            decay_s: SECTION_S,
            left: 0,
            section: 0,
            intensity: 0.0,
            bumped: false,
            rng: 0x1BAD_C0DE,
        }
    }

    /// Start over in `mode` — entering a world, or falling back to the menu.
    ///
    /// **The random stream survives**, deliberately: resetting it too would
    /// make every world in a session draw the identical gap sequence, which
    /// is the one thing determinism is not for. It is the same distinction
    /// `Steps::reset` draws — forget where the player was, not who they are.
    pub fn reset(&mut self, mode: Mode) {
        let rng = self.rng;
        *self = Self::new(mode);
        self.rng = rng;
    }

    /// Raise the intensity. Clamped at 1, and marks the running section as
    /// one in which something happened, which is what stops it decaying.
    ///
    /// Bumps land whenever they land; the *tier* is only read at a boundary
    /// ([`Self::tick`]), which is the reference's no-lurch rule.
    pub fn bump(&mut self, amount: f32) {
        if amount <= 0.0 {
            return;
        }
        self.intensity = (self.intensity + amount).clamp(0.0, 1.0);
        self.bumped = true;
    }

    /// Where the intensity is, 0..1. For tests and for a reader; nothing in
    /// the client draws it.
    pub fn intensity(&self) -> f32 {
        self.intensity
    }

    /// The tier a boundary right now would pick.
    pub fn tier(&self) -> Tier {
        Tier::of(self.intensity)
    }

    /// Is a song running? False in the gap.
    pub fn in_song(&self) -> bool {
        self.left > 0
    }

    /// Seconds until the next piece starts.
    pub fn next_in_s(&self) -> f32 {
        self.next_s
    }

    /// Advance the clocks. Returns a piece on the frame one should start.
    ///
    /// At most one piece per call by construction — the two clocks are
    /// reloaded from constants that are never zero, so no `dt` can cross two
    /// boundaries and no loop is needed. A hitch that swallowed a whole
    /// section costs that section, which is the `voice::Voices` rule
    /// (assign on fire, never bank).
    pub fn tick(&mut self, dt_s: f32) -> Option<Piece> {
        // The intensity's own clock, and it runs in the gap too.
        self.decay_s -= dt_s;
        if self.decay_s <= 0.0 {
            self.decay_s = SECTION_S;
            if !self.bumped {
                self.intensity = (self.intensity - SECTION_DECAY).max(0.0);
            }
            self.bumped = false;
        }

        self.next_s -= dt_s;
        if self.next_s > 0.0 {
            return None;
        }

        if self.left == 0 {
            // The gap ended. A song is however many sections it is, and it
            // opens on whatever the intensity has become — a fight during the
            // silence is exactly what should decide the first clip.
            self.left = self.draw_sections();
            self.section = 0;
            // The decay clock realigns to the song, so a section boundary and
            // a decay boundary are the same instant for the whole song. They
            // drift apart again in the gap, where there is no section to
            // align to.
            self.decay_s = SECTION_S;
            self.bumped = false;
        }

        let tier = self.tier();
        let section = self.section;
        let cue = PIECES[section][tier.idx()];
        self.section = (section + 1) % SECTIONS;
        self.left -= 1;
        // The gap is measured from the END of the song, so the last piece's
        // own body is added to it — `songgapmin` is a gap *between songs*.
        self.next_s = if self.left == 0 {
            SECTION_S + self.draw_gap()
        } else {
            SECTION_S
        };
        Some(Piece { cue, section, tier })
    }

    /// How long the next gap is, seconds.
    fn draw_gap(&mut self) -> f32 {
        match self.mode {
            // No gap on the menus: the next song follows the last one's body,
            // so the music is continuous and the tail still covers the join.
            Mode::Menu => 0.0,
            Mode::World => SONG_GAP_MIN_S + (SONG_GAP_MAX_S - SONG_GAP_MIN_S) * self.roll01(),
        }
    }

    /// How many sections the next song runs for, inclusive of both ends.
    fn draw_sections(&mut self) -> u32 {
        let span = SONG_SECTIONS_MAX - SONG_SECTIONS_MIN + 1;
        // `roll01` is never 1.0, so the cast lands in `0..span` — the `min` is
        // the belt for the day somebody widens that guarantee.
        SONG_SECTIONS_MIN + ((self.roll01() * span as f32) as u32).min(span - 1)
    }

    /// xorshift32 in [0, 1) — see [`Self::rng`].
    fn roll01(&mut self) -> f32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        // Top 24 bits: exact in f32, uniform, never 1.0 — `voice::hash01`'s
        // convention, so two draws in this module cannot disagree about what
        // "uniform" means.
        (x >> 8) as f32 / (1u32 << 24) as f32
    }
}

/// The gap a fresh director opens on.
fn first_gap(mode: Mode) -> f32 {
    match mode {
        Mode::Menu => 0.0,
        Mode::World => FIRST_GAP_S,
    }
}
