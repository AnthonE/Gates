# The browser's sound hiccups while running — the mechanism, read off the source

Dated 2026-09-13. A diagnosis, not a measurement: every row below carries the
file that produced it, and §4 says what has NOT been measured. Operator's
report: *"in the browser the running sound eventually causes the whole system
to kinda like hiccup like garbage collection almost but just the sound."*

## 1 · Our half is bounded and allocation-free, and it is not the problem

`crates/client/src/sound/` decides every level, cull and cadence, headless
(`tests/sound.rs`, ~90 tests). The mixer is fixed arrays end to end
(`sound/mixer.rs:82-106`): `VOICE_CAP = 24`, `STARTS_PER_FRAME = 4`,
`CUE_QUEUE_CAP = 32`, refuse-not-steal, drop-newest-and-count
(`sound/mod.rs:760-783`). A footstep is one cue per `STRIDE_M = 0.85 m`
(`sound/steps.rs:28`), so a sprint at `STEP_FULL_SPEED = 5.5 m/s` is ~6.5
starts a second locally, plus one odometer per drawn remote body
(`render/audio.rs:413-431`). **Running is the highest voice-start rate the
game has**, which is why it is the case that shows the defect first.

## 2 · Everything after `Mixer::tick` is not ours, and on the web it is the wrong shape

| layer | what happens per footstep | where |
|---|---|---|
| `render/audio.rs::pump` | spawns one entity: `AudioPlayer(handle)` + `PlaybackSettings { mode: Despawn, volume, speed, spatial }`, then a second command inserting its `Transform` | `crates/client/src/render/audio.rs:1101-1129` |
| `bevy_audio 0.18.1` | `play_queued_audio_system` calls `AudioSource::decoder()` → `rodio::Decoder::new(Cursor::new(bytes))`, a fresh WAV parser per play; `Sink::try_new`/`SpatialSink::try_new` per entity; `cleanup_finished_audio` despawns it when `sink.empty()` | upstream `crates/bevy_audio/src/audio_output.rs`, `audio_source.rs` (read 2026-09-13 at tag `v0.18.1`) |
| `rodio 0.20.1` | `Sink::append` wraps the source in `speed → track_position → pausable → amplify → skippable → stoppable → periodic_access(5 ms) → Done` — allocated per play; the sink's queue is created `keep_alive_if_empty = true` and stays in the mixer until the `Sink` drops; `DynamicMixer::next` runs `sum_current_sources` over **every** live source per sample, `add` pushes through a `Mutex<Vec<_>>` | upstream `src/sink.rs`, `src/dynamic_mixer.rs` (tag `v0.20.1`) |
| `cpal 0.15.3`, the ONLY wasm host | `host/webaudio/mod.rs`: an `AudioContext` created at the config's rate (44 100 forced, so Chrome resamples the whole graph to the device rate); output driven by **`set_timeout_with_callback_and_timeout_and_arguments_0` on the main thread**, two alternating workers, each period creating a new `web_sys::AudioBuffer` (`DEFAULT_BUFFER_SIZE = 2048` frames ≈ 46 ms at 44.1 k) **and** a new `AudioBufferSourceNode`, `copy_to_channel` from a `Vec<f32>`, `start_with_when(t)`. The data callback — the whole rodio mix — runs inside that timer | upstream `src/host/webaudio/mod.rs` (tag `v0.15.3`); the page names the same file for its `eval` probe (`crates/client-web/web/app.js:3-27`) |

Three consequences, and the operator's sentence names the third:

1. **A late timer is a gap.** The rAF frame, the sim step (`Session::pump`
   from `render/input.rs:443-450`), chunk builds and the audio timer share
   the tab's one thread. A frame longer than the period fires the timer late
   and the buffer is scheduled in the past. `findings/web-build-20260909.md`
   §4.1 measured the units that land on that thread natively — a near chunk
   heightfield 5.15 ms, a clutter fill 1.0 ms, the far mesh ~138 ms — and
   says the wasm numbers are worse and unmeasured. Running is what streams
   chunks.
2. **The mix itself runs on the main thread**, over up to 24 one-shot sinks
   + 3 beds + a music piece, each a chain of seven iterator adapters over a
   WAV decoder, at `[profile.web] opt-level = "s"` with no SIMD (root
   `Cargo.toml:47-58`; `render/audio.rs:201-204` already records "several
   times" native cost for the same arithmetic).
3. **The output path allocates JS garbage forever**: one `AudioBuffer`
   (2048 × 2 × 4 B = 16 KB) and one source node every 46 ms, for the life of
   the page, on top of the per-play Rust allocations above. Periodic JS GC
   is exactly what a player hears as "like garbage collection, but just the
   sound" — the frame keeps drawing, the audio thread's schedule does not.

## 3 · Why upgrading does not fix it

- cpal 0.17.0 (2025-12-20) added an `AudioWorklet` host, and its header says
  *"Requires atomics support"* — shared memory, which needs cross-origin
  isolation (COOP/COEP headers on `elopros.com`, an operator hosting act)
  **and** an atomics-enabled Bevy. `findings/web-build-20260909.md` §4.1
  measured that Bevy 0.18 loses threading at three `target_arch`-keyed
  compile-time sites that no header or `-Ctarget-feature` flips.
- bevy `main` (`bevy_audio 0.20.0-dev`) pins rodio 0.22 → a cpal whose
  default web host is still the timer loop; the worklet host is a feature
  behind the same atomics requirement. (Read 2026-09-13 off
  `raw.githubusercontent.com`; `CHANGELOG.md` for the version dates.)

So the fix is ours to build: a renderer that owns time on the audio thread —
natively as bevy_audio's one `Decodable` source, in the browser inside an
`AudioWorkletProcessor` running a second, small wasm module with no shared
memory, fed bounded commands over `postMessage`. That design is the approved
plan of 2026-09-13 (`NOW.md` §0x, §0web).

## 4 · What was NOT measured, and the instrument

Nothing in this note is a number from a browser: this box has no GPU, no
sound card, and no browser session against a shard. What the tree can
already print is `web::heap_report` (`crates/client/src/render/web.rs:287`) —
`dt`, the audio asset count and the wasm heap every 2 s. What it could not
see is main-thread stalls between those prints, so `app.js` now counts them:
a `PerformanceObserver` on `longtask` entries (tasks over 50 ms), printed as
a count and the longest one every five seconds, with the JS heap beside it
where the browser exposes it (`performance.memory`, Chrome only). A hiccup
that lines up with a long task is consequence 1 above; one that does not is
consequence 3. The `AudioContext`'s own `baseLatency` / `outputLatency` /
`state` are cpal's today and become the page's when the worklet lands, which
is when they get printed.
