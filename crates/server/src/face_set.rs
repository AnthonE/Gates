//! `jev-bot --set-face`: an agent sets its own Elo Pros name and picture,
//! the face every Gates nametag shows (`faces.rs` reads it back by wallet).
//!
//! Three requests to the platform's `meter/faces.py`, and the one rule that
//! matters: **the key signs a text it composed itself** (`agentkey`'s
//! `sign_face`). The platform's `sign_this` is only compared against it,
//! never signed, so an origin that asked for something else gets a refusal
//! and no signature. The day comes from the platform's text (its clock
//! decides which day it verifies against), after the key checks its shape.

use agentkey::{AgentKey, Labels};
use base64::Engine;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::time::Duration;

/// The platform's picture cap (`faces.PIC_MAX_BYTES`), checked here first
/// so a wrong file costs no request.
pub const FACE_PIC_MAX_BYTES: usize = 256 * 1024;
const TIMEOUT: Duration = Duration::from_secs(20);
const MAX_RESPONSE_BYTES: u64 = 64 * 1024;
/// What an agent's face declares it is.
const KIND: &str = "agent";

/// The `day:` line of a play text, or `None`.
pub fn day_in(text: &str) -> Option<&str> {
    text.lines().find_map(|l| l.strip_prefix("day: "))
}

/// Set this key's face at `origin` (`https://elopros.com`): `name`, a PNG's
/// bytes, or both, with the harness and model that play it when `labels`
/// names them. Returns what the platform says the face is now.
pub fn set_face(
    key: &AgentKey,
    origin: &str,
    name: Option<&str>,
    png: Option<&[u8]>,
    labels: Labels,
) -> Result<Value, String> {
    if name.is_none() && png.is_none() {
        return Err("--set-face needs --name, --pic or both".into());
    }
    // The platform trims a name before it binds it; so does this.
    let name = name.map(str::trim);
    if png.is_some_and(|p| p.len() > FACE_PIC_MAX_BYTES) {
        return Err(format!(
            "the picture is over the platform's {FACE_PIC_MAX_BYTES}-byte cap"
        ));
    }
    let origin = origin.trim_end_matches('/');
    let hex = key.address().to_hex();
    let wallet = String::from_utf8_lossy(&hex).into_owned();
    let sha = png.map(|p| {
        Sha256::digest(p)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    });
    let mut ask = Map::new();
    ask.insert("wallet".into(), json!(wallet));
    if let Some(name) = name {
        ask.insert("name".into(), json!(name));
    }
    if let Some(sha) = &sha {
        ask.insert("picture_sha256".into(), json!(sha));
    }
    // An agent's face says it is one (self-declared, like the game's tag).
    ask.insert("kind".into(), json!(KIND));
    for (field, label) in [("harness", labels.harness), ("model", labels.model)] {
        if let Some(label) = label {
            ask.insert(field.into(), json!(label));
        }
    }
    let prepared = post(&format!("{origin}/api/face/prepare"), &Value::Object(ask))?;
    let sign_this = prepared["sign_this"]
        .as_str()
        .ok_or("the platform's prepare answer has no sign_this")?;
    let at = prepared["at"]
        .as_u64()
        .ok_or("the platform's prepare answer has no at")?;
    let day = day_in(sign_this).ok_or("the platform's text has no day")?;
    // A platform that predates `kind`, or the labels, leaves them out of its
    // text; the key composes each and signs whichever one it asked for.
    let tries = [
        (labels, Some(KIND)),
        (Labels::NONE, Some(KIND)),
        (Labels::NONE, None),
    ];
    let (sent, kind, text, signature) = tries
        .into_iter()
        .filter_map(|(l, kind)| {
            let (text, sig) = key.sign_face(name, sha.as_deref(), kind, l, at, day)?;
            Some((l, kind, text, sig))
        })
        .find(|(_, _, text, _)| text == sign_this)
        .or_else(|| {
            let (text, sig) = key.sign_face(name, sha.as_deref(), Some(KIND), labels, at, day)?;
            Some((labels, Some(KIND), text, sig))
        })
        .ok_or("the platform would refuse that name (1-16 printable characters)")?;
    if text != sign_this {
        return Err(format!(
            "refused to sign: the platform asked for a different text than this key composes\n  asked:    {sign_this:?}\n  composed: {text:?}"
        ));
    }
    let sig: String = signature.0.iter().map(|b| format!("{b:02x}")).collect();
    let mut set = Map::new();
    set.insert("wallet".into(), json!(wallet));
    set.insert("at".into(), json!(at));
    set.insert("signature".into(), json!(format!("0x{sig}")));
    if let Some(name) = name {
        set.insert("name".into(), json!(name));
    }
    if let Some(kind) = kind {
        set.insert("kind".into(), json!(kind));
    }
    for (field, label) in [("harness", sent.harness), ("model", sent.model)] {
        if let Some(label) = label {
            set.insert(field.into(), json!(label));
        }
    }
    if let Some(png) = png {
        set.insert(
            "picture_b64".into(),
            json!(base64::engine::general_purpose::STANDARD.encode(png)),
        );
    }
    post(&format!("{origin}/api/face"), &Value::Object(set))
}

/// POST JSON, read JSON back; a refusal is the platform's own `error`.
fn post(url: &str, body: &Value) -> Result<Value, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .http_status_as_error(false)
        .build()
        .into();
    let mut res = agent
        .post(url)
        .content_type("application/json")
        .send(body.to_string())
        .map_err(|e| format!("{url}: {e}"))?;
    let status = res.status().as_u16();
    let text = res
        .body_mut()
        .with_config()
        .limit(MAX_RESPONSE_BYTES)
        .read_to_string()
        .map_err(|e| format!("{url}: {e}"))?;
    let value: Value =
        serde_json::from_str(&text).map_err(|_| format!("{url}: HTTP {status}, not JSON"))?;
    if !(200..300).contains(&status) {
        let why = value["error"]
            .as_str()
            .or_else(|| value["detail"].as_str())
            .map(str::to_owned)
            .unwrap_or_else(|| value.to_string());
        return Err(format!("{url}: HTTP {status}: {why}"));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_day_is_read_from_the_platforms_text() {
        let t = "elo play\naction: profile\nwallet: 0xab\nday: 2026-10-14\ndetail: {}";
        assert_eq!(day_in(t), Some("2026-10-14"));
        assert_eq!(day_in("elo play\ndetail: {}"), None);
    }

    /// A platform stand-in: answers `prepare` with `sign_this`, and hands
    /// the `set` body it then gets to the test.
    fn platform(sign_this: String) -> (String, std::thread::JoinHandle<Value>) {
        use std::io::{BufRead, BufReader, Read, Write};
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = format!("http://{}", l.local_addr().unwrap());
        let set = std::thread::spawn(move || {
            let mut sent = Value::Null;
            for stream in l.incoming().take(2) {
                let mut s = BufReader::new(stream.unwrap());
                let (mut first, mut len) = (String::new(), 0usize);
                s.read_line(&mut first).unwrap();
                loop {
                    let mut h = String::new();
                    s.read_line(&mut h).unwrap();
                    if h.trim().is_empty() {
                        break;
                    }
                    if let Some(v) = h.to_ascii_lowercase().strip_prefix("content-length:") {
                        len = v.trim().parse().unwrap();
                    }
                }
                let mut body = vec![0u8; len];
                s.read_exact(&mut body).unwrap();
                let answer = if first.contains("/prepare") {
                    json!({"sign_this": sign_this, "at": 5}).to_string()
                } else {
                    sent = serde_json::from_slice(&body).unwrap();
                    json!({"name": "Rook"}).to_string()
                };
                let _ = s.get_mut().write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                         Content-Length: {}\r\nConnection: close\r\n\r\n{answer}",
                        answer.len()
                    )
                    .as_bytes(),
                );
            }
            sent
        });
        (origin, set)
    }

    /// **The labels are signed where the platform asks for them, and left
    /// out where it does not** (C1): one that takes them gets them in its
    /// text and in the set; one that predates them gets the face it always
    /// did.
    #[test]
    fn labels_ride_the_face_only_when_the_platform_takes_them() {
        let mut one = [0u8; 32];
        one[31] = 1;
        let key = AgentKey::from_secret(&one).unwrap();
        let head = "elo play\naction: profile\nwallet: 0x7e5f4552091a69125d5dfcb7b8c2659029395bdf\nday: 2026-01-02\ndetail: ";
        let labels = Labels {
            harness: Some("h/1"),
            model: Some("test-model"),
        };
        let (origin, set) = platform(format!(
            r#"{head}{{"at":5,"harness":"h/1","kind":"agent","model":"test-model","name":"Rook"}}"#
        ));
        set_face(&key, &origin, Some("Rook"), None, labels).unwrap();
        let sent = set.join().unwrap();
        assert_eq!(
            (
                sent["harness"].as_str(),
                sent["model"].as_str(),
                sent["kind"].as_str()
            ),
            (Some("h/1"), Some("test-model"), Some("agent"))
        );
        let (origin, set) = platform(format!(r#"{head}{{"at":5,"kind":"agent","name":"Rook"}}"#));
        set_face(&key, &origin, Some("Rook"), None, labels).unwrap();
        let sent = set.join().unwrap();
        assert!(sent.get("harness").is_none() && sent.get("model").is_none());
        assert_eq!(sent["kind"], "agent");
    }

    #[test]
    fn nothing_to_set_and_an_oversize_picture_cost_no_request() {
        let mut one = [0u8; 32];
        one[31] = 1;
        let key = AgentKey::from_secret(&one).unwrap();
        // An unroutable origin: reaching the network would fail differently.
        let origin = "http://192.0.2.1:9";
        assert!(set_face(&key, origin, None, None, Labels::NONE)
            .unwrap_err()
            .contains("needs --name"));
        let big = vec![0u8; FACE_PIC_MAX_BYTES + 1];
        assert!(set_face(&key, origin, None, Some(&big), Labels::NONE)
            .unwrap_err()
            .contains("cap"));
    }
}
