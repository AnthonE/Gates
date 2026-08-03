#!/usr/bin/env node
// Gate: every knob the registry DECLARES is the knob that actually SHIPS.
//
// `CLAUDE.md` names `DECISIONS.md` "authoritative on every **(knob)**", and
// the loop discipline says knobs are spoken or carry their documented default
// — a number invented into code is a wall breach. Both of those claims rest
// on the registry and the source agreeing, and until this file nothing in CI
// could see when they stopped.
//
// It cost a pass. On 2026-08-02 the materials v2 row proposed
// `BUMP_MAX_SLOPE = 1.0`, derived as four octaves at the top of the 0.03-0.25
// per-octave slope band. The same pass then measured that 1.0 is a 45°
// perturbation against a key light 21° above the horizon, watched the shadow
// probe collapse to 0.131% of the frame against its 15% floor, and shipped
// 0.55 with a different derivation — editing that very row in the same commit
// without correcting the clause. Nine gates ran green over the disagreement.
// The merge-gate judge caught it by reading, which is the one detection
// method that does not scale.
//
// So the rule this asserts is narrow and mechanical: a `NAME = value`
// declaration inside a backtick span, in the "Open" section, is a claim about
// a constant in this repo, and it must be true of the constant. The backtick
// form is the doc's own existing convention for a declaration, and it is what
// separates a claim from a mention — the same section says "`SCALE_MESO` 1/9.5
// -> 1/4.0" and "`FADE_MESO` was `[2.0, 7.0]`" about values it deliberately did
// NOT ship, and neither is a declaration, because neither puts a value inside
// the span with its name. Prose stays prose; the declaration form is the
// promise.
//
// Three properties keep this from becoming a gate that passes by matching
// nothing (the trap list's worst bug class):
//
//   1. An unresolved declaration is a FAILURE, not a skip. The section header
//      is "Open (defaults ship until spoken)" — a knob recorded there is by
//      the header's own words a value that ships, so it has a constant, and a
//      name that resolves to nothing means the row is stale or misspelled.
//   2. A declaration that resolves in two files with two different values is
//      a FAILURE. Which one is "the knob" is exactly the question the
//      registry exists to answer.
//   3. The suite asserts it found the section AND a non-zero declaration set,
//      so a reformat that silently empties the scan goes red instead of
//      quietly passing.
//
// Language-agnostic on purpose: the knobs it pins today are two JS material
// constants and one Rust limit (`MAX_BACKPACKS`), and a registry that could
// only see one language would drift in the other.

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

// --- the registry side ------------------------------------------------------

const DECISIONS = path.join(ROOT, "DECISIONS.md");
const md = fs.readFileSync(DECISIONS, "utf8");

// The section header, verbatim. If it is ever renamed this goes red rather
// than scanning an empty string and reporting success.
const OPEN_HEADER = "## Open (defaults ship until spoken)";
const openStart = md.indexOf(OPEN_HEADER);
check(
  openStart >= 0,
  `DECISIONS.md has no "${OPEN_HEADER}" section — this gate scans it by name, ` +
    `so a rename must be made here too rather than silently emptying the scan`,
);
// Run to the next top-level section if one is ever added below it.
const openRest = md.slice(openStart + OPEN_HEADER.length);
const nextHeader = openRest.search(/\n## /);
const openSection = nextHeader >= 0 ? openRest.slice(0, nextHeader) : openRest;

// A declaration: NAME = value, both inside ONE backtick span. SCREAMING_SNAKE
// only — that is what a knob is called in both languages here, and it keeps
// prose like `fade_m = cpp / scale` (a formula, lowercase) out of the set.
const DECL_RE = /`([A-Z][A-Z0-9_]{2,})\s*=\s*([^`]+)`/g;
const declared = [];
const ignored = [];
for (const m of openSection.matchAll(DECL_RE)) {
  const entry = { name: m[1], raw: m[2].trim() };
  // A span whose value is not a number, ratio or numeric array is not a
  // numeric knob declaration and this gate cannot pin it — prose about the
  // convention itself lands here. Scoped out, never silently: every one is
  // printed below, because an ignore nobody can see is how a gate rots into
  // matching nothing.
  if (parseValue(entry.raw) === null) ignored.push(entry);
  else declared.push(entry);
}

check(
  declared.length > 0,
  "no `<KNOB> = <number>` declarations found in the Open section — either the " +
    "convention changed or the scan broke; an empty knob gate is not a pass",
);

// --- the source side --------------------------------------------------------

// Only the names the registry declares are parsed out of source. Scanning
// every constant in the repo would drag in template literals and macros this
// has no business interpreting; scanning exactly what was claimed cannot.
const wanted = new Set(declared.map((d) => d.name));

const SKIP_DIRS = new Set([
  "node_modules",
  "target",
  "dist",
  ".git",
  "pkg",
  "Rust Images",
]);

function* sources(dir) {
  for (const ent of fs.readdirSync(dir, { withFileTypes: true })) {
    if (ent.name.startsWith(".") && ent.name !== ".") continue;
    if (SKIP_DIRS.has(ent.name)) continue;
    const full = path.join(dir, ent.name);
    if (ent.isDirectory()) yield* sources(full);
    else if (/\.(rs|js|mjs)$/.test(ent.name)) yield full;
  }
}

// Rust: `pub const NAME: ty = value;`  JS: `const NAME = value;`
// Non-greedy to the first semicolon, which is correct for the scalar and
// array literals a knob can be. A name whose initializer is something else
// entirely surfaces below as an unparseable value, loudly.
const rustDecl = (name) =>
  new RegExp(`\\bconst\\s+${name}\\s*:\\s*[^=]+=\\s*([^;]+);`);
const jsDecl = (name) =>
  new RegExp(`\\bconst\\s+${name}\\s*=\\s*([^;]+);`);

const found = new Map(); // name -> [{file, raw}]
for (const file of sources(ROOT)) {
  const src = fs.readFileSync(file, "utf8");
  for (const name of wanted) {
    if (!src.includes(name)) continue;
    const re = file.endsWith(".rs") ? rustDecl(name) : jsDecl(name);
    const m = src.match(re);
    if (!m) continue;
    if (!found.has(name)) found.set(name, []);
    found.get(name).push({ file: path.relative(ROOT, file), raw: m[1].trim() });
  }
}

// --- values -----------------------------------------------------------------

// Parse to a number or an array of numbers. Anything else returns null and is
// reported as unparseable rather than skipped — a knob this cannot read is a
// knob this cannot pin, and silently declaring victory on it is the bug.
function parseValue(raw) {
  let s = raw.trim();
  // Rust type suffixes and digit separators.
  s = s.replace(/_/g, "");
  s = s.replace(/\b(\d+(?:\.\d+)?)(?:usize|u\d+|i\d+|f32|f64)\b/g, "$1");
  if (s.startsWith("[")) {
    const inner = s.replace(/^\[/, "").replace(/\]$/, "");
    const parts = inner
      .split(",")
      .map((p) => p.trim())
      .filter((p) => p.length > 0);
    const nums = parts.map(scalar);
    return nums.some((n) => n === null) ? null : nums;
  }
  return scalar(s);
}

function scalar(s) {
  s = s.trim();
  // A colour, in the `0xRRGGBB` form both languages here write one in. Added
  // when the daylight register landed four of them in one row: they had been
  // falling into the "not a numeric knob" bucket, which is printed but not
  // pinned, so `DECISIONS.md` could have claimed one sky and the client could
  // have shipped another with the registry gate green over the disagreement.
  // That is the exact failure this file was written for, and the colours are
  // the knobs a lighting row is mostly MADE of.
  if (/^0x[0-9a-fA-F]+$/.test(s)) return Number(s);
  if (/^-?\d+(\.\d+)?$/.test(s)) return Number(s);
  // A knob written as a ratio, e.g. `1 / 9.5`.
  const div = s.match(/^(-?\d+(?:\.\d+)?)\s*\/\s*(-?\d+(?:\.\d+)?)$/);
  if (div) return Number(div[1]) / Number(div[2]);
  return null;
}

const same = (a, b) => {
  if (Array.isArray(a) || Array.isArray(b)) {
    if (!Array.isArray(a) || !Array.isArray(b)) return false;
    return a.length === b.length && a.every((v, i) => v === b[i]);
  }
  return a === b;
};

const show = (v) => (Array.isArray(v) ? `[${v.join(", ")}]` : String(v));

// --- the assertions ---------------------------------------------------------

for (const d of declared) {
  const hits = found.get(d.name);

  // 1. It must exist. "defaults ship until spoken" — so it ships.
  check(
    hits && hits.length > 0,
    `registry declares \`${d.name} = ${d.raw}\` in DECISIONS.md §open but no ` +
      `\`const ${d.name}\` exists in any .rs/.js/.mjs source. Either the knob ` +
      `was renamed or removed in code (fix the row), or it was never ` +
      `implemented (then it is not a default that ships, and the row does not ` +
      `belong in §open yet).`,
  );

  // 2. It must be unambiguous.
  const values = hits.map((h) => ({ ...h, val: parseValue(h.raw) }));
  for (const v of values) {
    check(
      v.val !== null,
      `\`${d.name}\` in ${v.file} initializes to \`${v.raw}\`, which this gate ` +
        `cannot read as a number, ratio or numeric array — so it cannot pin ` +
        `the registry's claim. Simplify the constant or teach this parser.`,
    );
  }
  const distinct = [];
  for (const v of values) if (!distinct.some((o) => same(o.val, v.val))) distinct.push(v);
  check(
    distinct.length === 1,
    `\`${d.name}\` is defined with different values in ${values
      .map((v) => `${v.file} (${show(v.val)})`)
      .join(" and ")} — the registry cannot be authoritative about a name ` +
      `that means two things.`,
  );

  // 3. It must match what the registry claims.
  const docVal = parseValue(d.raw);
  const codeVal = distinct[0].val;
  check(
    same(docVal, codeVal),
    `KNOB DRIFT — \`${d.name}\`: DECISIONS.md §open (the registry CLAUDE.md ` +
      `calls authoritative on every knob) declares ${d.raw}, but ` +
      `${distinct[0].file} ships ${show(codeVal)}. Change the code to match ` +
      `the doc, or change the doc and say why — leaving both standing sends ` +
      `the next reader to a number this repo does not use.`,
  );

  console.log(
    `  ${d.name} = ${show(codeVal)}  (registry ${DECISIONS.split("/").pop()} ` +
      `<-> ${distinct[0].file})`,
  );
}

for (const g of ignored) {
  console.log(
    `  (not a numeric knob, not pinned: \`${g.name} = ${g.raw}\`)`,
  );
}

console.log(
  `knob registry: ${declared.length} declaration(s) pinned to source, ` +
    `${ignored.length} non-numeric span(s) skipped, ${checks} checks passed`,
);
