//! The half of a gunshot that cannot wait for the shard (guns v1): the shot
//! the trigger just broke, predicted at the gun's own cadence, and the kick
//! it gives the view.
//!
//! **The kick is the recoil**, as Rust's is: the camera climbs and the player
//! pulls it back down. The shard aims every shot down the view the client
//! sends (`ranged::hitscan`), so a kick the player does not answer is a
//! spray that walks up the wall, and one they do is a tight group. Nothing
//! here decides a hit — the cone is the shard's, rolled where nobody can
//! choose it — this only moves the view the way the gun would and draws the
//! shot on the frame it is fired rather than a round trip later.
//!
//! Each shot kicks up between the gun's two pitch ends and sideways between
//! its two yaw ends (`protocol::GunRow`, from `weapons.toml`'s `[weapon.gun]`),
//! through the sights by `ads_recoil_pct` of that and crouched by a further
//! seventh off. An automatic's sideways kick **builds over the first shots of
//! a burst**, the way Rust's assault rifle drifts right as it climbs, so a
//! spray is a shape a hand can learn rather than noise. The kick is spent
//! over a few frames rather than in one, Rust's `timeToTake`, so the view
//! rides up instead of snapping.

use bevy::prelude::*;

use super::binds::{Action, Binds};
use super::input::Look;
use super::Net;
use protocol::GunRow;
use sim_core::input::{BTN_AIM, BTN_PRIMARY};

/// Seconds over which a shot's kick is mostly spent (Rust's `timeToTake`,
/// at most a tenth): the exponential's time constant, so ~95 % of the kick
/// lands in three of these.
pub const KICK_TAU_S: f32 = 0.03;

/// Shots over which an automatic's sideways drift builds to its full width.
pub const DRIFT_SHOTS: f32 = 6.0;

/// A crouch steadies the kick by this much (Rust halves it; this is gentler,
/// because a crouch is already the cone's own reward).
pub const CROUCH_KICK: f32 = 0.7;

/// The pitch the look may climb to, the input's own limit.
const PITCH_LIMIT: f32 = crate::look::PITCH_LIMIT;

/// What the local gun did this frame, for everything that draws or sounds a
/// shot of one's own (`fx::gun`, `audio`, `shake`, `viewmodel`).
#[derive(Resource, Default)]
pub struct GunFeel {
    /// Shots fired this frame (predicted). Almost always 0 or 1.
    pub fired: u8,
    /// The held gun, if the hand holds one.
    pub gun: Option<GunRow>,
    /// The held item's index.
    pub item: u16,
    /// Through the sights on this frame.
    pub ads: bool,
    /// Rounds fired in the current burst, for the drift and for anything
    /// that wants to know a spray from a tap.
    pub burst: u32,
    /// The view kick still owed, radians: `x` sideways (right), `y` up.
    kick: Vec2,
    /// The clock (seconds) the next shot may leave at.
    next_at: f64,
    /// Until when a reload holds the gun.
    reload_until: f64,
    /// When the last predicted shot left.
    last_at: f64,
    /// A semi-automatic's trigger came up since its last shot.
    armed: bool,
    /// Predicted shots the shard has not yet charged to the magazine.
    unconfirmed: u16,
    /// The magazine count last read, to see the shard's charges arrive.
    seen_loaded: u16,
    /// The kick's dice.
    rng: u64,
}

impl GunFeel {
    /// The next of the kick's dice, uniform in `[0, 1)`.
    fn roll(&mut self) -> f32 {
        self.rng = self
            .rng
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.rng >> 40) as f32 / 16_777_216.0
    }
}

/// The kick one shot gives, radians, `(right, up)`: the gun's pitch and yaw
/// ends at `u` and `v` across them, the sideways part grown over a burst on
/// an automatic, and the whole scaled by the sights and the crouch.
pub fn kick_of(g: &GunRow, burst: u32, ads: bool, crouched: bool, u: f32, v: f32) -> Vec2 {
    let cdeg = |c: f32| (c / 100.0).to_radians();
    let [p0, p1] = g.recoil_pitch.map(f32::from);
    let [y0, y1] = g.recoil_yaw.map(f32::from);
    let up = p0 + (p1 - p0) * u;
    let side = if g.semi {
        y0 + (y1 - y0) * v
    } else {
        // An automatic: the drift's centre grows over the first shots of a
        // burst and the jitter about it stays half the span.
        let grow = ((burst as f32 + 1.0) / DRIFT_SHOTS).min(1.0);
        (y0 + y1) * 0.5 * grow + (y1 - y0) * 0.5 * (v * 2.0 - 1.0) * 0.5
    };
    let mut k = if ads {
        f32::from(g.ads_recoil_pct) / 100.0
    } else {
        1.0
    };
    if crouched {
        k *= CROUCH_KICK;
    }
    Vec2::new(cdeg(side), cdeg(up)) * k
}

/// Predict the local gun's shots and spend their kick on the view.
///
/// After `input::gather`, which has just decided what the trigger sends
/// (`ClientCore::buttons`), and before the feed drains, so every reader of
/// [`GunFeel`] sees this frame's shot.
#[allow(clippy::too_many_arguments)]
pub fn predict(
    time: Res<Time>,
    net: Option<NonSend<Net>>,
    mut look: ResMut<Look>,
    mut gf: ResMut<GunFeel>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    binds: Res<Binds>,
) {
    gf.fired = 0;
    let now = time.elapsed_secs_f64();
    let dt = time.delta_secs();
    // The owed kick, spent whatever the hand now holds: a gun dropped
    // mid-climb still finishes the climb it started.
    if gf.kick != Vec2::ZERO {
        let take = gf.kick * (1.0 - (-dt / KICK_TAU_S).exp());
        gf.kick -= take;
        if gf.kick.length_squared() < 1e-12 {
            gf.kick = Vec2::ZERO;
        }
        if !look.frozen {
            look.pitch = (look.pitch + take.y).clamp(-PITCH_LIMIT, PITCH_LIMIT);
            // The wire's yaw turns toward +X as it grows, which is to the
            // left of a body facing +Z (`look.rs`): a kick right is a fall.
            look.yaw -= take.x;
        }
    }
    let Some(net) = net else {
        return;
    };
    let core = &net.session.core;
    let sel = net.sel as usize;
    let item = core
        .inv
        .get(sel)
        .filter(|s| s.count > 0)
        .map_or(u16::MAX, |s| s.item);
    let gun = (item != u16::MAX)
        .then(|| core.catalog.row(item as usize).gun)
        .flatten();
    if item != gf.item {
        // A new hand: the burst, the trigger and the magazine's ledger
        // start over, and nothing fires on the frame it comes up.
        gf.item = item;
        gf.burst = 0;
        gf.armed = false;
        gf.unconfirmed = 0;
        gf.seen_loaded = core.mag().0;
        gf.next_at = gf.next_at.max(now);
    }
    gf.gun = gun;
    let buttons = core.buttons();
    gf.ads = buttons & BTN_AIM != 0;
    let Some(g) = gun else {
        return;
    };
    if gf.rng == 0 {
        gf.rng = 0x9E37_79B9_7F4A_7C15 ^ (now.to_bits());
    }

    // The shard's charges, arriving: each round it took off the magazine is
    // one of ours confirmed; a fill clears the ledger.
    let (loaded, _cap) = core.mag();
    if loaded < gf.seen_loaded {
        gf.unconfirmed = gf.unconfirmed.saturating_sub(gf.seen_loaded - loaded);
    } else if loaded > gf.seen_loaded {
        gf.unconfirmed = 0;
    }
    gf.seen_loaded = loaded;

    // A reload holds the gun for its whole length, from the press.
    if binds.just_pressed(Action::Reload, &keys, &mouse) {
        gf.reload_until = now + f64::from(g.reload_ticks) / f64::from(sim_core::limits::TICK_HZ);
        gf.unconfirmed = 0;
    }

    let trigger = buttons & BTN_PRIMARY != 0;
    if !trigger {
        gf.armed = true;
    }
    let interval = f64::from(g.rate_ticks.max(1)) / f64::from(sim_core::limits::TICK_HZ);
    // A burst ends when the trigger has rested two shots' worth.
    if now - gf.last_at > interval * 2.0 {
        gf.burst = 0;
    }
    let ready = now >= gf.next_at && now >= gf.reload_until;
    let loaded_left = loaded > gf.unconfirmed;
    if trigger && ready && loaded_left && (!g.semi || gf.armed) {
        gf.fired = 1;
        gf.armed = false;
        gf.unconfirmed += 1;
        // Paid from when it was due, not from this frame, so a frame rate
        // that does not divide the cadence still fires at the gun's rate.
        gf.next_at = if now - gf.next_at < interval {
            gf.next_at + interval
        } else {
            now + interval
        };
        gf.last_at = now;
        let (u, v) = (gf.roll(), gf.roll());
        let k = kick_of(&g, gf.burst, gf.ads, core.crouched(), u, v);
        gf.kick += k;
        gf.burst += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rifle() -> GunRow {
        GunRow {
            rate_ticks: 4,
            reload_ticks: 132,
            semi: false,
            pellets: 1,
            aim: 20,
            hip: 250,
            recoil_pitch: [180, 220],
            recoil_yaw: [-20, 80],
            ads_recoil_pct: 50,
        }
    }

    /// Through the sights the kick is the gun's share of the hip's, and a
    /// crouch takes its seventh off on top.
    #[test]
    fn the_sights_and_the_crouch_steady_the_kick() {
        let g = rifle();
        let hip = kick_of(&g, 10, false, false, 0.5, 0.5);
        let ads = kick_of(&g, 10, true, false, 0.5, 0.5);
        let low = kick_of(&g, 10, true, true, 0.5, 0.5);
        assert!((ads.y - hip.y * 0.5).abs() < 1e-6);
        assert!((low.y - ads.y * CROUCH_KICK).abs() < 1e-6);
        assert!(hip.y > 0.0, "a kick climbs");
    }

    /// An automatic's drift builds over the burst toward the side its yaw
    /// range leans to; the first shot is nearly straight up.
    #[test]
    fn an_automatic_drifts_as_the_burst_builds() {
        let g = rifle();
        let first = kick_of(&g, 0, false, false, 0.5, 0.5);
        let later = kick_of(&g, 20, false, false, 0.5, 0.5);
        assert!(later.x > first.x * 3.0, "{first:?} → {later:?}");
        assert!(later.x > 0.0, "this rifle's drift is to the right");
    }
}
