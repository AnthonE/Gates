//! The report key: `F7`, on every screen, all run long.
//!
//! One line of typing and one keypress produce two files beside the player's
//! screenshots — `gates-report-<stamp>-<fp>.md` for a person and `.json` for
//! anything listing them — plus a `.png` of the frame the player was looking
//! at when they pressed it. [`crate::report`] owns what goes in them and why;
//! this file owns nodes, keys, and the live facts.
//!
//! **Why a key rather than a website.** `AGENTS.md` §the deal has paid a flat
//! 100,000 ELO for any accepted pull request since 2026-08-09 and `NOW.md`
//! §0gh records the ledger as `[]` — the loop works and nothing has ever
//! walked through it. The distance between *a player noticing a bug* and *a
//! report anybody can act on* is the whole gap, and every step in it (find the
//! repo, read the walls, describe the build, remember the seed) is a step
//! where the report stops existing. This is that distance made one key.
//!
//! ## It writes a file; it does not send one
//!
//! Nothing here opens a socket. The report lands on the player's disk, next to
//! their pictures, and they decide what happens to it — paste it into an
//! issue, hand it to their own agent, or delete it. That is a deliberate
//! shape rather than an unbuilt feature: a game that quietly posted what its
//! player typed would be a game whose bug button nobody presses twice, and an
//! intake endpoint can be added later behind a prompt without changing a line
//! of what the document *is*.
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
use bevy::render::view::screenshot::{save_to_disk, Screenshot};
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};

use crate::report::{self, Build, Kind, Netstat, Place, Report, Vitals, TITLE_MAX};
use crate::shot;

use super::menu::Screen;
use super::Net;

/// The key. `F7` because the two function keys this client already binds are
/// `F3` (`collider_debug`) and `F12` (`shot`), and `F7` sits with neither a
/// debug view nor a screenshot in any game a player has come from.
pub const KEY: KeyCode = KeyCode::F7;

/// How long the "filed" line stays up, in seconds.
///
/// The HUD's toast cadence is 250 ms and this is not a toast: it names a file
/// the player is about to go and look for, so it has to survive tabbing away
/// and back. Six seconds is long enough to read a filename aloud and short
/// enough that it is gone before it is furniture.
pub const NOTICE_SECS: f32 = 6.0;

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

/// The composer, the chosen kind, and the last thing this key said.
#[derive(Resource)]
pub struct Reports {
    /// `None` when the environment named no base — [`shot::dir`]'s rule, and
    /// resolved **once** at startup so the key does not read the environment
    /// on the frame it is pressed.
    pub dir: Option<std::path::PathBuf>,
    /// Does the composer own the keyboard this frame?
    pub open: bool,
    kind: usize,
    text: String,
    notice: String,
    notice_left: f32,
    /// Reports written this run. Read by the tests.
    pub filed: u32,
    dirty: bool,
}

impl Default for Reports {
    fn default() -> Self {
        let dir = shot::dir();
        if dir.is_none() {
            warn!(
                "gates: no home directory in the environment, so the {KEY:?} report key is off \
                 for this run — set GATES_SHOTS_DIR to name a directory"
            );
        }
        Self {
            dir,
            open: false,
            kind: 0,
            text: String::new(),
            notice: String::new(),
            notice_left: 0.0,
            filed: 0,
            dirty: true,
        }
    }
}

impl Reports {
    /// Aimed at a directory, or at nothing — `shot::Shots::new`'s shape, and
    /// for its reason: a test points this somewhere without touching the
    /// process's real (and test-shared) environment.
    pub fn new(dir: Option<std::path::PathBuf>) -> Self {
        Self {
            dir,
            open: false,
            kind: 0,
            text: String::new(),
            notice: String::new(),
            notice_left: 0.0,
            filed: 0,
            dirty: true,
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

pub fn setup(mut commands: Commands) {
    commands.spawn((
        ReportLine,
        Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(52.0),
            left: Val::Px(22.0),
            max_width: Val::Percent(92.0),
            ..default()
        },
        Text::new(""),
        super::ui::font(15.0),
        TextColor(Color::srgba(0.98, 0.94, 0.80, 0.95)),
        Pickable::IGNORE,
    ));
}

/// `F7` opens; `Tab` changes the kind; `Enter` files; `Esc` abandons.
///
/// `keyboard` is `ResMut` for `chat::keys`' reason: **a key this consumed must
/// not reach the system after it**, which is what stops a typed line also
/// being a walk, a swing and an inventory toggle.
#[allow(clippy::too_many_arguments)]
pub fn keys(
    mut commands: Commands,
    mut reports: ResMut<Reports>,
    net: Option<NonSend<Net>>,
    who: Option<Res<super::boot::Who>>,
    direct: Option<Res<super::boot::Direct>>,
    screen: Res<State<Screen>>,
    time: Res<Time>,
    mut keyboard: ResMut<ButtonInput<KeyCode>>,
    mut chars: MessageReader<KeyboardInput>,
    mut cursor: Query<&mut CursorOptions, With<PrimaryWindow>>,
    chat: Option<Res<super::chat::Chat>>,
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
            if reports.dir.is_none() {
                reports.say("no directory for reports — set GATES_SHOTS_DIR");
            } else {
                reports.open = true;
                reports.text.clear();
                reports.dirty = true;
                if let Ok(mut c) = cursor.single_mut() {
                    c.grab_mode = CursorGrabMode::None;
                    c.visible = true;
                }
            }
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
        let png = file(
            &mut commands,
            &mut reports,
            net.as_deref(),
            who.as_deref(),
            direct.as_deref(),
            &screen,
        );
        if let Some(png) = png {
            commands
                .spawn(Screenshot::primary_window())
                .observe(save_to_disk(png));
        }
        reports.close();
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
    if let Ok(mut c) = cursor.single_mut() {
        c.grab_mode = CursorGrabMode::Locked;
        c.visible = false;
    }
}

/// Build the report from whatever this client currently knows, write both
/// files, and answer with the screenshot path the caller should shoot into.
///
/// Every fact is read rather than kept: this function is the only place the
/// live session is touched, so there is no second copy of "what the netcode
/// believed" to drift from `ClientCore`'s own counters.
fn file(
    commands: &mut Commands,
    reports: &mut Reports,
    net: Option<&Net>,
    who: Option<&super::boot::Who>,
    direct: Option<&super::boot::Direct>,
    screen: &State<Screen>,
) -> Option<std::path::PathBuf> {
    let _ = commands;
    let dir = reports.dir.clone()?;
    let at = shot::now_secs();
    let mut r = Report::new(reports.kind(), &reports.text, "", at, Build::here());

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

    // The picture is named before the document, so the document can name it.
    // `pick` answers `None` only when `shot::PER_STAMP` is spent inside one
    // second, which a report cannot reach — but a report that silently
    // referenced a picture nobody took would be worse than one that admits it
    // has none, so the `None` is carried rather than assumed away.
    let png = shot::pick(&dir, &shot::stamp(at), |p| p.exists());
    if let Some(p) = &png {
        r.attach_shot(p);
    }

    let (stem, json) = match report::write_pair(&dir, &r) {
        Ok(v) => v,
        Err(why) => {
            error!("gates: {why}");
            reports.say("report failed — see the log");
            return None;
        }
    };
    if !json {
        // The Markdown is the report and it landed; the JSON is for a machine
        // listing them. One line, and nothing the player has to act on.
        error!("gates: the report's json half did not reach disk: {stem}.json");
    }

    info!(
        "gates: report → {}",
        dir.join(format!("{stem}.md")).display()
    );
    reports.filed += 1;
    reports.say(format!("report filed  ·  {stem}.md"));
    png
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
            "report · [{}]  Tab changes  ·  {}_    ({} left, Enter files it, Esc cancels)",
            reports.kind().label(),
            reports.text,
            reports.remaining()
        )
    } else {
        reports.notice.clone()
    };
}

/// Install the panic hook: a crash writes the same document a player would.
///
/// **Chained, never replaced.** The default hook prints the message and the
/// backtrace, and a hook that swallowed those would trade a report for the two
/// things a developer already relies on. This one writes its file and then
/// calls the hook it displaced.
///
/// Called from the binary rather than from a plugin: `set_hook` is
/// process-global, and a test harness that installed it would be a test
/// harness changing how every other test in the binary reports its failures.
pub fn install_panic_hook() {
    let Some(dir) = shot::dir() else {
        return;
    };
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "unknown".into());
        // The payload can quote a stranger — a message built from a shard's
        // name or a chat line carries that text in here — so it goes through
        // `Report::new`'s door like any other prose.
        let at = shot::now_secs();
        let mut r = Report::new(Kind::Crash, "the client panicked", "", at, Build::here());
        let (message, cut) =
            report::many_lines(&info.to_string(), report::BODY_MAX, report::BODY_LINES_MAX);
        r.truncated |= cut;
        r.crash = Some(report::Crash { location, message });
        if let Ok((stem, _)) = report::write_pair(&dir, &r) {
            eprintln!(
                "gates: a crash report was written to {}",
                dir.join(format!("{stem}.md")).display()
            );
        }
        previous(info);
    }));
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
                    // And well before `input::gather`, which reads
                    // `Reports::open` to decide whether the world's binds
                    // stand down this frame — `chat::register`'s rule, and
                    // the reason typing a report does not also walk you into
                    // a wall.
                    .before(super::input::gather),
                draw.after(keys),
            ),
        );
}
