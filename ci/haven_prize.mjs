#!/usr/bin/env node
// Gate: the destination is worth more than the route to it — the CONTENT
// half, which no Rust test can see.
//
// Restated 2026-08-05 in EXPECTED ITEMS PER SITE, which is the quantity a
// player actually collects. It used to be stated in containers per square
// metre, and `pass-20260805-074623-02-judge.md` fix 2 named why that was the
// wrong quantity: the two tiers on the ring drew the SAME `crate` table, so
// per container the waystation paid exactly what the pad paid and only
// geometry separated them — a gradient a yields edit could invert with every
// Rust wall green. A site's worth is its container count times what that
// container pays, and the road's is the shoulder the site stands on and
// therefore replaces. Both halves are below, under "per SITE".
//
// `crates/sim-core/tests/haven.rs` proves the geometry: the pad concentrates
// containers 2.64x denser than the road shoulder. That is only half the
// claim. The other half lives in `content/loot.toml`, which the sim does not
// read at all (no verb opens a container yet — `crates/content/src/
// validate.rs` says so in its own words), so nothing in the workspace
// notices if the tables stop being different.
//
// They started identical in the way that matters. `findings/pass-20260804-
// 205133-01-judge.md` ranked gap 2: "what the new route pays is, by
// construction, exactly what the beach already pays — ROAD_BARREL_PERMILLE
// = 250 is the beach row's own barrel rate, so walking the loop is worth the
// same as standing where you spawned." The pad answers that by placing a
// RICHER container, not more of the same one, and "richer" is a property of
// two content tables that a rebalance could quietly delete. A pass that
// tuned `loot.barrel` upward for its own reasons would flatten the gradient
// with every Rust gate still green.
//
// Same standard as `ci/pine_shape.mjs`: read the shipped artifact, never a
// copy of it. The Rust constants come out of the source text and the loot
// entries out of the `.toml` the server boots on, so a number restated here
// is a number that cannot drift here.
//
// What this deliberately does NOT assert is a balance target. Which items a
// crate holds is CONTENT and belongs to `CONTENT.md`; what this gate owns is
// the ORDERING between two tables, which is a structural property of the
// world design — the destination outpays the route — and structural
// properties are what a gate can hold without freezing a designer's hands.

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
let checks = 0;
const fail = (msg) => {
  console.error(`GATE FAIL: ${msg}`);
  process.exit(1);
};
const check = (cond, msg) => {
  checks++;
  if (!cond) fail(msg);
};

// --- the shipped constants, out of the shipped source -----------------------

const terrainSrc = fs.readFileSync(
  path.join(ROOT, "crates/sim-core/src/terrain.rs"),
  "utf8",
);

/** Read `pub const NAME: ty = value;` out of Rust source. */
function rustConst(src, name) {
  const m = src.match(
    new RegExp(`const\\s+${name}\\s*:\\s*[A-Za-z0-9_]+\\s*=\\s*([0-9._]+)`),
  );
  if (!m) fail(`could not read ${name} from crates/sim-core/src/terrain.rs`);
  return Number(m[1].replace(/_/g, ""));
}

// Held in an object, not in `const NAME = …` bindings: `ci/knob_registry.mjs`
// reads a top-level `const` whose name matches a registry declaration as a
// claim about that knob's value, and these are reads OF the knob rather than
// declarations of it. A gate that made the registry unable to parse the
// source would be a gate breaking a gate.
const k = {
  crates: rustConst(terrainSrc, "HAVEN_CRATES"),
  padRadius: rustConst(terrainSrc, "HAVEN_RADIUS_M"),
  ringRadius: rustConst(terrainSrc, "HAVEN_CRATE_R_M"),
  waystations: rustConst(terrainSrc, "WAYSTATIONS"),
  wsCrates: rustConst(terrainSrc, "WAYSTATION_CRATES"),
  wsRadius: rustConst(terrainSrc, "WAYSTATION_RADIUS_M"),
  cellSize: rustConst(terrainSrc, "CELL_SIZE"),
  roadHalfW: rustConst(terrainSrc, "ROAD_HALF_W"),
  shoulderHalfW: rustConst(terrainSrc, "ROAD_SHOULDER_HALF_W"),
  barrelPermille: rustConst(terrainSrc, "ROAD_BARREL_PERMILLE"),
};

check(
  k.crates >= 1,
  `HAVEN_CRATES is ${k.crates} — the pad places no containers, so there is nothing for this gate to be about`,
);
check(
  k.ringRadius < k.padRadius,
  `the container ring (${k.ringRadius} m) is not inside the pad (${k.padRadius} m)`,
);
check(
  k.waystations >= 1 && k.wsCrates >= 1,
  `the lesser tier is ${k.waystations} site(s) x ${k.wsCrates} container(s) — with none of either ` +
    `there is no tier for the per-site half of this gate to be about`,
);

// --- the shipped loot tables ------------------------------------------------

const lootSrc = fs.readFileSync(path.join(ROOT, "content/loot.toml"), "utf8");

/**
 * Parse `content/loot.toml` into `{id, container, rolls_min, rolls_max,
 * entries: [{item, weight, count_min, count_max}]}`.
 *
 * A hand parser rather than a dependency, for the same reason `pine_shape`
 * imports the shipped builder: this file has one shape, the content gate in
 * `crates/content` already validates it against the real schema at boot, and
 * a parser that silently matched nothing is the failure mode that matters —
 * so the table count and every field are asserted below.
 */
function parseLoot(src) {
  const tables = [];
  for (const block of src.split(/^\[\[loot_table\]\]$/m).slice(1)) {
    const scalar = (k) => {
      const m = block.match(new RegExp(`^${k}\\s*=\\s*"?([^"\\n]+)"?`, "m"));
      return m ? m[1].trim() : null;
    };
    const entries = [];
    for (const row of block.matchAll(/\{([^}]*)\}/g)) {
      const f = (k) => {
        const m = row[1].match(new RegExp(`${k}\\s*=\\s*"?([^",}]+)"?`));
        return m ? m[1].trim() : null;
      };
      entries.push({
        item: f("item"),
        weight: Number(f("weight")),
        count_min: Number(f("count_min")),
        count_max: Number(f("count_max")),
      });
    }
    tables.push({
      id: scalar("id"),
      container: scalar("container"),
      rolls_min: Number(scalar("rolls_min")),
      rolls_max: Number(scalar("rolls_max")),
      hits: Number(scalar("hits")),
      entries,
    });
  }
  return tables;
}

const tables = parseLoot(lootSrc);
check(
  tables.length >= 2,
  `parsed ${tables.length} loot table(s) from content/loot.toml — the parser matched nothing, which would make every check below vacuous`,
);

const byContainer = new Map(tables.map((t) => [t.container, t]));
const barrel = byContainer.get("barrel");
const cache = byContainer.get("cache");
const crate = byContainer.get("crate");
check(
  !!barrel,
  'content/loot.toml has no table for container "barrel" — the road shoulder\'s containers have no contents',
);
check(
  !!cache,
  'content/loot.toml has no table for container "cache" — the waystations place CacheSlot occupants whose container is not described',
);
check(
  !!crate,
  'content/loot.toml has no table for container "crate" — the pad places CrateSlot occupants whose container is not described',
);

// The three-way name binding, because the tier's price and the tier's
// placement are written in three files and only their agreement is load
// bearing. A rename that lands in two of them boots (the third's table is
// simply never selected) and this gate is the only thing that would notice.
const lootSrcRs = fs.readFileSync(
  path.join(ROOT, "crates/sim-core/src/loot.rs"),
  "utf8",
);
const bakeSrc = fs.readFileSync(
  path.join(ROOT, "crates/content/src/bake.rs"),
  "utf8",
);
for (const [container, occupant, index] of [
  ["barrel", "BarrelSlot", "LOOT_BARREL"],
  ["cache", "CacheSlot", "LOOT_CACHE"],
  ["crate", "CrateSlot", "LOOT_CRATE"],
]) {
  check(
    new RegExp(`\\b${occupant}\\b`).test(terrainSrc),
    `content/loot.toml describes container "${container}" and crates/sim-core/src/terrain.rs has no ${occupant} — a table with no site is a table that never pays`,
  );
  check(
    new RegExp(`pub const ${index}\\b`).test(lootSrcRs),
    `container "${container}" has no ${index} in crates/sim-core/src/loot.rs`,
  );
  check(
    new RegExp(`"${container}"\\s*=>\\s*${index}`).test(bakeSrc),
    `crates/content/src/bake.rs does not bake container "${container}" to ${index} — the table is authored and unreachable`,
  );
}

for (const t of [barrel, cache, crate]) {
  check(
    Number.isFinite(t.rolls_min) && Number.isFinite(t.rolls_max) && t.rolls_max >= t.rolls_min,
    `${t.id}: rolls ${t.rolls_min}..${t.rolls_max} is not an ordered pair`,
  );
  check(
    Number.isFinite(t.hits) && t.hits > 0,
    `${t.id}: hits ${t.hits} — the parser did not read the field the chain below orders on`,
  );
  check(
    t.entries.length > 0,
    `${t.id}: no entries parsed — an empty container is not a prize`,
  );
  for (const e of t.entries) {
    check(
      e.item && Number.isFinite(e.weight) && e.weight > 0,
      `${t.id}: entry ${e.item} has weight ${e.weight}`,
    );
    check(
      Number.isFinite(e.count_min) && Number.isFinite(e.count_max) && e.count_max >= e.count_min,
      `${t.id}: entry ${e.item} counts ${e.count_min}..${e.count_max} is not an ordered pair`,
    );
  }
}

// --- the ordering the world design rests on ---------------------------------

/** Expected items out of one opening: mean rolls x weighted mean count. */
function expectedItems(t) {
  const total = t.entries.reduce((a, e) => a + e.weight, 0);
  const perRoll = t.entries.reduce(
    (a, e) => a + (e.weight / total) * ((e.count_min + e.count_max) / 2),
    0,
  );
  return ((t.rolls_min + t.rolls_max) / 2) * perRoll;
}

const ev = {
  barrel: expectedItems(barrel),
  cache: expectedItems(cache),
  crate: expectedItems(crate),
};

for (const t of [barrel, cache, crate]) {
  console.log(
    `haven prize: ${t.container.padEnd(6)} ${String(t.entries.length).padStart(2)} entries, ` +
      `rolls ${t.rolls_min}-${t.rolls_max}, hits ${t.hits ?? "?"}, ` +
      `E[items] ${ev[t.container].toFixed(2)}`,
  );
}

// --- per CONTAINER: the chain, tier by tier ---------------------------------
//
// Three tiers now, so the claim is a chain and not a pair. Each link is
// strict on both structural axes, and 1.0 is not a tolerance — it is the
// definition of "this tier pays what the tier below it pays", which is the
// defect the whole item exists to remove.
const CHAIN = [
  ["barrel", "cache", "the road shoulder", "a waystation"],
  ["cache", "crate", "a waystation", "the haven pad"],
];
for (const [lo, hi, loWhat, hiWhat] of CHAIN) {
  const a = byContainer.get(lo);
  const b = byContainer.get(hi);
  check(
    b.rolls_min >= a.rolls_min && b.rolls_max >= a.rolls_max,
    `${hi} rolls ${b.rolls_min}-${b.rolls_max} against ${lo}'s ${a.rolls_min}-${a.rolls_max} — ` +
      `${hiWhat} does not roll richer than ${loWhat}`,
  );
  check(
    ev[hi] > ev[lo],
    `${hi} yields ${ev[hi].toFixed(2)} items against ${lo}'s ${ev[lo].toFixed(2)} ` +
      `(${(ev[hi] / ev[lo]).toFixed(2)}x) — ${hiWhat} does not outpay ${loWhat} per container`,
  );
  // Swings to open, non-decreasing up the chain: a richer container that is
  // also cheaper to open is a strictly better container in both hands, and
  // the tier below stops being a decision at all.
  check(
    b.hits >= a.hits,
    `${hi} opens in ${b.hits} swings against ${lo}'s ${a.hits} — the richer container is also ` +
      `the cheaper one, so there is no reason to ever open the lesser tier`,
  );
  // Non-vacuity, in the direction this gate can actually be fooled: if two
  // tables became copies of each other every check above still passes on
  // weights alone. Assert each tier REACHES something the one below cannot.
  const below = new Set(a.entries.map((e) => e.item));
  const only = b.entries.filter((e) => !below.has(e.item));
  check(
    only.length > 0,
    `every item in the ${hi} table is also in the ${lo} table — ${hiWhat} pays more of the same ` +
      `things, which reads as a longer walk rather than a better one`,
  );
  console.log(
    `haven prize: ${only.length} item(s) ${hi} reaches that ${lo} does not ` +
      `(${only.map((e) => e.item).join(", ")})`,
  );
}

// Each tier must draw a different container KIND, or the gradient is one
// content edit away from being an accident again — which it was until
// 2026-08-05, when a waystation placed the pad's own `CrateSlot`.
check(
  new Set([barrel.container, cache.container, crate.container]).size === 3,
  "two of the three tiers draw the same container kind",
);

// --- per SITE: what a player actually collects ------------------------------
//
// The judge's fix, and the reason this section exists: `tests/waystation.rs`
// proves the gradient in CONTAINERS PER SQUARE METRE and a player collects
// ITEMS. Those are not the same quantity, and while they disagreed the gate
// that looked green was measuring the wrong one — a density gradient can be
// inverted by a yields edit with every Rust wall still green.
//
// A site's worth is its container count times what that container pays. The
// road's is stated the only way a road's can be: the shoulder that the site
// STANDS ON and therefore replaces, since the exclusion zone vetoes scatter
// inside it. So the ratio below is an opportunity cost — what arriving is
// worth against what the same ground would have paid as plain road.
const shoulderWidth = 2 * (k.shoulderHalfW - k.roadHalfW);
check(
  shoulderWidth > 0,
  `the shoulder is ${shoulderWidth} m wide (half-widths ${k.shoulderHalfW} vs carriageway ` +
    `${k.roadHalfW}) — there is no barrel band, so the road baseline below is vacuous`,
);
/** Expected barrel items on the stretch of shoulder a site of radius R covers. */
const roadItemsUnder = (r) =>
  ((2 * r * shoulderWidth) / (k.cellSize * k.cellSize)) *
  (k.barrelPermille / 1000) *
  ev.barrel;

const site = {
  pad: k.crates * ev.crate,
  waystation: k.wsCrates * ev.cache,
};
const tierTotal = k.waystations * site.waystation;
const roadUnderPad = roadItemsUnder(k.padRadius);
const roadUnderWs = roadItemsUnder(k.wsRadius);

console.log(
  `haven prize: per site — pad ${k.crates}x crate = ${site.pad.toFixed(1)} items, ` +
    `waystation ${k.wsCrates}x cache = ${site.waystation.toFixed(1)}, ` +
    `whole lesser tier ${k.waystations}x = ${tierTotal.toFixed(1)}`,
);
console.log(
  `haven prize: against the shoulder each site replaces — pad ${(site.pad / roadUnderPad).toFixed(1)}x ` +
    `(${roadUnderPad.toFixed(1)} items), waystation ${(site.waystation / roadUnderWs).toFixed(1)}x ` +
    `(${roadUnderWs.toFixed(1)} items)`,
);

check(
  site.pad > site.waystation,
  `the pad pays ${site.pad.toFixed(1)} items and a waystation ${site.waystation.toFixed(1)} — ` +
    `the destination does not outpay the stop on the way to it`,
);
check(
  site.waystation > roadUnderWs,
  `a waystation pays ${site.waystation.toFixed(1)} items where the shoulder it stands on would ` +
    `have paid ${roadUnderWs.toFixed(1)} — the lesser tier costs the road more than it pays back`,
);
check(
  site.pad > roadUnderPad,
  `the pad pays ${site.pad.toFixed(1)} items where the shoulder it stands on would have paid ` +
    `${roadUnderPad.toFixed(1)} — the destination costs the road more than it pays back`,
);
// `tests/waystation.rs`'s aggregate rule, restated in the quantity a player
// collects: the whole lesser tier put together still pays less than the one
// destination it is subordinate to. In containers that is 2x2 < 5 and cannot
// move without a knob edit; in items it moves with any yields edit, which is
// exactly why it needs saying here as well as there.
check(
  tierTotal < site.pad,
  `the ${k.waystations} waystations pay ${tierTotal.toFixed(1)} items between them against the ` +
    `pad's ${site.pad.toFixed(1)} — added up, the lesser tier outpays the destination`,
);

console.log(`haven prize: ${checks} checks passed`);
