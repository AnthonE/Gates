//! The boot splash's **model**: what the client is still warming, and which
//! screen it opens when it stops.
//!
//! **Why there is a splash at all, and it is not decoration.** Until this
//! landed, double-clicking the game showed *nothing* until the window
//! existed, and on the launcher path that interval contained: a blocking
//! round trip to the scry launcher over a local socket, a blocking QUIC
//! connect that ended in `exit(1)` on failure, wgpu adapter enumeration,
//! window creation, the generated sound bank, ten rasterised wheel rings,
//! two TTF parses and 57 icon PNGs. A player who double-clicked a shortcut
//! and got a failed connect saw a console line they had no console for. Rust
//! and League both cover the same interval with a window that says the game
//! is starting; the reason is the same in all three.
//!
//! **It is a state, not a splash bitmap** — the same argument
//! [`crate::render::menu`] already makes about connecting. Once the window is
//! the first thing that happens, the slow work moves *inside* the app where
//! it can be reported, cancelled and recovered from, and a failed connect
//! lands on the server list with its reason instead of on stderr.
//!
//! ## The two waits, and why neither may be skipped
//!
//! Exactly the shape of [`crate::ui::load`], one screen earlier:
//!
//! 1. **Is the client warm?** The textures, icons, animations and mob meshes
//!    that `Startup` asks the asset server for. A menu drawn before them is a
//!    menu with no icons on it, and — the trap `CLAUDE.md` names — the first
//!    frame that *uses* a pipeline is the frame that pays to specialize it.
//! 2. **Who is playing?** `Scry::discover` is a blocking socket round trip.
//!    It used to happen before the window, which is what made the window
//!    late; run from inside, it must still be *answered* before the shell
//!    draws an identity, or the chip says "anonymous" and then changes its
//!    mind in front of the player.
//!
//! Neither is a clock. `CLAUDE.md`: a gate — or a player — waits on
//! observable state, never on elapsed milliseconds.

/// Where the splash hands off to.
///
/// **This is the arithmetic worth gating**, and it is one bit wide: a client
/// the launcher already pointed at a shard must not be asked to pick one
/// again (`args::server_given`, the same bit `menu`'s header calls out), and
/// a client with no shard must not be sent to a connect screen with an
/// address nobody chose. Returning an enum rather than setting a state
/// inside a system is what lets `tests/ui.rs` drive both arms with no window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Next {
    /// Pick a shard. The bare `gates` case, and the launcher case where the
    /// `{server}` placeholder went unfilled.
    Menu,
    /// The launcher (or the command line) already chose. Straight to the
    /// connect screen, which owns the timeout and the way back.
    Connect,
}

/// What the splash is waiting for.
#[derive(Clone, Copy, Debug)]
pub struct Boot {
    /// Every asset `Startup` asked for is resident.
    pub warm: bool,
    /// The scry handshake has answered — including "there is no launcher
    /// running", which is a normal answer and not a failure.
    pub greeted: bool,
    /// A shard was named on the command line or by the launcher.
    pub chosen: bool,
}

impl Boot {
    /// Nothing warmed, nobody greeted. The state every launch passes through.
    pub fn new(chosen: bool) -> Self {
        Self {
            warm: false,
            greeted: false,
            chosen,
        }
    }

    /// Is the client ready to draw a real screen?
    ///
    /// Both waits, never one. They finish in either order — the asset server
    /// and the launcher socket know nothing about each other — so this is an
    /// `&&` rather than a sequence, and [`line`](Self::line) is what tells
    /// the player which one is still out.
    pub fn done(&self) -> bool {
        self.warm && self.greeted
    }

    /// Where to go when [`done`](Self::done). `None` until then, so a caller
    /// cannot leave the splash by asking the wrong question — the state
    /// change and the readiness test are one call.
    pub fn next(&self) -> Option<Next> {
        if !self.done() {
            return None;
        }
        Some(if self.chosen {
            Next::Connect
        } else {
            Next::Menu
        })
    }

    /// 0.0 to 1.0, over the two waits. Coarse on purpose: it is two bits and
    /// it says so, rather than interpolating toward a full bar on a clock the
    /// way a splash that has nothing real to report has to.
    pub fn fraction(&self) -> f32 {
        (self.warm as u32 as f32 + self.greeted as u32 as f32) / 2.0
    }

    /// What the screen says it is doing. Names the outstanding wait rather
    /// than a generic "loading", for the reason `Progress::line` gives: a
    /// stage that reports nothing is a stage nobody can diagnose from a
    /// screenshot.
    pub fn line(&self) -> &'static str {
        match (self.warm, self.greeted) {
            (false, false) => "WARMING THE CLIENT",
            (false, true) => "BUILDING TEXTURES, ICONS AND MESHES",
            (true, false) => "ASKING THE LAUNCHER WHO IS PLAYING",
            (true, true) => "READY",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neither_wait_alone_opens_the_game() {
        let mut b = Boot::new(false);
        assert!(!b.done() && b.next().is_none());
        b.warm = true;
        assert!(!b.done(), "a warm client with no identity is not ready");
        b.warm = false;
        b.greeted = true;
        assert!(!b.done(), "an identity with a cold client is not ready");
    }

    #[test]
    fn the_chosen_bit_decides_which_screen_opens() {
        let ready = |chosen| Boot {
            warm: true,
            greeted: true,
            chosen,
        };
        assert_eq!(ready(false).next(), Some(Next::Menu));
        assert_eq!(ready(true).next(), Some(Next::Connect));
    }
}
