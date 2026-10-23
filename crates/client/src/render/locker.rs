//! The front end's INVENTORY and ITEM STORE, drawn the reference's way: the
//! store as a grid of cards over the scene, each a picture of the item in the
//! skin's colours with its name, what it fits and a price; the inventory as a
//! panel of counts beside a grid of the skins the wallet owns.
//!
//! Both read the platform's two documents the browser page reads
//! (`crate::ui::locker`), so the two front ends list the same skins. Buying
//! is still the launcher's: a card or OPEN STORE hands off to it, because a
//! game must never draw its own checkout (`crate::ui::hub`).

use bevy::prelude::*;

use super::icons::Icons;
use super::screen::{Menu, Who};
use super::ui;
use crate::ui::locker::{self, Shelf, StoreSkin};

type Rx<T> = tokio::sync::mpsc::UnboundedReceiver<Result<T, String>>;
/// A body → what it says (`crate::ui::locker::parse_*`).
type Parse<T> = fn(&[u8]) -> Result<T, String>;

/// One platform read.
#[derive(Default)]
pub enum Read<T> {
    /// Nothing asked: no wallet to read, or not yet.
    #[default]
    Idle,
    Waiting,
    Got(T),
    Failed(String),
}

impl<T> Read<T> {
    pub fn got(&self) -> Option<&T> {
        match self {
            Read::Got(t) => Some(t),
            _ => None,
        }
    }
}

/// The store and this wallet's skins.
#[derive(Resource, Default)]
pub struct Locker {
    pub store: Read<Vec<StoreSkin>>,
    pub owned: Read<Vec<(u32, u32)>>,
    /// The wallet `owned` was read for.
    owned_for: Option<protocol::Address>,
    store_rx: Option<Rx<Vec<StoreSkin>>>,
    owned_rx: Option<Rx<Vec<(u32, u32)>>>,
    /// The store's open tab.
    pub shelf: Shelf,
    /// REFRESH INVENTORY: read both again on the next frame.
    pub refresh: bool,
    /// How the last OPEN STORE went (`hub::Opened::line`), under the cards.
    pub note: Option<String>,
}

impl Locker {
    fn owned_rows(&self) -> &[(u32, u32)] {
        self.owned.got().map_or(&[], Vec::as_slice)
    }
}

/// A store tab.
#[derive(Component)]
pub struct ShelfButton(pub Shelf);

/// A store card, or OPEN STORE: both hand off to the launcher's store.
#[derive(Component)]
pub struct OpenStore;

/// REFRESH INVENTORY.
#[derive(Component)]
pub struct RefreshInventory;

/// Ask for what is missing and collect what has landed. Every answer raises
/// `menu.dirty`, which is the menu's one redraw.
pub fn poll(who: Res<Who>, mut menu: ResMut<Menu>, mut locker: ResMut<Locker>) {
    let origin = crate::ui::names::origin_of(menu.servers_url.as_deref());
    let refresh = std::mem::take(&mut locker.refresh);

    if refresh || matches!(locker.store, Read::Idle) {
        locker.store = Read::Waiting;
        locker.store_rx = Some(read(
            locker::store_url(&origin),
            locker::MAX_STORE_BYTES,
            locker::parse_store,
        ));
        menu.dirty = true;
    }

    let wallet = who
        .0
        .address()
        .and_then(|a| protocol::Address::from_hex(a.as_bytes()))
        .filter(|a| !a.is_guest());
    if refresh || wallet != locker.owned_for {
        locker.owned_for = wallet;
        locker.owned_rx = None;
        locker.owned = match wallet {
            Some(a) => {
                let hex = a.to_hex();
                let url = locker::owned_url(&origin, core::str::from_utf8(&hex).unwrap_or(""));
                locker.owned_rx = Some(read(url, locker::MAX_OWNED_BYTES, locker::parse_owned));
                Read::Waiting
            }
            None => Read::Idle,
        };
        menu.dirty = true;
    }

    if let Some(got) = locker.store_rx.as_mut().and_then(|rx| rx.try_recv().ok()) {
        locker.store_rx = None;
        locker.store = settle(got);
        menu.dirty = true;
    }
    if let Some(got) = locker.owned_rx.as_mut().and_then(|rx| rx.try_recv().ok()) {
        locker.owned_rx = None;
        locker.owned = settle(got);
        menu.dirty = true;
    }
}

fn settle<T>(got: Result<T, String>) -> Read<T> {
    match got {
        Ok(t) => Read::Got(t),
        Err(why) => Read::Failed(why),
    }
}

/// One capped GET on a thread; the frame only ever `try_recv`s.
#[cfg(not(target_arch = "wasm32"))]
fn read<T: Send + 'static>(url: String, cap: usize, parse: Parse<T>) -> Rx<T> {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    std::thread::spawn(move || {
        let got = super::faces::get_bytes(&url, cap)
            .ok_or_else(|| "couldn't reach Elo Pros just now".to_string())
            .and_then(|b| parse(&b));
        let _ = tx.send(got);
    });
    rx
}

/// A store card's width, px: three across a 1280 window beside the nav, the
/// reference's own count.
const CARD_W: f32 = 214.0;
/// An inventory tile's side, px: the reference's six-across grid.
const TILE_W: f32 = 118.0;

/// ITEM STORE: the tab bar with OPEN STORE at its right, the cards, and the
/// line that says where buying happens. No control panel, as in the
/// reference: the grid is the page.
pub fn store(
    body: &mut ChildSpawnerCommands,
    locker: &Locker,
    icons: Option<&Icons>,
    note: &str,
    scroll: f32,
) {
    let owned = locker.owned_rows();
    body.spawn(ui::pane_with(true)).with_children(|pane| {
        // ---- the tabs, and the cart's place ------------------------------
        pane.spawn((
            Node {
                height: Val::Px(48.0),
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: Val::Px(4.0),
                padding: UiRect::axes(Val::Px(8.0), Val::Px(0.0)),
                border: UiRect::bottom(Val::Px(1.0)),
                flex_shrink: 0.0,
                ..default()
            },
            BorderColor::all(ui::RULE),
        ))
        .with_children(|bar| {
            for shelf in Shelf::ALL {
                let picked = shelf == locker.shelf;
                let n = locker.store.got().map(|s| shelf.rows(s, owned).len());
                let (idle, over) = if picked {
                    (ui::ACCENT, ui::ACCENT_HOVER)
                } else {
                    (Color::NONE, ui::ROW_HOVER)
                };
                bar.spawn((
                    Button,
                    Node {
                        height: Val::Px(34.0),
                        padding: UiRect::axes(Val::Px(16.0), Val::Px(0.0)),
                        align_items: AlignItems::Center,
                        ..default()
                    },
                    BackgroundColor(idle),
                    ui::Hover::new(idle, over),
                    ShelfButton(shelf),
                ))
                .with_children(|b| {
                    let text = match n {
                        Some(n) => format!("{}  {n}", shelf.label()),
                        None => shelf.label().to_string(),
                    };
                    b.spawn(ui::strong(
                        text,
                        13.0,
                        if picked { ui::TITLE } else { ui::DIM },
                    ));
                });
            }
            bar.spawn(ui::spacer());
            bar.spawn((
                Button,
                Node {
                    height: Val::Px(34.0),
                    padding: UiRect::axes(Val::Px(18.0), Val::Px(0.0)),
                    align_items: AlignItems::Center,
                    ..default()
                },
                BackgroundColor(ui::MARK),
                ui::Hover::new(ui::MARK, ui::GO_HOVER),
                OpenStore,
            ))
            .with_children(|b| {
                b.spawn(ui::strong("OPEN STORE", 13.0, ui::GO_INK));
            });
        });

        // ---- the cards ---------------------------------------------------
        pane.spawn(ui::scroll_area(scroll)).with_children(|area| {
            let rows = match &locker.store {
                Read::Got(s) => locker.shelf.rows(s, owned),
                Read::Failed(why) => {
                    area.spawn(empty(format!("Couldn't load the item store: {why}.")));
                    return;
                }
                Read::Idle | Read::Waiting => {
                    area.spawn(empty("Loading the item store...".to_string()));
                    return;
                }
            };
            let store = locker.store.got().map_or(&[][..], Vec::as_slice);
            if rows.is_empty() {
                area.spawn(empty(match locker.shelf {
                    Shelf::All => "Nothing is in the item store yet.".to_string(),
                    Shelf::OnSale => {
                        "Nothing is on sale right now. The looks in ALL SKINS go on sale \
                         as they're priced."
                            .to_string()
                    }
                    Shelf::Owned if locker.owned_for.is_none() => {
                        "Start the game from the Elo launcher, signed in, and the skins your \
                         wallet owns are marked here."
                            .to_string()
                    }
                    Shelf::Owned => "You don't own any Gates skins yet.".to_string(),
                }));
                return;
            }
            area.spawn(grid()).with_children(|g| {
                for i in rows {
                    card(
                        g,
                        &store[i],
                        locker::count_of(owned, store[i].catalog),
                        icons,
                    );
                }
            });
        });

        pane.spawn((
            ui::label(note.to_string(), 12.0, ui::FAINT),
            Node {
                margin: UiRect::axes(Val::Px(16.0), Val::Px(10.0)),
                flex_shrink: 0.0,
                ..default()
            },
        ));
    });
}

/// One store card: the reference's green card, picture over name, what it
/// fits, and the price chip at the foot.
fn card(g: &mut ChildSpawnerCommands, skin: &StoreSkin, owned: u32, icons: Option<&Icons>) {
    g.spawn((
        Button,
        Node {
            width: Val::Px(CARD_W),
            flex_direction: FlexDirection::Column,
            border: UiRect::all(Val::Px(1.0)),
            ..default()
        },
        BackgroundColor(ui::CARD),
        BorderColor::all(ui::RULE),
        ui::Hover::new(ui::CARD, ui::CARD_HOVER),
        OpenStore,
    ))
    .with_children(|c| {
        c.spawn(Node {
            height: Val::Px(150.0),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        })
        .with_children(|art| {
            picture(art, skin, icons, 112.0);
            if owned > 0 {
                art.spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        top: Val::Px(8.0),
                        right: Val::Px(8.0),
                        padding: UiRect::axes(Val::Px(6.0), Val::Px(4.0)),
                        ..default()
                    },
                    BackgroundColor(ui::ACCENT),
                    children![ui::strong(
                        if owned > 1 {
                            format!("OWNED x{owned}")
                        } else {
                            "OWNED".to_string()
                        },
                        10.5,
                        ui::TITLE
                    )],
                ));
            }
        });
        c.spawn((
            Node {
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(3.0),
                padding: UiRect::new(Val::Px(12.0), Val::Px(10.0), Val::Px(10.0), Val::Px(10.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.18)),
        ))
        .with_children(|t| {
            t.spawn(ui::strong(skin.name.to_uppercase(), 15.0, ui::TITLE));
            t.spawn(ui::strong(skin.covers.to_uppercase(), 11.0, ui::DIM));
            t.spawn(Node {
                justify_content: JustifyContent::FlexEnd,
                margin: UiRect::top(Val::Px(8.0)),
                ..default()
            })
            .with_children(|row| {
                let (bg, text, ink) = match (&skin.price, &skin.earn) {
                    (Some(p), _) => (ui::PRICE, p.clone(), ui::TITLE),
                    (None, Some(e)) => (ui::ROW_IDLE, e.to_uppercase(), ui::DIM),
                    (None, None) => (ui::ROW_IDLE, "COMING SOON".to_string(), ui::FAINT),
                };
                row.spawn((
                    Node {
                        padding: UiRect::axes(Val::Px(10.0), Val::Px(5.0)),
                        ..default()
                    },
                    BackgroundColor(bg),
                    children![ui::strong(text, 13.0, ink)],
                ));
            });
        });
    });
}

/// The skin as the store sells it: a render of its own model where it has
/// one (Rust's store shows the skinned item), else the item's own picture in
/// the skin's colours — the tint IS a multiply over the item's colours in
/// the game (`content/skins.toml`), and an image node's colour is a
/// multiply. No picture for the item draws its name.
fn picture(parent: &mut ChildSpawnerCommands, skin: &StoreSkin, icons: Option<&Icons>, px: f32) {
    let catalog = u16::try_from(skin.catalog).unwrap_or(0);
    let [r, g, b] = skin.tint;
    let look = icons.and_then(|i| {
        i.skin(catalog)
            .map(|h| (h, Color::WHITE))
            .or_else(|| i.item(&skin.covers).map(|h| (h, Color::srgb_u8(r, g, b))))
    });
    match look {
        Some((h, colour)) => {
            parent.spawn((
                ImageNode::new(h).with_color(colour),
                Node {
                    width: Val::Px(px),
                    height: Val::Px(px),
                    ..default()
                },
            ));
        }
        None => {
            parent.spawn(ui::strong(skin.covers.to_uppercase(), 13.0, ui::DIM));
        }
    }
}

/// INVENTORY: the reference's counts panel and refresh button, beside a
/// grid of what the wallet holds.
pub fn inventory(
    body: &mut ChildSpawnerCommands,
    locker: &Locker,
    icons: Option<&Icons>,
    panel_w: f32,
    scroll: f32,
) {
    let owned = locker.owned_rows();
    let store = locker.store.got().map_or(&[][..], Vec::as_slice);
    let held: u32 = owned.iter().map(|(_, n)| n).sum();
    let items = {
        let mut covers: Vec<&str> = owned
            .iter()
            .filter_map(|(c, _)| store.iter().find(|s| s.catalog == *c))
            .map(|s| s.covers.as_str())
            .collect();
        covers.sort_unstable();
        covers.dedup();
        covers.len()
    };

    body.spawn(ui::panel(panel_w)).with_children(|p| {
        p.spawn(ui::strong("INVENTORY", 26.0, ui::TITLE));
        // The reference's WOOD / METAL / CLOTH strip, in our terms.
        p.spawn(Node {
            flex_direction: FlexDirection::Row,
            margin: UiRect::vertical(Val::Px(8.0)),
            flex_shrink: 0.0,
            ..default()
        })
        .with_children(|row| {
            for (n, what) in [
                (held as usize, "SKINS"),
                (items, "ITEMS"),
                (store.len(), "IN STORE"),
            ] {
                row.spawn(Node {
                    flex_grow: 1.0,
                    flex_basis: Val::Px(0.0),
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    row_gap: Val::Px(4.0),
                    ..default()
                })
                .with_children(|c| {
                    c.spawn(ui::strong(n.to_string(), 30.0, ui::TITLE));
                    c.spawn(ui::strong(what, 11.0, ui::FAINT));
                });
            }
        });
        p.spawn(ui::label(
            "Skins your wallet owns on Elo Pros. They follow the wallet onto every \
             server and change how an item looks, never what it does. In game, press P \
             over an item at a workbench to wear one.",
            14.0,
            ui::DIM,
        ));
        p.spawn(ui::spacer());
        p.spawn((ui::button(panel_w - 42.0, false), RefreshInventory))
            .with_children(|b| {
                b.spawn(ui::strong("REFRESH INVENTORY", 13.0, ui::TITLE));
            });
        let line = match (&locker.owned, locker.owned_for) {
            (_, None) => "Not signed in: start the game from the Elo launcher.".to_string(),
            (Read::Waiting, _) => "Reading your wallet...".to_string(),
            (Read::Failed(why), _) => format!("{why}."),
            (Read::Got(_), Some(a)) => {
                let hex = a.to_hex();
                let w = core::str::from_utf8(&hex).unwrap_or("");
                format!("Wallet {}", crate::ui::streamer::hide(w))
            }
            (Read::Idle, _) => String::new(),
        };
        p.spawn(ui::label(line, 12.0, ui::FAINT));
    });

    body.spawn(ui::pane()).with_children(|pane| {
        pane.spawn((
            Node {
                height: Val::Px(44.0),
                align_items: AlignItems::Center,
                padding: UiRect::axes(Val::Px(16.0), Val::Px(0.0)),
                border: UiRect::bottom(Val::Px(1.0)),
                flex_shrink: 0.0,
                ..default()
            },
            BorderColor::all(ui::RULE),
            children![ui::strong("YOUR SKINS", 12.0, ui::FAINT)],
        ));
        pane.spawn(ui::scroll_area(scroll)).with_children(|area| {
            let say = match (&locker.owned, locker.owned_for) {
                (_, None) => Some(
                    "Start the game from the Elo launcher, signed in, and the skins your \
                     wallet owns show up here.",
                ),
                (Read::Waiting | Read::Idle, _) => Some("Reading your wallet..."),
                (Read::Failed(_), _) => {
                    Some("Couldn't read your wallet. REFRESH INVENTORY tries again.")
                }
                (Read::Got(o), _) if o.is_empty() => {
                    Some("You don't own any Gates skins yet. They're in the ITEM STORE.")
                }
                (Read::Got(_), _) => None,
            };
            if let Some(say) = say {
                area.spawn(empty(say.to_string()));
                return;
            }
            area.spawn(grid()).with_children(|g| {
                for (catalog, n) in owned {
                    tile(
                        g,
                        *catalog,
                        *n,
                        store.iter().find(|s| s.catalog == *catalog),
                        icons,
                    );
                }
            });
        });
    });
}

/// One inventory tile: the picture on a sunken square, the name under it.
fn tile(
    g: &mut ChildSpawnerCommands,
    catalog: u32,
    n: u32,
    skin: Option<&StoreSkin>,
    icons: Option<&Icons>,
) {
    g.spawn(Node {
        width: Val::Px(TILE_W),
        flex_direction: FlexDirection::Column,
        align_items: AlignItems::Center,
        row_gap: Val::Px(6.0),
        ..default()
    })
    .with_children(|t| {
        t.spawn((
            Node {
                width: Val::Px(TILE_W),
                height: Val::Px(TILE_W),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                border: UiRect::all(Val::Px(1.0)),
                ..default()
            },
            BackgroundColor(ui::WELL),
            BorderColor::all(ui::RULE),
        ))
        .with_children(|art| {
            match skin {
                Some(s) => picture(art, s, icons, 84.0),
                None => {
                    art.spawn(ui::strong(format!("#{catalog}"), 14.0, ui::DIM));
                }
            }
            if n > 1 {
                art.spawn((
                    ui::strong(format!("x{n}"), 12.0, ui::TEXT),
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(6.0),
                        top: Val::Px(4.0),
                        ..default()
                    },
                ));
            }
        });
        t.spawn((
            ui::strong(
                skin.map_or_else(|| format!("Skin #{catalog}"), |s| s.name.clone()),
                12.0,
                ui::TEXT,
            ),
            TextLayout::new_with_justify(Justify::Center),
        ));
    });
}

/// The wrapping grid both pages lay their pictures in.
fn grid() -> Node {
    Node {
        flex_direction: FlexDirection::Row,
        flex_wrap: FlexWrap::Wrap,
        align_content: AlignContent::FlexStart,
        column_gap: Val::Px(12.0),
        row_gap: Val::Px(14.0),
        padding: UiRect::all(Val::Px(16.0)),
        ..default()
    }
}

/// A sentence where a grid would be.
fn empty(text: String) -> impl Bundle {
    (
        ui::label(text, 14.0, ui::DIM),
        Node {
            margin: UiRect::all(Val::Px(20.0)),
            max_width: Val::Px(560.0),
            ..default()
        },
    )
}
