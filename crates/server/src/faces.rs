//! **What does this player call themselves, and what do they look like?**
//! Rust asks Steam for a persona name and an avatar. A Gates shard asks the
//! platform instead: `GET {origin}/api/face/{wallet}` (`meter/faces.py` in
//! `AnthonE/scry-forge`), the name and picture that wallet set on its Elo
//! Pros account page, keyed by the wallet and nothing else — no vow.
//!
//! The address is the identity and the shard already proved it; this is only
//! the label. So the policy is the gentlest of the three platform reads: a
//! failed read ([`Face::Unknown`]) changes nothing, a name the wire cannot
//! carry becomes no name (never a truncated one), and nobody waits on it —
//! the read runs after the player is already in the world, and until it lands
//! everyone sees the short address.
//!
//! ## Where this runs
//!
//! On a blocking thread spawned by the accept loop (`net.rs`), once per join.
//! What it learns reaches the sim thread as a [`crate::slot::FaceMsg`] and
//! goes out as `EventMsg::Tag`; it never enters `sim-core`.

use std::time::Duration;

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(4);

/// The whole answer is a wallet, a 16-byte name, a url and two numbers.
pub const MAX_RESPONSE_BYTES: usize = 4 * 1024;

/// Where to ask, from `shard.toml` (`faces_origin`). Absent ⇒ every player
/// is their short address, which is every test's and every unarmed shard's
/// state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    pub origin: Option<String>,
    pub timeout: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self::off()
    }
}

impl Config {
    pub fn off() -> Self {
        Self {
            origin: None,
            timeout: DEFAULT_TIMEOUT,
        }
    }
}

/// What a read said.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Face {
    /// The platform answered: the name (possibly empty) and the picture's
    /// revision (0 for none).
    Known { name: protocol::Name, pic: u32 },
    /// We could not look. Changes nothing.
    Unknown,
}

/// One player's face. Blocking; the caller is a `spawn_blocking` task.
pub fn face_of(cfg: &Config, wallet: &str) -> Face {
    let Some(origin) = cfg.origin.as_deref() else {
        return Face::Unknown;
    };
    if !crate::entitle::is_wallet(wallet) {
        return Face::Unknown;
    }
    let url = format!("{origin}/api/face/{wallet}");
    match crate::entitle::get_capped_status(&url, cfg.timeout, MAX_RESPONSE_BYTES) {
        crate::entitle::Got::Body(body) => parse(&body),
        crate::entitle::Got::NotFound | crate::entitle::Got::Failed => Face::Unknown,
    }
}

/// A `/api/face/{wallet}` body → a face. A name the wire's [`protocol::Name`]
/// refuses (too long, not printable ASCII) is no name rather than a cut one:
/// a truncated name is a different name. `pic` is the first eight hex digits
/// of the picture's sha256; a real picture never encodes as 0.
pub fn parse(body: &str) -> Face {
    use serde_json::Value;
    let Ok(v) = serde_json::from_str::<Value>(body) else {
        return Face::Unknown;
    };
    if !v.is_object() {
        return Face::Unknown;
    }
    let name = v
        .get("name")
        .and_then(Value::as_str)
        .and_then(|s| protocol::Name::new(s.trim()))
        .unwrap_or(protocol::Name::EMPTY);
    let pic = v
        .get("pic")
        .and_then(Value::as_str)
        .filter(|s| s.len() == 8)
        .and_then(|s| u32::from_str_radix(s, 16).ok())
        .map_or(0, |p| p.max(1));
    Face::Known { name, pic }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn known(body: &str) -> (String, u32) {
        match parse(body) {
            Face::Known { name, pic } => (name.as_str().to_string(), pic),
            Face::Unknown => panic!("a readable body is an answer: {body}"),
        }
    }

    #[test]
    fn a_set_face_reads_and_a_bad_name_is_no_name_never_a_cut_one() {
        let set = r#"{"wallet":"0xab","name":"Ash Walker","picture":"/api/face/0xab/pic.png?v=1a2b3c4d","pic":"1a2b3c4d","updated":1}"#;
        assert_eq!(known(set), ("Ash Walker".into(), 0x1a2b_3c4d));
        assert_eq!(known(r#"{"name":null,"pic":null}"#), (String::new(), 0));
        assert_eq!(
            known(r#"{"name":"seventeen chars!!","pic":"zz"}"#),
            (String::new(), 0)
        );
        assert_eq!(
            known(r#"{"name":"Zoë","pic":"00000000"}"#),
            (String::new(), 1)
        );
    }

    #[test]
    fn could_not_look_is_unknown() {
        assert_eq!(parse("<html>502</html>"), Face::Unknown);
        assert_eq!(parse("[1,2]"), Face::Unknown);
        assert_eq!(
            face_of(&Config::off(), "0x00000000000000000000000000000000000000a1"),
            Face::Unknown
        );
        let armed = Config {
            origin: Some("https://origin.test".into()),
            ..Config::off()
        };
        assert_eq!(face_of(&armed, "not a wallet"), Face::Unknown);
    }
}
