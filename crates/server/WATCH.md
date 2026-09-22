# One shared bot viewpoint

Run from the repository root with the normal Gates render assets staged:

```sh
cargo run -p server --features watch --bin jev-watch -- --scripted
```

Open **http://127.0.0.1:8081/**. Every viewer watches the same bot, camera and
run. The page shows its received health and inventory, controller activity,
trees completed and elapsed play time. Viewing creates no game connection or
model request. `--assets PATH` selects an existing asset directory;
`--listen 127.0.0.1:PORT` changes the web listener.

The command needs a desktop graphics display. On a headless Linux render
host, use Xvfb:

```sh
xvfb-run -a -s '-screen 0 640x360x24' \
  cargo run -p server --features watch --bin jev-watch -- --scripted
```

For Jev, set `TYPESAFE_API_KEY` in the host environment and omit `--scripted`.
The page identifies the controller explicitly; an API failure never switches
to scripted decisions. No key, prompt or model response is sent to viewers.
See [JEV.md](JEV.md) for the movement model and the local wood-collecting
controller. The policy is still rough: it can get stuck or die, and cannot
craft, eat, respawn or reconnect.

The temporary shard uses shipped content, a game-selected beach and no saves.
The run lasts 120 seconds **after the world loads**; `--seconds N` accepts
1–86400. A startup that exceeds 300 seconds fails. The watcher closes when
the duration ends or its session fails; Ctrl-C also ends it and the temporary
shard. Death is shown on the normal game screen until the run ends.

## What carries the picture

`jev-watch` embeds the existing Rust `GatesRenderPlugin` and ordinary client
`Session`. The bot supplies input before the client's normal pump, prediction
and camera update. One bounded observer copies accepted reliable messages to
the collector, which uses the same received inventory and procedural scenery
as a player. If that observer overflows or rejects an event, both client
output lanes stop. This is the bot's own first-person session.

The renderer targets 30 fps at 640×360. At most four frames per second are
captured as JPEGs; actual delivery depends on render speed. This prototype
has no audio or smooth video encoding. One pending GPU readback and one
queued image bound the capture work. JPEG conversion and HTTP live on a
separate worker, and viewers share the last encoded frame. Slow viewers
cannot queue more captures or control the player.

The loopback origin serves only the page, its CSS/JS, `/state.json` and
`/frame.jpg`. Frame IDs bind the image to the displayed state; a superseded
ID returns 409 and the page retries. The page marks frames older than five
seconds as stalled. HTTP has a 16-connection ceiling, an 8 KiB header bound
and a five-second request deadline; excess connections close. These are
prototype limits, not a public audience capacity claim. All experimental
delivery defaults are registered in `DECISIONS.md` §Open.

## Website handoff

The standalone page is ready for a same-origin website route, for example
`/games/gates/watch/`. A web proxy must strip that prefix when forwarding to
the loopback origin, preserve the trailing slash, and leave frame/state
responses uncached. Relative asset/API URLs support that mount. It can also
be embedded by a page on the same origin; its CSP refuses other origins.

Public hosting still needs an operator-selected render host, a supervised
run/recovery policy, HTTPS routing and an audience/load check. The current
command is a finite local experiment, and a CPU-only host does not establish
video throughput. No public website, shard, certificate or domain is changed
by this PR. Publishing follows `CLAUDE.md` §The loop discipline.

## Checked locally

`cargo test -p server --features watch --lib watch::` exercises harvesting
through the ordinary client session over real WebTransport, including held
input between client ticks and agreement with the received inventory. It
also checks that observer failure stops outgoing traffic, that two viewers
receive identical encoded frames, and that an obsolete frame ID is refused.

Manual Chromium checks cover desktop and mobile layout, real renderer
frames, no horizontal overflow, and the stalled indicator after losing the
feed. A local Xvfb/llvmpipe run on 2026-09-22 reached the world and moved with
zero decode errors. Software rendering delivered fewer than one frame per
second, and that rendered run did not collect wood within 120 seconds; the
separate real-session harvest test did. Live Jev remains untested without a
TypeSafe key. There is no pixel gate.
