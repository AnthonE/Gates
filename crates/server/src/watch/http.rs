//! A loopback, read-only origin for one shared live view. JPEG encoding and
//! socket work stay on this worker; viewers share the last completed image.
use super::{Capture, Captured, HEIGHT, WIDTH};
use std::net::{SocketAddr, TcpListener};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const CLIENTS: usize = 16;
const HEADER_BYTES: usize = 8192;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const POLL: Duration = Duration::from_millis(50);
const MAX_JPEG_BYTES: usize = WIDTH as usize * HEIGHT as usize * 4;

pub struct Broadcast {
    pub address: SocketAddr,
    stop: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl Broadcast {
    pub fn start(address: SocketAddr) -> Result<(Self, Capture), String> {
        if !address.ip().is_loopback() {
            return Err(
                "the broadcast origin must bind to loopback; publish through a reviewed web proxy"
                    .into(),
            );
        }
        let listener = TcpListener::bind(address).map_err(|e| format!("watch listener: {e}"))?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let address = listener.local_addr().map_err(|e| e.to_string())?;
        let (tx, rx) = rtrb::RingBuffer::new(1);
        let stop = Arc::new(AtomicBool::new(false));
        let done = stop.clone();
        let worker = std::thread::Builder::new()
            .name("bot-broadcast".into())
            .spawn(move || {
                let result = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|e| e.to_string())
                    .and_then(|rt| rt.block_on(serve(listener, rx, done)));
                if let Err(e) = result {
                    eprintln!("jev-watch: {e}");
                }
            })
            .map_err(|e| e.to_string())?;
        Ok((
            Self {
                address,
                stop,
                worker: Some(worker),
            },
            Capture::new(tx),
        ))
    }
}

impl Drop for Broadcast {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

struct Frame {
    jpeg: Vec<u8>,
    status: super::Status,
    at: std::time::Instant,
    id: u64,
}

fn encode(captured: Captured) -> Result<Frame, String> {
    if captured.image.width() != WIDTH || captured.image.height() != HEIGHT {
        return Err("broadcast capture dimensions changed".into());
    }
    let rgb = captured
        .image
        .try_into_dynamic()
        .map_err(|_| "unsupported capture format")?
        .to_rgb8();
    let mut jpeg = Vec::new();
    // image's documented default JPEG quality (75).
    image::codecs::jpeg::JpegEncoder::new(&mut jpeg)
        .encode_image(&rgb)
        .map_err(|e| format!("broadcast JPEG: {e}"))?;
    if jpeg.len() > MAX_JPEG_BYTES {
        return Err("broadcast JPEG exceeded its bound".into());
    }
    Ok(Frame {
        jpeg,
        status: captured.status,
        at: captured.at,
        id: captured.id,
    })
}

async fn serve(
    listener: TcpListener,
    mut frames: rtrb::Consumer<Captured>,
    stop: Arc<AtomicBool>,
) -> Result<(), String> {
    let listener = tokio::net::TcpListener::from_std(listener).map_err(|e| e.to_string())?;
    let mut latest = None;
    let mut clients = tokio::task::JoinSet::new();
    let mut poll = tokio::time::interval(POLL);
    loop {
        tokio::select! {
            _ = poll.tick() => {
                if stop.load(Ordering::Relaxed) || frames.is_abandoned() { break; }
                if let Ok(frame) = frames.pop() { latest = Some(Arc::new(encode(frame)?)); }
            }
            connected = listener.accept() => {
                let (socket, _) = connected.map_err(|e| e.to_string())?;
                if clients.len() >= CLIENTS { drop(socket); continue; }
                let frame = latest.clone();
                clients.spawn(async move {
                    let _ = tokio::time::timeout(REQUEST_TIMEOUT, request(socket, frame)).await;
                });
            }
            _ = clients.join_next(), if !clients.is_empty() => {}
        }
    }
    clients.abort_all();
    Ok(())
}

fn route(header: &[u8]) -> (u16, &'static str) {
    let Ok(header) = std::str::from_utf8(header) else {
        return (400, "");
    };
    let mut line = header.lines().next().unwrap_or_default().split_whitespace();
    if line.next() != Some("GET") {
        return (405, "");
    }
    let path = line
        .next()
        .unwrap_or_default()
        .split('?')
        .next()
        .unwrap_or_default();
    if !matches!(line.next(), Some("HTTP/1.0" | "HTTP/1.1")) || line.next().is_some() {
        return (400, "");
    }
    match path {
        "/" | "/index.html" => (200, "page"),
        "/watch.js" => (200, "js"),
        "/watch.css" => (200, "css"),
        "/state.json" => (200, "state"),
        "/frame.jpg" => (200, "frame"),
        _ => (404, ""),
    }
}

fn state(frame: Option<&Frame>) -> String {
    match frame {
        None => serde_json::json!({ "ready": false }),
        Some(f) => serde_json::json!({
            "ready": true, "frame": f.id, "age_ms": f.at.elapsed().as_millis() as u64,
            "mode": if f.status.scripted { "Scripted demo" } else { "Jev 1.13.0" },
            "action": f.status.action(), "seconds": f.status.seconds,
            "hp": f.status.hp, "hp_max": f.status.hp_max,
            "wood": f.status.wood, "wood_gained": f.status.gained,
            "trees": f.status.trees, "retreats": f.status.retreats,
            "decisions": f.status.decisions, "model_errors": f.status.failures,
            "decode_errors": f.status.decode_errors,
        }),
    }
    .to_string()
}

async fn request(
    mut socket: tokio::net::TcpStream,
    frame: Option<Arc<Frame>>,
) -> std::io::Result<()> {
    let mut header = [0u8; HEADER_BYTES];
    let mut n = 0;
    let (mut code, route) = loop {
        if n == header.len() {
            break (431, "");
        }
        let got = socket.read(&mut header[n..]).await?;
        if got == 0 {
            return Ok(());
        }
        n += got;
        if header[..n].windows(4).any(|x| x == b"\r\n\r\n") {
            break route(&header[..n]);
        }
    };
    let json;
    if route == "frame" {
        let target = std::str::from_utf8(&header[..n])
            .unwrap_or_default()
            .split_whitespace()
            .nth(1)
            .unwrap_or_default();
        if let Some(requested) = target.strip_prefix("/frame.jpg?n=") {
            if requested.parse::<u64>().ok() != frame.as_ref().map(|f| f.id) {
                socket.write_all(b"HTTP/1.1 409 Frame changed\r\nContent-Length: 0\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n").await?;
                return socket.shutdown().await;
            }
        }
    }
    let (kind, body): (&str, &[u8]) = match route {
        "page" => ("text/html; charset=utf-8", include_bytes!("index.html")),
        "js" => ("text/javascript; charset=utf-8", include_bytes!("watch.js")),
        "css" => ("text/css; charset=utf-8", include_bytes!("watch.css")),
        "state" => {
            json = state(frame.as_deref());
            ("application/json", json.as_bytes())
        }
        "frame" => match &frame {
            Some(f) => ("image/jpeg", f.jpeg.as_slice()),
            None => {
                code = 503;
                ("text/plain", b"Waiting for the first frame")
            }
        },
        _ => ("text/plain", b"Request unavailable"),
    };
    // One response carries both image and state. A viewer whose round trip
    // exceeds the capture interval must not chase superseded frame IDs.
    let metadata = if route == "frame" {
        format!("X-Bot-State: {}\r\n", state(frame.as_deref()))
    } else {
        String::new()
    };
    let headers = format!("HTTP/1.1 {code} Response\r\nContent-Type: {kind}\r\nContent-Length: {}\r\n{metadata}Cache-Control: no-store\r\nConnection: close\r\nX-Content-Type-Options: nosniff\r\nContent-Security-Policy: default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self' blob:; connect-src 'self'; base-uri 'none'; frame-ancestors 'self'\r\n\r\n", body.len());
    socket.write_all(headers.as_bytes()).await?;
    socket.write_all(body).await?;
    socket.shutdown().await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get(address: SocketAddr, path: &str) -> Vec<u8> {
        use std::io::{Read, Write};
        let mut socket = std::net::TcpStream::connect(address).unwrap();
        socket.set_read_timeout(Some(REQUEST_TIMEOUT)).unwrap();
        write!(socket, "GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
        let mut response = Vec::new();
        socket.read_to_end(&mut response).unwrap();
        response
    }

    #[test]
    fn viewers_share_one_encoded_frame_and_state_ids_cannot_drift() {
        use bevy::asset::RenderAssetUsages;
        use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
        let (server, mut capture) = Broadcast::start("127.0.0.1:0".parse().unwrap()).unwrap();
        let response = get(server.address, "/state.json");
        assert!(String::from_utf8_lossy(&response).contains("\"ready\":false"));
        let image = bevy::prelude::Image::new_fill(
            Extent3d {
                width: WIDTH,
                height: HEIGHT,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            &[20, 40, 60, 255],
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::default(),
        );
        capture
            .frames
            .push(Captured {
                image,
                id: 7,
                at: std::time::Instant::now(),
                status: super::super::Status {
                    scripted: true,
                    phase: crate::explorer::Phase::Harvesting,
                    seconds: 12,
                    hp: 80,
                    hp_max: 100,
                    wood: 42,
                    gained: 42,
                    trees: 0,
                    retreats: 1,
                    decisions: 3,
                    failures: 0,
                    decode_errors: 0,
                },
            })
            .ok()
            .unwrap();
        let deadline = std::time::Instant::now() + REQUEST_TIMEOUT;
        loop {
            let response = get(server.address, "/state.json");
            if String::from_utf8_lossy(&response).contains("\"frame\":7") {
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(POLL);
        }
        let a = get(server.address, "/frame.jpg?n=7");
        let b = get(server.address, "/frame.jpg");
        let body = a.windows(4).position(|b| b == b"\r\n\r\n").unwrap() + 4;
        let other_body = b.windows(4).position(|b| b == b"\r\n\r\n").unwrap() + 4;
        assert_eq!(
            &a[body..],
            &b[other_body..],
            "viewers consume the same encoded image"
        );
        for response in [&a, &b] {
            let end = response.windows(4).position(|b| b == b"\r\n\r\n").unwrap();
            let headers = std::str::from_utf8(&response[..end]).unwrap();
            let metadata = headers
                .lines()
                .find_map(|line| line.strip_prefix("X-Bot-State: "))
                .unwrap();
            let state: serde_json::Value = serde_json::from_str(metadata).unwrap();
            assert_eq!(state["frame"], 7);
            assert_eq!(state["wood"], 42);
            assert_eq!(state["action"], "Gathering wood");
        }
        let decoded = image::load_from_memory(&a[body..]).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (WIDTH, HEIGHT));
        assert!(get(server.address, "/frame.jpg?n=6").starts_with(b"HTTP/1.1 409"));
        let state = get(server.address, "/state.json");
        let state = String::from_utf8_lossy(&state);
        assert!(state.contains("Scripted demo"));
        assert!(state.contains("\"wood\":42"));
        drop(capture);
        drop(server);
    }
    #[test]
    fn the_watch_origin_has_no_control_or_file_routes() {
        for target in [
            "/control",
            "/../Cargo.toml",
            "/%2e%2e/Cargo.toml",
            "/frame.jpg/../key",
        ] {
            assert_eq!(
                route(format!("GET {target} HTTP/1.1\r\n\r\n").as_bytes()).0,
                404
            );
        }
        assert_eq!(route(b"POST / HTTP/1.1\r\n\r\n").0, 405);
        assert_eq!(
            route(b"GET /frame.jpg?n=2 HTTP/1.1\r\n\r\n"),
            (200, "frame")
        );
        assert_eq!(route(b"GET / HTTP/1.1 extra\r\n\r\n").0, 400);
        assert!(Broadcast::start("0.0.0.0:0".parse().unwrap()).is_err());
    }
}
