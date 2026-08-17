//! The HUD and the viewmodel — `ART.md` §6 and §8's last bullet.
//!
//! "A frame with no viewmodel and no HUD reads as a flythrough, which the
//! blind reader has named on every capture so far." It is the cheapest scored
//! criterion in the art rubric and the one the browser client had first, so
//! the native client is not shipping captures without it.
//!
//! The reference set's shape, measured off the reference set: a **bottom-centre
//! hotbar** with item cells, a **right-side vitals stack** with numbers —
//! small, unobtrusive, never centred — and a **held item** in the lower
//! right of frame.
//!
//! Every number drawn here is the server's. `hp_max` or `max_food` at zero
//! means no such message has ever arrived — a shard whose content disarms
//! combat or has no `[survival]` section never sends one — and the rule that
//! `core.rs` states for those fields is honoured: draw nothing rather than
//! draw an empty bar for a player who cannot be hurt.

use bevy::prelude::*;
use sim_core::limits::HOTBAR_SLOTS;

use super::verbs::Aimed;
use super::Net;

/// How long a toast stays up, seconds. Cosmetic (`DECISIONS.md` §open,
/// client cosmetics). A clock is fine here and would not be in a gate: this
/// is a fade, not an assertion (`CLAUDE.md`, "a gate that waits on a clock
/// is not a gate").
pub const TOAST_SECS: f32 = 3.0;

/// How many announce lines are on screen at once, newest at the top.
///
/// **The cap is the point, not the count.** One frame can produce a refusal,
/// a kill, two gathers and two spills; the sink used to be one `String` and
/// last-writer-wins, so five of those six were computed, formatted and
/// thrown away. Overflow policy, stated as wall 4 asks: **drop-oldest-note,
/// then drop-oldest, counted** ([`Toast::dropped`]) — the victim is the
/// oldest [`Rank::Note`], and only an all-alarm stack falls back to the
/// oldest line outright. A push is never refused either way: the newest fact
/// is the one a player is reacting to, and a queue that dropped the newest
/// would be a queue that hides the thing that just happened. [`Toast::push`]
/// is where the rule lives.
///
/// Four because four rows fit between the prompt and the readout at the
/// pitch below without reaching the crosshair — a sentence that was prose
/// until 2026-08-14 and is now three assertions in
/// `the_stack_hangs_between_the_prompt_and_the_hotbar`. Knob:
/// `DECISIONS.md` §open, client cosmetics (declared there, so
/// `ci/knob_registry.mjs` pins it).
///
/// **It multiplies with [`TOAST_ROW_DIM`]**, which is why raising it is not
/// free: at 8 the deepest row draws at alpha zero and the cap would hold a
/// line nothing can show (`every_row_the_cap_holds_can_be_seen`).
pub const TOAST_LINES: usize = 4;

/// Where the newest announce line sits, percent of screen height, and the
/// gap to the next one. Row 0 keeps the single toast's own position, so
/// nothing moved for the one-line case that was all this could ever show.
/// Cosmetic (`DECISIONS.md` §open, client cosmetics).
const TOAST_TOP_PCT: f32 = 58.0;
const TOAST_PITCH_PCT: f32 = 2.8;

/// How much dimmer each row is than the one above it. The stack has to read
/// as a stack — without it four equal lines are a paragraph, and the eye has
/// no reason to start at the new one. Cosmetic (as above).
const TOAST_ROW_DIM: f32 = 0.16;

/// The announce stack's own type size, px. Named rather than typed at the
/// spawn because [`toast_min_window_height_px`] divides by its line box, and
/// a font size that lived only at the spawn site would be a number the
/// geometry could not see.
const TOAST_FONT_PX: f32 = 14.0;

/// The centre prompt: where it sits and how big it is. The announce stack
/// hangs below it, so it is one of the two ends of the "four rows fit
/// between the prompt and the readout" claim in [`TOAST_LINES`]' doc — and
/// that claim was prose until [`the_stack_hangs_between_the_prompt_and_the_
/// hotbar`] made it arithmetic.
const PROMPT_TOP_PCT: f32 = 54.0;
const PROMPT_FONT_PX: f32 = 15.0;

/// The crosshair's centre and the `(dx, dy, w, h)` of its four ticks. The
/// tick table is here rather than inline in [`setup`] because the other half
/// of the same claim — *without reaching the crosshair* — is a question
/// about how far the bottom tick reaches, which is arithmetic over this
/// table and not a number anybody should retype.
const CROSSHAIR_CENTRE_PCT: f32 = 50.0;
const CROSSHAIR_TICKS: [(f32, f32, f32, f32); 4] = [
    (0.0, -9.0, 2.0, 7.0),
    (0.0, 9.0, 2.0, 7.0),
    (-9.0, 0.0, 7.0, 2.0),
    (9.0, 0.0, 7.0, 2.0),
];

/// The hotbar's two numbers: how far its row floats off the bottom edge, and
/// how tall a cell is. Named for the same reason as the prompt's — the
/// readout has to clear them, and a clearance is arithmetic between two
/// things that must both be visible to it.
///
/// **They are px off the BOTTOM and the stack is percent off the TOP**, which
/// is the whole reason [`toast_min_window_height_px`] exists: the gap between
/// the two shrinks as the window does, and nothing in this file could see
/// that until both ends had names.
const HOTBAR_BOTTOM_PX: f32 = 18.0;
const HOTBAR_CELL_PX: f32 = 46.0;

/// How long the hitmarker flashes, seconds. Short on purpose — it is
/// confirmation, not a readout.
pub const HITMARK_SECS: f32 = 0.25;

// ---- the announce stack's geometry --------------------------------------
//
// **Four knobs that are coupled, and nothing checked the coupling.**
// `TOAST_LINES`, `TOAST_TOP_PCT`, `TOAST_PITCH_PCT` and `TOAST_ROW_DIM` were
// each defensible alone and were only ever read at the two spawn sites, which
// computed the same expression twice and agreed by luck. Three passes landed
// announce work on top of that and `NOW.md` §0tq ended every one of them on
// the same sentence: *row pitch, the per-row dim and the readout's new home
// are all unlooked-at arithmetic.*
//
// This block is the looking. It is arithmetic about a frame — the shape
// `CLAUDE.md` explicitly allows ("what may be gated about a frame is
// arithmetic … in Rust, the shape of `crates/client/tests/tree.rs`") and not
// a pixel statistic, which that same list forbids for the reason a beige
// smear once scored 36 of 36.

/// Where announce row `row` sits, percent of screen height. Row 0 is the
/// newest and keeps the single toast's own position.
///
/// A function rather than the expression it replaces, because [`setup`]
/// computed it in two places — once per row and once for the readout — and
/// two copies of one rule is one rule and one bug waiting.
fn toast_row_top_pct(row: usize) -> f32 {
    TOAST_TOP_PCT + row as f32 * TOAST_PITCH_PCT
}

/// Where the pinned readout sits: one pitch below the last row the stack can
/// hold, whether or not that row is live.
///
/// **Derived from [`TOAST_LINES`], never typed.** The readout is the thing a
/// deeper stack would land on top of, so a cap that grew and a readout that
/// did not is the exact drift this expression exists to make impossible.
fn readout_top_pct() -> f32 {
    toast_row_top_pct(TOAST_LINES)
}

/// What row `row`'s alpha is scaled by — 1.0 at the newest, one
/// [`TOAST_ROW_DIM`] step per row down. Clamped at zero, which is also the
/// value [`every_row_the_cap_holds_can_be_seen`] refuses to reach.
fn toast_row_dim(row: usize) -> f32 {
    (1.0 - TOAST_ROW_DIM * row as f32).max(0.0)
}

/// A text node's line box, px.
///
/// **Read off Bevy's own default rather than written down as 1.2.** In Bevy
/// 0.18 `LineHeight` stopped being a `TextFont` field and became a required
/// component of `Text` in its own right, and nothing in this crate sets one —
/// so every HUD line takes the default, and a bump that moves it must move
/// this with it. A literal here would go quietly wrong at exactly that
/// moment.
///
/// `#[cfg(test)]` because the RUNTIME never needs it — Bevy lays the text out
/// and this only says what Bevy will do. The two functions below it are the
/// same: they derive a consequence of the shipped constants for the gate to
/// assert, which is the opposite of a gate computing its own answer.
#[cfg(test)]
fn line_box_px(font_size: f32) -> f32 {
    match bevy::text::LineHeight::default() {
        bevy::text::LineHeight::Px(px) => px,
        bevy::text::LineHeight::RelativeToFont(scale) => scale * font_size,
    }
}

/// The shortest window in which the announce stack does not overlap itself.
///
/// **The mixed-unit seam, and the one number in this file nobody had.** The
/// pitch is a percent of window height and the type size is px, so the gap
/// between two rows shrinks with the window while the text in them does not.
/// Above this height the rows are separated; below it row 1's box climbs into
/// row 0's and four lines become a smear — the same failure the single-slot
/// sink had, arriving from the other direction.
///
/// At the shipped knobs this is 600 px, against the 720 the client opens at,
/// so the margin is real but it is not large: a `TOAST_PITCH_PCT` trimmed to
/// 2.3 would put the threshold above the default window.
#[cfg(test)]
fn toast_min_window_height_px() -> f32 {
    line_box_px(TOAST_FONT_PX) / (TOAST_PITCH_PCT / 100.0)
}

/// A vitals bar, px. The reference's are ~112 × 19 in a 1200-wide frame;
/// these are the same proportion at 1280, and the height is what makes the
/// icon square square. Cosmetic (`DECISIONS.md` §open, client cosmetics).
pub const VITAL_BAR_W: f32 = 118.0;
pub const VITAL_BAR_H: f32 = 19.0;

/// What a line is worth when the sink is full.
///
/// **Not importance in the abstract — recoverability.** A line is an
/// [`Rank::Alarm`] when the fact it carries exists nowhere else the moment
/// the line is gone: a refusal (the sim said no, and nothing but this
/// sentence records that it did), a spill (your loot is on the floor and the
/// only other clue is a bag you have to notice underfoot), a charge going
/// live (a clock somebody else started). A [`Rank::Note`] is recoverable by
/// looking — a gather and a craft are in the pack, the hearth's stock is in
/// its panel, a kill is somebody else's news.
///
/// **Why the sink needs this at all.** Until now the only ordering the
/// overflow policy had was *insertion*, and insertion order was set by which
/// system happened to run first: `feedback` wrote refusals before payouts,
/// so drop-oldest ate the refusal and kept the mushrooms. That made the
/// write order load-bearing without saying so anywhere — the next
/// `toast.say` added at the top of a system would silently re-rank
/// everything below it, and there are 35 of them across five files. Rank is
/// the same decision written down where the eviction can read it.
#[derive(Default, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Rank {
    /// Recoverable by looking. Eaten first when the cap bites.
    #[default]
    Note,
    /// Gone with the line. Survives a flood of notes.
    Alarm,
}

/// One announce line, with its own clock.
#[derive(Default)]
pub struct Say {
    /// The line as drawn, repeat suffix and all.
    text: String,
    /// What the cap does to this line when it has to eat one.
    rank: Rank,
    /// Bytes of `text` that are the sentence itself. The repeat suffix is
    /// rewritten from here, so the fifth swing at a tree costs no allocation
    /// — a `String` grown once and truncated back, never a new one. The
    /// client is a hot path too (`CLAUDE.md`, "no per-frame allocations").
    base: usize,
    /// Seconds left before this line goes.
    left: f32,
    /// How many times this exact sentence was said while it was up.
    reps: u16,
}

impl Say {
    /// The line as it should appear on screen.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Seconds left. The fade reads it; nothing asserts on it.
    pub fn left(&self) -> f32 {
        self.left
    }

    pub fn reps(&self) -> u16 {
        self.reps
    }

    /// What the cap would do to this line. Read by the tests; the eviction
    /// reads the field directly.
    pub fn rank(&self) -> Rank {
        self.rank
    }
}

/// What just happened in the world: a refusal, a full action lane, a verb
/// that found nothing, a gather, a spill, a death.
///
/// **A bounded queue, not a line.** Until 2026-08-14 this was one `String`
/// and one timer, so `feedback` wrote gathers, crafts, refusals, knocks,
/// kills and spills into the same slot in a fixed order and the player saw
/// whichever spoke last. That was not cosmetic: chopping a shipped tree with
/// a full pack pushes two `EV_GATHER` zeros (wood and the mushroom
/// secondary, `content/gatherables.toml`), so the one thing the spill line
/// exists to say — *your wood went on the floor* — was overwritten by the
/// mushrooms every time. Every fact in `Feed` is already bounded and
/// drained once; only the sink was one slot.
///
/// Distinct from `panels::Ui::status`, which says what happened *in a panel*
/// and is only on screen while one is open. Every refusal the sim announces
/// used to reach this client and stop — `pop_hit`, `pop_toast`,
/// `pop_craft_refusal`, `pop_build_refusal` and `pop_deploy_refusal` had zero
/// call sites in the whole render path.
#[derive(Resource)]
pub struct Toast {
    /// Oldest first. `len` of these are live; the rest are spent slots kept
    /// for their allocation.
    lines: [Say; TOAST_LINES],
    len: usize,
    /// Lines the cap ate, ever. Wall 4 wants an overflow policy *and* a way
    /// to see it fire; a drop nobody counts is a drop nobody can find.
    dropped: u32,
    /// Lines the cap ate that the player never saw, in the current burst —
    /// reset when the stack next empties, because that is when the burst is
    /// over and there is nothing left on screen for a count to qualify.
    ///
    /// `dropped` is the lifetime counter and stays one: it is evidence for a
    /// test. This is the one the HUD draws, and the two are different
    /// questions ("has the cap ever bitten" vs "how much am I not being
    /// told right now").
    unseen: u16,
    /// [`Self::unseen`] as drawn, rewritten only when the count moves.
    ///
    /// The draw loop runs every frame and the count changes on an overflow,
    /// so formatting it at the draw site would be a per-frame `format!` on
    /// the client's hot path (`CLAUDE.md`: no per-frame allocations). Same
    /// trick as `Say::base` one level out — a `String` grown once and
    /// rewritten in place.
    overflow: String,
    /// Seconds left on the hitmarker, counted down beside the text because
    /// the two are the same kind of thing: a brief confirmation the player
    /// reads without looking away from the crosshair.
    pub hit_left: f32,
    /// Damage the last landed hit dealt, drawn beside the marker.
    pub hit_damage: u16,
}

impl Default for Toast {
    fn default() -> Self {
        Self {
            lines: std::array::from_fn(|_| Say::default()),
            len: 0,
            dropped: 0,
            unseen: 0,
            overflow: String::new(),
            hit_left: 0.0,
            hit_damage: 0,
        }
    }
}

impl Toast {
    /// Add a recoverable line ([`Rank::Note`]) — a gather, a craft, a kill.
    pub fn say(&mut self, what: impl Into<String>) {
        self.push(Rank::Note, what.into());
    }

    /// Add a line whose fact dies with it ([`Rank::Alarm`]) — a refusal, a
    /// spill, a charge going live. The cap eats every note on the stack
    /// before it touches one of these.
    pub fn warn(&mut self, what: impl Into<String>) {
        self.push(Rank::Alarm, what.into());
    }

    /// Add a line. A repeat of the newest live line refreshes it and counts
    /// instead of spending a slot: ten swings at a tree must not push a
    /// refusal off the stack, and "+1 × WOOD ×10" is more honest than ten
    /// identical rows anyway.
    ///
    /// **Overflow is drop-oldest-note, then drop-oldest** (wall 4 wants the
    /// policy stated): the victim is the oldest [`Rank::Note`] on the stack,
    /// and only when every live line is an [`Rank::Alarm`] does it fall back
    /// to the oldest line outright. A push is never refused — the newest
    /// fact is the one being reacted to, so a queue that dropped the newest
    /// would hide the thing that just happened.
    ///
    /// Note that this is the only ordering rank has any say in. **Drawing
    /// order stays recency**: rank picks what leaves, never where a survivor
    /// sits, so a frame that does not overflow looks exactly as it did
    /// before rank existed.
    ///
    /// **A repeat can only raise a line's rank, never lower it.** The fast
    /// path returns without spending a slot, so it is the one write that
    /// does not set the rank the eviction reads — and a sentence that
    /// arrives once as recoverable and again as an alarm is an alarm, since
    /// the second arrival is a fact that dies with the line whatever the
    /// first one was. The reverse would be worse than the bug it fixes: an
    /// alarm demoted by a later note would put a refusal back on the menu
    /// the cap eats from.
    fn push(&mut self, rank: Rank, what: String) {
        if self.len > 0 {
            let top = &mut self.lines[self.len - 1];
            if top.text[..top.base] == what {
                top.left = TOAST_SECS;
                top.reps = top.reps.saturating_add(1);
                if rank == Rank::Alarm {
                    top.rank = rank;
                }
                top.text.truncate(top.base);
                use std::fmt::Write as _;
                let _ = write!(top.text, "  ×{}", top.reps);
                return;
            }
        }
        if self.len == TOAST_LINES {
            // The oldest note, or the oldest line if the whole stack is
            // alarms. Rotating from the victim rather than from 0 shifts
            // only what is above it and parks the freed slot at the end,
            // which is where the push below expects to find one.
            let victim = (0..self.len)
                .find(|&i| self.lines[i].rank == Rank::Note)
                .unwrap_or(0);
            self.lines[victim..].rotate_left(1);
            self.len -= 1;
            self.dropped = self.dropped.saturating_add(1);
            self.unseen = self.unseen.saturating_add(1);
            self.write_overflow();
        }
        let slot = &mut self.lines[self.len];
        slot.text.clear();
        slot.text.push_str(&what);
        slot.base = slot.text.len();
        slot.rank = rank;
        slot.left = TOAST_SECS;
        slot.reps = 1;
        self.len += 1;
    }

    /// Rewrite the drawn overflow marker. Called on the two edges `unseen`
    /// moves on — an eviction and the stack emptying — never per frame.
    fn write_overflow(&mut self) {
        self.overflow.clear();
        if self.unseen > 0 {
            use std::fmt::Write as _;
            let _ = write!(self.overflow, "   …+{} more", self.unseen);
        }
    }

    /// Count every line's clock down and retire the ones that ran out.
    ///
    /// Expiry is by timer rather than by position: the newest line's clock
    /// is refreshed by a repeat, so "oldest first" is an insertion order and
    /// not a promise about which runs out first.
    pub fn tick(&mut self, dt: f32) {
        let mut keep = 0;
        for r in 0..self.len {
            self.lines[r].left = (self.lines[r].left - dt).max(0.0);
            if self.lines[r].left > 0.0 {
                self.lines.swap(keep, r);
                keep += 1;
            }
        }
        for slot in self.lines[keep..].iter_mut() {
            slot.text.clear();
            slot.base = 0;
            slot.left = 0.0;
            slot.reps = 0;
        }
        self.len = keep;
        if self.len == 0 && self.unseen > 0 {
            // The burst is over: nothing is on screen for "+N more" to
            // qualify, so the count goes with the lines it was counting.
            // `dropped` does not — that one is evidence, not a readout.
            self.unseen = 0;
            self.write_overflow();
        }
        if self.hit_left > 0.0 {
            self.hit_left = (self.hit_left - dt).max(0.0);
        }
    }

    /// The line drawn in row `row`, newest first. `None` past the live end,
    /// which is the empty row the HUD blanks rather than leaves stale.
    pub fn row(&self, row: usize) -> Option<&Say> {
        if row >= self.len {
            return None;
        }
        Some(&self.lines[self.len - 1 - row])
    }

    /// How many lines are up.
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// What row `row` draws: the line, and the overflow marker if this is
    /// where it rides.
    ///
    /// **The marker is appended to the last live row, never drawn over it.**
    /// The cheap alternative — replace the bottom row's sentence with
    /// "…+N more" — costs a fact to report that facts were lost, and after
    /// the rank change it would cost the *wrong* one: eviction keeps an
    /// alarm by making it the oldest survivor, so the bottom row is exactly
    /// where a rescued refusal ends up. A suffix hides nothing, needs no
    /// fifth row (there is none — the readout sits at
    /// `TOAST_TOP_PCT + TOAST_LINES * TOAST_PITCH_PCT`), and does not move
    /// anything the eye is already on.
    pub fn row_parts(&self, row: usize) -> Option<(&Say, &str)> {
        let say = self.row(row)?;
        let last = row + 1 == self.len;
        Some((say, if last { self.overflow.as_str() } else { "" }))
    }

    /// The overflow marker as drawn — `"   …+2 more"`, or empty when the cap
    /// has not bitten in this burst.
    pub fn overflow(&self) -> &str {
        &self.overflow
    }

    /// How many lines the cap has eaten. Never reset — it is a counter, not
    /// a state.
    pub fn dropped(&self) -> u32 {
        self.dropped
    }

    pub fn hit(&mut self, damage: u16) {
        self.hit_left = HITMARK_SECS;
        self.hit_damage = damage;
    }
}

/// The crosshair's resting colour, and the colour a landed hit flashes it.
/// Cosmetics (`DECISIONS.md` §open, client cosmetics).
const CROSSHAIR: Color = Color::srgba(0.94, 0.93, 0.89, 0.72);
const CROSSHAIR_HIT: Color = Color::srgba(0.98, 0.42, 0.30, 0.95);

/// A hotbar cell, by index.
#[derive(Component)]
pub struct Cell(usize);

/// The picture in a hotbar cell. Index-aligned with [`Cell`] rather than
/// found by parent lookup: the row is spawned once and never reordered, so an
/// index is stable and a hierarchy walk per frame is not worth its cost.
#[derive(Component)]
pub struct CellIcon(usize);

/// The stack count over a hotbar cell's icon.
#[derive(Component)]
pub struct CellCount(usize);

/// The durability pip's trough under a hotbar cell's icon — hidden outright
/// for a slot whose item carries no condition or has not been worn yet, which
/// is `ui::slots::pip_fraction`'s `None`.
///
/// Index-aligned like [`CellIcon`], and spawned once for every cell rather
/// than added and removed as tools come and go: a hotbar cell's contents
/// change every time the player picks something up, and a per-frame spawn is
/// an allocation on the frame path for a bar that is three pixels tall.
#[derive(Component)]
pub struct CellPip(usize);

/// The fill inside [`CellPip`]'s trough. Its width is the fraction.
#[derive(Component)]
pub struct CellPipFill(usize);

/// Which vital a row draws. The reference's order top-to-bottom, which is
/// also its colour order: green, blue, orange.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Vital {
    Hp,
    Water,
    Food,
}

impl Vital {
    /// The vital's value and its maximum. A maximum of 0 means no `Vitals`
    /// message has ever arrived, which is not the same as a zeroed vital —
    /// see the module header.
    fn read(self, core: &client_core::core::ClientCore) -> (u16, u16) {
        match self {
            Vital::Hp => (core.hp, core.hp_max),
            Vital::Water => (core.water, core.max_water),
            Vital::Food => (core.food, core.max_food),
        }
    }

    /// The glyph standing in for the reference's icon.
    ///
    /// **A placeholder, and named as one.** The reference puts a drawn icon
    /// in that square; a glyph is what a client with no icons can say
    /// truthfully until `NOW.md` §0p item 3 lands them.
    fn glyph(self) -> &'static str {
        match self {
            Vital::Hp => "+",
            Vital::Water => "~",
            Vital::Food => "*",
        }
    }

    fn fill(self) -> Color {
        match self {
            Vital::Hp => super::panels::VITAL_HP,
            Vital::Water => super::panels::VITAL_WATER,
            Vital::Food => super::panels::VITAL_FOOD,
        }
    }
}

/// The row for one vital — hidden wholesale when its maximum is 0, which is
/// the "draw nothing rather than an empty bar" rule this module has always
/// held, moved from a string to a `Display`.
#[derive(Component)]
pub struct VitalRow(pub Vital);

/// The coloured part of a bar. Its width is the vital, as a percentage.
#[derive(Component)]
pub struct VitalFill(pub Vital);

/// The number drawn inside the bar.
#[derive(Component)]
pub struct VitalNum(pub Vital);

/// What the build wheel last chose. Drawn beside the hotbar because a
/// selection nothing shows is a selection the player cannot trust — the
/// wheel latches a piece and does not place one yet (`render/ui/wheel.rs`),
/// so this line is the whole of its visible effect.
#[derive(Component)]
pub struct Plan;

/// The centre-screen verb hint: `[E] OPEN BOX`, and the compass strip's
/// neighbour in the reference frames. **This is how a player learns the
/// island has verbs at all** — a key nothing advertises is a key nobody
/// presses.
#[derive(Component)]
pub struct PromptLine;

/// One row of the announce stack, under the prompt. Row 0 is the newest and
/// sits where the single toast line always sat; the rest run downward.
#[derive(Component)]
pub struct ToastLine(pub usize);

/// The pinned readout, under the toast: the WHERE half of the two latched
/// address-carrying signals (`NOW.md` §0x item 6) — the wall being broken
/// and the live charge. Its own line because the toast is an announce
/// surface any gather can stomp, and a raid's progress line vanishing
/// under `+2 × Wood` was the defect.
#[derive(Component)]
pub struct ReadoutLine;

/// What the readout line is holding. `ClientCore::struct_hit` and
/// `::charge_placed` are latches carrying the LAST of their kind, so this
/// resource — filled off `Feed`'s freshness bits — is what turns "latest"
/// into "live": the wall refreshes on every hit and fades like a toast,
/// the charge counts down from the fuse the wire named and clears at zero.
#[derive(Resource, Default)]
pub struct Readout {
    /// The wall's cell, hp pair, and seconds left on the surface.
    wall: Option<(u16, u16)>,
    wall_left: u16,
    wall_max: u16,
    wall_secs: f32,
    /// The charge's cell and seconds to detonation. Cleared at zero — the
    /// blast's own `EV_STRUCT_HIT` takes the surface back over.
    charge: Option<(u16, u16)>,
    charge_secs: f32,
}

/// The crosshair's four ticks. A frame with no crosshair reads as a
/// flythrough for the same reason `ART.md` §8 says one with no viewmodel
/// does, and it is the only thing on screen that says where a swing goes.
#[derive(Component)]
pub struct Crosshair;

/// The hitmarker: the crosshair's ticks, recoloured for a quarter second
/// when a swing lands.
#[derive(Component)]
pub struct HitMark;

/// The compass strip. Bearing only — the reference also pins markers to it
/// (death skull, map pin) and ours carries none, because `ALPHA.md` §1 has a
/// rule about position an operator should read before we pin anything.
#[derive(Component)]
pub struct Compass;

/// The code lock's keypad, drawn — the root of [`pad_overlay`]'s small
/// panel. Spawned and despawned by the system itself, so it lives here in
/// the HUD (over the world, pointer-free) and not in `panels::` (which
/// would grab the pointer, the one thing a keypad must not do —
/// `crate::ui::keypad` has the argument).
#[derive(Component)]
pub struct PadRoot;

pub fn setup(mut commands: Commands) {
    // The hotbar: six cells, bottom centre.
    commands
        .spawn((
            super::WorldEntity,
            Node {
                position_type: PositionType::Absolute,
                bottom: Val::Px(HOTBAR_BOTTOM_PX),
                width: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                column_gap: Val::Px(6.0),
                ..default()
            },
            Pickable::IGNORE,
        ))
        .with_children(|row| {
            for i in 0..HOTBAR_SLOTS {
                row.spawn((
                    Cell(i),
                    Node {
                        width: Val::Px(HOTBAR_CELL_PX),
                        height: Val::Px(HOTBAR_CELL_PX),
                        border: UiRect::all(Val::Px(2.0)),
                        // The count sits bottom-right over the icon, which is
                        // the arrangement every inventory in the genre uses
                        // and the the one the reference `inventory` shows.
                        justify_content: JustifyContent::FlexEnd,
                        align_items: AlignItems::FlexEnd,
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.05, 0.05, 0.06, 0.55)),
                    BorderColor::all(Color::srgba(0.75, 0.72, 0.62, 0.35)),
                ))
                .with_children(|cell| {
                    // The icon fills the cell and is spawned EMPTY: a handle
                    // is set by `fill_cells` when the slot has something in
                    // it. Absolute so the count can overlap it rather than
                    // being laid out beside it.
                    cell.spawn((
                        CellIcon(i),
                        Node {
                            position_type: PositionType::Absolute,
                            left: Val::Px(3.0),
                            top: Val::Px(3.0),
                            width: Val::Px(36.0),
                            height: Val::Px(36.0),
                            ..default()
                        },
                        ImageNode {
                            image: Handle::default(),
                            // The icons are white-on-transparent silhouettes
                            // (`icons.rs`), so the tint IS the colour and a
                            // slightly warm off-white keeps them from
                            // vibrating against the pale border.
                            color: Color::srgba(0.90, 0.88, 0.82, 0.95),
                            ..default()
                        },
                        Visibility::Hidden,
                        Pickable::IGNORE,
                    ));
                    cell.spawn((
                        CellCount(i),
                        Text::new(""),
                        super::ui::font_bold(12.0),
                        TextColor(Color::srgba(0.97, 0.95, 0.88, 0.95)),
                        Node {
                            position_type: PositionType::Absolute,
                            right: Val::Px(3.0),
                            // Lifted off the edge by the pip's own height, so
                            // the digits sit ON the cell rather than over the
                            // durability bar. The count moved; the bar is the
                            // thing anchored to the edge, because a bar that
                            // floats is a bar that reads as a progress
                            // indicator for the cell above it.
                            bottom: Val::Px(super::panels::PIP_H_PX + 1.0),
                            ..default()
                        },
                        Pickable::IGNORE,
                    ));
                    // The durability pip: spawned hidden, shown by `update`
                    // for a worn tool only. Same three states as the
                    // inventory panel's, drawn by the same numbers — the
                    // hotbar and the bag must never disagree about how worn
                    // the thing in your hand is.
                    cell.spawn((
                        CellPip(i),
                        Node {
                            position_type: PositionType::Absolute,
                            left: Val::Px(0.0),
                            right: Val::Px(0.0),
                            bottom: Val::Px(0.0),
                            height: Val::Px(super::panels::PIP_H_PX),
                            ..default()
                        },
                        BackgroundColor(super::panels::PIP_TROUGH),
                        Visibility::Hidden,
                        Pickable::IGNORE,
                    ))
                    .with_children(|trough| {
                        trough.spawn((
                            CellPipFill(i),
                            Node {
                                width: Val::Percent(0.0),
                                height: Val::Percent(100.0),
                                ..default()
                            },
                            BackgroundColor(super::panels::PIP_FILL),
                            Pickable::IGNORE,
                        ));
                    });
                });
            }
        });

    // The vitals stack: right side, small, never centred — and **bars, not
    // text**. the reference `crafting.png` draws three filled bars with an icon
    // square each, bottom right; ours read `HP 100/100` in a column, which is
    // a debug readout wearing a HUD's position. A bar is also the only one of
    // the two that answers the question a player actually asks mid-fight,
    // which is "how much is left" and not "what integer is it".
    commands
        .spawn((
            super::WorldEntity,
            Node {
                position_type: PositionType::Absolute,
                bottom: Val::Px(18.0),
                right: Val::Px(20.0),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(3.0),
                ..default()
            },
            Pickable::IGNORE,
        ))
        .with_children(|stack| {
            for v in [Vital::Hp, Vital::Water, Vital::Food] {
                stack
                    .spawn((
                        VitalRow(v),
                        Node {
                            flex_direction: FlexDirection::Row,
                            column_gap: Val::Px(3.0),
                            align_items: AlignItems::Center,
                            ..default()
                        },
                    ))
                    .with_children(|row| {
                        // The icon square, left of the bar. A glyph today; a
                        // drawn icon when there are icons.
                        row.spawn((
                            Node {
                                width: Val::Px(VITAL_BAR_H),
                                height: Val::Px(VITAL_BAR_H),
                                justify_content: JustifyContent::Center,
                                align_items: AlignItems::Center,
                                ..default()
                            },
                            BackgroundColor(super::panels::VITAL_TROUGH),
                            children![(
                                Text::new(v.glyph().to_string()),
                                super::ui::font_bold(13.0),
                                TextColor(v.fill()),
                            )],
                        ));
                        // The bar: a trough, the fill over it, and the number
                        // over both. Only the fill's width animates.
                        row.spawn((
                            Node {
                                width: Val::Px(VITAL_BAR_W),
                                height: Val::Px(VITAL_BAR_H),
                                ..default()
                            },
                            BackgroundColor(super::panels::VITAL_TROUGH),
                            children![
                                (
                                    VitalFill(v),
                                    Node {
                                        position_type: PositionType::Absolute,
                                        left: Val::Px(0.0),
                                        top: Val::Px(0.0),
                                        width: Val::Percent(0.0),
                                        height: Val::Percent(100.0),
                                        ..default()
                                    },
                                    BackgroundColor(v.fill()),
                                ),
                                (
                                    VitalNum(v),
                                    Node {
                                        position_type: PositionType::Absolute,
                                        left: Val::Px(6.0),
                                        top: Val::Px(1.0),
                                        ..default()
                                    },
                                    Text::new(""),
                                    super::ui::font_bold(13.0),
                                    TextColor(Color::srgb(0.98, 0.97, 0.95)),
                                ),
                            ],
                        ));
                    });
            }
        });

    // The build plan, bottom left. Only ever text: the piece it names is a
    // client-side latch, not sim state.
    //
    // `WorldEntity` like its two neighbours: leaving a shard is a state change
    // now, and a HUD line that outlived the world would be drawn over the
    // server list.
    commands.spawn((
        super::WorldEntity,
        Plan,
        Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(20.0),
            left: Val::Px(22.0),
            ..default()
        },
        Text::new(""),
        super::ui::font_bold(14.0),
        TextColor(Color::srgba(0.86, 0.83, 0.76, 0.80)),
        Pickable::IGNORE,
    ));

    // The build stamp, top left — the one free corner (the compass owns top
    // centre, the hotbar bottom centre, the vitals bottom right, the plan
    // bottom left).
    //
    // **Static text, spawned once and never updated**, because the value is a
    // compile-time constant: there is no system for it and nothing per-frame
    // to pay. Dim and 11 px on purpose — its job is to be readable in a
    // screenshot somebody pastes into a bug report, not to be part of the
    // frame a player is looking at. `protocol::version::BUILD_ID` is the same
    // string the shard's boot line prints and the same one `ci/depot.py`
    // names an install directory with, so a report, a depot receipt and a
    // server log all say one thing.
    commands.spawn((
        super::WorldEntity,
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(10.0),
            left: Val::Px(12.0),
            ..default()
        },
        Text::new(protocol::version::BUILD_ID),
        super::ui::font(11.0),
        TextColor(Color::srgba(0.86, 0.83, 0.76, 0.45)),
        Pickable::IGNORE,
    ));

    // The crosshair: four ticks around a gap, never a dot. A dot vanishes
    // against light ground and a full cross hides the thing you are aiming
    // at; the reference uses ticks for both reasons.
    commands
        .spawn((
            super::WorldEntity,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Percent(CROSSHAIR_CENTRE_PCT),
                top: Val::Percent(CROSSHAIR_CENTRE_PCT),
                width: Val::Px(0.0),
                height: Val::Px(0.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            Pickable::IGNORE,
        ))
        .with_children(|c| {
            for (dx, dy, w, h) in CROSSHAIR_TICKS {
                c.spawn((
                    Crosshair,
                    HitMark,
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(dx - w * 0.5),
                        top: Val::Px(dy - h * 0.5),
                        width: Val::Px(w),
                        height: Val::Px(h),
                        ..default()
                    },
                    BackgroundColor(CROSSHAIR),
                    Pickable::IGNORE,
                ));
            }
        });

    // The centre prompt and the toast beneath it. Both sit BELOW the
    // crosshair rather than on it: text over the aim point is text in the way
    // of the thing it is describing.
    commands.spawn((
        super::WorldEntity,
        PromptLine,
        Node {
            position_type: PositionType::Absolute,
            top: Val::Percent(PROMPT_TOP_PCT),
            width: Val::Percent(100.0),
            justify_content: JustifyContent::Center,
            ..default()
        },
        Text::new(""),
        super::ui::font_bold(PROMPT_FONT_PX),
        TextColor(Color::srgba(0.96, 0.94, 0.88, 0.92)),
        TextLayout::new_with_justify(Justify::Center),
        Pickable::IGNORE,
    ));
    // The announce stack. Absolutely positioned per row rather than a flex
    // column: every row then has a fixed home whether or not the ones above
    // it are live, so a line arriving does not shove the readout — a number
    // being watched must not move under the eye watching it.
    for row in 0..TOAST_LINES {
        commands.spawn((
            super::WorldEntity,
            ToastLine(row),
            Node {
                position_type: PositionType::Absolute,
                top: Val::Percent(toast_row_top_pct(row)),
                width: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                ..default()
            },
            Text::new(""),
            super::ui::font(TOAST_FONT_PX),
            TextColor(Color::srgba(0.98, 0.82, 0.55, 0.0)),
            TextLayout::new_with_justify(Justify::Center),
            Pickable::IGNORE,
        ));
    }

    // The pinned readout, under the whole stack (cosmetics as the toast's:
    // same family, one step down the screen). Bold where the toast is not,
    // because it is a number being watched rather than a sentence being
    // noticed.
    commands.spawn((
        super::WorldEntity,
        ReadoutLine,
        Node {
            position_type: PositionType::Absolute,
            top: Val::Percent(readout_top_pct()),
            width: Val::Percent(100.0),
            justify_content: JustifyContent::Center,
            ..default()
        },
        Text::new(""),
        super::ui::font_bold(14.0),
        TextColor(Color::srgba(0.98, 0.82, 0.55, 0.0)),
        TextLayout::new_with_justify(Justify::Center),
        Pickable::IGNORE,
    ));

    // The compass strip, top centre: a 90° window on the bearing, letters at
    // the cardinals. `hud.js`'s shape, in text rather than in a canvas.
    commands.spawn((
        super::WorldEntity,
        Compass,
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(14.0),
            width: Val::Percent(100.0),
            justify_content: JustifyContent::Center,
            ..default()
        },
        Text::new(""),
        super::ui::font_bold(15.0),
        TextColor(Color::srgba(0.90, 0.88, 0.82, 0.75)),
        TextLayout::new_with_justify(Justify::Center),
        Pickable::IGNORE,
    ));

    // The viewmodel used to be spawned here, as two untextured `Cuboid`s
    // welded rigidly to the camera. It is `render::viewmodel` now — a textured
    // tool plus bob, sway and swing — because it stopped being a static prop
    // and became a thing with state, and `hud.rs` is the HUD.
}

/// Redraw from the core. Cheap enough per frame: six background colours and
/// one string.
#[allow(clippy::type_complexity)]
// Eight, and the eighth arrived with the aiming refusal: the vitals stack grew
// three queries of its own on `main` in the same window. Each is a distinct
// source this frame reads, which is the same justification `ghost::track`
// carries for its own count.
#[allow(clippy::too_many_arguments)]
pub fn update(
    net: NonSend<Net>,
    mut cells: Query<(&Cell, &mut BorderColor, &mut BackgroundColor)>,
    mut icons: Query<(&CellIcon, &mut ImageNode, &mut Visibility)>,
    mut counts: Query<(&CellCount, &mut Text), (Without<Plan>, Without<VitalNum>)>,
    mut pips: Query<(&CellPip, &mut Visibility), Without<CellIcon>>,
    mut pip_fills: Query<
        (&CellPipFill, &mut Node),
        (Without<VitalFill>, Without<VitalRow>, Without<Cell>),
    >,
    art: Res<super::icons::Icons>,
    mut plan: Query<&mut Text, (With<Plan>, Without<VitalNum>)>,
    mut rows: Query<(&VitalRow, &mut Node), (Without<VitalFill>, Without<Cell>)>,
    mut fills: Query<(&VitalFill, &mut Node), Without<Cell>>,
    mut nums: Query<(&VitalNum, &mut Text), Without<Plan>>,
    // `Option`, because a capture run does not register the menus at all.
    ui: Option<Res<super::panels::Ui>>,
    ghost: Option<Res<super::ghost::Ghost>>,
) {
    let core = &net.session.core;

    if let (Ok(mut text), Some(ui)) = (plan.single_mut(), ui.as_ref()) {
        use crate::ui::build::{material_label, row_for, shape_label, PLACE_MATERIAL, SHAPES};
        let shape = SHAPES[ui.shape.min(SHAPES.len() - 1)];
        let material = PLACE_MATERIAL;
        // Named only when the content actually has that piece — the wheel
        // draws a dead segment for a pair the shard did not bake, and the
        // HUD must not contradict it.
        // **The refusal is shown while AIMING, not after the press.** The
        // ghost has always known why it is red — `place::verdict` returns the
        // same sentence the server's refusal would carry — and until now that
        // string was only spoken by `ghost::place_key`, i.e. after the player
        // had already spent the click. A red box with no reason teaches
        // nothing; "TOO FAR" or "NO ROOM" beside it teaches the rule. Level is
        // here for the same reason: `R`/`F` step it and nothing showed it, so
        // a player building on level 2 by accident had no way to notice.
        // A held deployable claims the line ahead of the build latch: the
        // hand decides what the mouse means (`ui::hold`), and its ghost has
        // a verdict to say while AIMING — the same grammar the build ghost
        // uses below, red reason and all, so a preview that has gone red
        // names the sim's own sentence before the click spends the item.
        let held = core.inv[(net.sel as usize).min(core.inv.len().saturating_sub(1))];
        let deploy_held =
            crate::ui::structure::row_for_item(&core.deploy_defs, core.deploy_defs_have, held.item)
                .is_some();
        let out = if deploy_held {
            let name = core
                .catalog
                .name(held.item as usize)
                .iter()
                .map(|b| (*b as char).to_ascii_uppercase())
                .collect::<String>();
            let why = match ghost.as_ref().map(|g| g.deploy_verdict) {
                Some(crate::ui::place::DeployVerdict::No(w)) if !w.is_empty() => w,
                _ => "",
            };
            if why.is_empty() {
                format!("PLACE  {name}   (right click)")
            } else {
                format!("PLACE  {name}  — {}", why.to_uppercase())
            }
        } else {
            match row_for(&core.piece_defs, shape, material) {
                Some(_) => {
                    let why = match ghost.as_ref().map(|g| g.verdict) {
                        Some(crate::ui::place::Verdict::No(w)) if !w.is_empty() => w,
                        _ => "",
                    };
                    let level = ghost.as_ref().map(|g| g.level).unwrap_or(0);
                    if why.is_empty() {
                        format!(
                            "BUILD  {} {}  L{}   (hold right)",
                            material_label(material),
                            shape_label(shape),
                            level
                        )
                    } else {
                        format!(
                            "BUILD  {} {}  L{}  — {}",
                            material_label(material),
                            shape_label(shape),
                            level,
                            why.to_uppercase()
                        )
                    }
                }
                None => "BUILD  -   (hold right)".to_string(),
            }
        };
        if text.0 != out {
            text.0 = out;
        }
    }

    // The cells' contents. Separate loops from the selection highlight below
    // because they answer different questions and touch different components
    // — the highlight is `net.sel`, this is the inventory mirror.
    for (icon, mut img, mut vis) in icons.iter_mut() {
        let stack = core.inv.get(icon.0).copied().unwrap_or_default();
        match crate::ui::icons::icon_stem(&core.catalog, stack).and_then(|s| art.verb(s)) {
            Some(h) => {
                if img.image != h {
                    img.image = h;
                }
                *vis = Visibility::Inherited;
            }
            // Hidden rather than cleared: an `ImageNode` with a default handle
            // draws Bevy's white placeholder square, which is a filled cell
            // claiming to hold something.
            None => *vis = Visibility::Hidden,
        }
    }
    for (slot, mut text) in counts.iter_mut() {
        let stack = core.inv.get(slot.0).copied().unwrap_or_default();
        // A count of 1 is not drawn: every cell would carry a "1" and the
        // digit stops meaning "how many" and starts being decoration.
        let want = if stack.count > 1 {
            let mut s = String::new();
            let _ = std::fmt::Write::write_fmt(&mut s, format_args!("{}", stack.count));
            s
        } else {
            String::new()
        };
        if text.0 != want {
            text.0 = want;
        }
    }

    // The durability pip. Two loops for one bar because the trough carries the
    // visibility and the fill carries the width, and Bevy will not hand one
    // system `&mut` on the same component twice.
    //
    // `pip_fraction` is the whole rule and it is not restated here: the
    // hotbar's job is to hand it the same two numbers the inventory panel
    // does, so the bar under the thing in your hand and the bar under the same
    // stack in the bag are one decision drawn twice.
    for (pip, mut vis) in pips.iter_mut() {
        let stack = core.inv.get(pip.0).copied().unwrap_or_default();
        let worn = stack.count > 0
            && crate::ui::slots::pip_fraction(
                stack.cond,
                core.catalog.cond_max(stack.item as usize),
            )
            .is_some();
        let want = if worn {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *vis != want {
            *vis = want;
        }
    }
    for (fill, mut node) in pip_fills.iter_mut() {
        let stack = core.inv.get(fill.0).copied().unwrap_or_default();
        let frac =
            crate::ui::slots::pip_fraction(stack.cond, core.catalog.cond_max(stack.item as usize))
                .unwrap_or(0.0);
        let want = Val::Percent(frac * 100.0);
        if node.width != want {
            node.width = want;
        }
    }

    for (cell, mut border, mut bg) in cells.iter_mut() {
        let selected = cell.0 == net.sel as usize;
        *border = BorderColor::all(if selected {
            Color::srgba(0.98, 0.86, 0.55, 0.95)
        } else {
            Color::srgba(0.75, 0.72, 0.62, 0.35)
        });
        *bg = BackgroundColor(if selected {
            Color::srgba(0.14, 0.13, 0.10, 0.72)
        } else {
            Color::srgba(0.05, 0.05, 0.06, 0.55)
        });
    }

    // The vitals: a row is hidden outright when its maximum is 0, the fill is
    // the fraction, and the number is the value. Same rule the string version
    // held — draw nothing rather than an empty bar for a player who cannot be
    // hurt — expressed as a `Display` instead of an absent line.
    for (row, mut node) in rows.iter_mut() {
        let (_, max) = row.0.read(core);
        let want = if max > 0 {
            Display::Flex
        } else {
            Display::None
        };
        if node.display != want {
            node.display = want;
        }
    }
    for (fill, mut node) in fills.iter_mut() {
        let (value, max) = fill.0.read(core);
        // Clamped, because the server is authoritative and a vital over its
        // stated maximum is a bar that would run off its trough rather than a
        // reason to disbelieve the number beside it.
        let pct = if max > 0 {
            (value as f32 / max as f32).clamp(0.0, 1.0) * 100.0
        } else {
            0.0
        };
        let want = Val::Percent(pct);
        if node.width != want {
            node.width = want;
        }
    }
    for (num, mut text) in nums.iter_mut() {
        let (value, _) = num.0.read(core);
        let out = value.to_string();
        if text.0 != out {
            text.0 = out;
        }
    }
}

/// Drain the core's feedback rings into the toast, and count the timers down.
///
/// **Every one of these had zero readers before this slice.** The sim has
/// announced hits, refusals and gather toasts since M1; `ClientCore` decoded
/// them into bounded rings; the native client popped none of them, so a
/// refused craft, a refused placement and a landed hit were all silence.
///
/// A ring is drained to EMPTY each frame rather than one entry per frame:
/// they are small (`TOAST_RING`) and a backlog drip-fed at frame rate would
/// still be showing the first refusal after the tenth.
pub fn feedback(
    net: NonSend<Net>,
    feed: Res<super::feed::Feed>,
    mut toast: ResMut<Toast>,
    time: Res<Time>,
    mut marks: Query<&mut BackgroundColor, With<HitMark>>,
    mut lines: Query<(&ToastLine, &mut Text, &mut TextColor)>,
) {
    let core = &net.session.core;

    // **Read, never popped.** `render::feed::drain` is the one caller of
    // `pop_*` in the client; this used to pop the rings itself, and the audio
    // systems popped the same ones — two correct halves that merged cleanly
    // and left the game silent. `feed.rs`'s header has the whole story.

    // Hits first: the marker is the only feedback with a deadline on it.
    if feed.hits > 0 {
        toast.hit(feed.damage);
    }

    // Refusals. Each store answers a different verb, and the reason codes are
    // integers by wall 3 — turning one into a sentence is the client's job
    // and `refusal_text` is where the whole mapping lives.
    for (which, code, item) in feed.refusals() {
        toast.warn(match which {
            super::feed::Refused::Craft => crate::ui::refusals::craft(code),
            super::feed::Refused::Build => crate::ui::refusals::build(code),
            super::feed::Refused::Deploy => crate::ui::refusals::deploy(code),
            super::feed::Refused::Research => crate::ui::refusals::research(code),
            super::feed::Refused::Consume => crate::ui::refusals::consume(code),
            // The one refusal whose sentence names the held item: "your
            // Torch cannot harvest this", or bare hands when nothing was.
            super::feed::Refused::Gather => {
                let held = if item == sim_core::gather::NO_ITEM {
                    "bare hands".to_string()
                } else {
                    format!("your {}", crate::ui::craft::item_label(&core.catalog, item))
                };
                crate::ui::refusals::gather(code, &held)
            }
        });
    }

    // The consume verbs' LANDED half (`NOW.md` §0eat). The refused half is in
    // the loop above and rides the queue, so the mixer's refusal cue answers
    // a dry shoreline for free. A ring since 2026-08-15: this read the
    // `last_eat` latch off `Feed::applied`, and two answers in one drain
    // window collapsed — a landed eat vanished under a refusal that arrived
    // in the same frame, which `KeyJ` + `KeyH` in one frame produces.
    //
    // Why say anything: a bandage that worked and a bandage that was refused
    // looked identical from the chair. The only evidence of the first was the
    // hp bar creeping for four seconds and the only evidence of the second was
    // nothing at all.
    for &(item, _slot) in feed.consumed() {
        // The slot was the sender's own claim and tells a player nothing
        // they did not just click, so only the item is named.
        let label = crate::ui::craft::item_label(&core.catalog, item);
        // "used", not "ate": the same verb spends a bandage, and a bandage is
        // what the session that found this was holding.
        //
        // `say`, not `warn`: by `Rank`'s own rule this is recoverable by
        // looking — the item left the pack and the meters moved — which puts
        // it beside the gather and craft lines rather than beside a refusal.
        toast.say(format!("used {label}"));
    }
    if feed.applied & client_core::core::APPLIED_DRANK != 0 {
        // `water restored << 16 | hp it cost`. The cost is the sea being salt
        // (`survival::drink`), and naming it is the difference between a
        // mechanic and a bug report — a body that drank its last points of
        // health away otherwise just dies. Nought is the fresh-water case and
        // says nothing about hp rather than saying `-0`.
        let water = core.last_drink >> 16;
        let cost = core.last_drink & 0xffff;
        toast.say(if cost > 0 {
            format!("drank: +{water} water, -{cost} hp")
        } else {
            format!("drank: +{water} water")
        });
    }
    // Knocks and grants (lock v1). A knock is broadcast, so this fires
    // for a door across the base as readily as the one in front of you —
    // that is the feature, not a leak: the news a defender needs is
    // exactly "somebody is at a door", and which door is on the event.
    for _ in feed.knocks() {
        toast.say("*knock knock*".to_string());
    }
    for (.., grant) in feed.auths() {
        toast.say(
            if *grant == sim_core::lock::GRANT_FULL {
                "the lock remembers you"
            } else {
                "the lock remembers you as a guest"
            }
            .to_string(),
        );
    }
    // What the hearth is holding, after you feed it. `stock` is a latched
    // ROW TABLE rather than one value, so the same freshness rule applies —
    // and `stock_count` is how many of the rows are live, which is why the
    // slice is taken rather than the array walked whole.
    if feed.applied & client_core::core::APPLIED_STOCK != 0 {
        let rows = &core.stock[..(core.stock_count as usize).min(core.stock.len())];
        if let Some(line) = stock_line(rows, &core.catalog) {
            toast.say(line);
        }
    }

    // A charge going live, anywhere you can see. This is BROADCAST rather
    // than own-fact, and `APPLIED2_CHARGE`'s own doc says why that is the
    // point: "the charge you most need drawn is the one you did not plant."
    // Word 1, not word 0 — word 0 is full.
    if feed.applied2 & client_core::core::APPLIED2_CHARGE != 0 {
        let (_, _, _, _, _, fuse) = core.charge_placed;
        if let Some(line) = charge_line(fuse) {
            toast.warn(line);
        }
    }

    // The wall being broken used to toast here and moved to [`readout`]:
    // a toast is replaced by whatever speaks next, so mid-raid the one
    // number a raider was watching vanished under every gather — and the
    // toast never said WHERE, which is the half `struct_hit` carries that
    // nothing drew. The charge's announce stays: it is a one-shot warning,
    // which is what a toast is; the readout is its clock.

    // The kill feed. Every death on the shard reaches this ring, and until
    // now the only reader was the mixer (`render::audio`) — so a death was
    // AUDIBLE and invisible, which is the half of `NOW.md` §0x item 6 that
    // cost nothing to close because the fact was already drained and sitting
    // in `Feed`.
    //
    // **Our own death is skipped, and it is in this ring.** `ClientCore`
    // buffers `(victim, killer)` for every `EV_DEATH` unconditionally and
    // only *then* asks whether the victim was us, so the death SCREEN
    // (`core.dead` + the `own_death_*` fields, `ui::death`) and this line
    // are fed by the same event. Without the skip a player would be told
    // they died twice, once in a sentence written for someone watching.
    //
    // No cause here: the ring carries `(victim, killer)` and nothing else,
    // so the feed says who, and the screen — which does have the cause, the
    // weapon and the range — says how.
    for &(victim, killer) in feed.deaths() {
        if let Some(line) = kill_line_for(victim, killer, core.player_id) {
            toast.say(line);
        }
    }

    // Gather and craft toasts are (item, count) pairs — something arrived.
    // `item_label` is the panels' own naming, reused rather than restated:
    // an index no name has dripped for prints as `#12`, which is honest,
    // where an empty cell is the dark-panel defect this repo has a rule
    // against.
    for &(item, count) in feed.gathered() {
        let label = crate::ui::craft::item_label(&core.catalog, item);
        toast.say(format!("+{count} × {label}"));
    }
    for &(item, count) in feed.crafted() {
        let label = crate::ui::craft::item_label(&core.catalog, item);
        toast.say(format!("crafted {count} × {label}"));
    }
    // Last, so it is the top row: `Toast` is a queue now and nothing is
    // discarded, but the newest line is the brightest and the one the eye
    // starts at, and of everything that can land in one frame this is the
    // one the player most needs. Gathering into a full pack used to print
    // nothing at all, so the only signal was a bag appearing underfoot; then
    // it printed one spill and swallowed the other, which is worse, because
    // a wrong sentence is read as the whole truth.
    //
    // `warn`, not `say`: the bag underfoot is the only other record of this,
    // so the line is the fact. The write order above no longer decides that
    // — `Rank` does — and this loop stays last for where it draws, not for
    // what survives.
    //
    // No amount, because the wire has none to give — see `Feed::spills`.
    // "at your feet" is where `world.rs`'s `drain_spill` stands the bag up;
    // a merge into a bag already standing puts it within the same reach.
    for &item in feed.spills() {
        let label = crate::ui::craft::item_label(&core.catalog, item);
        toast.warn(format!("pack full — {label} dropped at your feet"));
    }

    // ---- the timers -----------------------------------------------------
    // Every line runs its own clock and the expired ones are retired here,
    // AFTER this frame's says — a line that arrived and expired in the same
    // frame is not a thing, and a `dt` applied before the push would make
    // one.
    toast.tick(time.delta_secs());

    let hot = toast.hit_left > 0.0;
    for mut bg in marks.iter_mut() {
        let want = if hot { CROSSHAIR_HIT } else { CROSSHAIR };
        if bg.0 != want {
            bg.0 = want;
        }
    }

    for (row, mut text, mut color) in lines.iter_mut() {
        // Fade over the last second rather than vanishing, so a line that
        // ran out reads as spent and not as a flicker. Each row dims a step
        // further than the one above so the stack reads newest-first, and an
        // empty row is blanked rather than left holding stale news.
        let (want, extra, alpha) = match toast.row_parts(row.0) {
            Some((say, extra)) => (
                say.text(),
                extra,
                say.left().min(1.0) * toast_row_dim(row.0),
            ),
            None => ("", "", 0.0),
        };
        // Compared against the two halves rather than a composed `String`:
        // this runs every frame for every row and the equal case is the
        // common one, so building the answer to ask the question would be a
        // per-frame allocation on the client's hot path.
        let same = text.0.len() == want.len() + extra.len()
            && text.0.starts_with(want)
            && text.0.ends_with(extra);
        if !same {
            text.0.clear();
            text.0.push_str(want);
            text.0.push_str(extra);
        }
        color.0 = color.0.with_alpha(alpha);
    }
}

/// The pinned readout: the WHERE of the two latched signals (`NOW.md` §0x
/// item 6). Until this landed, `struct_hit` and `charge_placed` reached
/// the toast, which said WHAT honestly and dropped the address half on
/// the floor. This line persists instead: the wall's fraction refreshes
/// on every hit with the bearing and distance of the wall it names, and
/// the charge is a ticking CLOCK counted down client-side from the fuse
/// the wire carried — with the bearing a defender actually runs on, since
/// the charge most worth drawing is the one somebody else planted.
///
/// A second reader of `Feed`, which is the architecture doing its job
/// (`feed.rs`: a reader is a `Res<_>` and cannot consume anything — no
/// `pop_*` here, `tests/sound.rs` greps). The world-space anchors — a
/// number at the wall itself, a clock on the charge mesh — are still not
/// built; this is the HUD half, and `charge_deploy` stays unread until
/// the mesh half wants it.
pub fn readout(
    net: NonSend<Net>,
    feed: Res<super::feed::Feed>,
    time: Res<Time>,
    mut ro: ResMut<Readout>,
    mut lines: Query<(&mut Text, &mut TextColor), With<ReadoutLine>>,
) {
    let core = &net.session.core;
    // Latch on the freshness bits, never on the fields being non-zero —
    // they hold the LAST of their kind forever (`Feed::applied`'s doc).
    if feed.applied & client_core::core::APPLIED_STRUCT_HIT != 0 {
        let (cx, cz, _, _, left, max) = core.struct_hit;
        // `max == 0` is the defs-not-arrived state and pins nothing: the
        // same honesty rule the toast held (`struct_hit_line`'s doc).
        if max > 0 {
            ro.wall = Some((cx, cz));
            ro.wall_left = left;
            ro.wall_max = max;
            ro.wall_secs = TOAST_SECS;
        }
    }
    if feed.applied2 & client_core::core::APPLIED2_CHARGE != 0 {
        let (cx, cz, _, _, _, fuse) = core.charge_placed;
        if fuse > 0 {
            ro.charge = Some((cx, cz));
            ro.charge_secs = fuse as f32 / sim_core::limits::TICK_HZ as f32;
        }
    }

    // The clocks. Cosmetic fades and a countdown mirror, not a gate — the
    // detonation itself is the sim's, and arrives as its own events.
    let dt = time.delta_secs();
    ro.wall_secs = (ro.wall_secs - dt).max(0.0);
    if ro.wall_secs == 0.0 {
        ro.wall = None;
    }
    ro.charge_secs = (ro.charge_secs - dt).max(0.0);
    if ro.charge_secs == 0.0 {
        ro.charge = None;
    }

    let Ok((mut text, mut color)) = lines.single_mut() else {
        return;
    };
    let [px, _, pz] = core.predict.render_position();
    // The clock outranks the wall on the one line: it is rarer, it is
    // fatal, and it is bounded by its own fuse — the wall's timer keeps
    // running underneath and the fraction is back when the charge clears.
    let (want, alpha) = if let Some((cx, cz)) = ro.charge {
        (
            charge_readout(ro.charge_secs, &whereabouts(px, pz, cx, cz)),
            1.0,
        )
    } else if let Some((cx, cz)) = ro.wall {
        (
            struct_readout(ro.wall_left, ro.wall_max, &whereabouts(px, pz, cx, cz)),
            // The toast's own fade: full until the last second, then out.
            ro.wall_secs.min(1.0),
        )
    } else {
        (None, 0.0)
    };
    let want = want.unwrap_or_default();
    if text.0 != want {
        text.0 = want;
    }
    color.0 = color.0.with_alpha(alpha);
}

/// The centre prompt and the compass.
// Eight sources and each is a distinct input: the two picks, the swing,
// the near structure, the look, the pad, and the two text nodes.
#[allow(clippy::too_many_arguments)]
pub fn prompt(
    aimed: Res<Aimed>,
    swung: Res<super::verbs::Swung>,
    in_weak: Res<super::verbs::InWeak>,
    near: Res<super::verbs::Near>,
    look: Res<super::input::Look>,
    pad: Res<super::verbs::Pad>,
    mut prompts: Query<&mut Text, (With<PromptLine>, Without<Compass>)>,
    mut compass: Query<&mut Text, (With<Compass>, Without<PromptLine>)>,
) {
    if let Ok(mut text) = prompts.single_mut() {
        // `E` outranks the swing, and the swing is drawn only where `E` is
        // silent. That ordering is the browser's and it is not arbitrary: a
        // box standing against a tree is both openable and choppable, and a
        // player who pressed `E` on a prompt that named a swing would spend
        // the wrong verb. One prompt, and the key it names is the key that
        // acts on it.
        // The keypad outranks both, and for the ordering's own reason
        // turned up a level: while four digits are being typed, `E` and
        // the swing are not what the keys mean, so a prompt naming them
        // would name the wrong verb (lock v1, `crate::ui::keypad`). The
        // pad itself is [`pad_overlay`]'s panel now, so this line goes
        // quiet rather than saying the same thing twice.
        // The side line is last: it names no key, so anything that does
        // outranks it.
        let want = if pad.0.is_open() {
            String::new()
        } else {
            match aimed.0.prompt() {
                s if !s.is_empty() => s,
                _ => match swing_prompt_weak(swung.0.occupant, in_weak.0) {
                    s if !s.is_empty() => s,
                    _ => side_line(&near.0),
                },
            }
        };
        if text.0 != want {
            text.0 = want;
        }
    }
    if let Ok(mut text) = compass.single_mut() {
        let want = compass_strip(look.yaw);
        if text.0 != want {
            text.0 = want;
        }
    }
}

/// The keypad, as a small panel in the house grammar (`NOW.md` §0z item 3).
///
/// **Pointer-free is the bar**: this is presentation, not a mouse flow.
/// The pad stays a resource beside [`super::verbs::Pad`] and its keys stay
/// `verbs::keypad_keys`'s; nothing here is clickable, the cursor stays
/// locked, and the world keeps rendering behind it. What changed is only
/// that four digits and seven ops are a panel above the hotbar instead of
/// one line through the middle of the door you are standing at.
///
/// Everything drawn is [`crate::ui::keypad`]'s own arithmetic —
/// [`Keypad::cells`] for the digit boxes, `OPS_HINT` for the key line, the
/// buffer's `len` for the cursor — so the overlay says exactly what the
/// old HUD line said ([`Keypad::line`] composes the same pieces, and the
/// unit tests hold them together).
///
/// Rebuilt only when the pad changes; a `Keypad` is sixteen bytes and the
/// compare is the cost of a still frame.
///
/// [`Keypad::cells`]: crate::ui::keypad::Keypad::cells
/// [`Keypad::line`]: crate::ui::keypad::Keypad::line
pub fn pad_overlay(
    mut commands: Commands,
    pad: Res<super::verbs::Pad>,
    roots: Query<Entity, With<PadRoot>>,
    mut seen: Local<Option<crate::ui::keypad::Keypad>>,
) {
    if *seen == Some(pad.0) {
        return;
    }
    *seen = Some(pad.0);
    for e in roots.iter() {
        commands.entity(e).despawn();
    }
    if !pad.0.is_open() {
        return;
    }

    // Cosmetics ride the toast's row (`DECISIONS.md` §open, client
    // cosmetics); the palette is `panels`'s, so a keypad and an inventory
    // read as one product. Bottom-centred at the wheel hint's own hotbar
    // clearance — the centre of the screen stays on the door.
    use super::panels::{CELL_BG, CELL_FULL, LINE, LINE_HOT, PANEL_BG, TEXT_DIM};
    let cells = pad.0.cells();
    let cursor = pad.0.len();
    commands
        .spawn((
            super::WorldEntity,
            PadRoot,
            Node {
                position_type: PositionType::Absolute,
                bottom: Val::Px(76.0),
                width: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                ..default()
            },
            Pickable::IGNORE,
        ))
        .with_children(|row| {
            row.spawn((
                Node {
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    padding: UiRect::all(Val::Px(10.0)),
                    row_gap: Val::Px(6.0),
                    border: UiRect::all(Val::Px(1.0)),
                    ..default()
                },
                BackgroundColor(PANEL_BG),
                BorderColor::all(LINE),
                Pickable::IGNORE,
            ))
            .with_children(|panel| {
                panel.spawn((
                    Text::new("CODE"),
                    super::ui::font_bold(13.0),
                    TextColor(TEXT_DIM),
                    Pickable::IGNORE,
                ));
                // The four digit cells. Typed digits in the clear — the
                // HUD line never masked them and the panel keeps its
                // information; the next empty cell wears the hot line, so
                // where the fifth keystroke would be dropped is visible.
                panel
                    .spawn((
                        Node {
                            flex_direction: FlexDirection::Row,
                            column_gap: Val::Px(6.0),
                            ..default()
                        },
                        Pickable::IGNORE,
                    ))
                    .with_children(|digits| {
                        for (i, c) in cells.iter().enumerate() {
                            digits.spawn((
                                Node {
                                    width: Val::Px(30.0),
                                    height: Val::Px(36.0),
                                    justify_content: JustifyContent::Center,
                                    align_items: AlignItems::Center,
                                    border: UiRect::all(Val::Px(1.0)),
                                    ..default()
                                },
                                BackgroundColor(if c.is_some() { CELL_FULL } else { CELL_BG }),
                                BorderColor::all(if i == cursor { LINE_HOT } else { LINE }),
                                Pickable::IGNORE,
                                children![(
                                    Text::new(match c {
                                        Some(d) => d.to_string(),
                                        None => String::new(),
                                    }),
                                    super::ui::font_bold(18.0),
                                    TextColor(Color::srgb(0.98, 0.97, 0.95)),
                                )],
                            ));
                        }
                    });
                panel.spawn((
                    Text::new(crate::ui::keypad::OPS_HINT),
                    super::ui::font(11.0),
                    TextColor(TEXT_DIM),
                    Pickable::IGNORE,
                ));
            });
        });
}

/// The swing prompt, plus the weak-spot cue when the player is standing in
/// the sector the server announced for this node.
///
/// **The cue is the whole teaching surface for the mechanic.** The bonus has
/// been in the sim since gather v0 and the mark has been on the wire and
/// decoded for as long, with nothing drawing it — so a player could only
/// discover it by noticing that some swings paid more, which is not
/// discoverable at all. The suffix is deliberately on the same line as the
/// verb rather than a second element: it is a property of the swing you are
/// about to take, not a separate thing happening.
fn swing_prompt_weak(occupant: u8, in_weak: bool) -> String {
    let base = swing_prompt(occupant);
    if base.is_empty() || !in_weak {
        return base;
    }
    format!("{base}  ·  WEAK SPOT")
}

/// Which face of the nearest sided piece the player stands on (hard/soft
/// v0) — the damage rule's one line of legibility. `side` was computed by
/// `sim_core::build::soft_side`, the same comparison `combat::raid`
/// prices the swing with, so this label cannot disagree with the bill;
/// shapes with no sides (and deployables) carry `None` and stay silent.
fn side_line(near: &Option<crate::ui::structure::Target>) -> String {
    match near.as_ref().and_then(|t| t.side) {
        Some(true) => "SOFT SIDE".to_string(),
        Some(false) => "HARD SIDE".to_string(),
        None => String::new(),
    }
}

/// What a fed hearth is holding, or `None` when it is holding nothing.
///
/// The rows are `(item, units)` and the count is authoritative — an empty
/// hearth acks with zero rows, and saying "0 ×" for a thing that is not there
/// is the dark-panel defect this repo has a rule against.
fn stock_line(rows: &[(u16, u32)], catalog: &protocol::event::ItemCatalog) -> Option<String> {
    let mut parts = Vec::new();
    for &(item, units) in rows {
        if units == 0 {
            continue;
        }
        parts.push(format!(
            "{units} × {}",
            crate::ui::craft::item_label(catalog, item)
        ));
    }
    if parts.is_empty() {
        return None;
    }
    Some(format!("HEARTH: {}", parts.join(", ")))
}

/// What a planted charge says, or `None` for a fuse of zero.
///
/// Seconds, not ticks: the fuse crosses the wire in ticks because the sim
/// counts in them, and `TICK_HZ` is the sim's own constant rather than a
/// number invented here. A player has no use for 240 and every use for 8.
///
/// **This is the warning, not a countdown.** A live timer ticking down on the
/// wall itself is a world-space element and is not built; what this buys is
/// the thing a defender actually needs first — that a charge exists at all,
/// and roughly how long they have. Said plainly here rather than implied,
/// because a toast that looked like a timer and did not move would be worse
/// than none.
fn charge_line(fuse_ticks: u16) -> Option<String> {
    if fuse_ticks == 0 {
        return None;
    }
    let hz = sim_core::limits::TICK_HZ as u16;
    // Round up: a fuse of one tick is "1s" and never "0s", for
    // `struct_hit_line`'s reason — a live thing must not read as finished.
    let secs = fuse_ticks.div_ceil(hz);
    Some(format!("CHARGE PLANTED  ·  {secs}s"))
}

/// What a hit on a structure says, or `None` when there is nothing honest to
/// say yet.
///
/// A raid is the one place in this game where a player spends real minutes
/// against a number they cannot see: before this, a wall took swings and
/// answered with nothing at all, so "is this working" was unanswerable except
/// by the wall eventually falling. The fraction is the answer.
///
/// `max == 0` is the defs-not-arrived state and draws nothing — a percentage
/// over an unknown maximum is worse than silence, and `ClientCore::struct_hit`
/// says so where the field is declared.
fn struct_hit_line(left: u16, max: u16) -> Option<String> {
    if max == 0 {
        return None;
    }
    let left = left.min(max);
    let pct = (left as u32 * 100).div_ceil(max as u32);
    Some(if left == 0 {
        "STRUCTURE DOWN".to_string()
    } else {
        format!("{left}/{max}  ·  {pct}%")
    })
}

/// Where a build cell stands relative to the player: an eight-point
/// bearing and a rounded distance, e.g. `NE 23M`. Planar, like every
/// reach test on the sim side.
///
/// **It CALLS [`crate::look::bearing_of`] rather than sharing its
/// convention**, and that is the 2026-08-15 row's whole argument made
/// structural. This function used to run its own `atan2` and say in a
/// comment that it agreed with the compass strip; when the compass was
/// found to be mirrored, the tree therefore had two sites to flip and
/// fixing one would have left the other reflected — half a switch, which
/// `CLAUDE.md`'s mingw entry says is worse than none. One conversion now,
/// so there is nothing left to keep in step.
fn whereabouts(px: f32, pz: f32, cx: u16, cz: u16) -> String {
    const POINTS: [&str; 8] = ["N", "NE", "E", "SE", "S", "SW", "W", "NW"];
    let half = sim_core::build::BUILD_CELL_M * 0.5;
    let dx = cx as f32 * sim_core::build::BUILD_CELL_M + half - px;
    let dz = cz as f32 * sim_core::build::BUILD_CELL_M + half - pz;
    let deg = crate::look::bearing_of(dx, dz);
    let idx = (((deg / 45.0) + 0.5) as usize) % 8;
    let dist = (dx * dx + dz * dz).sqrt();
    format!("{} {:.0}M", POINTS[idx], dist)
}

/// The wall readout: [`struct_hit_line`]'s fraction plus where the wall
/// stands. Composed rather than restated, so the honesty rules up there —
/// `max == 0` says nothing, a standing wall never reads 0% — hold here by
/// construction.
fn struct_readout(left: u16, max: u16, whereat: &str) -> Option<String> {
    Some(format!("{}  ·  {}", struct_hit_line(left, max)?, whereat))
}

/// The charge clock's line, or `None` once it is spent. Ceiling, for
/// [`charge_line`]'s stated reason — a live thing must not read as
/// finished, so the line says `1s` until the instant it is gone.
fn charge_readout(secs_left: f32, whereat: &str) -> Option<String> {
    if secs_left <= 0.0 {
        return None;
    }
    let secs = secs_left.ceil() as u32;
    Some(format!("CHARGE  ·  {secs}s  ·  {whereat}"))
}

/// One kill-feed line for a `(victim, killer)` pair.
///
/// `killer == victim` is how the ring reports a death nobody dealt — the
/// clock, the sea, or a player's own hand — because the wire pair carries no
/// cause. The feed says who; the death screen says how.
fn kill_line(victim: u32, killer: u32) -> Option<String> {
    Some(if killer == victim {
        format!("#{victim} died")
    } else {
        format!("#{killer} killed #{victim}")
    })
}

/// [`kill_line`], but silent for our own death: it is in the same ring, and
/// `ui::death` already owns that sentence with the cause and the range.
fn kill_line_for(victim: u32, killer: u32, own: u32) -> Option<String> {
    if victim == own {
        return None;
    }
    kill_line(victim, killer)
}

/// What the crosshair says for a swing pick, or `""` for a whiff.
///
/// `[LMB]` rather than a verb name because the swing is a button, and the
/// button is the thing the player has to connect the text to — the same
/// reasoning `Pick::prompt` uses for naming `[E]`.
fn swing_prompt(occupant: u8) -> String {
    let label = crate::ui::interact::swing_label(occupant);
    if label.is_empty() {
        String::new()
    } else {
        format!("[LMB] {label}")
    }
}

/// The eight-point bearing plus degrees, e.g. `NE  045°`.
///
/// The number comes from [`crate::look::bearing_deg`] rather than from the
/// free-running yaw, so this and the map's heading are one fact drawn twice
/// and not two samples a frame apart.
fn compass_strip(yaw: f32) -> String {
    const POINTS: [&str; 8] = ["N", "NE", "E", "SE", "S", "SW", "W", "NW"];
    let deg = crate::look::bearing_deg(yaw);
    let idx = (((deg / 45.0) + 0.5) as usize) % 8;
    format!("{}   {:03.0}°", POINTS[idx], deg)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The defect this queue exists for, in the shipped world.** A tree
    /// carries a secondary (`content/gatherables.toml`: wood, then
    /// mushrooms), so one swing into a full pack pushes two `EV_GATHER`
    /// zeros and `feedback` says two spill lines in one frame. The single
    /// slot showed the second and ate the first, so the player was told
    /// about the mushrooms and never about the wood — the one fact the
    /// feature was built to deliver.
    ///
    /// Red before the queue: with `say` as an overwrite, `row(1)` is `None`.
    #[test]
    fn both_spills_of_one_swing_survive_the_frame() {
        let mut t = Toast::default();
        t.say("pack full — WOOD dropped at your feet");
        t.say("pack full — MUSHROOMS dropped at your feet");
        assert_eq!(t.len(), 2, "two facts in one frame must be two lines");
        // Newest first: the row the eye starts at is the last thing said.
        assert_eq!(
            t.row(0).map(Say::text),
            Some("pack full — MUSHROOMS dropped at your feet")
        );
        assert_eq!(
            t.row(1).map(Say::text),
            Some("pack full — WOOD dropped at your feet"),
            "the earlier line of the same frame must still be on screen"
        );
        assert_eq!(
            t.row(2).map(Say::text),
            None,
            "and nothing beyond the live end"
        );
        assert_eq!(t.dropped(), 0, "nothing was over the cap");
    }

    /// The cap holds and says so. **Every line here is a `say`, so the whole
    /// stack is notes and drop-oldest-note IS drop-oldest** — this case
    /// exercises the common half of the policy (`TOAST_LINES`' doc), not the
    /// all-alarm fallback, which `a_flood_of_notes_cannot_push_the_refusal_off`
    /// and `an_all_alarm_stack_still_drops_the_oldest` own. What it holds either
    /// way: the newest fact is the one being reacted to, so the queue may
    /// never refuse the new line in favour of an old one.
    #[test]
    fn the_stack_is_bounded_and_drops_the_oldest() {
        let mut t = Toast::default();
        for i in 0..TOAST_LINES + 2 {
            t.say(format!("line {i}"));
        }
        assert_eq!(t.len(), TOAST_LINES, "the cap is the cap");
        assert_eq!(t.dropped(), 2, "and the two it ate are counted");
        let newest = format!("line {}", TOAST_LINES + 1);
        assert_eq!(
            t.row(0).map(Say::text),
            Some(newest.as_str()),
            "the newest survives"
        );
        assert_eq!(
            t.row(TOAST_LINES - 1).map(Say::text),
            Some("line 2"),
            "the oldest two went, in order"
        );
        assert_eq!(t.row(TOAST_LINES).map(Say::text), None);
    }

    /// **The refusal is the line the cap used to eat first.** `feedback`
    /// wrote refusals before payouts, so on a frame with more facts than
    /// rows the sentence the player asked for by pressing a key was the
    /// oldest and went first, while four gathers it could read off its own
    /// pack stayed. That was not a policy anybody chose — it was insertion
    /// order standing in for importance.
    ///
    /// Red before rank: with drop-oldest the refusal is gone and `row(3)` is
    /// `line 1`.
    #[test]
    fn a_flood_of_notes_cannot_push_the_refusal_off() {
        let mut t = Toast::default();
        t.warn("you do not have the parts");
        for i in 0..TOAST_LINES + 1 {
            t.say(format!("+1 × WOOD {i}"));
        }
        assert_eq!(t.len(), TOAST_LINES, "the cap still holds");
        assert_eq!(
            t.row(TOAST_LINES - 1).map(Say::text),
            Some("you do not have the parts"),
            "the alarm survives every note, and stays the oldest"
        );
        assert_eq!(
            t.row(TOAST_LINES - 1).map(Say::rank),
            Some(Rank::Alarm),
            "and it is an alarm that kept it, not luck"
        );
        assert_eq!(t.dropped(), 2, "the two notes it ate are counted");
        assert_eq!(
            t.row(0).map(Say::text),
            Some(format!("+1 × WOOD {}", TOAST_LINES).as_str()),
            "drawing order is still recency — the newest note is on top"
        );
    }

    /// Rank picks the victim; it does not refuse the push. A stack that is
    /// all alarms falls back to drop-oldest, because the newest fact is the
    /// one being reacted to and a sink that rejected it would hide the thing
    /// that just happened.
    ///
    /// Red if the fallback refused instead: `row(0)` would be the fourth
    /// alarm, not the fifth.
    #[test]
    fn an_all_alarm_stack_still_drops_the_oldest() {
        let mut t = Toast::default();
        for i in 0..TOAST_LINES + 1 {
            t.warn(format!("refused {i}"));
        }
        assert_eq!(t.len(), TOAST_LINES, "the cap is the cap for alarms too");
        assert_eq!(t.dropped(), 1);
        assert_eq!(
            t.row(0).map(Say::text),
            Some(format!("refused {}", TOAST_LINES).as_str()),
            "the newest alarm landed"
        );
        assert_eq!(
            t.row(TOAST_LINES - 1).map(Say::text),
            Some("refused 1"),
            "and the oldest one went"
        );
    }

    /// **The sink says how much it is not telling you.** `dropped` was
    /// written by the cap and read by nothing but a test, so a frame that
    /// lost two facts looked exactly like a frame that lost none. The marker
    /// rides the last live row as a suffix rather than replacing a sentence:
    /// after the rank change the bottom row is precisely where a rescued
    /// alarm sits, so drawing over it would eat the line the policy above
    /// exists to keep.
    ///
    /// Red before the marker: `overflow()` is `""` and `row_parts` does not
    /// exist.
    #[test]
    fn the_sink_says_how_many_it_ate_and_covers_nothing() {
        let mut t = Toast::default();
        t.warn("you do not have the parts");
        for i in 0..TOAST_LINES + 1 {
            t.say(format!("+1 × WOOD {i}"));
        }
        assert_eq!(
            t.overflow(),
            "   …+2 more",
            "two notes went, and it says so"
        );

        let (say, extra) = t.row_parts(TOAST_LINES - 1).expect("the last live row");
        assert_eq!(
            say.text(),
            "you do not have the parts",
            "nothing is covered"
        );
        assert_eq!(extra, "   …+2 more", "the marker rides the last live row");

        for row in 0..TOAST_LINES - 1 {
            assert_eq!(
                t.row_parts(row).map(|(_, e)| e),
                Some(""),
                "and only that row"
            );
        }
        assert!(
            t.row_parts(TOAST_LINES).is_none(),
            "past the live end, nothing"
        );
    }

    /// The marker is a state and the counter is evidence, so they clear on
    /// different schedules: `unseen` goes when the stack empties — the burst
    /// is over and there is nothing left for a count to qualify — and
    /// `dropped` never does.
    ///
    /// Red if the reset were dropped, or if it reset `dropped` with it.
    #[test]
    fn the_marker_clears_when_the_stack_does_and_the_counter_does_not() {
        let mut t = Toast::default();
        for i in 0..TOAST_LINES + 2 {
            t.say(format!("line {i}"));
        }
        assert_eq!(t.overflow(), "   …+2 more");

        t.tick(TOAST_SECS + 0.1);
        assert_eq!(t.len(), 0, "every line ran out");
        assert_eq!(t.overflow(), "", "so the marker has nothing to qualify");
        assert_eq!(t.dropped(), 2, "but the cap did bite, and that is evidence");

        t.say("+1 × WOOD");
        assert_eq!(
            t.row_parts(0).map(|(_, e)| e),
            Some(""),
            "a fresh burst starts unmarked"
        );
    }

    /// A repeat refreshes and counts rather than spending a slot. Ten swings
    /// at a tree would otherwise flush every other fact off the stack, which
    /// is the single-slot defect wearing a queue's clothes.
    #[test]
    fn a_repeat_counts_instead_of_spending_a_slot() {
        let mut t = Toast::default();
        t.say("nothing in reach");
        t.say("+1 × WOOD");
        t.say("+1 × WOOD");
        t.say("+1 × WOOD");
        assert_eq!(t.len(), 2, "three identical swings are one line");
        assert_eq!(t.row(0).map(Say::text), Some("+1 × WOOD  ×3"));
        assert_eq!(t.row(0).map(Say::reps), Some(3));
        // The suffix is rewritten from the sentence, never appended to the
        // last render of it — `×2×3×4` is the bug this guards.
        t.say("+1 × WOOD");
        assert_eq!(t.row(0).map(Say::text), Some("+1 × WOOD  ×4"));
        // And the refusal it must not have evicted is still there.
        assert_eq!(t.row(1).map(Say::text), Some("nothing in reach"));
        // A different sentence takes its own row even if the old one is up.
        t.say("+1 × STONE");
        assert_eq!(t.len(), 3);
        assert_eq!(t.row(1).map(Say::text), Some("+1 × WOOD  ×4"));
    }

    /// **A repeat raises a line's rank and never lowers it.** The fast path
    /// is the one write that returns without touching the field the eviction
    /// reads, so a sentence first said as a note stayed evictable no matter
    /// how it arrived the second time. Not reachable on today's call sites —
    /// no string is produced by both a `say` and a `warn` — which is exactly
    /// why it needs a gate rather than a comment: the next `warn` that
    /// borrows a payout's wording would re-open it silently.
    ///
    /// Red without the two-line upgrade in `push`: the flood eats the line
    /// and `row(TOAST_LINES - 1)` is a note.
    #[test]
    fn a_repeat_can_only_raise_a_lines_rank() {
        let mut t = Toast::default();
        t.say("your pack is full");
        t.warn("your pack is full");
        assert_eq!(t.len(), 1, "it is still one line");
        assert_eq!(t.row(0).map(Say::reps), Some(2), "and it still counted");
        assert_eq!(
            t.row(0).map(Say::rank),
            Some(Rank::Alarm),
            "the alarm arrival wins — the fact dies with the line either way"
        );
        // The upgrade is worth what the eviction pays for it, so prove it
        // there rather than on the field alone.
        for i in 0..TOAST_LINES {
            t.say(format!("+1 × WOOD {i}"));
        }
        assert_eq!(
            t.row(TOAST_LINES - 1).map(Say::text),
            Some("your pack is full  ×2"),
            "and it survives the flood it would have been eaten by"
        );

        // The other direction: an alarm is never demoted by a later note,
        // which would put a refusal back on the menu the cap eats from.
        let mut t = Toast::default();
        t.warn("you do not have the parts");
        t.say("you do not have the parts");
        assert_eq!(
            t.row(0).map(Say::rank),
            Some(Rank::Alarm),
            "a note repeat cannot demote an alarm"
        );
    }

    /// Each line runs its own clock, and the survivors close ranks. Only a
    /// repeat can refresh a line, and only the newest can be repeated, so
    /// the queue always retires from the front — but `tick` retires by
    /// TIMER, and this proves the compaction rather than the assumption.
    #[test]
    fn every_line_keeps_its_own_clock() {
        let mut t = Toast::default();
        t.say("old news");
        t.tick(TOAST_SECS - 0.5);
        t.say("fresh news");
        assert_eq!(t.len(), 2);
        // Half a second later the first is out of clock and the second is
        // not, and the second must slide up into row 0's brightness.
        t.tick(0.6);
        assert_eq!(t.len(), 1, "the expired line went");
        assert_eq!(t.row(0).map(Say::text), Some("fresh news"));
        assert!(t.row(0).unwrap().left() > 0.0);
        assert_eq!(t.dropped(), 0, "an expiry is not an overflow");
        // Run the rest of its clock out and the stack is empty, not stale.
        t.tick(TOAST_SECS);
        assert!(t.is_empty());
        assert_eq!(t.row(0).map(Say::text), None);
    }

    /// The hitmarker keeps its own, shorter clock, and a talkative frame
    /// must not extend it — it is confirmation, not a readout.
    #[test]
    fn the_hitmarker_is_not_a_toast() {
        let mut t = Toast::default();
        t.hit(37);
        t.say("+1 × WOOD");
        assert_eq!(t.hit_damage, 37);
        t.tick(HITMARK_SECS + 0.01);
        assert_eq!(t.hit_left, 0.0, "the marker is off");
        assert_eq!(t.len(), 1, "…while the line it landed with is still up");
    }

    /// `E` outranks the swing and the swing fills the silence. Asserted
    /// here because it is an ORDERING, and an ordering is the one thing a
    /// compile cannot check — the browser shipped this as sixteen swept
    /// combinations in a gate that no longer exists.
    /// The charge warning. Ticks are the wire's unit and seconds are the
    /// player's; `TICK_HZ` does the conversion and is the sim's own.
    #[test]
    fn a_charge_warns_in_seconds() {
        let hz = sim_core::limits::TICK_HZ as u16;
        assert_eq!(charge_line(0), None, "no fuse is not a charge");
        assert_eq!(
            charge_line(hz * 8),
            Some("CHARGE PLANTED  ·  8s".to_string())
        );
        // A fuse about to blow must not read as already blown.
        assert_eq!(charge_line(1), Some("CHARGE PLANTED  ·  1s".to_string()));
        // And a part-second rounds up rather than down.
        assert_eq!(
            charge_line(hz + 1),
            Some("CHARGE PLANTED  ·  2s".to_string())
        );
    }

    /// The hearth readout. An empty hearth says nothing rather than "0 ×".
    #[test]
    fn a_hearth_lists_what_it_holds() {
        let cat = catalog_with_names(&[(3, "WOOD"), (4, "CLOTH")]);
        assert_eq!(stock_line(&[], &cat), None, "no rows must say nothing");
        assert_eq!(
            stock_line(&[(3, 0)], &cat),
            None,
            "a zero row is not a holding"
        );
        assert_eq!(
            stock_line(&[(3, 120)], &cat),
            Some("HEARTH: 120 × WOOD".to_string())
        );
        assert_eq!(
            stock_line(&[(3, 120), (4, 5)], &cat),
            Some("HEARTH: 120 × WOOD, 5 × CLOTH".to_string())
        );
        // A zero row between two live ones is skipped, not printed empty.
        assert_eq!(
            stock_line(&[(3, 120), (4, 0)], &cat),
            Some("HEARTH: 120 × WOOD".to_string())
        );
    }

    fn catalog_with_names(rows: &[(usize, &str)]) -> protocol::event::ItemCatalog {
        let mut c = protocol::event::ItemCatalog::EMPTY;
        let mut top = 0;
        for &(idx, name) in rows {
            c.set(idx, name.as_bytes(), 0).unwrap();
            top = top.max(idx);
        }
        c.count = (top + 1) as u16;
        c
    }

    /// The structure readout. `max == 0` must stay silent — that is the
    /// defs-not-arrived state, and a percentage over an unknown maximum is
    /// the lie `ClientCore::struct_hit`'s own doc warns against.
    #[test]
    fn a_struct_hit_reads_out_or_stays_quiet() {
        assert_eq!(struct_hit_line(0, 0), None, "no defs yet must say nothing");
        assert_eq!(struct_hit_line(900, 0), None, "…even with hp in hand");
        assert_eq!(
            struct_hit_line(875, 1750),
            Some("875/1750  ·  50%".to_string())
        );
        assert_eq!(struct_hit_line(0, 1750), Some("STRUCTURE DOWN".to_string()));
        // A wall one point from falling must not round to 0% and read as
        // down when it is still standing — the round is up for that reason.
        assert_eq!(
            struct_hit_line(1, 1750),
            Some("1/1750  ·  1%".to_string()),
            "a standing wall must never report 0%"
        );
        // And a full wall is 100%, not 101%.
        assert_eq!(
            struct_hit_line(1750, 1750),
            Some("1750/1750  ·  100%".to_string())
        );
        // hp above max is a server bug; clamp rather than print >100%.
        assert_eq!(
            struct_hit_line(9999, 1750),
            Some("1750/1750  ·  100%".to_string())
        );
    }

    /// The side label (hard/soft v0): says which face you are on for a
    /// sided piece, and nothing at all otherwise — a sideless shape must
    /// not read as "HARD" when the truth is "has no sides".
    #[test]
    fn the_side_line_names_the_face_or_stays_quiet() {
        use crate::ui::structure::{Store, Target};
        let t = |side| {
            Some(Target {
                store: Store::Piece,
                cx: 0,
                cz: 0,
                level: 0,
                loc: 2,
                row: 0,
                // Undamaged; this test is about the side label and nothing
                // else reads the band (wire v44).
                dmg: 0,
                hp_max: 10,
                side,
            })
        };
        assert_eq!(side_line(&t(Some(true))), "SOFT SIDE");
        assert_eq!(side_line(&t(Some(false))), "HARD SIDE");
        assert_eq!(side_line(&t(None)), "");
        assert_eq!(side_line(&None), "");
    }

    /// The kill-feed line, as a pure function of the pair the ring carries.
    /// Extracted so the sentence has a gate: the system around it needs a
    /// live `Net`, and a sentence assembled inside a Bevy system is one no
    /// headless test can read back (`ui::death`'s header, same argument).
    #[test]
    fn the_kill_feed_names_who_and_skips_our_own() {
        // A stranger killed by another stranger.
        assert_eq!(kill_line(9, 4), Some("#4 killed #9".to_string()));
        // Self-inflicted, or the world: the ring gives killer == victim.
        assert_eq!(kill_line(9, 9), Some("#9 died".to_string()));
        // Ours never reaches the feed — the death SCREEN owns it, and the
        // same EV_DEATH feeds both.
        assert_eq!(kill_line_for(9, 4, 9), None);
        assert_eq!(kill_line_for(9, 9, 9), None);
    }

    #[test]
    fn e_outranks_the_swing_and_the_swing_fills_the_silence() {
        use crate::ui::interact::{Pick, Verb};
        use sim_core::terrain::Occupant;

        let silent = Pick::default();
        assert_eq!(silent.prompt(), "", "a whiffed E must say nothing");

        // Where E is silent, the swing speaks.
        assert_eq!(swing_prompt(Occupant::Tree as u8), "[LMB] CHOP TREE");
        assert_eq!(
            swing_prompt(Occupant::BarrelSlot as u8),
            "[LMB] SMASH BARREL"
        );

        // Where E has something, it wins — the caller takes E's string first
        // and never reaches the swing. This asserts E is non-empty for every
        // verb, which is the precondition that makes that `match` correct.
        for v in [Verb::Door, Verb::Bag, Verb::Box, Verb::Hearth] {
            let p = Pick {
                verb: v,
                ..Default::default()
            };
            assert!(!p.prompt().is_empty(), "{v:?} must claim the prompt");
        }

        // And a swing at nothing is silent rather than "[LMB] ".
        assert_eq!(swing_prompt(0), "");
        assert_eq!(swing_prompt(Occupant::Rock as u8), "");
    }

    /// The readout's WHERE. North is `+Z` and east is `-X`
    /// (`DECISIONS.md` 2026-08-15), a build cell is 3 m, and the distance is
    /// measured to the cell's centre — the same point every reach test on the
    /// sim side measures to.
    #[test]
    fn the_readout_names_where_the_wall_stands() {
        let half = sim_core::build::BUILD_CELL_M * 0.5;
        // Standing on cell (0,0)'s centre: cell (3,0) is 9 m due WEST
        // (+X), cell (0,3) is 9 m due north (+Z).
        assert_eq!(whereabouts(half, half, 3, 0), "W 9M");
        assert_eq!(whereabouts(half, half, 0, 3), "N 9M");
        assert_eq!(whereabouts(half, half, 3, 3), "NW 13M");
        // From the far side the bearing flips.
        let (px, pz) = (
            6.0 * sim_core::build::BUILD_CELL_M + half,
            6.0 * sim_core::build::BUILD_CELL_M + half,
        );
        assert_eq!(whereabouts(px, pz, 6, 3), "S 9M");
        assert_eq!(whereabouts(px, pz, 3, 6), "E 9M");
        // Underfoot must not panic or NaN; it reads as zero metres.
        assert!(whereabouts(half, half, 0, 0).ends_with("0M"));
    }

    /// The wall line keeps `struct_hit_line`'s honesty (composed, not
    /// restated), and the clock never says `0s` while it is live.
    #[test]
    fn the_pinned_lines_stay_honest() {
        assert_eq!(
            struct_readout(875, 0, "E 3M"),
            None,
            "no defs yet must pin nothing"
        );
        assert_eq!(
            struct_readout(875, 1750, "E 3M"),
            Some("875/1750  ·  50%  ·  E 3M".to_string())
        );
        assert_eq!(
            struct_readout(0, 1750, "E 3M"),
            Some("STRUCTURE DOWN  ·  E 3M".to_string())
        );
        assert_eq!(charge_readout(0.0, "NE 9M"), None, "a spent clock is gone");
        assert_eq!(charge_readout(-1.0, "NE 9M"), None);
        assert_eq!(
            charge_readout(0.2, "NE 9M"),
            Some("CHARGE  ·  1s  ·  NE 9M".to_string()),
            "about to blow is 1s, never 0s"
        );
        assert_eq!(
            charge_readout(7.01, "NE 9M"),
            Some("CHARGE  ·  8s  ·  NE 9M".to_string()),
            "a part-second rounds up, like the announce toast's"
        );
    }

    #[test]
    fn the_compass_walks_the_sims_bearing() {
        // Yaw 0 is +Z is north. Yaw turns the view LEFT (`look.rs`'s header),
        // and east is `-X`, so a quarter turn lands on WEST.
        assert!(compass_strip(0.0).starts_with('N'));
        assert!(compass_strip(std::f32::consts::FRAC_PI_2).starts_with('W'));
        assert!(compass_strip(std::f32::consts::PI).starts_with('S'));
        assert!(compass_strip(3.0 * std::f32::consts::FRAC_PI_2).starts_with('E'));
        // And it wraps rather than reading 360.
        assert!(compass_strip(std::f32::consts::TAU - 0.001).starts_with('N'));
        // Due north prints `000`, not `-00` — `look::bearing_of`'s signed-zero
        // trap, gated at the `{:03.0}` site that would have shown it.
        assert_eq!(compass_strip(0.0), "N   000°");
    }

    // ---- the announce stack's geometry ----------------------------------
    //
    // **`NOW.md` §0tq's last bullet, and the merge-gate judge's ranked fix 3
    // of pass -12: nobody has looked at the arithmetic.** Three passes landed
    // announce work — the stack, the spill line, the rank and the `…+N more`
    // marker — and every one of them ended on the same sentence.
    //
    // What these gate is the part of "looking" that a person is BAD at and a
    // machine is good at: whether four boxes of a known size, placed at known
    // percentages of a window of a known height, land on top of each other or
    // on top of something else. What they deliberately do not gate is whether
    // the result reads well, which is a person's job and which `CLAUDE.md`
    // forbids automating for a reason it paid for once already.

    /// The height the client actually opens at.
    ///
    /// **Read off Bevy rather than written down as 720.** The client sets no
    /// `WindowPlugin` (`bin/gates.rs` overrides `AssetPlugin` and nothing
    /// else), so the window it gets is the dependency's default — and a
    /// `--capture` run pins itself windowed (`render/mod.rs`), which is why
    /// every PNG the probe has ever written is this tall. A literal here
    /// would keep passing through a Bevy bump that moved the default out from
    /// under the layout.
    fn opening_height_px() -> f32 {
        Window::default().resolution.physical_height() as f32
    }

    /// How far the lowest crosshair tick reaches below the aim point, px.
    fn crosshair_reach_px() -> f32 {
        CROSSHAIR_TICKS
            .iter()
            .map(|&(_, dy, _, h)| dy - h * 0.5 + h)
            .fold(f32::MIN, f32::max)
    }

    /// **[`TOAST_LINES`]' doc claim, made arithmetic.** It says four rows fit
    /// *between the prompt and the readout* *without reaching the crosshair*,
    /// and until now that was three assertions in prose with nothing behind
    /// them. Every box below is `top` + its line box, in px, at the window
    /// the client opens at.
    ///
    /// Red if any knob is nudged into its neighbour: `TOAST_TOP_PCT` down to
    /// 55 puts row 0 inside the prompt, `PROMPT_TOP_PCT` up to 51 puts the
    /// prompt on the crosshair.
    #[test]
    fn the_stack_hangs_between_the_prompt_and_the_hotbar() {
        let h = opening_height_px();
        let pct = |p: f32| p / 100.0 * h;

        // The crosshair is the top end, and text over the aim point is text
        // in the way of the thing it describes.
        let crosshair_bottom = pct(CROSSHAIR_CENTRE_PCT) + crosshair_reach_px();
        let prompt_top = pct(PROMPT_TOP_PCT);
        assert!(
            prompt_top > crosshair_bottom,
            "the prompt sits on the crosshair: prompt {prompt_top} vs tick {crosshair_bottom}"
        );

        // The prompt is the stack's own ceiling.
        let prompt_bottom = prompt_top + line_box_px(PROMPT_FONT_PX);
        let row0_top = pct(toast_row_top_pct(0));
        assert!(
            row0_top >= prompt_bottom,
            "row 0 overlaps the prompt: row {row0_top} vs prompt bottom {prompt_bottom}"
        );

        // The readout is the floor, and it must clear the last row the cap
        // can hold rather than the last row that happens to be live.
        let last_row_bottom = pct(toast_row_top_pct(TOAST_LINES - 1)) + line_box_px(TOAST_FONT_PX);
        let readout_top = pct(readout_top_pct());
        assert!(
            readout_top >= last_row_bottom,
            "the readout overlaps the deepest row: {readout_top} vs {last_row_bottom}"
        );

        // And the whole thing clears the hotbar, which is the one neighbour
        // measured from the OTHER edge of the screen.
        let readout_bottom = readout_top + line_box_px(TOAST_FONT_PX);
        let hotbar_top = h - HOTBAR_BOTTOM_PX - HOTBAR_CELL_PX;
        assert!(
            readout_bottom < hotbar_top,
            "the readout is behind the hotbar: {readout_bottom} vs {hotbar_top}"
        );
    }

    /// **The mixed-unit seam: pitch is a percent, type is px.** So the gap
    /// between two rows shrinks with the window and the text in them does
    /// not, and below some height four lines are a smear — the single-slot
    /// defect arriving from the other direction.
    ///
    /// Nobody had computed that height. It is 600 px against the 720 the
    /// client opens at, so the shipped margin is 120 px and this test is what
    /// notices when a knob spends it.
    ///
    /// Red at `TOAST_PITCH_PCT = 2.3`, which is a plausible tightening and
    /// would put the threshold at 730 — above the window the probe photographs
    /// in.
    #[test]
    fn the_rows_do_not_overlap_at_the_window_the_client_opens_at() {
        let min = toast_min_window_height_px();
        assert!(
            min <= opening_height_px(),
            "the announce rows overlap at the window the client opens at: \
             needs {min} px, opens at {} px",
            opening_height_px()
        );
        // The threshold is the pitch's own definition, so state it both ways
        // — a pitch that no longer separates the boxes is the same bug as a
        // window that is too short.
        let pitch_px = TOAST_PITCH_PCT / 100.0 * opening_height_px();
        assert!(
            pitch_px >= line_box_px(TOAST_FONT_PX),
            "a row's box is taller than the step to the next one: \
             box {} px, pitch {pitch_px} px",
            line_box_px(TOAST_FONT_PX)
        );
        // Rows march one way. A negative pitch would satisfy the two checks
        // above on absolute value alone.
        for row in 1..TOAST_LINES {
            assert!(
                toast_row_top_pct(row) > toast_row_top_pct(row - 1),
                "row {row} is not below row {}",
                row - 1
            );
        }
    }

    /// **Every row the cap holds must be a row something can be seen in.**
    /// [`TOAST_LINES`] and [`TOAST_ROW_DIM`] multiply, and nothing said so:
    /// at the shipped 4 × 0.16 the deepest row draws at 0.52, but at 7 rows
    /// it is 0.04 and at 8 it is exactly zero — a queue that accepts a line,
    /// counts it as delivered and renders it invisible, which is worse than
    /// the drop it replaced because `dropped` would not move either.
    ///
    /// The floor asserted is *greater than zero*, not a legibility number.
    /// Whether 0.52 is dim enough to read is a look, and a look is the
    /// operator's — this is only the half that can be wrong arithmetically.
    #[test]
    fn every_row_the_cap_holds_can_be_seen() {
        for row in 0..TOAST_LINES {
            assert!(
                toast_row_dim(row) > 0.0,
                "row {row} of {TOAST_LINES} draws at alpha {} — the cap holds \
                 a line nothing can show",
                toast_row_dim(row)
            );
        }
        // Newest brightest, and each step a real step: equal rows are the
        // paragraph `TOAST_ROW_DIM` exists to break up.
        assert_eq!(toast_row_dim(0), 1.0, "the newest line is undimmed");
        for row in 1..TOAST_LINES {
            assert!(
                toast_row_dim(row) < toast_row_dim(row - 1),
                "row {row} is not dimmer than the one above it"
            );
        }
    }

    /// **One rule, one place — asserted on the spawn, not on the function.**
    /// The two geometry calls used to be the same expression typed twice, and
    /// a test that calls `toast_row_top_pct` cannot tell whether [`setup`]
    /// still does. This is a grep, in the shape `tests/ui.rs` §F and
    /// `tests/sound.rs` already use for the same reason: the defect is a call
    /// site rather than a value, so the value cannot detect it.
    ///
    /// Red the moment either spawn goes back to computing its own top.
    #[test]
    fn the_spawn_reads_the_geometry_rather_than_recomputing_it() {
        let src = include_str!("hud.rs");
        // A spawn writes a percent `Val`; the geometry functions return a
        // bare `f32` and never mention `Val`. So the two appearing on one
        // line is exactly the shape being forbidden — a placement computing
        // its own answer — and it cannot match the functions themselves.
        //
        // Both needles are assembled at runtime so this test does not match
        // its OWN source, which is the first thing it did.
        let placed = ["Val::", "Percent("].concat();
        let raw_top = ["TOAST_", "TOP_PCT"].concat();
        for (n, line) in src.lines().enumerate() {
            assert!(
                !(line.contains(&placed) && line.contains(&raw_top)),
                "hud.rs:{} is computing its own row top again — call \
                 toast_row_top_pct/readout_top_pct instead:\n  {}",
                n + 1,
                line.trim()
            );
        }
        assert!(
            src.contains("Val::Percent(toast_row_top_pct(row))"),
            "the announce rows no longer take their top from the geometry"
        );
        assert!(
            src.contains("Val::Percent(readout_top_pct())"),
            "the readout no longer takes its top from the geometry"
        );
    }
}
