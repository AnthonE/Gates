//! What a base costs to keep, and how it rots when nobody pays — the
//! arithmetic half of upkeep v2 (`reference/BUILDING.md` §4b, §5 and §9
//! items 25–27). `deploy::upkeep_sweep` still owns *when* a piece is
//! charged and every store it writes; this module owns *how much*, pure
//! over its arguments, so the sweep, the hearth's readout and the gates ask
//! one function one question.
//!
//! Three rules on top of upkeep/decay v1's per-material charge and decay
//! ladder, each the reference's:
//!
//! 1. **The rent rises with the base** ([`tax`]). Their upkeep is a
//!    fraction of what a building cost, and the fraction steps up with how
//!    many blocks the building holds — a starter hut pays a tenth a day,
//!    and every block past the 190th a third. Ours was one flat rate for
//!    every base, so the one lever the reference built against sprawl did
//!    not exist here.
//! 2. **Inside rots slower** ([`inside`]). An unpaid block with something
//!    built over it decays at a fraction of its rate, so a lapsed base
//!    comes down from the roof inward instead of everywhere at once — the
//!    shape `BUILDING.md` §5 fact 3 describes.
//! 3. **The hearth can say how long it lasts** ([`bill`], [`lasts`]).
//!    Their cupboard's first line is how long the base is protected; ours
//!    could only say what it held, and a stock with no rate beside it is a
//!    number nobody can act on.
//!
//! Integer arithmetic throughout (wall 1): a rate is a ratio of two `u64`s
//! and an hour's charge is a difference of floors ([`Tax::due`]), so the
//! hours sum to the exact daily rent rather than to 24 round-ups.

use crate::build::{
    BuildContent, Pieces, LOC_DIAG_A, LOC_DIAG_B, LOC_EDGE_XLO, LOC_EDGE_ZLO, MAT_TWIG,
    SHAPE_DOORWAY, SHAPE_FRAME, SHAPE_WALL, SHAPE_WINDOW,
};
use crate::collide::{ColIndex, ColMasks};
use crate::deploy::{cell_center, pays_rent, DeployContent, Deploys, PERIODS_PER_DAY, WORLD_OWNER};
use crate::limits::{HEARTH_STOCK_ROWS, MAX_BUILD_LEVELS, MAX_BUILD_SOCKETS};

// ---------------------------------------------------------------------------
// 1 · The rent
// ---------------------------------------------------------------------------

/// A day's rent on one base, as the exact fraction `num / den` of a piece's
/// build cost. A ratio rather than a rate so the blend across the ladder's
/// rungs never rounds: the only rounding is in [`Tax::due`] and
/// [`Tax::per_day`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tax {
    pub num: u64,
    pub den: u64,
}

impl Tax {
    /// What a piece owes in hour `k` for one material whose cost rows sum
    /// to `cost`: the hour's slice of the exact daily rent, `floor(C·k/D) −
    /// floor(C·(k−1)/D)` with `C = cost × num` and `D = den × 24`. Any 24
    /// consecutive hours sum to the day's rent within one unit, so a
    /// 100-wood piece at 10 % pays 10 a day. It paid 24 while every hour
    /// rounded up on its own.
    ///
    /// `k` must be at least 1. In u128, because a phase-shifted `k` times a
    /// whole base's cost can pass `u64`.
    pub fn due(self, cost: u32, k: u64) -> u32 {
        let c = cost as u128 * self.num as u128;
        let d = self.den as u128 * PERIODS_PER_DAY as u128;
        if d == 0 || k == 0 {
            return 0;
        }
        let k = k as u128;
        (c * k / d - c * (k - 1) / d) as u32
    }

    /// A day's rent on `cost`, rounded up once: the hearth's readout.
    pub fn per_day(self, cost: u64) -> u32 {
        let c = cost as u128 * self.num as u128;
        if self.den == 0 {
            return 0;
        }
        c.div_ceil(self.den as u128).min(u32::MAX as u128) as u32
    }
}

/// **The group tax** (`NOW.md` §0up 5): Facepunch's "Breach and Clear"
/// (3 Sep 2026, `rust.facepunch.com/news/breach-and-clear`). The first four
/// members cost nothing, the fifth to the tenth add 2 % each, every one
/// past ten 4 %, capped at 300 %. Theirs counts the cupboard's list, the
/// base's code-lock lists and anyone deauthed in the last day; ours counts
/// the hearth's crew, which is the list a claim has.
pub const GROUP_FREE: u32 = 4;
pub const GROUP_TEN: u32 = 10;
pub const GROUP_PCT_TO_TEN: u32 = 2;
pub const GROUP_PCT_PAST_TEN: u32 = 4;
pub const GROUP_CAP_PCT: u32 = 300;

/// The group tax's surcharge for a crew of `n`, in per cent.
pub fn group_pct(n: u32) -> u32 {
    let to_ten = n.min(GROUP_TEN).saturating_sub(GROUP_FREE) * GROUP_PCT_TO_TEN;
    let past = n.saturating_sub(GROUP_TEN) * GROUP_PCT_PAST_TEN;
    (to_ten + past).min(GROUP_CAP_PCT)
}

impl Tax {
    /// This rent with `pct` per cent on top.
    pub fn plus_pct(self, pct: u32) -> Tax {
        Tax {
            num: self.num * (100 + pct as u64),
            den: self.den * 100,
        }
    }
}

/// Hearth `hi`'s rent: the ladder at its claim's size, plus the group tax
/// on its crew. What the sweep charges and the readout quotes.
pub fn hearth_tax(dc: &DeployContent, deploys: &Deploys, hi: usize) -> Tax {
    let crew = deploys.hearths().get(hi).map_or(1, |h| h.crew.len() as u32);
    tax(dc, deploys.claim_graded(hi)).plus_pct(group_pct(crew))
}

/// The rent on a base holding `graded` pieces that pay upkeep (twig does
/// not, `deploy::upkeep_sweep`), read off `dc`'s ladder. See [`tax_parts`].
pub fn tax(dc: &DeployContent, graded: u32) -> Tax {
    tax_parts(
        dc.upkeep_pct_per_day,
        &dc.upkeep_steps[..(dc.upkeep_step_count as usize).min(dc.upkeep_steps.len())],
        graded,
    )
}

/// The ladder walk itself, over its parts — public so the content crate's
/// balance anchor prices a starter base with the sim's arithmetic rather
/// than a second copy of it.
///
/// **Their blend, not a marginal rate.** Pieces `1..=first step` pay
/// `base_pct`, the next run pays the first step's rate, and so on; the base
/// then pays the *average* of those on every piece. Summed over the base it
/// is the same bill as charging each piece by its position, and it has the
/// property a position cannot: no single wall is "the sixteenth", so
/// demolishing one does not re-price its neighbours differently from
/// demolishing another.
///
/// A base with no graded piece counted — a detached shack inside a claim's
/// cushion, or a cache that has not seen the base yet — pays the first rung,
/// which is also what a one-piece base pays.
pub fn tax_parts(base_pct: u16, steps: &[(u16, u16)], graded: u32) -> Tax {
    let n = graded.max(1);
    let mut total = 0u64;
    let mut priced = 0u32;
    let mut rate = base_pct as u64 * 10; // percent → per mille
    for &(after, permille) in steps {
        let upto = n.min(after as u32);
        if upto > priced {
            total += (upto - priced) as u64 * rate;
            priced = upto;
        }
        rate = permille as u64;
    }
    if n > priced {
        total += (n - priced) as u64 * rate;
    }
    Tax {
        num: total,
        den: n as u64 * 1000,
    }
}

// ---------------------------------------------------------------------------
// 2 · Inside
// ---------------------------------------------------------------------------

/// The height of socket `level` in half-storeys — `build::level_y` in
/// 1.5 m units, integer. Levels `0..MAX_BUILD_LEVELS` are whole storeys and
/// the half-storey sockets above them sit one unit higher (catalogue v2's
/// encoding), so bit order in a column mask is **not** height order, which
/// is why every "above" question here goes through this.
pub fn half_steps(level: u8) -> u8 {
    (level % MAX_BUILD_LEVELS as u8) * 2 + level / MAX_BUILD_LEVELS as u8
}

/// How many half-storeys a piece of this shape stands above its own
/// socket — where its top is, for the "is anything over it" question. A
/// full-height edge piece and a flight of stairs rise a storey; a plane, a
/// half wall and a low wall are covered by the next socket up.
fn rise(shape: u8) -> u8 {
    match shape {
        SHAPE_WALL | SHAPE_DOORWAY | SHAPE_WINDOW | SHAPE_FRAME => 2,
        s if crate::circulation::is_riser(s) => 2,
        _ => 1,
    }
}

/// The column sockets at or above `need` half-storeys, as a level mask.
fn from(need: u8) -> u16 {
    let mut m = 0u16;
    for b in 0..MAX_BUILD_SOCKETS as u8 {
        if half_steps(b) >= need {
            m |= 1 << b;
        }
    }
    m
}

/// A triangle corner's frame flags as a level mask (`ColMasks::tri_frames`
/// packs four per level).
fn tri_frame_bits(m: &ColMasks, corner: u32) -> u16 {
    let mut out = 0u16;
    for b in 0..MAX_BUILD_SOCKETS as u32 {
        if (m.tri_frames >> (b * 4 + corner)) & 1 != 0 {
            out |= 1 << b;
        }
    }
    out
}

/// What stands over a cell's **centre**: a solid plane or triangle, a
/// flight, or a diagonal wall through it. Frames are holes and do not count
/// — a stair under a floor frame is out in the weather.
fn centre_bits(m: &ColMasks) -> u16 {
    let mut solid_tri = 0u16;
    for (corner, bits) in [m.tri_xlo_zlo, m.tri_xhi_zlo, m.tri_xlo_zhi, m.tri_xhi_zhi]
        .into_iter()
        .enumerate()
    {
        solid_tri |= bits & !tri_frame_bits(m, corner as u32);
    }
    m.planes
        | solid_tri
        | m.stairs
        | m.stairs_xhi
        | m.stairs_zlo
        | m.stairs_xlo
        | m.diag_a
        | m.diag_b
        | m.half_diag_a
        | m.half_diag_b
        | m.low_diag_a
        | m.low_diag_b
}

/// What stands over a cell's **rim** — the centre's set plus the frames,
/// because a floor frame's open middle still sits on the walls around it.
fn rim_bits(m: &ColMasks) -> u16 {
    centre_bits(m) | m.floor_frames | m.tri_xlo_zlo | m.tri_xhi_zlo | m.tri_xlo_zhi | m.tri_xhi_zhi
}

/// Every edge piece on one of a cell's two canonical edges.
fn edge_bits(m: &ColMasks, zlo: bool) -> u16 {
    if zlo {
        m.walls_zlo | m.doors_zlo | m.wins_zlo | m.frames_zlo | m.half_zlo | m.low_zlo
    } else {
        m.walls_xlo | m.doors_xlo | m.wins_xlo | m.frames_xlo | m.half_xlo | m.low_xlo
    }
}

/// Whether the piece at this address is **inside** — whether anything of a
/// base stands over its top (upkeep v2, rule 2).
///
/// The reference publishes an upward test out to 50 m
/// (`decay.outside_test_range`) and, for deployables, *"no roof / overhang
/// above them"* — not the ray's exact start (`BUILDING.md` §5). Ours asks
/// the grid that question: over a cell's centre it meets planes,
/// triangles, flights and diagonal walls; over an edge it meets the same
/// plus anything whose rim reaches the edge on either side, and any edge
/// piece stacked on the same line. What the grid cannot say it does not
/// guess — terrain overhead is ignored, because no build cell has any.
///
/// The consequence is the reference's order of rot: a roof has nothing
/// over it and goes first; the storey under it is inside until it does.
pub fn inside(cols: &ColIndex, cx: u16, cz: u16, level: u8, loc: u8, shape: u8) -> bool {
    inside_at(cols, cx, cz, level, loc, rise(shape))
}

/// A deployable's version of [`inside`]: a body on a plane is covered by
/// the next socket up, a door in an edge by what covers a wall. The door's
/// own doorway is not counted over it — a lintel over a leaf is the
/// doorway sheltering itself, and every door would be inside by fiat.
///
/// A body deployable's `loc` is a slot name (`deploy::BODY_LOCS`), not a
/// place in the cell — slot 8 is not a diagonal — so everything that is not
/// an edge is asked as the plane it stands on.
pub fn deploy_inside(cols: &ColIndex, cx: u16, cz: u16, level: u8, loc: u8) -> bool {
    if loc == LOC_EDGE_XLO || loc == LOC_EDGE_ZLO {
        inside_at(cols, cx, cz, level, loc, 2)
    } else {
        inside_at(cols, cx, cz, level, crate::build::LOC_PLANE, 1)
    }
}

fn inside_at(cols: &ColIndex, cx: u16, cz: u16, level: u8, loc: u8, rise: u8) -> bool {
    let above = from(half_steps(level).saturating_add(rise));
    if above == 0 {
        return false;
    }
    let own = cols.get(cx, cz);
    match loc {
        LOC_EDGE_XLO | LOC_EDGE_ZLO => {
            let zlo = loc == LOC_EDGE_ZLO;
            if (edge_bits(&own, zlo) | rim_bits(&own)) & above != 0 {
                return true;
            }
            // The cell on the edge's other side, when there is one: an edge
            // on the grid's own low boundary has no neighbour to ask.
            let other = if zlo {
                cz.checked_sub(1).map(|z| (cx, z))
            } else {
                cx.checked_sub(1).map(|x| (x, cz))
            };
            other.is_some_and(|(x, z)| rim_bits(&cols.get(x, z)) & above != 0)
        }
        // A diagonal wall splits its own cell; the pieces stacked on the
        // same diagonal cover it as a wall above a wall does.
        LOC_DIAG_A => {
            (centre_bits(&own) | own.diag_a | own.half_diag_a | own.low_diag_a) & above != 0
        }
        LOC_DIAG_B => {
            (centre_bits(&own) | own.diag_b | own.half_diag_b | own.low_diag_b) & above != 0
        }
        _ => centre_bits(&own) & above != 0,
    }
}

/// One unpaid period's decay for a piece of `max_hp` at `pct` % a period,
/// scaled to `scale` % of that: `max(1, max_hp × pct × scale / 10 000)`.
/// Outside, `scale` is 100 and this is upkeep/decay v1's step exactly.
///
/// The floor of 1 is v1's and keeps its reason: a rate that rounds to zero
/// is decay switched off by arithmetic, and off is a thing content has to
/// *say* (`upkeep_pct_per_day = 0`).
pub fn decay_step(max_hp: u16, pct: u32, scale: u32) -> u16 {
    ((max_hp as u32 * pct * scale) / 10_000).max(1) as u16
}

/// The scale [`decay_step`] runs at: 100 outside, the content's inside
/// rate over something built. An inside rate of zero is the content saying
/// nothing, and answers with the full rate — never with immortality.
pub fn scale(dc: &DeployContent, inside: bool) -> u32 {
    if inside && dc.inside_decay_pct > 0 {
        dc.inside_decay_pct as u32
    } else {
        100
    }
}

// ---------------------------------------------------------------------------
// 3 · How long it lasts
// ---------------------------------------------------------------------------

/// What hearth `hi` charges in a **day** for everything it covers, per
/// stock row (aligned to `dc.mats`): each material's cost over the covered
/// pieces, doors and inserts, priced at the hearth's rent and rounded up once.
///
/// Read off the cached claim volume the sweep reads, so a readout and the
/// rent cannot disagree about which pieces are this hearth's; like the
/// sweep, it prices twig at nothing. Where two hearths cover one piece the
/// sweep charges whichever can pay and this counts it against both — an
/// over-statement in the direction that makes a player feed early, never
/// late.
///
/// Bounded by the piece and deploy stores (`MAX_PIECES` + `MAX_DEPLOYS`
/// visits, each O(1) for an address in the base itself —
/// `claim::ClaimCache::covers`' membership probe), and asked per feed
/// press and by the crew vital's push (`deploy::CREW_VITAL_TICKS`, phased
/// by connection so a tick asks it at most once), never per player per
/// tick.
pub fn bill(
    dc: &DeployContent,
    bc: &BuildContent,
    pieces: &Pieces,
    deploys: &Deploys,
    hi: usize,
) -> [u32; HEARTH_STOCK_ROWS] {
    let mut out = [0u32; HEARTH_STOCK_ROWS];
    if hi >= deploys.hearths().len() {
        return out;
    }
    let t = hearth_tax(dc, deploys, hi);
    let mats = &dc.mats[..(dc.mat_count as usize).min(HEARTH_STOCK_ROWS)];
    let mut cost = [0u64; HEARTH_STOCK_ROWS];
    for rec in pieces.entries() {
        let def = bc.pieces[rec.row as usize];
        if def.material == MAT_TWIG {
            continue;
        }
        let (x, z) = cell_center(rec.cx, rec.cz);
        if !deploys.hearth_covers(hi, x, z) {
            continue;
        }
        for &(item, c) in def.costs.iter().take(def.n_costs as usize) {
            if let Some(m) = mats.iter().position(|&mi| mi == item) {
                cost[m] += c as u64;
            }
        }
    }
    // The doors and inserts the sweep charges (`deploy::pays_rent`), at
    // their own build cost.
    for rec in deploys.entries() {
        let def = dc.defs[rec.row as usize];
        if rec.owner == WORLD_OWNER || !pays_rent(&def) {
            continue;
        }
        let (x, z) = rec.xz();
        if !deploys.hearth_covers(hi, x, z) {
            continue;
        }
        for &(item, c) in def.costs.iter().take(def.n_costs as usize) {
            if let Some(m) = mats.iter().position(|&mi| mi == item) {
                cost[m] += c as u64;
            }
        }
    }
    for (o, &c) in out.iter_mut().zip(&cost) {
        *o = t.per_day(c);
    }
    out
}

/// How many whole hours a stock covers a **daily** bill for: the least of
/// `stock × 24 / bill` over the rows that charge anything, or `None` when
/// nothing is charged at all (a base of twig, or a hearth on bare
/// foundations it does not pay for) — "protected for ∞" is a claim this
/// readout declines to make about a base that is not being protected.
///
/// The least row because upkeep is per material (v1): the stone runs out
/// first, so the stone is what starts to rot first, and a readout quoting
/// the wood's forty hours would be describing a base that is already
/// crumbling.
pub fn lasts(stock: &[u32], bill: &[u32]) -> Option<u32> {
    stock
        .iter()
        .zip(bill)
        .filter(|&(_, &b)| b > 0)
        .map(|(&s, &b)| (s as u64 * PERIODS_PER_DAY as u64 / b as u64).min(u32::MAX as u64) as u32)
        .min()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build::{
        PieceDef, LOC_PLANE, MAT_STONE, SHAPE_FLOOR, SHAPE_FLOOR_FRAME, SHAPE_FOUNDATION,
    };
    use crate::limits::MAX_BUILD_LEVELS;

    /// The reference's ladder as `content/balance.toml` ships it: 10 % a
    /// day, then 15 / 20 / 33.3 % past 15, 65 and 190 graded pieces
    /// (`BUILDING.md` §4b — the sizes reading of their bracket convars).
    const REF: [(u16, u16); 3] = [(15, 150), (65, 200), (190, 333)];

    /// A rate in hundredths of a percent, exact: `num × 10 000 / den`.
    fn bp(t: Tax) -> u64 {
        t.num * 10_000 / t.den
    }

    #[test]
    fn a_day_of_hours_pays_the_days_rent_at_any_phase() {
        // 100 wood at 10 %/day is 10 a day, not 24 rounded-up hours.
        for n in [1u32, 15, 20, 200] {
            let t = tax_parts(10, &REF, n);
            for cost in [1u32, 5, 24, 100, 300, 2400, 65_535, 1_000_000] {
                let day = (cost as u128 * t.num as u128) / t.den as u128;
                for phase in [0u64, 1, 7, 23, 1_000, u32::MAX as u64] {
                    let sum: u128 = (1..=24).map(|h| t.due(cost, phase + h) as u128).sum();
                    assert!(
                        sum == day || sum == day + 1,
                        "{n} pieces, cost {cost}, phase {phase}: {sum} for a {day} day"
                    );
                }
            }
        }
        let t = tax_parts(10, &REF, 1);
        assert_eq!((1..=24).map(|h| t.due(100, h)).sum::<u32>(), 10);
        assert_eq!(t.per_day(100), 10);
    }

    #[test]
    fn the_group_tax_is_breach_and_clears() {
        // Their worked examples: six players +4 %, twelve +20 %.
        assert_eq!(group_pct(1), 0);
        assert_eq!(group_pct(4), 0);
        assert_eq!(group_pct(6), 4);
        assert_eq!(group_pct(10), 12);
        assert_eq!(group_pct(12), 20);
        assert_eq!(group_pct(1000), GROUP_CAP_PCT);
        // A 100-wood piece at 10 % a day pays 10; with six on the crew, 10.4.
        let t = tax_parts(10, &[], 1).plus_pct(group_pct(6));
        assert_eq!(t.per_day(1000), 104);
    }

    #[test]
    fn the_ladder_blends_to_the_references_own_rates() {
        // (15 × 10 % + 5 × 15 %) / 20 = 11.25 % — the worked example the
        // reference's guides quote as 11.3 %, and the rest of the sizes
        // reading's row from `BUILDING.md` §4b.
        assert_eq!(bp(tax_parts(10, &REF, 20)), 1125);
        assert_eq!(bp(tax_parts(10, &REF, 65)), 1384);
        assert_eq!(bp(tax_parts(10, &REF, 190)), 1789);
        assert_eq!(bp(tax_parts(10, &REF, 300)), 2354);
        // Monotone in the base's size — no piece ever makes a base cheaper
        // per piece by being added.
        let mut last = 0;
        for n in 1..=400u32 {
            let r = bp(tax_parts(10, &REF, n));
            assert!(r >= last, "the rate fell at {n} pieces");
            last = r;
        }
    }

    #[test]
    fn an_empty_ladder_is_the_flat_rate_and_nobody_counted_is_one_piece() {
        for n in [1u32, 7, 100] {
            assert_eq!(bp(tax_parts(10, &[], n)), 1000);
        }
        assert_eq!(tax_parts(10, &REF, 0), tax_parts(10, &REF, 1));
    }

    #[test]
    fn lasts_names_the_first_material_to_run_out() {
        // Daily bills, answered in hours.
        assert_eq!(
            lasts(&[100, 30], &[10, 5]),
            Some(144),
            "the stone runs out first"
        );
        assert_eq!(
            lasts(&[100, 0], &[10, 0]),
            Some(240),
            "an unbilled row is not a limit"
        );
        assert_eq!(
            lasts(&[0, 50], &[10, 5]),
            Some(0),
            "short now is zero, not none"
        );
        assert_eq!(
            lasts(&[9, 9], &[0, 0]),
            None,
            "nothing charged, nothing claimed"
        );
    }

    #[test]
    fn decay_inside_is_the_contents_fraction_and_never_nothing() {
        assert_eq!(decay_step(200, 20, 100), 40, "outside is v1's step");
        assert_eq!(decay_step(200, 20, 10), 4, "inside is a tenth of it");
        assert_eq!(decay_step(10, 1, 10), 1, "and still subtracts");
        let mut dc = DeployContent::probe_fixture();
        assert_eq!(scale(&dc, true), 10);
        assert_eq!(scale(&dc, false), 100);
        dc.inside_decay_pct = 0;
        assert_eq!(scale(&dc, true), 100, "unpriced is the full rate");
    }

    #[test]
    fn half_steps_order_the_sockets_by_height() {
        assert_eq!(half_steps(0), 0);
        assert_eq!(half_steps(1), 2);
        assert_eq!(
            half_steps(MAX_BUILD_LEVELS as u8),
            1,
            "the first half-storey"
        );
        assert_eq!(half_steps(MAX_BUILD_LEVELS as u8 + 1), 3);
        for l in 0..MAX_BUILD_SOCKETS as u8 {
            let y = crate::build::level_y(l);
            assert_eq!(half_steps(l) as f32 * crate::build::LEVEL_H_M * 0.5, y);
        }
    }

    /// A build table with a floor frame (row 7) beside the probe rows.
    fn with_frame() -> BuildContent {
        let mut bc = BuildContent::probe_fixture();
        bc.pieces[7] = PieceDef {
            shape: SHAPE_FLOOR_FRAME,
            material: MAT_STONE,
            hp: 200,
            n_costs: 1,
            costs: [(1, 4), (0, 0)],
        };
        bc.piece_count = 8;
        bc
    }

    #[test]
    fn a_roof_shelters_what_stands_under_it_and_nothing_shelters_the_roof() {
        let bc = BuildContent::probe_fixture();
        let mut p = Pieces::new();
        p.insert_for_test(10, 10, 0, LOC_PLANE, 0, &bc);
        p.insert_for_test(10, 10, 0, LOC_EDGE_XLO, 1, &bc);
        p.insert_for_test(10, 10, 1, LOC_PLANE, 2, &bc);
        let c = p.cols();
        assert!(inside(c, 10, 10, 0, LOC_PLANE, SHAPE_FOUNDATION));
        assert!(
            inside(c, 10, 10, 0, LOC_EDGE_XLO, SHAPE_WALL),
            "under the roof's rim"
        );
        assert!(
            !inside(c, 10, 10, 1, LOC_PLANE, SHAPE_FLOOR),
            "open sky over the roof"
        );
        assert!(deploy_inside(c, 10, 10, 0, LOC_PLANE), "a box on the floor");
    }

    #[test]
    fn open_ground_is_outside() {
        let bc = BuildContent::probe_fixture();
        let mut p = Pieces::new();
        p.insert_for_test(10, 10, 0, LOC_PLANE, 0, &bc);
        p.insert_for_test(10, 10, 0, LOC_EDGE_ZLO, 1, &bc);
        let c = p.cols();
        assert!(!inside(c, 10, 10, 0, LOC_PLANE, SHAPE_FOUNDATION));
        assert!(!inside(c, 10, 10, 0, LOC_EDGE_ZLO, SHAPE_WALL));
        assert!(!deploy_inside(c, 10, 10, 0, LOC_PLANE));
    }

    #[test]
    fn an_edge_is_sheltered_from_either_side_and_by_a_wall_above_it() {
        let bc = BuildContent::probe_fixture();
        // The wall on (10,10)'s low-x edge, roofed only from the cell on
        // its far side, (9,10).
        let mut p = Pieces::new();
        p.insert_for_test(10, 10, 0, LOC_EDGE_XLO, 1, &bc);
        p.insert_for_test(9, 10, 1, LOC_PLANE, 2, &bc);
        assert!(inside(p.cols(), 10, 10, 0, LOC_EDGE_XLO, SHAPE_WALL));
        // A second storey of wall on the same line and nothing else: the
        // lower one is covered, the upper one is not.
        let mut q = Pieces::new();
        q.insert_for_test(10, 10, 0, LOC_EDGE_XLO, 1, &bc);
        q.insert_for_test(10, 10, 1, LOC_EDGE_XLO, 1, &bc);
        assert!(inside(q.cols(), 10, 10, 0, LOC_EDGE_XLO, SHAPE_WALL));
        assert!(!inside(q.cols(), 10, 10, 1, LOC_EDGE_XLO, SHAPE_WALL));
        // An edge on the grid's own low boundary asks no neighbour.
        let mut r = Pieces::new();
        r.insert_for_test(0, 10, 0, LOC_EDGE_XLO, 1, &bc);
        assert!(!inside(r.cols(), 0, 10, 0, LOC_EDGE_XLO, SHAPE_WALL));
    }

    #[test]
    fn a_floor_frame_shelters_the_walls_under_its_rim_but_not_the_floor_under_its_hole() {
        let bc = with_frame();
        let mut p = Pieces::new();
        p.insert_for_test(10, 10, 0, LOC_PLANE, 0, &bc);
        p.insert_for_test(10, 10, 0, LOC_EDGE_XLO, 1, &bc);
        p.insert_for_test(10, 10, 1, LOC_PLANE, 7, &bc);
        let c = p.cols();
        assert!(
            !inside(c, 10, 10, 0, LOC_PLANE, SHAPE_FOUNDATION),
            "under the hole"
        );
        assert!(
            inside(c, 10, 10, 0, LOC_EDGE_XLO, SHAPE_WALL),
            "under the rim"
        );
    }

    #[test]
    fn a_half_storey_floor_covers_a_floor_but_not_the_wall_it_meets_halfway() {
        let bc = BuildContent::probe_fixture();
        let half = MAX_BUILD_LEVELS as u8; // one and a half metres up
        let mut p = Pieces::new();
        p.insert_for_test(10, 10, 0, LOC_PLANE, 0, &bc);
        p.insert_for_test(10, 10, 0, LOC_EDGE_XLO, 1, &bc);
        p.insert_for_test(10, 10, half, LOC_PLANE, 2, &bc);
        let c = p.cols();
        assert!(inside(c, 10, 10, 0, LOC_PLANE, SHAPE_FOUNDATION));
        assert!(
            !inside(c, 10, 10, 0, LOC_EDGE_XLO, SHAPE_WALL),
            "a slab halfway up a wall is beside its top, not over it"
        );
    }

    /// The readout's bill is the sweep's charge summed, and it is what the
    /// ladder is for: a bigger base pays more per piece. Costs are raised
    /// past the probe fixture's so the per-period ceiling does not hide the
    /// rate — at a cost of 5 every rate up to 480 % a day charges 1.
    #[test]
    fn the_bill_rises_faster_than_the_base() {
        let dc = DeployContent::probe_fixture(); // steps past 3 and 6
        let mut bc = BuildContent::probe_fixture();
        bc.pieces[5].costs = [(0, 3000), (0, 0)];
        let bill_for = |n: u16| {
            let mut pieces = Pieces::new();
            let mut deploys = Deploys::new();
            for cx in 100..100 + n {
                pieces.insert_for_test(cx, 100, 0, LOC_PLANE, 5, &bc);
            }
            // A twig piece in the base changes nothing: no rent, no count.
            pieces.insert_for_test(99, 100, 0, LOC_PLANE, 0, &bc);
            deploys.push_hearth_for_test(100, 100, 0, 7);
            deploys.refresh_claims(&pieces, &bc);
            assert_eq!(deploys.claim_graded(0), n as u32);
            bill(&dc, &bc, &pieces, &deploys, 0)[0]
        };
        // A day's bill. One piece: 3000 × 10 % = 300. Eight: past both
        // steps, (3 × 100 + 3 × 150 + 2 × 200) / 8 = 143.75 ‰ of 24 000 =
        // 3450.
        assert_eq!(bill_for(1), 300);
        assert_eq!(bill_for(8), 3450);
        assert!(bill_for(8) > 8 * bill_for(1));
    }
}
