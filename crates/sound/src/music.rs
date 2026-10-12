//! The music director: when a song plays and which one.
//!
//! **Music here is a gap system, not a soundtrack**, which is the reference
//! game's design (`reference/AUDIO.md` §8):
//!
//! - **Music is not continuous.** Their `music.songgapmin` /
//!   `music.songgapmax` default to **240 and 480 seconds** — four to eight
//!   minutes of silence between songs. Taken verbatim ([`SONG_GAP_MIN_S`],
//!   [`SONG_GAP_MAX_S`]).
//! - **The intensity is read at a boundary, never mid-song.** Bumps land
//!   whenever they land and decay when nothing happens; the director reads the
//!   tier only when a song is due to start.
//!
//! The score is four whole recorded tracks ([`Track`], `assets/music/`, listed
//! in `assets/sound/MANIFEST.md`), not the reference's clip-per-section
//! themes: [`INTRO`] opens a session — the menus loop it, and it carries on
//! into the world until it ends — and the three [`WORLD`] tracks take turns
//! after it, a gap apart, never the same one twice running. With whole
//! tracks the intensity decides two things: **a song never opens in the
//! middle of a fight** (the combat tier holds the gap until it calms down),
//! and a song that opens tense is [`TENSE_TRACK`].
//!
//! The tracks are streamed, never banked: five minutes of PCM is ~26 MB a
//! track, so they are read off disk natively (`render/audio_out.rs`) and
//! fetched by the page's own `<audio>` element in the browser
//! (`render/audio_web.rs`, `web/app.js`), outside the engine's bank and
//! its voice pool alike.
//!
//! Pure, allocation-free, no clock of its own: `dt_s` comes in and a track
//! comes out, so `crates/client/tests/sound.rs` drives a whole session's
//! worth of music headless in microseconds.

/// Shortest gap between two songs, seconds. The reference's
/// `music.songgapmin`, verbatim.
pub const SONG_GAP_MIN_S: f32 = 240.0;

/// Longest gap between two songs, seconds. The reference's
/// `music.songgapmax`, verbatim. They tuned the pair in both directions
/// across their devblogs, which is why it is a knob and not a derivation.
pub const SONG_GAP_MAX_S: f32 = 480.0;

/// How long after entering a world the first world song plays when nothing
/// is playing and the intro has already been heard this session, seconds.
/// Ours: a player who joins for three minutes should hear the score.
pub const FIRST_GAP_S: f32 = 30.0;

/// How often the intensity gets its chance to fall, seconds.
pub const DECAY_S: f32 = 8.0;

/// How much intensity falls at the end of a [`DECAY_S`] in which nothing
/// happened. One [`BUMP_HURT`] decays to calm over two quiet periods.
pub const DECAY: f32 = 0.34;

/// Intensity at or above which a song opens as [`Tier::Tense`].
pub const TENSE_AT: f32 = 0.30;
/// Intensity at or above which no song opens at all ([`Tier::Combat`]).
pub const COMBAT_AT: f32 = 0.70;

/// A tool swung — the reference's "a weapon equipped", the *small* bump.
pub const BUMP_SWING: f32 = 0.08;
/// You connected with something — their "a bullet passing your head", the
/// middle bump.
pub const BUMP_HIT: f32 = 0.22;
/// You took damage. **The biggest**, and theirs is too.
pub const BUMP_HURT: f32 = 0.55;

/// One recorded track of the score.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Track {
    /// The intro: the menus, the page's PLAY press, the first minutes.
    SafeZone = 0,
    /// A low, steady drone.
    GreenVeil = 1,
    /// Thin, high and wide; the uneasy one.
    RuinRespiration = 2,
    /// A sub-bass drone, flat from end to end.
    EmptyCitadel = 3,
}

/// The track that opens a session.
pub const INTRO: Track = Track::SafeZone;

/// The tracks a world plays, a gap apart.
pub const WORLD: [Track; 3] = [
    Track::GreenVeil,
    Track::EmptyCitadel,
    Track::RuinRespiration,
];

/// The track a song opens on when the intensity reads tense.
pub const TENSE_TRACK: Track = Track::RuinRespiration;

impl Track {
    /// Every track, in discriminant order.
    pub const ALL: [Track; 4] = [
        Track::SafeZone,
        Track::GreenVeil,
        Track::RuinRespiration,
        Track::EmptyCitadel,
    ];

    /// The file, relative to the asset root (`assets/`): what the native
    /// stream opens and the page fetches.
    pub fn file(self) -> &'static str {
        match self {
            Track::SafeZone => "music/safe_zone.ogg",
            Track::GreenVeil => "music/green_veil.ogg",
            Track::RuinRespiration => "music/ruin_respiration.ogg",
            Track::EmptyCitadel => "music/empty_citadel.ogg",
        }
    }

    /// The track's length, seconds: its last Ogg granule over 44.1 kHz.
    /// `tests/sound.rs` reads the files and holds these to them.
    pub fn len_s(self) -> f32 {
        self.samples() as f32 / 44_100.0
    }

    /// The track's length in samples at 44.1 kHz.
    pub fn samples(self) -> u64 {
        match self {
            Track::SafeZone | Track::EmptyCitadel => 10_585_728,
            Track::GreenVeil | Track::RuinRespiration => 13_231_872,
        }
    }

    /// The track whose [`Track::file`] is `file`, if any.
    pub fn of_file(file: &str) -> Option<Track> {
        Track::ALL.into_iter().find(|t| t.file() == file)
    }
}

/// How the intensity reads when a song is due.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Tier {
    Calm = 0,
    Tense = 1,
    Combat = 2,
}

impl Tier {
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

/// Where the music is playing.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Mode {
    /// In a world: gaps between songs, intensity from what happens.
    #[default]
    World,
    /// On the menus: the intro, looped, and nothing to be tense about. The
    /// reference ships a separate `audio.menumusicvolume`, so the menu
    /// having its own behaviour is theirs; that it is continuous is ours.
    Menu,
}

/// The music director. See the module docs.
pub struct Director {
    mode: Mode,
    /// The track the director last started and has not yet run out.
    playing: Option<Track>,
    /// Seconds left of [`Self::playing`].
    left_s: f32,
    /// Seconds until the next song, while nothing plays.
    gap_s: f32,
    /// The last track started, so the next is never the same one.
    last: Option<Track>,
    /// Has the intro started this session? A session opens on it once.
    intro_heard: bool,
    intensity: f32,
    /// Did anything bump during the running decay period?
    bumped: bool,
    decay_s: f32,
    /// The gap and pick stream. Deterministic and owned here, so a whole
    /// session's music is replayable in a test.
    rng: u32,
}

impl Default for Director {
    fn default() -> Self {
        Self::new(Mode::World)
    }
}

impl Director {
    /// A fresh session: nothing playing, and the intro starts on the first
    /// tick in either mode.
    pub fn new(mode: Mode) -> Self {
        Self {
            mode,
            playing: None,
            left_s: 0.0,
            gap_s: 0.0,
            last: None,
            intro_heard: false,
            intensity: 0.0,
            bumped: false,
            decay_s: DECAY_S,
            rng: 0x1BAD_C0DE,
        }
    }

    /// Put the director on a new screen's schedule — entering a world, or
    /// falling back to the menu.
    ///
    /// The intro carries across either way: a menu that is already playing
    /// it keeps it, and a world joined from the menu lets it finish before
    /// the first gap. A world track playing when the menu opens is dropped
    /// here and the intro asked for at once; the render layer fades the old
    /// one out under it. The random stream and the session's history
    /// survive.
    pub fn reset(&mut self, mode: Mode) {
        self.mode = mode;
        self.intensity = 0.0;
        self.bumped = false;
        self.decay_s = DECAY_S;
        match mode {
            Mode::Menu => {
                if self.playing != Some(INTRO) {
                    self.playing = None;
                    self.gap_s = 0.0;
                }
            }
            Mode::World => {
                if self.playing.is_none() {
                    self.gap_s = if self.intro_heard { FIRST_GAP_S } else { 0.0 };
                }
            }
        }
    }

    /// Take over a track something else started — the page starts the intro
    /// inside the PLAY press, before the game exists — `elapsed_s` into it.
    pub fn adopt(&mut self, track: Track, elapsed_s: f32) {
        self.playing = Some(track);
        self.left_s = (track.len_s() - elapsed_s.max(0.0)).max(0.0);
        self.last = Some(track);
        self.intro_heard |= track == INTRO;
    }

    /// Raise the intensity. Clamped at 1. Ignored on the menus, where there
    /// is nothing to be tense about.
    pub fn bump(&mut self, amount: f32) {
        if amount <= 0.0 || self.mode == Mode::Menu {
            return;
        }
        self.intensity = (self.intensity + amount).clamp(0.0, 1.0);
        self.bumped = true;
    }

    /// Where the intensity is, 0..1.
    pub fn intensity(&self) -> f32 {
        self.intensity
    }

    /// The tier a song starting now would read.
    pub fn tier(&self) -> Tier {
        Tier::of(self.intensity)
    }

    /// The track playing, if one is.
    pub fn playing(&self) -> Option<Track> {
        self.playing
    }

    /// Seconds until the next song may start (zero while one plays or is
    /// due).
    pub fn gap_s(&self) -> f32 {
        if self.playing.is_some() {
            0.0
        } else {
            self.gap_s.max(0.0)
        }
    }

    /// Advance the clocks. Returns a track on the frame one should start.
    pub fn tick(&mut self, dt_s: f32) -> Option<Track> {
        self.decay_s -= dt_s;
        if self.decay_s <= 0.0 {
            self.decay_s = DECAY_S;
            if !self.bumped {
                self.intensity = (self.intensity - DECAY).max(0.0);
            }
            self.bumped = false;
        }

        if self.playing.is_some() {
            self.left_s -= dt_s;
            if self.left_s > 0.0 {
                return None;
            }
            self.playing = None;
            let gap = match self.mode {
                Mode::Menu => 0.0,
                Mode::World => SONG_GAP_MIN_S + (SONG_GAP_MAX_S - SONG_GAP_MIN_S) * self.roll01(),
            };
            // The frame's overshoot past the song's end counts toward the
            // gap, so a looped intro on the menu starts the same frame the
            // last one ended.
            self.gap_s = gap + self.left_s;
        } else {
            self.gap_s -= dt_s;
        }
        if self.gap_s > 0.0 {
            return None;
        }
        let track = match self.mode {
            Mode::Menu => INTRO,
            Mode::World if !self.intro_heard => INTRO,
            Mode::World => match self.tier() {
                // Held, not dropped: the gap stays spent and the song opens
                // the moment the fight has decayed below combat.
                Tier::Combat => return None,
                Tier::Tense if self.last != Some(TENSE_TRACK) => TENSE_TRACK,
                _ => self.pick(),
            },
        };
        self.playing = Some(track);
        self.left_s = track.len_s();
        self.last = Some(track);
        self.intro_heard |= track == INTRO;
        Some(track)
    }

    /// A world track other than the last one.
    fn pick(&mut self) -> Track {
        let pool: [Track; 3] = WORLD;
        let start = (self.roll01() * pool.len() as f32) as usize % pool.len();
        (0..pool.len())
            .map(|i| pool[(start + i) % pool.len()])
            .find(|t| Some(*t) != self.last)
            .unwrap_or(pool[start])
    }

    /// xorshift32 in [0, 1).
    fn roll01(&mut self) -> f32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        (x >> 8) as f32 / (1u32 << 24) as f32
    }
}
