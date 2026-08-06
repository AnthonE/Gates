//! The loading screen's **model**: whether the server has told us what to
//! build, and how much of it is built.
//!
//! **Why this is not in `render/loading.rs`.** The same rule the panels
//! obey ([`crate::ui`]): everything with arithmetic in it lives here, pure
//! and headless, and the render layer is nodes and colours. The arithmetic
//! in question is small but it is the thing that decides *when a player
//! enters the world*, and the version of that decision that lived inside a
//! Bevy system could only be tested by a windowed run against a live shard.
//! `crates/client/tests/ui.rs` §E drives it in the code tier instead.
//!
//! ## The order the two questions come in
//!
//! There are two, and they are not interchangeable:
//!
//! 1. **Has the server placed us?** The welcome carries `player_id`, `seed`
//!    and `tick` — and no position. Where the player actually stands arrives
//!    one snapshot later, when the first packet carrying our own entity lets
//!    `Predictor::adopt` take the authoritative spawn. Until that lands the
//!    client does not know what to load.
//! 2. **Is what it named built yet?** The three ring builders each report
//!    how many of their cells are resident, and the far mesh is a bit.
//!
//! Asking the second one first is the bug this module exists to make
//! untestable-by-inspection. `Predictor::body` before its first snapshot is
//! `Body::default()`, whose position is the **world origin** — and the world
//! origin is a real place on this island, not a sentinel. A ring builder
//! handed it does not fault or notice; it builds a perfectly good
//! neighbourhood of somewhere the server never named, and one snapshot later
//! the eye moves to the real spawn and every one of those chunks is evicted
//! and rebuilt. That much happened on every connect. The worse outcome is a
//! **race** rather than the common case — the rings fill one cell a frame, so
//! it takes a first snapshot later than the whole ring build for the bar to
//! reach full at the origin and the world to open around an unplaced player.
//!
//! So [`Progress::placed`] gates the other four fields rather than sitting
//! beside them: unplaced reads 0.0 and is never [`done`](Progress::done),
//! whatever the rings happen to say.

/// What the client has been told to build, and how much of it it has.
///
/// Totals are carried rather than looked up so this stays free of the
/// render module's constants — `render::loading::read` fills them from the
/// shipped ring sizes, and the tests fill them from whatever they need.
#[derive(Clone, Copy, Debug)]
pub struct Progress {
    /// Has the server said where the player is?
    ///
    /// False from the welcome until the first snapshot carrying our own
    /// entity (`Predictor::started`). Nothing about the world is knowable
    /// before it, which is why it is not one term of the mean below but a
    /// gate in front of it.
    pub placed: bool,
    /// Ground chunks resident, out of the near ring's total.
    pub ground: (usize, usize),
    /// Scatter parents resident, out of the same ring's total.
    pub scatter: (usize, usize),
    /// Clutter tiles resident, out of the clutter ring's total.
    pub clutter: (usize, usize),
    /// The far mesh, which is one build rather than a ring and so is a bit
    /// rather than a fraction.
    pub far: bool,
}

impl Progress {
    /// Nothing built, and nothing yet known about what to build. The state
    /// every connect passes through.
    pub fn waiting() -> Self {
        Self {
            placed: false,
            ground: (0, 1),
            scatter: (0, 1),
            clutter: (0, 1),
            far: false,
        }
    }

    /// 0.0 to 1.0. The mean of the three rings, so no single ring can carry
    /// the bar to full while another has not started — and flat zero until
    /// the server has placed us, because a ring that is filling around an
    /// unplaced eye is filling in the wrong place and its count is not
    /// progress toward anything.
    pub fn fraction(&self) -> f32 {
        if !self.placed {
            return 0.0;
        }
        let f = |(a, b): (usize, usize)| (a as f32 / b.max(1) as f32).clamp(0.0, 1.0);
        (f(self.ground) + f(self.scatter) + f(self.clutter)) / 3.0
    }

    /// Is there a world to walk into? Placed **and** every ring resident
    /// **and** the far mesh built.
    pub fn done(&self) -> bool {
        self.placed
            && self.far
            && self.ground.0 >= self.ground.1
            && self.scatter.0 >= self.scatter.1
            && self.clutter.0 >= self.clutter.1
    }

    /// The counts line under the bar. Says which of the two questions is
    /// outstanding rather than printing three honest-looking zeroes for a
    /// stage in which no ring has been asked for anything.
    pub fn line(&self) -> String {
        if !self.placed {
            return "WAITING FOR THE SERVER TO PLACE YOU".to_string();
        }
        format!(
            "GROUND {}/{}      SCATTER {}/{}      CLUTTER {}/{}",
            self.ground.0,
            self.ground.1,
            self.scatter.0,
            self.scatter.1,
            self.clutter.0,
            self.clutter.1
        )
    }
}
