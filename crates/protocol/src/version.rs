//! Build identity: what this binary is, in the three forms anybody needs it.
//!
//! There are **three** version numbers in this game and conflating any two of
//! them is the bug this module exists to prevent:
//!
//! | number | what it describes | who gates on it |
//! |---|---|---|
//! | [`crate::PROTO_VER`] | the packet layout | the shard, exactly — a mismatch is `REFUSE_VERSION` |
//! | [`VER`] | the *release* a player downloaded | the shard, as a **minimum** — below it is `REFUSE_BUILD` |
//! | [`GIT_SHA`] | the commit it was built from | nobody — it is displayed and logged |
//!
//! The middle one is the one this file is really about. `PROTO_VER` already
//! answers "can these two parse each other's bytes", and it is deliberately
//! stingy: it moves only on a layout change, so two builds a month apart can
//! share it. What it cannot answer is "is this client old enough to be
//! wrong about something that is not a byte" — a prediction rule that
//! changed, a refusal that grew a case, a number that moved in `content/`.
//! For that the shard needs to know which *release* it is talking to, and a
//! release is what a player downloads and can be told to update.
//!
//! ## Why a minimum and not an equality
//!
//! An equality gate on the build would be the strongest correctness claim and
//! the wrong product: every commit, including a docs commit, changes the sha,
//! so every shard restart would lock out every installed client until each
//! one re-downloaded. The reference game does force updates like that, but it
//! ties them to its *protocol* number, which is exactly what `PROTO_VER`
//! already is here. So the layering above is that design, spelled: the wire
//! is exact, the release is a floor, and the commit is evidence.
//!
//! ## The packing, and why it is decimal
//!
//! [`VER`] is `major × 1_000_000 + minor × 1_000 + patch`. Two properties pay
//! for the odd-looking base: comparing the `u32` compares the semver (so the
//! shard's gate is one `<`), and the number is **legible in a log** — `1000`
//! reads back as 0.1.0 without a decoder ring, which matters because this
//! value's other job is to appear in a line somebody is reading at 2am. Each
//! component is capped at 999 and the cap is a compile error, not a wrap.
//!
//! A prerelease suffix (`-alpha.1`) is **not** in the packed number: semver
//! orders a prerelease *below* its release and a decimal packing cannot say
//! that, so a build carrying one compares equal to the release it precedes.
//! It still shows up in [`BUILD_ID`], which is what a person reads. Nothing
//! ships with one today; if that changes, the honest fix is a separate
//! prerelease field on the wire rather than a cleverer packing here.

/// The release, as typed in the workspace `Cargo.toml` — `"0.1.0"`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The commit, nine hex, `-dirty` when the tree had uncommitted changes, or
/// `"unknown"` when there was no git to ask. Stamped by `build.rs`, whose
/// header states what the dirty marker does and does not prove.
pub const GIT_SHA: &str = env!("GATES_GIT_SHA");

/// `0.1.0-g2cccac26` — what the client prints in its corner, what the shard
/// logs for a connecting player, and the shape `ci/depot.py` keys an install
/// directory on. One string, so a bug report and a depot receipt name the
/// same thing.
pub const BUILD_ID: &str = concat!(env!("CARGO_PKG_VERSION"), "-g", env!("GATES_GIT_SHA"));

/// The release as one comparable integer: `major × 1_000_000 + minor × 1_000
/// + patch`. This is the value that crosses the wire in [`crate::Hello`] and
/// the one a shard's `min_client` is compared against.
pub const VER: u32 = pack(
    parse(env!("CARGO_PKG_VERSION_MAJOR")),
    parse(env!("CARGO_PKG_VERSION_MINOR")),
    parse(env!("CARGO_PKG_VERSION_PATCH")),
);

/// A 64-bit digest of [`BUILD_ID`]. Crosses the wire so a shard's log can name
/// the exact client build that hit a bug, which a semver alone cannot — two
/// players on `0.1.0` may be on different commits, and when one of them is
/// reproducing something the other is not, that is the first thing worth
/// knowing.
///
/// **Not a gate and not a security boundary.** It is FNV-1a, which is a
/// checksum: a client may state whatever it likes here and nothing downstream
/// trusts it. Its whole job is to make two builds distinguishable in a log.
pub const BUILD: u64 = fnv1a(BUILD_ID);

/// Pack a semver triple the way [`VER`] is packed.
///
/// Public because the shard parses `min_client` from `shard.toml` and must
/// arrive at the same number this crate would: one packing, both sides, which
/// is the argument `refuse_text` makes about a sentence.
pub const fn pack(major: u32, minor: u32, patch: u32) -> u32 {
    // A component at 1000 would carry into the field above it and silently
    // read as a different version — 0.1.1000 and 0.2.0 would be the same
    // number, and the shard would let in a client it meant to refuse. A
    // const panic is a compile error at the one site that types a version.
    assert!(
        minor < 1_000,
        "minor version must be < 1000 for the decimal packing"
    );
    assert!(
        patch < 1_000,
        "patch version must be < 1000 for the decimal packing"
    );
    major * 1_000_000 + minor * 1_000 + patch
}

/// `(major, minor, patch)` back out of a packed [`VER`]. For a caller that has
/// a number off the wire and owes a player a sentence with dots in it.
pub const fn unpack(ver: u32) -> (u32, u32, u32) {
    (ver / 1_000_000, (ver / 1_000) % 1_000, ver % 1_000)
}

/// Decimal `&str` → `u32`, const so [`VER`] is computed at compile time.
/// Cargo hands version components over as strings and there is no const
/// `str::parse`.
const fn parse(s: &str) -> u32 {
    let b = s.as_bytes();
    let mut i = 0;
    let mut n: u32 = 0;
    while i < b.len() {
        assert!(
            b[i] >= b'0' && b[i] <= b'9',
            "version component is not a number"
        );
        n = n * 10 + (b[i] - b'0') as u32;
        i += 1;
    }
    n
}

/// FNV-1a 64. Const so [`BUILD`] costs nothing at runtime, and hand-written
/// because pulling a hasher crate into the one crate that must stay
/// dependency-thin is not worth a checksum.
const fn fnv1a(s: &str) -> u64 {
    let b = s.as_bytes();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut i = 0;
    while i < b.len() {
        h ^= b[i] as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
        i += 1;
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The packing is an order-preserving map from semver, which is the whole
    /// reason the shard's gate can be one `<`. A case here that failed would
    /// mean a shard letting in a client it meant to refuse.
    #[test]
    fn the_packing_orders_the_way_semver_does() {
        let ordered = [
            pack(0, 0, 1),
            pack(0, 1, 0),
            pack(0, 1, 1),
            pack(0, 2, 0),
            pack(0, 10, 0),
            pack(0, 999, 999),
            pack(1, 0, 0),
            pack(1, 0, 1),
            pack(2, 0, 0),
        ];
        for w in ordered.windows(2) {
            assert!(w[0] < w[1], "{} should sort below {}", w[0], w[1]);
        }
    }

    /// Legible in a log without a decoder: 0.1.0 is 1000, 1.2.3 is 1002003.
    /// This is the property the decimal base was chosen for, so it is worth a
    /// gate rather than a comment.
    #[test]
    fn a_packed_version_reads_back_as_its_digits() {
        assert_eq!(pack(0, 1, 0), 1_000);
        assert_eq!(pack(1, 2, 3), 1_002_003);
        assert_eq!(pack(0, 0, 0), 0);
    }

    #[test]
    fn unpack_is_packs_inverse() {
        for (ma, mi, pa) in [(0, 0, 0), (0, 1, 0), (1, 2, 3), (7, 999, 999), (4095, 0, 1)] {
            assert_eq!(unpack(pack(ma, mi, pa)), (ma, mi, pa));
        }
    }

    /// `VER` is this crate's own version, and this is the gate that the build
    /// plumbing actually ran: a `parse` that silently returned 0 would leave
    /// `VER` at 0 and make every `min_client` gate pass.
    #[test]
    fn ver_agrees_with_the_cargo_version_string() {
        let mut parts = VERSION.split('-').next().expect("a version").split('.');
        let ma: u32 = parts
            .next()
            .expect("major")
            .parse()
            .expect("major is a number");
        let mi: u32 = parts
            .next()
            .expect("minor")
            .parse()
            .expect("minor is a number");
        let pa: u32 = parts
            .next()
            .expect("patch")
            .parse()
            .expect("patch is a number");
        assert_eq!(VER, pack(ma, mi, pa), "VER disagrees with {VERSION}");
        // A const block, because clippy is right that this is decided at
        // compile time — which is the point: a `parse` that silently returned
        // 0 would leave `VER` at 0 and make every `min_client` gate pass, and
        // catching that when the crate builds beats catching it when the test
        // runs.
        const {
            assert!(
                VER > 0,
                "a zero VER would make every minimum-client gate pass"
            )
        };
    }

    /// The id a player quotes in a bug report has to contain the two things
    /// we would ask them for.
    #[test]
    fn the_build_id_names_the_release_and_the_commit() {
        assert!(
            BUILD_ID.starts_with(VERSION),
            "{BUILD_ID} does not open with {VERSION}"
        );
        assert!(
            BUILD_ID.ends_with(GIT_SHA),
            "{BUILD_ID} does not end with {GIT_SHA}"
        );
        assert!(BUILD_ID.contains("-g"), "{BUILD_ID} has no commit marker");
    }

    /// Two different ids must not collide into one log line. Not a security
    /// claim — see `BUILD`'s own doc — just the property a checksum is for.
    #[test]
    fn different_builds_hash_apart() {
        assert_ne!(fnv1a("0.1.0-gaaaaaaaaa"), fnv1a("0.1.0-gbbbbbbbbb"));
        assert_ne!(fnv1a("0.1.0-gaaaaaaaaa"), fnv1a("0.1.1-gaaaaaaaaa"));
        assert_ne!(
            BUILD, 0,
            "a zero build hash would name every build the same"
        );
    }
}
