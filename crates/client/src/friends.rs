//! Friends from the Elo Pros platform: the list a wallet keeps there, who on
//! it is playing Gates and on which shard, and the signed text that asks,
//! accepts, declines, cancels, removes, blocks or unblocks one, or invites a
//! friend into the game.
//!
//! The platform owns the list (`docs/FRIENDS.md` in
//! `AnthonE/elo-pros-launcher`; its `meter/friends.py`):
//! `GET /api/friends/{wallet}` is public, and every change is prepare → sign →
//! act. Presence comes from the
//! shards' own rosters, never from what a client says, so it lags by about a
//! minute and a half.
//!
//! Pure and on every target, wasm included: parsing, the signed text, the
//! name lookup's rules and which shard a friend is on are arithmetic.
//! `render/friends.rs` owns the reads and the signing.

use protocol::Address;
use serde::{Deserialize, Deserializer};

use crate::ui::servers::Listing;

/// Largest `/api/friends/{wallet}` body read, in bytes. 250 friends, 200
/// asks in and 50 out at a few hundred bytes a row, with headroom.
pub const MAX_VIEW_BYTES: usize = 128 * 1024;
/// Largest `/api/who` body read when a name is typed into the add box.
pub const MAX_WHO_BYTES: usize = 64 * 1024;
/// Largest `/api/friends/prepare` answer read.
pub const MAX_PREPARE_BYTES: usize = 8 * 1024;
/// How far the platform's `at` may sit from this clock and still be signed
/// (`FRIENDS.md` §1: within ten minutes of now).
pub const AT_SKEW_S: u64 = 600;
/// A name drawn in a row is cut here: the platform caps its own, and a row is
/// one line.
pub const MAX_NAME_CHARS: usize = 32;
/// The game a friend must be playing for JOIN to mean anything here.
pub const GAME: &str = crate::deeplink::SLUG;
/// Where a friends list is changed without a launcher.
pub const PROFILE_PATH: &str = "/profile.html";

/// One change to the list, and the key its one argument goes under
/// (`meter/friends.py` `OPS`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Ask,
    Accept,
    Decline,
    Cancel,
    Remove,
    Block,
    Unblock,
    /// Steam's "Invite to game": the friend's list shows where you are.
    Invite,
}

impl Op {
    pub const ALL: [Op; 8] = [
        Op::Ask,
        Op::Accept,
        Op::Decline,
        Op::Cancel,
        Op::Remove,
        Op::Block,
        Op::Unblock,
        Op::Invite,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Op::Ask => "ask",
            Op::Accept => "accept",
            Op::Decline => "decline",
            Op::Cancel => "cancel",
            Op::Remove => "remove",
            Op::Block => "block",
            Op::Unblock => "unblock",
            Op::Invite => "invite",
        }
    }

    /// The key the other wallet goes under in `args`.
    pub fn key(self) -> &'static str {
        match self {
            Op::Ask | Op::Cancel | Op::Invite => "to",
            Op::Accept | Op::Decline => "from",
            Op::Remove | Op::Block | Op::Unblock => "wallet",
        }
    }

    pub fn parse(s: &str) -> Option<Op> {
        Op::ALL.into_iter().find(|o| o.name() == s)
    }

    /// What the status line says once the act landed.
    pub fn done(self, view: &View, other: &str) -> &'static str {
        match self {
            Op::Ask if view.friends.iter().any(|f| f.wallet == other) => "You're friends now.",
            Op::Ask => "Asked. They'll see it on their profile.",
            Op::Accept => "You're friends now.",
            Op::Decline => "Declined.",
            Op::Cancel => "Ask withdrawn.",
            Op::Remove => "Removed.",
            Op::Block => "Blocked.",
            Op::Unblock => "Unblocked.",
            Op::Invite => "Invited. They'll see it in Gates.",
        }
    }
}

/// A field the platform may send as `null` reads as its default.
fn or_default<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}

/// `GET /api/friends/{wallet}`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct View {
    #[serde(default, deserialize_with = "or_default")]
    pub wallet: String,
    #[serde(default, deserialize_with = "or_default")]
    pub friends: Vec<Friend>,
    /// The wallet's own AI buddies. Friends of their owner without an ask.
    #[serde(default, deserialize_with = "or_default")]
    pub buddies: Vec<Friend>,
    #[serde(default, deserialize_with = "or_default")]
    pub asks_in: Vec<Ask>,
    #[serde(default, deserialize_with = "or_default")]
    pub asks_out: Vec<Ask>,
    /// Friends asking you into their game: their row, plus `at`.
    #[serde(default, deserialize_with = "or_default")]
    pub invites_in: Vec<Friend>,
    #[serde(default)]
    pub presence: Option<Presence>,
}

/// One friend (or buddy) row.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct Friend {
    #[serde(default, deserialize_with = "or_default")]
    pub wallet: String,
    #[serde(default)]
    pub since: Option<u64>,
    #[serde(default)]
    pub name: Option<String>,
    /// `/api/face/{wallet}/pic.png?v=<rev>`, or null.
    #[serde(default)]
    pub pic: Option<String>,
    #[serde(default, deserialize_with = "or_default")]
    pub agent: bool,
    #[serde(default, deserialize_with = "or_default")]
    pub online: bool,
    #[serde(default)]
    pub playing: Option<Playing>,
    #[serde(default)]
    pub last_online: Option<u64>,
    /// Set on a row of [`View::buddies`].
    #[serde(default)]
    pub buddy: Option<Buddy>,
    /// When the invite was sent, on a row of [`View::invites_in`].
    #[serde(default)]
    pub at: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct Playing {
    #[serde(default)]
    pub game: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    /// The shard row's id in the title's shard list (`eu-1`).
    #[serde(default)]
    pub server: Option<String>,
    #[serde(default)]
    pub server_name: Option<String>,
    #[serde(default)]
    pub since: Option<u64>,
    /// `elo://join/gates/host:port`, or null when the list names no address.
    #[serde(default)]
    pub join: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct Buddy {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
}

/// A pending ask, either way.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct Ask {
    #[serde(default, deserialize_with = "or_default")]
    pub wallet: String,
    #[serde(default)]
    pub at: Option<u64>,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct Presence {
    #[serde(default)]
    pub reachable: Option<bool>,
    #[serde(default)]
    pub why: Option<String>,
}

/// A wallet as the platform writes it: `0x` and forty hex digits.
pub fn wallet_ok(s: &str) -> bool {
    Address::from_hex(s.as_bytes()).is_some()
}

fn tidy_name(n: &mut Option<String>) {
    *n = n
        .take()
        .map(|s| s.trim().chars().take(MAX_NAME_CHARS).collect::<String>())
        .filter(|s| !s.is_empty() && !s.chars().any(char::is_control));
}

/// Parse a list. Rows with no readable wallet are dropped and every wallet is
/// lowercased, so a row can be keyed and compared as text.
pub fn parse(bytes: &[u8]) -> Result<View, String> {
    if bytes.len() > MAX_VIEW_BYTES {
        return Err(format!(
            "the friends list is {} bytes, over the {MAX_VIEW_BYTES}-byte cap",
            bytes.len()
        ));
    }
    let v: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|e| format!("friends list: {e}"))?;
    view_of(v)
}

/// [`parse`] for a body already read as JSON (the act's `view`).
pub fn view_of(v: serde_json::Value) -> Result<View, String> {
    if !v.is_object() {
        return Err("friends list: not an object".into());
    }
    let mut view: View = serde_json::from_value(v).map_err(|e| format!("friends list: {e}"))?;
    view.wallet.make_ascii_lowercase();
    for rows in [&mut view.friends, &mut view.buddies, &mut view.invites_in] {
        rows.retain(|f| wallet_ok(&f.wallet));
        for f in rows.iter_mut() {
            f.wallet.make_ascii_lowercase();
            tidy_name(&mut f.name);
        }
    }
    for rows in [&mut view.asks_in, &mut view.asks_out] {
        rows.retain(|a| wallet_ok(&a.wallet));
        for a in rows.iter_mut() {
            a.wallet.make_ascii_lowercase();
            tidy_name(&mut a.name);
        }
    }
    Ok(view)
}

impl View {
    /// Every friend, then every buddy that is not also a friend: the rows a
    /// panel draws and the set a nametag is checked against.
    pub fn everyone(&self) -> impl Iterator<Item = &Friend> + '_ {
        self.friends.iter().chain(
            self.buddies
                .iter()
                .filter(|b| !self.friends.iter().any(|f| f.wallet == b.wallet)),
        )
    }

    /// The platform could not see the shards' rosters, so nobody reads online.
    pub fn blind(&self) -> bool {
        self.presence
            .as_ref()
            .is_some_and(|p| p.reachable == Some(false))
    }

    pub fn online(&self) -> usize {
        self.everyone().filter(|f| f.online).count()
    }

    pub fn find(&self, wallet: &Address) -> Option<&Friend> {
        self.everyone().find(|f| f.address() == Some(*wallet))
    }
}

/// What changed between two reads of one list that a player would want told
/// in the game, Steam's corner popups: a new ask, a new invite, a friend
/// starting to play Gates. Nothing on a blind read, which says nobody is on.
pub fn notices(old: &View, new: &View) -> Vec<String> {
    let mut out = Vec::new();
    for a in &new.asks_in {
        if !old.asks_in.iter().any(|o| o.wallet == a.wallet) {
            let who = Friend {
                wallet: a.wallet.clone(),
                name: a.name.clone(),
                ..Default::default()
            };
            out.push(format!("{} wants to be your friend", who.label()));
        }
    }
    for i in &new.invites_in {
        if !old
            .invites_in
            .iter()
            .any(|o| o.wallet == i.wallet && o.at == i.at)
        {
            out.push(format!("{} invited you to play - Esc to join", i.label()));
        }
    }
    if new.blind() || old.blind() {
        return out;
    }
    for f in new.everyone() {
        let was = old
            .everyone()
            .find(|o| o.wallet == f.wallet)
            .and_then(Friend::on_gates)
            .is_some();
        if f.on_gates().is_some() && !was {
            out.push(format!("{} is now playing Gates", f.label()));
        }
    }
    out
}

impl Friend {
    pub fn address(&self) -> Option<Address> {
        Address::from_hex(self.wallet.as_bytes())
    }

    pub fn is_buddy(&self) -> bool {
        self.buddy.is_some()
    }

    /// The name, else the short wallet, marked when it is a program: your own
    /// buddy as yours, anyone else's agent the way a nametag marks one.
    pub fn label(&self) -> String {
        let name = if crate::ui::streamer::on() {
            crate::ui::streamer::alias(&self.wallet.to_ascii_lowercase())
        } else {
            self.name
                .clone()
                .unwrap_or_else(|| short_wallet(&self.wallet))
        };
        if self.is_buddy() {
            format!("{name} (your agent)")
        } else if self.agent {
            format!("{name} (agent)")
        } else {
            name
        }
    }

    /// Where this friend is playing Gates, if they are.
    pub fn on_gates(&self) -> Option<&Playing> {
        self.playing
            .as_ref()
            .filter(|p| self.online && p.game.as_deref() == Some(GAME))
    }

    /// The picture's revision, for `render::faces::Pics`.
    pub fn pic_rev(&self) -> Option<u32> {
        self.pic.as_deref().and_then(pic_rev)
    }
}

/// `0x1234…abcd`, the way every surface in this client shortens a wallet.
pub fn short_wallet(w: &str) -> String {
    if crate::ui::streamer::on() {
        return crate::ui::streamer::HIDDEN.to_string();
    }
    short_wallet_real(w)
}

/// [`short_wallet`] whatever streamer mode says, for text a launcher shows
/// beside a signature rather than text drawn here.
pub fn short_wallet_real(w: &str) -> String {
    match Address::from_hex(w.as_bytes()) {
        Some(a) => crate::ui::spectate::short_real(&a),
        None => w.chars().take(10).collect(),
    }
}

/// `3h ago`, floored.
pub fn ago(secs: u64) -> String {
    match secs {
        s if s < 60 => "just now".into(),
        s if s < 3_600 => format!("{}m ago", s / 60),
        s if s < 86_400 => format!("{}h ago", s / 3_600),
        s => format!("{}d ago", s / 86_400),
    }
}

/// The line under a friend's name: `Playing Gates · Gates US East 1`,
/// `online`, `last seen 3h ago`, or `offline`.
pub fn presence_line(f: &Friend, now: u64) -> String {
    if f.online {
        if let Some(p) = &f.playing {
            let title = p.title.as_deref().or(p.game.as_deref()).unwrap_or("a game");
            // Streamer mode names the game and never the server: where a
            // friend is is where a viewer would go looking.
            if crate::ui::streamer::on() {
                return format!("Playing {title}");
            }
            return match p.server_name.as_deref().or(p.server.as_deref()) {
                Some(s) => format!("Playing {title} · {s}"),
                None => format!("Playing {title}"),
            };
        }
        return "online".into();
    }
    match f.last_online {
        Some(t) => format!("last seen {}", ago(now.saturating_sub(t))),
        None => "offline".into(),
    }
}

/// The Esc menu's block: `Friends online 2`, then up to `max` of them as
/// `name · where`, then how many more.
pub fn online_lines(view: &View, now: u64, max: usize) -> Vec<String> {
    let on: Vec<&Friend> = view.everyone().filter(|f| f.online).collect();
    let mut out = vec![format!("Friends online {}", on.len())];
    if view.blind() {
        out.push("Elo Pros can't see who's playing right now".into());
    }
    out.extend(
        on.iter()
            .take(max)
            .map(|f| format!("{} · {}", f.label(), presence_line(f, now))),
    );
    if on.len() > max {
        out.push(format!("+ {} more", on.len() - max));
    }
    out
}

/// The picture revision in a `pic` url's `?v=`, as the hex the platform
/// writes (`faces.py`: the first eight digits of the picture's sha256).
pub fn pic_rev(url: &str) -> Option<u32> {
    let q = url.split_once('?')?.1;
    let v = q
        .split('&')
        .find_map(|kv| kv.strip_prefix("v="))?
        .split('#')
        .next()?;
    if v.is_empty() || v.len() > 8 {
        return None;
    }
    u32::from_str_radix(v, 16).ok().map(|r| r.max(1))
}

/// Friends and buddies by wallet, for a nametag or a chat line.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FriendSet(Vec<Address>);

impl FriendSet {
    pub fn of(view: &View) -> Self {
        let mut v: Vec<Address> = view
            .everyone()
            .filter_map(Friend::address)
            .filter(|a| !a.is_guest())
            .collect();
        v.sort_by_key(|a| a.0);
        v.dedup();
        Self(v)
    }

    pub fn has(&self, a: &Address) -> bool {
        !a.is_guest() && self.0.binary_search_by(|x| x.0.cmp(&a.0)).is_ok()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// Friends and buddies playing Gates on shard row `row_id`.
pub fn on_shard<'a>(view: &'a View, row_id: &str) -> Vec<&'a Friend> {
    view.everyone()
        .filter(|f| {
            f.on_gates()
                .is_some_and(|p| p.server.as_deref() == Some(row_id))
        })
        .collect()
}

/// Every shard row id a friend is playing Gates on, sorted, once each.
pub fn shard_ids(view: &View) -> Vec<String> {
    let mut ids: Vec<String> = view
        .everyone()
        .filter_map(|f| f.on_gates()?.server.clone())
        .collect();
    ids.sort();
    ids.dedup();
    ids
}

/// The `host:port` to dial to join `wallet`: their shard's row when the list
/// has it, else the platform's `join` link.
pub fn join_target(view: &View, wallet: &Address, rows: &[Listing]) -> Result<String, String> {
    let f = view
        .find(wallet)
        .ok_or_else(|| "that wallet is not on your friends list".to_string())?;
    let p = f
        .on_gates()
        .ok_or_else(|| format!("{} is not on a Gates server right now", f.label()))?;
    if let Some(row) = p
        .server
        .as_deref()
        .and_then(|id| rows.iter().find(|r| !r.direct && r.id == id))
    {
        return Ok(row.addr.clone());
    }
    match p.join.as_deref() {
        Some(link) => crate::deeplink::parse(link).map(|j| j.addr),
        None => Err(format!("{}'s server is not on your shard list", f.label())),
    }
}

/// `{origin}/api/friends/{wallet}`.
pub fn url(origin: &str, wallet: &str) -> String {
    format!(
        "{}/api/friends/{}",
        origin.trim_end_matches('/'),
        wallet.to_ascii_lowercase()
    )
}

pub fn prepare_url(origin: &str) -> String {
    format!("{}/api/friends/prepare", origin.trim_end_matches('/'))
}

pub fn act_url(origin: &str) -> String {
    format!("{}/api/friends/act", origin.trim_end_matches('/'))
}

/// Where the platform's own friends list lives.
pub fn profile_url(origin: &str) -> String {
    format!("{}{PROFILE_PATH}", origin.trim_end_matches('/'))
}

/// `{origin}/api/who?handle=<name>`, the name percent-encoded.
pub fn who_url(origin: &str, name: &str) -> String {
    let mut q = String::with_capacity(name.len() * 3);
    for b in name.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            q.push(b as char);
        } else {
            q.push_str(&format!("%{b:02X}"));
        }
    }
    format!("{}/api/who?handle={q}", origin.trim_end_matches('/'))
}

/// UTC `YYYY-MM-DD` of unix second `at`. Howard Hinnant's civil-from-days,
/// the SDK's `utc_day` with the clock taken out.
pub fn utc_day(at: u64) -> String {
    let z = (at / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

/// The exact text a wallet signs for one change: the platform's playauth
/// text with action `friends`, its detail the sorted compact JSON of the op.
///
/// Written out here rather than through `elo::play_message`, because the
/// launcher's module is compiled out of the browser build; a test holds the
/// two to the same bytes.
pub fn sign_text(me: &str, op: Op, other: &str, at: u64) -> Result<String, String> {
    if !wallet_ok(me) {
        return Err("sign in with a wallet first".into());
    }
    if !wallet_ok(other) {
        return Err(format!("{other:?} is not a wallet"));
    }
    let me = me.to_ascii_lowercase();
    let other = other.to_ascii_lowercase();
    Ok(format!(
        "elo play\naction: friends\nwallet: {me}\nday: {}\ndetail: {}",
        utc_day(at),
        detail(op, &other, at)
    ))
}

/// `{"args":{"<key>":"<other>"},"at":<at>,"op":"<op>"}`, keys sorted.
pub fn detail(op: Op, other: &str, at: u64) -> String {
    format!(
        "{{\"args\":{{\"{}\":\"{}\"}},\"at\":{at},\"op\":\"{}\"}}",
        op.key(),
        other.to_ascii_lowercase(),
        op.name()
    )
}

/// `POST /api/friends/prepare`'s body.
pub fn prepare_body(me: &str, op: Op, other: &str) -> String {
    serde_json::json!({
        "wallet": me.to_ascii_lowercase(),
        "op": op.name(),
        "args": { op.key(): other.to_ascii_lowercase() },
    })
    .to_string()
}

/// `POST /api/friends/act`'s body.
pub fn act_body(me: &str, op: Op, other: &str, at: u64, signature: &str) -> String {
    serde_json::json!({
        "wallet": me.to_ascii_lowercase(),
        "op": op.name(),
        "args": { op.key(): other.to_ascii_lowercase() },
        "at": at,
        "signature": signature,
    })
    .to_string()
}

/// What prepare offered to have signed.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Offered {
    #[serde(default, deserialize_with = "or_default")]
    pub sign_this: String,
    #[serde(default, deserialize_with = "or_default")]
    pub at: u64,
    #[serde(default, deserialize_with = "or_default")]
    pub op: String,
    #[serde(default, deserialize_with = "or_default")]
    pub args: serde_json::Map<String, serde_json::Value>,
}

pub fn parse_offered(bytes: &[u8]) -> Result<Offered, String> {
    if bytes.len() > MAX_PREPARE_BYTES {
        return Err("the platform's answer was too long".into());
    }
    serde_json::from_slice(bytes).map_err(|_| "the platform's answer did not read".to_string())
}

/// The text to sign, if prepare offered exactly the change asked for.
///
/// The text is rebuilt here and compared, never taken on trust: a client that
/// signs what a server hands it has given that server its signature.
pub fn check_offered(
    me: &str,
    op: Op,
    other: &str,
    now: u64,
    offered: &Offered,
) -> Result<String, String> {
    let nothing = "nothing was signed";
    if offered.op != op.name() {
        return Err(format!(
            "the platform offered a different change ({:?}); {nothing}",
            offered.op
        ));
    }
    let arg = offered.args.get(op.key()).and_then(|v| v.as_str());
    if offered.args.len() != 1 || !arg.is_some_and(|a| a.eq_ignore_ascii_case(other)) {
        return Err(format!(
            "the platform offered a change to another wallet; {nothing}"
        ));
    }
    if offered.at.abs_diff(now) > AT_SKEW_S {
        return Err(format!(
            "this computer's clock and the platform's are more than ten minutes apart; {nothing}"
        ));
    }
    let text = sign_text(me, op, other, offered.at)?;
    if offered.sign_this != text {
        return Err(format!(
            "the platform asked to sign other words than this change; {nothing}"
        ));
    }
    Ok(text)
}

/// The view an act answered with, or why it did not land.
pub fn parse_act(status: u16, bytes: &[u8]) -> Result<View, String> {
    if status != 200 {
        return Err(error_of(status, bytes));
    }
    if bytes.len() > MAX_VIEW_BYTES {
        return Err("the platform's answer was too long".into());
    }
    let v: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|_| "the platform's answer did not read")?;
    match v.get("view") {
        Some(view) if v.get("ok").and_then(|o| o.as_bool()) == Some(true) => view_of(view.clone()),
        _ => Err(error_of(status, bytes)),
    }
}

/// A refusal in the platform's own words (`{"error": "..."}`), else one
/// sentence per status.
pub fn error_of(status: u16, bytes: &[u8]) -> String {
    let said = serde_json::from_slice::<serde_json::Value>(bytes)
        .ok()
        .and_then(|v| v.get("error")?.as_str().map(str::to_string))
        .map(|s| s.trim().chars().take(160).collect::<String>())
        .filter(|s| !s.is_empty() && !s.chars().any(char::is_control));
    if let Some(s) = said {
        return s;
    }
    match status {
        401 => "the signature did not check out; try again".into(),
        429 => "that is enough changes for today; try tomorrow".into(),
        0 => "could not reach Elo Pros".into(),
        s => format!("Elo Pros answered {s}"),
    }
}

/// What the add box holds: a wallet to use as it is, or a name to look up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lookup {
    Wallet(String),
    Name(String),
}

pub fn lookup(typed: &str) -> Result<Lookup, String> {
    let t = typed.trim();
    if t.is_empty() {
        return Err("Type a wallet or a name.".into());
    }
    if wallet_ok(t) {
        return Ok(Lookup::Wallet(t.to_ascii_lowercase()));
    }
    Ok(Lookup::Name(t.to_string()))
}

/// The wallet `/api/who` names for a typed name, the profile page's rules
/// (`store.js` `frResolve`): exactly one wallet, or a sentence saying why not.
pub fn resolve_who(typed: &str, status: u16, body: &[u8]) -> Result<String, String> {
    let t = typed.trim();
    if status == 404 || status == 422 {
        return Err(format!(
            "Couldn't find \"{t}\". Paste their wallet instead."
        ));
    }
    let d: Option<serde_json::Value> = (status == 200 && body.len() <= MAX_WHO_BYTES)
        .then(|| serde_json::from_slice(body).ok())
        .flatten();
    let Some(d) = d else {
        return Err("Couldn't look that up. Try again.".into());
    };
    let Some(found) = d.get("found").filter(|f| f.is_object()) else {
        return Err(format!(
            "Couldn't find \"{t}\". Paste their wallet instead."
        ));
    };
    let ambiguous = d
        .get("ambiguous")
        .is_some_and(|a| !a.is_null() && a != false);
    let one = if ambiguous { one_wallet(&d) } else { None };
    if ambiguous && one.is_none() {
        return Err(format!(
            "More than one goes by \"{t}\". Paste their wallet instead."
        ));
    }
    let w = one.or_else(|| {
        found
            .get("wallet")
            .and_then(|w| w.as_str())
            .map(str::to_string)
    });
    match w.filter(|w| wallet_ok(w)) {
        Some(w) => Ok(w.to_ascii_lowercase()),
        None => Err(format!("\"{t}\" has no wallet to add.")),
    }
}

/// Several vows under one name are one person when every candidate carries
/// the same wallet as the subject (`store.js` `frOneWallet`).
fn one_wallet(d: &serde_json::Value) -> Option<String> {
    let s = d
        .get("subject")?
        .get("wallet")?
        .as_str()?
        .to_ascii_lowercase();
    let c = d.get("ambiguous")?.get("candidates")?.as_array()?;
    (wallet_ok(&s)
        && !c.is_empty()
        && c.iter().all(|x| {
            x.get("wallet")
                .and_then(|w| w.as_str())
                .is_some_and(|w| w.eq_ignore_ascii_case(&s))
        }))
    .then_some(s)
}

/// The add box's cap: a wallet, `0x` and forty digits. Past the search box's
/// [`crate::ui::MAX_QUERY_CHARS`] on purpose, which would cut one off.
pub const MAX_ADD_CHARS: usize = 42;

/// The add box: what the keyboard has typed, capped.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AddBox {
    pub text: String,
}

impl AddBox {
    pub fn push(&mut self, c: char) -> bool {
        if c.is_control() || self.text.chars().count() >= MAX_ADD_CHARS {
            return false;
        }
        self.text.push(c);
        true
    }

    pub fn backspace(&mut self) -> bool {
        self.text.pop().is_some()
    }

    pub fn clear(&mut self) {
        self.text.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }
}
