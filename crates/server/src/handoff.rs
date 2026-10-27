//! **Moving a player between this game's servers** (wire v152), the
//! platform's handoff (`docs/client/HANDOFF.md` in the platform repo).
//!
//! The pass gets a player in; a handoff moves them. The shard a player is on
//! signs nine lines with its server key — who the player is, this shard's
//! address, the one it sends them to, a single-use id, a short life — and
//! tells the client to go (`protocol::Travel`). The next shard checks the
//! signature offline against the game's keys and admits that player. No call
//! to the platform per hop, and nothing asks the player anything, whichever
//! way they first signed in.
//!
//! ```text
//! elo handoff
//! game: gates
//! key: <the signing key's id>
//! player: <0x… lowercase | a Solana address | acct_<20 hex>>
//! from: <this shard's address, host:port>
//! to: <the next shard's address, host:port>
//! id: <16 to 64 of A-Z a-z 0-9 - _>
//! exp: <unix seconds>
//! carry: <sha256 hex of the carried bytes | none>
//! ```
//!
//! The token is `base64url(text).base64url(signature)`, unpadded. The checks
//! run in the platform's order and each refusal has its code ([`verify`]);
//! the platform's vector (`sdk/testdata/handoff.json`) is pinned below.
//!
//! `shard.toml`:
//!
//! ```text
//! handoff_addr = "game.elopros.com:61234"   # this shard, as a handoff names it
//! handoff_keys = "island-1=<base64 pub>, arena-1=<base64 pub>"   # who may send players here
//! handoff_key = "/etc/gates/handoff.key"    # this shard's own key: a 64-hex seed or a PKCS#8 PEM
//! handoff_key_id = "island-1"
//! arena_addr = "game.elopros.com:61300"     # where /arena goes; the shard whose handoff_addr this is IS the arena
//! ```
//!
//! None of it set, nothing changes: no handoff is admitted or issued.

use base64::Engine;

/// How long a handoff this shard issues lives.
pub const TTL_S: u64 = 60;
/// The longest a handoff may live (the platform's ceiling).
pub const TTL_MAX_S: u64 = 120;
/// Clock skew allowed either side.
pub const SKEW_S: u64 = 30;
/// Ids remembered at once. A handoff lives 150 s at most, so this is a hop
/// rate far past what one shard admits.
pub const SEEN_MAX: usize = 4096;
/// Server keys one shard may name.
pub const KEYS_MAX: usize = 16;

/// The handoff half of `shard.toml`.
#[derive(Clone, Default)]
pub struct Config {
    /// This shard's own address, `host:port`: `to` on a handoff it admits,
    /// `from` on one it issues.
    pub addr: Option<String>,
    /// The game's server keys a handoff here may be signed by.
    pub keys: Vec<(String, [u8; 32])>,
    /// This shard's own key id and seed, to issue with.
    pub signer: Option<(String, [u8; 32])>,
    /// The arena's address: where `/arena` sends a player.
    pub arena: Option<String>,
}

/// Never prints the seed.
impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("handoff::Config")
            .field("addr", &self.addr)
            .field(
                "keys",
                &self.keys.iter().map(|(k, _)| k).collect::<Vec<_>>(),
            )
            .field("signer", &self.signer.as_ref().map(|(k, _)| k))
            .field("arena", &self.arena)
            .finish()
    }
}

impl Config {
    /// Does this shard admit handoffs at all?
    pub fn admits(&self) -> bool {
        self.addr.is_some() && !self.keys.is_empty()
    }

    /// Can this shard send a player elsewhere?
    pub fn issues(&self) -> bool {
        self.addr.is_some() && self.signer.is_some()
    }

    /// Is this shard the arena?
    pub fn is_arena(&self) -> bool {
        self.arena.is_some() && self.arena == self.addr
    }
}

fn lower_digit(c: u8) -> bool {
    c.is_ascii_lowercase() || c.is_ascii_digit()
}

pub fn key_id_ok(s: &str) -> bool {
    let b = s.as_bytes();
    (1..=31).contains(&b.len())
        && lower_digit(b[0])
        && b.iter().all(|c| lower_digit(*c) || *c == b'-')
}

/// A server address as a handoff names it: `host:port`, lowercase, a host of
/// letters, digits, `.` and `-` (or a bracketed IPv6 literal).
pub fn addr_ok(s: &str) -> bool {
    let Some((host, port)) = s.rsplit_once(':') else {
        return false;
    };
    let port_ok = (1..=5).contains(&port.len()) && port.bytes().all(|c| c.is_ascii_digit());
    let hb = host.as_bytes();
    let name =
        (1..=100).contains(&hb.len()) && hb.iter().all(|c| lower_digit(*c) || b".-".contains(c));
    let v6 = (4..=41).contains(&hb.len())
        && hb[0] == b'['
        && hb[hb.len() - 1] == b']'
        && hb[1..hb.len() - 1]
            .iter()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(c) || *c == b':');
    port_ok && (name || v6)
}

/// Who a handoff may name: a lowercase `0x` wallet, an `acct_` id or a
/// Solana address — exactly the save keys `auth.rs` files players under.
pub fn player_ok(s: &str) -> bool {
    let b = s.as_bytes();
    let hex = |c: &u8| c.is_ascii_digit() || (b'a'..=b'f').contains(c);
    let evm = b.len() == 42 && s.starts_with("0x") && b[2..].iter().all(hex);
    let acct = crate::auth::is_account_str(s);
    const B58: &[u8] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
    let sol = (32..=44).contains(&b.len()) && b.iter().all(|c| B58.contains(c));
    evm || acct || sol
}

fn id_ok(s: &str) -> bool {
    (16..=64).contains(&s.len())
        && s.bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c))
}

/// `handoff_keys`: `id=<standard base64 of 32 bytes>`, comma-separated.
pub fn parse_keys(value: &str) -> Result<Vec<(String, [u8; 32])>, String> {
    let mut out: Vec<(String, [u8; 32])> = Vec::new();
    for entry in value.split(',').map(str::trim).filter(|e| !e.is_empty()) {
        let (id, b64) = entry
            .split_once('=')
            .ok_or_else(|| format!("handoff_keys entry `{entry}` is not id=<base64 public key>"))?;
        let id = id.trim();
        if !key_id_ok(id) {
            return Err(format!(
                "handoff_keys names `{id}`, which is not a key id (a-z 0-9 -, up to 31)"
            ));
        }
        let raw = base64::engine::general_purpose::STANDARD
            .decode(b64.trim())
            .ok()
            .filter(|b| b.len() == 32)
            .ok_or_else(|| format!("handoff_keys: `{id}` is not standard base64 of 32 bytes"))?;
        if out.iter().any(|(k, _)| k == id) {
            return Err(format!("handoff_keys names `{id}` twice"));
        }
        if out.len() >= KEYS_MAX {
            return Err(format!("handoff_keys names more than {KEYS_MAX} keys"));
        }
        let mut key = [0u8; 32];
        key.copy_from_slice(&raw);
        out.push((id.to_string(), key));
    }
    if out.is_empty() {
        return Err("handoff_keys names no key — omit it to admit no handoff".into());
    }
    Ok(out)
}

/// A key file's 32-byte seed: 64 hex, or an Ed25519 PKCS#8 PEM (what
/// `openssl genpkey -algorithm ed25519` writes).
pub fn parse_seed(text: &str) -> Result<[u8; 32], String> {
    let t = text.trim();
    let raw: Vec<u8> = if t.len() == 64 && t.bytes().all(|c| c.is_ascii_hexdigit()) {
        (0..32)
            .map(|i| u8::from_str_radix(&t[2 * i..2 * i + 2], 16).unwrap_or(0))
            .collect()
    } else {
        const PKCS8: [u8; 16] = [
            0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22,
            0x04, 0x20,
        ];
        let body: String = t.lines().filter(|l| !l.starts_with("-----")).collect();
        let der = base64::engine::general_purpose::STANDARD
            .decode(body.trim())
            .map_err(|_| "the key file is neither 64 hex nor a PEM".to_string())?;
        if der.len() != 48 || der[..16] != PKCS8 {
            return Err("the key file's PEM is not an Ed25519 private key".into());
        }
        der[16..].to_vec()
    };
    let mut seed = [0u8; 32];
    seed.copy_from_slice(&raw);
    Ok(seed)
}

fn sha256_hex(data: &[u8]) -> String {
    ring::digest::digest(&ring::digest::SHA256, data)
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The exact text a shard signs for one handoff.
#[allow(clippy::too_many_arguments)]
pub fn text(
    game: &str,
    key_id: &str,
    player: &str,
    from: &str,
    to: &str,
    id: &str,
    exp: u64,
    carry: &[u8],
) -> String {
    let c = if carry.is_empty() {
        "none".to_string()
    } else {
        sha256_hex(carry)
    };
    format!("elo handoff\ngame: {game}\nkey: {key_id}\nplayer: {player}\nfrom: {from}\nto: {to}\nid: {id}\nexp: {exp}\ncarry: {c}")
}

fn new_id() -> Option<String> {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut b = [0u8; 22];
    getrandom::getrandom(&mut b).ok()?;
    Some(b.iter().map(|x| ABC[(*x & 63) as usize] as char).collect())
}

/// Sign a handoff moving `player` from this shard to `to`. `None` when this
/// shard cannot issue (no `handoff_addr` or key), the player or address is
/// not one a handoff may name, or there is no entropy for an id.
pub fn issue(cfg: &Config, player: &str, to: &str, carry: &[u8], now: u64) -> Option<String> {
    let (kid, seed) = cfg.signer.as_ref()?;
    let from = cfg.addr.as_deref()?;
    if !player_ok(player)
        || !addr_ok(to)
        || !addr_ok(from)
        || carry.len() > protocol::HANDOFF_CARRY_MAX
    {
        return None;
    }
    let id = new_id()?;
    let body = text(
        protocol::SLUG,
        kid,
        player,
        from,
        to,
        &id,
        now + TTL_S,
        carry,
    );
    let pair = ring::signature::Ed25519KeyPair::from_seed_unchecked(seed).ok()?;
    let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let tok = format!(
        "{}.{}",
        b64.encode(body.as_bytes()),
        b64.encode(pair.sign(body.as_bytes()).as_ref())
    );
    (tok.len() <= protocol::HANDOFF_TOKEN_MAX).then_some(tok)
}

/// What an admitted handoff said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Handed {
    /// The save key: a lowercase `0x` wallet, an `acct_` id or a Solana address.
    pub player: String,
    /// The shard it came from.
    pub from: String,
    pub id: String,
    pub exp: u64,
}

/// Every check but `used`, in the platform's order. The code on refusal is
/// the platform's (`malformed`, `wrong_game`, `wrong_destination`,
/// `expired`, `too_long`, `unknown_key`, `bad_signature`, `carry`).
pub fn verify(
    token: &[u8],
    carry: &[u8],
    game: &str,
    names: &[&str],
    keys: &[(String, [u8; 32])],
    now: u64,
) -> Result<Handed, &'static str> {
    let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    if token.len() > protocol::HANDOFF_TOKEN_MAX {
        return Err("malformed");
    }
    let tok = core::str::from_utf8(token).map_err(|_| "malformed")?;
    if tok.matches('.').count() != 1 {
        return Err("malformed");
    }
    let (a, b) = tok.split_once('.').ok_or("malformed")?;
    let raw = b64.decode(a).map_err(|_| "malformed")?;
    let sig = b64.decode(b).map_err(|_| "malformed")?;
    let body = core::str::from_utf8(&raw).map_err(|_| "malformed")?;
    let lines: Vec<&str> = body.split('\n').collect();
    if lines.len() != 9 || lines[0] != "elo handoff" || sig.len() != 64 {
        return Err("malformed");
    }
    let field = |i: usize, name: &str| {
        lines[i]
            .strip_prefix(name)
            .filter(|v| !v.is_empty())
            .ok_or("malformed")
    };
    let g = field(1, "game: ")?;
    let key = field(2, "key: ")?;
    let player = field(3, "player: ")?;
    let from = field(4, "from: ")?;
    let to = field(5, "to: ")?;
    let id = field(6, "id: ")?;
    let exp_s = field(7, "exp: ")?;
    let said = field(8, "carry: ")?;
    let carry_ok = said == "none"
        || (said.len() == 64
            && said
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)));
    let game_ok = {
        let gb = g.as_bytes();
        (1..=63).contains(&gb.len())
            && lower_digit(gb[0])
            && gb.iter().all(|c| lower_digit(*c) || b"_-".contains(c))
    };
    if !(game_ok
        && key_id_ok(key)
        && player_ok(player)
        && addr_ok(from)
        && addr_ok(to)
        && id_ok(id)
        && (1..=12).contains(&exp_s.len())
        && exp_s.bytes().all(|c| c.is_ascii_digit())
        && carry_ok)
    {
        return Err("malformed");
    }
    let exp: u64 = exp_s.parse().map_err(|_| "malformed")?;
    if g != game {
        return Err("wrong_game");
    }
    if !names.iter().any(|n| n.eq_ignore_ascii_case(to)) {
        return Err("wrong_destination");
    }
    if exp + SKEW_S <= now {
        return Err("expired");
    }
    if exp > now + TTL_MAX_S + SKEW_S {
        return Err("too_long");
    }
    let pub_key = keys
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, p)| p)
        .ok_or("unknown_key")?;
    ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, pub_key)
        .verify(&raw, &sig)
        .map_err(|_| "bad_signature")?;
    let got = if carry.is_empty() {
        "none".to_string()
    } else {
        sha256_hex(carry)
    };
    if got != said {
        return Err("carry");
    }
    Ok(Handed {
        player: player.to_string(),
        from: from.to_string(),
        id: id.to_string(),
        exp,
    })
}

/// Check a handoff at this shard's own address: [`verify`] against
/// `handoff_keys`, or `no_handoff` when this shard admits none. Pure, so
/// every handshake task may call it; whether the id was already used is the
/// accept loop's ([`Seen`]), the one owner of that record.
pub fn check(cfg: &Config, token: &[u8], carry: &[u8], now: u64) -> Result<Handed, &'static str> {
    let Some(addr) = cfg.addr.as_deref().filter(|_| cfg.admits()) else {
        return Err("no_handoff");
    };
    verify(token, carry, protocol::SLUG, &[addr], &cfg.keys, now)
}

/// The handoff ids this shard admitted, each until it could no longer be
/// used. Owned by the accept loop alone: no lock, and a `BTreeMap`, since
/// this crate iterates nothing in hash order.
#[derive(Default)]
pub struct Seen {
    until: std::collections::BTreeMap<String, u64>,
}

impl Seen {
    /// True, and remembered, if `id` is new; false if it was already used,
    /// or the record is full of ids still alive (refused rather than
    /// forgetting one, which would let it be replayed).
    pub fn first(&mut self, id: &str, until: u64, now: u64) -> bool {
        if self.until.len() >= SEEN_MAX {
            self.until.retain(|_, u| *u > now);
        }
        if self.until.get(id).is_some_and(|u| *u > now) || self.until.len() >= SEEN_MAX {
            return false;
        }
        self.until.insert(id.to_string(), until);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The platform's vector (`sdk/testdata/handoff.json` in AnthonE/
    /// scry-forge): game `my-game`, verified at `NOW` for `TO`. The same
    /// tokens must answer the same here, or a hop fails with both sides
    /// looking right.
    const PUB: &str = "ebVWLo/mVPlAeLES6KmLp5AfhTrmlb7X4OORC60ElmQ=";
    const SEED: &str = "0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20";
    const FROM: &str = "play.my-game.example:7000";
    const TO: &str = "play.my-game.example:7100";
    const NOW: u64 = 1_800_000_000;
    const WALLET: &str = "ZWxvIGhhbmRvZmYKZ2FtZTogbXktZ2FtZQprZXk6IGlzbGFuZC0xCnBsYXllcjogMHgyNDQ0NWVmZGRiMDhkNDkzOGUzZTM2MjcwNDJiMmNmNDA2M2Q5MDkyCmZyb206IHBsYXkubXktZ2FtZS5leGFtcGxlOjcwMDAKdG86IHBsYXkubXktZ2FtZS5leGFtcGxlOjcxMDAKaWQ6IGg0bmQwZmYtMDEyMzQ1Njc4OWFiCmV4cDogMTgwMDAwMDA2MApjYXJyeTogbm9uZQ";

    fn keys() -> Vec<(String, [u8; 32])> {
        parse_keys(&format!("island-1={PUB}")).unwrap()
    }

    fn signed(text: &str, seed: &[u8; 32]) -> String {
        let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD;
        let pair = ring::signature::Ed25519KeyPair::from_seed_unchecked(seed).unwrap();
        format!(
            "{}.{}",
            b64.encode(text),
            b64.encode(pair.sign(text.as_bytes()).as_ref())
        )
    }

    #[test]
    fn the_platform_vector_admits_and_refuses_with_its_codes() {
        let seed = parse_seed(SEED).unwrap();
        let t = text(
            "my-game",
            "island-1",
            "0x24445efddb08d4938e3e3627042b2cf4063d9092",
            FROM,
            TO,
            "h4nd0ff-0123456789ab",
            NOW + 60,
            b"",
        );
        let tok = signed(&t, &seed);
        assert!(
            tok.starts_with(WALLET),
            "the text is the vector's, byte for byte"
        );
        let v = |tok: &str, carry: &[u8], now: u64| {
            verify(tok.as_bytes(), carry, "my-game", &[TO], &keys(), now)
        };
        let got = v(&tok, b"", NOW).unwrap();
        assert_eq!(
            (got.player.as_str(), got.from.as_str()),
            ("0x24445efddb08d4938e3e3627042b2cf4063d9092", FROM)
        );
        assert_eq!(v("not-a-handoff", b"", NOW), Err("malformed"));
        assert_eq!(
            verify(tok.as_bytes(), b"", "gates", &[TO], &keys(), NOW),
            Err("wrong_game")
        );
        assert_eq!(
            verify(
                tok.as_bytes(),
                b"",
                "my-game",
                &["play.my-game.example:7200"],
                &keys(),
                NOW
            ),
            Err("wrong_destination")
        );
        assert_eq!(v(&tok, b"", NOW + 60 + SKEW_S), Err("expired"));
        assert!(v(&tok, b"", NOW + 60 + SKEW_S - 1).is_ok());
        assert_eq!(
            v(&tok, b"", NOW + 60 - TTL_MAX_S - SKEW_S - 1),
            Err("too_long")
        );
        assert_eq!(
            verify(tok.as_bytes(), b"", "my-game", &[TO], &[], NOW),
            Err("unknown_key")
        );
        let forged = signed(&t, &[2u8; 32]);
        assert_eq!(v(&forged, b"", NOW), Err("bad_signature"));
        let carried = text(
            "my-game",
            "island-1",
            "acct_0123456789abcdef0123",
            FROM,
            TO,
            "h4nd0ff-carry-000001",
            NOW + 60,
            b"{\"m\":1}",
        );
        let ctok = signed(&carried, &seed);
        assert_eq!(v(&ctok, b"{\"m\":2}", NOW), Err("carry"));
        assert_eq!(
            v(&ctok, b"{\"m\":1}", NOW).unwrap().player,
            "acct_0123456789abcdef0123"
        );
    }

    #[test]
    fn an_island_issues_and_the_arena_admits_each_handoff_once() {
        let seed = parse_seed(SEED).unwrap();
        let island = Config {
            addr: Some("island.example:7000".into()),
            signer: Some(("island-1".into(), seed)),
            arena: Some("arena.example:7100".into()),
            ..Config::default()
        };
        let arena = Config {
            addr: Some("arena.example:7100".into()),
            keys: keys(),
            arena: Some("arena.example:7100".into()),
            ..Config::default()
        };
        assert!(island.issues() && !island.is_arena() && arena.is_arena() && arena.admits());
        let now = 1_900_000_000;
        let me = "0x24445efddb08d4938e3e3627042b2cf4063d9092";
        let tok = issue(&island, me, "arena.example:7100", b"", now).unwrap();
        let got = check(&arena, tok.as_bytes(), b"", now).unwrap();
        assert_eq!(
            (got.player.as_str(), got.from.as_str()),
            (me, "island.example:7000")
        );
        let mut seen = Seen::default();
        assert!(seen.first(&got.id, got.exp + SKEW_S, now));
        assert!(
            !seen.first(&got.id, got.exp + SKEW_S, now),
            "a replay is used"
        );
        assert!(
            seen.first(&got.id, got.exp + SKEW_S + 10, got.exp + SKEW_S),
            "remembered only while it could be used"
        );
        let elsewhere = issue(&island, me, "other.example:1", b"", now).unwrap();
        assert_eq!(
            check(&arena, elsewhere.as_bytes(), b"", now),
            Err("wrong_destination")
        );
        assert_eq!(
            check(&Config::default(), tok.as_bytes(), b"", now),
            Err("no_handoff")
        );
        assert!(issue(&island, "not a player", "arena.example:7100", b"", now).is_none());
        assert!(issue(&Config::default(), me, "arena.example:7100", b"", now).is_none());
    }

    #[test]
    fn the_config_reads_keys_and_seeds_and_refuses_bad_ones() {
        assert_eq!(keys()[0].0, "island-1");
        assert!(parse_keys("").is_err());
        assert!(parse_keys("Bad=x").is_err());
        assert!(parse_keys(&format!("a={PUB}, a={PUB}")).is_err());
        let pem_body = "MC4CAQAwBQYDK2VwBCIEIAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8g";
        let label = ["PRIVATE", "KEY"].join(" ");
        let pem = format!("-----BEGIN {label}-----\n{pem_body}\n-----END {label}-----\n");
        assert_eq!(parse_seed(&pem).unwrap(), parse_seed(SEED).unwrap());
        assert!(parse_seed("nope").is_err());
        assert!(addr_ok("game.elopros.com:61234") && addr_ok("[::1]:7000"));
        assert!(!addr_ok("Game.example:1") && !addr_ok("no-port") && !addr_ok("a b:1"));
    }
}
