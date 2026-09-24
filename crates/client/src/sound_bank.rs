//! The bank the engine plays, cue by cue: recorded takes where the game has
//! them (Kenney's CC0 impact pack, `assets/sound/MANIFEST.md`), the
//! synthesizer's everywhere else (`sound::synth`).
//!
//! A cue with several takes is one buffer of equal slices
//! (`synth::join_takes`): the renderer plays one slice per start and
//! `render/audio.rs::pump` picks which, never the one it played last. That is
//! Rust's own answer to a repeated sound (several recorded variations per
//! action); the pitch nudge (`Cue::pitch_var`) rides on top of it.
//!
//! The recordings are decoded here at boot rather than shipped decoded: ~0.6 MB
//! of OGG against several MB of PCM, and the browser decodes the same bytes
//! with the same code. Not behind `render`, so `tests/sound.rs` checks every
//! take headless.

use sound::{synth, Cue, SAMPLE_RATE};

/// Five takes of one Kenney group, compiled in.
macro_rules! five {
    ($name:literal) => {
        [
            include_bytes!(concat!("../../../assets/sound/kenney/", $name, "_000.ogg")),
            include_bytes!(concat!("../../../assets/sound/kenney/", $name, "_001.ogg")),
            include_bytes!(concat!("../../../assets/sound/kenney/", $name, "_002.ogg")),
            include_bytes!(concat!("../../../assets/sound/kenney/", $name, "_003.ogg")),
            include_bytes!(concat!("../../../assets/sound/kenney/", $name, "_004.ogg")),
        ]
    };
}

/// Which recording plays which cue. One row per cue; `MANIFEST.md` lists the
/// files.
static RECORDED: [(Cue, [&[u8]; 5]); 12] = [
    (Cue::StepGrass, five!("footstep_grass")),
    (Cue::StepRock, five!("footstep_concrete")),
    (Cue::ImpactStone, five!("impactMining")),
    (Cue::ImpactWood, five!("impactWood_medium")),
    (Cue::ImpactMetal, five!("impactMetal_heavy")),
    (Cue::Place, five!("impactPlank_medium")),
    (Cue::BulletSoil, five!("impactSoft_medium")),
    (Cue::BulletStone, five!("impactGeneric_light")),
    (Cue::BulletWood, five!("impactWood_light")),
    (Cue::BulletMetal, five!("impactMetal_light")),
    (Cue::FleshHit, five!("impactPunch_medium")),
    (Cue::Knock, five!("impactWood_heavy")),
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

/// How many takes the synthesizer renders for a cue nothing was recorded for:
/// the sounds heard most often in a row get several, the rest one.
pub fn synth_takes(cue: Cue) -> u8 {
    match source(cue) {
        Cue::Ricochet => 4,
        Cue::ShotGun
        | Cue::ShotGunFar
        | Cue::ShotBow
        | Cue::StepSand
        | Cue::StepLitter
        | Cue::StepWater => 3,
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
    let src = source(cue);
    if let Some((_, files)) = RECORDED.iter().find(|(c, _)| *c == src) {
        let takes: Vec<Box<[i16]>> = files.iter().filter_map(|f| decode(f)).collect();
        if takes.len() == files.len() {
            return (synth::join_takes(&takes), takes.len() as u8);
        }
    }
    let n = synth_takes(src);
    if n == 1 {
        return (synth::pcm(src), 1);
    }
    let takes: Vec<Box<[i16]>> = (0..n).map(|t| synth::pcm_take(src, t)).collect();
    (synth::join_takes(&takes), n)
}

/// Silence under this fraction of the peak is trimmed off both ends.
const TRIM: f32 = 0.02;
/// Kept ahead of the first sound, seconds, so the attack is not cut.
const PRE_ROLL_S: f32 = 0.002;
/// Kept after the last sound, seconds, for the tail's fade.
const POST_ROLL_S: f32 = 0.012;

/// One OGG take, as the bank holds it: mono at the bank's rate, trimmed,
/// faded at both ends like every synthesized cue, and normalized to the
/// bank's one peak so `CueDef::gain` alone decides how loud it plays. `None`
/// for a file that does not decode or is not at the bank's rate.
pub fn decode(ogg: &[u8]) -> Option<Box<[i16]>> {
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
