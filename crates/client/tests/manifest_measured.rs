//! Gate: every number this repo states about a shipped texture is that file's
//! own, re-measured here off the file.
//!
//! **Why this exists.** `assets/textures/MANIFEST.md` says in its own words
//! that "when a better source is found, drop it in with the same name" — a file
//! swap is the *designed* way to change art here. Every measurement written
//! down about one is therefore a mirror of something expected to move, and
//! `CLAUDE.md` says twice what happens to a hand-kept mirror of somebody else's
//! surface: it goes stale, silently, and reads as covered while nothing checks
//! it. The *constants* were already safe — `GRAIN_GAIN` and `ROUGH_MEAN`
//! (`ground_splat.rs` legs 1 and 5), `GRAIN_SHARE` and
//! `SAND_GRAIN_SHARE_AT_PUBLISHED` (`ground_tiling.rs` leg 5) all re-measure
//! themselves off the `.jpg`s. The **tables and prose around them** were not,
//! and on 2026-08-28 every one of the three tables and three prose statements
//! this file now reads was wrong somewhere.
//!
//! Most of it is one event. The 2026-08-27 swap of the `rock` identity from aCG
//! `Rock023` to `Gravel004` moved every constant that had a gate pointing at it
//! and **nothing that did not**:
//!
//! - `MANIFEST.md`'s prop-bind table still held `Rock023`'s row — a mean 10%
//!   high, an sd 32% low, and a span (1.054) that is the entire stated basis
//!   for "the ground can only take `rock`";
//! - `render/textures.rs`'s prop table held the same row;
//! - `render/terrain_mesh.rs`'s ground-source table held it too, and its whole
//!   `albedo sd` column turned out to reproduce under *no* reading of the four
//!   files — not full-resolution, not 512², not Rec.601, not per-channel;
//! - `MANIFEST.md`'s `ground_detail` row restated the four ground spans, rock's
//!   among them, and its own two statistics had drifted as well;
//! - `terrain_mesh::ROCK_GAIN` was a **`pub` constant carrying `Rock023`'s
//!   per-channel means** that nothing in the repo read. Deleted rather than
//!   re-measured: the per-channel gain it implements is the approach the §7
//!   note above it explains was superseded by the mean-1 luminance field.
//!
//! Two more are older and unrelated to the swap: the `sand` row's
//! grain percentages (the judged defect,
//! `findings/pass-20260828-042715-01-judge.md` ranked fix 1) said **66.3% →
//! 22.5%** where the file, the constants, the commit body and `DECISIONS.md`
//! all say 79.8% → 47.6% — in the direction that *enlarges* the drop
//! justifying sand's tile clamp — and the `grass` row's AO **sd 0.162** does
//! not reproduce under any reading of `grass_ao.jpg`, which measures 0.150.
//!
//! One mechanism throughout: prose nothing reads. So this gate reads it.
//!
//! **The bound is the document's own precision, halved.** A row printing
//! `0.245` is a claim to three decimals and cannot be held to four, so the
//! tolerance is derived from the row rather than tuned — a row that states more
//! digits is held tighter, and nothing here carries an epsilon anyone chose.
//! Half a place rather than a whole one is what makes it mean *the text is the
//! measurement, correctly rounded*; see [`ulp`], where the mutants decided it.
//!
//! **The basis is fixed and it has to be**, because these statistics move with
//! it — and it has two halves that are equally easy to get wrong.
//!
//! *Resolution.* The same `rock_albedo.jpg` reads an sd of 0.1379 at its full
//! 1024², 0.1287 at 512² and 0.1131 at 256². Everything here is the shipped
//! file at full resolution, linearised from sRGB, Rec.709 luma. `MANIFEST.md`'s
//! *candidate* table is on a 512² basis and says so; it is deliberately not
//! scraped, because holding two bases to one number would be this gate
//! inventing a claim neither table made.
//!
//! *Decoder.* **Two JPEG decoders of one file disagree by more than the last
//! digit these tables print**, which is not obvious and cost a round of wrong
//! corrections while this gate was being written: the first pass at fixing the
//! tables was measured with Pillow, and `image` 0.25 put five more cells out of
//! range — `bark`'s span 2.000 → 1.995, `litter`'s 3.586 → 3.559, up to 0.45%
//! apart. Neither decoder is wrong; the one that *matters* is the one the game
//! ships, because Bevy decodes these same files through `image` and the frame
//! is what any of this describes. So the numbers are `image`'s, and a text
//! measured with anything else will redden this gate rather than agree with it.
//!
//! That is also why the failure prints **the whole table as measured**, at each
//! cell's own precision, ready to paste: re-deriving a table by hand is what
//! these rows had instead of a gate, and it is not a step worth doing twice.
//!
//! A table or row the scrape cannot find **fails loudly** rather than skipping:
//! silently checking nothing is the failure this whole file is about.
//!
//! Headless — no GPU, no window, no shard.

#![cfg(feature = "render")]

use client::render::terrain_mesh::{GRAIN_SHARE, SAND_GRAIN_SHARE_AT_PUBLISHED};

const MANIFEST: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../assets/textures/MANIFEST.md"
);
const TEXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/src/render/textures.rs");
const TERRAIN_MESH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/src/render/terrain_mesh.rs");

fn source(path: &str) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

/// sRGB → linear, the transfer the GPU applies to an `is_srgb` texture and the
/// one `ground_splat.rs`'s and `ground_tiling.rs`'s helpers already use. Stated
/// once more here rather than shared, because this file must keep measuring the
/// same thing if either of those is ever re-scoped.
fn linear(u: f64) -> f64 {
    if u <= 0.040_45 {
        u / 12.92
    } else {
        ((u + 0.055) / 1.055).powf(2.4)
    }
}

fn texture(name: &str) -> image::DynamicImage {
    let path = format!(
        "{}/../../assets/textures/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    image::open(&path).unwrap_or_else(|e| {
        panic!(
            "{path}: {e}\nIt ships — `assets/textures/MANIFEST.md` carries its \
             row, and this gate is that row's check."
        )
    })
}

/// An albedo's per-channel linear means, its Rec.709 luma, the sd of its
/// per-texel linear luma, and `max(mean) / min(mean)`.
///
/// The span is the manifest's own definition, restated in its own words:
/// "`max(mean_rgb) / min(mean_rgb)`, which is what `materials.js`'s
/// `baseGainSpan` computes".
fn albedo_stats(role: &str) -> ([f64; 3], f64, f64, f64) {
    let img = texture(&format!("{role}_albedo.jpg")).to_rgb8();
    let n = f64::from(img.width()) * f64::from(img.height());

    let mut sum = [0.0f64; 3];
    let mut luma_sum = 0.0;
    let mut luma_sq = 0.0;
    for px in img.pixels() {
        let mut lin = [0.0f64; 3];
        for (ch, l) in lin.iter_mut().enumerate() {
            *l = linear(f64::from(px.0[ch]) / 255.0);
            sum[ch] += *l;
        }
        let y = 0.2126 * lin[0] + 0.7152 * lin[1] + 0.0722 * lin[2];
        luma_sum += y;
        luma_sq += y * y;
    }

    let mean = [sum[0] / n, sum[1] / n, sum[2] / n];
    let luma = 0.2126 * mean[0] + 0.7152 * mean[1] + 0.0722 * mean[2];
    let sd = (luma_sq / n - (luma_sum / n).powi(2)).max(0.0).sqrt();
    let hi = mean[0].max(mean[1]).max(mean[2]);
    let lo = mean[0].min(mean[1]).min(mean[2]);
    (mean, luma, sd, hi / lo)
}

/// A single-channel map's mean and sd, read as stored.
///
/// An AO map is loaded `is_srgb = false` — it is a multiplier, not a colour —
/// so the honest statistic is the stored value and not a linearised one.
fn grey_stats(name: &str) -> (f64, f64) {
    let img = texture(name).to_luma8();
    let n = f64::from(img.width()) * f64::from(img.height());
    let mut sum = 0.0;
    let mut sq = 0.0;
    for px in img.pixels() {
        let v = f64::from(px.0[0]) / 255.0;
        sum += v;
        sq += v * v;
    }
    let mean = sum / n;
    (mean, (sq / n - mean * mean).max(0.0).sqrt())
}

/// The first run of digits-and-a-point in `s`.
///
/// Prose runs on past the number it states — `"sd 0.0768. Derived, never
/// edited: ..."` — so the number has to be taken by shape rather than by
/// splitting on a delimiter the sentence also uses for its full stop.
fn first_number(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_digit() || (c == '.' && !out.is_empty()) {
            out.push(c);
        } else if !out.is_empty() {
            break;
        }
    }
    out.trim_end_matches('.').to_string()
}

/// A single-channel sRGB-encoded map's mean and sd **linearised**.
///
/// `ground_detail.jpg` is a luminance field, not a multiplier: the manifest's
/// row derives it as "Rec.601 luma of the source's LINEAR albedo, re-encoded to
/// sRGB greyscale", so the statistic that describes it is the linear one it was
/// built from and not the bytes it was stored as.
fn grey_linear_stats(name: &str) -> (f64, f64) {
    let img = texture(name).to_luma8();
    let n = f64::from(img.width()) * f64::from(img.height());
    let mut sum = 0.0;
    let mut sq = 0.0;
    for px in img.pixels() {
        let v = linear(f64::from(px.0[0]) / 255.0);
        sum += v;
        sq += v * v;
    }
    let mean = sum / n;
    (mean, (sq / n - mean * mean).max(0.0).sqrt())
}

/// Half of one in the last decimal place `printed` was written to — the bound
/// that says *the text is the measurement, correctly rounded*.
///
/// `"0.245"` → `0.0005`; `"0.1379"` → `0.00005`; `"79.8"` → `0.05`. A document
/// can only be checked to the precision it committed to, so the tolerance is
/// derived from the row rather than tuned; but it has to be **half** a place
/// and not a whole one, and that distinction was found by running the mutants.
/// At a full ulp, moving `47.6%` to `47.5%` passed — the true 47.562 is within
/// one place of both — so the gate accepted a number that does not round to the
/// measurement. Six of seven legs caught their mutant and the seventh did not;
/// the bound was the reason, not the leg.
fn ulp(printed: &str) -> f64 {
    let place = match printed.split_once('.') {
        Some((_, frac)) => 10f64.powi(-(frac.len() as i32)),
        None => 1.0,
    };
    place / 2.0
}

/// The text said `printed`; the file measures `got`.
fn agrees(what: &str, printed: &str, got: f64) {
    let want: f64 = printed.parse().unwrap_or_else(|e| {
        panic!("{what}: the text prints `{printed}`, which is not a number: {e}")
    });
    let bound = ulp(printed);
    assert!(
        (got - want).abs() <= bound,
        "{what}: the text says {printed} and the shipped file measures \
         {got:.6} (off by {:.6}, and its own precision allows \u{b1}{bound}).\n\
         The file is the truth. If the source was swapped deliberately, \
         re-measure and update the row; if it was not, the text is describing a \
         texture this repo no longer ships.",
        (got - want).abs()
    );
}

/// Every `**bold**`, backtick and surrounding space out, so a scraped cell
/// parses whether or not the prose emphasised it.
fn plain(cell: &str) -> String {
    cell.replace(['*', '`'], "").trim().to_string()
}

/// `text`'s lines with any doc-comment prefix removed, so a `///`-quoted
/// markdown table scrapes exactly like a `.md` one.
fn undoc(text: &str) -> Vec<String> {
    text.lines()
        .map(|l| {
            let t = l.trim_start();
            t.strip_prefix("///").unwrap_or(t).trim().to_string()
        })
        .collect()
}

/// The rows of the first `|`-delimited table whose header contains every one of
/// `head`, as cells with the markup stripped.
///
/// The header match is what binds this gate to a specific table rather than to
/// a line number; a table that is renamed or deleted stops matching and this
/// panics, which is the whole point.
fn table(where_: &str, text: &str, head: &[&str]) -> Vec<Vec<String>> {
    let lines = undoc(text);
    let at = lines
        .iter()
        .position(|l| l.starts_with('|') && head.iter().all(|h| l.contains(h)))
        .unwrap_or_else(|| {
            panic!(
                "{where_}: no table whose header holds {head:?}. This gate reads \
                 that table; if it was renamed or removed, say so here rather \
                 than leaving the scrape to find nothing and pass."
            )
        });
    lines[at + 1..]
        .iter()
        .take_while(|l| l.starts_with('|'))
        .filter(|l| !l.trim_matches(['|', '-', ' ', ':'].as_ref()).is_empty())
        .map(|l| l.split('|').map(plain).collect())
        .collect()
}

/// The row of `rows` whose first cell names `role`.
fn row_for<'a>(where_: &str, rows: &'a [Vec<String>], role: &str) -> &'a Vec<String> {
    rows.iter()
        .find(|r| r.get(1).map(String::as_str) == Some(role))
        .unwrap_or_else(|| {
            panic!(
                "{where_}: no `{role}` row. A row that disappears stops being \
                 checked, which is the defect this gate exists for."
            )
        })
}

/// `got` written to the same number of decimals `printed` used.
///
/// A suggested replacement is printed at the precision the table already chose,
/// so applying one never silently changes what a row claims to know.
fn like(printed: &str, got: f64) -> String {
    let d = printed.split_once('.').map_or(0, |(_, f)| f.len());
    format!("{got:.d$}")
}

/// A table's disagreements with the files, collected so one run reports the
/// whole thing.
///
/// Cell-at-a-time was the first shape of this and it was the wrong one: these
/// tables drift a **row** at a time — a source is swapped and every number in
/// its row moves together — so failing on the first cell hides the size of what
/// happened and makes fixing it a loop of re-runs. This reports every bad cell
/// and then prints the table as the files measure it, at each cell's own
/// precision, ready to paste.
#[derive(Default)]
struct Drift {
    bad: Vec<String>,
    rows: Vec<String>,
}

impl Drift {
    /// Check one cell, and return it as the file would have written it.
    fn cell(&mut self, what: &str, printed: &str, got: f64) -> String {
        let Ok(want) = printed.parse::<f64>() else {
            self.bad
                .push(format!("{what}: `{printed}` is not a number"));
            return printed.to_string();
        };
        if (got - want).abs() > ulp(printed) {
            self.bad.push(format!(
                "{what}: the text says {printed}, the file measures {got:.6} \
                 (off by {:.6}; its own precision allows \u{b1}{})",
                (got - want).abs(),
                ulp(printed)
            ));
        }
        like(printed, got)
    }

    /// Check a `linear mean rgb` cell against the three measured means.
    fn means(&mut self, what: &str, cell: &str, mean: [f64; 3]) -> String {
        let rgb: Vec<&str> = cell.split_whitespace().collect();
        assert_eq!(
            rgb.len(),
            3,
            "{what}: the `linear mean rgb` cell reads `{cell}`, which is not \
             three numbers."
        );
        rgb.iter()
            .zip(mean.iter())
            .enumerate()
            .map(|(ch, (printed, got))| {
                self.cell(
                    &format!("{what} linear mean {}", ["r", "g", "b"][ch]),
                    printed,
                    *got,
                )
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn row(&mut self, cells: &[String]) {
        self.rows.push(format!("| {} |", cells.join(" | ")));
    }

    fn finish(self, where_: &str) {
        assert!(
            self.bad.is_empty(),
            "{where_} disagrees with the files it describes, in {} cell(s):\n\
             \n  {}\n\nThe files are the truth. As they measure, at each \
             cell\u{2019}s own precision:\n\n{}\n\n\
             If a source was swapped deliberately, paste that in; if it was \
             not, this text is describing a texture the repo no longer ships. \
             Both sides decode with the pinned `image` crate — **the decoder is \
             part of the basis**, and a number measured with another one can \
             differ by more than the last digit here (0.45% was the worst seen \
             between `image` 0.25 and Pillow on these nine files).",
            self.bad.len(),
            self.bad.join("\n  "),
            self.rows.join("\n")
        );
    }
}

/// Leg 1. `MANIFEST.md`'s prop-bind table is the shipped albedos' own.
///
/// Six roles × four numbers, every one a statistic of a `.jpg` this repo ships
/// and expects to swap. This is the leg that was red when the file was written:
/// `rock`'s row carried `Rock023`'s numbers a day after `Gravel004` replaced
/// it, and the mean it stated (0.269) is 10% above what the file delivers.
#[test]
fn the_manifests_prop_bind_table_is_the_shipped_files_own() {
    let md = source(MANIFEST);
    let rows = table(
        "MANIFEST.md",
        &md,
        &["role", "linear mean rgb", "luma", "albedo sd", "gain span"],
    );

    let mut d = Drift::default();
    for role in ["rock", "bark", "twig", "wood", "stone", "metal"] {
        let r = row_for("MANIFEST.md's prop-bind table", &rows, role);
        assert!(
            r.len() >= 6,
            "`{role}`'s row has {} cells; the prop-bind table has four columns \
             after the role. A row this gate cannot read is a row it cannot \
             check.",
            r.len()
        );
        let (mean, luma, sd, span) = albedo_stats(role);
        let cells = [
            format!("`{role}`"),
            d.means(role, &r[2], mean),
            d.cell(&format!("{role} luma"), &r[3], luma),
            d.cell(&format!("{role} albedo sd"), &r[4], sd),
            d.cell(&format!("{role} gain span"), &r[5], span),
        ];
        d.row(&cells);
    }
    d.finish("MANIFEST.md's prop-bind table");
}

/// Leg 2. `render/textures.rs`'s prop table says the same thing, and its ✓ is
/// a claim rather than a decoration.
///
/// The same five numbers written down a second time, in the crate that binds
/// the maps. Two mirrors of one set of files is exactly the arrangement that
/// let `Rock023` survive its own replacement in three places at once, so both
/// are read here rather than held equal to each other — comparing the two
/// tables would have been green on 2026-08-27, because they were wrong
/// together.
///
/// The `in band` column is checked against the band the doc line states, so
/// a ✓ beside a luma outside `ALBEDO_LUMA_BAND` fails.
#[test]
fn the_prop_map_table_in_textures_is_the_shipped_files_own() {
    let rs = source(TEXTURES);
    let rows = table(
        "render/textures.rs",
        &rs,
        &["role", "linear mean rgb", "luma", "albedo sd", "in band"],
    );

    let band = undoc(&rs)
        .iter()
        .find_map(|l| {
            let (_, tail) = l.split_once("ALBEDO_LUMA_BAND = [")?;
            let (inner, _) = tail.split_once(']')?;
            let v: Vec<f64> = inner
                .split(',')
                .filter_map(|s| s.trim().parse().ok())
                .collect();
            (v.len() == 2).then_some([v[0], v[1]])
        })
        .expect(
            "render/textures.rs no longer states `ALBEDO_LUMA_BAND = [lo, hi]`; \
             the `in band` column has nothing to be checked against.",
        );

    let mut d = Drift::default();
    for role in ["rock", "bark", "wood", "stone", "metal"] {
        let r = row_for("render/textures.rs's prop table", &rows, role);
        assert!(r.len() >= 6, "`{role}`'s row has {} cells", r.len());
        let (mean, luma, sd, _) = albedo_stats(role);
        let cells = [
            role.to_string(),
            d.means(role, &r[2], mean),
            d.cell(&format!("{role} luma"), &r[3], luma),
            d.cell(&format!("{role} albedo sd"), &r[4], sd),
            r[5].clone(),
        ];
        d.row(&cells);

        let claimed = r[5] == "✓";
        let inside = luma >= band[0] && luma <= band[1];
        assert_eq!(
            claimed, inside,
            "`{role}`: the table marks `in band` as `{}` and its luma is \
             {luma:.4} against a band of {band:?}. The tick is the claim that \
             the map ships its colour whole; it has to be true.",
            r[5]
        );
    }
    d.finish("render/textures.rs's prop table");
}

/// Leg 3. `render/terrain_mesh.rs`'s ground-source table is the four ground
/// albedos' own.
///
/// This is the table whose `albedo sd` column reproduced under no reading at
/// all — not at 1024², not at 512², not Rec.601, not a mean of per-channel sds.
/// A statistic with no recoverable basis cannot be checked and cannot be
/// trusted; it is on this file's stated basis now, and the doc comment says so
/// above the table.
#[test]
fn the_ground_source_table_is_the_shipped_files_own() {
    let rs = source(TERRAIN_MESH);
    let rows = table(
        "render/terrain_mesh.rs",
        &rs,
        &["source", "linear mean rgb", "gain span", "albedo sd"],
    );

    let mut d = Drift::default();
    let mut spans = Vec::new();
    for role in ["grass", "sand", "litter", "rock"] {
        let r = row_for("render/terrain_mesh.rs's ground-source table", &rows, role);
        assert!(r.len() >= 5, "`{role}`'s row has {} cells", r.len());
        let (mean, _, sd, span) = albedo_stats(role);
        let cells = [
            role.to_string(),
            d.means(role, &r[2], mean),
            format!("**{}**", d.cell(&format!("{role} gain span"), &r[3], span)),
            d.cell(&format!("{role} albedo sd"), &r[4], sd),
        ];
        d.row(&cells);
        spans.push((role, span));
    }
    d.finish("render/terrain_mesh.rs's ground-source table");

    // The claim the table is printed to support, checked rather than read.
    let (worst, _) = *spans
        .iter()
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .expect("four rows");
    assert_eq!(
        worst, "rock",
        "the table's conclusion is that only `rock` clears `ART.md` §7's ×1 \
         deviation rule, and `{worst}` now has the narrowest span. If a source \
         was swapped, the sentence under the table has to move with it."
    );
}

/// Leg 4. A row that restates the table's numbers in prose agrees with it.
///
/// `twig`'s row does — it landed with its measurement written out — so the
/// manifest carries the same four numbers twice. The table is checked above;
/// this holds the prose to the file too, so the pair cannot drift apart the way
/// `rock`'s three copies did.
#[test]
fn a_row_that_restates_its_measurement_agrees_with_the_file() {
    let md = source(MANIFEST);
    let row = md
        .lines()
        .find(|l| l.starts_with("| `twig` |") && l.contains("Linear mean rgb"))
        .expect(
            "MANIFEST.md's `twig` row no longer states its own measurement. If \
             the prose was removed deliberately, drop this leg with it; do not \
             let it search for nothing and pass.",
        );

    let (mean, luma, sd, _) = albedo_stats("twig");
    let after = row.split_once("Linear mean rgb").expect("checked above").1;
    let nums: Vec<String> = after
        .split(',')
        .take(3)
        .map(plain)
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .filter(|t| t.parse::<f64>().is_ok())
        .map(str::to_string)
        .collect();
    assert!(
        nums.len() >= 5,
        "twig's prose reads `{after}`; this gate expects `Linear mean rgb R G \
         B, luma L, sd S`."
    );

    for (ch, printed) in nums[..3].iter().enumerate() {
        agrees(
            &format!("twig prose linear mean {}", ["r", "g", "b"][ch]),
            printed,
            mean[ch],
        );
    }
    agrees("twig prose luma", &nums[3], luma);
    agrees("twig prose sd", &nums[4], sd);
}

/// Leg 5. The `sand` row's grain percentages are the measured ones.
///
/// This is the judged defect (`findings/pass-20260828-042715-01-judge.md`,
/// ranked fix 1). The two numbers are the entire case for clamping sand's tile
/// from its published 15 m to the 4 m ceiling, and they were the *only* part of
/// that case living in prose: `GRAIN_SHARE[0]` and
/// `SAND_GRAIN_SHARE_AT_PUBLISHED` are constants, and `ground_tiling.rs` leg 5
/// already re-measures both off `sand_albedo.jpg`. So this leg re-measures
/// nothing — it ties the sentence to the numbers that are already measured,
/// which is the shortest honest chain.
#[test]
fn the_sand_rows_grain_percentages_are_the_measured_ones() {
    let md = source(MANIFEST);
    let row = md
        .lines()
        .find(|l| l.starts_with("| `sand` |"))
        .expect("no `sand` row in MANIFEST.md");

    // `... falls **79.8% → 47.6%**.`
    let (_, tail) = row
        .split_once("falls")
        .expect("sand's row no longer says its grain share `falls` A% → B%");
    let pcts: Vec<String> = tail
        .split('→')
        .take(2)
        .map(|s| {
            plain(s)
                .split('%')
                .next()
                .unwrap_or("")
                .split_whitespace()
                .next_back()
                .unwrap_or("")
                .to_string()
        })
        .collect();
    assert_eq!(
        pcts.len(),
        2,
        "sand's row reads `{tail}`; this gate expects `falls **A% → B%**`."
    );

    agrees(
        "sand grain share at its drawn tile (GRAIN_SHARE[0])",
        &pcts[0],
        f64::from(GRAIN_SHARE[0]),
    );
    agrees(
        "sand grain share at its published 15 m (SAND_GRAIN_SHARE_AT_PUBLISHED)",
        &pcts[1],
        f64::from(SAND_GRAIN_SHARE_AT_PUBLISHED),
    );
}

/// Leg 6. The `grass` row's AO statistics are `grass_ao.jpg`'s own.
///
/// The mean reproduced and the sd did not, which is the tell that a number was
/// carried across a file change rather than mistyped: 0.477 is this file and
/// 0.162 is not, under greyscale, per-channel or linearised readings alike.
///
/// The row's *claim* — that grass's AO is the strongest of the set — survives
/// the correction and is checked here as a claim rather than left as an
/// adjective, because the sd was never the evidence for it: occlusion strength
/// is how far the mean falls below 1, and on that reading grass leads by a
/// distance while its sd is only third.
#[test]
fn the_grass_rows_ao_statistics_are_the_shipped_files_own() {
    let md = source(MANIFEST);
    let row = md
        .lines()
        .find(|l| l.starts_with("| `grass` |") && l.contains("Its AO is"))
        .expect(
            "MANIFEST.md's `grass` row no longer states its AO statistics. If \
             they were removed deliberately, drop this leg with them.",
        );

    let (_, tail) = row
        .split_once("(mean")
        .expect("grass's AO reads `(mean M, sd S)`");
    let nums: Vec<String> = tail
        .split(')')
        .next()
        .unwrap_or("")
        .split(',')
        .map(|s| {
            plain(s)
                .split_whitespace()
                .next_back()
                .unwrap_or("")
                .to_string()
        })
        .collect();
    assert_eq!(
        nums.len(),
        2,
        "grass's AO reads `{tail}`; this gate expects `(mean M, sd S)`."
    );

    let (mean, sd) = grey_stats("grass_ao.jpg");
    agrees("grass AO mean", &nums[0], mean);
    agrees("grass AO sd", &nums[1], sd);

    // The claim the numbers are there to support.
    for other in ["sand", "litter", "rock"] {
        let (m, _) = grey_stats(&format!("{other}_ao.jpg"));
        assert!(
            mean < m,
            "grass's row claims its AO is the strongest of the set, but \
             {other}'s mean is {m:.4} against grass's {mean:.4} — a *lower* \
             mean is more occlusion, so the claim is now false and the row has \
             to say so."
        );
    }
}

/// Leg 7. The `ground_detail` row restates the four ground spans, and its own
/// two statistics are its own file's.
///
/// The last mirror in the set, and the one that shows how far a single swap
/// travels: this row exists to argue that a *luminance field* has a span of
/// 1.000 where the four colour sources do not, so it quotes all four of them —
/// which made `rock`'s 2026-08-27 replacement wrong in a fourth place, in a
/// sentence about a different file entirely.
#[test]
fn the_ground_detail_row_is_the_files_own() {
    let md = source(MANIFEST);
    let row = md
        .lines()
        .find(|l| l.starts_with("| `ground_detail` |"))
        .expect("no `ground_detail` row in MANIFEST.md");

    let (_, tail) = row
        .split_once("measure")
        .expect("the `ground_detail` row no longer quotes the four source spans");
    let quoted: Vec<String> = tail
        .split("(grass")
        .next()
        .unwrap_or("")
        .split('/')
        .map(plain)
        .collect();
    assert_eq!(
        quoted.len(),
        4,
        "the `ground_detail` row quotes `{}`; this gate expects four spans, \
         `A / B / C / D (grass / sand / litter / rock)`.",
        tail.split("(grass").next().unwrap_or("")
    );

    let mut d = Drift::default();
    let mut cells = Vec::new();
    for (role, printed) in ["grass", "sand", "litter", "rock"].iter().zip(&quoted) {
        let (_, _, _, span) = albedo_stats(role);
        cells.push(d.cell(
            &format!("{role} gain span (ground_detail row)"),
            printed,
            span,
        ));
    }
    d.row(&cells);

    let (_, tail) = row
        .split_once("Linear luma mean")
        .expect("the `ground_detail` row no longer states its own mean and sd");
    let own: Vec<String> = [tail, tail.split_once("sd ").map_or("", |(_, t)| t)]
        .iter()
        .map(|t| first_number(t))
        .collect();
    assert!(
        own.iter().all(|n| n.parse::<f64>().is_ok()),
        "the `ground_detail` row reads `{tail}`; this gate expects `Linear luma \
         mean M, sd S.`"
    );

    let (mean, sd) = grey_linear_stats("ground_detail.jpg");
    let m = d.cell("ground_detail linear luma mean", &own[0], mean);
    let v = d.cell("ground_detail linear luma sd", &own[1], sd);
    d.row(&[format!("Linear luma mean {m}"), format!("sd {v}")]);
    d.finish("MANIFEST.md's `ground_detail` row");
}
