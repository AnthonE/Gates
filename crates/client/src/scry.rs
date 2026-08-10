//! Reaching the scry launcher — identity and signatures, with no key in this
//! process and no crate added to the tree.
//!
//! `scry_overlay.rs` beside this file is **vendored byte-for-byte** from the
//! scry repo, `sdk/rust/scry_overlay.rs` (first vendored 2026-08-05). It is
//! `std`-only by design, so vendoring costs nothing and adds no dependency —
//! which is the whole reason that file was written the way it was.
//!
//! The pin is [`VENDORED_SHA256`], not a commit id: a hash identifies the
//! bytes we actually have, and a commit id in a comment goes stale the moment
//! upstream is re-vendored — as it did within an hour of this file being
//! written, twice, because vendoring surfaced two defects in the source (a doc
//! example that never compiled, and a file that was not rustfmt-clean and so
//! could not pass a vendoring project's fmt gate). Both were fixed upstream and
//! re-vendored, which is the loop this comment is describing.
//!
//! **Do not edit the vendored file.** [`VENDORED_SHA256`] pins it and
//! `vendored_sdk_has_not_been_edited_here` goes red if anyone does. A patch
//! applied here would fix Gates and leave every other game on the broken
//! version — fix it upstream and re-vendor.
//!
//! ⚠ **The pin catches a local edit and CANNOT catch upstream moving**, and
//! confusing the two cost this repo a real capability. Both sides stayed green
//! for days while the vendored copy sat 326 lines behind the source: no
//! Windows transport (the file `use`d `std::os::unix::net::UnixStream`
//! unconditionally, so a Windows build of this client could not compile at
//! all), no `prove`, no `profile`. Upstream's own gate — scry's
//! `crates/scry-broker/tests/sdk_parity.rs` — compiles `sdk/rust/…` and calls
//! it *"what Gates has compiled into its binary byte-for-byte"*, which was a
//! claim about a file in another repo that nothing checked. So the drift check
//! is a **command somebody runs**, not a gate either repo owns, and it is one
//! line against a number scry publishes beside the SDK:
//!
//! ```text
//! sha256sum crates/client/src/scry_overlay.rs
//! # must appear in sdk/SHA256SUMS in AnthonE/scry — if it does not, upstream
//! # has moved: re-vendor, re-pin, and run the client's tests.
//! ```
//!
//! That file did not exist until this re-vendor asked for it, and neither did
//! the two upstream checks beside it (`sdk/test_sdk.py` §what a vendoring game
//! checks against): the SDK is now gated rustfmt-clean, because an unformatted
//! upstream reddens THIS repo's `cargo fmt --all --check` and upstream has no
//! fmt gate of its own — measured, both times it happened.
//!
//! Re-vendored 2026-08-09 and the pin moved with it. That re-vendor was a
//! drop-in: this wrapper calls `connect`, `signer`, `servers_url`, `address`
//! and `sign`, none of which changed shape. `Overlay::title` DID change
//! (`Option<Value>` → `Option<String>`) and nothing here calls it, which is
//! luck rather than design — check the call sites, not just the compile, on
//! the next one.
//!
//! ## What the launcher is for, and what it is not
//!
//! It answers *who is playing* and it produces *signatures*. It is not a login,
//! it is not required, and it holds nothing of ours. Three rules, each of which
//! the SDK's own docs state and this module obeys:
//!
//! - **No launcher is a NORMAL state.** [`Player::discover`] returns
//!   [`Player::Anonymous`] and the game plays. There is no third branch where
//!   we ask for a private key, and there must never be one.
//! - **An address is a CLAIM, not authentication.** The launcher reports the
//!   address the player asked it to watch; anything can say a number. When it
//!   starts mattering — a raid ledger, an item, a settled round — the wire has
//!   to carry a signature over something the shard chose, and the shard has to
//!   recover the signer. Nothing here is that, and nothing here pretends to be.
//! - **This never blocks the frame.** Discovery happens once, before the window
//!   opens. `Overlay::sign` waits on a human at a consent prompt (the SDK's
//!   read timeout is five minutes), so it may never be called from a system.
//!
//! ## What it does not do yet, said plainly
//!
//! `PROTO_VER`'s `Hello` carries a version and nothing else, so the shard does
//! not learn the player's address and does not check one. Wiring identity into
//! the handshake is a wire change — a version bump and every golden regenerated
//! in the same commit (`CLAUDE.md` wall 6) — and it is not this slice. What
//! lands here is the client knowing who it is, from the launcher when one is
//! running and from `--identity` when the player says so, which is the half
//! that has to exist first either way.
//!
//! When that slice IS built, the verb to reach for is `Overlay::prove(server,
//! nonce)`, not [`sign_siwe`]. `sign_siwe` hands the launcher a string this
//! process composed; `prove` binds the signature to a server name and a nonce
//! the SHARD chose, which is the difference between a signature a shard can
//! verify and one it can only replay. The SDK's own doc on `Proof::message`
//! says the rest: the echoed message is for logging, and a server MUST
//! recompute what it verifies against. `prove` arrived with the 2026-08-09
//! re-vendor and has no call site here yet.

#[path = "scry_overlay.rs"]
pub mod overlay;

pub use overlay::{play_message, Overlay, SignError, Signature};

/// This game's slug in the scry catalog — the key its manifest, its depot and
/// its shard list are all filed under.
pub const SLUG: &str = "gates";

/// sha256 of the vendored `scry_overlay.rs`, byte-for-byte as it ships in the
/// scry repo. Regenerate ONLY when re-vendoring an upstream change:
/// `sha256sum crates/client/src/scry_overlay.rs`.
pub const VENDORED_SHA256: &str =
    "3a81c7020ef166702d34f4dd5708e1782df0adc75b09d7aa679915e2deab93cd";

/// Who is playing, and how we came to believe it. The variants are kept
/// distinct because they carry different weight and a single `Option<String>`
/// would flatten that: an address the launcher reported and an address typed on
/// the command line are equally unverified, but only one of them means a
/// launcher is running and can be asked for a signature later.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Player {
    /// No launcher, no `--identity`. The normal state, and a playable one.
    Anonymous,
    /// The player named an address on the command line — `{wallet}` from a
    /// depot's launch args, or typed by hand. No launcher was reached.
    Declared(String),
    /// A launcher answered. `address` is still a claim; `signer` is which
    /// backend it would use if we asked (`browser` · `local` · `arca` ·
    /// `external` · `none`), and it is worth knowing before asking.
    Launcher {
        address: Option<String>,
        signer: String,
    },
}

impl Player {
    /// The address, whatever its provenance. Never treat it as authentication.
    pub fn address(&self) -> Option<&str> {
        match self {
            Player::Anonymous => None,
            Player::Declared(a) => Some(a),
            Player::Launcher { address, .. } => address.as_deref(),
        }
    }

    /// One line for a boot log or a HUD corner. Says how the address was
    /// learned, because "signed in" and "typed a number" must not look alike.
    pub fn line(&self) -> String {
        match self {
            Player::Anonymous => "playing anonymously — no scry launcher".into(),
            Player::Declared(a) => format!("identity {a} (declared, unverified)"),
            Player::Launcher {
                address: Some(a),
                signer,
            } => format!("identity {a} via the scry launcher (signer: {signer}, unverified)"),
            Player::Launcher {
                address: None,
                signer,
            } => format!("scry launcher connected, no address set (signer: {signer})"),
        }
    }
}

/// The connected launcher, if one answered. Held so a later slice can ask for
/// a signature; dropped with the game.
pub struct Scry {
    pub player: Player,
    pub servers_url: Option<String>,
    overlay: Option<Overlay>,
}

impl Scry {
    /// Ask the launcher who is playing. Runs ONCE, before the window opens.
    ///
    /// `declared` is `--identity` if the player passed one. It wins over the
    /// launcher's answer, because a player who names an address on the command
    /// line has said something more specific than their launcher's default —
    /// and both are unverified, so nothing is being trusted more.
    pub fn discover(declared: Option<&str>, version: &str) -> Scry {
        match Overlay::connect(SLUG, version) {
            Ok(mut ov) => {
                let signer = ov.signer();
                let servers_url = ov.servers_url(SLUG);
                let address = declared.map(str::to_string).or_else(|| ov.address());
                Scry {
                    player: Player::Launcher { address, signer },
                    servers_url,
                    overlay: Some(ov),
                }
            }
            // Not an error path. `why` is logged at most once and the game
            // proceeds — a launcher that is not running is the majority case
            // and must never read as a fault.
            Err(_why) => Scry {
                player: match declared {
                    Some(a) => Player::Declared(a.to_string()),
                    None => Player::Anonymous,
                },
                servers_url: None,
                overlay: None,
            },
        }
    }

    /// Ask the player's own signer for a signature over `text`.
    ///
    /// ⚠ **Never call this from a frame system.** It blocks until a human
    /// answers a prompt in the launcher's window. Unused by the current
    /// slices; it is here because the seam is the point of vendoring the SDK,
    /// and an unused seam that compiles is cheaper than one that does not.
    pub fn sign(&mut self, text: &str, why: &str) -> Result<Signature, SignError> {
        match self.overlay.as_mut() {
            Some(ov) => ov.sign(text, why),
            None => Err(SignError::NoLauncher("no scry launcher is running".into())),
        }
    }

    pub fn connected(&self) -> bool {
        self.overlay.is_some()
    }

    /// Where this title's manifest lives — the document naming its news,
    /// store and workshop pages (`crate::manifest`).
    ///
    /// ⚠ **A URL, not the document.** The launcher does not proxy a game's
    /// own files; `Overlay::title` says so in full, and the same rule already
    /// governs `servers_url`. The caller fetches it, capped, off the frame.
    ///
    /// ⚠ **Not from a frame system.** One round trip over a local socket is
    /// cheap — the SDK calls `overlay()` cheap enough to poll every frame —
    /// but this runs beside the boot handshake on a thread anyway, and the
    /// habit is worth more than the microseconds.
    pub fn title_url(&mut self) -> Option<String> {
        self.overlay.as_mut()?.title(SLUG)
    }

    /// Ask the launcher to open a page in the player's browser.
    ///
    /// **This is how NEWS, ITEM STORE and WORKSHOP work, and the handoff is
    /// the point rather than a shortcut.** A game draws its own pixels, so it
    /// can draw a convincing fake of any dialog — the SDK's warning on
    /// `Overlay::overlay`, in those words. A checkout drawn by a game teaches
    /// players that a checkout drawn by a game is normal, and the next one
    /// they see will be a forgery. So the money door is the launcher's.
    ///
    /// Returns whether the launcher accepted it. `false` covers both "no
    /// launcher" and "it declined", and the caller says so rather than
    /// pretending the click worked — a button that silently does nothing is
    /// the worst of the three outcomes.
    ///
    /// The URL is checked against `manifest::check_url` **by the caller**,
    /// before it ever reaches here, because that is where a bad one is worth
    /// reporting; this is the last mile and the launcher has its own opinion
    /// too.
    pub fn open(&mut self, url: &str) -> bool {
        match self.overlay.as_mut() {
            Some(ov) => ov.open_url(url),
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The drift gate. The vendored file is the scry SDK's, byte-for-byte, and
    /// a local edit is the failure this catches: it would fix Gates and leave
    /// every other game on the broken copy.
    #[test]
    fn vendored_sdk_has_not_been_edited_here() {
        // A tiny sha256 rather than a dependency — `sha2` is not in this
        // crate's tree and this test is not a reason to put it there.
        let bytes = include_bytes!("scry_overlay.rs");
        assert_eq!(
            sha256_hex(bytes),
            VENDORED_SHA256,
            "crates/client/src/scry_overlay.rs has been edited. It is VENDORED from \
             AnthonE/scry sdk/rust/scry_overlay.rs — fix it there and re-vendor, or \
             every other game keeps the broken version. If this IS a re-vendor, \
             update scry::VENDORED_SHA256 in the same commit."
        );
    }

    #[test]
    fn no_launcher_is_a_normal_state_and_yields_a_playable_game() {
        // The socket env is pointed at nothing, which is what a machine with
        // no launcher looks like.
        std::env::set_var("SCRY_LAUNCHER_SOCKET", "/nonexistent/scry-gates-test.sock");
        let s = Scry::discover(None, "0.1.0");
        assert_eq!(s.player, Player::Anonymous);
        assert!(!s.connected());
        assert!(s.player.address().is_none());
        assert!(s.player.line().contains("anonymously"));

        // ...and --identity still works with no launcher at all.
        let s = Scry::discover(Some("0xabc"), "0.1.0");
        assert_eq!(s.player, Player::Declared("0xabc".into()));
        assert_eq!(s.player.address(), Some("0xabc"));
        assert!(s.player.line().contains("unverified"));
    }

    #[test]
    fn a_launcher_address_never_reads_as_verified() {
        // Every rendering of an address says so. The wire carries no identity
        // yet, so a line that read "signed in as" would be a claim the whole
        // stack cannot back.
        for p in [
            Player::Declared("0x1".into()),
            Player::Launcher {
                address: Some("0x1".into()),
                signer: "browser".into(),
            },
        ] {
            assert!(p.line().contains("unverified"), "{}", p.line());
        }
    }

    fn sha256_hex(data: &[u8]) -> String {
        const K: [u32; 64] = [
            0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
            0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
            0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
            0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
            0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
            0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
            0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
            0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
            0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
            0xc67178f2,
        ];
        let mut h: [u32; 8] = [
            0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
            0x5be0cd19,
        ];
        let mut msg = data.to_vec();
        let bitlen = (data.len() as u64) * 8;
        msg.push(0x80);
        while msg.len() % 64 != 56 {
            msg.push(0);
        }
        msg.extend_from_slice(&bitlen.to_be_bytes());
        for chunk in msg.chunks(64) {
            let mut w = [0u32; 64];
            for i in 0..16 {
                w[i] = u32::from_be_bytes([
                    chunk[i * 4],
                    chunk[i * 4 + 1],
                    chunk[i * 4 + 2],
                    chunk[i * 4 + 3],
                ]);
            }
            for i in 16..64 {
                let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
                let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
                w[i] = w[i - 16]
                    .wrapping_add(s0)
                    .wrapping_add(w[i - 7])
                    .wrapping_add(s1);
            }
            let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh) =
                (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]);
            for i in 0..64 {
                let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
                let ch = (e & f) ^ ((!e) & g);
                let t1 = hh
                    .wrapping_add(s1)
                    .wrapping_add(ch)
                    .wrapping_add(K[i])
                    .wrapping_add(w[i]);
                let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
                let maj = (a & b) ^ (a & c) ^ (b & c);
                let t2 = s0.wrapping_add(maj);
                hh = g;
                g = f;
                f = e;
                e = d.wrapping_add(t1);
                d = c;
                c = b;
                b = a;
                a = t1.wrapping_add(t2);
            }
            for (i, v) in [a, b, c, d, e, f, g, hh].iter().enumerate() {
                h[i] = h[i].wrapping_add(*v);
            }
        }
        h.iter().map(|w| format!("{w:08x}")).collect()
    }
}

/// Sign a shard's SIWE challenge through the launcher, for
/// [`crate::Session::connect`].
///
/// `None` is **not an error** and is the common case: no launcher running,
/// the player declined the consent prompt, or a handoff was needed. All
/// three mean "connect as a guest", which a shard that takes guests will
/// do. A shard that requires identity refuses, and that refusal is the
/// player learning their launcher is not running — which is the right place
/// to learn it.
///
/// The signature comes back as `0x…` hex and has to be exactly 65 bytes:
/// r ‖ s ‖ v. A short or long one is refused rather than padded, because a
/// padded signature recovers a *different* address, and the failure would
/// present as "the shard says I am somebody else".
pub fn sign_siwe(text: &str) -> Option<protocol::Signature> {
    let mut scry = Scry::discover(None, env!("CARGO_PKG_VERSION"));
    let sig = scry.sign(text, "sign in to this Gates shard").ok()?;
    parse_signature(&sig.signature)
}

/// `0x…` hex → 65 bytes. Refuses anything else.
pub(crate) fn parse_signature(hex: &str) -> Option<protocol::Signature> {
    let body = hex.strip_prefix("0x").or_else(|| hex.strip_prefix("0X"))?;
    if body.len() != protocol::SIGNATURE_BYTES * 2 {
        return None;
    }
    let nib = |c: u8| -> Option<u8> {
        match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            b'A'..=b'F' => Some(c - b'A' + 10),
            _ => None,
        }
    };
    let b = body.as_bytes();
    let mut out = [0u8; protocol::SIGNATURE_BYTES];
    for (i, slot) in out.iter_mut().enumerate() {
        *slot = (nib(b[i * 2])? << 4) | nib(b[i * 2 + 1])?;
    }
    Some(protocol::Signature(out))
}

#[cfg(test)]
mod siwe_tests {
    use super::*;

    #[test]
    fn a_well_formed_signature_parses() {
        let hex = format!("0x{}", "ab".repeat(protocol::SIGNATURE_BYTES));
        let sig = parse_signature(&hex).expect("65 bytes of hex parses");
        assert_eq!(sig.0, [0xabu8; protocol::SIGNATURE_BYTES]);
    }

    /// Every malformed shape is refused rather than padded. A padded
    /// signature recovers a different address, so the player would be told
    /// they are somebody else instead of being told the launcher misbehaved.
    #[test]
    fn a_malformed_signature_is_refused_never_padded() {
        assert!(parse_signature("").is_none(), "empty");
        assert!(parse_signature("0x").is_none(), "prefix only");
        assert!(
            parse_signature(&"ab".repeat(protocol::SIGNATURE_BYTES)).is_none(),
            "no 0x prefix"
        );
        assert!(
            parse_signature(&format!("0x{}", "ab".repeat(protocol::SIGNATURE_BYTES - 1))).is_none(),
            "one byte short"
        );
        assert!(
            parse_signature(&format!("0x{}", "zz".repeat(protocol::SIGNATURE_BYTES))).is_none(),
            "not hex"
        );
    }
}
