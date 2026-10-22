//! All randomness in the sim: splitmix64 for stateless spatial hashes
//! (TERRAIN.md §0) and a PCG32 hierarchy for runtime streams (DESIGN.md §7).
//! Integer-only; deterministic on every target by construction.

/// splitmix64 finalizer — the spatial hash primitive of record.
#[inline]
pub fn splitmix64(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Hash of `(seed, cell_x, cell_z, channel)` — the worldgen draw
/// (TERRAIN.md §0). Channels keep independent noise fields independent.
#[inline]
pub fn cell_hash(seed: u64, x: i32, z: i32, channel: u32) -> u64 {
    let mut h = seed ^ 0x517C_C1B7_2722_0A95;
    h = splitmix64(h ^ (x as u32 as u64));
    h = splitmix64(h ^ (z as u32 as u64));
    splitmix64(h ^ (channel as u64))
}

/// SipHash-2-4 of the two words `(m0, m1)` under the key `(k0, k1)`: the
/// keyed draw for a roll a client can watch and time.
///
/// splitmix64 is a bijection, so a secret xored into its input comes back
/// out of enough observed outputs (a gun's cone reads ~20 bits a shot at a
/// tick the shooter picks). A PRF's key does not: what one output shows
/// says nothing about the key or about any other input. Pure u64 wrapping
/// ops, so deterministic on every target, and no allocation.
#[inline]
pub fn sip24(k0: u64, k1: u64, m0: u64, m1: u64) -> u64 {
    let mut v = [
        k0 ^ 0x736F_6D65_7073_6575,
        k1 ^ 0x646F_7261_6E64_6F6D,
        k0 ^ 0x6C79_6765_6E65_7261,
        k1 ^ 0x7465_6462_7974_6573,
    ];
    #[inline]
    fn round(v: &mut [u64; 4]) {
        v[0] = v[0].wrapping_add(v[1]);
        v[1] = v[1].rotate_left(13) ^ v[0];
        v[0] = v[0].rotate_left(32);
        v[2] = v[2].wrapping_add(v[3]);
        v[3] = v[3].rotate_left(16) ^ v[2];
        v[0] = v[0].wrapping_add(v[3]);
        v[3] = v[3].rotate_left(21) ^ v[0];
        v[2] = v[2].wrapping_add(v[1]);
        v[1] = v[1].rotate_left(17) ^ v[2];
        v[2] = v[2].rotate_left(32);
    }
    // Two message words, then the length block (16 bytes, no tail).
    for m in [m0, m1, 16u64 << 56] {
        v[3] ^= m;
        round(&mut v);
        round(&mut v);
        v[0] ^= m;
    }
    v[2] ^= 0xFF;
    round(&mut v);
    round(&mut v);
    round(&mut v);
    round(&mut v);
    v[0] ^ v[1] ^ v[2] ^ v[3]
}

/// `seed` under the world's secret `salt` (`mech::ArcState::salt`), for
/// channel `channel`: the seed a client-timeable roll draws from. A salt of
/// 0 is the seed itself, so an unsalted world, every fixture and the parity
/// probe roll as they always did; any other salt keys a [`sip24`] of the
/// seed, so nothing a client reads off one channel's rolls names the salt
/// or another channel's seed.
#[inline]
pub fn salted(seed: u64, salt: u64, channel: u64) -> u64 {
    if salt == 0 {
        seed
    } else {
        sip24(salt, seed, channel, 0)
    }
}

/// PCG32 (XSH-RR): the per-system runtime stream. World seed -> stream id
/// -> an independent, reproducible sequence.
#[derive(Clone, Copy)]
pub struct Pcg32 {
    state: u64,
    inc: u64,
}

impl Pcg32 {
    pub fn new(seed: u64, stream: u64) -> Self {
        let mut r = Self {
            state: 0,
            inc: (stream << 1) | 1,
        };
        r.next_u32();
        r.state = r.state.wrapping_add(seed);
        r.next_u32();
        r
    }

    #[inline]
    pub fn next_u32(&mut self) -> u32 {
        let old = self.state;
        self.state = old
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(self.inc);
        let xorshifted = (((old >> 18) ^ old) >> 27) as u32;
        let rot = (old >> 59) as u32;
        xorshifted.rotate_right(rot)
    }

    /// Uniform in `[0, bound)` without modulo bias (Lemire).
    #[inline]
    pub fn next_bounded(&mut self, bound: u32) -> u32 {
        (((self.next_u32() as u64) * (bound as u64)) >> 32) as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The reference: std's `SipHasher` is SipHash-2-4, and a `u64` written
    /// to it as its 8 little-endian bytes makes 16 bytes our two words.
    #[allow(deprecated)]
    fn reference(k0: u64, k1: u64, m0: u64, m1: u64) -> u64 {
        use std::hash::Hasher;
        let mut h = std::hash::SipHasher::new_with_keys(k0, k1);
        h.write(&m0.to_le_bytes());
        h.write(&m1.to_le_bytes());
        h.finish()
    }

    #[test]
    fn sip24_is_siphash_2_4() {
        // The paper's key and message (bytes 0..16), then a spread.
        let k0 = u64::from_le_bytes([0, 1, 2, 3, 4, 5, 6, 7]);
        let k1 = u64::from_le_bytes([8, 9, 10, 11, 12, 13, 14, 15]);
        assert_eq!(sip24(k0, k1, k0, k1), reference(k0, k1, k0, k1));
        let mut x = 1u64;
        for _ in 0..64 {
            let (a, b, c, d) = (x, splitmix64(x), splitmix64(x ^ 1), splitmix64(x ^ 2));
            assert_eq!(sip24(a, b, c, d), reference(a, b, c, d));
            x = splitmix64(x);
        }
    }

    #[test]
    fn a_zero_salt_is_the_seed_and_any_other_moves_it() {
        assert_eq!(salted(7, 0, 1), 7);
        assert_ne!(salted(7, 1, 1), 7);
        assert_ne!(salted(7, 1, 1), salted(7, 2, 1));
        assert_ne!(salted(7, 1, 1), salted(7, 1, 2), "channels are apart");
    }
}
