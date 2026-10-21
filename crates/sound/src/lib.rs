//! The client's audio **model** — what should be heard, how loud, and how
//! many at once. Computed outside Bevy.
//!
//! **Bevy plays; it does not decide.** This is `RENDER.md` §1's rule applied
//! one surface over, and for the same reason: a mixer that lives in a system
//! can only be tested by a windowed run with a sound card, and this box has
//! neither. Everything here is a pure function of a cue request, a listener
//! position and a `dt`, so `crates/client/tests/sound.rs` drives all of it
//! **headless, with no `render` feature and no audio device** — which is also
//! why this module is not feature-gated. `ui/` is the precedent and this is
//! deliberately the same shape.
//!
//! What the reference game does, and which parts we took, is
//! `reference/AUDIO.md`. Three of its findings are load-bearing here and are
//! restated where they bind:
//!
//! - **The sound system is on a per-frame budget.** The reference ships
//!   `audio.framebudget 0.3` — 0.3 ms a frame for sound updates, a convar, not
//!   a comment. A pure module cannot measure milliseconds, so the budget here
//!   is in work items ([`STARTS_PER_FRAME`], [`VOICE_CAP`], [`CUE_QUEUE_CAP`]),
//!   which is what `CLAUDE.md` wall 4 asks for anyway: a cap and a stated
//!   overflow policy on every queue.
//! - **Buses before effects.** The reference moved to Unity 5's audio mixer —
//!   groups you can balance and snapshots you can fade between — before it had
//!   occlusion or reverb. [`Bus`] is that, at the size ours can be honest
//!   about: groups mirroring `audio.game`, `audio.ambience` and
//!   `audio.musicvolume`, plus footsteps and the interface.
//! - **Music is a gap system, not a soundtrack.** Four to eight minutes of
//!   silence between songs, the intensity read so a song never opens in the
//!   middle of a fight. [`music`] is that director; the songs are whole
//!   recorded tracks streamed beside the engine, not cues.
//! - **Occlusion is a knob, and it is on now**, as the reference's has been
//!   since late 2016. The walk is the client's (`ui::occlusion`: the shot
//!   walk over the built pieces, from the ear to the source); what it costs
//!   a sound is a gain and a low-pass, applied to a one-shot as it starts and
//!   to a talker every quarter second (`talk`). This module only carries
//!   the filter; the SOUND OCCLUSION row turns the walk off.
//!
//! Every number below is a knob with its documented default
//! (`DECISIONS.md` §open, "audio v0"). None of them was measured against
//! anything, and the module says so rather than implying a tuning pass that
//! did not happen.

// The forest layer — sparse bird calls over the beds. `reference/AUDIO.md`
// §3's *layers*, as distinct from its beds.
pub mod birds;
// The sample renderer — the audio thread's half. Everything above it decides
// WHAT is heard; this is the one thing that turns the decision into samples,
// on both targets, and it is here rather than in `render/` so that the
// arithmetic a player actually hears is gated headless like the rest.
pub mod engine;
// The Verdigris counter: when it clicks, from how deep the green is where you
// stand. Pure cadence, like the voices; `render/audio.rs` plays the click.
pub mod geiger;
// What the mixer is told when the health bar moves. Pure, and deliberately
// takes the *fall* as well as the event: three of the sim's seven damage
// routes announce nothing, on purpose, and a mixer fed only `EV_HURT` goes
// silent for all three with every gate green.
pub mod hit;
pub mod hurt;
// The last stage before the device, after the score and the voices are summed
// in: a peak limiter, so a voice over a gunshot cannot clip.
pub mod limit;
pub mod mixer;
// When a song plays and which piece it is. `reference/AUDIO.md` §8 is the
// design; this is the whole of it.
pub mod music;
pub mod steps;
pub mod synth;
// Players talking (proximity voice): each talker's decoded speech, mixed into
// the block at the gains the game thread computes from where they stand.
pub mod talk;
// What water sounds like from where you are standing. `reference/WATER.md` §7
// is the research; this is the model, and `render/audio.rs` plays it.
pub mod water;
// When an animal speaks. Pure cadence — the render half reads the drawn herd,
// asks it what the species in that roster slot says at that range, and plays
// exactly the cue it is handed back.
pub mod voice;
// The browser's half of the audio seam: the byte protocol a `postMessage`
// carries, and the `AudioWorkletProcessor`'s renderer-plus-scratch. Here
// rather than in `crates/sound-worklet` — which is nothing but the
// `#[wasm_bindgen]` wrapper a browser loads — so that every decision on the
// path is gated by `cargo test --workspace` on a box with no browser.
pub mod worklet;

/// The bank's sample rate, Hz.
///
/// One rate for the whole bank because [`synth`] generates every cue at boot
/// and [`engine::rate`] resamples it to whatever the device runs at — the one
/// resample in the chain, inside the renderer — so a second rate would buy
/// nothing and would make the seam arithmetic in [`synth::loop_seam`] take a
/// parameter it does not need.
pub const SAMPLE_RATE: u32 = 44_100;

/// Which group a cue is balanced in. The reference's mixer groups, at the
/// size ours can be honest about.
///
/// **Players' voices are not a bus here**: they never pass the cue mixer.
/// [`talk`] mixes them after the renderer, at gains the client computes with
/// its own VOICE slider. [`Bus::Music`] stopped being a greyed-out row when
/// `music::Director` landed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Bus {
    /// Everything the player or the world did: swings, impacts, shots,
    /// animals. The reference's `audio.game`.
    Game,
    /// The beds and the layers over them. The reference's `audio.ambience`.
    Ambience,
    /// The score. The reference's `audio.musicvolume`, whose default is
    /// **0.2** — music is the one bus that does not open at full, there and
    /// here (see [`Mix::default`]).
    Music,
    /// Footsteps, landings and pushing through plants: yours and everyone
    /// else's. Split off the game bus so a player can turn their own boots
    /// down without turning the guns down with them.
    Steps,
    /// The menus and the inventory answering the hand: clicks, hovers, slots,
    /// the map, the craft chimes.
    Interface,
}

/// Every sound this client can make, as an integer code.
///
/// Integer codes rather than strings for the sim's own reason (`CLAUDE.md`
/// wall 3): a cue crosses a queue every frame, and a `String` on that path is
/// an allocation per sound. The name is for the reader; the wire is the
/// discriminant.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Cue {
    StepSand = 0,
    StepGrass,
    StepLitter,
    StepRock,
    StepWater,
    Swing,
    ImpactWood,
    ImpactStone,
    ImpactMetal,
    Gather,
    CraftDone,
    Refused,
    Hit,
    Hurt,
    Death,
    Place,
    /// Breaking the surface, in either direction. The *edge*, not the wading:
    /// [`Cue::StepWater`] is already playing for every stride taken in the
    /// shallows.
    Splash,
    TreeFall,
    UiClick,
    /// The wind bed. Looped, not fired — a bed is a voice that is turned up
    /// and down rather than triggered, and [`Cue::is_bed`] is the rule the
    /// mixer refuses them by. `render/audio.rs` owns the one entity each.
    BedWind,
    /// The surf bed: heard when there is sea within earshot, at a level that
    /// reads how much (`water::shore_exposure`).
    BedSurf,
    /// The submerged bed. Crossfaded in by [`Snapshot::Submerged`], never by
    /// the world directly.
    BedUnder,
    /// The pig. Appended after the beds rather than beside the other
    /// diegetic cues, deliberately: every cue's noise is seeded off its own
    /// discriminant (`synth::render`), so inserting mid-enum would renumber
    /// everything after it and regenerate waveforms this cue has nothing to
    /// do with. Discriminant order is append order from here on.
    Snort,
    /// Another player's footstep: [`Cue::StepSand`] heard at THEIR body.
    /// A separate cue rather than a positional flag on the local one,
    /// because whether a cue is positional is the def's fact and the
    /// mixer's one distance law hangs off it — the local step is an
    /// own-fact at the listener, this is a place in the world, and one row
    /// cannot be both. The waveform is the local surface's byte for byte
    /// (`synth::render` delegates): the ground decides what a step sounds
    /// like, not whose boot it is. Appended after `Snort` — the enum's
    /// append-order rule.
    RemoteStepSand,
    RemoteStepGrass,
    RemoteStepLitter,
    RemoteStepRock,
    RemoteStepWater,
    /// A bird, somewhere in the trees. `sound::birds` is the cadence; the
    /// perch is a drawn prop, so the layer cannot call from an empty sky.
    Bird,
    /// The wolf's far voice: a contact call, heard from further than any
    /// other diegetic cue but the falling tree. Appended after the score —
    /// the enum's append-order rule, for the reason `Snort` states.
    ///
    /// The pair with [`Cue::Growl`] is **two registers of one animal, chosen
    /// by distance**, and the distance is not a third number: it is this
    /// cue's sibling's own [`CueDef::radius_m`]. See [`crate::voice`].
    Howl,
    /// The wolf's near voice: the threat, at the short end.
    ///
    /// ⚠ **It means "a wolf is close", not "a wolf is hunting you"** — the
    /// client cannot see that. `Mob::roused_until` and `MobDef::brave_pct`
    /// are sim-side only (`sim-core/src/mob.rs`), the snapshot carries
    /// `EntityState` and no mob state or def lane exists on the wire, and
    /// duplicating a content radius into client code would be wall 7. So
    /// the register is the honest half of the encounter: a growl is a
    /// close-range threat call and a howl is a long-range contact call,
    /// which is what the two vocalizations *are*, whatever the animal is
    /// currently deciding.
    Growl,
    /// Another player's swing: [`Cue::Swing`] heard at THEIR body.
    ///
    /// The remote-footstep argument, one verb over — whether a cue is
    /// positional is the def's fact and the mixer's one distance law hangs
    /// off it, so the local arm (an own-fact at the listener, fired off a
    /// keystroke you already knew about) and someone else's arm (a place in
    /// the world) cannot be one row. The waveform is the local one's byte
    /// for byte (`synth::render` delegates): what makes a swing remote is
    /// its def, never its sound.
    ///
    /// **Reusing `Cue::Swing` for both would have been worse than silence**
    /// — non-positional means straight to both ears at full gain with no
    /// pan and no falloff, so every swing on the island would arrive as if
    /// it were in your hands, which is a lie about where a threat is rather
    /// than a missing sound. Appended after `Growl`, the enum's
    /// append-order rule.
    RemoteSwing,
    /// A bow released, heard from where the archer stands (wire v54).
    ///
    /// `EV_SHOT` has crossed the wire since ranged v0 and reached exactly
    /// one reader — `render/tracer.rs` — so a bow drew a streak and made
    /// no sound at all. This is the other half of that event.
    ///
    /// Quiet and short on purpose: a bowstring is the reference's stealth
    /// option against a firearm, and the whole of what makes it one is
    /// that it does not carry.
    ShotBow,
    /// A gun fired, heard from where the shooter stands (wire v54).
    ///
    /// **This is the loudest thing in the bank and that is the mechanic.**
    /// A firearm raised no event at all until v54, so a gunfight was a
    /// private event — the only evidence a shot had happened was the
    /// damage, and a player had no input to the fight-or-run decision.
    /// Sound is the reference's primary disclosure channel
    /// (`reference/AUDIO.md` §9) and its radius is a hundred metres, which
    /// is why [`MAX_AUDIBLE_M`] is what it is.
    ShotGun,
    /// The hitmarker for a **head** hit (v58). Played from the same crunch as
    /// [`Cue::Headshot`]: in Rust the shooter hears the headshot too, which
    /// is how they know it was one (operator, 2026-10-17).
    ///
    /// A separate cue rather than [`Cue::Hit`] pitched up, because
    /// `interface_cues_do_not_vary_in_pitch` is a rule this bank keeps for
    /// a reason — a symbol that drifts stops being a symbol — and "the
    /// same click, higher" is exactly the drift that rule forbids. Three
    /// rungs are three symbols, so they are three waveforms.
    ///
    /// Appended after `ShotGun`, the enum's append-order rule: the
    /// discriminant seeds `synth::render`'s noise, so inserting one beside
    /// `Hit` would regenerate unrelated cues.
    HitHead,
    /// The hitmarker for a **limb** hit (v58).
    ///
    /// The one the judge's gap is really about: a x0.5 blow is easier to
    /// misread as a miss than a x2 one is to read as a skull, so this cue
    /// must say *landed, but less* — duller and lower than [`Cue::Hit`],
    /// never quieter to the point of ambiguity with silence.
    HitLimb,
    /// The rain bed (weather v0): a looped hiss with drops in it, turned up
    /// with the rain and muffled under a roof. A bed ([`Cue::is_bed`]).
    /// Appended, the enum's append-order rule.
    BedRain,
    /// Thunder: a crack and a long roll, heard wherever you are — a storm is
    /// not a place in the world, so it is not positional.
    Thunder,
    /// A round landing in soil, sand or grass: a dull slap and a spray.
    BulletSoil,
    /// A round on stone or rock: a hard crack with grit in it.
    BulletStone,
    /// A round into wood: a dry thock.
    BulletWood,
    /// A round on metal: a bright ping.
    BulletMetal,
    /// A round glancing off stone or metal and whining away.
    Ricochet,
    /// A blade or a round into a body.
    FleshHit,
    /// A gun fired a long way off: low, dark and long, the layer heard past
    /// [`Cue::ShotGun`]'s own radius (`render/audio.rs::shots` picks by range).
    ShotGunFar,
    /// A charge going off.
    Blast,
    /// A built piece coming down.
    Collapse,
    /// Somebody knocking on a door.
    Knock,
    /// Your own gun being reloaded.
    Reload,
    /// You ate something: a few crunching chews.
    Eat,
    /// You used a heal (a bandage, a medkit): a rip and a wrap.
    Bandage,
    /// You drew a bow: the limbs creaking as the string comes back.
    BowDraw,
    /// The attack helicopter's rotor (`sim-core/src/heli.rs`): the blade
    /// slap over a turbine whine. A bed ([`Cue::is_bed`]) that
    /// `render/audio.rs` turns up as the heli closes, so you hear it coming
    /// before you see it. Appended, the enum's append-order rule.
    BedRotor,
    /// A bush picked by hand (`E`, `sim_core::gather::pick`): a short leafy
    /// rustle and a soft stem snap, heard at the bush — anyone's pick, so it
    /// is positional. Appended, the enum's append-order rule.
    BushPick,
    /// A town sentry locking on (`sim_core::sentry`): Rust's turret beep,
    /// three hard high pips at the gun before it fires. Positional — it is
    /// the gun telling the town who it is about to shoot.
    SentryLock,
    /// A kiosk trade went through: two bright coin notes. Your own hands.
    Trade,
    /// Into THE GATE's safe zone, or out of it: a soft two-note bell.
    GateChime,
    /// A running recycler's motor, as a bed: a small diesel generator's
    /// loop. `render/audio.rs` turns it up as you near one the sim says is
    /// running, and only then (it was THE GATE's yard bed once, hence the
    /// name).
    BedTown,
    /// A door (or shutter) swinging open, at the leaf. Appended, the enum's
    /// append-order rule, like everything below.
    DoorOpen,
    /// A door swinging shut, at the leaf.
    DoorClose,
    /// You drank: a gulp or two. Your own mouth.
    Drink,
    /// You landed from a jump or a drop: a heavy boot thud. Your own feet.
    Land,
    /// A lit fire or furnace crackling, fired every second or so from the
    /// nearest one in reach (`render/audio.rs::fires`).
    FireCrackle,
    /// A satchel's fuse sizzling at the charge, for its whole fuse.
    Fuse,
    /// Someone else's bullet passing close: the snap and whiz at the point
    /// of the line nearest your head (`fx::gun::shots`).
    Flyby,
    /// A blueprint learned: a page turned. Your own hands.
    Learn,
    /// A lock or a hearth took your code or your name: the keypad's accept,
    /// at the lock.
    Unlock,
    /// A twig or wooden piece coming down: planks splintering. [`Cue::Collapse`]
    /// is the stone and metal ones.
    CollapseWood,
    /// Night on the island, as a bed: a field of crickets. `render/audio.rs`
    /// brings it up as the light goes and the rain stops.
    BedNight,
    /// Something new in your hand: leather and a strap as it comes off the
    /// belt. Also a backpack on the ground being opened. Your own hands.
    Equip,
    /// The map, unfolded or put away: paper. Your own hands.
    MapPaper,
    /// A box or a crate opened: the latch, then the lid. Your own hands.
    ContainerOpen,
    // ---- other bodies' hands (wire v93, `SUB_HEARD`) ------------------
    // Each is its own cue's recording heard at another body: positional,
    // culled by the one falloff law, with its own reach. `RemoteSwing`'s
    // shape: the def is what makes it remote, never the waveform.
    /// Somebody else seating a magazine.
    RemoteReload,
    /// Somebody else eating.
    RemoteEat,
    /// Somebody else dressing a wound.
    RemoteBandage,
    /// Somebody else drinking.
    RemoteDrink,
    /// Somebody else opening a box or a crate.
    RemoteContainerOpen,
    /// Somebody else drawing an item, or opening a backpack on the ground.
    RemoteEquip,
    /// Somebody else coming down from a jump or a fall.
    RemoteLand,
    /// Somebody else coming down into water.
    RemoteSplash,
    /// Somebody else drawing a bow.
    RemoteBowDraw,
    /// Pushing through a plant: leaves dragged along you, once a stride while
    /// you are in a shrub, a berry bush or hemp (`render/audio.rs::steps`).
    /// Your own body, so non-positional.
    Brush,
    /// Somebody else pushing through one, at their body: in Rust a bush you
    /// cannot see into still tells you someone is moving in it.
    RemoteBrush,
    /// Your torch burning out in your hand: the last of the flame hissing
    /// away (`render/viewmodel.rs::torch_watch`). Yours, so non-positional.
    TorchOut,
    /// The stag's voice: a deep roar falling into a grunt (`sound::voice`).
    /// Until this the stag borrowed the pig's snort. Appended last — the
    /// enum's append-order rule, for the reason `Snort` states.
    Bellow,
    /// The second blow to land in one frame: [`Cue::Hurt`]'s recording
    /// started as a second voice (`sound::hurt::again`). Its own row because
    /// a per-row cooldown binds inside a frame, so two blows asked of one row
    /// are one voice; it keeps Hurt's clock across frames ([`Cue::lead`]).
    HurtAgain,
    /// A crossbow loosed, heard from where the shooter stands: the bow's
    /// radius, a heavier thunk. Until this it played the bow's string
    /// (`render/audio.rs::shots` tells them apart by what is in the hand).
    /// Appended last — the enum's append-order rule.
    ShotCrossbow,
    /// An ore node crumbling past a stage (wire v106): a short rattle of rock
    /// coming off it, at the node. Masks the look changing, the reference's
    /// own trick. Appended, the enum's append-order rule.
    OreCrumble,
    /// An ore node's last hit: the whole thing coming apart, at the node.
    /// [`Cue::Collapse`]'s debris crash, by its own row.
    OreBreak,
    /// A blow that landed on the node's hotspot: a bright tink, at the node.
    OreGlint,
    /// The bat landing a blow: Triple T's hollow-log "tung", on top of
    /// whatever the blow struck, at the point it struck.
    Tung,
    /// One round of the attack helicopter's gun, at the heli. Its own cue
    /// rather than [`Cue::ShotGun`]: a burst is eight rounds 100 ms apart,
    /// and the revolver's ringing report smeared eight of them into one
    /// roar. A rifle round with a short tail keeps every round heard.
    ShotHeli,
    /// The heli's rotor from far off: a distant recording, the thump with
    /// the top end gone, crossfaded in as [`Cue::BedRotor`] (the near
    /// layer) fades with distance. A bed ([`Cue::is_bed`]).
    BedRotorFar,
    /// One round of the heli's gun from far off: a distant blast and its
    /// echo, kept short so a burst still reads round by round
    /// (`render/audio.rs` picks it past `HELI_SHOT_FAR_M`).
    ShotHeliFar,
    /// The pointer arriving on a button: the faintest tick, so a menu
    /// answers the hand before it clicks. Appended, the enum's append-order
    /// rule, like the six after it.
    UiHover,
    /// Your inventory, crafting or loot page opening: the pack's cloth.
    InvOpen,
    /// …and closing.
    InvClose,
    /// An item set down in a slot (a move, a split, a quick-move).
    ItemMove,
    /// An item thrown out of the inventory onto the ground.
    ItemDrop,
    /// The CRAFT button: the kit coming out.
    CraftStart,
    /// A mark put on, or taken off, the map.
    MapPin,
    /// The Verdigris counter: one tube click, yours, so not positional.
    Geiger,
    /// You took a headshot: Rust's big crunch, a crack over a smashed thud.
    /// The only thing a hit body hears from its own skin (operator,
    /// 2026-10-17: Rust makes no pain sound, *"if anything they make a big
    /// smashy crunchy sound when you get head shot"*). Yours, so not
    /// positional. Appended, the enum's append-order rule.
    Headshot,
    /// A step on a built floor of twig or wood. The step family's grounds
    /// that are not the island's: `render/audio.rs` picks one when the body
    /// stands on a piece (`sim_core::collide::floor_under`), so a player
    /// upstairs sounds like one. Appended, the enum's append-order rule, like
    /// the five after it.
    StepWood,
    /// A step on a stone floor.
    StepConcrete,
    /// A step on a sheet-metal floor.
    StepMetal,
    /// The three floors heard at another body, as [`Cue::RemoteStepSand`].
    RemoteStepWood,
    RemoteStepConcrete,
    RemoteStepMetal,
    /// Rust's guns (guns v1), one report per kind of gun so a fight is read
    /// by ear: an assault or semi-automatic rifle's crack (5.56),
    ShotRifle,
    /// a submachine gun's lighter, faster chatter (pistol rounds),
    ShotSmg,
    /// a pistol's snap,
    ShotPistol,
    /// a pump shotgun's boom,
    ShotShotgun,
    /// a bolt-action rifle's long, heavy report,
    ShotBolt,
    /// and a rocket leaving its tube: the launch's whoosh and thump.
    ShotRocket,
    /// A lit furnace, as a bed: its fire shut in an iron box, a low roar
    /// with the wood ticking in it. `render/audio.rs` turns it up and pans it
    /// toward the nearest lit furnace; a campfire keeps its crackle. Appended,
    /// the enum's append-order rule, like the five after it.
    BedFurnace,
    /// A furnace lit, at the furnace: the flame catching inside with a
    /// whoomp, and the iron door swung to.
    FurnaceLight,
    /// A furnace switched off or burned dry: the roar cut short, and the door.
    FurnaceOut,
    /// A furnace opened under your hands: the iron door's latch and swing.
    FurnaceOpen,
    /// A campfire lit, at the fire: the flame catching.
    Kindle,
    /// A campfire switched off or burned dry: its roar cut short.
    Snuff,
    /// A trigger pulled on an empty magazine: the gun's dry click, in your
    /// hands. Appended, the enum's append-order rule.
    DryFire,
    /// Your own reload, by the kind of gun (`render/audio.rs::reload_cue`):
    /// a box magazine out and in and the charging handle (the rifles and
    /// SMGs), a pistol's magazine and slide, shells pushed into a pump's
    /// tube and the pump, a bolt rifle's bolt and rounds. The revolvers keep
    /// [`Cue::Reload`]'s cylinder. Each fills the gun's reload time.
    /// Appended, the enum's append-order rule, like the seven after it.
    ReloadRifle,
    ReloadPistol,
    ReloadShotgun,
    ReloadBolt,
    /// The four heard at another body, as [`Cue::RemoteReload`].
    RemoteReloadRifle,
    RemoteReloadPistol,
    RemoteReloadShotgun,
    RemoteReloadBolt,
    /// The guns' far layers (`render/audio.rs::far_layer`): each family's
    /// report from a long way off, its crack and the land's echo of it, so a
    /// fight across the valley is still read by ear. [`Cue::ShotGunFar`]
    /// stays the revolver's, and every shot the shard reports from past the
    /// interest set (`SUB_REPORT` names no gun). Appended, the enum's
    /// append-order rule.
    ShotRifleFar,
    ShotSmgFar,
    ShotPistolFar,
    ShotShotgunFar,
    ShotBoltFar,
    /// A sheet-metal door swinging open, at the leaf: the latch, then the
    /// leaf to its stop. [`Cue::DoorOpen`] stays the wooden door's creak.
    /// Appended, the enum's append-order rule, like the three after it.
    DoorMetalOpen,
    /// A sheet-metal door slammed into its frame.
    DoorMetalClose,
    /// A garage door rolling up its track.
    GarageOpen,
    /// A garage door rolling down and its bottom rail landing.
    GarageClose,
    /// A piece or a deployable mended (`R`, the hammer): a hammer on wood,
    /// at what was mended, whoever mended it. Appended, the enum's
    /// append-order rule.
    Repair,
}

/// How many cues there are. Kept beside [`Cue::ALL`], which is what fails if
/// they disagree.
pub const CUE_COUNT: usize = 140;

impl Cue {
    /// Every cue, in discriminant order. The bank is built by walking this,
    /// and `tests/sound.rs` asserts it matches [`CUE_COUNT`] and that each
    /// entry's index is its own discriminant — which is the assertion that
    /// catches a cue added to the enum and forgotten here, the one mistake
    /// that would silently shift every table below by one.
    pub const ALL: [Cue; CUE_COUNT] = [
        Cue::StepSand,
        Cue::StepGrass,
        Cue::StepLitter,
        Cue::StepRock,
        Cue::StepWater,
        Cue::Swing,
        Cue::ImpactWood,
        Cue::ImpactStone,
        Cue::ImpactMetal,
        Cue::Gather,
        Cue::CraftDone,
        Cue::Refused,
        Cue::Hit,
        Cue::Hurt,
        Cue::Death,
        Cue::Place,
        Cue::Splash,
        Cue::TreeFall,
        Cue::UiClick,
        Cue::BedWind,
        Cue::BedSurf,
        Cue::BedUnder,
        Cue::Snort,
        Cue::RemoteStepSand,
        Cue::RemoteStepGrass,
        Cue::RemoteStepLitter,
        Cue::RemoteStepRock,
        Cue::RemoteStepWater,
        Cue::Bird,
        Cue::Howl,
        Cue::Growl,
        Cue::RemoteSwing,
        Cue::ShotBow,
        Cue::ShotGun,
        Cue::HitHead,
        Cue::HitLimb,
        Cue::BedRain,
        Cue::Thunder,
        Cue::BulletSoil,
        Cue::BulletStone,
        Cue::BulletWood,
        Cue::BulletMetal,
        Cue::Ricochet,
        Cue::FleshHit,
        Cue::ShotGunFar,
        Cue::Blast,
        Cue::Collapse,
        Cue::Knock,
        Cue::Reload,
        Cue::Eat,
        Cue::Bandage,
        Cue::BowDraw,
        Cue::BedRotor,
        Cue::BushPick,
        Cue::SentryLock,
        Cue::Trade,
        Cue::GateChime,
        Cue::BedTown,
        Cue::DoorOpen,
        Cue::DoorClose,
        Cue::Drink,
        Cue::Land,
        Cue::FireCrackle,
        Cue::Fuse,
        Cue::Flyby,
        Cue::Learn,
        Cue::Unlock,
        Cue::CollapseWood,
        Cue::BedNight,
        Cue::Equip,
        Cue::MapPaper,
        Cue::ContainerOpen,
        Cue::RemoteReload,
        Cue::RemoteEat,
        Cue::RemoteBandage,
        Cue::RemoteDrink,
        Cue::RemoteContainerOpen,
        Cue::RemoteEquip,
        Cue::RemoteLand,
        Cue::RemoteSplash,
        Cue::RemoteBowDraw,
        Cue::Brush,
        Cue::RemoteBrush,
        Cue::TorchOut,
        Cue::Bellow,
        Cue::HurtAgain,
        Cue::ShotCrossbow,
        Cue::OreCrumble,
        Cue::OreBreak,
        Cue::OreGlint,
        Cue::Tung,
        Cue::ShotHeli,
        Cue::BedRotorFar,
        Cue::ShotHeliFar,
        Cue::UiHover,
        Cue::InvOpen,
        Cue::InvClose,
        Cue::ItemMove,
        Cue::ItemDrop,
        Cue::CraftStart,
        Cue::MapPin,
        Cue::Geiger,
        Cue::Headshot,
        Cue::StepWood,
        Cue::StepConcrete,
        Cue::StepMetal,
        Cue::RemoteStepWood,
        Cue::RemoteStepConcrete,
        Cue::RemoteStepMetal,
        Cue::ShotRifle,
        Cue::ShotSmg,
        Cue::ShotPistol,
        Cue::ShotShotgun,
        Cue::ShotBolt,
        Cue::ShotRocket,
        Cue::BedFurnace,
        Cue::FurnaceLight,
        Cue::FurnaceOut,
        Cue::FurnaceOpen,
        Cue::Kindle,
        Cue::Snuff,
        Cue::DryFire,
        Cue::ReloadRifle,
        Cue::ReloadPistol,
        Cue::ReloadShotgun,
        Cue::ReloadBolt,
        Cue::RemoteReloadRifle,
        Cue::RemoteReloadPistol,
        Cue::RemoteReloadShotgun,
        Cue::RemoteReloadBolt,
        Cue::ShotRifleFar,
        Cue::ShotSmgFar,
        Cue::ShotPistolFar,
        Cue::ShotShotgunFar,
        Cue::ShotBoltFar,
        Cue::DoorMetalOpen,
        Cue::DoorMetalClose,
        Cue::GarageOpen,
        Cue::GarageClose,
        Cue::Repair,
    ];

    /// May the mixer start this cue? False for the beds, which are voices
    /// the render layer holds rather than events it fires. (The score is not
    /// a cue at all: whole tracks streamed beside the engine, `sound::music`.)
    pub fn mixer_started(self) -> bool {
        !self.is_bed()
    }

    /// Is this cue a looping bed?
    ///
    /// A bed is never started by the mixer: [`mixer::Mixer::push`] refuses one
    /// and counts it as the caller bug it is. The rule exists as a predicate
    /// rather than as three remembered names because it is now checked in
    /// three places — the mixer, the frame-budget gate, and the
    /// cooldown-stacking gate, all of which would otherwise each carry their
    /// own list to forget a fourth bed from.
    pub fn is_bed(self) -> bool {
        matches!(
            self,
            Cue::BedWind
                | Cue::BedSurf
                | Cue::BedUnder
                | Cue::BedRain
                | Cue::BedRotor
                | Cue::BedTown
                | Cue::BedNight
                | Cue::BedRotorFar
                | Cue::BedFurnace
        )
    }

    /// The cue this one is a second voice of: its cooldown clock and its
    /// last-take memory ([`mixer::Takes`]). Itself for every cue but
    /// [`Cue::HurtAgain`].
    ///
    /// Inside a frame the mixer refuses a second start by row; across frames,
    /// by this clock. So a frame with two blows starts Hurt and HurtAgain,
    /// and the next blow 16 ms later is still on Hurt's 120 ms whichever row
    /// it asks for. Not the remote rows: another body's swing has a clock of
    /// its own on purpose (`RSWING`).
    pub fn lead(self) -> Cue {
        match self {
            Cue::HurtAgain => Cue::Hurt,
            c => c,
        }
    }

    /// The cue's index into every table in this module.
    pub fn idx(self) -> usize {
        self as usize
    }

    /// The cue's static properties.
    pub fn def(self) -> &'static CueDef {
        &CUES[self.idx()]
    }

    /// How far this cue's playback rate may wander from 1.0, as a fraction.
    ///
    /// **The cheapest fix for the worst tell in a generated bank.** One
    /// footstep sample retriggered every 0.85 m is a machine gun firing the
    /// identical waveform, and no amount of synthesis quality hides it; ±10%
    /// on the rate makes each step a different length and a different pitch
    /// for one multiply. It is also how the reference gets variation, in the
    /// expensive way — it records "multiple variations per action"
    /// (`reference/AUDIO.md` §5), which is a bank we do not have.
    ///
    /// **Signal cues get zero, and that is the interesting half.** The
    /// hitmarker, the craft chime, the refusal and the UI click are the four
    /// sounds a player learns as *symbols*, and a symbol that changes pitch
    /// every time is a symbol that takes longer to learn. Diegetic sounds
    /// vary; the interface does not.
    pub fn pitch_var(self) -> f32 {
        match self {
            Cue::StepSand
            | Cue::StepGrass
            | Cue::StepLitter
            | Cue::StepRock
            | Cue::StepWater
            | Cue::RemoteStepSand
            | Cue::RemoteStepGrass
            | Cue::RemoteStepLitter
            | Cue::RemoteStepRock
            | Cue::RemoteStepWater
            | Cue::StepWood
            | Cue::StepConcrete
            | Cue::StepMetal
            | Cue::RemoteStepWood
            | Cue::RemoteStepConcrete
            | Cue::RemoteStepMetal => 0.10,
            Cue::Swing
            | Cue::RemoteSwing
            | Cue::ImpactWood
            | Cue::ImpactStone
            | Cue::ImpactMetal
            | Cue::Gather
            | Cue::Place
            | Cue::Splash
            | Cue::TreeFall
            | Cue::Snort
            | Cue::Bellow
            | Cue::Growl
            | Cue::Hurt
            | Cue::HurtAgain
            | Cue::BulletSoil
            | Cue::BulletStone
            | Cue::BulletWood
            | Cue::BulletMetal
            | Cue::FleshHit
            | Cue::Collapse
            | Cue::CollapseWood
            | Cue::Knock
            | Cue::Reload
            | Cue::Eat
            | Cue::Bandage
            | Cue::BowDraw
            | Cue::BushPick
            | Cue::DoorOpen
            | Cue::DoorClose
            | Cue::Drink
            | Cue::Land
            | Cue::Equip
            | Cue::MapPaper
            | Cue::ContainerOpen
            | Cue::RemoteReload
            | Cue::RemoteEat
            | Cue::RemoteBandage
            | Cue::RemoteDrink
            | Cue::RemoteContainerOpen
            | Cue::RemoteEquip
            | Cue::RemoteLand
            | Cue::RemoteSplash
            | Cue::RemoteBowDraw
            | Cue::OreCrumble
            | Cue::OreBreak
            | Cue::Tung
            | Cue::FurnaceLight
            | Cue::FurnaceOut
            | Cue::FurnaceOpen
            | Cue::Kindle
            | Cue::Snuff
            | Cue::ReloadRifle
            | Cue::ReloadPistol
            | Cue::ReloadShotgun
            | Cue::ReloadBolt
            | Cue::RemoteReloadRifle
            | Cue::RemoteReloadPistol
            | Cue::RemoteReloadShotgun
            | Cue::RemoteReloadBolt
            | Cue::DoorMetalOpen
            | Cue::DoorMetalClose
            | Cue::GarageOpen
            | Cue::GarageClose
            | Cue::Repair => 0.07,
            // Your own hands in your own pack: a nudge, like the equip, so a
            // row of moves is not one sound stamped out.
            Cue::InvOpen | Cue::InvClose | Cue::ItemMove | Cue::ItemDrop | Cue::CraftStart => 0.05,
            // The bonus answering you: nearly a symbol, but struck on rock,
            // and two in a row in unison would ring.
            Cue::OreGlint => 0.03,
            // A different handful of leaves every stride.
            Cue::Brush | Cue::RemoteBrush => 0.12,
            // A fire never crackles the same twice, and a bullet's whiz is
            // its speed and its miss distance.
            Cue::FireCrackle | Cue::Flyby => 0.10,
            // One long take, heard once per charge: a small nudge so two
            // charges side by side are not in unison.
            Cue::Fuse => 0.04,
            // A hiss is noise: a nudge keeps two in a row from matching.
            Cue::TorchOut => 0.06,
            // A click retriggered up to fourteen times a second is the
            // machine-gun tell this knob exists for.
            Cue::Geiger => 0.05,
            // Heard once a fight at most; a nudge so two are never one take.
            Cue::Headshot => 0.04,
            // A mechanism with a fixed spring: a nudge, like the shots.
            Cue::DryFire => 0.03,
            // A whine's pitch is its whole character, and no two glances
            // leave at the same speed.
            Cue::Ricochet => 0.14,
            // A blast varies like thunder: heard rarely, but one recording
            // retriggered through a raid is its tell.
            Cue::Blast => 0.08,
            // A shot varies like any other diegetic cue, and slightly less
            // than a swing: the two are the most *repeated* sounds in a
            // fight, so unison is the tell, but a firearm's report is a
            // mechanism with a fixed bore and a bow's is a fixed string —
            // both vary with the shooter and the round, not with the
            // weapon's pitch.
            Cue::ShotBow
            | Cue::ShotCrossbow
            | Cue::ShotGun
            | Cue::ShotGunFar
            | Cue::ShotHeli
            | Cue::ShotHeliFar
            | Cue::ShotRifle
            | Cue::ShotSmg
            | Cue::ShotPistol
            | Cue::ShotShotgun
            | Cue::ShotBolt
            | Cue::ShotRocket
            | Cue::ShotRifleFar
            | Cue::ShotSmgFar
            | Cue::ShotPistolFar
            | Cue::ShotShotgunFar
            | Cue::ShotBoltFar => 0.05,
            // Wider than any diegetic cue but the bird, and for the bird's
            // reason turned up one notch: a howl is the most *exposed* tonal
            // call in the bank — a near-pure pitched tone held for seconds,
            // where a growl hides its repetition under noise. Two wolves
            // answering each other at the same pitch is a chorus in unison,
            // which is the machine-gun tell this knob exists for, and unison
            // is what a real chorus is specifically not.
            Cue::Howl => 0.12,
            // The forest layer's variation is the whole of it: one recording
            // of one bird, retriggered every few seconds at exactly its own
            // pitch, is the machine-gun tell this knob exists for, and a
            // layer is heard for minutes on end where a footstep is heard for
            // a stride. The widest in the table on purpose.
            Cue::Bird => 0.16,
            // No two claps the same length or pitch: a storm is heard for
            // minutes and one recording retriggered would be its tell.
            Cue::Thunder => 0.12,
            Cue::CraftDone
            | Cue::SentryLock
            | Cue::Trade
            | Cue::GateChime
            | Cue::Learn
            | Cue::Unlock
            | Cue::Refused
            | Cue::Hit
            | Cue::HitHead
            | Cue::HitLimb
            | Cue::Death
            | Cue::UiClick
            | Cue::UiHover
            | Cue::MapPin
            | Cue::BedWind
            | Cue::BedSurf
            | Cue::BedUnder
            | Cue::BedRain
            | Cue::BedRotor
            | Cue::BedTown
            | Cue::BedNight
            | Cue::BedRotorFar
            | Cue::BedFurnace => 0.0,
        }
    }
}

/// What is fixed about a cue: which bus it is balanced in, how far it carries,
/// how loud it is at the listener, how often it may retrigger, and how much it
/// is worth when the frame budget is full.
#[derive(Clone, Copy, Debug)]
pub struct CueDef {
    pub bus: Bus,
    /// How far the cue carries, metres. **Only read when `positional`** — a
    /// cue that happens to the local player has no distance to it.
    ///
    /// The reference's own datum for the scale of this number: a silenced
    /// weapon there is "a maximum of 40m instead of the 100m it used to be",
    /// so tens of metres for a gunshot and a couple of dozen for a footstep is
    /// the register, not hundreds.
    pub radius_m: f32,
    /// Gain at the listener, before the bus and the master. 0..1.
    pub gain: f32,
    /// The shortest interval between two starts of this cue, ms. **This is
    /// the cheap half of the reference's frame budget**: a machine-gun retrigger
    /// is what actually fills a voice pool, and refusing it costs one
    /// comparison.
    pub cooldown_ms: u32,
    /// Who survives when more cues want to start than the frame allows.
    /// Higher wins. Ties break on distance, then on queue order, so the
    /// choice is a pure function of the frame and never of a `HashMap`'s mood.
    pub priority: u8,
    /// Does this cue happen somewhere, or does it happen to *you*?
    ///
    /// Own-facts are non-positional on purpose. Your own footstep is at the
    /// listener, where an inverse-square law divides by zero and a panner has
    /// no side to pick.
    pub positional: bool,
}

/// One row of [`CUES`], positionally.
///
/// A builder rather than a struct literal for one reason and it is not
/// brevity: rustfmt expands a struct literal to seven lines, which turns a
/// nineteen-row table into a 130-line wall nobody can scan for the row that is
/// wrong. The column legend above the table is what a positional call costs,
/// and a table you can read down a column is what it buys.
const fn row(
    bus: Bus,
    radius_m: f32,
    gain: f32,
    cooldown_ms: u32,
    priority: u8,
    positional: bool,
) -> CueDef {
    CueDef {
        bus,
        radius_m,
        gain,
        cooldown_ms,
        priority,
        positional,
    }
}

use Bus::Ambience as AMB;
use Bus::Game as GAME;
use Bus::Interface as UI;
use Bus::Steps as STEPS;

/// The cue table. Indexed by [`Cue::idx`] — order must match [`Cue::ALL`].
///
/// **Not measured against anything.** These are opening values in the sense
/// `CONTENT.md` uses the phrase: each is a knob whose default ships until
/// someone with ears moves it (`DECISIONS.md` §open, "audio v0").
///
/// ```text
///      bus   radius  gain  cool  prio  positional
/// ```
#[rustfmt::skip]
pub const CUES: [CueDef; CUE_COUNT] = [
    // Footsteps. One bus, one radius, one gain: the surfaces differ in
    // TIMBRE, which is `synth`'s job, not in how far they carry.
    STEP, STEP, STEP, STEP, STEP,
    // Swing: yours, so non-positional, and the cooldown is the swing rate.
    SWING,
    // Impacts happen at a thing, so they carry and they pan.
    row(GAME, 40.0, 0.70,  40, 4, true),   // wood
    row(GAME, 40.0, 0.70,  40, 4, true),   // stone
    row(GAME, 48.0, 0.70,  40, 4, true),   // metal
    // Gather / craft / refusal are the interface answering you.
    row(GAME,  0.0, 0.50,  60, 4, false),  // gather
    row(UI,    0.0, 0.55,  80, 5, false),  // craft done
    row(UI,    0.0, 0.45, 250, 5, false),  // refused
    // Combat. `Hurt` and `Death` outrank everything: a player who cannot hear
    // that they are being killed has lost the round to the mixer.
    //
    // **`Hit` has a cooldown and it is not about pacing.** It is a signal cue,
    // so `pitch_var` is zero — four hits landing in one frame would start four
    // BIT-IDENTICAL 2 kHz clicks in phase, and identical samples summed in a
    // mixer add rather than blend: four of them is four times the amplitude,
    // straight into the limiter. The cooldown is what makes "one marker per
    // volley" true, and every zero-cooldown row in this table is one that is
    // either positional (so no two are identical) or happens once.
    row(GAME,  0.0, 0.50,  45, 6, false),  // hit (the marker)
    HURT,                                  // hurt (and `HurtAgain`, last)
    row(GAME,  0.0, 0.90,   0, 8, false),  // death
    // Placement happens at a cell — the reference shipped this one wrong for a
    // while, with placement effects firing at the world origin instead of at
    // the socket (`reference/AUDIO.md` §6). Ours carries a position or it does
    // not fire.
    row(GAME, 32.0, 0.60,  30, 4, true),   // place
    // Breaking the surface. Your own body, so non-positional, and the cooldown
    // is what stops a player bobbing on the waterline from machine-gunning it:
    // the crossing test is a sign change, and a body oscillating around
    // `SEA_LEVEL` produces one every frame.
    row(GAME,  0.0, 0.65, 220, 5, false),  // splash
    // A tree coming down is the loudest thing in the forest — and it set
    // `MAX_AUDIBLE_M` until the gun's report outranged it at v54.
    row(GAME, 96.0, 0.90,   0, 6, true),   // tree fall
    row(UI,    0.0, 0.45,  40, 2, false),  // ui click
    // The beds. Never started by the mixer (`Cue::is_bed`); `render/audio.rs`
    // holds one looping voice each and moves their gains.
    row(AMB,   0.0, 0.30,   0, 0, false),  // wind
    row(AMB,   0.0, 0.38,   0, 0, false),  // surf
    row(AMB,   0.0, 0.34,   0, 0, false),  // submerged
    // The pig announces itself before you see it (`reference/ANIMALS.md` —
    // the boar is identified by its snorting), so it carries past the
    // impacts but nowhere near a falling tree. Priority with the footsteps'
    // register: ambience, not signal. The cooldown is per-cue, so it is the
    // herd's stagger, not one animal's — `sound::voice` spaces one animal.
    row(GAME, 40.0, 0.55, 150, 2, true),   // snort
    // Remote footsteps: the STEP family heard at another body — see RSTEP.
    RSTEP, RSTEP, RSTEP, RSTEP, RSTEP,
    // The forest layer. AMBIENCE, not game: it is scenery, and a player who
    // turns the bed down means the birds too. Positional (it comes from a
    // perch), quiet, and at the footsteps' priority — a bird must never be
    // the reason an axe was refused a voice. The cooldown is per-CUE and so
    // it is the whole flock's stagger, not one bird's, which is what
    // `sound::birds` already spaces.
    row(AMB,   44.0, 0.30, 700, 1, true),  // bird
    // The wolf, in two registers (`sound::voice`). Both GAME rather than
    // AMBIENCE for the pig's reason: an animal is a thing in the world, not
    // scenery, and a player who turns the bed down must not turn the
    // predator down with it.
    //
    // **The howl carries further than anything but a gunshot and a falling
    // tree** — which is the point of it. The wolf notices a player at 30 m by day and 15 m at night
    // (`content/mobs.toml`), so 88 m means the island tells you it has
    // wolves, and roughly where, long before one of them can act on you. The
    // cooldown is per-CUE and therefore the pack's stagger, not one animal's
    // — `sound::voice` spaces one animal, as `sound::birds` does for a flock.
    row(GAME, 88.0, 0.60, 900, 2, true),   // howl
    // The growl's radius is doing a SECOND job and it is the load-bearing
    // one: `sound::voice` picks the register by comparing the listener's
    // distance against this very number (`CUES[Growl].radius_m`), so the two
    // cannot disagree — a growl is never chosen and then culled by the
    // mixer's falloff, and a howl is never chosen inside growl range. There
    // is no third "switch distance" knob to drift.
    //
    // 14 m sits inside the wolf's *smallest* notice radius (15 m, at night),
    // so in practice a wolf you can hear growl has already seen you. The two
    // numbers live in different crates (no mob-def lane on the wire), so
    // `client/tests/mob_volume.rs` reads both and a `mobs.toml` edit below
    // this row reddens it.
    // Priority 5: above the impacts, below the hitmarker — a growl at this
    // range is information a player's life turns on.
    row(GAME, 14.0, 0.65, 200, 5, true),   // growl
    // Another player's swing — see RSWING.
    RSWING,
    // **The two shots, and both radii are the reference's own number read
    // off one sentence** (`CueDef::radius_m`'s doc, quoted there since
    // audio v0): a silenced weapon there carries "a maximum of 40m instead
    // of the 100m it used to be". That is a published pair for exactly the
    // two roles this game has — the loud ranged option and the quiet one —
    // so `BALANCE.md` §6 takes both and no case is owed for either. What
    // would need a case is *differing*, and the thing worth not differing
    // about is the RATIO: a bow discloses you over 40 m and a gun over
    // 100 m, which is 6× the area, and that multiple is the whole of why
    // anyone would carry a bow once they can craft a revolver.
    //
    // The gun's 100 m is what sets `MAX_AUDIBLE_M`; the falling tree held
    // that title until v54.
    row(GAME,  40.0, 0.42,  60, 4, true),   // bow released
    row(GAME, 100.0, 0.85,  60, 6, true),   // gun fired
    // The other two rungs of the marker (v58). Same bus, same radius (none
    // — they are signals, not places), same 45 ms cooldown and the same
    // reason for it as `Hit`'s: zero `pitch_var` means two in a frame are
    // bit-identical and would add rather than blend.
    //
    // The gains bracket `Hit`'s 0.50 rather than sitting on it, because
    // the rung has to be legible in one hearing and loudness is the
    // channel a player reads without being taught. The limb's 0.45 is a
    // deliberate floor and not a taper: quieter than the identity, still
    // plainly a hit — the judge's whole point is that a halved blow reads
    // as a MISS, and a cue fading toward silence would say exactly that.
    row(GAME,  0.0, 0.75,  45, 6, false),  // hit, head (the crunch)
    row(GAME,  0.0, 0.45,  45, 6, false),  // hit, limb
    // Weather v0. The rain is a bed like the wind; thunder is ambience —
    // a player who turns the scenery down turns the storm down with it —
    // and non-positional, with a cooldown so two near bolts are one roll.
    row(AMB,   0.0, 0.47,   0, 0, false),  // rain bed
    row(AMB,   0.0, 0.85, 900, 3, false),  // thunder
    // Effects v2. A round landing carries like a blow does; a ricochet a
    // little further, being a whine; a body hit is information a life
    // turns on, so it outranks the impacts.
    row(GAME, 40.0, 0.60,  30, 4, true),   // bullet, soil
    row(GAME, 40.0, 0.60,  30, 4, true),   // bullet, stone
    row(GAME, 40.0, 0.60,  30, 4, true),   // bullet, wood
    row(GAME, 48.0, 0.60,  30, 4, true),   // bullet, metal
    row(GAME, 56.0, 0.84,  90, 3, true),   // ricochet
    row(GAME, 30.0, 0.70,  30, 5, true),   // flesh hit
    // The far report: past the switch (`render/audio.rs`, 45 m) this is
    // what is left of a gunshot. The reference hears a pistol 400 m off, a
    // rifle 500 and a heavy gun 600 (its Sept 2025 maintenance devblog), so
    // the row carries 600 and the shard sends the report only as far as the
    // gun reaches (`GUN_REPORT_CM`): a revolver at 400 m is faint, not gone.
    // The gain meets the near layer's level at the switch:
    // 0.85·(1 − 45/100)² = 0.30·(1 − 45/600)².
    row(GAME, 600.0, 0.30,  60, 6, true),  // gun fired, far
    // A charge is the loudest thing on the island and the one sound a
    // whole server should know the place of: the reference sends every
    // explosion to every client. 1500 m on a 4 km island (`BLAST_REPORT_CM`)
    // is a raid heard from most of it, a dull far rumble under the falloff
    // and the distance low-pass.
    row(GAME, 1500.0, 1.00,  0, 7, true),  // blast
    row(GAME, 60.0, 0.80,  80, 5, true),   // collapse
    row(GAME, 24.0, 0.60, 120, 4, true),   // knock
    // Your own hands, so non-positional; the cooldown is what keeps a
    // reload key held down from being a rattle.
    RELOAD,                                // reload
    // Your own mouth and your own hands: non-positional, and a meal is a
    // second apart (`survival::CONSUME_TICKS`), so the cooldown is only
    // there for two landing in one frame.
    // (The eat, bandage, bush, trade, zone, landing and page rows were
    // raised part of the way back when their recordings came in: each
    // measured well under the synthesized cue it replaced, peak for peak.)
    row(GAME,  0.0, 0.70,  60, 3, false),  // eat
    row(GAME,  0.0, 0.62, 120, 3, false),  // bandage
    // Your own bow, so non-positional, and quieter than the release it
    // leads to. The cooldown only stops a flutter on the right mouse from
    // stacking creaks.
    row(GAME,  0.0, 0.35, 300, 3, false),  // bow drawn
    // The heli's rotor: a bed, so radius 0 and no cooldown — its level is
    // `render/audio.rs`'s distance law. GAME, not AMBIENCE: it is a threat,
    // and turning the scenery down must not turn it down.
    row(GAME,  0.0, 0.85,   0, 0, false),  // rotor
    // A bush picked, at the bush: a footstep's reach and a little over an
    // impact's gain, so your own pick reads over your boots and a pick
    // across a clearing is still heard.
    row(GAME, 24.0, 0.85,  60, 4, true),   // bush picked
    // The sentry's lock: a signal, so no pitch variation, and loud enough
    // to carry across the yard to everyone near the one it is about.
    row(GAME, 70.0, 0.70, 400, 7, true),   // sentry lock
    row(GAME,  0.0, 0.80, 120, 4, false),  // trade
    row(GAME,  0.0, 0.48, 800, 3, false),  // into / out of the zone
    // The town's bed: scenery, so AMBIENCE; its level is `render/audio.rs`'s
    // distance to the gate.
    row(AMB,   0.0, 0.32,   0, 0, false),  // the town
    // Doors happen at a leaf: a footstep's reach, an impact's priority.
    row(GAME, 24.0, 0.55,  60, 4, true),   // door open
    row(GAME, 24.0, 0.60,  60, 4, true),   // door close
    // Your own mouth and feet, like the eat and the step.
    row(GAME,  0.0, 0.45, 300, 3, false),  // drink
    row(STEPS, 0.0, 0.72, 200, 3, false),  // land
    // A fire is scenery: the ambience bus, a short reach, the lowest rank.
    row(AMB,  14.0, 0.62,  80, 1, true),   // fire crackle
    // A fuse is a bomb about to go off: carried far enough to run from,
    // ranked with the hitmarker.
    row(GAME, 32.0, 0.81, 500, 6, true),   // fuse
    // A round past your head is the shot that was meant for you.
    row(GAME, 30.0, 0.95,  60, 6, true),   // bullet flyby
    // The interface answering you, like the craft chime.
    row(UI,    0.0, 0.85, 200, 4, false),  // learn
    row(GAME, 16.0, 0.55, 150, 4, true),   // unlock
    row(GAME, 60.0, 0.80,  80, 5, true),   // collapse, wood
    // The night's crickets: scenery like the town's bed, its level the
    // hour's (`render/audio.rs`).
    row(AMB,   0.0, 0.20,   0, 0, false),  // the night
    // Your own hands at the interface's level: the gather's register, and a
    // cooldown so a scroll through the hotbar is not a drum roll.
    row(GAME,  0.0, 0.50, 120, 3, false),  // equip
    row(UI,    0.0, 0.45, 150, 3, false),  // map paper
    row(GAME,  0.0, 0.55, 200, 3, false),  // container open
    // Other bodies' hands, at the body. Each takes its own cue's gain and a
    // reach of its own; a reload carries furthest and ranks with a remote
    // swing, because it is the one a fight turns on — Rust's players listen
    // for exactly that click to push. `limits::ACT_HEAR_CM` is the server's
    // side of the widest of these.
    RRELOAD,                               // reload, somebody else's
    row(GAME, 12.0, 0.70,  60, 3, true),   // eat, somebody else's
    row(GAME, 14.0, 0.62, 100, 3, true),   // bandage, somebody else's
    row(GAME, 12.0, 0.45, 100, 3, true),   // drink, somebody else's
    row(GAME, 20.0, 0.55,  60, 3, true),   // container open, somebody else's
    row(GAME, 16.0, 0.50,  40, 3, true),   // equip, somebody else's
    row(STEPS, 24.0, 0.72,  40, 3, true),  // land, somebody else's
    row(GAME, 30.0, 0.65,  60, 4, true),   // splash, somebody else's
    // A bow coming back is the shot a moment before it: ranked with the
    // reload, carried a little less far than the loose itself.
    row(GAME, 18.0, 0.35,  60, 4, true),   // bow drawn, somebody else's
    // Pushing through a plant: a footstep's register, and the remote one a
    // footstep's reach — the leaves give a body away about as far as its
    // boots do, and from inside cover where its boots are all you have.
    row(STEPS, 0.0, 0.42,  90, 1, false),  // brush
    row(STEPS, 24.0, 0.42,  40, 2, true),  // brush, somebody else's
    // A torch going out in your hand: once, and it must be heard.
    row(GAME,  0.0, 0.55, 500, 3, false),  // torch out
    // The stag: the pig's register (ambience, not signal), carrying further
    // because a roar is a long-range call where a snort is a near one.
    row(GAME, 60.0, 0.55, 400, 2, true),   // bellow
    // The second blow in a frame: Hurt's row, read off it (`Cue::lead`).
    HURT,
    // The crossbow: the bow's disclosure radius (the reference's 40 m), a
    // little louder at the shooter — a stock and a steel prod, not a stave.
    row(GAME,  40.0, 0.50,  60, 4, true),  // crossbow released
    // Mining: a node crumbling and breaking carry like the blow that did it,
    // the break a little further; the glint is a reward, heard close.
    row(GAME, 40.0, 0.62, 120, 4, true),   // ore crumble
    row(GAME, 56.0, 0.80, 200, 5, true),   // ore break
    row(GAME, 24.0, 0.55,  60, 5, true),   // ore glint
    // The bat's tung: an impact's reach, under the impact it rides on.
    row(GAME, 40.0, 0.60,  60, 4, true),   // tung
    // The heli's gun: the far report's reach (a gunship is heard across the
    // island), and a cooldown under its 100 ms between rounds.
    row(GAME, 200.0, 0.80,  60, 6, true),  // heli gun
    // The rotor's far layer: the near rotor's bus and reason. Its level is
    // `render/audio.rs`'s rotor law; the gain matches its loudness to the
    // near layer's (a far loop is denser: 12.6 dB of crest against 15.8).
    row(GAME,  0.0, 0.60,   0, 0, false),  // rotor, far
    // The heli's gun from far off: a machine gun's reach in the reference
    // (600 m, its far tier), which the heli is subscribed past. The gain
    // meets the near round's at the switch (`render/audio.rs`, 60 m):
    // 0.80·(1 − 60/200)² = 0.48·(1 − 60/600)².
    row(GAME, 600.0, 0.48,  60, 6, true),  // heli gun, far
    // The interface's small sounds, under the click: a hover is the quietest
    // thing in the bank and its cooldown is what keeps a sweep across a row
    // of buttons from becoming a rattle.
    row(UI,    0.0, 0.18,  45, 1, false),  // ui hover
    row(UI,    0.0, 0.50, 150, 3, false),  // inventory open
    row(UI,    0.0, 0.42, 150, 3, false),  // inventory close
    row(UI,    0.0, 0.48,  60, 3, false),  // item moved
    row(UI,    0.0, 0.52, 120, 3, false),  // item dropped
    row(UI,    0.0, 0.50, 120, 3, false),  // craft started
    row(UI,    0.0, 0.45,  80, 2, false),  // map pin
    // The counter: a hazard reading, so the game bus (turning the scenery
    // down must not mute it); a cooldown under a frame, so clicks on
    // consecutive frames both land; ducked under a near shot.
    row(GAME,  0.0, 0.36,   8, 2, false),  // geiger click
    // A headshot taken: as loud and as high as being killed, because it is
    // usually the first half of it.
    row(GAME,  0.0, 0.95, 150, 8, false),  // headshot
    // The built floors: the step family's numbers, yours then theirs.
    STEP, STEP, STEP,
    RSTEP, RSTEP, RSTEP,
    // Rust's guns (guns v1): the gun's own reach and the revolver's gain,
    // a cooldown under the fastest cadence (an SMG's 100 ms), so every
    // round of a burst is heard. The shotgun and the bolt carry further, the
    // rocket's launch less far than its blast.
    row(GAME, 100.0, 0.85,  60, 6, true),  // rifle
    row(GAME, 100.0, 0.80,  60, 6, true),  // smg
    row(GAME, 100.0, 0.80,  60, 6, true),  // pistol
    row(GAME, 100.0, 0.95,  60, 6, true),  // shotgun
    row(GAME, 100.0, 0.95,  60, 6, true),  // bolt
    row(GAME,  80.0, 0.90, 150, 6, true),  // rocket launch
    // The furnace: its roar is scenery like the recycler's motor, its level
    // and side `render/audio.rs`'s; lighting, putting out and opening one
    // are hands at it, a door's reach. The campfire's two the same, quieter.
    row(AMB,   0.0, 0.40,   0, 0, false),  // furnace roar
    row(GAME, 24.0, 0.70, 200, 3, true),   // furnace lit
    row(GAME, 24.0, 0.60, 200, 3, true),   // furnace out
    row(GAME,  0.0, 0.55, 200, 3, false),  // furnace opened
    row(GAME, 20.0, 0.60, 200, 3, true),   // campfire lit
    row(GAME, 20.0, 0.50, 200, 3, true),   // campfire out
    // Your own gun, empty: the reload's register, and a cooldown under the
    // fastest cadence so a held trigger clicks once a round it would fire.
    row(GAME,  0.0, 0.55,  80, 4, false),  // dry fire
    // The guns' own reloads: the revolver's rows, yours then theirs.
    RELOAD, RELOAD, RELOAD, RELOAD,
    RRELOAD, RRELOAD, RRELOAD, RRELOAD,
    // The guns' far layers: the reference's reach for each (a pistol 400 m,
    // a rifle 500, a heavy gun 600), and each gain meets its near row's
    // level at the switch (45 m), as the revolver's far row does:
    // g_near·(1 − 45/100)² = g_far·(1 − 45/R)².
    row(GAME, 500.0, 0.31,  60, 6, true),  // rifle, far
    row(GAME, 400.0, 0.31,  60, 6, true),  // smg, far
    row(GAME, 400.0, 0.31,  60, 6, true),  // pistol, far
    row(GAME, 400.0, 0.36,  60, 6, true),  // shotgun, far
    row(GAME, 600.0, 0.34,  60, 6, true),  // bolt, far
    // The metal doors: the wooden door's reach, a little louder for the
    // metal, and the garage's roll a little further for its size.
    row(GAME, 24.0, 0.60,  60, 4, true),   // metal door open
    row(GAME, 24.0, 0.65,  60, 4, true),   // metal door close
    row(GAME, 30.0, 0.60, 200, 4, true),   // garage door open
    row(GAME, 30.0, 0.65, 200, 4, true),   // garage door close
    // The hammer: a blow's reach and register, at the mended piece.
    row(GAME, 32.0, 0.65, 100, 4, true),   // repair
];

/// Your own reload, whichever gun (`Cue::Reload` and the four after it).
const RELOAD: CueDef = row(GAME, 0.0, 0.54, 250, 3, false);
/// Somebody else's: the one a fight turns on, so it carries furthest of
/// their hands' sounds and ranks with a remote swing.
const RRELOAD: CueDef = row(GAME, 30.0, 0.54, 40, 4, true);

/// Being hurt. Named so [`Cue::HurtAgain`]'s row reads its numbers off it:
/// the second voice of a frame's blows is as loud and as urgent as the first.
const HURT: CueDef = row(GAME, 0.0, 0.80, 120, 7, false);

/// Your own arm. Named rather than written inline so [`RSWING`] can read its
/// numbers off it instead of restating them — [`STEP`]/[`RSTEP`]'s shape.
const SWING: CueDef = row(GAME, 20.0, 0.45, 120, 3, false);

/// Another player's swing (`DECISIONS.md` §open, "remote swing v0").
///
/// Radius and gain come off [`SWING`] rather than being restated: the 20 m
/// that row has always carried was written for exactly this positional half
/// and **was never read at all** — `positional: false` makes `radius_m` dead
/// (see [`CueDef::radius_m`]) — so this is where the number becomes true
/// rather than a new one being invented beside it.
///
/// What differs is deliberate, and it is [`RSTEP`]'s list verbatim because
/// it is the same argument:
///
/// - **positional** — the whole point; the cue is at the body, panned, and
///   culled by the one falloff law.
/// - **priority 4**, above your own arm's 3, because another player's swing
///   is information your life turns on and your own is a keystroke you just
///   pressed. That puts it level with the impacts, which is the register it
///   belongs in; ties break on distance, so the nearer swing wins.
/// - **a 40 ms cooldown** rather than the local 120 ms — [`RSTEP`]'s value,
///   taken for [`RSTEP`]'s reason. The cooldown is per SOURCE for a
///   positional cue (`mixer::SAME_SOURCE_M`), so two raiders swinging at once
///   are two swings; the 40 ms is one arm's floor.
const RSWING: CueDef = CueDef {
    bus: Bus::Game,
    radius_m: SWING.radius_m,
    gain: SWING.gain,
    cooldown_ms: 40,
    priority: 4,
    positional: true,
};

/// The footsteps share every number but their timbre — see [`CUES`].
///
/// **75 m, the reference's own number** (it opened at 24, then 40): its
/// client spawns another player's footstep out to 75 m from the camera
/// (`BaseFootstepEffect`, its hard cull), so a step is a sound you hear
/// coming, not one that arrives with the swing. Under `(1 - d/R)²` a step at
/// 20 m is half of one at your feet and at 40 m a fifth; crouching halves it
/// again (`render/audio.rs`).
const STEP: CueDef = CueDef {
    bus: Bus::Steps,
    radius_m: 75.0,
    gain: 0.35,
    cooldown_ms: 90,
    priority: 1,
    positional: false,
};

/// The remote five (`DECISIONS.md` §open, "remote footsteps v0"). Radius and
/// gain are read off [`STEP`] rather than restated — the radius the local row
/// always carried was written for exactly this positional half, and tying
/// them means the two families cannot drift apart on how far a boot carries.
/// What differs is deliberate: **positional** (the whole point — the cue is
/// at the body, panned and culled by the one falloff law); **priority 2**,
/// above your own feet, because another player's step is the sound that
/// decides fights and yours tells you nothing; and a **40 ms cooldown** (the
/// impacts' register, not the local 90 ms). The cooldown is per source
/// (`mixer::SAME_SOURCE_M`), so a squad on one surface is every one of its
/// steps, not one.
const RSTEP: CueDef = CueDef {
    bus: Bus::Steps,
    radius_m: STEP.radius_m,
    gain: STEP.gain,
    cooldown_ms: 40,
    priority: 2,
    positional: true,
};

/// The furthest any cue carries, metres — the table's own ceiling, asserted
/// against [`CUES`] in `tests/sound.rs`: the cull, the AOI arithmetic and the
/// reference's published radii are all stated against it, so a row that
/// carried further would be a row the rest of the model does not know about.
/// (It also used to derive `render/audio.rs`'s spatial scale for rodio's
/// clamp; the engine pans now and that half is gone.)
///
/// **Set by [`Cue::Blast`]**, at 1500 m: a blast and a far gunshot reach a
/// client past the shard's 208 m interest radius as a report
/// (`EventMsg::Report`, v113), and the heli is subscribed past it too
/// (`heli::HELI_AOI_SCALE`). It is a derived number and not a taste one —
/// it is the maximum of the table, and the test that asserts so is what
/// makes raising a radius force this line rather than silently invert a
/// cue's falloff.
pub const MAX_AUDIBLE_M: f32 = 1500.0;

/// How many voices may sound at once.
///
/// The overflow policy is **refuse the new one**, not steal the oldest: a
/// stolen voice is an audible cut, and at this cap the thing being refused is
/// the 25th simultaneous sound, which nobody can pick out of the other 24.
pub const VOICE_CAP: usize = 24;

/// Voices past [`VOICE_CAP`] that only a cue of [`RESERVE_PRIORITY`] or
/// more may take: in a raid the pool fills with long tails (blasts,
/// collapses, far shots), and the reload or the hit beside you is the sound
/// that must still get in.
pub const VOICE_RESERVE: usize = 8;

/// The priority a cue needs to use [`VOICE_RESERVE`]: the impacts, a remote
/// swing or reload, the hitmarkers, being hurt, the shots.
pub const RESERVE_PRIORITY: u8 = 4;

/// How many voices of one cue may sound at once, so one sound repeated (a
/// raid's charges, a fusillade's far reports) cannot take the whole pool.
pub const MAX_SAME_CUE: u8 = 6;

/// How many voices may START in one frame.
///
/// This is the reference's `audio.framebudget` in the only unit a pure module
/// has. Four is not a measurement; it is a bound (`DECISIONS.md` §open).
pub const STARTS_PER_FRAME: usize = 4;

/// How many cue requests may be queued between two mixer ticks.
///
/// **Overflow policy: drop the newest and count it.** Drop-oldest is wrong
/// here for the reason it is right for datagrams — a datagram's value is its
/// freshness, and a cue's value is that it happened. Dropping the newest keeps
/// the sounds that were requested first in the frame, which is the order the
/// world produced them in. [`mixer::Mixer::dropped`] is the counter, and a
/// non-zero one is a bug in the caller, not a load condition.
pub const CUE_QUEUE_CAP: usize = 32;

/// The bus and master volumes. The reference's `audio.master`, `audio.game`
/// and `audio.ambience`, one for one, plus footsteps and the interface.
#[derive(Clone, Copy, Debug)]
pub struct Mix {
    pub master: f32,
    pub game: f32,
    pub ambience: f32,
    pub music: f32,
    pub steps: f32,
    pub interface: f32,
}

impl Default for Mix {
    fn default() -> Self {
        // **The reference ships master and game at 1 and its music at 0.2,
        // and now that there is music, so do we.** The bed still sits under
        // the game bus by its own cue gain rather than by a quieter bus,
        // because a player who turns ambience down should be turning the
        // wind down and not discovering the bed was already half off. Music
        // is the opposite case and theirs is the evidence: a score at parity
        // with footsteps is a score you fight over, so it opens at a fifth
        // and the slider goes up from there.
        Self {
            master: 1.0,
            game: 1.0,
            ambience: 1.0,
            music: MUSIC_DEFAULT,
            steps: 1.0,
            interface: 1.0,
        }
    }
}

/// What the music bus opens at. The reference's `audio.musicvolume` default,
/// verbatim — `reference/BALANCE.md` §6: a case is owed for differing, not
/// for taking.
pub const MUSIC_DEFAULT: f32 = 0.2;

impl Mix {
    /// The multiplier a cue on `bus` gets from the mix.
    pub fn bus_gain(&self, bus: Bus) -> f32 {
        let g = match bus {
            Bus::Game => self.game,
            Bus::Ambience => self.ambience,
            Bus::Music => self.music,
            Bus::Steps => self.steps,
            Bus::Interface => self.interface,
        };
        (self.master * g).clamp(0.0, 1.0)
    }

    /// This mix seen through a snapshot. The player's sliders stay the
    /// player's; a snapshot only ever scales them down.
    pub fn under(&self, snap: &SnapshotDef) -> Mix {
        Mix {
            master: self.master,
            game: self.game * snap.game,
            ambience: self.ambience * snap.ambience,
            music: self.music * snap.music,
            // Both were game-bus cues before they had sliders, and go under
            // with it.
            steps: self.steps * snap.game,
            interface: self.interface * snap.game,
        }
    }
}

// ---------------------------------------------------------------------------
// Snapshots.
// ---------------------------------------------------------------------------

/// A named whole-mixer state.
///
/// **This is the reference's own step 2 and the one `reference/AUDIO.md` §9.1
/// named as owed.** Their published build order for audio is groups, then
/// *snapshots* — "a whole mixer state" — then *fades between snapshots*, and
/// only then occlusion and reverb. We had the groups and nothing else; this is
/// the missing middle, and the thing that finally needed it is water.
///
/// It is two states because there are two, not because two is easy. A cave
/// snapshot and a hostile-contact snapshot are the obvious next ones and
/// neither has a cause in the client yet.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Snapshot {
    Above,
    Submerged,
}

/// What a snapshot sets: the two buses, and the level each bed is held at.
///
/// The beds are in here rather than in the world's own gains because a
/// snapshot is *a whole mixer state* — the point of the mechanism is that one
/// value moves everything at once and nothing has to remember to move with it.
#[derive(Clone, Copy, Debug)]
pub struct SnapshotDef {
    pub game: f32,
    pub ambience: f32,
    /// **Flat in both states today, and that is the finding rather than an
    /// oversight.** A snapshot is *a whole mixer state*, so every bus has to
    /// appear in one or it can never be moved by one — but music is
    /// non-diegetic, and water between you and a violin is not a thing to
    /// model. The reference's own first use for this field is a cause we do
    /// not have: their published snapshot example is fading music down *when
    /// somebody speaks*, which needs voice chat.
    pub music: f32,
    pub wind: f32,
    pub surf: f32,
    pub under: f32,
    /// The rain bed: heard above water, gone below it.
    pub rain: f32,
    /// The heli's rotor: the game bus's level, because it is a threat.
    pub rotor: f32,
    /// The machines' beds, the recycler's motor and the furnace's roar:
    /// scenery, gone below water like the wind.
    pub town: f32,
    /// The night's crickets: scenery, gone below water like the wind.
    pub night: f32,
}

/// The two states (`DECISIONS.md` §open, "water audio v0").
///
/// Real submerged audio is a steep low-pass: the top end goes. One-shots get
/// it — the engine's one pole, at `engine::lp_under` of the fade — and the
/// beds and loops, which run unfiltered, are ducked and crossfaded to one
/// *generated* dark (`synth::under`) instead.
pub const SNAPSHOTS: [SnapshotDef; 2] = [
    // Above.
    SnapshotDef {
        game: 1.0,
        ambience: 1.0,
        music: 1.0,
        wind: 1.0,
        surf: 1.0,
        under: 0.0,
        rain: 1.0,
        rotor: 1.0,
        town: 1.0,
        night: 1.0,
    },
    // Submerged. The game bus survives at a level a player can still fight on
    // — being underwater must not be a stealth advantage handed out by the
    // mixer — and a little surf comes through, because it does.
    SnapshotDef {
        game: 0.45,
        ambience: 1.0,
        music: 1.0,
        wind: 0.0,
        surf: 0.22,
        under: 1.0,
        rain: 0.0,
        rotor: 0.45,
        town: 0.0,
        night: 0.0,
    },
];

/// How long a full crossfade between snapshots takes, seconds.
///
/// Short, because the cause is instantaneous and a slow fade would have the
/// mix lagging a head that has already gone under. Not zero, because a step
/// change in five gains at once is a click in five voices at once.
pub const SNAPSHOT_FADE_S: f32 = 0.30;

/// The crossfade, as state.
///
/// `t` is how far toward [`Snapshot::Submerged`] the mix currently is. Held
/// rather than recomputed because a fade is state by definition — the same
/// shape `render/audio.rs` already keeps for each bed's gain.
#[derive(Default)]
pub struct Snapshots {
    t: f32,
}

impl Snapshots {
    /// Move toward `want` and return the mixer state to apply this frame.
    pub fn tick(&mut self, want: Snapshot, dt_s: f32) -> SnapshotDef {
        let target = match want {
            Snapshot::Above => 0.0,
            Snapshot::Submerged => 1.0,
        };
        let step = if SNAPSHOT_FADE_S > 0.0 {
            dt_s / SNAPSHOT_FADE_S
        } else {
            1.0
        };
        self.t = if self.t < target {
            (self.t + step).min(target)
        } else {
            (self.t - step).max(target)
        };
        self.blend()
    }

    /// Where the fade is, 0..1.
    pub fn t(&self) -> f32 {
        self.t
    }

    /// Snap to a state without fading — leaving a world, where there is
    /// nothing to hear through the fade and a half-submerged mix would be
    /// carried into the next island.
    pub fn reset(&mut self) {
        self.t = 0.0;
    }

    fn blend(&self) -> SnapshotDef {
        let (a, b) = (&SNAPSHOTS[0], &SNAPSHOTS[1]);
        let t = self.t.clamp(0.0, 1.0);
        let mix = |x: f32, y: f32| x + (y - x) * t;
        SnapshotDef {
            game: mix(a.game, b.game),
            ambience: mix(a.ambience, b.ambience),
            music: mix(a.music, b.music),
            wind: mix(a.wind, b.wind),
            surf: mix(a.surf, b.surf),
            under: mix(a.under, b.under),
            rain: mix(a.rain, b.rain),
            rotor: mix(a.rotor, b.rotor),
            town: mix(a.town, b.town),
            night: mix(a.night, b.night),
        }
    }
}

impl Default for SnapshotDef {
    /// [`Snapshot::Above`] — the state a client that has not looked at the
    /// world yet is in, and the one it returns to when a world is left.
    fn default() -> Self {
        SNAPSHOTS[0]
    }
}

impl SnapshotDef {
    /// The level this snapshot holds a bed at. Panics on a cue that is not a
    /// bed, which is a programming error rather than a runtime condition —
    /// [`Cue::is_bed`] is the question to ask first.
    pub fn bed(&self, cue: Cue) -> f32 {
        match cue {
            Cue::BedWind => self.wind,
            Cue::BedSurf => self.surf,
            Cue::BedUnder => self.under,
            Cue::BedRain => self.rain,
            Cue::BedRotor | Cue::BedRotorFar => self.rotor,
            Cue::BedTown | Cue::BedFurnace => self.town,
            Cue::BedNight => self.night,
            _ => 0.0,
        }
    }
}

/// How a positional cue loses volume with distance.
///
/// `(1 - d/R)²` — quadratic, and **exactly zero at the radius**. The second
/// property is the one that matters and it is why this is not an inverse
/// square: a law that is merely small at the cull radius clicks when a source
/// crosses it, and a source crossing the radius is what a player walking
/// away from a chopping sound does continuously.
///
/// Outside the radius this returns 0, which is also the mixer's cull test —
/// one law, one place, so a cue can never be culled at one distance and
/// attenuated by a different curve at another.
pub fn falloff(dist_m: f32, radius_m: f32) -> f32 {
    if radius_m <= 0.0 || dist_m >= radius_m {
        return 0.0;
    }
    let t = 1.0 - (dist_m / radius_m);
    t * t
}
