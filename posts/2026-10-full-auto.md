# Full Auto

**Patch notes · 22 October 2026 · Gates 0.8.0**

Since Green Veil, the island got nine new guns, rockets and C4, a tech tree
that goes much further, and a casino in the middle of THE GATE. Melee now
lands where the swing is drawn. Cliffs are solid. You can run a shop out of
your own base, and loot, food and water work the way they do in Rust. Here
is what changed.

---

## Guns

Gates now has Rust's gun roster. Every gun has an aim cone, bloom from
sustained fire, recoil you pull down, and damage that falls off with range.
Aiming down the sights tightens the cone and halves the kick.

| Gun | Damage | Rounds | Fire | Ammo | Hits to kill, unarmoured |
|---|---|---|---|---|---|
| Assault Rifle | 50 | 30 | full auto, 450/min | 5.56 | 2 |
| Semi-Automatic Rifle | 40 | 16 | semi | 5.56 | 3 |
| Bolt Action Rifle | 80 | 4 | bolt, pin-point when aimed | 5.56 | 2 |
| Thompson | 38 | 20 | full auto, 450/min | pistol | 3 |
| Custom SMG | 30 | 30 | full auto, 600/min | pistol | 4 |
| Python Revolver | 55 | 6 | semi | pistol | 2 |
| Semi-Automatic Pistol | 40 | 10 | semi | pistol | 3 |
| Revolver | 35 | 8 | semi | pistol | 3 |
| Pump Shotgun | 15 pellets × 14 | 8 | pump | 12 gauge buckshot | 1 shell |

- Every gun is a Blueprint: research it at a table, or unlock it on the tech
  tree. They're crafted from frags, pipes, springs and a gun body. The
  Black Ziggurat's blue room can drop a Python, and the sanctum drops rifles,
  Thompsons, shotguns, rockets and C4.
- 5.56 rifle ammo (3 for 10 frags and 5 gunpowder) and 12 gauge buckshot (2
  for 5 frags and 10 gunpowder) are new.

## Raiding

- **Rocket Launcher.** A rocket flies 30 m a second, drops a little, and
  bursts 3 m wide with 275 damage. Against walls it hits as hard as a satchel.
- **Timed Explosive Charge (C4).** Ten-second fuse, 3 m blast, never a dud,
  and more than twice a satchel's damage to a building.
- **Beancan Grenade.** Cheap (60 gunpowder, 20 frags), a four-second fuse
  and a 2 m blast. About one in seven is a dud.
- **Barbed Wooden Barricade** (400 hp) and **Floor Spikes** cut anyone who
  touches them.

## A bigger tech tree

The tech tree is a full-screen page now, Rust's layout: a LEVEL tab for each
bench, your JUNK count, and an item rail with UNLOCK. Unlocking a node buys
the whole path to it.

Sixteen new items to unlock:

- **Melee:** Salvaged Hammer, Salvaged Sword, Mace, Salvaged Cleaver,
  Longsword, Salvaged Axe and Salvaged Icepick. Of the new blades, the
  longsword hits hardest and reaches furthest.
- **Armour:** Coffee Can Helmet, Tank Top, and **Boots** in a new feet slot.
- **Hazmat Suit:** a full-body piece. It takes 25% off hits, keeps out half
  the cold and wards off half the Verdigris, but you can't wear a helmet or
  boots with it.
- The beancan, the barbed barricade and the floor spikes.

Benches and the research table stand on bare ground now, like Rust.

## Melee lands where it's drawn

- A swing lands 133 ms after you click, at the moment the animation strikes,
  so you see the blow land when it lands. You can read a swing coming and
  step out of it.
- Every weapon reaches as far as its model does: the bat 1.35 m, hatchets
  0.95 m, pickaxes about 1.1 m, the torch 0.9 m, spears 1.75–1.9 m. A spear
  wins the fight it starts.
- Lag compensation rewinds to exactly what you saw, up to 500 ms back, so
  shots and swings on a high ping land where they looked like they would.
- **Punching.** An empty hand throws a weak punch (5 damage) that costs you
  2 hp.
- No grunt when you're hurt, as in Rust. A headshot plays a crunch, and
  the shooter hears it too.

## THE GATE

- **The casino.** Rust's Big Wheel and slot machines, staked in JUNK.
  - The wheel has 25 segments: twelve 1s, six 3s, four 5s, two 10s and one
    20. A win pays your stake times the number, plus your stake back. Bets are open for 45
    seconds, then it spins for 15, up to 100 JUNK a colour.
  - Slots cost 2 JUNK a line, one to five lines, and pay on triples.
  - The house edge is posted: the wheel returns 84–96% depending on the
    colour, and slots return 88%.
- **Vending machines** stand at the six stalls, run by shopkeepers who greet
  you, face you and thank you after a sale.
- **Guards** walk the town with revolvers, and one walks the casino.
- The recyclers only hum while one is running. The workbench, recyclers and
  other stations are Rust's size now, and easier to find.

## Your own shop

- Craft a **Vending Machine** (300 wood, 75 frags) and put it outside your
  base. Stock it, set offers like "10 Low Grade for 5 JUNK", and anyone can
  walk up and buy, even while you're offline.
- Only you and your base's crew can stock it, change its offers or take the
  JUNK out. A purchase either goes through whole or not at all.

## Bases, doors and locks

- **You can see locks.** A keypad with a red or green light, or a padlock,
  sits on every locked door, box and hearth.
- A key lock has its own panel: press L to lock, unlock or take it.
- Doors fill their doorways (1.2 × 2.2 m), and a door opens where you're
  looking at it. Your eye is at Rust's height, 1.5 m.
- **The hearth is Rust's tool cupboard.** E opens the full inventory page with
  the hearth beside it: upkeep cost per 24 hours, how long you're protected,
  the stock as cells (a click takes 500 back), and a FEED button. Drag or
  right-click materials onto it to feed it.
- Twig pays upkeep now, and a stocked cupboard keeps it standing.
- Doors, bags and boxes need you in arm's reach, in sight and on the same
  floor.
- Building tools act only on the piece under your crosshair. The placement
  ghost aims where you look and slides out of walls.

## Survival like Rust

- **You wake hurt, hungry and thirsty:** 55 health, 75 food, 50 water.
- **Food and water last longer.** Standing still, a full stomach lasts three
  hours (from one) and full water two hours (from 40 minutes).
- **Loot follows Rust.** Trees give wood only. Barrels, crates and the
  Ziggurat's rooms give JUNK, parts and gear at Rust's rates, with no cloth,
  fat, frags, sulfur, gunpowder or corn. Ore pickups give 50.
- The first workbench costs 500 wood and 100 frags. THE GATE sells the frags.
- **Cacti** grow in the desert. They're solid, prick you if you push into
  one, and give cloth and Cactus Flesh, which feeds you and quenches thirst.
- The frost at the edge of the screen is softer.

## The island

- **Cliffs are solid.** You can't walk up a rock face from any angle, and you
  slide down one. Bodies, arrows and dropped items all hit the rock you see.
- Every boulder and loose rock is 1.4 times bigger, and rock blends at its
  seams instead of in blocks.
- Flower patches, thicker undergrowth, bigger bushes, and far trees drawn
  further out. Bushier hemp, fuller palms and new saguaro and ball cactus
  models.
- Felled trees sink away after they land. A stump says CHOP STUMP.
- Animal packs, carcasses and bodies press the grass flat.
- **The glyph stones are carved.** The ancients' script is cut into both faces
  of every stone. Once you've read one, its carving glows Verdigris green,
  brighter at night.
- **Wolves outrun a sprint, and pigs keep up with one.** Animal hitboxes follow
  their bodies.
- **The attack helicopter only hunts armed players**: a gun on your hotbar or a
  full set of armour. It stays out of the rain, heads home when rain starts,
  and shows on the map.

## Voice and sound

- **Push to talk is V** (and Mouse 4), as in Rust. Pick up moves to `` ` ``.
- A VOICE CHAT section in AUDIO settings: on or off, mic volume, hold, toggle
  or open mic, the open-mic threshold, and the mic device on desktop.
- New binds to mute your mic and mute all voices, and MUTE IN BACKGROUND.
- FOOTSTEPS and INTERFACE volume sliders.
- Voices behind your head sound duller. Footsteps carry 75 m, and only on
  your floor.
- Intro music on the browser's main menu.

## Achievements

- **81 achievements** in four sets: Survival, Building & Raiding, Combat and
  The Arc.
- An unlock pops up in the top right with its icon and your count in the set.
  `/ach` shows where you stand.

## Skins, unlocks and getting paid

- **A free copy of Gates** goes to every player who signs in and joins, until
  1 December. The house pays the gas.
- **Unlocks**, like Rust's Frog Boots: an item only its owners can craft. The
  first is the **Abyss Rug**, which gives comfort with no fire. THE OUTFITTER
  sells it for 100 JUNK, and it arrives in your wallet as an item you can
  sell. Whoever holds it can craft it.
- Skins can be priced in JUNK.
- On servers that pay, carrying JUNK out through THE EXCHANGE pays wallets that
  played at least **2 hours** that wipe.
- Blueprints now wipe every fourth wipe, about once a month.

## Agents and buddies

- Agents keep seats for people: at most 80 of the 100 can be agents.
- Up to 40 buddies at once, with people first.
- Agents talk on global chat, keep team business on team chat, and say up to
  three lines.
- Agents take cover when shot at by someone they can't see, research at THE
  GATE's table, roof their base before stone, key-lock a lone base's doors,
  find the long way round a cliff, and stop crafting things they already
  have.
- Agents can't post links or wallet addresses, and their recaps never carry
  your team chat.
- Agents can file bug reports on what they hit.

---

## Changelist

### Added

- Assault Rifle, Semi-Automatic Rifle, Bolt Action Rifle, Thompson, Custom
  SMG, Python Revolver, Semi-Automatic Pistol and Pump Shotgun
- 5.56 rifle ammo and 12 gauge buckshot
- Aim cones, bloom, recoil and damage falloff for every gun
- Rocket Launcher and rockets
- Timed Explosive Charge (C4)
- Beancan Grenade
- Barbed Wooden Barricade and Floor Spikes
- Salvaged Hammer, Salvaged Sword, Mace, Salvaged Cleaver, Longsword, Salvaged
  Axe and Salvaged Icepick
- Coffee Can Helmet, Tank Top, Boots and Hazmat Suit
- A feet slot for armour
- The full-screen tech tree, with LEVEL tabs and UNLOCK
- Punching with an empty hand
- The headshot crunch, for both players
- THE GATE's casino: the Big Wheel and slot machines
- Vending machines and shopkeepers at THE GATE's stalls
- Armed guards at THE GATE and in the casino
- The player Vending Machine, with offers priced in JUNK
- Visible locks on doors, boxes and hearths
- The key lock panel (L)
- The tool cupboard page at the hearth, with FEED
- Cacti, and Cactus Flesh
- Carved glyphs on the lore stones, glowing green once read
- Flower patches
- The helicopter on the map
- 81 achievements in four sets, the unlock popup and `/ach`
- VOICE CHAT settings: on/off, mic volume, hold, toggle or open mic, threshold
  and mic device
- Mute-my-mic and mute-all-voices binds, and MUTE IN BACKGROUND
- FOOTSTEPS and INTERFACE volume sliders
- Intro music on the browser's main menu
- 16× anisotropic filtering on desktop
- FPS and worst frame on F4
- Unlocks, and the Abyss Rug from THE OUTFITTER
- Skins priced in JUNK
- A free copy of Gates for every player who joins, until 1 December
- A CHOP STUMP prompt

### Improved

- Melee lands on the swing's strike, 133 ms after the click
- Every melee weapon reaches as far as its model
- Lag compensation is exact and reaches 500 ms back
- Cliffs are solid: no walking up faces, and you slide down them
- Bodies, arrows and dropped items hit the rock that's drawn
- Boulders and loose rocks are 1.4× bigger
- Rock blends at its seams; sand loses its speckle
- Thicker undergrowth, bigger bushes, and trees drawn further out
- Bushier hemp, lower limbs on some trees, and fuller palms
- Felled trees sink away after they land
- Packs, carcasses and bodies flatten the grass
- Wolves outrun a sprint, and pigs match one
- Animal hitboxes follow their bodies
- The helicopter only hunts armed players, and stays out of the rain
- You spawn at 55 health, 75 food and 50 water
- Food lasts three hours and water two, standing still
- Loot and gathering follow Rust: trees give wood only, and containers give
  JUNK, parts and gear
- Ore pickups give 50
- The first workbench costs 500 wood and 100 frags
- Benches and the research table stand on bare ground
- Unlocking a tech-tree node buys the whole path to it
- THE GATE's stations are Rust's size
- The recycler hum plays only while a recycler runs
- Doors fill their doorways, and open where you look
- Your eye height is 1.5 m
- Twig pays upkeep, and a stocked cupboard keeps it standing
- Doors, bags and boxes need you in reach, in sight and on your floor
- Building tools act only on the piece under the crosshair
- The deploy ghost aims where you look and slides out of walls
- Push to talk is V and Mouse 4; pick up is `` ` ``
- Voices behind you sound duller; footsteps carry 75 m on your floor
- Chat lines hold 120 characters, from 48
- A softer frost overlay
- Pickups and looting no longer swing your hand
- Walking into a trunk off-centre slides you round it
- Blueprints wipe every fourth wipe
- On paying servers, payouts go to wallets that played 2 hours that wipe
- Agents: seats kept for people, global and team chat, cover from unseen
  shooters, smarter building and pathing
- Up to 40 buddies at once

### Fixed

- Lag compensation rewound one tick short of what you saw
- You could walk up a cliff face on a diagonal and stand inside the rock
- A locked door turned to sheet metal
- Tree flicker at distance, and the sun stepping
- A broken download in the browser ended the session; it now retries
- A failed sign-in joined you as a guest instead of saying why
- A twig foundation's upkeep was zero
- Bots quit when joining a busy island
- The desktop client and the server no longer trust the three emSign root
  certificates, after the eMudhra compromise

### Removed

- Pain grunts when you're hurt
- Cloth, fat, frags, sulfur, gunpowder and corn from barrels and crates
- Mushrooms from trees
- The workshop plinths at THE GATE
- The kiosk counters; vending machines stand at the stalls instead
- `` ` `` as the default talk key (it's pick up now)

---

[Play Gates in your browser ▸](/games/gates/play/) · [Get paid to build it ▸](/games/gates/board/)
