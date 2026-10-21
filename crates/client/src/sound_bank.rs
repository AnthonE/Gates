//! The bank the engine plays, cue by cue — **every cue a recording**: CC0
//! Freesound field recordings and Kenney's CC0 packs
//! (`assets/sound/MANIFEST.md` lists every file and where it came from). The
//! score is not here: it is whole tracks streamed beside the engine
//! (`sound::music`). The synthesizer
//! (`sound::synth`) is only the fallback for a recording that fails to
//! decode, and `tests/sound.rs` fails if any cue lacks one.
//!
//! A cue with several takes is one buffer of equal slices
//! (`synth::join_takes`): the renderer plays one slice per start and
//! `render/audio.rs::pump` picks which, never the one it played last. That is
//! Rust's own answer to a repeated sound (several recorded variations per
//! action); the pitch nudge (`Cue::pitch_var`) rides on top of it.
//!
//! The beds are **whole files**: a bed is a seamless loop whose last sample
//! flows into its first, so it is not trimmed or faded the way a take is.
//!
//! The recordings are decoded here at boot rather than shipped decoded: a few
//! MB of OGG against many MB of PCM, and the browser decodes the same bytes
//! with the same code. Not behind `render`, so `tests/sound.rs` checks every
//! take headless.

use sound::{synth, Cue, SAMPLE_RATE};

/// One recording, as [`Rec`] holds it on this target: a path under
/// `assets/sound/`, compiled in natively and named for the page to fetch on
/// wasm32.
#[cfg(not(target_arch = "wasm32"))]
macro_rules! rec {
    ($path:expr) => {
        include_bytes!(concat!("../../../assets/sound/", $path))
    };
}
#[cfg(target_arch = "wasm32")]
macro_rules! rec {
    ($path:expr) => {
        concat!("sound/", $path)
    };
}

/// One recording as this build holds it. Natively, its bytes, compiled in.
///
/// **In a browser, its path under `assets/`, and the bytes are fetched**
/// (`render::audio`, [`landed`]). 304 OGGs are 4.56 MB that do not compress,
/// which compiled in were a third of the gzipped module and more of the
/// brotli one — every byte of it on the download a player waits through
/// before the first frame, for a bank that is filled a cue a frame after it
/// anyway.
#[cfg(not(target_arch = "wasm32"))]
pub type Rec = &'static [u8];
/// See the native half above.
#[cfg(target_arch = "wasm32")]
pub type Rec = &'static str;

/// Five takes of one group from Kenney's *Impact Sounds*, as [`Rec`]s.
macro_rules! five {
    ($name:literal) => {
        &[
            rec!(concat!("kenney/", $name, "_000.ogg")),
            rec!(concat!("kenney/", $name, "_001.ogg")),
            rec!(concat!("kenney/", $name, "_002.ogg")),
            rec!(concat!("kenney/", $name, "_003.ogg")),
            rec!(concat!("kenney/", $name, "_004.ogg")),
        ]
    };
}

/// Numbered takes of one recorded group, as [`Rec`]s:
/// `takes!("freesound", "howl", 0 1)` is `freesound/howl_0.ogg` and
/// `howl_1.ogg`.
macro_rules! takes {
    ($dir:literal, $name:literal, $($n:literal)+) => {
        &[$(rec!(concat!($dir, "/", $name, "_", $n, ".ogg"))),+]
    };
}

/// Which recording plays which cue. One row per cue; `MANIFEST.md` lists the
/// files.
static RECORDED: &[(Cue, &[Rec])] = &[
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
    (Cue::StepWood, five!("footstep_wood")),
    (Cue::StepConcrete, five!("footstep_concrete")),
    (Cue::StepMetal, five!("impactPlate_light")),
    (Cue::Howl, takes!("freesound", "howl", 0 1 2 3)),
    (Cue::Growl, takes!("freesound", "growl", 0 1 2 3 4 5)),
    (Cue::Hurt, takes!("freesound", "hurt", 0 1 2 3 4 5)),
    (Cue::Death, takes!("freesound", "death", 0 1 2 3)),
    (Cue::FleshHit, takes!("freesound", "flesh", 0 1 2 3 4 5)),
    (Cue::ImpactStone, five!("impactMining")),
    (Cue::ImpactWood, takes!("pd", "chop", 0 1 2 3 4)),
    (Cue::ImpactMetal, five!("impactMetal_heavy")),
    (Cue::Place, five!("impactPlank_medium")),
    (Cue::BulletSoil, five!("impactSoft_medium")),
    (Cue::BulletStone, five!("impactGeneric_light")),
    (Cue::BulletWood, five!("impactWood_light")),
    (Cue::BulletMetal, five!("impactMetal_light")),
    (Cue::Knock, five!("impactWood_heavy")),
    (Cue::Tung, takes!("kenney", "tung", 0 1 2)),
    (Cue::UiClick, takes!("kenney", "ui_click", 0)),
    (Cue::Hit, takes!("kenney", "hit", 0)),
    // The shooter's headshot is the victim's crunch, as in Rust.
    (Cue::HitHead, takes!("pd", "headshot", 0 1 2)),
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
    (Cue::ShotGun, takes!("freesound", "shot_gun", 0 1 2 3)),
    (Cue::ShotGunFar, takes!("freesound", "shot_gun_far", 0 1 2)),
    (Cue::ShotHeli, takes!("pd", "shot_heli", 0 1 2 3)),
    (Cue::ShotHeliFar, takes!("pd", "shot_heli_far", 0 1 2)),
    (Cue::ShotRifle, takes!("pd", "shot_rifle", 0 1 2 3)),
    (Cue::ShotSmg, takes!("pd", "shot_smg", 0 1 2 3)),
    (Cue::ShotPistol, takes!("pd", "shot_pistol", 0 1 2 3)),
    (Cue::ShotShotgun, takes!("pd", "shot_shotgun", 0 1 2)),
    (Cue::ShotBolt, takes!("pd", "shot_bolt", 0 1 2)),
    (Cue::ShotRocket, takes!("pd", "shot_rocket", 0 1 2)),
    (Cue::ShotBow, takes!("freesound", "shot_bow", 0 1 2)),
    (Cue::ShotCrossbow, takes!("pd", "shot_crossbow", 0 1 2)),
    (Cue::UiHover, takes!("kenney", "ui_hover", 0)),
    (Cue::InvOpen, takes!("kenney", "inv_open", 0 1)),
    (Cue::InvClose, takes!("kenney", "inv_close", 0)),
    (Cue::ItemMove, takes!("kenney", "item_move", 0 1)),
    (Cue::ItemDrop, takes!("kenney", "item_drop", 0)),
    (Cue::CraftStart, takes!("kenney", "craft_start", 0 1)),
    (Cue::MapPin, takes!("kenney", "map_pin", 0)),
    (Cue::Geiger, takes!("pd", "geiger", 0 1 2 3)),
    (Cue::Headshot, takes!("pd", "headshot", 0 1 2)),
    (Cue::BowDraw, takes!("freesound", "bow_draw", 0 1)),
    (Cue::Reload, takes!("freesound", "reload", 0 1)),
    (Cue::Ricochet, takes!("freesound", "ricochet", 0 1 2 3)),
    (Cue::Fuse, takes!("freesound", "fuse", 0)),
    (Cue::Flyby, takes!("pd", "flyby", 0 1 2 3)),
    (Cue::Blast, takes!("freesound", "blast", 0 1 2)),
    (Cue::Collapse, takes!("freesound", "collapse", 0 1)),
    (Cue::OreCrumble, takes!("freesound", "ore_crumble", 0 1 2)),
    (Cue::OreGlint, takes!("kenney", "ore_glint", 0 1)),
    (Cue::TreeFall, takes!("freesound", "tree_fall", 0 1)),
    (Cue::Thunder, takes!("freesound", "thunder", 0 1 2)),
    (Cue::Splash, takes!("freesound", "splash", 0 1 2)),
    (Cue::Land, takes!("freesound", "land", 0 1 2 3)),
    (Cue::CollapseWood, takes!("freesound", "collapse_wood", 0 1)),
    (Cue::BedWind, takes!("freesound", "bed_wind", 0)),
    (Cue::BedSurf, takes!("freesound", "bed_surf", 0)),
    (Cue::BedUnder, takes!("freesound", "bed_under", 0)),
    (Cue::BedRain, takes!("freesound", "bed_rain", 0)),
    (Cue::BedRotor, takes!("pd", "bed_rotor", 0)),
    (Cue::BedRotorFar, takes!("pd", "bed_rotor_far", 0)),
    (Cue::BedTown, takes!("freesound", "bed_town", 0)),
    (Cue::BedNight, takes!("freesound", "bed_night", 0)),
    (Cue::Equip, takes!("kenney", "equip", 0 1 2)),
    (Cue::MapPaper, takes!("kenney", "map_paper", 0 1)),
    (
        Cue::ContainerOpen,
        takes!("kenney", "container_open", 0 1 2),
    ),
    (
        Cue::FireCrackle,
        takes!("freesound", "fire_crackle", 0 1 2 3 4 5 6 7),
    ),
    (Cue::Swing, takes!("freesound", "swing", 0 1 2 3)),
    (Cue::Gather, takes!("kenney", "gather", 0 1 2)),
    (Cue::Eat, takes!("freesound", "eat", 0 1 2)),
    (Cue::Drink, takes!("freesound", "drink", 0 1 2)),
    (Cue::Bandage, takes!("freesound", "bandage", 0 1)),
    (Cue::BushPick, takes!("freesound", "bush_pick", 0 1 2)),
    (Cue::Brush, takes!("freesound", "brush", 0 1 2 3)),
    (Cue::TorchOut, takes!("pd", "torch_out", 0 1)),
    (Cue::Bellow, takes!("pd", "bellow", 0 1 2 3)),
    (Cue::Snort, takes!("freesound", "snort", 0 1 2)),
    (
        Cue::Bird,
        &[
            rec!("freesound/bird_0.ogg"),
            rec!("freesound/bird_1.ogg"),
            rec!("freesound/bird_2.ogg"),
            rec!("freesound/bird_3.ogg"),
            rec!("freesound/bird_4.ogg"),
            rec!("freesound/bird_5.ogg"),
            rec!("pd/bird_6.ogg"),
            rec!("pd/bird_7.ogg"),
            rec!("pd/bird_8.ogg"),
            rec!("pd/bird_9.ogg"),
            rec!("pd/bird_10.ogg"),
            rec!("pd/bird_11.ogg"),
        ],
    ),
];

/// The cue whose bank a cue plays. The remote halves are the same boot on the
/// same ground and the same arm (`sound::synth` delegates them the same way),
/// so they share its takes; so does a frame's second hurt.
pub fn source(cue: Cue) -> Cue {
    match cue {
        Cue::RemoteStepSand => Cue::StepSand,
        Cue::RemoteStepGrass => Cue::StepGrass,
        Cue::RemoteStepLitter => Cue::StepLitter,
        Cue::RemoteStepRock => Cue::StepRock,
        Cue::RemoteStepWater => Cue::StepWater,
        Cue::RemoteStepWood => Cue::StepWood,
        Cue::RemoteStepConcrete => Cue::StepConcrete,
        Cue::RemoteStepMetal => Cue::StepMetal,
        Cue::RemoteSwing => Cue::Swing,
        Cue::RemoteReload => Cue::Reload,
        Cue::RemoteEat => Cue::Eat,
        Cue::RemoteBandage => Cue::Bandage,
        Cue::RemoteDrink => Cue::Drink,
        Cue::RemoteContainerOpen => Cue::ContainerOpen,
        Cue::RemoteEquip => Cue::Equip,
        Cue::RemoteLand => Cue::Land,
        Cue::RemoteSplash => Cue::Splash,
        Cue::RemoteBowDraw => Cue::BowDraw,
        Cue::RemoteBrush => Cue::Brush,
        Cue::HurtAgain => Cue::Hurt,
        Cue::OreBreak => Cue::Collapse,
        c => c,
    }
}

/// Is this cue played from a recording?
pub fn recorded(cue: Cue) -> bool {
    RECORDED.iter().any(|(c, _)| *c == source(cue))
}

/// The files a cue's recording is made of, empty for a synthesized one.
pub fn files(cue: Cue) -> &'static [Rec] {
    let src = source(cue);
    RECORDED
        .iter()
        .find(|(c, _)| *c == src)
        .map_or(&[], |(_, f)| f)
}

/// Every file in the bank, once, in the order `order` lists cues: the order
/// a page fetches them in.
pub fn all_files(order: &[Cue]) -> Vec<Rec> {
    let mut out: Vec<Rec> = Vec::new();
    for cue in order {
        for f in files(*cue) {
            if !out.contains(f) {
                out.push(*f);
            }
        }
    }
    out
}

#[cfg(target_arch = "wasm32")]
thread_local! {
    /// The page's fetched recordings: `None` for a file that failed for good,
    /// whose cue then falls back to the synthesizer as a bad file does.
    static FETCHED: std::cell::RefCell<std::collections::HashMap<Rec, Option<Vec<u8>>>> =
        Default::default();
}

/// A fetched file has arrived (`Some`), or will not (`None`).
#[cfg(target_arch = "wasm32")]
pub fn landed(file: Rec, bytes: Option<Vec<u8>>) {
    FETCHED.with(|f| f.borrow_mut().insert(file, bytes));
}

/// Whether every file `cue` needs is here, or has failed for good. Always,
/// natively, where they are compiled in.
pub fn ready(cue: Cue) -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        FETCHED.with(|f| {
            let f = f.borrow();
            files(cue).iter().all(|r| f.contains_key(r))
        })
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = cue;
        true
    }
}

/// Run `with` over a lookup from a [`Rec`] to its bytes: the bytes
/// themselves natively, the fetched ones in a browser.
fn read_with<T>(with: impl for<'b> FnOnce(&'b dyn Fn(Rec) -> Option<&'b [u8]>) -> T) -> T {
    #[cfg(target_arch = "wasm32")]
    {
        FETCHED.with(|f| {
            let f = f.borrow();
            with(&|r| f.get(r).and_then(|b| b.as_deref()))
        })
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        with(&|r| Some(r))
    }
}

/// Is this cue one whole file rather than takes: a looping bed, which may not
/// be trimmed or faded.
pub fn whole(cue: Cue) -> bool {
    cue.is_bed()
}

/// How many takes the synthesizer renders for a cue whose recording failed:
/// the sounds heard most often in a row get several, the rest one.
pub fn synth_takes(cue: Cue) -> u8 {
    match source(cue) {
        Cue::Ricochet => 4,
        Cue::ShotGun
        | Cue::ShotGunFar
        | Cue::ShotHeli
        | Cue::ShotHeliFar
        | Cue::ShotBow
        | Cue::ShotCrossbow
        | Cue::StepSand
        | Cue::StepLitter
        | Cue::StepWater
        | Cue::Eat
        | Cue::BushPick
        | Cue::Brush
        | Cue::Geiger => 3,
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
    let files = files(src);
    if files.is_empty() {
        return None;
    }
    read_with(|bytes| {
        if whole(src) {
            return Some((decode_whole(bytes(files[0])?)?, 1));
        }
        let takes: Vec<Box<[i16]>> = files
            .iter()
            .copied()
            .filter_map(|f| bytes(f).and_then(decode))
            .collect();
        (takes.len() == files.len() && !takes.is_empty())
            .then(|| (synth::join_takes(&takes), takes.len() as u8))
    })
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
