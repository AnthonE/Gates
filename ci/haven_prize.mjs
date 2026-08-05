#!/usr/bin/env node
// Gate: the destination is worth more than the route to it — the CONTENT
// half, which no Rust test can see.
//
// `crates/sim-core/tests/haven.rs` proves the geometry: the pad concentrates
// containers 2.64x denser than the road shoulder. That is only half the
// claim. The other half lives in `content/loot.toml`, which the sim does not
// read at all (no verb opens a container yet — `crates/content/src/
// validate.rs` says so in its own words), so nothing in the workspace
// notices if the two tables stop being different.
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
};

check(
  k.crates >= 1,
  `HAVEN_CRATES is ${k.crates} — the pad places no containers, so there is nothing for this gate to be about`,
);
check(
  k.ringRadius < k.padRadius,
  `the container ring (${k.ringRadius} m) is not inside the pad (${k.padRadius} m)`,
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
const crate = byContainer.get("crate");
check(
  !!barrel,
  'content/loot.toml has no table for container "barrel" — the road shoulder\'s containers have no contents',
);
check(
  !!crate,
  'content/loot.toml has no table for container "crate" — the pad places CrateSlot occupants whose container is not described',
);

for (const t of [barrel, crate]) {
  check(
    Number.isFinite(t.rolls_min) && Number.isFinite(t.rolls_max) && t.rolls_max >= t.rolls_min,
    `${t.id}: rolls ${t.rolls_min}..${t.rolls_max} is not an ordered pair`,
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

const barrelEv = expectedItems(barrel);
const crateEv = expectedItems(crate);
const evRatio = crateEv / barrelEv;

console.log(
  `haven prize: barrel ${barrel.entries.length} entries, rolls ${barrel.rolls_min}-${barrel.rolls_max}, ` +
    `E[items] ${barrelEv.toFixed(2)}`,
);
console.log(
  `haven prize: crate  ${crate.entries.length} entries, rolls ${crate.rolls_min}-${crate.rolls_max}, ` +
    `E[items] ${crateEv.toFixed(2)} (${evRatio.toFixed(2)}x the barrel)`,
);

// Strict dominance on the two structural axes, and it is not a taste call:
// 1.0 is the definition of "the pad pays what the road pays", which is the
// defect this whole item exists to remove.
check(
  crate.rolls_min >= barrel.rolls_min && crate.rolls_max >= barrel.rolls_max,
  `the crate rolls ${crate.rolls_min}-${crate.rolls_max} against the barrel's ` +
    `${barrel.rolls_min}-${barrel.rolls_max} — the pad's container is not the richer one, ` +
    `so walking the loop to the destination pays what standing on the road pays`,
);
check(
  evRatio > 1.0,
  `a crate yields ${crateEv.toFixed(2)} items against a barrel's ${barrelEv.toFixed(2)} ` +
    `(${evRatio.toFixed(2)}x) — the destination does not outpay the route`,
);

// The pad's containers must be a different KIND from the road's, or the
// gradient is one number away from being an accident again.
check(
  crate.container !== barrel.container,
  "the pad and the road draw the same container kind",
);

// Non-vacuity, in the direction this gate can actually be fooled: if the two
// tables ever became copies of each other, every check above still passes on
// weights alone. Assert they differ in what they hold, not only how much.
const barrelItems = new Set(barrel.entries.map((e) => e.item));
const crateOnly = crate.entries.filter((e) => !barrelItems.has(e.item));
check(
  crateOnly.length > 0,
  "every item in the crate table is also in the barrel table — the destination pays " +
    "more of the same things, which reads as a longer walk rather than a better one",
);
console.log(
  `haven prize: ${crateOnly.length} item(s) reachable only from the pad's container ` +
    `(${crateOnly.map((e) => e.item).join(", ")})`,
);

console.log(`haven prize: ${checks} checks passed`);
