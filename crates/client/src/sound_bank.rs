//! The bank the engine plays, cue by cue — **every cue a recording**: CC0
//! Freesound field recordings, Kenney's CC0 packs and the nine music pieces
//! rendered from the CC0 VSCO-2 orchestral samples (`assets/sound/MANIFEST.md`
//! lists every file and where it came from). The synthesizer
//! (`sound::synth`) is only the fallback for a recording that fails to
//! decode, and `tests/sound.rs` fails if any cue lacks one.
//!
//! A cue with several takes is one buffer of equal slices
//! (`synth::join_takes`): the renderer plays one slice per start and
//! `render/audio.rs::pump` picks which, never the one it played last. That is
//! Rust's own answer to a repeated sound (several recorded variations per
//! action); the pitch nudge (`Cue::pitch_var`) rides on top of it.
//!
//! The beds and the music are **whole files**: a bed is a seamless loop whose
//! last sample flows into its first, and a piece's downbeat is its first
//! sample and its length is the director's grid (`music::PIECE_S`), so
//! neither is trimmed or faded the way a take is.
//!
//! The recordings are decoded here at boot rather than shipped decoded: a few
//! MB of OGG against many MB of PCM, and the browser decodes the same bytes
//! with the same code. Not behind `render`, so `tests/sound.rs` checks every
//! take headless.

use sound::{synth, Cue, SAMPLE_RATE};

/// Five takes of one group from Kenney's *Impact Sounds*, compiled in.
macro_rules! five {
    ($name:literal) => {
        &[
            include_bytes!(concat!("../../../assets/sound/kenney/", $name, "_000.ogg")),
            include_bytes!(concat!("../../../assets/sound/kenney/", $name, "_001.ogg")),
            include_bytes!(concat!("../../../assets/sound/kenney/", $name, "_002.ogg")),
            include_bytes!(concat!("../../../assets/sound/kenney/", $name, "_003.ogg")),
            include_bytes!(concat!("../../../assets/sound/kenney/", $name, "_004.ogg")),
        ]
    };
}

/// Numbered takes of one recorded group, compiled in:
/// `takes!("freesound", "howl", 0 1)` is `freesound/howl_0.ogg` and
/// `howl_1.ogg`.
macro_rules! takes {
    ($dir:literal, $name:literal, $($n:literal)+) => {
        &[$(include_bytes!(concat!("../../../assets/sound/", $dir, "/", $name, "_", $n, ".ogg"))),+]
    };
}

/// One whole file (a bed's loop, a piece of music), compiled in.
macro_rules! whole {
    ($dir:literal, $name:literal) => {
        &[include_bytes!(concat!(
            "../../../assets/sound/",
            $dir,
            "/",
            $name,
            ".ogg"
        ))]
    };
}

/// Which recording plays which cue. One row per cue; `MANIFEST.md` lists the
/// files.
static RECORDED: &[(Cue, &[&[u8]])] = &[
    (
        Cue::StepSand,
        takes!("freesound", "step_sand", 0 1 2 3 4 5 6 7),
    ),
    (
        Cue::StepGrass,
        takes!("freesound", "step_grass", 0 1 2 3 4 5 6 7),
    ),
    (
        Cue::StepLitter,
        takes!("freesound", "step_litter", 0 1 2 3 4 5 6 7),
    ),
    (
        Cue::StepRock,
        takes!("freesound", "step_rock", 0 1 2 3 4 5 6 7),
    ),
    (Cue::StepWater, takes!("freesound", "step_water", 0 1 2 3 4)),
    (Cue::Howl, takes!("freesound", "howl", 0 1 2 3)),
    (Cue::Growl, takes!("freesound", "growl", 0 1 2 3 4 5)),
    (Cue::Hurt, takes!("freesound", "hurt", 0 1 2 3 4 5)),
    (Cue::Death, takes!("freesound", "death", 0 1 2 3)),
    (Cue::FleshHit, takes!("freesound", "flesh", 0 1 2 3 4 5)),
    (Cue::ImpactStone, five!("impactMining")),
    (Cue::ImpactWood, five!("impactWood_medium")),
    (Cue::ImpactMetal, five!("impactMetal_heavy")),
    (Cue::Place, five!("impactPlank_medium")),
    (Cue::BulletSoil, five!("impactSoft_medium")),
    (Cue::BulletStone, five!("impactGeneric_light")),
    (Cue::BulletWood, five!("impactWood_light")),
    (Cue::BulletMetal, five!("impactMetal_light")),
    (Cue::Knock, five!("impactWood_heavy")),
    (Cue::UiClick, takes!("kenney", "ui_click", 0)),
    (Cue::Hit, takes!("kenney", "hit", 0)),
    (Cue::HitHead, takes!("kenney", "hit_head", 0)),
    (Cue::HitLimb, takes!("kenney", "hit_limb", 0)),
    (Cue::Refused, takes!("kenney", "refused", 0)),
    (Cue::CraftDone, takes!("kenney", "craft_done", 0)),
    (Cue::Trade, takes!("kenney", "trade", 0)),
    (Cue::Learn, takes!("kenney", "learn", 0)),
    (Cue::DoorOpen, takes!("kenney", "door_open", 0 1)),
    (Cue::DoorClose, takes!("kenney", "door_close", 0 1 2 3)),
    (Cue::GateChime, takes!("freesound", "gate_chime", 0)),
    (Cue::SentryLock, takes!("freesound", "sentry_lock", 0)),
    (Cue::Unlock, takes!("freesound", "unlock", 0)),
    (Cue::Blast, takes!("freesound", "blast", 0 1 2)),
    (Cue::Collapse, takes!("freesound", "collapse", 0 1)),
    (Cue::TreeFall, takes!("freesound", "tree_fall", 0 1)),
    (Cue::Thunder, takes!("freesound", "thunder", 0 1 2)),
    (Cue::Splash, takes!("freesound", "splash", 0 1 2)),
    (Cue::Land, takes!("freesound", "land", 0 1 2 3)),
    (Cue::CollapseWood, takes!("freesound", "collapse_wood", 0 1)),
    (Cue::Swing, takes!("freesound", "swing", 0 1 2 3)),
    (Cue::Gather, takes!("kenney", "gather", 0 1 2)),
    (Cue::Eat, takes!("freesound", "eat", 0 1 2)),
    (Cue::Drink, takes!("freesound", "drink", 0 1 2)),
    (Cue::Bandage, takes!("freesound", "bandage", 0 1)),
    (Cue::BushPick, takes!("freesound", "bush_pick", 0 1 2)),
    (Cue::Snort, takes!("freesound", "snort", 0 1 2)),
    (Cue::Bird, takes!("freesound", "bird", 0 1 2 3 4 5)),
    (Cue::MusicOpenCalm, whole!("vsco", "music_open_calm")),
    (Cue::MusicOpenTense, whole!("vsco", "music_open_tense")),
    (Cue::MusicOpenCombat, whole!("vsco", "music_open_combat")),
    (Cue::MusicTurnCalm, whole!("vsco", "music_turn_calm")),
    (Cue::MusicTurnTense, whole!("vsco", "music_turn_tense")),
    (Cue::MusicTurnCombat, whole!("vsco", "music_turn_combat")),
    (Cue::MusicCloseCalm, whole!("vsco", "music_close_calm")),
    (Cue::MusicCloseTense, whole!("vsco", "music_close_tense")),
    (Cue::MusicCloseCombat, whole!("vsco", "music_close_combat")),
];

/// The cue whose bank a cue plays. The remote halves are the same boot on the
/// same ground and the same arm (`sound::synth` delegates them the same way),
/// so they share its takes.
pub fn source(cue: Cue) -> Cue {
    match cue {
        Cue::RemoteStepSand => Cue::StepSand,
        Cue::RemoteStepGrass => Cue::StepGrass,
        Cue::RemoteStepLitter => Cue::StepLitter,
        Cue::RemoteStepRock => Cue::StepRock,
        Cue::RemoteStepWater => Cue::StepWater,
        Cue::RemoteSwing => Cue::Swing,
        c => c,
    }
}

/// Is this cue played from a recording?
pub fn recorded(cue: Cue) -> bool {
    RECORDED.iter().any(|(c, _)| *c == source(cue))
}

/// Is this cue one whole file rather than takes: a looping bed or a piece of
/// music, neither of which may be trimmed or faded.
pub fn whole(cue: Cue) -> bool {
    cue.is_bed() || cue.is_music()
}

/// How many takes the synthesizer renders for a cue whose recording failed:
/// the sounds heard most often in a row get several, the rest one.
pub fn synth_takes(cue: Cue) -> u8 {
    match source(cue) {
        Cue::Ricochet => 4,
        Cue::ShotGun
        | Cue::ShotGunFar
        | Cue::ShotBow
        | Cue::StepSand
        | Cue::StepLitter
        | Cue::StepWater
        | Cue::Eat
        | Cue::BushPick => 3,
        Cue::Blast | Cue::Collapse => 2,
        _ => 1,
    }
}

/// A cue's bank entry and how many takes it holds.
///
/// A recording that fails to decode falls back to the synthesizer for the
/// whole cue — a bank that refuses to build is a client that refuses to boot
/// over a sound.
pub fn pcm(cue: Cue) -> (Box<[i16]>, u8) {
    if let Some(rec) = recording(cue) {
        return rec;
    }
    let src = source(cue);
    let n = synth_takes(src);
    if n == 1 {
        return (synth::pcm(src), 1);
    }
    let takes: Vec<Box<[i16]>> = (0..n).map(|t| synth::pcm_take(src, t)).collect();
    (synth::join_takes(&takes), n)
}

/// A cue's recording as the bank holds it, and how many takes — `None` if
/// it has none or any of its files fails to decode.
pub fn recording(cue: Cue) -> Option<(Box<[i16]>, u8)> {
    let src = source(cue);
    let (_, files) = RECORDED.iter().find(|(c, _)| *c == src)?;
    if whole(src) {
        return Some((decode_whole(files.first()?)?, 1));
    }
    let takes: Vec<Box<[i16]>> = files.iter().filter_map(|f| decode(f)).collect();
    (takes.len() == files.len() && !takes.is_empty())
        .then(|| (synth::join_takes(&takes), takes.len() as u8))
}

/// Silence under this fraction of the peak is trimmed off both ends.
const TRIM: f32 = 0.02;
/// Kept ahead of the first sound, seconds, so the attack is not cut.
const PRE_ROLL_S: f32 = 0.002;
/// Kept after the last sound, seconds, for the tail's fade.
const POST_ROLL_S: f32 = 0.012;

/// One OGG file as mono samples at the bank's rate, or `None` for a file that
/// does not decode or is not at the bank's rate.
fn mono(ogg: &[u8]) -> Option<Vec<f32>> {
    let mut r = lewton::inside_ogg::OggStreamReader::new(std::io::Cursor::new(ogg)).ok()?;
    let ch = r.ident_hdr.audio_channels as usize;
    if ch == 0 || r.ident_hdr.audio_sample_rate != SAMPLE_RATE {
        return None;
    }
    let mut mono: Vec<f32> = Vec::new();
    while let Some(pkt) = r.read_dec_packet_itl().ok()? {
        for fr in pkt.chunks_exact(ch) {
            let sum: i32 = fr.iter().map(|s| *s as i32).sum();
            mono.push(sum as f32 / (ch as f32 * 32_768.0));
        }
    }
    Some(mono)
}

/// One OGG take, as the bank holds it: mono at the bank's rate, trimmed,
/// faded at both ends like every synthesized cue, and normalized to the
/// bank's one peak so `CueDef::gain` alone decides how loud it plays. `None`
/// for a file that does not decode or is not at the bank's rate.
pub fn decode(ogg: &[u8]) -> Option<Box<[i16]>> {
    let mut mono = mono(ogg)?;
    let peak = mono.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    if peak <= 1e-4 {
        return None;
    }
    let loud = |v: &f32| v.abs() > peak * TRIM;
    let first = mono.iter().position(loud)?;
    let last = mono.iter().rposition(loud)?;
    let sr = SAMPLE_RATE as f32;
    let from = first.saturating_sub((PRE_ROLL_S * sr) as usize);
    let to = (last + (POST_ROLL_S * sr) as usize + 1).min(mono.len());
    let body = &mut mono[from..to];
    let n = body.len();
    let fade_in = ((0.0005 * sr) as usize).clamp(1, n / 4 + 1);
    let fade_out = ((0.004 * sr) as usize).clamp(1, n / 4 + 1);
    let k = synth::PEAK / peak;
    Some(
        body.iter()
            .enumerate()
            .map(|(i, v)| {
                let head = (i as f32 / fade_in as f32).min(1.0);
                let tail = ((n - i) as f32 / fade_out as f32).min(1.0);
                (v * k * head * tail).clamp(-1.0, 1.0) * i16::MAX as f32
            })
            .map(|v| v as i16)
            .collect(),
    )
}

/// One OGG file whole — a bed's loop or a piece — normalized to the bank's
/// peak and otherwise untouched: no trim, no fade, every sample where the
/// file put it.
pub fn decode_whole(ogg: &[u8]) -> Option<Box<[i16]>> {
    let mono = mono(ogg)?;
    let peak = mono.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    if peak <= 1e-4 {
        return None;
    }
    let k = synth::PEAK / peak;
    Some(
        mono.iter()
            .map(|v| ((v * k).clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
            .collect(),
    )
}
