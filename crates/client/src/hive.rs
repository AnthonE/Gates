//! Friend chat, as arithmetic: the Elo Pros hive (a Nostr relay, buzz) carries
//! a whisper between two friends in a private two-person room, the same room
//! elopros.com's chat page and the elo launcher open.
//!
//! **The key is the wallet's own chat key**, the one elopros.com and the
//! launcher derive: the wallet signs [`derivation_message`] once and the key is
//! `sha256(r‖s) mod n` of that signature (`meter/hive.py` `derive_seckey`,
//! `elo-hive/src/voice.rs`). So one wallet speaks with one key whichever
//! client it is in. It is a BIP-340 schnorr key: it moves no money and signs
//! no transaction, and it lives in the launcher's own voice file
//! (`voice.nsec`, 0600) so the launcher's chat and this game are one voice.
//!
//! A friend's key is read off `GET /api/hive/link/{wallet}`, the public link a
//! wallet signs once; a room is opened with buzz's kind:41010 command and
//! spoken in with kind:9 `#h <room>`, over the player's own NIP-42
//! authenticated socket (buzz binds an event's author to the socket's).
//!
//! Desktop only: a page holds no key (`Cargo.toml`, `agentkey`'s rule).
//! `render/dms.rs` owns the socket and the thread.

use k256::schnorr::signature::hazmat::{PrehashSigner, PrehashVerifier};
use k256::schnorr::{Signature, SigningKey, VerifyingKey};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

/// NIP-42: a client authenticating to a relay.
pub const KIND_AUTH: i64 = 22242;
/// A line in a room (NIP-29), `#h` the room.
pub const KIND_ROOM_NOTE: i64 = 9;
/// buzz's `dms open --pubkey` command: the relay makes (or reuses) the
/// private room for the signer and one other key.
pub const KIND_DM_OPEN: i64 = 41010;
/// NIP-29 group members, `d` the room, a `p` per member.
pub const KIND_MEMBERS: i64 = 39002;
/// The note a chat key signs to link itself to a wallet.
pub const KIND_LINK: i64 = 1;
/// The play action a wallet signs to link its chat key (`playauth.py`).
pub const LINK_ACTION: &str = "chatkey";
/// The file the launcher keeps its chat key in (`elo-hive` `KEY_NAME`).
pub const KEY_NAME: &str = "voice.nsec";
/// The largest whisper drawn: a chat line, like the shard's.
pub const MAX_LINE_CHARS: usize = 280;

/// The text a wallet signs once to derive its chat key — byte for byte
/// `meter/hive.py` `derivation_message(wallet)`.
pub fn derivation_message(wallet: &str) -> String {
    format!(
        "elo hive voice v1\nwallet: {}\nthis signature IS my hive voice key.\n\
         sign it only to derive your Elo Pros chat identity — never on any other site.",
        wallet.trim().to_lowercase()
    )
}

/// What the chat key signs to say it speaks for `wallet` (`link_content`).
pub fn link_content(wallet: &str) -> String {
    format!(
        "elo chat key\nwallet: {}\nthis key speaks for this wallet in Elo Pros chat.",
        wallet.trim().to_lowercase()
    )
}

/// The play text the wallet signs to link `npub` (`hive.py` `link_detail`
/// inside `play_message("chatkey", …, day_of(at))`).
pub fn link_sign_text(wallet: &str, at: u64, npub: &str) -> String {
    format!(
        "elo play\naction: {LINK_ACTION}\nwallet: {}\nday: {}\ndetail: {}",
        wallet.trim().to_lowercase(),
        crate::friends::utc_day(at),
        json!({"at": at, "npub": npub})
    )
}

/// Where the launcher keeps its chat key (`elo_hive::voice::default_dir`):
/// `ELO_VOICE_DIR`, else `voice/` under its state root.
pub fn voice_dir() -> std::path::PathBuf {
    use std::path::PathBuf;
    let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    if let Some(p) = env("ELO_VOICE_DIR") {
        return PathBuf::from(p);
    }
    if let Some(p) = env("ELO_STATE") {
        return PathBuf::from(p).join("voice");
    }
    #[cfg(windows)]
    let base = ["LOCALAPPDATA", "APPDATA", "USERPROFILE"]
        .into_iter()
        .find_map(env)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    #[cfg(not(windows))]
    let base = env("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| env("HOME").map(|h| PathBuf::from(h).join(".local/state")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("elo").join("voice")
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

fn is_lower_hex(s: &str, len: usize) -> bool {
    s.len() == len && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// `npub1…` → the x-only key as lowercase hex.
pub fn npub_hex(npub: &str) -> Option<String> {
    let (hrp, data) = bech32::decode(npub.trim()).ok()?;
    (hrp.as_str() == "npub" && data.len() == 32).then(|| hex(&data))
}

/// The x-only key as `npub1…`.
pub fn npub_of(pubkey_hex: &str) -> Option<String> {
    let bytes = unhex(pubkey_hex).filter(|b| b.len() == 32)?;
    bech32::encode::<bech32::Bech32>(bech32::Hrp::parse_unchecked("npub"), &bytes).ok()
}

/// A chat key. Never printed.
pub struct Voice {
    key: SigningKey,
}

impl std::fmt::Debug for Voice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Voice({})", self.npub())
    }
}

impl Voice {
    /// The wallet's chat key from its EIP-191 signature over
    /// [`derivation_message`]: `sha256(r‖s) mod n`, `0x01` appended and
    /// rehashed in the zero case. `v` is dropped: wallets disagree on it.
    pub fn from_wallet_signature(signature_hex: &str) -> Result<Voice, String> {
        use k256::elliptic_curve::ops::Reduce;
        let raw = unhex(signature_hex.trim().trim_start_matches("0x"))
            .filter(|r| r.len() >= 64)
            .ok_or("a wallet signature is 65 bytes of hex")?;
        let mut rs = raw[..64].to_vec();
        loop {
            let digest = Sha256::digest(&rs);
            let d = <k256::Scalar as Reduce<k256::U256>>::reduce_bytes(&digest);
            if !bool::from(d.is_zero()) {
                return Voice::from_secret(&d.to_bytes());
            }
            rs.push(1);
        }
    }

    pub fn from_secret(bytes: &[u8]) -> Result<Voice, String> {
        let arr: [u8; 32] = bytes.try_into().map_err(|_| "a secret key is 32 bytes")?;
        SigningKey::from_bytes(&arr)
            .map(|key| Voice { key })
            .map_err(|_| "not a valid secp256k1 secret key".into())
    }

    pub fn from_nsec(nsec: &str) -> Result<Voice, String> {
        let (hrp, data) = bech32::decode(nsec.trim()).map_err(|e| format!("not a key: {e}"))?;
        if hrp.as_str() != "nsec" {
            return Err("not an nsec".into());
        }
        Voice::from_secret(&data)
    }

    pub fn to_nsec(&self) -> String {
        bech32::encode::<bech32::Bech32>(bech32::Hrp::parse_unchecked("nsec"), &self.key.to_bytes())
            .expect("32 bytes encode")
    }

    pub fn pubkey_hex(&self) -> String {
        hex(&self.key.verifying_key().to_bytes())
    }

    pub fn npub(&self) -> String {
        npub_of(&self.pubkey_hex()).expect("a key is 32 bytes")
    }

    /// Sign an event: fills in `pubkey`, `id` and `sig`.
    pub fn sign(
        &self,
        created_at: i64,
        kind: i64,
        tags: Vec<Vec<String>>,
        content: String,
    ) -> Event {
        let pubkey = self.pubkey_hex();
        let id = event_id(&pubkey, created_at, kind, &tags, &content);
        // `sign_prehash`: Nostr signs the 32-byte id itself, never its hash.
        let sig: Signature = self
            .key
            .sign_prehash(&unhex(&id).expect("an id is hex"))
            .expect("a 32-byte prehash signs");
        Event {
            id,
            pubkey,
            created_at,
            kind,
            tags,
            content,
            sig: hex(&sig.to_bytes()),
        }
    }
}

/// The NIP-01 id: sha256 of compact `[0, pubkey, created_at, kind, tags, content]`.
pub fn event_id(
    pubkey: &str,
    created_at: i64,
    kind: i64,
    tags: &[Vec<String>],
    content: &str,
) -> String {
    let bytes = serde_json::to_vec(&json!([0, pubkey, created_at, kind, tags, content]))
        .expect("plain values");
    hex(&Sha256::digest(&bytes))
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Event {
    pub id: String,
    pub pubkey: String,
    pub created_at: i64,
    pub kind: i64,
    pub tags: Vec<Vec<String>>,
    pub content: String,
    pub sig: String,
}

impl Event {
    /// The id recomputed and the signature checked. Every event read off the
    /// relay goes through this: the relay's bytes are a stranger's.
    pub fn verify(&self) -> bool {
        if !is_lower_hex(&self.id, 64)
            || !is_lower_hex(&self.pubkey, 64)
            || !is_lower_hex(&self.sig, 128)
        {
            return false;
        }
        if event_id(
            &self.pubkey,
            self.created_at,
            self.kind,
            &self.tags,
            &self.content,
        ) != self.id
        {
            return false;
        }
        let (Some(pk), Some(sig), Some(id)) =
            (unhex(&self.pubkey), unhex(&self.sig), unhex(&self.id))
        else {
            return false;
        };
        let (Ok(vk), Ok(sig)) = (
            VerifyingKey::from_bytes(&pk),
            Signature::try_from(sig.as_slice()),
        ) else {
            return false;
        };
        vk.verify_prehash(&id, &sig).is_ok()
    }

    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).expect("plain fields")
    }

    /// Every value of tag `name`.
    pub fn tag_values<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        self.tags
            .iter()
            .filter(move |t| t.first().map(String::as_str) == Some(name))
            .filter_map(|t| t.get(1).map(String::as_str))
    }

    pub fn tag<'a>(&'a self, name: &'a str) -> Option<&'a str> {
        self.tag_values(name).next()
    }
}

/// The AUTH event for a relay's challenge.
pub fn auth_event(voice: &Voice, relay: &str, challenge: &str, now: i64) -> Event {
    voice.sign(
        now,
        KIND_AUTH,
        vec![
            vec!["relay".into(), relay.into()],
            vec!["challenge".into(), challenge.into()],
        ],
        String::new(),
    )
}

/// The command that opens (or finds) the private room with `other_hex`.
pub fn dm_open_event(voice: &Voice, other_hex: &str, now: i64) -> Event {
    voice.sign(
        now,
        KIND_DM_OPEN,
        vec![vec!["p".into(), other_hex.into()]],
        String::new(),
    )
}

/// One line in room `room`.
pub fn note_event(voice: &Voice, room: &str, text: &str, now: i64) -> Event {
    voice.sign(
        now,
        KIND_ROOM_NOTE,
        vec![vec!["h".into(), room.into()]],
        text.into(),
    )
}

/// buzz answers a kind:41010 with `response:{"channel_id":"<uuid>",…}` in the
/// OK reason. Only that shape is read: anything else is "we don't know the
/// room", never a guessed one.
pub fn dm_room_of(reason: &str) -> Option<String> {
    let body = reason.strip_prefix("response:")?;
    let v: Value = serde_json::from_str(body).ok()?;
    let id = v.get("channel_id")?.as_str()?.to_ascii_lowercase();
    room_ok(&id).then_some(id)
}

/// A room id is a relay-assigned UUID.
pub fn room_ok(id: &str) -> bool {
    id.len() == 36
        && id.char_indices().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => c == '-',
            _ => c.is_ascii_hexdigit(),
        })
}

/// A frame off the relay.
#[derive(Debug, Clone, PartialEq)]
pub enum Frame {
    Auth(String),
    Event(String, Box<Event>),
    Eose(String),
    Ok(String, bool, String),
    Closed(String, String),
    Other,
}

/// Read one relay frame. Anything that does not parse is [`Frame::Other`].
pub fn frame(text: &str) -> Frame {
    let Ok(Value::Array(a)) = serde_json::from_str::<Value>(text) else {
        return Frame::Other;
    };
    let s = |i: usize| a.get(i).and_then(Value::as_str).unwrap_or("").to_string();
    match a.first().and_then(Value::as_str) {
        Some("AUTH") if a.get(1).is_some_and(Value::is_string) => Frame::Auth(s(1)),
        Some("EVENT") => match a.get(2).cloned().map(serde_json::from_value::<Event>) {
            Some(Ok(ev)) => Frame::Event(s(1), Box::new(ev)),
            _ => Frame::Other,
        },
        Some("EOSE") => Frame::Eose(s(1)),
        Some("OK") => Frame::Ok(
            s(1),
            a.get(2).and_then(Value::as_bool).unwrap_or(false),
            s(3),
        ),
        Some("CLOSED") => Frame::Closed(s(1), s(2)),
        _ => Frame::Other,
    }
}

/// `["REQ", id, filter]`.
pub fn req(sub: &str, filter: Value) -> String {
    json!(["REQ", sub, filter]).to_string()
}

/// `["EVENT", event]`.
pub fn publish(ev: &Event) -> String {
    json!(["EVENT", ev.to_json()]).to_string()
}

/// `["AUTH", event]`.
pub fn auth(ev: &Event) -> String {
    json!(["AUTH", ev.to_json()]).to_string()
}

/// The private rooms this key is in, each with the one other member.
/// Learned from kind:39002 member lists; a room of any size but two is not a
/// whisper and is left out, as `chat.js` leaves it out.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Rooms {
    /// (room id, the other member's key), newest member list per room.
    pub rows: Vec<(String, String)>,
}

/// The most rooms remembered: a member list per friend, with headroom.
pub const MAX_ROOMS: usize = 300;

impl Rooms {
    /// Take one member list. True when it changed the rooms.
    pub fn take(&mut self, ev: &Event, me: &str) -> bool {
        if ev.kind != KIND_MEMBERS {
            return false;
        }
        let Some(room) = ev
            .tag("d")
            .filter(|r| room_ok(r))
            .map(str::to_ascii_lowercase)
        else {
            return false;
        };
        let people: Vec<&str> = ev.tag_values("p").collect();
        let other = match people.as_slice() {
            [a, b] if *a == me => Some(*b),
            [a, b] if *b == me => Some(*a),
            _ => None,
        };
        let had = self.rows.iter().position(|(r, _)| *r == room);
        match (had, other) {
            (Some(i), Some(o)) if self.rows[i].1 == o => false,
            (Some(i), Some(o)) => {
                self.rows[i].1 = o.to_string();
                true
            }
            (Some(i), None) => {
                self.rows.remove(i);
                true
            }
            (None, Some(o)) if self.rows.len() < MAX_ROOMS && o != me => {
                self.rows.push((room, o.to_string()));
                true
            }
            _ => false,
        }
    }

    pub fn with(&self, other_hex: &str) -> Option<&str> {
        self.rows
            .iter()
            .find(|(_, o)| o == other_hex)
            .map(|(r, _)| r.as_str())
    }

    pub fn other(&self, room: &str) -> Option<&str> {
        self.rows
            .iter()
            .find(|(r, _)| r == room)
            .map(|(_, o)| o.as_str())
    }

    pub fn ids(&self) -> Vec<String> {
        self.rows.iter().map(|(r, _)| r.clone()).collect()
    }
}

/// A whisper's text as drawn: one line, no control characters, capped.
pub fn tidy_line(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(MAX_LINE_CHARS)
        .collect::<String>()
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const WALLET: &str = "0x3e20ab25eb721dd4b691516238df14f0f5d3f7a3";
    /// `eth_account`'s signature of the derivation text with key 0x7a×32: the
    /// launcher's vector (`elo-hive/src/link.rs`).
    const DERIVED: &str = "0x026c0b8e8e1f842384bc6534bc630cc469f741217567e32e8f946148d6b0bd72\
                           4395acee54fdf9a353d797d3ce98124bb46352e48dc31636271b0e75aba8283a1c";
    const NPUB: &str = "npub1pavc42tfwxnyvparu5ljky2k3papvnewmstvnx59tz2cddm4avsskvfs3n";

    #[test]
    fn the_wallets_chat_key_is_the_one_the_site_and_launcher_make() {
        let v = Voice::from_wallet_signature(DERIVED).unwrap();
        assert_eq!(v.npub(), NPUB);
        assert_eq!(npub_hex(NPUB).as_deref(), Some(v.pubkey_hex().as_str()));
        let again = Voice::from_nsec(&v.to_nsec()).unwrap();
        assert_eq!(again.npub(), NPUB);
        assert!(derivation_message(&WALLET.to_uppercase()).contains(WALLET));
    }

    #[test]
    fn the_link_text_is_the_meters_byte_for_byte() {
        assert_eq!(
            link_sign_text(
                "0x3E20AB25EB721DD4B691516238DF14F0F5D3F7A3",
                1_790_000_000,
                "-"
            ),
            "elo play\naction: chatkey\nwallet: 0x3e20ab25eb721dd4b691516238df14f0f5d3f7a3\n\
             day: 2026-09-21\ndetail: {\"at\":1790000000,\"npub\":\"-\"}"
        );
    }

    #[test]
    fn a_signed_event_verifies_and_a_tampered_one_does_not() {
        let v = Voice::from_wallet_signature(DERIVED).unwrap();
        let room = "0f8fad5b-d9cb-469f-a165-70867728950e";
        let ev = note_event(&v, room, "meet at the gate \"now\"\n", 1_790_000_000);
        assert!(ev.verify());
        assert_eq!(ev.tag("h"), Some(room));
        let mut bad = ev.clone();
        bad.content.push('!');
        assert!(!bad.verify());
        let mut bad = ev.clone();
        bad.sig
            .replace_range(0..2, if &ev.sig[..2] == "00" { "01" } else { "00" });
        assert!(!bad.verify());
        // Round trip through the wire.
        match frame(&format!("[\"EVENT\",\"d\",{}]", ev.to_json())) {
            Frame::Event(sub, got) => {
                assert_eq!(sub, "d");
                assert_eq!(*got, ev);
            }
            f => panic!("{f:?}"),
        }
    }

    #[test]
    fn frames_and_the_room_a_dm_open_names() {
        assert_eq!(frame(r#"["AUTH","abc"]"#), Frame::Auth("abc".into()));
        assert_eq!(
            frame(
                r#"["OK","id",true,"response:{\"channel_id\":\"0F8FAD5B-D9CB-469F-A165-70867728950E\"}"]"#
            ),
            Frame::Ok(
                "id".into(),
                true,
                "response:{\"channel_id\":\"0F8FAD5B-D9CB-469F-A165-70867728950E\"}".into()
            )
        );
        assert_eq!(
            dm_room_of(r#"response:{"channel_id":"0F8FAD5B-D9CB-469F-A165-70867728950E"}"#)
                .as_deref(),
            Some("0f8fad5b-d9cb-469f-a165-70867728950e")
        );
        assert_eq!(dm_room_of(r#"response:{"channel_id":"general"}"#), None);
        assert_eq!(dm_room_of("duplicate:"), None);
        assert_eq!(frame("not json"), Frame::Other);
        assert_eq!(frame(r#"["EVENT","s",{"id":1}]"#), Frame::Other);
    }

    #[test]
    fn only_a_two_person_room_with_me_in_it_is_a_whisper_room() {
        let v = Voice::from_wallet_signature(DERIVED).unwrap();
        let me = v.pubkey_hex();
        let them = "ab".repeat(32);
        let room = "0f8fad5b-d9cb-469f-a165-70867728950e";
        let members = |people: &[&str]| {
            let mut tags = vec![vec!["d".to_string(), room.to_string()]];
            tags.extend(people.iter().map(|p| vec!["p".to_string(), p.to_string()]));
            v.sign(1, KIND_MEMBERS, tags, String::new())
        };
        let mut rooms = Rooms::default();
        assert!(
            !rooms.take(&members(&[&them, &"cd".repeat(32)]), &me),
            "not mine"
        );
        assert!(rooms.take(&members(&[&them, &me]), &me));
        assert_eq!(rooms.with(&them), Some(room));
        assert_eq!(rooms.other(room), Some(them.as_str()));
        assert!(
            !rooms.take(&members(&[&me, &them]), &me),
            "the same room again"
        );
        assert!(
            rooms.take(&members(&[&me, &them, &"cd".repeat(32)]), &me),
            "it grew"
        );
        assert_eq!(rooms.with(&them), None);
    }

    #[test]
    fn a_whisper_line_is_one_tidy_line() {
        assert_eq!(tidy_line("  hi\nthere\x1b[2J  "), "hi there [2J");
        assert_eq!(tidy_line(&"x".repeat(1000)).chars().count(), MAX_LINE_CHARS);
    }
}
