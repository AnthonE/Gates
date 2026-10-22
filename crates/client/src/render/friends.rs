//! Friends from Elo Pros, fetched and changed: the menu's FRIENDS pane, the
//! server browser's Friends rail, the pause menu's list, and the mark on a
//! friend's nametag and chat line all read [`Friends`].
//!
//! **Read when it is looked at**: on entering the menu, on picking FRIENDS or
//! its REFRESH, once on the first entry to a world (when the player's own tag
//! lands), and on opening the Esc menu — and every [`REFRESH_S`] besides, the
//! pace presence itself moves at, so Steam's corner notices (an ask, an
//! invite, a friend starting to play) can fire. A change takes the list its
//! act answered with rather than asking again.
//!
//! A change is prepare → sign → act, and signing goes through the elo
//! launcher's consent prompt, so it is desktop only: a page changes its list
//! on the Elo Pros site, where the wallet lives.
//!
//! All arithmetic is `crate::friends`; this file owns the threads, the
//! channel and the one resource.

use bevy::prelude::*;
use protocol::Address;

use super::faces::Pics;
use super::screen::{Menu, Who};
use super::Net;
use crate::friends::{self, FriendSet, View};

/// What every friends surface says when nobody is signed in.
pub const SIGNED_OUT: &str = "Sign in to see your friends.";
/// What the read says while it is out.
const READING: &str = "reading your friends list...";
/// What a read that failed says, before why.
const READ_FAILED: &str = "couldn't read your friends list";
/// The slow re-read, seconds: presence lags about a minute and a half anyway.
pub const REFRESH_S: f32 = 90.0;
/// Notices held for a surface to show; past it the oldest go.
const MAX_NOTICES: usize = 8;
/// How long one platform round trip may take, natively.
#[cfg(not(target_arch = "wasm32"))]
const HTTP_TIMEOUT_S: u64 = 15;

/// What came back from a thread (natively) or a promise (in a page).
enum Landed {
    Read {
        me: Address,
        got: Result<View, String>,
    },
    // Only an act sends this, and acts are desktop only.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    Acted {
        me: Address,
        op: friends::Op,
        got: Result<(View, String, String), String>,
    },
}

type Tx = tokio::sync::mpsc::UnboundedSender<Landed>;
type Rx = tokio::sync::mpsc::UnboundedReceiver<Landed>;

/// The signed-in player's friends list, as last read.
#[derive(Resource)]
pub struct Friends {
    /// `scheme://host` of the platform (`ui::names::origin_of`).
    pub origin: String,
    /// Whose list this is. `None` signed out.
    pub me: Option<Address>,
    pub view: Option<View>,
    /// Friends and buddies by wallet, for nametags and chat.
    pub set: FriendSet,
    /// One line for whichever surface is up: a read in flight, a refusal, or
    /// what the last change did.
    pub status: String,
    /// Bumped on every change a surface would draw; they redraw on a new one.
    pub gen: u64,
    /// A change is out; a second waits for it.
    pub acting: bool,
    /// The read out right now, and for whom.
    reading: Option<Address>,
    /// Read once the player's own tag lands, on the first world entry.
    want_world: bool,
    /// Wallets this player blocked this session, lowercase hex. The platform
    /// never serves a block list, so these are the only ones the client knows.
    blocked: Vec<String>,
    /// What changed between reads, for the HUD to say (`friends::notices`).
    pub notices: Vec<String>,
    /// A friend's server picked from inside a world: the menu dials it once
    /// the world is left.
    pub pending_join: Option<String>,
    tx: Tx,
    rx: Rx,
}

impl Default for Friends {
    fn default() -> Self {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        Self {
            origin: crate::ui::names::PLATFORM_ORIGIN.to_string(),
            me: None,
            view: None,
            set: FriendSet::default(),
            status: SIGNED_OUT.to_string(),
            gen: 0,
            acting: false,
            reading: None,
            want_world: false,
            blocked: Vec::new(),
            notices: Vec::new(),
            pending_join: None,
            tx,
            rx,
        }
    }
}

pub fn hex(a: Address) -> String {
    String::from_utf8_lossy(&a.to_hex()).into_owned()
}

/// The address `Who` claims, if it is one.
pub fn who_address(who: &Who) -> Option<Address> {
    who.0
        .address()
        .and_then(|a| Address::from_hex(a.as_bytes()))
        .filter(|a| !a.is_guest())
}

/// Who is playing, in a world: the shard's own tag for this player (proven
/// at the handshake), else what the launcher claimed.
pub fn me_in_world(net: Option<&Net>, who: &Who) -> Option<Address> {
    net.and_then(|n| {
        let core = &n.session.core;
        core.tag(core.player_id).map(|t| t.address)
    })
    .filter(|a| !a.is_guest())
    .or_else(|| who_address(who))
}

impl Friends {
    /// Read `me`'s list from `origin`. One read at a time per wallet; a new
    /// wallet drops the old list at once, so nothing shows another's friends.
    pub fn ask(&mut self, origin: &str, me: Option<Address>) {
        if me != self.me {
            self.view = None;
            self.set = FriendSet::default();
        }
        self.origin = origin.trim_end_matches('/').to_string();
        self.me = me;
        self.gen += 1;
        let Some(me) = me else {
            self.status = SIGNED_OUT.into();
            return;
        };
        if self.reading == Some(me) {
            return;
        }
        self.reading = Some(me);
        if self.view.is_none() || self.status.starts_with(READ_FAILED) {
            self.status = READING.into();
        }
        read(friends::url(&self.origin, &hex(me)), me, self.tx.clone());
    }

    /// The pending asks waiting on this player, for the nav's count.
    pub fn asks_in(&self) -> usize {
        self.view.as_ref().map_or(0, |v| v.asks_in.len())
    }

    /// This player blocked `who` this session.
    pub fn blocked(&self, who: &Address) -> bool {
        self.blocked.contains(&hex(*who))
    }

    pub fn say(&mut self, line: impl Into<String>) {
        self.status = line.into();
        self.gen += 1;
    }

    /// Change the list: `target` is a wallet, or a name the add box took.
    /// Off the frame — it waits on a person at the launcher's prompt.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn act(&mut self, op: friends::Op, target: &str) {
        let Some(me) = self.me else {
            self.say(SIGNED_OUT);
            return;
        };
        if self.acting {
            self.say("one change at a time - wait for the last one");
            return;
        }
        self.acting = true;
        self.say(match op {
            friends::Op::Ask => "looking them up - then approve the ask in your launcher",
            friends::Op::Invite => "approve the invite in your launcher",
            _ => "approve it in your launcher",
        });
        let (origin, tx, target) = (self.origin.clone(), self.tx.clone(), target.to_string());
        std::thread::spawn(move || {
            let got = act_blocking(&origin, me, op, &target);
            let _ = tx.send(Landed::Acted { me, op, got });
        });
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn read(url: String, me: Address, tx: Tx) {
    std::thread::spawn(move || {
        let (status, body) = http(&url, None, friends::MAX_VIEW_BYTES);
        let got = if status == 200 {
            friends::parse(&body)
        } else {
            Err(friends::error_of(status, &body))
        };
        let _ = tx.send(Landed::Read { me, got });
    });
}

#[cfg(target_arch = "wasm32")]
fn read(url: String, me: Address, tx: Tx) {
    wasm_bindgen_futures::spawn_local(async move {
        let got = match crate::net::web::fetch_bytes(&url, friends::MAX_VIEW_BYTES).await {
            Some(b) => friends::parse(&b),
            None => Err("Elo Pros did not answer".into()),
        };
        let _ = tx.send(Landed::Read { me, got });
    });
}

/// One capped round trip, natively, with a refusal's status and body kept
/// (the platform's `{"error": ...}` is the sentence to show). Status 0 is
/// "never reached".
#[cfg(not(target_arch = "wasm32"))]
fn http(url: &str, post: Option<&str>, cap: usize) -> (u16, Vec<u8>) {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .tls_config(crate::tls::ureq())
        .timeout_global(Some(std::time::Duration::from_secs(HTTP_TIMEOUT_S)))
        .http_status_as_error(false)
        .build()
        .into();
    let res = match post {
        Some(body) => agent
            .post(url)
            .header("content-type", "application/json")
            .send(body),
        None => agent.get(url).call(),
    };
    let Ok(mut res) = res else {
        return (0, Vec::new());
    };
    let status = res.status().as_u16();
    let body = res
        .body_mut()
        .with_config()
        .limit((cap + 1) as u64)
        .read_to_vec()
        .unwrap_or_default();
    (status, body)
}

/// Resolve, prepare, check, sign, act. Blocking; on a thread.
#[cfg(not(target_arch = "wasm32"))]
fn act_blocking(
    origin: &str,
    me: Address,
    op: friends::Op,
    target: &str,
) -> Result<(View, String, String), String> {
    use crate::elo::SignError;
    use friends::Lookup;

    let me_hex = hex(me);
    let other = match friends::lookup(target)? {
        Lookup::Wallet(w) => w,
        Lookup::Name(n) => {
            let (status, body) = http(&friends::who_url(origin, &n), None, friends::MAX_WHO_BYTES);
            friends::resolve_who(&n, status, &body)?
        }
    };
    if other == me_hex {
        return Err("That's you.".into());
    }
    let (status, body) = http(
        &friends::prepare_url(origin),
        Some(&friends::prepare_body(&me_hex, op, &other)),
        friends::MAX_PREPARE_BYTES,
    );
    if status != 200 {
        return Err(friends::error_of(status, &body));
    }
    let offered = friends::parse_offered(&body)?;
    let text = friends::check_offered(&me_hex, op, &other, crate::shot::now_secs(), &offered)?;

    let mut elo = crate::elo::Elo::discover(None, env!("CARGO_PKG_VERSION"));
    let why = match op {
        friends::Op::Invite => format!(
            "Gates: invite {} to join your game",
            friends::short_wallet_real(&other)
        ),
        _ => format!(
            "Gates: {} {} on your Elo Pros friends list",
            op.name(),
            friends::short_wallet_real(&other)
        ),
    };
    let sig = match elo.sign(&text, &why) {
        Ok(sig) => sig,
        // The player's signer is their browser, and the change finishes
        // there: the profile page has the same list and the same buttons.
        Err(SignError::Handoff(_)) => {
            let url = friends::profile_url(origin);
            return Err(if elo.open(&url) {
                "Your launcher signs in your browser: finish this on your Elo Pros profile, \
                 which it just opened."
                    .into()
            } else {
                format!("Your launcher signs in your browser: finish this at {url}")
            });
        }
        Err(SignError::RefusedByPlayer(_)) => {
            return Err("You said no in the launcher. Nothing changed.".into())
        }
        Err(SignError::Refused(why)) => return Err(format!("The launcher would not sign: {why}")),
        Err(SignError::NoLauncher(_)) => {
            return Err(format!(
                "Changing your friends list needs the Elo launcher running - or do it at {}",
                friends::profile_url(origin)
            ))
        }
    };
    if let Some(a) = sig.address.as_deref() {
        if !a.eq_ignore_ascii_case(&me_hex) {
            return Err(format!(
                "your launcher signs as {}, and this list is {}",
                friends::short_wallet(a),
                friends::short_wallet(&me_hex)
            ));
        }
    }
    let (status, body) = http(
        &friends::act_url(origin),
        Some(&friends::act_body(
            &me_hex,
            op,
            &other,
            offered.at,
            &sig.signature,
        )),
        friends::MAX_VIEW_BYTES,
    );
    let view = friends::parse_act(status, &body)?;
    let done = op.done(&view, &other).to_string();
    Ok((view, done, other))
}

/// Take what landed. A read for someone who is no longer signed in is
/// dropped; the menu redraws on anything that moved.
pub fn pump(
    mut friends: ResMut<Friends>,
    menu: Option<ResMut<Menu>>,
    screen: Res<State<super::Screen>>,
) {
    if friends.rx.is_empty() {
        return;
    }
    let mut moved = false;
    while let Ok(landed) = friends.rx.try_recv() {
        moved = true;
        match landed {
            Landed::Read { me, got } => {
                if friends.reading == Some(me) {
                    friends.reading = None;
                }
                if friends.me != Some(me) {
                    continue;
                }
                match got {
                    Ok(view) => {
                        if let Some(old) = &friends.view {
                            let new = friends::notices(old, &view);
                            friends.notices.extend(new);
                            let over = friends.notices.len().saturating_sub(MAX_NOTICES);
                            friends.notices.drain(..over);
                        }
                        friends.set = FriendSet::of(&view);
                        friends.view = Some(view);
                        if friends.status == READING {
                            friends.status.clear();
                        }
                    }
                    Err(why) => {
                        friends.status = format!("{READ_FAILED}: {why}");
                    }
                }
            }
            Landed::Acted { me, op, got } => {
                friends.acting = false;
                if friends.me != Some(me) {
                    continue;
                }
                match got {
                    Ok((view, done, other)) => {
                        match op {
                            friends::Op::Block if !friends.blocked.contains(&other) => {
                                friends.blocked.push(other)
                            }
                            friends::Op::Unblock => friends.blocked.retain(|w| *w != other),
                            _ => {}
                        }
                        friends.set = FriendSet::of(&view);
                        friends.view = Some(view);
                        friends.status = done;
                    }
                    Err(why) => friends.status = why,
                }
            }
        }
        friends.gen += 1;
    }
    // Only the menu redraws on a flag; the pause block reads `gen`.
    if moved && *screen.get() == super::Screen::Menu {
        if let Some(mut menu) = menu {
            menu.dirty = true;
        }
    }
}

/// Entering the menu: the signed-in player's list, and a friend's server
/// picked from the Esc menu, dialled now that the world is left.
pub fn on_menu(
    mut friends: ResMut<Friends>,
    who: Res<Who>,
    menu: Res<Menu>,
    mut connecting: NonSendMut<super::screen::Connecting>,
    mut next: ResMut<NextState<super::Screen>>,
) {
    let origin = crate::ui::names::origin_of(menu.servers_url.as_deref());
    friends.ask(&origin, who_address(&who));
    if let Some(addr) = friends.pending_join.take() {
        match crate::shardlist::check_addr(&addr) {
            Ok(()) => {
                connecting.addr = addr;
                connecting.rx = None;
                next.set(super::Screen::Connecting);
            }
            Err(why) => friends.say(format!("can't join that server: {why}")),
        }
    }
}

/// The slow re-read, while signed in, so a notice can fire without the
/// player opening anything.
pub fn refresh(mut friends: ResMut<Friends>, time: Res<Time>, mut since: Local<f32>) {
    if friends.me.is_none() || friends.acting {
        *since = 0.0;
        return;
    }
    *since += time.delta_secs();
    if *since < REFRESH_S {
        return;
    }
    *since = 0.0;
    let (origin, me) = (friends.origin.clone(), friends.me);
    friends.ask(&origin, me);
}

/// Say what changed: in the world on the HUD's toast line, at the menu on
/// the friends status line. The Esc menu holds them; it lists invites itself.
pub fn notify(
    mut friends: ResMut<Friends>,
    toast: Option<ResMut<super::hud::Toast>>,
    screen: Res<State<super::Screen>>,
    menu: Option<ResMut<Menu>>,
) {
    if friends.notices.is_empty() {
        return;
    }
    match screen.get() {
        super::Screen::InWorld => {
            let Some(mut toast) = toast else {
                return;
            };
            for line in friends.notices.drain(..) {
                toast.say(line);
            }
        }
        super::Screen::Menu => {
            if let Some(last) = friends.notices.pop() {
                friends.notices.clear();
                friends.say(last);
                if let Some(mut menu) = menu {
                    menu.dirty = true;
                }
            }
        }
        _ => {}
    }
}

/// Entering a world: read once the player's own tag lands.
pub fn arm_world(mut friends: ResMut<Friends>) {
    friends.want_world = true;
}

pub fn on_world(
    mut friends: ResMut<Friends>,
    net: Option<NonSend<Net>>,
    who: Res<Who>,
    menu: Res<Menu>,
) {
    if !friends.want_world {
        return;
    }
    let Some(me) = me_in_world(net.as_deref(), &who) else {
        return;
    };
    friends.want_world = false;
    let origin = crate::ui::names::origin_of(menu.servers_url.as_deref());
    friends.ask(&origin, Some(me));
}

/// Opening the Esc menu.
pub fn on_paused(
    mut friends: ResMut<Friends>,
    net: Option<NonSend<Net>>,
    who: Res<Who>,
    menu: Res<Menu>,
) {
    let me = me_in_world(net.as_deref(), &who);
    let origin = crate::ui::names::origin_of(menu.servers_url.as_deref());
    friends.ask(&origin, me);
}

/// A friend's face still waiting on its picture: the initial on a dark tile
/// until `Pics` has it.
#[derive(Component)]
pub struct FriendPic {
    pub address: Address,
    pub pic: u32,
}

/// The tile a face is drawn on, `px` square, with the initial over it.
pub fn face(parent: &mut ChildSpawnerCommands, f: &friends::Friend, px: f32) {
    let mut tile = parent.spawn((
        Node {
            width: Val::Px(px),
            height: Val::Px(px),
            flex_shrink: 0.0,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        },
        BackgroundColor(super::ui::ROW_HOVER),
    ));
    if let (Some(address), Some(pic)) = (f.address(), f.pic_rev()) {
        tile.insert(FriendPic { address, pic });
    }
    let initial: String = f
        .label()
        .chars()
        .find(|c| c.is_alphanumeric())
        .map(|c| c.to_uppercase().collect())
        .unwrap_or_else(|| "?".into());
    tile.with_children(|t| {
        t.spawn(super::ui::strong(initial, px * 0.5, super::ui::FRIEND));
    });
}

/// Swap a face's initial for the picture once it lands.
pub fn fill_pics(
    mut commands: Commands,
    pics: Option<ResMut<Pics>>,
    waiting: Query<(Entity, &FriendPic)>,
) {
    let Some(mut pics) = pics else {
        return;
    };
    for (e, p) in &waiting {
        if let Some(h) = pics.want(p.address, p.pic) {
            commands
                .entity(e)
                .despawn_related::<Children>()
                .remove::<FriendPic>()
                .insert((ImageNode::new(h), BackgroundColor(Color::NONE)));
        }
    }
}

/// Wire the reads in. Registered beside the faces and, like them, not on a
/// capture run: a gate whose frames depend on a network read is not a gate.
pub fn register(app: &mut App) {
    app.add_systems(Update, (refresh, pump, notify, fill_pics).chain())
        .add_systems(OnEnter(super::Screen::Loading), arm_world)
        .add_systems(Update, on_world.run_if(in_state(super::Screen::InWorld)))
        .add_systems(OnEnter(super::Screen::Paused), on_paused);
}
