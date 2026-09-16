//! The browser client's audio output: the engine inside an `AudioWorklet`.
//!
//! The page's twin of `render/audio_out.rs`, and deliberately the same shape:
//! `render/audio.rs` decides nothing and neither does this file — it is the
//! seam between the game thread's [`Engine`](super::audio::Engine) buffer and
//! a `sound::engine::Renderer` running on the browser's audio thread, and it
//! is exactly two moves: [`flush`] posts the frame's installs and commands in
//! `PostUpdate`, and `crates/client-web/web/audio.js` hands them to
//! `sound::worklet` inside `process()`. Everything between is
//! `crates/sound/src/worklet.rs`, gated headless in
//! `crates/sound/tests/worklet.rs`.
//!
//! **What replaced what.** Until 2026-09-14 every cue on this target was a
//! `bevy_audio` entity — a rodio decoder and a sink per play, summed inside
//! cpal's only wasm host, which is a `setTimeout` loop **on the main thread**
//! that allocates a fresh `AudioBuffer` and source node every ~46 ms
//! (`findings/browser-audio-20260913.md`). The cpal seam removed that path on
//! both targets and gave this one nothing, so `render/mod.rs` registered
//! `audio::clear` — a system whose whole job was to empty the command buffer
//! so it could not count phantom drops. **The browser has been silent since,
//! with every gate green**, which is `CLAUDE.md`'s decal trap exactly: a path
//! switched off for one target, the reason written above it, reading as
//! coverage. This is the reader that buffer was always missing.
//!
//! **Why the page opens the context and not this file.** An `AudioContext`
//! created outside a user gesture starts `suspended`, and a `suspended`
//! context is silence with no error anywhere — the same failure shape again.
//! The page has a PLAY button, so `app.js` creates the context, loads the
//! processor and builds the node inside that click, where the gesture is a
//! fact rather than a hope, and leaves the result on `globalThis.gatesAudio`.
//! It also means the two async steps a worklet needs — `addModule` and
//! `WebAssembly.compile` — finish before Bevy's plugin builds, so nothing
//! here is async and there is no frame where the game is running and the
//! audio thread is not.
//!
//! **A page with no worklet** — an older browser, a refused `addModule`, a
//! module that failed to compile — is the case this file is written for
//! first, and it is the same case `audio_out.rs` calls a box with no device:
//! [`open`] finds no `gatesAudio`, says so ONCE, and [`flush`] then drops
//! every command and install and counts them ([`Web::unrouted`]) rather than
//! posting into nothing. `audio::pump` gates its warnings on
//! [`Diag::alive`](super::audio::Diag), which stays false until the processor
//! has answered.

#![cfg(target_arch = "wasm32")]

use std::cell::Cell;
use std::rc::Rc;

use bevy::prelude::*;
use js_sys::{Int16Array, Object, Reflect, Uint8Array};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{MessageEvent, MessagePort};

use crate::sound::engine::{Stats, CMD_BYTES};
use crate::sound::worklet::{encode, encode_cap, Report, REPORT_BYTES};
use crate::sound::{Cue, SAMPLE_RATE};

use super::audio::{self, CMD_FRAME_CAP};

/// Where the page leaves the node it built inside the PLAY click.
const HANDOFF: &str = "gatesAudio";

/// What the processor posts back, as the page's side sees it.
#[derive(Default)]
struct Inbox {
    /// The last report decoded, if one has arrived since the flush read it.
    report: Cell<Option<Report>>,
    /// Has the processor answered at all? Until it has, the audio thread is
    /// not alive and `audio::pump` must not warn about counters nobody has
    /// filled in.
    ready: Cell<bool>,
    /// Messages that were not a report this side understands — counted
    /// rather than ignored, because a silent mismatch between the two halves
    /// of a protocol is the thing this repo keeps paying for.
    bad: Cell<u32>,
}

/// The game thread's end of the browser seam.
///
/// **Non-send**, for `Native`'s reason one target over: a `MessagePort` and a
/// `Closure` are `JsValue`s, which are neither `Send` nor `Sync`, while a
/// Bevy `Resource` has to be both. `NonSend` pins [`flush`] to the main
/// thread, which is the only thread a tab has.
pub struct Web {
    port: Option<MessagePort>,
    /// The encode buffer, allocated once at [`encode_cap`] of a frame's cap.
    /// Reused every flush so a frame of commands costs no allocation here —
    /// the `Uint8Array` that crosses is a view over the bytes that did fit.
    buf: Vec<u8>,
    inbox: Rc<Inbox>,
    /// Kept alive because dropping a `Closure` unregisters the callback and
    /// the reports stop with nothing to say so.
    _on_message: Option<Closure<dyn FnMut(MessageEvent)>>,
    /// Is there a processor to post to? False is the no-worklet state, where
    /// [`flush`] drops and counts.
    routed: bool,
    /// Commands and installs dropped at the flush for want of an output.
    pub unrouted: u32,
    /// Installs the page refused to post — it cannot happen on the shipped
    /// path and is counted rather than assumed.
    pub install_refused: u32,
    /// The renderer's rate: the `AudioContext`'s, or [`SAMPLE_RATE`] with no
    /// context.
    pub out_rate: u32,
}

impl Web {
    /// The device-less state, said once.
    fn unrouted(why: &str) -> Web {
        info!("audio output: none - {why}; commands are dropped at the flush and counted");
        Web {
            port: None,
            buf: Vec::new(),
            inbox: Rc::new(Inbox::default()),
            _on_message: None,
            routed: false,
            unrouted: 0,
            install_refused: 0,
            out_rate: SAMPLE_RATE,
        }
    }

    /// Hand a cue's samples across. The `Int16Array` is built here and its
    /// buffer **transferred**, so the samples are copied once (out of the
    /// wasm heap into a JS array) rather than twice.
    fn install(&mut self, cue: Cue, pcm: &[i16]) -> bool {
        let Some(port) = self.port.as_ref() else {
            return false;
        };
        let samples = Int16Array::from(pcm);
        let msg = Object::new();
        let ok = Reflect::set(&msg, &"kind".into(), &"install".into()).is_ok()
            && Reflect::set(&msg, &"cue".into(), &(cue.idx() as u32).into()).is_ok()
            && Reflect::set(&msg, &"pcm".into(), &samples).is_ok();
        if !ok {
            return false;
        }
        let transfer = js_sys::Array::of1(&samples.buffer());
        port.post_message_with_transferable(&msg, &transfer).is_ok()
    }

    /// Post this frame's commands as one batch of 16-byte records. Nothing is
    /// posted for a frame with no commands, which is most of them — a message
    /// per silent frame is 60 allocations a second on the audio thread for
    /// nothing.
    fn send(&mut self, n: usize) -> bool {
        if n == 0 {
            return true;
        }
        let Some(port) = self.port.as_ref() else {
            return false;
        };
        let bytes = Uint8Array::from(&self.buf[..n]);
        let msg = Object::new();
        let ok = Reflect::set(&msg, &"kind".into(), &"cmds".into()).is_ok()
            && Reflect::set(&msg, &"bytes".into(), &bytes).is_ok();
        if !ok {
            return false;
        }
        let transfer = js_sys::Array::of1(&bytes.buffer());
        port.post_message_with_transferable(&msg, &transfer).is_ok()
    }

    /// Is a processor rendering? False until it has answered `ready`.
    pub fn alive(&self) -> bool {
        self.inbox.ready.get()
    }

    /// Is there anywhere to post?
    pub fn routed(&self) -> bool {
        self.routed
    }

    /// Reports that did not decode.
    pub fn bad_reports(&self) -> u32 {
        self.inbox.bad.get()
    }
}

/// Find the node the page built, wire the report channel, and tell
/// [`audio::Engine`] the context's rate.
///
/// The rate is the load-bearing half: `audio::pump` builds every `Start`'s
/// playback rate against `Engine::out_rate`, and a browser's `AudioContext`
/// runs at whatever the device does — 48 kHz on most, 44.1 on some. Reading
/// it here rather than assuming [`SAMPLE_RATE`] is what keeps
/// `engine::rate`'s resample the only one in the chain, exactly as `open`
/// does with cpal's device rate natively.
pub fn open(app: &mut App) -> Web {
    let Some(window) = web_sys::window() else {
        return Web::unrouted("no window");
    };
    let handoff = match Reflect::get(&window, &JsValue::from_str(HANDOFF)) {
        Ok(v) if !v.is_undefined() && !v.is_null() => v,
        _ => {
            return Web::unrouted(
                "the page built no AudioWorklet (globalThis.gatesAudio is absent)",
            )
        }
    };
    let port = Reflect::get(&handoff, &JsValue::from_str("port"))
        .ok()
        .and_then(|v| v.dyn_into::<MessagePort>().ok());
    let Some(port) = port else {
        return Web::unrouted("gatesAudio carries no MessagePort");
    };
    let rate = Reflect::get(&handoff, &JsValue::from_str("rate"))
        .ok()
        .and_then(|v| v.as_f64())
        .filter(|r| *r >= 1.0 && *r <= 768_000.0)
        .map(|r| r as u32)
        .unwrap_or(SAMPLE_RATE);

    let inbox = Rc::new(Inbox::default());
    let sink = Rc::clone(&inbox);
    // Every message the processor sends arrives here. A report is `REPORT_BYTES`
    // of `sound::worklet::Report`; anything else is counted, because a
    // protocol whose two halves silently disagree is what `CLAUDE.md`'s
    // byte-golden entry is about.
    let on_message = Closure::<dyn FnMut(MessageEvent)>::new(move |e: MessageEvent| {
        let data = e.data();
        let kind = Reflect::get(&data, &JsValue::from_str("kind"))
            .ok()
            .and_then(|v| v.as_string());
        match kind.as_deref() {
            Some("ready") => sink.ready.set(true),
            Some("report") => {
                let bytes = Reflect::get(&data, &JsValue::from_str("bytes"))
                    .ok()
                    .and_then(|v| v.dyn_into::<Uint8Array>().ok());
                match bytes {
                    Some(b) if b.length() as usize == REPORT_BYTES => {
                        let mut rec = [0u8; REPORT_BYTES];
                        b.copy_to(&mut rec);
                        sink.report.set(Some(Report::from_bytes(&rec)));
                        sink.ready.set(true);
                    }
                    _ => sink.bad.set(sink.bad.get().saturating_add(1)),
                }
            }
            _ => sink.bad.set(sink.bad.get().saturating_add(1)),
        }
    });
    port.set_onmessage(Some(on_message.as_ref().unchecked_ref()));

    app.world_mut().resource_mut::<audio::Engine>().out_rate = rate;
    info!("audio output: {rate} Hz, AudioWorklet, the engine at the context's rate");
    Web {
        port: Some(port),
        buf: vec![0u8; encode_cap(CMD_FRAME_CAP)],
        inbox,
        _on_message: Some(on_message),
        routed: true,
        unrouted: 0,
        install_refused: 0,
        out_rate: rate,
    }
}

/// Move the frame's traffic across the seam, `PostUpdate` — `audio_out::flush`'s
/// twin, in the same order and with the same refusal policy: installs first
/// (so a command in the same batch can play what just landed), then the
/// commands, then whatever the processor last reported into
/// [`audio::Diag`](super::audio::Diag).
///
/// With no processor nothing is posted and the frame's traffic is dropped and
/// counted, for the reason the native side does not fill a ring nobody
/// drains: a queue with no reader would fill and then misreport every
/// command as an overflow, which is the wrong fact.
pub fn flush(mut engine: ResMut<audio::Engine>, mut out: NonSendMut<Web>) {
    if out.routed {
        // Collected rather than drained in place so the install's borrow of
        // `engine` ends before the post; the box is dropped here either way,
        // which is where it was allocated — the browser seam has no return
        // ring to reclaim over, because `postMessage` copies and nothing the
        // processor holds is ours to free.
        //
        // **This allocates only while the bank is landing.** `SPREAD_BANK`
        // hands over one cue a frame, so it is a one-element `Vec` on each of
        // `CUE_COUNT` frames at boot and an empty `collect` — which does not
        // allocate — on every frame after. The steady state is what the
        // no-allocation claim above is about, and it holds.
        let pending: Vec<(Cue, Box<[i16]>)> = engine.take_installs().collect();
        for (cue, pcm) in pending {
            if !out.install(cue, &pcm) {
                out.install_refused = out.install_refused.saturating_add(1);
            }
        }
        // The buffer is taken and put back so the encode can borrow it while
        // `out` is borrowed for the post; it keeps its one allocation either
        // way. **It always fits**: `buf` is `encode_cap(CMD_FRAME_CAP)` and
        // `Engine::push` refuses past `CMD_FRAME_CAP`, so `encode` never
        // reaches its own bound and no command is silently left unwritten.
        let mut buf = core::mem::take(&mut out.buf);
        let n = encode(engine.take(), &mut buf);
        debug_assert!(n <= buf.len(), "the frame buffer outgrew the encode buffer");
        out.buf = buf;
        if !out.send(n) {
            out.unrouted = out.unrouted.saturating_add((n / CMD_BYTES) as u32);
        }
    } else {
        let n = engine.take_installs().count() + engine.take().count();
        out.unrouted = out.unrouted.saturating_add(n as u32);
    }

    if let Some(report) = out.inbox.report.take() {
        engine.diag.stats = report.stats;
        // The two counters that are the browser seam's own ride the same
        // record. `bad_cmd` is the renderer's name for a command it refused
        // as malformed, and a record this side could not decode is the same
        // fault one layer out, so they are summed rather than given a field
        // on `Diag` that only one target could ever fill.
        engine.diag.stats.bad_cmd = engine
            .diag
            .stats
            .bad_cmd
            .saturating_add(report.bad_records)
            .saturating_add(report.bad_installs);
        engine.diag.live_thread = report.stats.live;
    }
    let diag = &mut engine.diag;
    diag.alive = out.alive();
    diag.routed = out.routed();
    diag.unrouted = out.unrouted;
    diag.install_refused = out.install_refused;
    diag.stream_errors = out.bad_reports();
    // No ring and no backlog on this target: `postMessage` has no cap we can
    // read, and the renderer's own queue overflow is `stats.dropped`, which
    // already has a reader. Left at zero rather than filled with a guess.
    diag.ring_dropped = 0;
    diag.backlog = 0;
}

const _: () = assert!(REPORT_BYTES > 0);
const _: () = assert!(core::mem::size_of::<Stats>() > 0);
/// A frame's commands must fit one batch, or `encode` would drop the tail
/// with nothing counting it. Both sides are `const`, so this is checkable
/// here rather than asserted at runtime.
const _: () = assert!(encode_cap(CMD_FRAME_CAP) >= CMD_FRAME_CAP * CMD_BYTES);
