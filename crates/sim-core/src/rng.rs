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
