# First Light

**Devblog · 17 September 2026**

This month Gates learned to run in a browser tab, a lethal blow stopped
always being lethal, barrels started spilling their loot onto the ground
where you have to walk over and pick it up, the map became something you can
read a grid reference off, and every normal map in the game turned out to have
been bent forty-one degrees sideways for three weeks. Plus roads you can see,
a forest with a floor, eight graphics settings that actually do something, and
the discovery that in a browser every shadow in the world was stopping twelve
metres from your feet.

---

## What this is

Gates is a survival game. You wake on a beach with a rock and a torch on a
two-kilometre island, and everything after that is yours: chop, mine, craft,
build, lock a door, and find out whether the person walking up the beach wants
to trade or wants your bag.

It is written in Rust, top to bottom — an authoritative server, a native
desktop client, and now a browser build, all sharing one simulation. The
simulation is the product. It is deterministic to the bit: the same island and
the same inputs produce the same world on your machine and ours, every time,
and there is a test that proves it by compiling the whole thing to a second
platform and comparing the results byte for byte.

This is the first of these posts. There is a lot of game in here that has
never been written about, so this one does double duty.

---

## The game runs in a browser now

The single biggest thing this month, and it started with one sentence: *people
don't want to download Gates.*

So it doesn't have to be downloaded. `gates` compiles to WebAssembly and runs
in a tab — and the important part is what **didn't** happen. There is no
JavaScript version of the game. It is the same Rust, the same simulation, the
same protocol, the same handshake, the same prediction code. The only thing
that differs between the desktop build and the browser build is how bytes
reach the network, and that seam is **one trait with one method**.

That matters because the alternative — a second implementation of the game for
the web — is how projects end up with two games that disagree with each other,
and every bug gets fixed twice or, worse, once.

What landed, in order: the WebTransport transport, wallet sign-in, the
renderer on wasm32, the three-array ground, a sky, models, a playable page,
and sound. The module is 34 MB, 8.6 MB gzipped. Model textures are converted
to 512² PNG for the page, which took the asset payload from 82 MB to 32 MB and
took 47 failures to load a model down to zero.

**What we are not claiming**: every browser frame so far has been rendered by
a software rasteriser in a headless test. Nobody has opened it on a machine
with a real GPU and a real wallet yet. The sky's colour, the load time and how
it looks on a high-DPI display are all unmeasured on hardware. That is the
next thing.

---

## A lethal blow doesn't always kill you

Take a hit that would have killed you and you now have a decent chance of
going **down** instead of out.

| | |
|---|---|
| Time on the floor | **40–50 seconds** |
| Health while down | **10** |
| Crawl speed | **a third** of a walk |
| Base chance of getting back up | **20 %** |
| Bonus for full food and water | **up to +25 %**, so 45 % at best |

You can still crawl, and you can still open a door — which is the whole point.
A minute spent dragging yourself behind a rock is a minute of story, and the
person who put you there has to decide whether to spend the time finishing the
job or take your friend's bag and run.

The roll at the end is a single hash of (seed, your id, the tick). It is not a
random number generator you can save-scum, and it is reproducible in a replay,
like everything else in the simulation.

**Not built yet, and named here so nobody is surprised**: you cannot pick
somebody up. There is no revive, no syringe, and a medkit in your belt does
nothing for you while you are down. Those are the next three things in this
system and they are the half that makes it social rather than solitary.

---

## Barrels spill

Break a barrel and it used to hand you a sack. Now it scatters the actual
items on the ground, in a spread of about 1.2 m, and you walk over and take
them.

The reason is about the frame rather than the store: a bag is one object
whatever it came out of, so a smashed barrel and a dead player read identically
at ten metres, and the thing you broke tells you nothing about what fell out of
it. Now it does.

This cost a whole second store in the simulation with its own eviction
policy — and that is the point rather than the price. Before, a barrel-heavy
server could quietly evict somebody's death bag to make room for three units
of cloth.

**What it deliberately doesn't do yet**: the items land where they land. The
*"roll a bit"* — a stack you watch tumble down a slope and out of easy reach —
is a landing spot right now, not a tumble. It is on the list.

---

## The map became a chart

Held with `G`, and it stopped being a picture of an island.

- **The roads are painted on it**, off the same function that decides where
  the road is in the world, so the map and the ground cannot disagree.
- **A grid label in all 256 cells** — A–P across, 1–16 down. The convention is
  taken deliberately: it is the one two people can say out loud to each other.
  "I'm in H7" is the whole feature.
- **Pictures instead of coloured dots** for your bed, your hearth, and a death
  bag still standing.

Your position and heading are on it, and it paints from the same island
function the 3D ground blends by — so it is not a texture somebody baked, it
is the actual world, drawn.

---

## Every normal map in the game was bent 41 degrees

This is the one worth reading if you like knowing how the sausage is made.

A normal map is a texture that tells the renderer which way a surface is
facing at every point — it is what makes a flat polygon look like rough rock
rather than a sheet of paper. It stores directions as colours, and it must be
read as **raw numbers**, not as a picture.

Our texture packer asked the compression tool for a linear output format. That
says what comes out. It never said what went **in** — and the tool's documented
default is to assume an untagged PNG is a picture, and apply a brightness
curve to it on the way through.

So every normal map in the game had a brightness curve applied to its
directions. Measured, the X and Y channels were centred on **0.212** where a
tangent-space normal map centres on **0.500**. That is roughly a 41° bend, and
because it is applied per-texel, it points a *different* wrong way on every
patch of every model. The visible result is a polygon-edged shading patchwork
on a boulder that should be smooth rock.

**Every automated check was green the entire time**, and every one of them had
to be: they all read the file's container and header, and none of them read a
single pixel. It shipped from 11 August to 5 September.

Two fixes. The packer now states the transfer function explicitly in both
directions, and a test reads the shipped file back through the game's own
decoder and checks the numbers. And then — because the packer was fixed on the
5th and the *assets* were not, so the bend kept shipping — a new step decodes
the maps back out of the models that already shipped, inverts the curve, and
repacks them. Two independent readings say it lands: normals 0.212 → **0.498**,
roughness 0.411 → **0.672** against the 0.67 that was originally delivered.

The general lesson, which is now written down where the next person will trip
over it: **a check that reads a file's header is not a check that reads the
file.**

---

## In the browser, every shadow stopped 12 metres away

Reported as *"shadow stuff is kinda garbage with distance, maybe it's web?"*

It was web, and it was one constant.

Shadows are drawn in cascades — nested boxes of decreasing resolution, so
things near you get crisp shadows and things far away get cheap ones. The
engine supports four. On WebGL2 it supports **one**. We asked for two.

It did not error. It did not rescale. It took the first one and threw the rest
away — and the first cascade's far edge is 12 metres. So from the browser
build's first day, everything past arm's length was lit as if nothing was
standing on it.

The fix is not asking for less, it is asking differently. Request exactly one
cascade and the engine's bounds calculation short-circuits and gives that
single cascade the **whole** 90 m range. Spending the freed budget on
resolution:

| | before | after |
|---|---|---|
| Shadows reach | 12 m | **90 m** |
| World per shadow texel | 27.5 cm | **13.8 cm** |

There is now a test for the underlying mistake — that every cascade we ask for
is one the engine will actually draw — and it is written so a normal desktop
run proves the mechanism that was broken in the browser.

**The general shape**: when you hand an engine a list, find out what it does
with a list that is too long. "Clamps the count" and "keeps the first and
silently bins the rest" look identical in a function signature, and only one
of them is a feature.

---

## The trees stopped being fern fronds

Four screenshots of our forest next to one of a game that has been doing this
for a decade, and the verdict was "trees need help". Four measurements said the
same thing.

The leaf texture was **64 pixels square**. A conifer branchlet card is 1.08 m
of world, so that is 16.9 mm per pixel, and the thinnest line you can draw at
that scale is a **34 mm** needle — 23 cm long. That is not a pine needle, it is
a fern frond. Broadleaves were worse: 37 cm leaves.

Both cards are **256²** now with hash-jittered placement: a 4 mm needle and an
11 cm leaf.

Two more things were wrong and both were about the crown having no interior. A
needle at the trunk and a needle at the branch tip were the same colour, and
the canopy had no normal pointing up — so it took no light from the sky. Both
are fixed by baking a depth term against the crown's actual radius at each
height.

---

## The world got roads, a floor, and a second monument

- **Roads have a surface.** Pavement and gravel are distinct, with faded
  markings, and the road is a thing you can see rather than a thing the map
  claims is there.
- **Side roads** clear their junctions properly, and there is a measured
  prototype for routing a second road across the interior.
- **The forest has an understory.** It was canopy and bare ground; there is a
  brush layer now.
- **A freight depot** joins the site roster as a second authored place, with
  paired road approaches that are validated rather than hoped for.

One honest measurement from the road work, because it says where the world
still needs to go: **38 % of our walkable land is more than 300 m from any
road.** More roads doesn't fix that. More places worth walking to does.

---

## Notable changes

**Sixty seconds to sign in.** A wallet prompt gave you the same five seconds
as every other handshake step, which is fine for a test stub that signs in
milliseconds and hopeless for a person reading a MetaMask dialog. The first
real sign-in attempt failed for exactly this reason and left nothing in the
log. Signing has its own deadline now and the page tells you when it runs out.

**The server waits until it is ready.** Players could connect before the
simulation had finished coming up.

**Quick-move.** Right-click an item with a container open and it goes across —
the binding people already have in their fingers.

**Impact effects.** A blow throws sparks and dust, and the swing is heard when
the arm moves rather than when the button is pressed.

**Eight graphics settings that do something**, plus a preset ladder, rather
than a list of rows that were there for decoration.

**The page nearly shipped the build system.** The web staging directory
happened to be the same path the compiler uses for its own intermediate files,
so an 85 MB upload quietly became 222 MB of object files and absolute build
paths. The publish step refuses that now, because a publish is the last place
to find out.

---

## What this month did not do

Published here on purpose, because a list of wins with nothing underneath it
is a sales page.

- **Nobody has played this on a GPU in a while.** There are 27 separate things
  waiting on one person booting the game on real hardware and looking at it.
  We deliberately do not have an automated visual test — we had one, and it
  passed all 36 of its checks on a beige smear with no sky, no horizon and
  nothing in it. A person looking at the screen is the visual test.
- **There is no wipe.** No scheduled reset, which means blueprints surviving a
  wipe is a feature with nothing to survive.
- **There is no soak test.** Nothing in this project has run for four hours
  straight.
- **Two animals.** A pig and a wolf. The roster is thin and we know it.

---

## Changelist

### Added

- The game runs in a browser: WebTransport transport, handshake, wallet
  sign-in, renderer on wasm32, sky, models, audio, playable page
- Wounded state — a lethal blow lays you down for 40–50 s at 10 hp
- Crawling while down, at a third of walking speed, including through a door
- A recovery roll at the end of the window, 20 % base, up to 45 % fed and watered
- Barrels spill loose items onto the ground instead of dropping a sack
- Pick up a loose stack off the ground
- Roads painted on the map
- Grid labels in all 256 map cells, A–P by 1–16
- Picture icons on the map for bed, hearth and standing death bag
- Eight graphics settings rows and a quality preset ladder
- A freight depot as a second authored site, with validated road approaches
- Side roads with cleared junctions
- A forest understory layer
- Distinct pavement and gravel road surfaces, with faded markings
- Impact sparks and dust on a blow
- Browser audio, running the same sound engine as the desktop in a worklet

### Improved

- Conifer and broadleaf canopy cards from 64² to 256² — a 4 mm needle where the
  finest mark used to be 34 mm
- Canopy shading gains an interior: crown-radius depth term and an upward normal
- Forest density and scale retuned
- Browser model payload 82 MB → 32 MB; 47 model load failures → 0
- Wallet signing gets its own 60-second deadline
- The shard waits for simulation readiness before accepting players
- Right-click quick-move between containers
- The item catalogue carries stack ceilings, so the client can pick a
  destination slot instead of only ever aiming at an empty one
- Melee aim, build level aiming, foundation height and viewmodel thrust
- Rock and ore-node art: a pool of rocks, and nodes with their own faces
- Audio spread and decal contrast
- World structure and live-slot banding

### Fixed

- Every normal map in the game decoded with a brightness curve applied to it,
  bending surface directions ~41°; the packer and every shipped asset are both
  corrected
- In the browser, every shadow in the world ended 12 m from the player; shadows
  now reach 90 m at twice the texel density
- The browser was silent — two separate causes, a content-security policy and a
  missing audio worklet
- The browser was grey
- The first real wallet sign-in failed with nothing in the server log
- An opaque hull was drawn over every real tree in every chunk entered after
  spawn, at any distance, on both builds
- The web publish could carry the compiler's own intermediate files onto a
  public web root
- Visible slits between adjacent walls and a bare notch at every corner
- A ray fed a rule written for feet reported a hit half a metre under every
  ceiling

### Removed

- The browser's unused audio host, which was allocating a buffer every 46 ms to
  mix a graph that nothing had put a sound into
