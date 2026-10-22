//! Friend whispers on the Elo Pros hive: the socket, the thread, and the one
//! resource the chat reads.
//!
//! `/w <friend> <words>` in chat goes to the friend wherever they are — in
//! Gates on another shard, in the launcher's chat, on elopros.com — in the
//! private two-person room the relay keeps for the pair. `/r` answers the
//! last one. All arithmetic is `crate::hive`; this file owns a thread that
//! holds the relay's socket and talks to the frame through two channels.
//!
//! **The key.** The launcher's chat key (`voice.nsec`), when it is the one
//! the wallet has linked: nothing to sign. Otherwise the first whisper asks
//! the launcher for two signatures, once ever — the text the wallet's chat
//! key is derived from, then the public link naming that key — and saves it
//! where the launcher keeps it, so both speak with one voice.
//!
//! **Nothing is said to have gone until the relay says so.** A sent line is
//! drawn when the relay's OK lands, never when it was typed.

use bevy::prelude::*;
use protocol::Address;
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::net::TcpStream;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::friends::{hex, Friends};
use crate::hive::{self, Frame, Rooms, Voice};

/// How long one socket read waits before the thread looks at its inbox.
const POLL: Duration = Duration::from_millis(150);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// A ping this often: an idle socket some proxy reaps is a deaf one.
const PING_EVERY: Duration = Duration::from_secs(20);
/// Backoff between reconnects, doubling to the cap.
const RETRY_MIN: Duration = Duration::from_secs(5);
const RETRY_MAX: Duration = Duration::from_secs(120);
/// The largest relay message read (a line is small; a member list is not).
const MAX_MESSAGE: usize = 256 << 10;
/// Whispers older than this when the socket opens are not shown: a backlog
/// replayed every launch is noise.
const BACKLOG_S: i64 = 15 * 60;
/// Event ids remembered to drop the relay's repeats.
const SEEN_CAP: usize = 512;
/// Whispers waiting on a socket or a room.
const QUEUE_CAP: usize = 16;
/// Wallets resolved to chat keys and back, kept for the session.
const KEYS_CAP: usize = 512;
const MAX_LINK_BYTES: usize = 8 * 1024;
const MAX_DMS_BYTES: usize = 16 * 1024;

/// Frame → thread.
enum Cmd {
    Send { to: String, text: String },
}

/// Thread → frame.
#[derive(Debug, Clone)]
pub enum Got {
    /// A sentence for the toast line.
    Say(String),
    /// A whisper the relay took (`mine`), or one a friend sent.
    Line {
        wallet: String,
        text: String,
        mine: bool,
    },
}

/// The signed-in player's whisper line.
#[derive(Resource, Default)]
pub struct Dms {
    /// Whose thread is running; a new wallet starts a new one.
    me: Option<Address>,
    tx: Option<std::sync::mpsc::Sender<Cmd>>,
    rx: Option<std::sync::Mutex<std::sync::mpsc::Receiver<Got>>>,
    /// Who whispered last, for `/r`.
    pub last_from: Option<String>,
}

impl Dms {
    /// Whisper `text` to `wallet` (lowercase hex). False when no thread runs.
    pub fn send(&mut self, wallet: &str, text: &str) -> bool {
        self.tx.as_ref().is_some_and(|tx| {
            tx.send(Cmd::Send {
                to: wallet.to_string(),
                text: text.to_string(),
            })
            .is_ok()
        })
    }

    /// What the thread said since the last frame.
    pub fn drain(&mut self) -> Vec<Got> {
        let Some(rx) = &self.rx else {
            return Vec::new();
        };
        let Ok(rx) = rx.lock() else {
            return Vec::new();
        };
        let got: Vec<Got> = rx.try_iter().collect();
        for g in &got {
            if let Got::Line {
                wallet,
                mine: false,
                ..
            } = g
            {
                self.last_from = Some(wallet.clone());
            }
        }
        got
    }
}

/// Start (or restart) the thread for whoever is signed in.
pub fn keep(mut dms: ResMut<Dms>, friends: Res<Friends>) {
    if dms.me == friends.me {
        return;
    }
    dms.me = friends.me;
    dms.last_from = None;
    // Dropping the old sender ends the old thread at its next look.
    dms.tx = None;
    dms.rx = None;
    let Some(me) = friends.me else {
        return;
    };
    let (ctx, crx) = std::sync::mpsc::channel();
    let (gtx, grx) = std::sync::mpsc::channel();
    dms.tx = Some(ctx);
    dms.rx = Some(std::sync::Mutex::new(grx));
    let origin = friends.origin.clone();
    std::thread::Builder::new()
        .name("dms".into())
        .spawn(move || Worker::new(origin, hex(me), crx, gtx).run())
        .ok();
}

/// Who a typed name means: a friend by name or wallet (`/w ash`, `/w 0x…`).
pub fn resolve(friends: &Friends, typed: &str) -> Result<(String, String), String> {
    let t = typed.trim().to_lowercase();
    if t.is_empty() {
        return Err("Whisper whom? /w <friend> <message>".into());
    }
    let Some(v) = &friends.view else {
        return Err("your friends list has not loaded yet".into());
    };
    let named: Vec<&crate::friends::Friend> = v
        .friends
        .iter()
        .filter(|f| {
            f.wallet == t
                || f.name.as_deref().is_some_and(|n| n.to_lowercase() == t)
                || (t.len() >= 6 && f.wallet.starts_with(&t))
        })
        .collect();
    match named.as_slice() {
        [f] => Ok((f.wallet.clone(), f.label())),
        [] => Err(format!(
            "No friend called \"{typed}\". Whispers go to friends."
        )),
        _ => Err(format!(
            "More than one friend is \"{typed}\" - use their wallet."
        )),
    }
}

/// What the line in flight is for.
enum Pending {
    Auth,
    Open { other: String },
    Note { wallet: String, text: String },
}

type Tls = rustls::StreamOwned<rustls::ClientConnection, TcpStream>;
type Socket = tungstenite::WebSocket<Tls>;

struct Worker {
    origin: String,
    me: String,
    cmds: std::sync::mpsc::Receiver<Cmd>,
    out: std::sync::mpsc::Sender<Got>,
    voice: Option<Voice>,
    rooms: Rooms,
    /// Whispers waiting on the socket, the key, or a room: (wallet, text).
    queue: VecDeque<(String, String)>,
    pending: HashMap<String, Pending>,
    seen: VecDeque<String>,
    /// wallet → chat key (hex), and back.
    keys: HashMap<String, String>,
    wallets: HashMap<String, String>,
    authed: bool,
    since: i64,
}

impl Worker {
    fn new(
        origin: String,
        me: String,
        cmds: std::sync::mpsc::Receiver<Cmd>,
        out: std::sync::mpsc::Sender<Got>,
    ) -> Self {
        Self {
            origin,
            me,
            cmds,
            out,
            voice: None,
            rooms: Rooms::default(),
            queue: VecDeque::new(),
            pending: HashMap::new(),
            seen: VecDeque::new(),
            keys: HashMap::new(),
            wallets: HashMap::new(),
            authed: false,
            since: 0,
        }
    }

    fn say(&self, s: impl Into<String>) {
        let _ = self.out.send(Got::Say(s.into()));
    }

    /// The thread: find a key, hold a socket, reconnect when it drops. Ends
    /// when the frame drops its sender.
    fn run(mut self) {
        self.voice = self.linked_voice();
        let mut wait = RETRY_MIN;
        loop {
            if self.voice.is_none() {
                // Nothing to listen with until a whisper asks for a key.
                match self.cmds.recv() {
                    Ok(Cmd::Send { to, text }) => {
                        self.enqueue(to, text);
                        // The launcher's key may be fine and the platform
                        // was only unreachable at start: look again first.
                        if let Some(v) = self.linked_voice() {
                            self.voice = Some(v);
                            continue;
                        }
                        match self.make_voice() {
                            Ok(v) => self.voice = Some(v),
                            Err(why) => {
                                self.queue.clear();
                                self.say(why);
                                continue;
                            }
                        }
                    }
                    Err(_) => return,
                }
            }
            match self.session() {
                Ok(()) => return, // the frame hung up
                Err(why) => {
                    info!("dms: {why}");
                    if !self.queue.is_empty() {
                        self.say(format!("Whisper not sent yet: {why}. Retrying."));
                    }
                }
            }
            // Wait out the backoff, still taking whispers to queue.
            let until = Instant::now() + wait;
            while Instant::now() < until {
                match self.cmds.recv_timeout(Duration::from_millis(250)) {
                    Ok(Cmd::Send { to, text }) => self.enqueue(to, text),
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                    Err(_) => return,
                }
            }
            wait = (wait * 2).min(RETRY_MAX);
        }
    }

    fn enqueue(&mut self, to: String, text: String) {
        if self.queue.len() >= QUEUE_CAP {
            self.say("Too many whispers waiting - wait for these to go.");
            return;
        }
        self.queue.push_back((to, text));
    }

    fn get(&self, path: &str, cap: usize) -> Option<Value> {
        let (status, body) = super::friends::http(&format!("{}{path}", self.origin), None, cap);
        (status == 200 && body.len() <= cap)
            .then(|| serde_json::from_slice(&body).ok())
            .flatten()
    }

    /// The npub `wallet` has linked, if any. `Err` when the platform could not
    /// be asked, which is not "none".
    fn linked_npub(&self, wallet: &str) -> Result<Option<String>, String> {
        let v = self
            .get(&format!("/api/hive/link/{wallet}"), MAX_LINK_BYTES)
            .ok_or("Elo Pros did not answer")?;
        Ok(v.get("npub").and_then(Value::as_str).map(str::to_string))
    }

    /// The launcher's chat key, when it is the one this wallet linked.
    fn linked_voice(&self) -> Option<Voice> {
        let raw = std::fs::read_to_string(hive::voice_dir().join(hive::KEY_NAME)).ok()?;
        let v = Voice::from_nsec(&raw).ok()?;
        (self.linked_npub(&self.me).ok()?.as_deref() == Some(v.npub().as_str())).then_some(v)
    }

    /// The wallet's chat key, derived and linked through the launcher's
    /// signer, then saved where the launcher keeps its own.
    fn make_voice(&self) -> Result<Voice, String> {
        use crate::elo::SignError;
        self.say("Setting up your chat key: approve it in your launcher (once).");
        let me = &self.me;
        let q0 = self
            .get(&format!("/api/hive/link?wallet={me}"), MAX_LINK_BYTES)
            .ok_or("Elo Pros did not answer")?;
        let derive = hive::derivation_message(me);
        if q0.pointer("/voice_key/sign_this").and_then(Value::as_str) != Some(derive.as_str()) {
            return Err(
                "Your chat key is set up differently on elopros.com - whisper from \
                        elopros.com/chat.html, or run `elo chat link`."
                    .into(),
            );
        }
        let mut elo = crate::elo::Elo::discover(None, env!("CARGO_PKG_VERSION"));
        let mut sign = |text: &str, why: &str| -> Result<String, String> {
            match elo.sign(text, why) {
                Ok(sig) => {
                    if sig
                        .address
                        .as_deref()
                        .is_some_and(|a| !a.eq_ignore_ascii_case(me))
                    {
                        return Err("your launcher signs as another wallet".into());
                    }
                    Ok(sig.signature)
                }
                Err(SignError::RefusedByPlayer(_)) => {
                    Err("You said no in the launcher. No whisper went.".into())
                }
                Err(SignError::Handoff(_)) => {
                    Err("Your launcher signs in your browser: set up chat once at \
                     elopros.com/chat.html, then `elo chat link`."
                        .into())
                }
                Err(SignError::Refused(why)) => Err(format!("The launcher would not sign: {why}")),
                Err(SignError::NoLauncher(_)) => {
                    Err("Whispers need the Elo launcher running, to make your chat key.".into())
                }
            }
        };
        let sig = sign(&derive, "Gates: make this wallet's chat key")?;
        let voice = Voice::from_wallet_signature(&sig)?;
        let npub = voice.npub();
        if self.linked_npub(me)?.as_deref() != Some(npub.as_str()) {
            let q1 = self
                .get(
                    &format!("/api/hive/link?wallet={me}&npub={npub}"),
                    MAX_LINK_BYTES,
                )
                .ok_or("Elo Pros did not answer")?;
            let at = q1
                .get("at")
                .and_then(Value::as_u64)
                .ok_or("no `at` to sign")?;
            let text = hive::link_sign_text(me, at, &npub);
            if q1.get("sign_this").and_then(Value::as_str) != Some(text.as_str())
                || q1.get("event_content").and_then(Value::as_str)
                    != Some(hive::link_content(me).as_str())
            {
                return Err(
                    "Elo Pros offered a different link text - nothing was signed. \
                            Update Gates."
                        .into(),
                );
            }
            let wsig = sign(
                &text,
                "Gates: link your chat key so friends can whisper you",
            )?;
            let event = voice.sign(now(), hive::KIND_LINK, Vec::new(), hive::link_content(me));
            let body = json!({"wallet": me, "npub": npub, "at": at, "signature": wsig,
                              "event": event.to_json()})
            .to_string();
            let (status, got) = super::friends::http(
                &format!("{}/api/hive/link", self.origin),
                Some(&body),
                MAX_LINK_BYTES,
            );
            if status != 200 {
                return Err(format!(
                    "Elo Pros would not link the key: {}",
                    crate::friends::error_of(status, &got)
                ));
            }
        }
        save(&voice)?;
        self.say("Chat key ready. Friends can whisper you now.");
        Ok(voice)
    }

    /// One socket's life. `Ok` only when the frame hung up.
    fn session(&mut self) -> Result<(), String> {
        let voice = self.voice.as_ref().ok_or("no chat key")?;
        let npub = voice.npub();
        let d = self
            .get(&format!("/api/hive/dms?npub={npub}"), MAX_DMS_BYTES)
            .ok_or("Elo Pros did not say where chat is")?;
        let relay = d
            .get("relay")
            .and_then(Value::as_str)
            .filter(|r| r.starts_with("wss://"))
            .ok_or("chat has no relay right now")?
            .to_string();
        let members = d.get("membership_sub").cloned().ok_or("no member filter")?;
        let mut ws = connect(&relay)?;
        self.authed = false;
        self.pending.clear();
        self.since = now() - BACKLOG_S;
        let mut pinged = Instant::now();
        loop {
            // The frame's whispers.
            loop {
                match self.cmds.try_recv() {
                    Ok(Cmd::Send { to, text }) => self.enqueue(to, text),
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        let _ = ws.close(None);
                        return Ok(());
                    }
                }
            }
            if self.authed {
                self.flush(&mut ws)?;
            }
            if pinged.elapsed() >= PING_EVERY {
                ws.send(tungstenite::Message::Ping(Vec::new().into()))
                    .map_err(|e| format!("chat dropped: {e}"))?;
                pinged = Instant::now();
            }
            let msg = match ws.read() {
                Ok(m) => m,
                Err(tungstenite::Error::Io(e))
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    continue
                }
                Err(e) => return Err(format!("chat dropped: {e}")),
            };
            let text = match msg {
                tungstenite::Message::Text(t) => t.to_string(),
                tungstenite::Message::Close(_) => return Err("chat closed".into()),
                _ => continue,
            };
            match hive::frame(&text) {
                Frame::Auth(challenge) => {
                    let ev =
                        hive::auth_event(self.voice.as_ref().unwrap(), &relay, &challenge, now());
                    self.pending.insert(ev.id.clone(), Pending::Auth);
                    send(&mut ws, hive::auth(&ev))?;
                }
                Frame::Ok(id, ok, reason) => match self.pending.remove(&id) {
                    Some(Pending::Auth) if ok => {
                        self.authed = true;
                        send(&mut ws, hive::req("m", members.clone()))?;
                    }
                    Some(Pending::Auth) => return Err(format!("chat refused this key: {reason}")),
                    Some(Pending::Open { other }) => match hive::dm_room_of(&reason).filter(|_| ok)
                    {
                        Some(room) => {
                            if self.rooms.with(&other).is_none() {
                                self.rooms.rows.push((room, other));
                            }
                            self.listen(&mut ws)?;
                        }
                        None => {
                            self.drop_queued_for(&other);
                            self.say(format!("Couldn't open a private chat: {reason}"));
                        }
                    },
                    Some(Pending::Note { wallet, text }) => {
                        if ok {
                            let _ = self.out.send(Got::Line {
                                wallet,
                                text,
                                mine: true,
                            });
                        } else {
                            self.say(format!("Whisper not sent: {reason}"));
                        }
                    }
                    None => {}
                },
                Frame::Event(_, ev) => self.event(&mut ws, &ev)?,
                Frame::Closed(sub, why) => info!("dms: relay closed {sub}: {why}"),
                Frame::Eose(_) | Frame::Other => {}
            }
        }
    }

    /// (Re)subscribe to every room's lines.
    fn listen(&self, ws: &mut Socket) -> Result<(), String> {
        let ids = self.rooms.ids();
        if ids.is_empty() {
            return Ok(());
        }
        send(
            ws,
            hive::req(
                "d",
                json!({"kinds": [hive::KIND_ROOM_NOTE], "#h": ids, "since": self.since, "limit": 50}),
            ),
        )
    }

    fn event(&mut self, ws: &mut Socket, ev: &hive::Event) -> Result<(), String> {
        if self.seen.contains(&ev.id) || !ev.verify() {
            return Ok(());
        }
        if self.seen.len() >= SEEN_CAP {
            self.seen.pop_front();
        }
        self.seen.push_back(ev.id.clone());
        let me = self
            .voice
            .as_ref()
            .map(Voice::pubkey_hex)
            .unwrap_or_default();
        if ev.kind == hive::KIND_MEMBERS {
            if self.rooms.take(ev, &me) {
                self.listen(ws)?;
            }
            return Ok(());
        }
        if ev.kind != hive::KIND_ROOM_NOTE || ev.pubkey == me || ev.created_at < self.since {
            return Ok(());
        }
        let Some(room) = ev.tag("h") else {
            return Ok(());
        };
        if self.rooms.other(room) != Some(ev.pubkey.as_str()) {
            return Ok(());
        }
        let Some(wallet) = self.wallet_of(&ev.pubkey) else {
            return Ok(()); // an unlinked key: not a friend we can name
        };
        let text = hive::tidy_line(&ev.content);
        if !text.is_empty() {
            let _ = self.out.send(Got::Line {
                wallet,
                text,
                mine: false,
            });
        }
        Ok(())
    }

    /// Send what is queued: open a room first where there is none.
    fn flush(&mut self, ws: &mut Socket) -> Result<(), String> {
        let mut held = VecDeque::new();
        while let Some((wallet, text)) = self.queue.pop_front() {
            let Some(other) = self.key_of(&wallet) else {
                self.say("That friend hasn't set up chat yet - they can at elopros.com/chat.html.");
                continue;
            };
            let voice = self.voice.as_ref().unwrap();
            match self.rooms.with(&other) {
                Some(room) => {
                    let ev = hive::note_event(voice, room, &text, now());
                    self.pending
                        .insert(ev.id.clone(), Pending::Note { wallet, text });
                    send(ws, hive::publish(&ev))?;
                }
                None => {
                    let opening = self
                        .pending
                        .values()
                        .any(|p| matches!(p, Pending::Open { other: o } if *o == other));
                    if !opening {
                        let ev = hive::dm_open_event(voice, &other, now());
                        self.pending.insert(
                            ev.id.clone(),
                            Pending::Open {
                                other: other.clone(),
                            },
                        );
                        send(ws, hive::publish(&ev))?;
                    }
                    held.push_back((wallet, text));
                }
            }
        }
        self.queue = held;
        Ok(())
    }

    fn drop_queued_for(&mut self, other: &str) {
        let keys = self.keys.clone();
        self.queue
            .retain(|(w, _)| keys.get(w).map(String::as_str) != Some(other));
    }

    /// A friend's chat key (hex), off their public link.
    fn key_of(&mut self, wallet: &str) -> Option<String> {
        if let Some(k) = self.keys.get(wallet) {
            return Some(k.clone());
        }
        let k = hive::npub_hex(&self.linked_npub(wallet).ok()??)?;
        self.remember(wallet, &k);
        Some(k)
    }

    /// The wallet a chat key is linked to.
    fn wallet_of(&mut self, key: &str) -> Option<String> {
        if let Some(w) = self.wallets.get(key) {
            return Some(w.clone());
        }
        let npub = hive::npub_of(key)?;
        let v = self.get(&format!("/api/hive/link/{npub}"), MAX_LINK_BYTES)?;
        let w = v.get("wallet")?.as_str()?.to_ascii_lowercase();
        crate::friends::wallet_ok(&w).then_some(())?;
        self.remember(&w, key);
        Some(w)
    }

    fn remember(&mut self, wallet: &str, key: &str) {
        if self.keys.len() >= KEYS_CAP {
            self.keys.clear();
            self.wallets.clear();
        }
        self.keys.insert(wallet.to_string(), key.to_string());
        self.wallets.insert(key.to_string(), wallet.to_string());
    }
}

fn now() -> i64 {
    crate::shot::now_secs() as i64
}

fn send(ws: &mut Socket, text: String) -> Result<(), String> {
    ws.send(tungstenite::Message::text(text))
        .map_err(|e| format!("chat dropped: {e}"))
}

/// Save the key where the launcher keeps its own, 0600, beside any key that
/// was there (renamed, never deleted: it is the only copy of what it said).
fn save(voice: &Voice) -> Result<(), String> {
    let dir = hive::voice_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("couldn't save the chat key: {e}"))?;
    let path = dir.join(hive::KEY_NAME);
    if let Ok(old) = std::fs::read_to_string(&path) {
        match Voice::from_nsec(&old) {
            Ok(o) if o.npub() == voice.npub() => return Ok(()),
            Ok(o) => {
                let tag: String = o
                    .npub()
                    .trim_start_matches("npub1")
                    .chars()
                    .take(10)
                    .collect();
                std::fs::rename(&path, dir.join(format!("{}.{tag}.old", hive::KEY_NAME)))
                    .map_err(|e| format!("couldn't move the old chat key aside: {e}"))?;
            }
            Err(_) => {}
        }
    }
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    use std::io::Write;
    opts.open(&path)
        .and_then(|mut f| f.write_all((voice.to_nsec() + "\n").as_bytes()))
        .map_err(|e| format!("couldn't save the chat key: {e}"))
}

/// A `wss://` socket to the relay, over this client's own trust store.
fn connect(url: &str) -> Result<Socket, String> {
    connect_with(url, &[])
}

/// [`connect`], trusting `extra` DER roots besides (a test behind a TLS proxy).
fn connect_with(url: &str, extra: &[Vec<u8>]) -> Result<Socket, String> {
    let rest = url
        .strip_prefix("wss://")
        .ok_or("the relay is not wss://")?;
    let authority = rest.split('/').next().unwrap_or(rest);
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) if !h.contains(']') => (h, p.parse().map_err(|_| "bad relay port")?),
        _ => (authority, 443u16),
    };
    let tcp = std::net::ToSocketAddrs::to_socket_addrs(&(host, port))
        .map_err(|e| format!("can't find the chat server: {e}"))?
        .find_map(|a| TcpStream::connect_timeout(&a, CONNECT_TIMEOUT).ok())
        .ok_or("can't reach the chat server")?;
    tcp.set_read_timeout(Some(CONNECT_TIMEOUT)).ok();
    tcp.set_write_timeout(Some(CONNECT_TIMEOUT)).ok();
    let mut roots = rustls::RootCertStore::empty();
    for der in crate::tls::root_certs() {
        let _ = roots.add(rustls::pki_types::CertificateDer::from(der.to_vec()));
    }
    for der in extra {
        let _ = roots.add(rustls::pki_types::CertificateDer::from(der.clone()));
    }
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(|e| format!("tls: {e}"))?
    .with_root_certificates(roots)
    .with_no_client_auth();
    let name =
        rustls::pki_types::ServerName::try_from(host.to_string()).map_err(|_| "bad relay host")?;
    let conn =
        rustls::ClientConnection::new(Arc::new(config), name).map_err(|e| format!("tls: {e}"))?;
    let limits = tungstenite::protocol::WebSocketConfig::default()
        .max_message_size(Some(MAX_MESSAGE))
        .max_frame_size(Some(MAX_MESSAGE));
    let (ws, _) = tungstenite::client::client_with_config(
        url,
        rustls::StreamOwned::new(conn, tcp),
        Some(limits),
    )
    .map_err(|e| format!("chat handshake failed: {e}"))?;
    ws.get_ref().sock.set_read_timeout(Some(POLL)).ok();
    Ok(ws)
}

/// Carry a `/w` or `/r` the chat composer took.
pub fn outgoing(
    chat: Option<ResMut<super::chat::Chat>>,
    mut dms: ResMut<Dms>,
    friends: Res<Friends>,
    toast: Option<ResMut<super::hud::Toast>>,
) {
    use crate::ui::chat::Whispered;
    let (Some(mut chat), Some(mut toast)) = (chat, toast) else {
        return;
    };
    let Some(w) = chat.whisper.take() else {
        return;
    };
    let (to, text) = match w {
        Whispered::To(who, text) => match resolve(&friends, &who) {
            Ok((wallet, _)) => (wallet, text),
            Err(why) => {
                toast.warn(why);
                return;
            }
        },
        Whispered::Reply(text) => match dms.last_from.clone() {
            Some(wallet) => (wallet, text),
            None => {
                toast.say("Nobody has whispered you yet.");
                return;
            }
        },
    };
    if text.is_empty() {
        toast.say("Say something after the name: /w <friend> <message>");
        return;
    }
    if friends.me.is_none() {
        toast.say(super::friends::SIGNED_OUT);
        return;
    }
    if !dms.send(&to, &text) {
        toast.warn("whispers aren't running - try again in a moment");
    }
}

/// Whispers in: on the chat line in a world, as a friends notice at the menu.
pub fn incoming(
    mut dms: ResMut<Dms>,
    mut friends: ResMut<Friends>,
    chat: Option<ResMut<super::chat::Chat>>,
    toast: Option<ResMut<super::hud::Toast>>,
    screen: Res<State<super::Screen>>,
    time: Res<Time<Real>>,
) {
    let got = dms.drain();
    if got.is_empty() {
        return;
    }
    let in_world = matches!(screen.get(), super::Screen::InWorld | super::Screen::Paused);
    let (mut chat, mut toast) = (chat, toast);
    for g in got {
        match g {
            Got::Say(s) => match toast.as_deref_mut().filter(|_| in_world) {
                Some(t) => t.say(s),
                None => friends.say(s),
            },
            Got::Line { wallet, text, mine } => {
                let addr = Address::from_hex(wallet.as_bytes());
                let name = friends
                    .view
                    .as_ref()
                    .and_then(|v| addr.and_then(|a| v.find(&a)))
                    .map(|f| f.label())
                    .unwrap_or_else(|| crate::friends::short_wallet(&wallet));
                match chat.as_deref_mut().filter(|_| in_world) {
                    Some(chat) => {
                        chat.log.push(crate::ui::chat::Line {
                            name,
                            text,
                            face: addr.map(|a| (a, 0)),
                            friend: true,
                            at: time.elapsed_secs_f64(),
                            whisper: if mine {
                                crate::ui::chat::Whisper::To
                            } else {
                                crate::ui::chat::Whisper::From
                            },
                            ..Default::default()
                        });
                        chat.dirty = true;
                    }
                    None if !mine => {
                        friends.notices.push(format!("{name} whispers: {text}"));
                    }
                    None => {}
                }
            }
        }
    }
}

pub fn register(app: &mut App) {
    app.init_resource::<Dms>().add_systems(
        Update,
        (
            keep,
            outgoing.run_if(in_state(super::Screen::InWorld)),
            incoming,
        )
            .chain(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The live relay, read-only: a throwaway key connects, authenticates and
    /// lists its (empty) private rooms. Writes nothing. `--ignored`: network.
    #[test]
    #[ignore]
    fn the_live_relay_takes_our_auth() {
        let relay = std::env::var("HIVE_RELAY").unwrap_or("wss://hive.elopros.com".into());
        let voice = Voice::from_secret(&[0x42; 32]).unwrap();
        // Behind a TLS-intercepting proxy (CI, a sandbox) its CA is trusted
        // here, in the test only.
        let extra: Vec<Vec<u8>> = std::env::var("SSL_CERT_FILE")
            .ok()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map(|pem| {
                pem.split("-----BEGIN CERTIFICATE-----")
                    .skip(1)
                    .filter_map(|b| b.split("-----END CERTIFICATE-----").next())
                    .filter_map(base64_der)
                    .collect()
            })
            .unwrap_or_default();
        let mut ws = connect_with(&relay, &extra).expect("connects");
        let deadline = Instant::now() + Duration::from_secs(20);
        let (mut auth_id, mut authed) = (String::new(), false);
        while Instant::now() < deadline {
            let text = match ws.read() {
                Ok(tungstenite::Message::Text(t)) => t.to_string(),
                Ok(_) => continue,
                Err(tungstenite::Error::Io(_)) => continue,
                Err(e) => panic!("{e}"),
            };
            match hive::frame(&text) {
                Frame::Auth(c) => {
                    let ev = hive::auth_event(&voice, &relay, &c, now());
                    auth_id = ev.id.clone();
                    send(&mut ws, hive::auth(&ev)).unwrap();
                }
                Frame::Ok(id, ok, why) if id == auth_id => {
                    assert!(ok, "AUTH refused: {why}");
                    authed = true;
                    let filter = json!({"kinds": [hive::KIND_MEMBERS],
                                        "#p": [voice.pubkey_hex()], "limit": 5});
                    send(&mut ws, hive::req("m", filter)).unwrap();
                }
                Frame::Eose(sub) if sub == "m" => return assert!(authed),
                Frame::Closed(sub, why) => panic!("closed {sub}: {why}"),
                f => eprintln!("{f:?}"),
            }
        }
        panic!("no EOSE in time (authed: {authed})");
    }

    fn base64_der(b64: &str) -> Option<Vec<u8>> {
        const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let (mut out, mut acc, mut bits) = (Vec::new(), 0u32, 0);
        for c in b64
            .bytes()
            .filter(|c| !c.is_ascii_whitespace() && *c != b'=')
        {
            acc = (acc << 6) | A.iter().position(|a| *a == c)? as u32;
            bits += 6;
            if bits >= 8 {
                bits -= 8;
                out.push((acc >> bits) as u8);
            }
        }
        Some(out)
    }
}
