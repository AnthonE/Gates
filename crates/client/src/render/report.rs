//! The report key: `F7` on every screen, or "Report a bug" in the Esc menu.
//!
//! One line of typing and Enter send a report to the platform's intake
//! ([`crate::report::INTAKE_PATH`]) with a PNG of the frame the player was
//! looking at. [`crate::report`] owns what goes in it and why; this file owns
//! nodes, keys, the live facts, and the send.
//!
//! **Why a key rather than a website.** `AGENTS.md` §the deal has paid the
//! board's posted price for every accepted pull request since 2026-08-09, and
//! `NOW.md` §0gh records the ledger as `[]` — the loop works and nothing has ever
//! walked through it. The distance between *a player noticing a bug* and *a
//! report anybody can act on* is the whole gap, and every step in it (find the
//! repo, read the walls, describe the build, remember the seed) is a step
//! where the report stops existing. This is that distance made one key.
//!
//! ## Desktop and browser
//!
//! The same composer and the same document on both; what differs is where a
//! copy can live and how the bytes leave.
//!
//! - **Desktop:** the `.md`/`.json` pair and the `.png` are written beside the
//!   screenshots as before, then the report is POSTed from a thread
//!   (`ureq`). If the send fails, the line says so and names the file.
//! - **Browser:** a tab has no disk, so the send IS the report — `fetch` from
//!   the page, same-origin on elopros.com. If it fails, what the player typed
//!   is kept and `F7` reopens with it.
//!
//! A panic files a crash report on both: the desktop hook writes its file and
//! posts it with a short timeout; the page's hook (`client-web`'s `start`)
//! calls [`web_crash`], which uses `navigator.sendBeacon` because the module
//! is about to stop and a beacon outlives it.
//!
//! ## The composer owns the keyboard while it is open
//!
//! `render/chat.rs`'s rule and its reason: without it, typing "the rock is
//! inside the wall" walks you forward, swings twice and eats whatever is in
//! slot 3. `input::gather` reads [`Reports::open`] the same way it reads
//! `Chat::open`.
//!
//! ## Its line draws on every screen, unlike the HUD's
//!
//! The node is spawned at startup and carries no `WorldEntity`, so it survives
//! a teardown and is up in the menu, on the map and on the death screen — the
//! screenshot key's posture, for the screenshot key's reason (*"a key that
//! works on four screens out of nine is a key a player has to remember the
//! rules for"*). It also means this file never reaches for `hud::Toast`, whose
//! lines ARE `WorldEntity` and would go nowhere on half of those screens: the
//! composer's own line reports success and failure.

use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::prelude::*;
use bevy::render::renderer::RenderAdapterInfo;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::window::{CursorOptions, PrimaryWindow};

use crate::report::{
    self, Audio, Build, Kind, Machine, Netstat, Place, Receipt, Report, Vitals, TITLE_MAX,
};
use crate::shot;

use super::audio::Diag;
use super::screen::Screen;
use super::Net;

/// The key. `F7` because it sits with neither a debug view (`F3`, `F4`) nor
/// a screenshot (`F12`) in any game a player has come from. Reserved from
/// binds (`binds::RESERVED`).
pub const KEY: KeyCode = KeyCode::F7;

/// How long the result line stays up, in seconds.
///
/// It names a reference or a file the player may want to copy, so it has to
/// survive tabbing away and back, and short enough that it is gone before it
/// is furniture.
pub const NOTICE_SECS: f32 = 8.0;

/// The longest side of the screenshot that is sent. A 4K frame is halved
/// until it fits — the evidence survives, the upload stays a few hundred KB.
/// The desktop's own copy on disk is the full frame.
pub const SHOT_SEND_SIDE: u32 = 1920;

/// How long the desktop waits on the intake, in seconds. On a panic the wait
/// is shorter ([`CRASH_TIMEOUT_S`]): the process is dying either way.
pub const SEND_TIMEOUT_S: u64 = 20;
pub const CRASH_TIMEOUT_S: u64 = 3;

/// What a player may choose, in the order `Tab` walks them, ending on the
/// honest one.
///
/// [`Kind::Crash`] is absent and must stay absent: the panic hook is the only
/// author of a crash report, and a player who *chooses* "it crashed" while the
/// game is plainly still running would file one whose fingerprint groups it
/// with real panics.
pub const KINDS: [Kind; 6] = [
    Kind::Look,
    Kind::World,
    Kind::Verb,
    Kind::Net,
    Kind::Numbers,
    Kind::Other,
];

/// One send's result, back on the frame thread.
struct Outcome {
    sent: Result<Receipt, String>,
    /// The desktop's copy on disk, by file name.
    saved: Option<String>,
    /// What the player typed, kept so a failed send is not a retype.
    draft: String,
}

type OutTx = tokio::sync::mpsc::UnboundedSender<Outcome>;

/// The composer, the chosen kind, and the last thing this key said.
#[derive(Resource)]
pub struct Reports {
    /// Where the desktop keeps its copy — [`shot::dir`]'s rule, resolved
    /// **once** at startup. `None` in a browser, and on a desktop whose
    /// environment names no home: the report is still sent.
    pub dir: Option<std::path::PathBuf>,
    /// Does the composer own the keyboard this frame?
    pub open: bool,
    kind: usize,
    text: String,
    /// Text from a send that failed, handed back on the next open.
    draft: String,
    notice: String,
    notice_left: f32,
    /// Reports filed this run. Read by the tests.
    pub filed: u32,
    dirty: bool,
    tx: OutTx,
    rx: tokio::sync::mpsc::UnboundedReceiver<Outcome>,
}

impl Default for Reports {
    fn default() -> Self {
        let dir = shot::dir();
        if dir.is_none() && !cfg!(target_arch = "wasm32") {
            warn!(
                "gates: no home directory in the environment, so {KEY:?} reports are sent \
                 but no copy is kept — set GATES_SHOTS_DIR to name a directory"
            );
        }
        Self::new(dir)
    }
}

impl Reports {
    /// Aimed at a directory, or at nothing — `shot::Shots::new`'s shape, and
    /// for its reason: a test points this somewhere without touching the
    /// process's real (and test-shared) environment.
    pub fn new(dir: Option<std::path::PathBuf>) -> Self {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        Self {
            dir,
            open: false,
            kind: 0,
            text: String::new(),
            draft: String::new(),
            notice: String::new(),
            notice_left: 0.0,
            filed: 0,
            dirty: true,
            tx,
            rx,
        }
    }

    /// The kind currently selected.
    pub fn kind(&self) -> Kind {
        KINDS[self.kind % KINDS.len()]
    }

    /// Characters left before [`TITLE_MAX`] — drawn, because a cap the player
    /// cannot see is a cap that feels like a broken keyboard (`ui::chat`'s
    /// own reasoning, and its wording).
    pub fn remaining(&self) -> usize {
        TITLE_MAX.saturating_sub(self.text.chars().count())
    }

    /// Open the composer, with the text of a send that failed if there was
    /// one. The F7 key and the Esc menu's row both come through here.
    pub fn open_composer(&mut self) {
        self.open = true;
        self.text = std::mem::take(&mut self.draft);
        self.dirty = true;
    }

    /// Open the composer with a line already written, for the player to
    /// finish and send — the death screen's REPORT CHEAT. Filed as
    /// [`Kind::Other`] until the player Tabs to another kind.
    pub fn open_with(&mut self, text: &str) {
        self.open = true;
        self.kind = KINDS.iter().position(|k| *k == Kind::Other).unwrap_or(0);
        self.text = text.chars().take(TITLE_MAX).collect();
        self.dirty = true;
    }

    fn say(&mut self, what: impl Into<String>) {
        self.notice = what.into();
        self.notice_left = NOTICE_SECS;
        self.dirty = true;
    }

    fn close(&mut self) {
        self.open = false;
        self.text.clear();
        self.dirty = true;
    }
}

/// The composer's line. One node, spawned once, never composed out of two
/// helpers — `CLAUDE.md`'s duplicate-component trap is a runtime panic no gate
/// in this repo can see.
#[derive(Component)]
pub struct ReportLine;

/// How far up the composer's line sits. The death screen lifts it over its
/// bottom bar while it is up (`render::death`) and puts it back here.
pub const LINE_BOTTOM: f32 = 52.0;

pub fn setup(mut commands: Commands) {
    commands.spawn((
        ReportLine,
        Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(LINE_BOTTOM),
            left: Val::Px(22.0),
            max_width: Val::Percent(92.0),
            ..default()
        },
        Text::new(""),
        super::ui::font(15.0),
        TextColor(Color::srgba(0.98, 0.94, 0.80, 0.95)),
        Pickable::IGNORE,
        // Over every screen, the death screen's opaque sheet included — its
        // REPORT CHEAT opens this composer.
        GlobalZIndex(super::death::DEATH_Z + 1),
    ));
}

/// `F7` opens; `Tab` changes the kind; `Enter` sends; `Esc` abandons.
///
/// `keyboard` is `ResMut` for `chat::keys`' reason: **a key this consumed must
/// not reach the system after it**, which is what stops a typed line also
/// being a walk, a swing and an inventory toggle.
#[allow(clippy::too_many_arguments)]
pub fn keys(
    mut commands: Commands,
    mut reports: ResMut<Reports>,
    net: Option<NonSend<Net>>,
    who: Option<Res<super::screen::Who>>,
    direct: Option<Res<super::screen::Direct>>,
    pics: Option<Res<super::faces::Pics>>,
    screen: Res<State<Screen>>,
    time: Res<Time>,
    mut keyboard: ResMut<ButtonInput<KeyCode>>,
    mut chars: MessageReader<KeyboardInput>,
    mut cursor: Query<&mut CursorOptions, With<PrimaryWindow>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    adapter: Option<Res<RenderAdapterInfo>>,
    chat: Option<Res<super::chat::Chat>>,
    // The audio seam's last report. Not optional: the engine exists from
    // plugin build, on the menus too, and a report filed anywhere carries it.
    engine: Res<super::audio::Engine>,
) {
    if reports.notice_left > 0.0 {
        reports.notice_left -= time.delta_secs();
        if reports.notice_left <= 0.0 {
            reports.notice.clear();
            reports.dirty = true;
        }
    }

    if !reports.open {
        // Chat already owns the keyboard — two text fields open at once is
        // one of them eating the other's keystrokes (`chat::keys`' rule about
        // the panels, applied to chat itself).
        if chat.is_some_and(|c| c.open()) {
            chars.clear();
            return;
        }
        if keyboard.just_pressed(KEY) {
            reports.open_composer();
            super::ui::free_cursor(&mut cursor);
            keyboard.clear_just_pressed(KEY);
        }
        // Not open: drain, so a keystroke pressed with the composer shut does
        // not arrive in it the moment it opens.
        chars.clear();
        return;
    }

    if keyboard.just_pressed(KeyCode::Escape) {
        reports.close();
        release(&mut cursor, &screen);
        keyboard.clear_just_pressed(KeyCode::Escape);
        chars.clear();
        return;
    }

    if keyboard.just_pressed(KeyCode::Tab) {
        reports.kind = (reports.kind + 1) % KINDS.len();
        reports.dirty = true;
        keyboard.clear_just_pressed(KeyCode::Tab);
        chars.clear();
        return;
    }

    if keyboard.just_pressed(KeyCode::Enter) {
        let machine = machine(adapter.as_deref(), windows.single().ok());
        let origin = pics
            .as_deref()
            .map(|p| p.origin.clone())
            .unwrap_or_else(|| crate::ui::names::PLATFORM_ORIGIN.to_string());
        let job = compose(
            &mut reports,
            net.as_deref(),
            who.as_deref(),
            direct.as_deref(),
            &screen,
            Some(&engine.diag),
            machine,
            report::intake_url(&origin),
        );
        // The frame is shot after the composer's line is cleared, so the
        // evidence is the world and not our own prompt over it. One job, one
        // send: an observer can fire again, the `take` cannot.
        let mut job = Some(job);
        commands
            .spawn(Screenshot::primary_window())
            .observe(move |cap: On<ScreenshotCaptured>| {
                if let Some(job) = job.take() {
                    job.finish(cap.image.clone());
                }
            });
        reports.close();
        reports.say("sending report…");
        release(&mut cursor, &screen);
        keyboard.clear_just_pressed(KeyCode::Enter);
        chars.clear();
        return;
    }

    for ev in chars.read() {
        if !ev.state.is_pressed() {
            continue;
        }
        match &ev.logical_key {
            Key::Backspace => {
                reports.text.pop();
                reports.dirty = true;
            }
            Key::Space => {
                if reports.remaining() > 0 {
                    reports.text.push(' ');
                    reports.dirty = true;
                }
            }
            Key::Character(s) => {
                for c in s.chars() {
                    if reports.remaining() == 0 {
                        break;
                    }
                    reports.text.push(c);
                    reports.dirty = true;
                }
            }
            _ => {}
        }
    }
}

/// Give the pointer back to whoever should have it. In the world that is the
/// window; on any screen a player reads, it is the player.
fn release(cursor: &mut Query<&mut CursorOptions, With<PrimaryWindow>>, screen: &State<Screen>) {
    if *screen.get() != Screen::InWorld {
        return;
    }
    super::ui::lock_cursor(cursor);
}

/// The browser's user agent; `None` on the desktop.
fn user_agent() -> Option<String> {
    #[cfg(target_arch = "wasm32")]
    {
        web_sys::window()?.navigator().user_agent().ok()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        None
    }
}

/// What the report ran on: the adapter wgpu picked, the window, and in a
/// browser the user agent.
fn machine(adapter: Option<&RenderAdapterInfo>, window: Option<&Window>) -> Machine {
    let (gpu, backend, driver) = adapter
        .map(|a| {
            (
                a.name.clone(),
                format!("{:?}", a.backend),
                format!("{} {}", a.driver, a.driver_info),
            )
        })
        .unwrap_or_default();
    let size = window
        .map(|w| [w.physical_width(), w.physical_height()])
        .unwrap_or([0, 0]);
    Machine::new(&gpu, &backend, driver.trim(), size, user_agent().as_deref())
}

/// A report waiting on its screenshot.
struct Job {
    report: Report,
    url: String,
    /// Where the desktop's full-size copy goes.
    png: Option<std::path::PathBuf>,
    saved: Option<String>,
    draft: String,
    tx: OutTx,
}

impl Job {
    /// The frame landed: encode it, keep the desktop's copy, send.
    #[cfg(not(target_arch = "wasm32"))]
    fn finish(self, img: Image) {
        // Off the frame thread: a 4K PNG is tens of milliseconds to encode,
        // and the POST can take seconds.
        std::thread::spawn(move || {
            let rgb = rgb8(img);
            let full = rgb.as_ref().and_then(|(px, w, h)| encode_png(px, *w, *h));
            if let (Some(path), Some(bytes)) = (&self.png, &full) {
                if let Err(e) = std::fs::write(path, bytes) {
                    error!("gates: report screenshot not saved: {e}");
                }
            }
            let send = match rgb {
                Some((px, w, h)) if w.max(h) > SHOT_SEND_SIDE => {
                    let (px, w, h) = shrink(px, w, h, SHOT_SEND_SIDE);
                    encode_png(&px, w, h)
                }
                _ => full,
            };
            let sent = post(
                &self.url,
                self.report.submission(send.as_deref()),
                SEND_TIMEOUT_S,
            );
            let _ = self.tx.send(Outcome {
                sent,
                saved: self.saved,
                draft: self.draft,
            });
        });
    }

    /// The frame landed: encode it and send. One thread, so the encode is on
    /// this frame — a single hitch at the moment the player pressed Enter.
    #[cfg(target_arch = "wasm32")]
    fn finish(self, img: Image) {
        // A tab has no disk, so `compose` never names a copy here.
        debug_assert!(self.png.is_none() && self.saved.is_none());
        let send = rgb8(img).and_then(|(px, w, h)| {
            let (px, w, h) = shrink(px, w, h, SHOT_SEND_SIDE);
            encode_png(&px, w, h)
        });
        let body = self.report.submission(send.as_deref());
        wasm_bindgen_futures::spawn_local(async move {
            let sent = post_web(&self.url, body).await;
            let _ = self.tx.send(Outcome {
                sent,
                saved: None,
                draft: self.draft,
            });
        });
    }
}

/// The captured frame as tightly packed RGB8. Alpha is dropped for
/// `save_to_disk`'s reason: with HDR on it carries brightness, not coverage.
fn rgb8(img: Image) -> Option<(Vec<u8>, u32, u32)> {
    let rgb = img.try_into_dynamic().ok()?.to_rgb8();
    let (w, h) = (rgb.width(), rgb.height());
    Some((rgb.into_raw(), w, h))
}

/// Halve an RGB8 frame (2×2 box) until its longest side is at most `side`.
fn shrink(mut px: Vec<u8>, mut w: u32, mut h: u32, side: u32) -> (Vec<u8>, u32, u32) {
    while w.max(h) > side && w >= 2 && h >= 2 {
        let (nw, nh) = (w / 2, h / 2);
        let mut out = vec![0u8; (nw * nh * 3) as usize];
        for y in 0..nh as usize {
            for x in 0..nw as usize {
                for c in 0..3 {
                    let at = |xx: usize, yy: usize| u32::from(px[(yy * w as usize + xx) * 3 + c]);
                    let sum = at(2 * x, 2 * y)
                        + at(2 * x + 1, 2 * y)
                        + at(2 * x, 2 * y + 1)
                        + at(2 * x + 1, 2 * y + 1);
                    out[(y * nw as usize + x) * 3 + c] = ((sum + 2) / 4) as u8;
                }
            }
        }
        (px, w, h) = (out, nw, nh);
    }
    (px, w, h)
}

fn encode_png(px: &[u8], w: u32, h: u32) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, w, h);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    enc.set_compression(png::Compression::Fast);
    let mut wr = enc.write_header().ok()?;
    wr.write_image_data(px).ok()?;
    wr.finish().ok()?;
    Some(out)
}

/// One POST to the intake, blocking. The status and the body both come back
/// so a refusal is read in the intake's own words.
#[cfg(not(target_arch = "wasm32"))]
fn post(url: &str, body: String, timeout_s: u64) -> Result<Receipt, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .tls_config(crate::tls::ureq())
        .timeout_global(Some(std::time::Duration::from_secs(timeout_s)))
        .http_status_as_error(false)
        .build()
        .into();
    let mut res = agent
        .post(url)
        .header("content-type", "application/json")
        .send(body)
        .map_err(|e| format!("{e}"))?;
    let status = res.status().as_u16();
    let bytes = res
        .body_mut()
        .with_config()
        .limit(64 * 1024)
        .read_to_vec()
        .map_err(|e| format!("{e}"))?;
    report::read_receipt(status, &bytes)
}

/// One POST to the intake from the page. The body goes as text, which keeps
/// it a simple request — no preflight, even from a dev page on another origin.
#[cfg(target_arch = "wasm32")]
async fn post_web(url: &str, body: String) -> Result<Receipt, String> {
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;
    let window = web_sys::window().ok_or("no window")?;
    let init = web_sys::RequestInit::new();
    init.set_method("POST");
    init.set_body(&wasm_bindgen::JsValue::from_str(&body));
    let res = JsFuture::from(window.fetch_with_str_and_init(url, &init))
        .await
        .map_err(|e| crate::net::web::js_err("send", &e))?;
    let res: web_sys::Response = res.dyn_into().map_err(|_| "not a response")?;
    let status = res.status();
    let text = match res.text() {
        Ok(p) => JsFuture::from(p).await.ok().and_then(|t| t.as_string()),
        Err(_) => None,
    };
    report::read_receipt(status, text.unwrap_or_default().as_bytes())
}

/// Build the report from whatever this client currently knows and, on the
/// desktop, write its two files — answering the job that sends it once the
/// screenshot lands.
///
/// Every fact is read rather than kept: this function is the only place the
/// live session is touched, so there is no second copy of "what the netcode
/// believed" to drift from `ClientCore`'s own counters — and the audio
/// seam's `diag` is the same rule one seam over, copied in by the flush
/// rather than read off the audio thread here.
#[allow(clippy::too_many_arguments)]
fn compose(
    reports: &mut Reports,
    net: Option<&Net>,
    who: Option<&super::screen::Who>,
    direct: Option<&super::screen::Direct>,
    screen: &State<Screen>,
    diag: Option<&Diag>,
    machine: Machine,
    url: String,
) -> Job {
    let at = shot::now_secs();
    let mut r = Report::new(reports.kind(), &reports.text, "", at, Build::here());
    r.machine = Some(machine);
    r.audio = diag.map(|d| Audio {
        alive: d.alive,
        routed: d.routed,
        live_thread: d.live_thread,
        blocks: d.stats.blocks,
        backlog: d.backlog,
        ring_dropped: d.ring_dropped,
        refused: d.stats.refused,
    });

    if let Some(net) = net {
        let core = &net.session.core;
        let p = core.predict.position();
        r.place = Some(Place {
            seed: net.session.welcome.seed,
            tick: core.clock.client_tick,
            pos: p,
            screen: screen_slug(screen.get()).into(),
            shard: direct.map(|d| d.0.clone()).unwrap_or_default(),
            player_id: core.player_id,
        });
        r.net = Some(Netstat {
            confirmations: core.predict.confirmations,
            mispredictions: core.predict.mispredictions,
            corrections_minor: core.predict.corrections_minor,
            snapshots_applied: core.snapshots_applied,
            snapshots_delta: core.snapshots_delta,
            snapshots_stale: core.snapshots_stale,
            snapshots_no_baseline: core.snapshots_no_baseline,
            decode_errors: core.decode_errors,
            resyncs: core.clock.resyncs,
        });
        r.vitals = Some(Vitals {
            hp: core.hp,
            hp_max: core.hp_max,
            food: core.food,
            water: core.water,
            dead: core.dead,
        });
    }
    r.wallet = who.and_then(|w| w.0.address().map(str::to_string));

    // The desktop's copy. The picture is named before the document, so the
    // document can name it; `pick` answers `None` only when
    // `shot::PER_STAMP` is spent inside one second, and a report that
    // referenced a picture nobody took would be worse than one that has none.
    let mut png = None;
    let mut saved = None;
    if let Some(dir) = reports.dir.clone() {
        png = shot::pick(&dir, &shot::stamp(at), |p| p.exists());
        if let Some(p) = &png {
            r.attach_shot(p);
        }
        match report::write_pair(&dir, &r) {
            Ok((stem, json)) => {
                if !json {
                    error!("gates: the report's json half did not reach disk: {stem}.json");
                }
                info!(
                    "gates: report → {}",
                    dir.join(format!("{stem}.md")).display()
                );
                saved = Some(format!("{stem}.md"));
            }
            Err(why) => {
                error!("gates: {why}");
                png = None;
            }
        }
    }

    reports.filed += 1;
    Job {
        report: r,
        url,
        png,
        saved,
        draft: reports.text.clone(),
        tx: reports.tx.clone(),
    }
}

/// Read what the sends said and put it on the line.
pub fn poll(mut reports: ResMut<Reports>) {
    while let Ok(o) = reports.rx.try_recv() {
        let line = match (&o.sent, &o.saved) {
            (Ok(r), _) => {
                let mut s = format!("report sent  ·  ref {}", r.id);
                if r.seen > 1 {
                    s.push_str(&format!("  ·  {} reports like it", r.seen));
                }
                s
            }
            (Err(why), Some(file)) => format!("report not sent ({why}) — saved as {file}"),
            (Err(why), None) => {
                format!("report NOT sent: {why} — press F7 to try again, your text is kept")
            }
        };
        if o.sent.is_err() {
            warn!("gates: report not sent: {line}");
            if !reports.open && reports.draft.is_empty() {
                reports.draft = o.draft;
            }
        }
        reports.say(line);
    }
}

/// Which screen, for the report's `place.screen`. A bug on the death screen
/// and a bug in the world are different bugs.
pub fn screen_slug(screen: &Screen) -> &'static str {
    match screen {
        Screen::Boot => "boot",
        Screen::Menu => "menu",
        Screen::Connecting => "connecting",
        Screen::Loading => "loading",
        Screen::InWorld => "world",
        Screen::Paused => "paused",
        Screen::Dead => "dead",
        Screen::Map => "map",
        Screen::Settings => "settings",
        Screen::Disconnected => "disconnected",
        Screen::Travelling => "travelling",
    }
}

/// Keep the line current. Rebuilt only when something changed, for
/// `chat::draw`'s reason: a string built 60 to 144 times a second to be
/// compared against itself is exactly the shape `CLAUDE.md`'s
/// no-per-frame-allocation rule is about.
pub fn draw(mut reports: ResMut<Reports>, mut line: Query<&mut Text, With<ReportLine>>) {
    if !reports.dirty {
        return;
    }
    reports.dirty = false;
    let Ok(mut text) = line.single_mut() else {
        return;
    };
    text.0 = if reports.open {
        format!(
            "report a bug · [{}]  Tab changes  ·  {}_    ({} left, Enter sends it with a \
             screenshot, Esc cancels)",
            reports.kind().label(),
            reports.text,
            reports.remaining()
        )
    } else {
        reports.notice.clone()
    };
}

/// Install the desktop's panic hook: a crash writes the same document a
/// player would, and sends it.
///
/// **Chained, never replaced.** The default hook prints the message and the
/// backtrace, and a hook that swallowed those would trade a report for the two
/// things a developer already relies on. This one writes its file, sends it
/// with a short timeout, and then calls the hook it displaced.
///
/// Called from the binary rather than from a plugin: `set_hook` is
/// process-global, and a test harness that installed it would be a test
/// harness changing how every other test in the binary reports its failures.
#[cfg(not(target_arch = "wasm32"))]
pub fn install_panic_hook() {
    let dir = shot::dir();
    let url = report::intake_url(crate::ui::names::PLATFORM_ORIGIN);
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let r = Report::crash(location(info), &info.to_string(), shot::now_secs());
        if let Some(dir) = &dir {
            if let Ok((stem, _)) = report::write_pair(dir, &r) {
                eprintln!(
                    "gates: a crash report was written to {}",
                    dir.join(format!("{stem}.md")).display()
                );
            }
        }
        match post(&url, r.submission(None), CRASH_TIMEOUT_S) {
            Ok(receipt) => eprintln!("gates: crash report sent, ref {}", receipt.id),
            Err(why) => eprintln!("gates: crash report not sent: {why}"),
        }
        previous(info);
    }));
}

/// The page's half of the panic hook, called from `client-web`'s `start`.
///
/// `sendBeacon` rather than `fetch`: the module traps the moment the hook
/// returns, and a beacon is the request a page hands to the browser to finish
/// after it is gone. Once per page — a second panic is the first one's echo.
#[cfg(target_arch = "wasm32")]
pub fn web_crash(info: &std::panic::PanicHookInfo<'_>) {
    use std::sync::atomic::{AtomicBool, Ordering};
    static SENT: AtomicBool = AtomicBool::new(false);
    if SENT.swap(true, Ordering::Relaxed) {
        return;
    }
    let mut r = Report::crash(location(info), &info.to_string(), shot::now_secs());
    r.machine = Some(Machine::new("", "", "", [0, 0], user_agent().as_deref()));
    let url = report::intake_url(crate::ui::names::PLATFORM_ORIGIN);
    if let Some(w) = web_sys::window() {
        let _ = w
            .navigator()
            .send_beacon_with_opt_str(&url, Some(&r.submission(None)));
    }
}

fn location(info: &std::panic::PanicHookInfo<'_>) -> String {
    info.location()
        .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
        .unwrap_or_else(|| "unknown".into())
}

/// Wire the report key in. **Not on a capture run** — the caller enforces
/// that, beside the screenshot key, and for the same reason: the probe
/// harness spawns its own `Screenshot` entities on a fixed schedule and a
/// second writer of the same frame is a gate whose frames depend on which key
/// was pressed.
pub fn register(app: &mut App) {
    app.init_resource::<Reports>()
        .add_systems(Startup, setup)
        .add_systems(
            Update,
            (
                keys
                    // Before chat, so this composer's `Tab`, `Enter` and `Esc`
                    // are consumed before anything else can read them, and
                    // before chat can decide it owns the keyboard.
                    .before(super::chat::keys)
                    // Before the panels: `Tab` cycles the kind here and opens
                    // the inventory there, and only the order decides which.
                    .before(super::panels::keys)
                    // Before the Esc menu, so cancelling a report does not
                    // also pause the game.
                    .before(super::pause::open)
                    // And before the Esc menu's own keys, so the digit that
                    // picks "Report a bug" is drained here before the
                    // composer it opens can read it as text.
                    .before(super::pause::keys)
                    // And well before `input::gather`, which reads
                    // `Reports::open` to decide whether the world's binds
                    // stand down this frame — `chat::register`'s rule, and
                    // the reason typing a report does not also walk you into
                    // a wall.
                    .before(super::input::gather),
                poll.before(draw),
                draw.after(keys),
            ),
        );
}
