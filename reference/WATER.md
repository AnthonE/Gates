# reference/WATER.md — how the reference game does water

Ripped facts, not design. `AUDIO.md` answers *how the reference decides what a
player hears*; this file answers **how it decides what the sea looks and sounds
like**, because we were about to build that and the only thing we had was one
translucent plane and a judge's note that it has no wave normals.

Dated 2026-08-08. §9 is the part that changes what we build.

## 0 · Provenance — read this first

**Source: the developer's own public devblogs, plus the shipped graphics
settings.** Same posture as `AUDIO.md` and deliberately so: nothing here was
decompiled, disassembled or extracted. Every claim is either a sentence a
developer published about their own work or a setting the game exposes to any
player who opens its options screen.

Two consequences, the same two:

- **Nothing here is a class name, a shader listing or an algorithm read off a
  binary.** Where a mechanism is described it is described from the outside,
  in our own notation, from what a developer said it does.
- **The dates matter.** The water rebuild is 2015–2016 devblog work
  (roughly 61 → 115) and the game has had a decade since. Read it as "the
  order they built it in and what each step bought", never as "what ships
  today".

Nothing here ships. No shader, no texture, no asset, no name.

## 1 · They rebuilt water once, and the order they did it in is the finding

The old ocean was replaced wholesale by a system the devblogs call **Water2**.
The sequence, in their own published order:

1. **Get the surface in, ugly.** It went to the development branch with what
   the developer calls *"a very basic optics"* — the important parts done, the
   pretty part explicitly deferred.
2. **Optics second**: a fog fix, **depth-based colour extinction** and
   **thickness-based visibility**, and *"the long awaited shoreline wetness"*.
3. **Then the simulation**, which moved to **native code and came out ~8×
   faster**, described as keeping the CPU cool. Only at that point was Water2
   merged to the main development branch — and it merged **without dynamic
   reflections**, which came weeks later.
4. **Reflections third** ("water 2.5"): local screen-space reflections
   revamped *"performance-wise to make it more accessible to mainstream
   hardware"*, then fog and scattering applied **to the reflections**, then
   rivers made to match the sea, then underwater mode.
5. **Foam last.** Ocean foam is described on arrival as *"subtle, mostly
   visible on terrain slopes with higher inclination"* — months after the
   surface shipped.

**A surface, then its optics, then its motion, then its reflections, then its
foam.** Every one of those steps is separable and each shipped alone. That is
the most useful sentence in this document, because the obvious build order —
waves first, because waves are what water *is* — is the one they did third.

## 2 · The optics are a volume, not a translucent plane

Three published mechanisms, and they are one idea:

- **Depth-based colour extinction.** The colour is a function of how much
  water the ray went through, not a constant with an alpha on it.
- **Thickness-based visibility.** How much you can see through the surface is
  the same function. Shallow water over sand is nearly clear; the same
  material 20 m out is not.
- **The fog fix.** Water had to agree with the scene's fog rather than carry
  its own. Later, rivers *"fade away nicely into the background as they are
  affected by global fog and atmospheric scattering"* — the same rule applied
  to a second water body.

That last one is `CLAUDE.md`'s coupled-lighting law in someone else's engine:
haze has one owner, and water is a client of it, never a second author.

**And a colour target, stated as one**: the ocean was retuned to *"a more
Atlantic/Baltic sea colour"*, at the same time rivers were given sediment, so
that water *"looks less out of place"* across biomes. The reference sea is a
cold, dark, desaturated blue-green. It is not tropical.

## 3 · The simulation was two simulations, faded by depth

The published plan: **two separate simulations, deep and shallow, faded
between depending on deepness**, plus *"an entirely independent interactive
simulation"* to be merged into the same deformation.

Three things follow from the shape rather than from the algorithm:

- **Deep and shallow are different problems.** Open-sea swell and what water
  does over a shelf are not one wave set with a parameter between them.
- **Deformation is a channel several producers write.** The interactive sim is
  *merged into* the deformation, not composited on top of it — one height
  field, several authors.
- **It had to be native to be affordable.** The 8× figure is the cost of the
  simulation being on the CPU at all.

## 4 · The shoreline is where the work went

Two separate mechanisms, both about the same seam:

- **Shoreline wetness** started as *"experimental"* terrain-water blending to
  make the transition *"more seamless"*, and ended as a **flag on any object
  using the standard shaders** — so a rock touching water is wet, not just the
  terrain. It was named as the long-awaited feature two devblogs running.
- **Foam** is keyed to the shore and, in their words, *"mostly visible on
  terrain slopes with higher inclination"* — foam reads off the ground's
  geometry, not off the water's. River flow got its own foam later, to close
  the river→ocean seam.

**The waterline is a material problem on the LAND side as much as a shader
problem on the water side.** Both of their published mechanisms modify things
that are not the sea.

## 5 · What it costs, in their own settings screen

`Water Quality` and `Water Reflections` are two separate player-facing sliders.
Published measurements from the optimization-guide ecosystem (players, not the
developer — weaker evidence, and consistent across sources):

| setting | what it controls | cost |
|---|---|---|
| Water Quality | the water surface itself | ~10–18% of frame rate |
| Water Reflections | SSR quality on water and polished surfaces | ~10–18%, ~8% for the last step to off |
| both, at max | | *"up to 27 fps"* |

Two of the developer's own perf notes name mechanisms rather than sliders:
**anisotropic filtering and parallax occlusion mapping** were the culprits in
the higher quality modes, and POM was made independently triggerable from the
quality setting because of it. Separately, *"water reflections access to the
main camera"* was found to be **extremely slow** and fixed.

So: water is one of the two or three most expensive things in the frame, it is
the first thing every optimization guide turns down, and the expensive half is
the reflection, not the surface.

## 6 · The reflection got the sky put into it, and that is the cheap win

When SSR was revamped they *"asked Andre to enable rendering of clouds, sun,
moon and stars into the reflection map"*, and the published verdict is that it
*"made a huge difference"*. It also caused problems with double-lighting and
specular highlights, which had to be sorted out.

Screen-space reflection can only reflect what is on screen, and the sky above
the horizon mostly is not. **The thing a flat sea reflects is the sky**, and
putting the sky in the reflection is a small change with a large result — and
its stated failure mode is energy counted twice.

## 7 · Underwater, and the sound of water

Sparser than the rest, and worth what there is:

- **Underwater mode** is a distinct render state that was still being *"fixed
  a little bit"* during the 2.5 pass, years into the system.
- **Underwater sounds** are their own work item, worked alongside landmine and
  construction sounds.
- The shipped bug worth knowing: **the underwater sound effect stayed on after
  disconnecting from a server.** A state machine over the mix, with a state
  that outlived its cause.
- **Ambience near water is localized and always has been.** Their early pass
  is described as *"seagull and tide noises when coming near bodies of
  water"*, and river sound was later folded into the localized ambience
  system specifically so it would come in and out more smoothly
  (`AUDIO.md` §3).
- The architecture they experimented with for ambience: **nodes that surround
  the camera and check weather and biome to decide what plays**. Water is a
  biome input, not a special case.

## 8 · Water is also a verb

Not rendering, but it is why the sea is not scenery: a **water bucket** item
that *"can be used to splash an area in front of you with water"*, which puts
out fires. Water as an item, with a container, a splash and a consumer.

## 9 · What it means for us

Owned by `crates/client/src/render/water.rs` and `crate::sound`; this section
is why those look the way they do.

1. **We built §1's steps in their order, and stopped where they did.** The
   surface, then its optics (depth-graded colour and thickness alpha), then
   its motion (a Gerstner sum), then its foam. **Reflections are not built**
   — §5 says they are the expensive half and §6 says the payoff is the sky,
   which our sea already gets from the atmosphere's own specular. That is
   their ordering, not a shortcut.
2. **The optics are a volume, so alpha is a function of depth.** `water.rs`
   grades colour *and* alpha off `SEA_LEVEL − terrain::height`, which is §2's
   two mechanisms in the one place they are the same arithmetic. The old plane
   was one colour at one alpha, which is the thing §2 replaced.
3. **The colour target is Atlantic, not Caribbean** (§2). `DEEP_LINEAR` is a
   cold desaturated blue-green and `SHALLOW_LINEAR` leans toward the sand it
   sits over. Written down here because it is a *stated target*, which is what
   `ART.md` means by measured.
4. **Haze has one owner and water is its client** (§2's fog fix). The sea is a
   `StandardMaterial` inside the same atmosphere every other surface is in.
   There is no water fog, no water-only distance term, and no underwater
   colour grade — the last of those is refused for exactly this reason, and it
   is why §7's underwater state is audio-only for us.
5. **The waterline is worked from the land side too** (§4). `terrain_mesh.rs`
   darkens and saturates the ground inside `WET_BAND_M` of sea level, which is
   `ART.md` §5's "wet sand" and the reference's shoreline wetness at the size
   ours can be: a vertex-colour modifier, not a flag on a shader we do not
   have.
6. **Foam is keyed off the LAND's slope, because they say so** (§4). Ours is
   depth-banded and slope-weighted, which is their published sentence turned
   into arithmetic.
7. **The two-simulation plan is not ours to copy** (§3). We have one wave set
   whose amplitude fades to zero as the water shallows, which is the shoaling
   half of their deep/shallow fade and none of the interactive half. An
   interactive deformation channel needs a producer — a boat, a body, a
   splash — and we have none.
8. **§7's stuck underwater sound is refused rather than reproduced.** The
   submerged snapshot is a pure function of the eye's height against
   `SEA_LEVEL` recomputed every frame, and leaving a world resets it — a state
   that cannot outlive its cause because it is not stored as a state.
9. **Ambience near water is localized ambience** (§7), which is the shape
   `AUDIO.md` §9.3 already said to grow into. The surf bed reads how much sea
   is within earshot from the same `terrain::height` the water is drawn from,
   so the sound cannot disagree with the picture — the footstep rule, one
   surface over. **No seagulls**: a bird is a sample, and `sound/synth.rs`
   generates tones.
10. **What we should NOT copy yet**: screen-space reflections (§5's expensive
    half), rivers and lakes (we have one sea and `TERRAIN.md` gives us no
    second water body), parallax occlusion (§5 names it as the perf culprit
    in a renderer with more budget than ours), and the water bucket (§8 is a
    content item, and `CONTENT.md` owns whether it exists).
