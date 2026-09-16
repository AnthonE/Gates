# reference/MAP.md — how the reference game draws a map

Research, not law. `CLAUDE.md`'s table says what that means: nothing here
owns anything, and a number in it may not reach `content/`. Our answer is
`crates/client/src/ui/map.rs` (the arithmetic) and `render/map.rs` (the
nodes); §9 is what was built off it and what was not.

## §0 · Provenance

**Good, and unusually easy to state, because the map is a thing they wrote
devblogs about rather than a thing that was decompiled.** Three tier-1 pages
were **fetched whole** on 2026-09-16 — `rust.facepunch.com` answered from
this box, as it did for `FORESTS.md`, `ROCKS.md` and `ROADS.md`, which is
`SOURCES.md` §0's standing instruction working: *probe*, never trust either
reachability claim.

| tier | source | how |
|---|---|---|
| 1 | Devblog 181 (2017-10-12) | fetched whole — the grid overlay, the debris marker, radius drawing |
| 1 | Devblog 149 (2017-02-23) | fetched whole — monuments labelled, vending machines as dots |
| 1 | "Make your Mark" (2023-04) | fetched whole — player-placed markers |
| 2 | wiki `rust/map` | fetched, and **it is about the world, not the screen** — one usable sentence (*"To view the map in-game, you hold g by default"*) and nothing about the UI. Recorded so nobody fetches it again expecting a UI page. |

**What the sources do NOT contain**, and this is said rather than guessed:
the monument ICONS' design, the label typeface, the zoom levels, and how the
map texture itself is generated. Every claim below about the *look* is from
the operator's own frame (2026-09-16), marked as such.

## §1 · The grid is a lettered/numbered overlay, and it is OFF by default

Devblog 181, verbatim: *"Each grid is 150x150 meters, and are lettered along
the horizontal axis, and numbered along the vertical axis."* It is *"off by
default, and you can toggle it with the top left button."* The same post is
honest about the first version: *"It's a little bit ugly right now and has
some minor artefacts when the map is in motion, but it works pretty well."*

Two things worth separating. **The convention** — letters across, numbers
down — is the load-bearing half, because it is what a player arriving from
that game can already read, and it is what two people say out loud to each
other. **The default being off** is a decision about clutter on a map that is
zoomable and pannable; ours is neither, and a 16×16 grid on a fixed 640 px
panel is not the same clutter problem as a 30×30 one under a moving camera.

The stated reason for the feature is the one that matters: *"If you've ever
had a hard time describing where something on the map was to a friend, this
feature should be helpful."* A grid is a naming scheme, not decoration.

## §2 · Monuments are labelled, and only monuments

Devblog 149: *"Monuments are labeled on the map"*, with a screenshot captioned
"Monument Map Markers" and the comment *"It's better this way, trust me."*

The same post adds vending machines as *"a little green dot on everyones
map"* and calls it explicitly temporary — *"while exploring more generalized
marker systems"*. So the tiering was there from the start: a **named** place,
and an **unnamed** dot for a thing that happens to be somewhere.

## §3 · Markers are capped, editable, and shared with a team

"Make your Mark" (2023): *"The maximum amount of markers increasing to 5 and
a new set of controls to help you identify each marker."* Right-mouse adds
and deletes; *"If you have reached the maximum number of markers (5 by
default) you will need to delete another marker to make room."* Left-click
edits: *"You can choose from multiple colour and icons as well as add a label
to each marker."* A team leader's markers are visible to the team.

One detail worth keeping for a HUD lane rather than a map one: *"Labels for
map markers will be slimmed down to three characters when displayed on the
compass."* The map and the compass are one system to them.

## §4 · Events put things on the map, and they expire

Devblog 181 again: *"I added an explosion marker to the map wherever a crash
site is … a yellow and red explosion icon will appear on the map at the
location of the crash site and will stick around for about 10 minutes."* And
the generalisation they built beside it: *"extra support for drawing a radius
of any color on the map"*, for gamemodes that want a radiation ring.

A marker with a **clock** and a marker with an **extent** are two shapes a map
needs that a list of points does not give you.

## §5 · Held, not toggled

The wiki's one useful sentence: *"To view the map in-game, you hold g by
default."* Ours already does (`render::map::open`, `DECISIONS.md`
2026-08-16) — arrived at independently, and it is worth knowing the reference
agrees, because a held map is the reason that screen may not stop the world.

## §9 · What it means for us

### 9.1 Built 2026-09-16 (map legibility v2)

1. **A label in every cell**, off one function shared with the readout above
   the island (`ui::map::grid_cell_label`), so the square you read off the
   picture and the square the panel says you are in cannot drift. Letters
   across, numbers down — §1's convention. Ours are 128 m, not 150, which is
   `ISLAND_SIZE / 16` and predates this doc.
2. **Pictures on the markers** (§2's "it's better this way"): a badge with an
   icon in it, the icon saying *what* and the colour keeping *whose* and
   *what state*. `ui::map::MarkKind::icon`.
3. **The authored tier is named** and nothing else is (§2) — `HAVEN`,
   `WAYSTATION`. A bed and a bag get a picture and no word, which is §2's
   green-dot tier.
4. **The roads are painted**, from `terrain::road_band` rather than from a
   second projection. Not from this doc — from our own `ROADS.md` §7, which
   measured that 38 % of our walkable land is over 300 m from a road; a map
   that does not draw the roads cannot show a player that.
5. **The player is an arrow that points where they are looking.** Not in any
   source here; it is what the operator's frame of ours was missing beside
   theirs, and the bearing was already on screen as three digits.

### 9.2 Not built, and each is a decision rather than a gap

1. **The grid is always on.** §1 says theirs ships off behind a button. A
   toggle is a control-scheme call (`DECISIONS.md` §open, map legibility v2)
   and the operator asked to *show the grid off*, so on is what landed.
2. **No player-placed markers** (§3). This is the real feature gap and it is
   the largest one: five per player, coloured, labelled, shared with a team.
   It needs a wire message and a cap, so it is a `NOW.md` item and not a
   builder's edit.
3. **No marker with a clock or a radius** (§4). We have neither an event that
   would use one nor a wire for it. Worth remembering when the first one
   lands: a bag already has a cooldown and the map draws it as a *weight*
   (`MarkKind::BedSpent`), which is the same problem solved without a clock.
4. **No zoom and no pan.** Ours is 2 048 m on a 640 px panel — 3.2 m a screen
   pixel, where theirs is 4 500 m and has to zoom. Cheap to add and nothing
   yet asks for it.
