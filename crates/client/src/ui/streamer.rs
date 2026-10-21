//! **Streamer mode** — Rust's, for a player who plays on a stream. Nothing a
//! viewer could use to find them, follow them onto a server or name the
//! people they play with is drawn:
//!
//! - every player, you included, is a made-up name ([`alias`]) that holds
//!   for the session and changes with the next, with no picture;
//! - no wallet, no server name or address on the connect, loading,
//!   disconnect and Esc screens, no invite link, no map seed;
//! - Discord gets no server, and a door code is dots as it is typed.
//!
//! What is saved stays real (the contacts book, chat as it arrives), so
//! turning it off brings everything back. Settings → SOCIAL turns it on, and
//! a launcher in streamer mode starts the game with it on (`ELO_STREAMER=1`).
//!
//! One switch for the whole client, written by the settings system each
//! frame ([`set`]) and read by whatever draws ([`on`]). Each helper also has
//! a form that takes the switch as an argument, so a test never flips it.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

static ON: AtomicBool = AtomicBool::new(false);
static SALT: AtomicU64 = AtomicU64::new(0);

/// What a hidden value is drawn as.
pub const HIDDEN: &str = "HIDDEN";

/// The variable a launcher in streamer mode starts the game with.
pub const ENV: &str = "ELO_STREAMER";

pub fn on() -> bool {
    ON.load(Ordering::Relaxed)
}

/// Written by the settings system (`render/settings.rs`), never by a test.
pub fn set(v: bool) {
    ON.store(v, Ordering::Relaxed);
}

/// The launcher asked for it (`ELO_STREAMER=1`). Never on the web, where
/// there is no launcher and no environment.
pub fn forced_by_launcher() -> bool {
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::env::var(ENV).map(|v| v.trim() == "1").unwrap_or(false)
    }
    #[cfg(target_arch = "wasm32")]
    {
        false
    }
}

/// This session's salt: the same player is the same alias all session, and
/// nobody can work an alias back to a wallet, because the salt is never
/// shown and the next session's differs.
fn salt() -> u64 {
    let s = SALT.load(Ordering::Relaxed);
    if s != 0 {
        return s;
    }
    #[cfg(not(target_arch = "wasm32"))]
    let fresh = {
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        t ^ (u64::from(std::process::id()) << 32) ^ (&SALT as *const _ as u64)
    };
    #[cfg(target_arch = "wasm32")]
    let fresh =
        (js_sys::Math::random() * 9_007_199_254_740_992.0) as u64 ^ js_sys::Date::now() as u64;
    let fresh = fresh | 1;
    let _ = SALT.compare_exchange(0, fresh, Ordering::Relaxed, Ordering::Relaxed);
    SALT.load(Ordering::Relaxed)
}

const FIRST: [&str; 32] = [
    "Amber", "Ash", "Birch", "Bramble", "Brass", "Cedar", "Cinder", "Clay", "Copper", "Dusk",
    "Ember", "Fern", "Flint", "Frost", "Gale", "Granite", "Hazel", "Iron", "Juniper", "Lichen",
    "Moss", "Oak", "Pine", "Quartz", "Reed", "Rust", "Sable", "Slate", "Sorrel", "Thorn", "Tide",
    "Willow",
];
const SECOND: [&str; 32] = [
    "Badger", "Bear", "Boar", "Crane", "Crow", "Deer", "Eagle", "Ferret", "Finch", "Fox", "Frog",
    "Gull", "Hare", "Hawk", "Heron", "Jay", "Lark", "Lynx", "Marten", "Mole", "Moth", "Newt",
    "Otter", "Owl", "Pike", "Raven", "Seal", "Stoat", "Toad", "Viper", "Wolf", "Wren",
];

/// The made-up name for `key` (a wallet, or `#id` for a guest), with this
/// session's salt: two words and a number, like `Moss Heron 42`.
pub fn alias(key: &str) -> String {
    alias_with(salt(), key)
}

/// [`alias`] with the salt given, for a test.
pub fn alias_with(salt: u64, key: &str) -> String {
    // FNV-1a over the salt and the key: no dependency, and good enough to
    // spread 2^16 names. Not a secret on its own; the salt is.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325 ^ salt;
    for b in salt.to_le_bytes().iter().chain(key.as_bytes()) {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    let a = FIRST[(h & 31) as usize];
    let b = SECOND[((h >> 5) & 31) as usize];
    format!("{a} {b} {}", (h >> 10) % 100)
}

/// `text` as streamer mode draws it: [`HIDDEN`] when on, else itself.
pub fn hide(text: &str) -> String {
    hide_if(on(), text)
}

pub fn hide_if(on: bool, text: &str) -> String {
    if on {
        HIDDEN.to_string()
    } else {
        text.to_string()
    }
}

/// A server's address or name in a sentence: `the server` when on.
pub fn server(addr: &str) -> String {
    server_if(on(), addr)
}

pub fn server_if(on: bool, addr: &str) -> String {
    if on {
        "the server".to_string()
    } else {
        addr.to_string()
    }
}

/// A door code as it is typed: dots when on.
pub fn digits(code: &str) -> String {
    digits_if(on(), code)
}

pub fn digits_if(on: bool, code: &str) -> String {
    if on {
        "•".repeat(code.chars().count())
    } else {
        code.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_alias_holds_for_a_session_and_moves_with_the_salt() {
        let a = alias_with(7, "0xabc");
        assert_eq!(
            a,
            alias_with(7, "0xabc"),
            "the same player is the same name all session"
        );
        assert_ne!(a, alias_with(7, "0xabd"), "two players are two names");
        assert_ne!(
            a,
            alias_with(8, "0xabc"),
            "the next session's name is another"
        );
        assert!(a.split(' ').count() == 3 && a.is_ascii(), "{a}");
    }

    #[test]
    fn hidden_things_are_hidden_only_when_on() {
        assert_eq!(hide_if(true, "1.2.3.4:4433"), HIDDEN);
        assert_eq!(hide_if(false, "1.2.3.4:4433"), "1.2.3.4:4433");
        assert_eq!(digits_if(true, "1234"), "••••");
        assert_eq!(digits_if(false, "12"), "12");
    }
}
