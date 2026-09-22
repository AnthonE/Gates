# One shared bot viewpoint

Run from the repository root with the normal Gates render assets staged:

```sh
cargo run -p server --features watch --bin jev-watch -- --scripted
```

Open **http://127.0.0.1:8081/**. Every viewer watches the same bot, camera and
run. The page shows its received health and inventory, controller activity,
trees completed and elapsed play time. Viewing creates no game connection or
model request. `--assets PATH` selects an existing asset directory;
`--content PATH` selects a shipped content directory for a standalone binary;
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
`/frame.jpg`. Each JPEG carries its matching state in an `X-Bot-State`
response header: one fetch serves both, so a slow connection need not chase
the next capture. Explicit requests for a superseded frame ID return 409.
The page marks frames older than five
seconds as stalled. HTTP has a 16-connection ceiling, an 8 KiB header bound
and a five-second request deadline; excess connections close. These are
prototype limits, not a public audience capacity claim. All experimental
delivery defaults are registered in `DECISIONS.md` §Open.

## Shared slow preview

The operator chose this server's slow preview on 2026-09-22. The public route
is **https://elopros.com/games/gates/watch/**. A web proxy strips that prefix when forwarding to
the loopback origin, preserve the trailing slash, and leave frame/state
responses uncached. Relative asset/API URLs support that mount. It can also
be embedded by a page on the same origin; its CSP refuses other origins.

`watch-deploy/gates-watch.service` runs one scripted instance under Xvfb on
the existing render host. Each run uses the command's existing 120 seconds
of play time, then systemd starts a fresh private island after five seconds.
A failed session follows the same restart policy; startup still has its
300-second ceiling. Viewers reconnect automatically and the page explains
the loading pauses. This is repeated short runs, not in-game respawning.
The service runs at nice level 10 so interactive work takes priority.

The installed bundle is `/mnt/hive-data/gates-watch/current`: a pinned binary
and its matching `content/`, with render assets at the sibling `assets/`.
It does not depend on the build checkout at runtime. The render-host nginx
snippet forwards `/gates/watch/` to loopback; the website's more-specific
`/games/gates/watch/` route forwards there over verified HTTPS. Both leave
frames uncached. The website proxy lives in scry-forge's
`deploy/nginx/elopros.com.conf`. Neither route changes the playable client's
static files or the public gameplay shard.

After staging a bundle, install the unit and snippet from `watch-deploy/`,
include the snippet inside the existing render-host HTTPS server, check
`nginx -t`, then reload nginx and enable `gates-watch.service`. Check the
public page and `frame.jpg` (including its `X-Bot-State` header).
`systemctl stop gates-watch` stops only the preview. To roll back routing,
remove the two watch locations/includes and reload the checked nginx config.
Continuous hosting was authorized for this slow preview; smooth video,
large audiences and live Jev remain unvalidated.

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
