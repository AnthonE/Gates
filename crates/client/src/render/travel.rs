//! **Following the shard to another of this game's servers** (wire v152):
//! `/arena`, `/leave`, and any door that sends a player on.
//!
//! The shard's last word is a travel message — an address and a handoff it
//! signed (`Session::take_travel`). This screen takes it: the world is torn
//! down as on a disconnect, the next server is joined on the handoff with
//! nothing asked of the player, and the welcome lands in `Loading` like any
//! join. Desktop and browser alike: only how the connect is spawned differs.
//!
//! **If the next server says no or does not answer**, the desktop joins the
//! server it came from again the usual way (`Screen::Connecting`), and the
//! browser, which cannot sign on its own, shows the disconnect screen with
//! the reason. A second handoff that also works would let a player stand in
//! two places, so there is no return ticket (`docs/client/HANDOFF.md`).

use bevy::prelude::*;

use super::disconnected::Reason;
use super::screen::{Connecting, Menu, Screen};
use super::{ui, Net, WorldId};

/// How long the hop may take before it is called.
pub const TRAVEL_TIMEOUT_S: f32 = 20.0;

/// The hop in flight. Non-send because it holds the connect's receiver,
/// beside the `Session` it will become.
#[derive(Default)]
pub struct Travelling {
    /// The server joined on the handoff, `host:port`.
    pub to: String,
    /// The server that sent us: where the desktop falls back to.
    pub from: String,
    pub rx: Option<std::sync::mpsc::Receiver<Result<crate::Session, crate::JoinError>>>,
    pub waited_s: f32,
}

impl Travelling {
    /// A hop is under way: the disconnect watch stands down for it.
    pub fn underway(&self) -> bool {
        self.rx.is_some()
    }
}

/// Everything this screen owns.
#[derive(Component)]
pub struct Root;

/// Take the shard's travel message, start the join, and leave the world.
/// After `input::place_eye` (which pumps the session) and before
/// `disconnected::watch`, so the close that follows the message is never read
/// as a loss.
#[allow(clippy::too_many_arguments)]
pub fn watch(
    net: Option<NonSendMut<Net>>,
    mut travelling: NonSendMut<Travelling>,
    connecting: NonSend<Connecting>,
    #[cfg(not(target_arch = "wasm32"))] rt: Option<NonSend<super::menu::Rt>>,
    #[cfg(not(target_arch = "wasm32"))] pin: Option<Res<super::screen::Pin>>,
    mut reason: ResMut<Reason>,
    mut next: ResMut<NextState<Screen>>,
) {
    let Some(mut net) = net else {
        return;
    };
    let Some(t) = net.session.take_travel() else {
        return;
    };
    let to = t.addr().to_string();
    let (tx, rx) = std::sync::mpsc::channel();
    #[cfg(not(target_arch = "wasm32"))]
    {
        let Some(rt) = rt else {
            reason.line = format!("{to}: this build cannot follow the server there");
            next.set(Screen::Disconnected);
            return;
        };
        let hash = pin.filter(|p| p.addr == to).map(|p| p.hash.clone());
        let addr = to.clone();
        rt.0.spawn(async move {
            let got = match crate::client_endpoint(&addr, hash.as_deref()) {
                Ok(endpoint) => crate::Session::connect_handoff(&endpoint, &addr, &t).await,
                Err(e) => Err(crate::JoinError::Failed(e)),
            };
            let _ = tx.send(got);
        });
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = &mut reason;
        let addr = to.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let got = crate::Session::connect_handoff(&addr, None, &t).await;
            let _ = tx.send(got);
        });
    }
    info!("gates: travelling to {to}");
    travelling.from = connecting.addr.clone();
    travelling.to = to;
    travelling.rx = Some(rx);
    travelling.waited_s = 0.0;
    next.set(Screen::Travelling);
}

/// The screen: one line, which way, and how to get out.
pub fn setup(mut commands: Commands, travelling: NonSend<Travelling>) {
    let line = if travelling.to.is_empty() {
        "on the move...".to_string()
    } else {
        format!(
            "on the way to {}...",
            crate::ui::streamer::server(&travelling.to)
        )
    };
    commands.spawn((Camera2d, Root));
    commands.spawn((
        Root,
        ui::screen(ui::BG),
        children![
            ui::wordmark(),
            (
                ui::strong(line, 20.0, ui::TEXT),
                Node {
                    margin: UiRect::top(Val::Px(18.0)),
                    ..default()
                },
            ),
            ui::label(
                "no sign-in needed: the server you left vouched for you",
                12.0,
                ui::FAINT
            ),
        ],
    ));
}

/// Collect the join: the next server's welcome goes to `Loading`; a refusal
/// or a timeout goes back the usual way (desktop) or to the disconnect
/// screen (browser). Exclusive, because the browser's `Session` is not
/// `Send` and so cannot ride a queued command into the world.
pub fn poll(world: &mut World) {
    let dt = world.resource::<Time>().delta_secs();
    let mut travelling = world.non_send_resource_mut::<Travelling>();
    if travelling.rx.is_none() {
        return;
    }
    travelling.waited_s += dt;
    let got = match travelling.rx.as_ref().map(|rx| rx.try_recv()) {
        Some(Ok(got)) => got,
        Some(Err(std::sync::mpsc::TryRecvError::Empty))
            if travelling.waited_s < TRAVEL_TIMEOUT_S =>
        {
            return
        }
        Some(Err(std::sync::mpsc::TryRecvError::Empty)) => Err(crate::JoinError::Failed(format!(
            "no answer after {TRAVEL_TIMEOUT_S:.0}s"
        ))),
        _ => Err(crate::JoinError::Failed(
            "the join did not finish".to_string(),
        )),
    };
    travelling.rx = None;
    let (to, from) = (travelling.to.clone(), travelling.from.clone());
    match got {
        Ok(session) => {
            info!(
                "gates: arrived at {to} - player {} seed {}",
                session.welcome.player_id, session.welcome.seed
            );
            world.non_send_resource_mut::<Connecting>().addr = to;
            world.insert_resource(WorldId {
                blight_salt: session.welcome.blight_salt,
                ..WorldId::with_haven(session.welcome.seed, *session.core.haven())
            });
            world.insert_non_send_resource(Net {
                session,
                sel: 0,
                light: false,
                farm_sent: None,
            });
            world
                .resource_mut::<NextState<Screen>>()
                .set(Screen::Loading);
        }
        Err(why) => {
            let to = crate::ui::streamer::server(&to);
            warn!("gates: could not reach {to}: {why}");
            if cfg!(not(target_arch = "wasm32")) && !from.is_empty() {
                // Back the way we came, signed in as ever: the server we left
                // has put our body to sleep in town, and a plain join wakes it.
                if let Some(mut menu) = world.get_resource_mut::<Menu>() {
                    menu.status = format!("{to}: {why}");
                }
                let mut connecting = world.non_send_resource_mut::<Connecting>();
                connecting.addr = from;
                connecting.watch = false;
                connecting.rx = None;
                world
                    .resource_mut::<NextState<Screen>>()
                    .set(Screen::Connecting);
            } else {
                world.resource_mut::<Reason>().line = format!("{to}: {why}");
                world
                    .resource_mut::<NextState<Screen>>()
                    .set(Screen::Disconnected);
            }
        }
    }
}
