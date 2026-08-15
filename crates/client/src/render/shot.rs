//! The screenshot key: `F12`, on every screen, all run long.
//!
//! **Why this exists** is `crate::shot`'s header in one line: on Linux the
//! desktop's own screenshot key is not one thing, and under Wayland a client
//! cannot read another client's pixels at all without a portal the box may not
//! have. A game that reads its own swapchain is blind to every branch of that,
//! which is the point.
//!
//! It is deliberately NOT the capture harness. `render/capture.rs` is a probe:
//! fixed seed, fixed spawn, six fixed bearings, exit when done. This is a
//! player pressing a key at a moment they chose, so it settles nothing, waits
//! for nothing, and never touches the view. The two share only Bevy's
//! `Screenshot` entity.
//!
//! **The announcement is immediate and the verification is late**, which is
//! the one non-obvious shape here. `save_to_disk` handles an IO error with
//! `error!(…)` and returns — there is no path back to the caller, so a
//! screenshot that never reaches disk is silent (`render/capture.rs` says the
//! same thing and re-checks the files for the same reason). A toast written
//! only after a successful `stat` would lag the keypress by the readback, so
//! the press is announced at once and a failure is announced *again*, as an
//! alarm, when the check comes back. Silence after the first line means it
//! worked.

use bevy::prelude::*;
use bevy::render::view::screenshot::{save_to_disk, Screenshot};
use std::path::PathBuf;

use crate::shot;

/// The key. `F12` because it is what a player already reaches for and because
/// this client binds no other function key at all (grep `KeyCode::F` — the
/// hotbar is the digits and everything else is a letter), so it collides with
/// nothing that exists and nothing that is planned.
pub const KEY: KeyCode = KeyCode::F12;

/// Frames between the shot and the check that it landed. A frame count rather
/// than a duration, for `capture.rs`'s reason: the readback is asynchronous in
/// *frames*, and elapsed milliseconds measure the box rather than the client.
/// The same 20 the capture harness holds for its own tail, which is the only
/// number here anything has ever measured.
pub const VERIFY_FRAMES: u32 = 20;

/// How many shots may be awaiting their check at once.
///
/// Wall 4 wants a cap and a stated policy on a player-driven path. The policy
/// is **refuse the new shot and say so**, and it is effectively unreachable:
/// `just_pressed` does not repeat, so a player cannot press this more than a
/// dozen times a second, and each entry clears in [`VERIFY_FRAMES`].
pub const PENDING_CAP: usize = 8;

/// Where the shots go, and which ones are still unverified.
#[derive(Resource)]
pub struct Shots {
    /// `None` when the environment named no base — see [`shot::dir`]. Resolved
    /// **once**, at startup, so the key does not read the environment on the
    /// frame it is pressed and so the "no home directory" complaint is
    /// logged once rather than per press.
    pub dir: Option<PathBuf>,
    pending: [(PathBuf, u32); PENDING_CAP],
    n_pending: usize,
    frame: u32,
    /// Shots taken this run, counting only the ones that reached [`spawn`].
    pub taken: u32,
}

impl Shots {
    /// Aimed at a directory, or at nothing. The environment is read by
    /// [`Default`] and not here, so a test can point this somewhere without
    /// touching the process's real (and test-shared) environment —
    /// `config`'s own testing posture, one crate over.
    pub fn new(dir: Option<PathBuf>) -> Self {
        Self {
            dir,
            pending: std::array::from_fn(|_| (PathBuf::new(), 0)),
            n_pending: 0,
            frame: 0,
            taken: 0,
        }
    }

    /// Shots taken but not yet checked. Read by the tests; wall 4 wants the
    /// cap it is measured against to be observable.
    pub fn pending(&self) -> usize {
        self.n_pending
    }
}

impl Default for Shots {
    fn default() -> Self {
        let dir = shot::dir();
        if dir.is_none() {
            warn!(
                "gates: no home directory in the environment, so the {KEY:?} screenshot key is \
                 off for this run — set GATES_SHOTS_DIR to name a directory"
            );
        }
        Self::new(dir)
    }
}

/// One system: take the shot on the press, and re-check the ones already out.
///
/// Both halves live here because they share the frame counter and the pending
/// list, and splitting them would be two systems ordered against each other
/// over one resource for no gain.
pub fn take(
    mut commands: Commands,
    mut shots: ResMut<Shots>,
    mut toast: ResMut<super::hud::Toast>,
    keys: Res<ButtonInput<KeyCode>>,
) {
    shots.frame = shots.frame.wrapping_add(1);
    verify(&mut shots, &mut toast);

    if !keys.just_pressed(KEY) {
        return;
    }
    let Some(dir) = shots.dir.clone() else {
        // Already warned once at startup; a press deserves an answer on
        // screen rather than a second line in a log the player is not reading.
        toast.warn("no screenshot directory — set GATES_SHOTS_DIR");
        return;
    };
    if shots.n_pending >= PENDING_CAP {
        toast.warn("screenshot: too many still saving");
        return;
    }
    if let Err(e) = std::fs::create_dir_all(&dir) {
        error!("gates: cannot create {}: {e}", dir.display());
        toast.warn("screenshot failed — see the log");
        return;
    }
    let stamp = shot::stamp(shot::now_secs());
    let Some(path) = shot::pick(&dir, &stamp, |p| p.exists()) else {
        // `shot::PER_STAMP` names spent inside one second.
        toast.warn("screenshot: too many this second");
        return;
    };

    info!("gates: screenshot → {}", path.display());
    // Announced now, not after the check — see the module header. The name
    // alone, because the directory is the same every time and a full path
    // across the screen is a toast nobody can read.
    toast.say(format!(
        "screenshot  {}",
        path.file_name().unwrap_or_default().to_string_lossy()
    ));

    commands
        .spawn(Screenshot::primary_window())
        .observe(save_to_disk(path.clone()));
    shots.taken += 1;

    let at = shots.frame;
    let n = shots.n_pending;
    shots.pending[n] = (path, at);
    shots.n_pending += 1;
}

/// Check every shot whose [`VERIFY_FRAMES`] are up, and complain about the
/// ones that are not on disk. Kept out of `take` only to keep that function
/// readable; it is not a system.
fn verify(shots: &mut Shots, toast: &mut super::hud::Toast) {
    let frame = shots.frame;
    let mut i = 0;
    while i < shots.n_pending {
        if frame.wrapping_sub(shots.pending[i].1) < VERIFY_FRAMES {
            i += 1;
            continue;
        }
        // Taken out rather than borrowed, so the removal below is not fighting
        // a live borrow of the slot it is removing.
        let path = std::mem::take(&mut shots.pending[i].0);
        // `is_file` as well as non-empty: a directory reports a non-zero
        // length, so a size check alone would accept one (`capture.rs`).
        let landed = matches!(
            std::fs::metadata(&path),
            Ok(m) if m.is_file() && m.len() > 0
        );
        if !landed {
            error!(
                "gates: the screenshot never reached disk: {}",
                path.display()
            );
            toast.warn("screenshot failed to save");
        }
        // Swap-remove. Order does not matter — every entry carries the frame
        // it was taken on, so each is checked on its own schedule. `i` is not
        // advanced: the entry swapped into this slot has not been looked at.
        shots.n_pending -= 1;
        let last = shots.n_pending;
        shots.pending.swap(i, last);
        shots.pending[last].1 = 0;
    }
}
