//! The companion: Gates' Rust+ (`MENUS.md`: "no `RUST+` equivalent"). A page
//! and a small JSON API on a loopback listener of their own, for a player
//! away from the game and for an agent that is not running a client: sign in
//! with the wallet you play with, then see your team on the map, read and
//! write team chat, and hear when your base takes damage.
//!
//! Rust+ needs a pairing step because a Steam login on a phone does not say
//! which server or character is yours. Here the wallet signature is the
//! identity on every surface, so signing in IS pairing.
//!
//! ## The API (also what the page uses)
//!
//! ```text
//! GET  api/challenge?address=0x…[&domain=elopros.com]
//!        → {"domain","nonce","issued_at","message"}   personal_sign `message`
//! POST api/login   {"address","nonce","issued_at","signature"}
//!                                                    → {"token","wallet","expires"}
//! GET  api/team    Authorization: Bearer <token>     → you, your team, your
//!                                                       bases, team chat, alerts
//! POST api/chat    Authorization: Bearer <token>  {"text"}  → 202, said on team chat
//! POST api/logout  Authorization: Bearer <token>     → 204
//! GET  map.png                                        the island, north up
//! ```
//!
//! `domain` is the page's own host, so a wallet's domain check passes; it is
//! used only if the shard accepts it (`siwe_domains`), else the shard's
//! `domain`. An agent with an `agentkey` signs the default challenge with
//! `AgentKey::sign_proof_hex(domain, nonce, issued_at)`: the message is the
//! game's own sign-in text over a nonce only this listener issued, used once.
//! Only a wallet with a character on the island signs in (403 otherwise).
//! Positions are whole metres in world axes; `grid` is the map's square.
//!
//! ## Who sees what
//!
//! Your own body, the members of your team, the bases whose crew you are on,
//! the team lines you were on the team for, and the alerts for your bases.
//! Nothing about anyone else.
//!
//! ## Threads
//!
//! Sim → here: [`ToWeb`] on one ring — a [`Snap`] every second, and team
//! lines and raid alerts as they happen. Here → sim: [`WebLine`], a line typed
//! on the page. Neither side waits; a full ring drops the message. This
//! thread answers one connection at a time with a deadline, like `status.rs`,
//! and nginx is the buffer in front of it.

use crate::store::PlayerKey;
use protocol::{Address, ChatText, Signature, NONCE_BYTES, SIGNATURE_BYTES, SIWE_MESSAGE_MAX};
use std::collections::{BTreeMap, VecDeque};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::time::{Duration, Instant};

/// Sim → companion messages in flight. One snapshot a second plus chat and
/// alerts; a backlog this deep means the thread is stuck.
const TO_WEB_RING: usize = 256;
/// Page → sim chat lines in flight. The sim drains it every tick.
const TO_SIM_RING: usize = 64;
/// Team lines kept, across all teams. Each viewer reads the ones they heard.
const LINES_KEPT: usize = 2000;
/// Lines a viewer gets back per read.
const LINES_SHOWN: usize = 50;
/// Raid alerts kept, and how many a viewer gets back.
const ALERTS_KEPT: usize = 500;
const ALERTS_SHOWN: usize = 20;
/// How long a challenge waits for its signature.
const CHALLENGE_SECS: u64 = 300;
/// Nonces signed in with and not yet expired, kept so none is used twice.
const USED_KEPT: usize = 65536;
/// Signed-in sessions, all wallets and one wallet's, and how long one lasts.
const SESSIONS_KEPT: usize = 4096;
const SESSIONS_PER_WALLET: usize = 8;
pub const SESSION_SECS: u64 = 30 * 24 * 3600;
/// The most of a request this reads: request line, headers and body.
const REQUEST_CAP: usize = 8192;
/// One connection's whole deadline, request and response together.
const IO_TIMEOUT: Duration = Duration::from_secs(2);
/// The map's side in pixels: 4 m a pixel on the 4 km island.
pub const MAP_PX: usize = 1024;
/// Ticks between snapshots: one a second.
pub const SNAP_TICKS: u64 = sim_core::limits::TICK_HZ as u64;
/// Ticks one base stays quiet after its alert, so a raid is one alert a
/// minute and not one a swing.
pub const RAID_QUIET_TICKS: u64 = 60 * sim_core::limits::TICK_HZ as u64;

/// One player with a body on the island, connected or asleep.
#[derive(Clone, Debug, PartialEq)]
pub struct Person {
    /// Lowercase `0x…`; empty for a guest.
    pub wallet: String,
    pub name: String,
    pub id: u32,
    /// `protocol::TEAM_AWAKE`, `TEAM_DEAD` or `TEAM_ASLEEP`.
    pub state: u8,
    pub x_m: u16,
    pub z_m: u16,
    pub online: bool,
    /// The leader of their team (the team's key), 0 for none.
    pub team: u32,
    pub leader: bool,
}

/// A hearth: where it stands and who is on its crew.
#[derive(Clone, Debug, PartialEq)]
pub struct Base {
    pub x_m: u16,
    pub z_m: u16,
    pub crew: Vec<u32>,
}

/// The island as the companion shows it, once a second.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snap {
    pub tick: u64,
    pub players: u32,
    pub people: Vec<Person>,
    pub bases: Vec<Base>,
}

/// One team line, and the wallets on the speaker's team when it was said.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub from: String,
    pub name: String,
    pub text: String,
    pub hearers: Vec<String>,
    /// Typed on the companion, not in the game.
    pub web: bool,
}

/// A base taking damage, and the wallets on its crew.
#[derive(Clone, Debug, PartialEq)]
pub struct Alert {
    pub x_m: u16,
    pub z_m: u16,
    pub wallets: Vec<String>,
}

pub enum ToWeb {
    Snap(Snap),
    Line(Line),
    Alert(Alert),
}

/// A line typed on the page, for the sim to say on the wallet's team chat.
#[derive(Clone, Copy, Debug)]
pub struct WebLine {
    pub wallet: PlayerKey,
    pub text: ChatText,
}

/// The sim side's end.
pub struct Companion {
    out: Option<rtrb::Producer<ToWeb>>,
    inbox: Option<rtrb::Consumer<WebLine>>,
}

impl Companion {
    pub fn off() -> Self {
        Companion {
            out: None,
            inbox: None,
        }
    }

    pub fn on(&self) -> bool {
        self.out.is_some()
    }

    pub fn send(&mut self, m: ToWeb) {
        if let Some(out) = self.out.as_mut() {
            let _ = out.push(m);
        }
    }

    /// The next line typed on the page, if any.
    pub fn pop(&mut self) -> Option<WebLine> {
        self.inbox.as_mut()?.pop().ok()
    }
}

/// A companion with no thread: the sim's end, plus the two ends the thread
/// would hold. For tests, and for anything hosting the API in-process.
pub fn pair() -> (Companion, rtrb::Consumer<ToWeb>, rtrb::Producer<WebLine>) {
    let (out, from_sim) = rtrb::RingBuffer::<ToWeb>::new(TO_WEB_RING);
    let (to_sim, inbox) = rtrb::RingBuffer::<WebLine>::new(TO_SIM_RING);
    (
        Companion {
            out: Some(out),
            inbox: Some(inbox),
        },
        from_sim,
        to_sim,
    )
}

/// Bind `addr` and start the companion thread for the island `seed`. Returns
/// the sim's end and the bound address. `domain` is the shard's SIWE name and
/// `accepts` every name a proof may be bound to (`ShardConfig::siwe_accepts`).
pub fn spawn(
    addr: SocketAddr,
    seed: u64,
    domain: String,
    accepts: Vec<String>,
) -> std::io::Result<(Companion, SocketAddr)> {
    let listener = TcpListener::bind(addr)?;
    let bound = listener.local_addr()?;
    listener.set_nonblocking(true)?;
    let (companion, from_sim, to_sim) = pair();
    std::thread::Builder::new()
        .name("companion".into())
        .spawn(move || {
            let mut web = Web::new(domain, accepts, to_sim);
            web.map = map_png(seed);
            serve(listener, web, from_sim)
        })?;
    Ok((companion, bound))
}

fn serve(listener: TcpListener, mut web: Web, mut from_sim: rtrb::Consumer<ToWeb>) {
    loop {
        let now = unix_now();
        while let Ok(m) = from_sim.pop() {
            web.take(m, now);
        }
        if from_sim.is_abandoned() && from_sim.is_empty() {
            return;
        }
        match listener.accept() {
            Ok((stream, _)) => {
                let _ = answer(stream, &mut web, now);
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(_) => std::thread::sleep(Duration::from_millis(50)),
        }
    }
}

/// The island, painted once at start the way the game's map paints it.
fn map_png(seed: u64) -> Vec<u8> {
    let haven = sim_core::terrain::haven(seed);
    let mut rgba = vec![0u8; MAP_PX * MAP_PX * 4];
    client::ui::map::paint(seed, &haven, MAP_PX, &mut rgba);
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, MAP_PX as u32, MAP_PX as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let ok = enc
        .write_header()
        .and_then(|mut w| w.write_image_data(&rgba));
    if ok.is_err() {
        out.clear();
    }
    out
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// One parsed request.
#[derive(Debug, Default)]
pub struct Request {
    pub method: String,
    pub path: String,
    pub query: String,
    pub bearer: Option<String>,
    pub body: Vec<u8>,
}

/// One response.
#[derive(Debug)]
pub struct Response {
    pub status: u16,
    pub ctype: &'static str,
    pub body: Vec<u8>,
}

impl Response {
    fn json(status: u16, v: serde_json::Value) -> Self {
        Response {
            status,
            ctype: "application/json",
            body: v.to_string().into_bytes(),
        }
    }

    fn error(status: u16, why: &str) -> Self {
        Self::json(status, serde_json::json!({ "error": why }))
    }

    fn file(ctype: &'static str, body: &[u8]) -> Self {
        Response {
            status: 200,
            ctype,
            body: body.to_vec(),
        }
    }

    fn empty(status: u16) -> Self {
        Response {
            status,
            ctype: "text/plain",
            body: Vec::new(),
        }
    }
}

const PAGE: &str = include_str!("companion/index.html");
const SCRIPT: &str = include_str!("companion/companion.js");
const STYLE: &str = include_str!("companion/companion.css");

/// The companion thread's state. Everything it knows came over the ring.
pub struct Web {
    domain: String,
    accepts: Vec<String>,
    snap: Snap,
    lines: VecDeque<(u64, Line)>,
    alerts: VecDeque<(u64, Alert)>,
    /// Signs this process's challenges: a nonce is 16 random bytes and 16
    /// of `keccak(secret, random, issued_at)`, so nothing is kept per
    /// challenge and nobody can flood one out of a table.
    secret: [u8; 32],
    /// nonce → when it stops mattering
    used: BTreeMap<[u8; NONCE_BYTES], u64>,
    /// token → (wallet, expires)
    sessions: BTreeMap<String, (String, u64)>,
    /// wallet → (tokens, unix ms of the last refill)
    buckets: BTreeMap<String, (u32, u64)>,
    map: Vec<u8>,
    to_sim: rtrb::Producer<WebLine>,
}

impl Web {
    pub fn new(domain: String, accepts: Vec<String>, to_sim: rtrb::Producer<WebLine>) -> Self {
        let mut secret = [0u8; 32];
        // No entropy leaves the secret zero; `challenge` refuses then anyway.
        let _ = getrandom::getrandom(&mut secret);
        Web {
            domain,
            accepts,
            snap: Snap::default(),
            lines: VecDeque::new(),
            alerts: VecDeque::new(),
            secret,
            used: BTreeMap::new(),
            sessions: BTreeMap::new(),
            buckets: BTreeMap::new(),
            map: Vec::new(),
            to_sim,
        }
    }

    pub fn take(&mut self, m: ToWeb, now: u64) {
        match m {
            ToWeb::Snap(s) => self.snap = s,
            ToWeb::Line(l) => {
                if self.lines.len() == LINES_KEPT {
                    self.lines.pop_front();
                }
                self.lines.push_back((now, l));
            }
            ToWeb::Alert(a) => {
                if self.alerts.len() == ALERTS_KEPT {
                    self.alerts.pop_front();
                }
                self.alerts.push_back((now, a));
            }
        }
    }

    pub fn handle(&mut self, req: &Request, now: u64) -> Response {
        let path = req.path.trim_start_matches('/');
        match (req.method.as_str(), path) {
            ("OPTIONS", _) => Response::empty(204),
            ("GET", "" | "index.html") => {
                Response::file("text/html; charset=utf-8", PAGE.as_bytes())
            }
            ("GET", "companion.js") => Response::file("text/javascript", SCRIPT.as_bytes()),
            ("GET", "companion.css") => Response::file("text/css", STYLE.as_bytes()),
            ("GET", "map.png") if !self.map.is_empty() => Response::file("image/png", &self.map),
            ("GET", "api/challenge") => self.challenge(&req.query, now),
            ("POST", "api/login") => self.login(&req.body, now),
            ("GET", "api/team") => match self.who(req, now) {
                Some(w) => Response::json(200, self.team(&w, now)),
                None => Response::error(401, "sign in"),
            },
            ("POST", "api/chat") => match self.who(req, now) {
                Some(w) => self.chat(&w, &req.body, now),
                None => Response::error(401, "sign in"),
            },
            ("POST", "api/logout") => {
                if let Some(t) = &req.bearer {
                    self.sessions.remove(t);
                }
                Response::empty(204)
            }
            _ => Response::error(404, "not found"),
        }
    }

    /// The wallet a request's token is signed in as.
    fn who(&mut self, req: &Request, now: u64) -> Option<String> {
        let t = req.bearer.as_ref()?;
        match self.sessions.get(t) {
            Some((w, until)) if *until > now => Some(w.clone()),
            Some(_) => {
                self.sessions.remove(t);
                None
            }
            None => None,
        }
    }

    fn challenge(&mut self, query: &str, now: u64) -> Response {
        let Some(address) = param(query, "address").and_then(|a| Address::from_hex(a.as_bytes()))
        else {
            return Response::error(400, "address");
        };
        if address.is_guest() {
            return Response::error(400, "address");
        }
        let domain = param(query, "domain")
            .filter(|d| self.accepts.iter().any(|a| a == d))
            .unwrap_or(&self.domain)
            .to_string();
        let mut nonce = [0u8; NONCE_BYTES];
        if getrandom::getrandom(&mut nonce[..16]).is_err() {
            return Response::error(503, "no entropy");
        }
        let mac = self.mac(&nonce, now);
        nonce[16..].copy_from_slice(&mac);
        let sum = address.to_checksum_hex();
        let addr = core::str::from_utf8(&sum).unwrap_or("0x");
        let mut text = [0u8; SIWE_MESSAGE_MAX];
        let n = protocol::siwe_message_for(
            &domain,
            &self.domain,
            addr,
            protocol::SLUG,
            &nonce,
            now,
            &mut text,
        )
        .min(SIWE_MESSAGE_MAX);
        Response::json(
            200,
            serde_json::json!({
                "domain": domain,
                "nonce": hex(&nonce),
                "issued_at": now,
                "message": String::from_utf8_lossy(&text[..n]),
            }),
        )
    }

    fn login(&mut self, body: &[u8], now: u64) -> Response {
        let Ok(v) = serde_json::from_slice::<serde_json::Value>(body) else {
            return Response::error(400, "json");
        };
        let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("");
        let Some(address) = Address::from_hex(s("address").as_bytes()) else {
            return Response::error(400, "address");
        };
        let (Some(nonce), Some(sig)) = (
            unhex::<NONCE_BYTES>(s("nonce")),
            unhex::<SIGNATURE_BYTES>(s("signature")),
        ) else {
            return Response::error(400, "nonce or signature");
        };
        let issued_at = v.get("issued_at").and_then(|x| x.as_u64()).unwrap_or(0);
        if nonce[16..] != self.mac(&nonce, issued_at) {
            return Response::error(401, "not this shard's challenge");
        }
        if now > issued_at + CHALLENGE_SECS || issued_at > now + 5 {
            return Response::error(401, "challenge expired");
        }
        self.used.retain(|_, until| *until > now);
        if self.used.contains_key(&nonce) {
            return Response::error(401, "challenge already used");
        }
        if self.used.len() >= USED_KEPT {
            return Response::error(503, "busy");
        }
        let auth = protocol::Auth {
            address,
            signature: Signature(sig),
        };
        let key = match crate::auth::verify_any(&self.accepts, &nonce, issued_at, &auth) {
            Ok(k) => k,
            Err(e) => return Response::error(401, e.reason()),
        };
        let wallet = String::from_utf8_lossy(key.as_bytes()).to_ascii_lowercase();
        // Only a wallet with a body on the island signs in here, which keeps
        // the wallets that can hold sessions to the island's own population.
        if !self.snap.people.iter().any(|p| p.wallet == wallet) {
            return Response::error(403, "not on the island");
        }
        // Spent only now, by a sign-in that lands: spent before the island
        // check, throwaway wallets (free to make) filled `used` and every
        // real sign-in met "busy" until their challenges expired.
        self.used.insert(nonce, issued_at + CHALLENGE_SECS + 5);
        let mut raw = [0u8; 32];
        if getrandom::getrandom(&mut raw).is_err() {
            return Response::error(503, "no entropy");
        }
        let token = hex(&raw);
        self.sessions.retain(|_, (_, until)| *until > now);
        let mine = self.sessions.values().filter(|(w, _)| *w == wallet).count();
        if mine >= SESSIONS_PER_WALLET || self.sessions.len() >= SESSIONS_KEPT {
            let only_mine = mine >= SESSIONS_PER_WALLET;
            let oldest = self
                .sessions
                .iter()
                .filter(|(_, (w, _))| !only_mine || *w == wallet)
                .min_by_key(|(_, (_, until))| *until)
                .map(|(t, _)| t.clone());
            if let Some(t) = oldest {
                self.sessions.remove(&t);
            }
        }
        let expires = now + SESSION_SECS;
        self.sessions
            .insert(token.clone(), (wallet.clone(), expires));
        Response::json(
            200,
            serde_json::json!({ "token": token, "wallet": wallet, "expires": expires }),
        )
    }

    /// The 16 bytes that make `nonce`'s first 16 this process's challenge
    /// issued at `issued_at`.
    fn mac(&self, nonce: &[u8; NONCE_BYTES], issued_at: u64) -> [u8; 16] {
        use tiny_keccak::{Hasher, Keccak};
        let mut k = Keccak::v256();
        let mut out = [0u8; 32];
        k.update(&self.secret);
        k.update(&nonce[..16]);
        k.update(&issued_at.to_be_bytes());
        k.finalize(&mut out);
        let mut mac = [0u8; 16];
        mac.copy_from_slice(&out[..16]);
        mac
    }

    /// Everything `wallet` may see.
    pub fn team(&self, wallet: &str, now: u64) -> serde_json::Value {
        let me = self.snap.people.iter().find(|p| p.wallet == wallet);
        let team: Vec<&Person> = match me {
            Some(m) if m.team != 0 => self
                .snap
                .people
                .iter()
                .filter(|p| p.team == m.team)
                .collect(),
            Some(m) => vec![m],
            None => Vec::new(),
        };
        let bases: Vec<serde_json::Value> = match me {
            Some(m) => self
                .snap
                .bases
                .iter()
                .filter(|b| b.crew.contains(&m.id))
                .map(|b| place(b.x_m, b.z_m))
                .collect(),
            None => Vec::new(),
        };
        let chat: Vec<serde_json::Value> = {
            let mut heard: Vec<_> = self
                .lines
                .iter()
                .rev()
                .filter(|(_, l)| l.hearers.iter().any(|h| h == wallet))
                .take(LINES_SHOWN)
                .map(|(at, l)| {
                    serde_json::json!({
                        "at": at, "wallet": l.from, "name": l.name,
                        "text": l.text, "web": l.web,
                    })
                })
                .collect();
            heard.reverse();
            heard
        };
        let alerts: Vec<serde_json::Value> = self
            .alerts
            .iter()
            .rev()
            .filter(|(_, a)| a.wallets.iter().any(|w| w == wallet))
            .take(ALERTS_SHOWN)
            .map(|(at, a)| {
                let mut v = place(a.x_m, a.z_m);
                v["at"] = (*at).into();
                v["kind"] = "raid".into();
                v
            })
            .collect();
        serde_json::json!({
            "at": now,
            "tick": self.snap.tick,
            "day": sim_core::world::day_frac(self.snap.tick),
            "night": sim_core::world::is_night(self.snap.tick),
            "players": self.snap.players,
            "wallet": wallet,
            "me": me.map(person),
            "team": team.into_iter().map(person).collect::<Vec<_>>(),
            "bases": bases,
            "chat": chat,
            "alerts": alerts,
        })
    }

    fn chat(&mut self, wallet: &str, body: &[u8], now: u64) -> Response {
        let text = serde_json::from_slice::<serde_json::Value>(body)
            .ok()
            .and_then(|v| v.get("text").and_then(|t| t.as_str()).map(str::to_string))
            .unwrap_or_default();
        if text.trim_start().starts_with('/') {
            return Response::error(400, "commands are for the game");
        }
        let Some(line) = ChatText::sanitize(text.as_bytes()) else {
            return Response::error(400, "text");
        };
        if !self
            .snap
            .people
            .iter()
            .any(|p| p.wallet == wallet && p.team != 0)
        {
            return Response::error(409, "no team");
        }
        let ms = now * 1000;
        let (tokens, at) = self
            .buckets
            .entry(wallet.to_string())
            .or_insert((protocol::chat::CHAT_BURST, ms));
        let refill = ms.saturating_sub(*at) / protocol::chat::CHAT_REFILL_MS;
        if refill > 0 {
            *tokens = (*tokens + refill as u32).min(protocol::chat::CHAT_BURST);
            *at = ms;
        }
        if *tokens == 0 {
            return Response::error(429, "slow down");
        }
        *tokens -= 1;
        let Some(key) = PlayerKey::new(wallet.as_bytes()) else {
            return Response::error(400, "wallet");
        };
        match self.to_sim.push(WebLine {
            wallet: key,
            text: line,
        }) {
            Ok(()) => Response::empty(202),
            Err(_) => Response::error(503, "busy"),
        }
    }
}

fn person(p: &Person) -> serde_json::Value {
    let mut v = place(p.x_m, p.z_m);
    v["wallet"] = p.wallet.clone().into();
    v["name"] = p.name.clone().into();
    v["online"] = p.online.into();
    v["leader"] = p.leader.into();
    v["state"] = match p.state {
        protocol::TEAM_AWAKE => "awake",
        protocol::TEAM_DEAD => "dead",
        _ => "asleep",
    }
    .into();
    v
}

fn place(x_m: u16, z_m: u16) -> serde_json::Value {
    let (fx, fz) = client::ui::map::world_to_map(x_m as f32, z_m as f32, 1);
    serde_json::json!({
        "x": x_m,
        "z": z_m,
        "grid": client::ui::map::grid_label(x_m as f32, z_m as f32),
        // Where on the map image, 0..1 from the top-left.
        "u": fx,
        "v": fz,
    })
}

/// `key`'s value in a query string, undecoded (the values here are hex and
/// host names, which need none).
fn param<'a>(query: &'a str, key: &str) -> Option<&'a str> {
    query
        .split('&')
        .filter_map(|kv| kv.split_once('='))
        .find(|(k, _)| *k == key)
        .map(|(_, v)| v)
}

fn hex(b: &[u8]) -> String {
    let mut s = String::with_capacity(b.len() * 2);
    for x in b {
        s.push_str(&format!("{x:02x}"));
    }
    s
}

fn unhex<const N: usize>(s: &str) -> Option<[u8; N]> {
    let s = s.strip_prefix("0x").unwrap_or(s).as_bytes();
    if s.len() != N * 2 {
        return None;
    }
    let mut out = [0u8; N];
    for (i, o) in out.iter_mut().enumerate() {
        let d = |c: u8| (c as char).to_digit(16).map(|v| v as u8);
        *o = (d(s[2 * i])? << 4) | d(s[2 * i + 1])?;
    }
    Some(out)
}

/// A connection with one deadline for everything said on it (`status.rs`'s
/// reason: a per-call timeout is renewed by a client trickling bytes).
struct Deadlined {
    s: TcpStream,
    until: Instant,
}

impl Deadlined {
    fn left(&self) -> std::io::Result<Duration> {
        let left = self.until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "companion deadline",
            ));
        }
        Ok(left)
    }
}

impl Read for Deadlined {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let left = self.left()?;
        self.s.set_read_timeout(Some(left))?;
        self.s.read(buf)
    }
}

impl Write for Deadlined {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let left = self.left()?;
        self.s.set_write_timeout(Some(left))?;
        self.s.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.s.flush()
    }
}

/// Read one request, bounded, answer it, close.
fn answer(stream: TcpStream, web: &mut Web, now: u64) -> std::io::Result<()> {
    stream.set_nonblocking(false)?;
    let mut s = Deadlined {
        s: stream,
        until: Instant::now() + IO_TIMEOUT,
    };
    let mut buf = vec![0u8; REQUEST_CAP];
    let mut have = 0;
    let head_end = loop {
        if let Some(at) = buf[..have].windows(4).position(|w| w == b"\r\n\r\n") {
            break at;
        }
        if have == buf.len() {
            return Ok(());
        }
        let n = s.read(&mut buf[have..])?;
        if n == 0 {
            return Ok(());
        }
        have += n;
    };
    let Some(mut req) = parse_head(&buf[..head_end]) else {
        return Ok(());
    };
    let body_at = head_end + 4;
    // Checked: a `Content-Length` near `usize::MAX` overflowed the sum, a
    // panic that took the companion's thread down.
    let Some(end) = body_at.checked_add(req.1).filter(|&e| e <= buf.len()) else {
        return Ok(());
    };
    while have < end {
        let n = s.read(&mut buf[have..end])?;
        if n == 0 {
            return Ok(());
        }
        have += n;
    }
    req.0.body = buf[body_at..end].to_vec();
    let r = web.handle(&req.0, now);
    let reason = match r.status {
        200 => "OK",
        202 => "Accepted",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        409 => "Conflict",
        429 => "Too Many Requests",
        _ => "Service Unavailable",
    };
    let head = format!(
        "HTTP/1.1 {} {reason}\r\nContent-Type: {}\r\nContent-Length: {}\r\n\
         Cache-Control: no-store\r\nAccess-Control-Allow-Origin: *\r\n\
         Access-Control-Allow-Headers: Authorization, Content-Type\r\n\
         Access-Control-Allow-Methods: GET, POST, OPTIONS\r\nConnection: close\r\n\r\n",
        r.status,
        r.ctype,
        r.body.len()
    );
    s.write_all(head.as_bytes())?;
    s.write_all(&r.body)
}

/// The request line and the two headers this reads, and the body's length.
fn parse_head(head: &[u8]) -> Option<(Request, usize)> {
    let text = std::str::from_utf8(head).ok()?;
    let mut lines = text.split("\r\n");
    let mut first = lines.next()?.split(' ');
    let method = first.next()?.to_string();
    let target = first.next()?;
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let mut req = Request {
        method,
        path: path.to_string(),
        query: query.to_string(),
        ..Default::default()
    };
    let mut len = 0usize;
    for l in lines {
        let Some((k, v)) = l.split_once(':') else {
            continue;
        };
        let v = v.trim();
        if k.eq_ignore_ascii_case("content-length") {
            len = v.parse().ok()?;
        } else if k.eq_ignore_ascii_case("authorization") {
            req.bearer = v.strip_prefix("Bearer ").map(|t| t.trim().to_string());
        }
    }
    Some((req, len))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn web() -> (Web, rtrb::Consumer<WebLine>) {
        let (tx, rx) = rtrb::RingBuffer::new(8);
        let d = "game.test".to_string();
        (Web::new(d.clone(), vec![d, "site.test".into()], tx), rx)
    }

    fn get(path: &str, bearer: Option<&str>) -> Request {
        let (path, query) = path.split_once('?').unwrap_or((path, ""));
        Request {
            method: "GET".into(),
            path: path.into(),
            query: query.into(),
            bearer: bearer.map(str::to_string),
            body: Vec::new(),
        }
    }

    fn post(path: &str, bearer: Option<&str>, body: serde_json::Value) -> Request {
        Request {
            method: "POST".into(),
            path: path.into(),
            bearer: bearer.map(str::to_string),
            body: body.to_string().into_bytes(),
            ..Default::default()
        }
    }

    fn body(r: &Response) -> serde_json::Value {
        serde_json::from_slice(&r.body).unwrap()
    }

    fn addr(key: &agentkey::AgentKey) -> String {
        std::str::from_utf8(&key.address().to_hex()).unwrap().into()
    }

    /// The island as a snapshot: `(wallet, team leader id)` per body.
    fn island(w: &mut Web, people: &[(&str, u32)]) {
        let people = people
            .iter()
            .enumerate()
            .map(|(i, (wallet, team))| who(wallet, i as u32 + 1, *team))
            .collect();
        w.take(
            ToWeb::Snap(Snap {
                people,
                ..Default::default()
            }),
            0,
        );
    }

    /// A signed login body for `key`, the way an agent makes one.
    fn login_body(w: &mut Web, key: &agentkey::AgentKey, now: u64) -> serde_json::Value {
        let a = addr(key);
        let c = body(&w.handle(&get(&format!("/api/challenge?address={a}"), None), now));
        let nonce = c["nonce"].as_str().unwrap();
        let issued = c["issued_at"].as_u64().unwrap();
        let sig = key
            .sign_proof_hex(c["domain"].as_str().unwrap(), nonce, issued)
            .unwrap();
        serde_json::json!({"address": a, "nonce": nonce, "issued_at": issued, "signature": hex(&sig.0)})
    }

    /// Sign in as `key` through the API; returns the token.
    fn sign_in(w: &mut Web, key: &agentkey::AgentKey, now: u64) -> String {
        let login = login_body(w, key, now);
        let r = w.handle(&post("/api/login", None, login), now);
        assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
        let v = body(&r);
        assert_eq!(v["wallet"], addr(key));
        v["token"].as_str().unwrap().into()
    }

    fn key(b: u8) -> agentkey::AgentKey {
        let mut s = [0u8; 32];
        s[31] = b;
        agentkey::AgentKey::from_secret(&s).unwrap()
    }

    fn who(wallet: &str, id: u32, team: u32) -> Person {
        Person {
            wallet: wallet.into(),
            name: format!("p{id}"),
            id,
            state: protocol::TEAM_AWAKE,
            x_m: 100 * id as u16,
            z_m: 50,
            online: true,
            team,
            leader: team == id,
        }
    }

    /// A signature over the challenge signs in once and only as its signer;
    /// a nonce this shard did not issue, or issued too long ago, is refused.
    #[test]
    fn a_challenge_signs_in_once_and_only_as_its_signer() {
        let (mut w, _rx) = web();
        island(&mut w, &[(&addr(&key(7)), 0)]);
        let token = sign_in(&mut w, &key(7), 1000);
        assert_eq!(w.handle(&get("/api/team", Some(&token)), 1001).status, 200);
        assert_eq!(w.handle(&get("/api/team", Some("nope")), 1001).status, 401);

        // The same signed body twice: the second is a replay.
        let login = login_body(&mut w, &key(7), 1000);
        assert_eq!(
            w.handle(&post("/api/login", None, login.clone()), 1000)
                .status,
            200
        );
        assert_eq!(
            w.handle(&post("/api/login", None, login.clone()), 1000)
                .status,
            401
        );

        // Too late, and a nonce whose second half this shard did not write.
        let login = login_body(&mut w, &key(7), 1000);
        assert_eq!(
            w.handle(
                &post("/api/login", None, login.clone()),
                1000 + CHALLENGE_SECS + 1
            )
            .status,
            401
        );
        let mut forged = login.clone();
        let mut n = login["nonce"].as_str().unwrap().to_string();
        n.replace_range(63.., if n.ends_with('0') { "1" } else { "0" });
        forged["nonce"] = n.into();
        assert_eq!(
            w.handle(&post("/api/login", None, forged), 1000).status,
            401
        );

        // Someone else's signature over my challenge recovers them, not me.
        let mut theirs = login_body(&mut w, &key(7), 1000);
        let sig = key(8)
            .sign_proof_hex("game.test", theirs["nonce"].as_str().unwrap(), 1000)
            .unwrap();
        theirs["signature"] = hex(&sig.0).into();
        assert_eq!(
            w.handle(&post("/api/login", None, theirs), 1000).status,
            401
        );
    }

    /// Signing in needs a body on the island, and one wallet holds a bounded
    /// number of sessions: the oldest goes first.
    #[test]
    fn sign_in_needs_a_body_and_sessions_are_bounded_per_wallet() {
        let (mut w, _rx) = web();
        let login = login_body(&mut w, &key(7), 1000);
        assert_eq!(w.handle(&post("/api/login", None, login), 1000).status, 403);
        // A refused wallet spends no room in the replay table: throwaway
        // wallets cannot fill it and lock everyone else out.
        assert!(w.used.is_empty());
        island(&mut w, &[(&addr(&key(7)), 0)]);
        let first = sign_in(&mut w, &key(7), 1000);
        let mut last = String::new();
        for i in 0..SESSIONS_PER_WALLET as u64 {
            last = sign_in(&mut w, &key(7), 1001 + i);
        }
        assert_eq!(w.handle(&get("/api/team", Some(&first)), 1100).status, 401);
        assert_eq!(w.handle(&get("/api/team", Some(&last)), 1100).status, 200);
    }

    /// The page's own host is signed when the shard accepts it; anything else
    /// falls back to the shard's name.
    #[test]
    fn the_domain_line_is_the_pages_only_when_accepted() {
        let (mut w, _rx) = web();
        let a = "0x00000000000000000000000000000000000000aa";
        let c = body(&w.handle(
            &get(
                &format!("/api/challenge?address={a}&domain=site.test"),
                None,
            ),
            1,
        ));
        assert_eq!(c["domain"], "site.test");
        assert!(c["message"]
            .as_str()
            .unwrap()
            .starts_with("site.test wants you"));
        assert!(c["message"]
            .as_str()
            .unwrap()
            .contains("URI: https://game.test"));
        let c = body(&w.handle(
            &get(
                &format!("/api/challenge?address={a}&domain=evil.test"),
                None,
            ),
            1,
        ));
        assert_eq!(c["domain"], "game.test");
    }

    /// You see your team, your bases, your lines and your alerts, and
    /// nothing of anyone else's.
    #[test]
    fn a_wallet_sees_its_own_team_and_nothing_else() {
        let (mut w, _rx) = web();
        let me = addr(&key(7));
        w.take(
            ToWeb::Snap(Snap {
                tick: 10,
                players: 3,
                people: vec![who(&me, 1, 1), who("0xbb", 2, 1), who("0xcc", 3, 0)],
                bases: vec![
                    Base {
                        x_m: 10,
                        z_m: 10,
                        crew: vec![1],
                    },
                    Base {
                        x_m: 20,
                        z_m: 20,
                        crew: vec![3],
                    },
                ],
            }),
            1000,
        );
        let token = sign_in(&mut w, &key(7), 1000);
        let heard = |h: &[&str], text: &str| {
            ToWeb::Line(Line {
                from: "0xbb".into(),
                name: "p2".into(),
                text: text.into(),
                hearers: h.iter().map(|s| s.to_string()).collect(),
                web: false,
            })
        };
        w.take(heard(&[&me, "0xbb"], "ours"), 1000);
        w.take(heard(&["0xcc"], "theirs"), 1000);
        w.take(
            ToWeb::Alert(Alert {
                x_m: 20,
                z_m: 20,
                wallets: vec!["0xcc".into()],
            }),
            1000,
        );
        let v = body(&w.handle(&get("/api/team", Some(&token)), 1001));
        let team: Vec<_> = v["team"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["wallet"].clone())
            .collect();
        assert_eq!(team, vec![serde_json::json!(me), serde_json::json!("0xbb")]);
        assert_eq!(v["bases"].as_array().unwrap().len(), 1);
        assert_eq!(v["bases"][0]["x"], 10);
        let lines: Vec<_> = v["chat"]
            .as_array()
            .unwrap()
            .iter()
            .map(|l| l["text"].clone())
            .collect();
        assert_eq!(lines, vec![serde_json::json!("ours")]);
        assert!(v["alerts"].as_array().unwrap().is_empty());
    }

    /// A line typed on the page goes to the sim only for a wallet on a team,
    /// never as a command, and at the game's own chat rate.
    #[test]
    fn page_chat_needs_a_team_and_keeps_the_games_rate() {
        let (mut w, mut rx) = web();
        let me = addr(&key(7));
        island(&mut w, &[(&me, 0)]);
        let token = sign_in(&mut w, &key(7), 1000);
        let say = |w: &mut Web, t: &str| {
            w.handle(
                &post("/api/chat", Some(&token), serde_json::json!({"text": t})),
                1000,
            )
            .status
        };
        assert_eq!(say(&mut w, "hi"), 409, "no team");
        island(&mut w, &[(&me, 1)]);
        assert_eq!(say(&mut w, "/kick 2"), 400);
        for _ in 0..protocol::chat::CHAT_BURST {
            assert_eq!(say(&mut w, "wood"), 202);
        }
        assert_eq!(say(&mut w, "wood"), 429);
        let l = rx.pop().unwrap();
        assert_eq!(l.text.as_bytes(), b"wood");
        assert_eq!(l.wallet.as_bytes(), me.as_bytes());
    }

    #[test]
    fn a_request_head_parses_its_token_and_length() {
        let head = b"POST /api/chat?x=1 HTTP/1.1\r\nHost: a\r\nauthorization: Bearer abc\r\nContent-Length: 12";
        let (r, len) = parse_head(head).unwrap();
        assert_eq!(
            (r.method.as_str(), r.path.as_str(), r.query.as_str()),
            ("POST", "/api/chat", "x=1")
        );
        assert_eq!(r.bearer.as_deref(), Some("abc"));
        assert_eq!(len, 12);
    }
}
