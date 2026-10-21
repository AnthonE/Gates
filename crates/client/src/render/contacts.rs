//! Contacts, watched and kept (`crate::contacts`): who has been near long
//! enough to be met, the mugshot taken when they are, the killer who goes in
//! as an enemy, and the file the book lives in.
//!
//! **The mugshot is a screen grab**, Rust's: when someone is met and is on
//! screen, the frame is captured (`Screenshot`) and the square around their
//! body is cut out and shrunk to `contacts::MUGSHOT_PX`. One grab at a time,
//! at most one every [`GRAB_GAP_S`], and a contact is grabbed again only
//! when the last one is `contacts::MUGSHOT_REFRESH_S` old.
//!
//! **Kept on this computer.** Natively the book is `contacts/<wallet>.json`
//! beside the settings file and each mugshot a PNG under
//! `contacts/mugshots/`; in a page the book is in `localStorage` and the
//! mugshots last the session.

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::window::PrimaryWindow;
use protocol::Address;

use super::friends::{hex, me_in_world};
use super::screen::Who;
use super::Net;
use crate::contacts::{self, Book, Rel, Sighting, Watch, MUGSHOT_PX};

/// How often the watch looks, seconds. A dwell of three seconds read four
/// times a second is plenty, and each look casts a line per player near.
const LOOK_S: f32 = 0.25;
/// Seconds between two grabs: a grab reads a whole frame back.
const GRAB_GAP_S: f64 = 2.0;
/// Seconds between two writes of a book that only moved its times.
const SAVE_GAP_S: f64 = 60.0;
/// Where a body's head is over its feet, metres, for the grab and the line.
const HEAD_M: f32 = 1.7;
/// Mugshot files read off disk per frame while the page is up.
const LOADS_PER_FRAME: usize = 4;
/// A grab that has not come back in this long, seconds, is given up on.
const GRAB_TIMEOUT_S: f64 = 10.0;

/// The book, and everything around it.
#[derive(Resource, Default)]
pub struct Contacts {
    pub book: Book,
    watch: Watch,
    /// Whose book is loaded: the player's wallet, `guest` without one.
    owner: Option<String>,
    /// The wallet `owner` was named for, so the check is not a string a frame.
    owner_of: Option<Option<Address>>,
    /// Bumped on every change a page would draw.
    pub gen: u64,
    /// Mugshot textures by wallet.
    pics: Vec<(String, Handle<Image>)>,
    /// Wallets whose mugshot file was looked for and is not there.
    missing: Vec<String>,
    /// A grab is out.
    grabbing: bool,
    last_grab: f64,
    since_look: f32,
    /// The book moved since it was written, and when it last was.
    dirty: bool,
    last_save: f64,
    /// The death the killer mark was taken for.
    dead_seen: bool,
}

impl Contacts {
    /// The mugshot to draw for `wallet`, once it is loaded.
    pub fn pic(&self, wallet: &str) -> Option<Handle<Image>> {
        self.pics
            .iter()
            .find(|(w, _)| w == wallet)
            .map(|(_, h)| h.clone())
    }

    /// Hold `h` as `wallet`'s mugshot.
    pub fn put_pic(&mut self, wallet: String, h: Handle<Image>) {
        self.pics.retain(|(w, _)| *w != wallet);
        self.missing.retain(|w| *w != wallet);
        self.pics.push((wallet, h));
    }

    /// A change the player made (a mark, a forget): written now.
    pub fn changed(&mut self) {
        self.gen += 1;
        self.dirty = true;
        self.save();
    }

    /// Mark a contact, and write the book.
    pub fn set_rel(&mut self, wallet: &Address, rel: Rel) {
        if self.book.set_rel(wallet, rel) {
            self.changed();
        }
    }

    /// Write a note on a contact, and the book. False when nothing changed.
    pub fn set_note(&mut self, wallet: &Address, note: &str) -> bool {
        let changed = self.book.set_note(wallet, note);
        if changed {
            self.changed();
        }
        changed
    }

    /// Take a contact out of the book, and its mugshot with it.
    pub fn forget(&mut self, wallet: &Address) {
        if self.book.forget(wallet) {
            let w = hex(*wallet);
            self.pics.retain(|(p, _)| *p != w);
            remove_mugshot(&w);
            self.changed();
        }
    }

    fn save(&mut self) {
        let Some(owner) = self.owner.as_deref() else {
            return;
        };
        if !self.dirty {
            return;
        }
        self.dirty = false;
        store_book(owner, &self.book.to_json());
    }

    /// Load `owner`'s book, writing the last one's first.
    fn switch(&mut self, owner: String) {
        self.save();
        self.book = load_book(&owner);
        self.owner = Some(owner);
        self.watch.clear();
        self.pics.clear();
        self.missing.clear();
        self.gen += 1;
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn dir() -> Option<std::path::PathBuf> {
    Some(crate::config::settings_path()?.parent()?.join("contacts"))
}

#[cfg(not(target_arch = "wasm32"))]
fn load_book(owner: &str) -> Book {
    dir()
        .and_then(|d| std::fs::read(d.join(format!("{owner}.json"))).ok())
        .map(|b| Book::parse(&b))
        .unwrap_or_default()
}

#[cfg(not(target_arch = "wasm32"))]
fn store_book(owner: &str, json: &str) {
    if let Some(d) = dir() {
        if let Err(e) = crate::config::save(&d.join(format!("{owner}.json")), json) {
            warn!("contacts: {e}");
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn mugshot_path(wallet: &str) -> Option<std::path::PathBuf> {
    Some(dir()?.join("mugshots").join(format!("{wallet}.png")))
}

#[cfg(not(target_arch = "wasm32"))]
fn remove_mugshot(wallet: &str) {
    if let Some(p) = mugshot_path(wallet) {
        let _ = std::fs::remove_file(p);
    }
}

/// Write a mugshot off the frame: an encode and a file.
#[cfg(not(target_arch = "wasm32"))]
fn store_mugshot(wallet: String, px: Vec<u8>) {
    let Some(path) = mugshot_path(&wallet) else {
        return;
    };
    std::thread::spawn(move || {
        let Some(png) = encode(&px) else {
            return;
        };
        if let Some(d) = path.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let tmp = path.with_extension("png.tmp");
        if std::fs::write(&tmp, png).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    });
}

#[cfg(not(target_arch = "wasm32"))]
fn load_mugshot(wallet: &str) -> Option<Vec<u8>> {
    decode(&std::fs::read(mugshot_path(wallet)?).ok()?)
}

/// A page keeps the book in `localStorage`; it is small. Mugshots, at a few
/// kilobytes each, would not fit two hundred times, so they last the session.
#[cfg(target_arch = "wasm32")]
fn storage_key(owner: &str) -> String {
    format!("gates.contacts.{owner}")
}

#[cfg(target_arch = "wasm32")]
fn load_book(owner: &str) -> Book {
    web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .and_then(|s| s.get_item(&storage_key(owner)).ok().flatten())
        .map(|t| Book::parse(t.as_bytes()))
        .unwrap_or_default()
}

#[cfg(target_arch = "wasm32")]
fn store_book(owner: &str, json: &str) {
    if let Some(s) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
        let _ = s.set_item(&storage_key(owner), json);
    }
}

#[cfg(target_arch = "wasm32")]
fn remove_mugshot(_wallet: &str) {}

#[cfg(target_arch = "wasm32")]
fn store_mugshot(_wallet: String, _px: Vec<u8>) {}

#[cfg(target_arch = "wasm32")]
fn load_mugshot(_wallet: &str) -> Option<Vec<u8>> {
    None
}

/// A mugshot's RGBA as PNG.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
fn encode(px: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, MUGSHOT_PX, MUGSHOT_PX);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut w = enc.write_header().ok()?;
        w.write_image_data(px).ok()?;
    }
    Some(out)
}

/// One of our own mugshot files back to RGBA: exactly the shape `encode`
/// writes, or nothing.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
fn decode(bytes: &[u8]) -> Option<Vec<u8>> {
    let dec = png::Decoder::new(std::io::Cursor::new(bytes));
    let mut reader = dec.read_info().ok()?;
    let info = reader.info();
    if info.width != MUGSHOT_PX
        || info.height != MUGSHOT_PX
        || info.color_type != png::ColorType::Rgba
        || info.bit_depth != png::BitDepth::Eight
    {
        return None;
    }
    let mut buf = vec![0; reader.output_buffer_size()?];
    reader.next_frame(&mut buf).ok()?;
    buf.truncate((MUGSHOT_PX * MUGSHOT_PX * 4) as usize);
    Some(buf)
}

/// A mugshot's RGBA as a texture.
pub fn mugshot_image(px: Vec<u8>) -> Image {
    Image::new(
        Extent3d {
            width: MUGSHOT_PX,
            height: MUGSHOT_PX,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        px,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    )
}

/// Load the player's own book once their wallet is known in a world.
pub fn own_book(mut contacts: ResMut<Contacts>, net: Option<NonSend<Net>>, who: Res<Who>) {
    let me = me_in_world(net.as_deref(), &who);
    if contacts.owner_of != Some(me) {
        contacts.owner_of = Some(me);
        contacts.switch(me.map(hex).unwrap_or_else(|| "guest".into()));
    }
}

/// Who is near, for how long; the book; and the grab when someone is met.
#[allow(clippy::too_many_arguments)]
pub fn watch(
    mut commands: Commands,
    mut contacts: ResMut<Contacts>,
    mut net: NonSendMut<Net>,
    time: Res<Time<Real>>,
    cam: Query<(&Camera, &GlobalTransform), With<super::rig::EyeCam>>,
    window: Query<&Window, With<PrimaryWindow>>,
    (ui, chat): (
        Option<Res<super::panels::Ui>>,
        Option<Res<super::chat::Chat>>,
    ),
) {
    contacts.since_look += time.delta_secs();
    if contacts.since_look < LOOK_S || contacts.owner.is_none() {
        return;
    }
    let dt = std::mem::take(&mut contacts.since_look);
    let clock = time.elapsed_secs_f64();
    let now = crate::shot::now_secs();
    let watching = net.session.watching.is_some();
    let core = &mut net.session.core;
    if core.dead || watching {
        contacts.watch.clear();
        return;
    }
    let Ok((camera, eye_xf)) = cam.single() else {
        return;
    };
    let eye = eye_xf.translation();
    let at = core.render_tick();
    let own = core.player_id;
    // Everyone alive and tagged with a wallet, where they stand now.
    let mut near: Vec<(u32, Address, Vec3)> = Vec::new();
    let mut rs = client_core::interp::RemoteState::default();
    for &(id, e) in core.view.entities.iter() {
        if id == own || e.dead || id & sim_core::limits::MOB_ID_TAG != 0 {
            continue;
        }
        let Some(tag) = core.tag(id).filter(|t| !t.address.is_guest()) else {
            continue;
        };
        if !core.interp.sample(id, at, &mut rs) {
            continue;
        }
        let feet = Vec3::new(rs.x, rs.y, rs.z);
        if feet.distance(eye) <= contacts::SEEN_M {
            near.push((id, tag.address, feet));
        }
    }
    // Only over a clear line: a wall between is not a meeting.
    near.retain(|(_, _, feet)| {
        let head = (feet.x, feet.y + HEAD_M, feet.z);
        crate::ui::names::clear_line(core, (eye.x, eye.y, eye.z), head)
    });
    let close: Vec<Address> = near.iter().map(|(_, a, _)| *a).collect();
    let met = contacts.watch.step(dt, &close);
    // Everyone met who is owed a mugshot; the first one on screen gets it.
    let mut owed: Vec<(Address, Vec3)> = Vec::new();
    for (address, _crossed) in met {
        let Some(&(id, _, feet)) = near.iter().find(|(_, a, _)| *a == address) else {
            continue;
        };
        let name = crate::ui::names::of(core, id);
        match contacts.book.seen(&address, &name, core.is_agent(id), now) {
            Sighting::New | Sighting::Again => {
                contacts.gen += 1;
                contacts.dirty = true;
                contacts.save();
            }
            Sighting::Still => contacts.dirty = true,
            Sighting::Full => continue,
        }
        if contacts.book.wants_mugshot(&address, now) {
            owed.push((address, feet));
        }
    }
    if contacts.dirty && clock - contacts.last_save >= SAVE_GAP_S {
        contacts.last_save = clock;
        contacts.save();
    }
    if owed.is_empty() {
        return;
    }
    if contacts.grabbing && clock - contacts.last_grab >= GRAB_TIMEOUT_S {
        contacts.grabbing = false;
    }
    if contacts.grabbing || clock - contacts.last_grab < GRAB_GAP_S {
        return;
    }
    // A grab is of the screen: not while a panel or the composer is over it.
    let covered =
        ui.is_some_and(|u| u.panel != super::panels::Panel::None) || chat.is_some_and(|c| c.open());
    if covered {
        return;
    }
    let Ok(win) = window.single() else {
        return;
    };
    let scale = win.scale_factor();
    let to_px = |p: Vec3| {
        camera
            .world_to_viewport(eye_xf, p)
            .ok()
            .map(|v| (v.x * scale, v.y * scale))
    };
    let (w, h) = (win.physical_width(), win.physical_height());
    let Some((address, rect)) = owed.into_iter().find_map(|(address, feet)| {
        let f = to_px(feet)?;
        let top = to_px(feet + Vec3::Y * HEAD_M)?;
        Some((address, contacts::mugshot_rect(f, top, w, h)?))
    }) else {
        return;
    };
    contacts.grabbing = true;
    contacts.last_grab = clock;
    let wallet = hex(address);
    commands.spawn(Screenshot::primary_window()).observe(
        move |cap: On<ScreenshotCaptured>,
              mut contacts: ResMut<Contacts>,
              mut images: ResMut<Assets<Image>>| {
            contacts.grabbing = false;
            let bgra = match cap.image.texture_descriptor.format {
                TextureFormat::Bgra8UnormSrgb | TextureFormat::Bgra8Unorm => true,
                TextureFormat::Rgba8UnormSrgb | TextureFormat::Rgba8Unorm => false,
                _ => return,
            };
            let Some(frame) = cap.image.data.as_deref() else {
                return;
            };
            if rect.1 + rect.2 > cap.image.height() {
                return;
            }
            let Some(px) = contacts::crop_mugshot(frame, cap.image.width(), bgra, rect) else {
                return;
            };
            let h = images.add(mugshot_image(px.clone()));
            contacts.put_pic(wallet.clone(), h);
            contacts.book.shot(&address, crate::shot::now_secs());
            store_mugshot(wallet.clone(), px);
            contacts.changed();
        },
    );
}

/// Whoever killed you goes in the book as an enemy (`Book::killed_by`).
pub fn killer(mut contacts: ResMut<Contacts>, net: NonSend<Net>) {
    let core = &net.session.core;
    if !core.dead {
        contacts.dead_seen = false;
        return;
    }
    if contacts.dead_seen || contacts.owner.is_none() {
        return;
    }
    contacts.dead_seen = true;
    let k = core.own_death_killer;
    if k == 0 || k == core.player_id || k & sim_core::limits::MOB_ID_TAG != 0 {
        return;
    }
    let Some(tag) = core.tag(k).filter(|t| !t.address.is_guest()) else {
        return;
    };
    let name = crate::ui::names::of(core, k);
    contacts.book.killed_by(
        &tag.address,
        &name,
        core.is_agent(k),
        crate::shot::now_secs(),
    );
    contacts.changed();
}

/// While the contacts page is up, bring the mugshots it shows off disk.
pub fn load_pics(
    mut contacts: ResMut<Contacts>,
    ui: Option<Res<super::panels::Ui>>,
    mut images: ResMut<Assets<Image>>,
) {
    if ui.is_none_or(|u| u.panel != super::panels::Panel::Contacts) {
        return;
    }
    let want: Vec<String> = contacts
        .book
        .contacts
        .iter()
        .filter(|c| c.mugshot_at != 0)
        .map(|c| c.wallet.clone())
        .filter(|w| contacts.pic(w).is_none() && !contacts.missing.contains(w))
        .take(LOADS_PER_FRAME)
        .collect();
    if want.is_empty() {
        return;
    }
    for w in want {
        match load_mugshot(&w) {
            Some(px) => {
                let h = images.add(mugshot_image(px));
                contacts.put_pic(w, h);
            }
            None => contacts.missing.push(w),
        }
    }
    contacts.gen += 1;
}

/// Leaving the world: write what moved.
pub fn flush(mut contacts: ResMut<Contacts>) {
    contacts.save();
    contacts.watch.clear();
}

pub fn register(app: &mut App) {
    app.init_resource::<Contacts>()
        // The killer on every world screen: the death raises `Screen::Dead`
        // the frame after `core.dead` lands, and the mark must not miss it.
        .add_systems(
            Update,
            (own_book, killer).chain().run_if(super::world_running),
        )
        .add_systems(
            Update,
            (watch, load_pics)
                .chain()
                .after(killer)
                .run_if(in_state(super::Screen::InWorld)),
        )
        .add_systems(OnExit(super::Screen::InWorld), flush);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mugshot_file_reads_back_as_it_was_written() {
        let px: Vec<u8> = (0..MUGSHOT_PX * MUGSHOT_PX * 4)
            .map(|i| (i % 251) as u8)
            .collect();
        let png = encode(&px).expect("encodes");
        assert_eq!(decode(&png), Some(px));
        assert_eq!(decode(b"not a png"), None);
    }
}
