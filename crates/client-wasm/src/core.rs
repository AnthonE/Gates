//! The whole client, composed: snapshot view, own-capsule predictor,
//! remote interpolation, and the dilated client clock. Pure — no I/O, no
//! wall clock. The host (a native test or the wasm bridge) feeds incoming
//! datagram bytes, real milliseconds, and the current input state, and
//! reads render state back. The server's integration gate drives this
//! exact struct against `ShardCore`; the browser drives it through
//! `bridge.rs`.

use crate::clock::ClientClock;
use crate::interp::Interp;
use crate::predict::Predictor;
use crate::view::{Applied, ClientView};
use protocol::{encode_input, InputDatagram};
use sim_core::input::InputFrame;

/// What `on_datagram` did with the bytes (the bridge's status code).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum Ingest {
    Error = 0,
    Applied = 1,
    AppliedDelta = 2,
    Stale = 3,
    NoBaseline = 4,
}

#[derive(Clone, Copy, Default)]
struct InputState {
    buttons: u8,
    yaw: u16,
    pitch: u8,
    move_x: i8,
    move_z: i8,
}

pub struct ClientCore {
    pub player_id: u32,
    pub view: ClientView,
    pub predict: Predictor,
    pub interp: Interp,
    pub clock: ClientClock,
    input: InputState,
    next_seq: u16,
    input_due: bool,
    pub snapshots_applied: u64,
    pub snapshots_delta: u64,
    pub snapshots_stale: u64,
    pub snapshots_no_baseline: u64,
    pub decode_errors: u64,
}

impl ClientCore {
    pub fn new(seed: u64, player_id: u32, server_tick: u32) -> Self {
        Self {
            player_id,
            view: ClientView::new(),
            predict: Predictor::new(seed),
            interp: Interp::new(),
            clock: ClientClock::new(server_tick),
            input: InputState::default(),
            next_seq: 1,
            input_due: false,
            snapshots_applied: 0,
            snapshots_delta: 0,
            snapshots_stale: 0,
            snapshots_no_baseline: 0,
            decode_errors: 0,
        }
    }

    /// The live input state; sampled once per generated frame.
    pub fn set_input(&mut self, buttons: u8, yaw: u16, pitch: u8, move_x: i8, move_z: i8) {
        self.input = InputState {
            buttons,
            yaw,
            pitch,
            move_x,
            move_z,
        };
    }

    /// Advance real time: run the fixed client ticks that elapsed, each
    /// generating one input frame and stepping prediction. Returns steps.
    pub fn advance(&mut self, dt_ms: f64) -> u32 {
        let steps = self.clock.advance(dt_ms);
        for _ in 0..steps {
            let frame = InputFrame {
                seq: self.next_seq,
                buttons: self.input.buttons,
                yaw: self.input.yaw,
                pitch: self.input.pitch,
                move_x: self.input.move_x,
                move_z: self.input.move_z,
            };
            self.next_seq = self.next_seq.wrapping_add(1);
            self.clock.client_tick = self.clock.client_tick.wrapping_add(1);
            self.predict.step(frame);
            self.input_due = true;
        }
        steps
    }

    /// Encode the due input datagram — the unacked tail plus the redundant
    /// ack header — into `buf`. Returns 0 when none is due. One datagram
    /// per client tick (30 Hz): the tail already carries the loss cover,
    /// and the upstream budget (DESIGN.md §5.3) prefers 30 to a 144 Hz
    /// render loop's worth of duplicates.
    pub fn poll_input(&mut self, buf: &mut [u8]) -> usize {
        if !self.input_due {
            return 0;
        }
        self.input_due = false;
        let (ack, ack_bits) = self.view.ack_fields();
        let tail = self.predict.tail();
        let first_tick = self.clock.client_tick.wrapping_sub(tail.len() as u32);
        let mut dg = InputDatagram::new(ack, ack_bits, first_tick);
        for f in tail {
            if dg.push(*f).is_err() {
                break; // tail is wire-capped already; defensive only
            }
        }
        encode_input(&dg, buf).unwrap_or(0)
    }

    /// One incoming datagram: apply through the view, then feed the clock,
    /// the interpolation history, and the predictor.
    pub fn on_datagram(&mut self, bytes: &[u8]) -> Ingest {
        match self.view.apply(bytes) {
            Ok(Applied::Ok { delta }) => {
                self.snapshots_applied += 1;
                if delta {
                    self.snapshots_delta += 1;
                }
                let Some(snap) = self.view.newest() else {
                    return Ingest::Error; // unreachable: just applied
                };
                let header = snap.header;
                if header.baseline_age == 0 {
                    self.interp.clear();
                }
                for &id in snap.removed() {
                    self.interp.remove(id);
                }
                for e in snap.entities() {
                    if e.id != self.player_id {
                        self.interp.push(header.tick, e);
                    }
                }
                self.clock.on_snapshot(header.tick, header.nudge);
                if let Some(own) = self.view.get(self.player_id).copied() {
                    self.predict.reconcile(&own, header.last_executed_seq);
                }
                if delta {
                    Ingest::AppliedDelta
                } else {
                    Ingest::Applied
                }
            }
            Ok(Applied::Stale) => {
                self.snapshots_stale += 1;
                Ingest::Stale
            }
            Ok(Applied::NoBaseline) => {
                self.snapshots_no_baseline += 1;
                Ingest::NoBaseline
            }
            Err(_) => {
                self.decode_errors += 1;
                Ingest::Error
            }
        }
    }

    /// The float server tick remote entities render at.
    pub fn render_tick(&self) -> f64 {
        self.clock.server_est - crate::interp::INTERP_DELAY_TICKS
    }
}
