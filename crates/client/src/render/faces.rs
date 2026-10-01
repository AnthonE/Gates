//! **Names and faces in the world**: the nametag over the player you are
//! aiming at, and the pictures players set on their Elo Pros account page.
//!
//! The shard says who each player id is (`EventMsg::Tag`, read through
//! `ClientCore::tag`); this file draws it. Two halves:
//!
//! - [`Pics`], a small bounded cache of 64×64 textures keyed by (wallet,
//!   picture revision). A picture is fetched once, off the frame — a thread
//!   and `ureq` natively, `fetch` in a page — decoded and shrunk off the frame
//!   too, and handed to Bevy as a plain RGBA image. A failed fetch is
//!   remembered as failed, so a missing picture costs one request, not one
//!   per frame.
//! - The nametag: aim-only and 8 m (`ui::names::NAMETAG_REACH_M`), and only
//!   over a clear line (`ui::names::clear_line`), so a wall hides a name the
//!   way it hides a body. Drawn just above the crosshair — it is aim-only, so
//!   the crosshair is where the eye already is.
//!
//! **Bevy draws, it does not decide**: which player, and whether they can be
//! seen, are `ui::interact::resolve_nametag` and `ui::names::clear_line`.

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use protocol::Address;

use super::input::Look;
use super::Net;
use crate::look::{pitch_u8, yaw_u16};
use crate::ui::interact::{self, SwingAim};
use crate::ui::names;

/// Side of every picture texture, in pixels. Nametags and the menu chip draw
/// at 28–40 px, so 64 is sharp on a 2x display and 16 KiB of GPU memory.
pub const PIC_PX: u32 = 64;
/// How many pictures stay decoded. Past it the least recently used goes.
const PIC_CACHE: usize = 64;
/// How many fetches may be out at once.
const PIC_IN_FLIGHT: usize = 4;
/// Refused past this many bytes: the platform caps a picture at 256 KiB.
pub const PIC_MAX_BYTES: usize = 256 * 1024;
/// Refused past this side before a pixel is allocated: the platform's cap.
const PIC_MAX_SIDE: u32 = 512;
/// The nametag picture, pixels on screen.
const TAG_PIC_PX: f32 = 28.0;
const TAG_FONT_PX: f32 = 15.0;
/// Where the nametag sits: just above the crosshair (the prompt sits below).
const TAG_TOP_PCT: f32 = 43.0;

enum PicState {
    Pending,
    Ready(Handle<Image>),
    Failed,
}

struct PicRow {
    address: Address,
    pic: u32,
    state: PicState,
    used: u64,
}

type PicResult = (Address, u32, Option<Vec<u8>>);
type PicTx = tokio::sync::mpsc::UnboundedSender<PicResult>;

/// The picture cache. See the module header.
#[derive(Resource)]
pub struct Pics {
    /// `scheme://host` of the platform. The desktop client learns it from
    /// `--servers` (`ui::names::origin_of`); a page and everyone else use
    /// `ui::names::PLATFORM_ORIGIN`.
    pub origin: String,
    rows: Vec<PicRow>,
    in_flight: usize,
    clock: u64,
    tx: PicTx,
    rx: tokio::sync::mpsc::UnboundedReceiver<PicResult>,
}

impl Default for Pics {
    fn default() -> Self {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        Self {
            origin: names::PLATFORM_ORIGIN.to_string(),
            rows: Vec::with_capacity(PIC_CACHE),
            in_flight: 0,
            clock: 0,
            tx,
            rx,
        }
    }
}

impl Pics {
    /// The picture for (`address`, `pic`), if it has landed. Asks for it the
    /// first time; `None` while it is on its way, or if it has none.
    pub fn want(&mut self, address: Address, pic: u32) -> Option<Handle<Image>> {
        if pic == 0 || address.is_guest() {
            return None;
        }
        self.clock += 1;
        let clock = self.clock;
        if let Some(row) = self
            .rows
            .iter_mut()
            .find(|r| r.address == address && r.pic == pic)
        {
            row.used = clock;
            return match &row.state {
                PicState::Ready(h) => Some(h.clone()),
                PicState::Pending | PicState::Failed => None,
            };
        }
        if self.in_flight >= PIC_IN_FLIGHT {
            return None; // asked again next frame
        }
        if self.rows.len() >= PIC_CACHE {
            // The least recently used row that is not still on its way.
            let oldest = self
                .rows
                .iter()
                .enumerate()
                .filter(|(_, r)| !matches!(r.state, PicState::Pending))
                .min_by_key(|(_, r)| r.used)
                .map(|(i, _)| i)?;
            self.rows.swap_remove(oldest);
        }
        let tag = client_core::core::Tag {
            id: 1,
            address,
            name: protocol::Name::EMPTY,
            pic,
        };
        let url = names::pic_url(&self.origin, &tag)?;
        self.rows.push(PicRow {
            address,
            pic,
            state: PicState::Pending,
            used: clock,
        });
        self.in_flight += 1;
        fetch(url, address, pic, self.tx.clone());
        None
    }
}

/// Turn landed fetches into textures.
pub fn pump(mut pics: ResMut<Pics>, mut images: ResMut<Assets<Image>>) {
    while let Ok((address, pic, rgba)) = pics.rx.try_recv() {
        pics.in_flight = pics.in_flight.saturating_sub(1);
        let Some(row) = pics
            .rows
            .iter_mut()
            .find(|r| r.address == address && r.pic == pic)
        else {
            continue;
        };
        row.state = match rgba {
            Some(px) => PicState::Ready(images.add(Image::new(
                Extent3d {
                    width: PIC_PX,
                    height: PIC_PX,
                    depth_or_array_layers: 1,
                },
                TextureDimension::D2,
                px,
                TextureFormat::Rgba8UnormSrgb,
                RenderAssetUsages::RENDER_WORLD,
            ))),
            None => PicState::Failed,
        };
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn fetch(url: String, address: Address, pic: u32, tx: PicTx) {
    std::thread::spawn(move || {
        let px = get_bytes(&url, PIC_MAX_BYTES).and_then(|b| decode(&b));
        let _ = tx.send((address, pic, px));
    });
}

#[cfg(target_arch = "wasm32")]
fn fetch(url: String, address: Address, pic: u32, tx: PicTx) {
    wasm_bindgen_futures::spawn_local(async move {
        let px = crate::net::web::fetch_bytes(&url, PIC_MAX_BYTES)
            .await
            .and_then(|b| decode(&b));
        let _ = tx.send((address, pic, px));
    });
}

/// One capped GET, natively. `None` for any failure, or a body past `cap`
/// (refused, never truncated: a cut PNG is a broken one).
#[cfg(not(target_arch = "wasm32"))]
pub fn get_bytes(url: &str, cap: usize) -> Option<Vec<u8>> {
    let mut res = ureq::get(url).call().ok()?;
    let bytes = res
        .body_mut()
        .with_config()
        .limit((cap + 1) as u64)
        .read_to_vec()
        .ok()?;
    (bytes.len() <= cap).then_some(bytes)
}

/// A PNG → [`PIC_PX`]² RGBA, box-filtered. Refuses anything that is not a
/// square PNG of at most [`PIC_MAX_SIDE`] before allocating its pixels.
pub fn decode(bytes: &[u8]) -> Option<Vec<u8>> {
    let mut dec = png::Decoder::new(std::io::Cursor::new(bytes));
    dec.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = dec.read_info().ok()?;
    let (w, h) = (reader.info().width, reader.info().height);
    if w != h || w == 0 || w > PIC_MAX_SIDE {
        return None;
    }
    let mut buf = vec![0; reader.output_buffer_size()?];
    let frame = reader.next_frame(&mut buf).ok()?;
    let channels = match frame.color_type {
        png::ColorType::Grayscale => 1,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        png::ColorType::Indexed => return None,
    };
    let side = w as usize;
    let stride = frame.line_size;
    let px = |x: usize, y: usize| -> [u32; 4] {
        let o = y * stride + x * channels;
        let p = &buf[o..o + channels];
        match channels {
            1 => [p[0] as u32, p[0] as u32, p[0] as u32, 255],
            2 => [p[0] as u32, p[0] as u32, p[0] as u32, p[1] as u32],
            3 => [p[0] as u32, p[1] as u32, p[2] as u32, 255],
            _ => [p[0] as u32, p[1] as u32, p[2] as u32, p[3] as u32],
        }
    };
    let out_side = PIC_PX as usize;
    let mut out = vec![0u8; out_side * out_side * 4];
    for oy in 0..out_side {
        let (y0, y1) = (
            oy * side / out_side,
            ((oy + 1) * side / out_side).max(oy * side / out_side + 1),
        );
        for ox in 0..out_side {
            let (x0, x1) = (
                ox * side / out_side,
                ((ox + 1) * side / out_side).max(ox * side / out_side + 1),
            );
            let mut sum = [0u32; 4];
            for y in y0..y1.min(side) {
                for x in x0..x1.min(side) {
                    let p = px(x, y);
                    for c in 0..4 {
                        sum[c] += p[c];
                    }
                }
            }
            let n = ((y1.min(side) - y0) * (x1.min(side) - x0)).max(1) as u32;
            let o = (oy * out_side + ox) * 4;
            for c in 0..4 {
                out[o + c] = (sum[c] / n) as u8;
            }
        }
    }
    Some(out)
}

// ---------------------------------------------------------------------------
// The nametag
// ---------------------------------------------------------------------------

#[derive(Component)]
struct Nametag;
#[derive(Component)]
struct NametagPic;
#[derive(Component)]
struct NametagName;

/// What the nametag shows now, so it is only rewritten when that changes.
#[derive(Resource, Default)]
struct Shown {
    who: Option<(u32, String, bool)>,
}

fn setup(mut commands: Commands) {
    commands
        .spawn((
            super::WorldEntity,
            Nametag,
            Node {
                position_type: PositionType::Absolute,
                top: Val::Percent(TAG_TOP_PCT),
                width: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                column_gap: Val::Px(8.0),
                ..default()
            },
            Visibility::Hidden,
            Pickable::IGNORE,
        ))
        .with_children(|row| {
            row.spawn((
                NametagPic,
                Node {
                    width: Val::Px(TAG_PIC_PX),
                    height: Val::Px(TAG_PIC_PX),
                    ..default()
                },
                ImageNode::default(),
                Visibility::Hidden,
                Pickable::IGNORE,
            ));
            row.spawn((
                NametagName,
                Text::new(""),
                super::ui::font_bold(TAG_FONT_PX),
                TextColor(Color::srgba(0.96, 0.94, 0.88, 0.95)),
                super::ui::TEXT_SHADOW,
                Pickable::IGNORE,
            ));
        });
}

/// Who the crosshair is on, and their name over it.
#[allow(clippy::type_complexity)]
fn nametag(
    mut net: NonSendMut<Net>,
    look: Res<Look>,
    mut pics: ResMut<Pics>,
    mut shown: ResMut<Shown>,
    mut root: Query<&mut Visibility, (With<Nametag>, Without<NametagPic>)>,
    mut pic: Query<(&mut ImageNode, &mut Visibility), (With<NametagPic>, Without<Nametag>)>,
    mut text: Query<&mut Text, With<NametagName>>,
) {
    let core = &mut net.session.core;
    let who = if core.dead {
        None
    } else {
        let [x, y, z] = core.predict.render_position();
        let aim = SwingAim {
            x,
            y,
            z,
            yaw: yaw_u16(look.yaw),
            pitch: pitch_u8(look.pitch),
            crouched: core.crouched(),
        };
        let own = core.player_id;
        interact::resolve_nametag(aim, own, &core.view.entities, |id| core.tag(id).is_some())
            .filter(|&(_, eye, head)| names::clear_line(core, eye, head))
            .and_then(|(id, _, _)| core.tag(id).copied())
    };
    let handle = who.and_then(|t| pics.want(t.address, t.pic));
    let next = who.map(|t| (t.id, names::label(Some(&t), t.id), handle.is_some()));
    if next == shown.who {
        return;
    }
    shown.who = next.clone();
    for mut v in &mut root {
        *v = if next.is_some() {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
    for mut t in &mut text {
        t.0 = next.as_ref().map(|n| n.1.clone()).unwrap_or_default();
    }
    for (mut img, mut v) in &mut pic {
        match &handle {
            Some(h) => {
                img.image = h.clone();
                *v = Visibility::Inherited;
            }
            None => *v = Visibility::Hidden,
        }
    }
}

fn forget(mut shown: ResMut<Shown>) {
    shown.who = None;
}

pub fn register(app: &mut App) {
    app.init_resource::<Pics>()
        .init_resource::<Shown>()
        .add_systems(Update, pump)
        // The nodes are `WorldEntity`, so they are built with the world and
        // `world_teardown` takes them.
        .add_systems(OnEnter(super::Screen::Loading), (forget, setup))
        .add_systems(Update, nametag.run_if(in_state(super::Screen::InWorld)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_square_png_shrinks_to_the_texture_and_anything_else_is_refused() {
        let side = 128u32;
        let rgba: Vec<u8> = (0..side * side)
            .flat_map(|i| [(i % 256) as u8, 40, 200, 255])
            .collect();
        let mut bytes = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut bytes, side, side);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            let mut w = enc.write_header().unwrap();
            w.write_image_data(&rgba).unwrap();
        }
        let out = decode(&bytes).expect("a square png decodes");
        assert_eq!(out.len(), (PIC_PX * PIC_PX * 4) as usize);
        assert_eq!(&out[1..4], &[40, 200, 255]);

        let mut wide = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut wide, 4, 2);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            let mut w = enc.write_header().unwrap();
            w.write_image_data(&[9u8; 4 * 2 * 4]).unwrap();
        }
        assert!(decode(&wide).is_none(), "not square");
        assert!(decode(b"\x89PNG not really").is_none());
    }
}
