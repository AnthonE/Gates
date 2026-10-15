//! `jev-bot --set-face`: an agent sets its own Elo Pros name and picture,
//! the face every Gates nametag shows (`faces.rs` reads it back by wallet).
//!
//! Three requests to the platform's `meter/faces.py`, and the one rule that
//! matters: **the key signs a text it composed itself** (`agentkey`'s
//! `sign_face`). The platform's `sign_this` is only compared against it,
//! never signed, so an origin that asked for something else gets a refusal
//! and no signature. The day comes from the platform's text (its clock
//! decides which day it verifies against), after the key checks its shape.

use agentkey::AgentKey;
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
/// bytes, or both. Returns what the platform says the face is now.
pub fn set_face(
    key: &AgentKey,
    origin: &str,
    name: Option<&str>,
    png: Option<&[u8]>,
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
    let prepared = post(&format!("{origin}/api/face/prepare"), &Value::Object(ask))?;
    let sign_this = prepared["sign_this"]
        .as_str()
        .ok_or("the platform's prepare answer has no sign_this")?;
    let at = prepared["at"]
        .as_u64()
        .ok_or("the platform's prepare answer has no at")?;
    let day = day_in(sign_this).ok_or("the platform's text has no day")?;
    // A platform that predates `kind` leaves it out of its text; the key
    // composes both and signs whichever one the platform asked for.
    let (text, signature) = [Some(KIND), None]
        .into_iter()
        .filter_map(|kind| key.sign_face(name, sha.as_deref(), kind, at, day))
        .find(|(text, _)| text == sign_this)
        .or_else(|| key.sign_face(name, sha.as_deref(), Some(KIND), at, day))
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
    if text.contains("\"kind\":") {
        set.insert("kind".into(), json!(KIND));
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

    #[test]
    fn nothing_to_set_and_an_oversize_picture_cost_no_request() {
        let mut one = [0u8; 32];
        one[31] = 1;
        let key = AgentKey::from_secret(&one).unwrap();
        // An unroutable origin: reaching the network would fail differently.
        let origin = "http://192.0.2.1:9";
        assert!(set_face(&key, origin, None, None)
            .unwrap_err()
            .contains("needs --name"));
        let big = vec![0u8; FACE_PIC_MAX_BYTES + 1];
        assert!(set_face(&key, origin, None, Some(&big))
            .unwrap_err()
            .contains("cap"));
    }
}
