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
    let Some(f) = frame else {
        return serde_json::json!({ "ready": false }).to_string();
    };
    let st = &f.status;
    let meter = |(v, max): (u16, u16)| {
        if max == 0 {
            serde_json::Value::Null
        } else {
            serde_json::json!([v, max])
        }
    };
    // Newest first, as the page lists them.
    let mut history: Vec<_> = st
        .history
        .iter()
        .map(|r| {
            serde_json::json!({
                "goal": r.goal.label().as_str(),
                "outcome": r.outcome.word(),
                "why": r.outcome.why().map(crate::mind::Why::text),
                "gained": r.gained,
                "seconds": r.secs,
            })
        })
        .collect();
    history.reverse();
    serde_json::json!({
        "ready": true, "frame": f.id, "age_ms": f.at.elapsed().as_millis() as u64,
        "controller": st.source.label(),
        "mode": st.mode_line(),
        "paused": st.mode == crate::mind::Mode::Paused,
        "action": st.action(), "seconds": st.seconds,
        "goal": st.goal.map(|g| g.label().as_str().to_owned()),
        "goal_seconds": st.goal_secs,
        "reason": st.reason.as_str(),
        "history": history,
        "hp": st.hp, "hp_max": st.hp_max,
        "food": meter(st.food), "water": meter(st.water),
        "inventory": st.items().iter().map(|(n, c)| serde_json::json!({"name": n.as_str(), "count": c})).collect::<Vec<_>>(),
        "wood": st.wood,
        "trees": st.trees, "crafted": st.crafted, "retreats": st.retreats,
        "deaths": st.deaths, "respawns": st.respawns,
        "requests": st.requests, "decisions": st.decisions, "model_errors": st.failures,
        "input_tokens": st.input_tokens, "output_tokens": st.output_tokens,
        "requests_hour": [st.hour.0, st.hour.1], "requests_day": [st.day.0, st.day.1],
        "decode_errors": st.decode_errors,
    })
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
                status: status(),
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
            assert_eq!(state["action"], "Harvesting");
            assert_eq!(state["goal"], "gather_wood");
            assert_eq!(state["reason"], "scripted: next resource in the rotation is in view");
            assert_eq!(state["history"][0]["goal"], "craft:Stone Hatchet");
            assert_eq!(state["history"][1]["outcome"], "failed");
            assert_eq!((state["deaths"].as_u64(), state["respawns"].as_u64()), (Some(1), Some(1)));
            assert_eq!(state["inventory"][0]["name"], "Wood");
        }
        let decoded = image::load_from_memory(&a[body..]).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (WIDTH, HEIGHT));
        assert!(get(server.address, "/frame.jpg?n=6").starts_with(b"HTTP/1.1 409"));
        let state = get(server.address, "/state.json");
        let state = String::from_utf8_lossy(&state);
        assert!(state.contains("Scripted · paused: spend cap"));
        assert!(state.contains("\"paused\":true"));
        assert!(state.contains("\"wood\":42"));
        drop(capture);
        drop(server);
    }
    fn status() -> super::super::Status {
        use crate::mind::{Goal, History, Name, Outcome, Reason, Report, Why};
        let hatchet = Name::new(b"Stone Hatchet").unwrap();
        let mut history = History::default();
        history.push(Report {
            goal: Goal::GatherStone,
            outcome: Outcome::Failed(Why::NotFound),
            gained: 0,
            secs: 20,
        });
        history.push(Report {
            goal: Goal::Craft(hatchet),
            outcome: Outcome::Done,
            gained: 1,
            secs: 15,
        });
        let mut items = [(Name::EMPTY, 0); super::super::PAGE_ITEMS];
        items[0] = (Name::new(b"Wood").unwrap(), 42);
        super::super::Status {
            source: crate::mind::SourceKind::Scripted,
            mode: crate::mind::Mode::Paused,
            phase: crate::explorer::Phase::Harvesting,
            goal: Some(Goal::GatherWood),
            goal_secs: 4,
            reason: Reason::from_text("scripted: next resource in the rotation is in view"),
            history,
            seconds: 12,
            hp: 80,
            hp_max: 100,
            food: (300, 500),
            water: (0, 0),
            items,
            items_len: 1,
            wood: 42,
            deaths: 1,
            respawns: 1,
            trees: 0,
            crafted: 1,
            retreats: 1,
            requests: 3,
            decisions: 3,
            failures: 0,
            input_tokens: 0,
            output_tokens: 0,
            hour: (3, 600),
            day: (3, 7200),
            decode_errors: 0,
        }
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
