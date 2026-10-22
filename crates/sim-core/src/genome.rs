//! Bred brains: a small f32 net whose genome is a recipe, not its weights.
//!
//! **Where it comes from.** Quake Golf's neural golfer (`AnthonE/Quake-Golf`,
//! `client/brain.mjs`): a 274-weight net bred by mutation and selection,
//! whose genome is a base seed and one 16-bit seed per accepted mutation.
//! Replaying the recipe rebuilds the weights bit for bit — the encoding of
//! Such et al. 2017, "Deep Neuroevolution" — so a whole bred brain fits in a
//! tweet. This is that recipe, op for op: the same splitmix32 draws, the
//! same three-uniform noise, the same seed mix and step table, the same
//! softsign. Shape 14-16-2 here rebuilds a golfer's weights to the bit
//! (`tests` below holds it to values `brain.mjs` printed).
//!
//! **Why it can live in the sim.** Every op is `+ − × ÷` or `sqrt` on f32,
//! and the squash is one division by `1 + |x|` (`fmath::fabs`), so native
//! and wasm get the same bits (wall 1) and a replay thinks the same thought.
//! Weights sit in a fixed array: rebuilding a net, applying a mutation and
//! running it allocate nothing (wall 2).
//!
//! **What a genome says.** Nothing on its own: it is a recipe for one
//! *shape* reading one *feature list*. The four-byte prefix of the text form
//! names both (`gw1:` is the wolf's, `brain.rs` `WOLF_NET_PREFIX`), and a
//! change to either is a new prefix, never a silently different animal.

use crate::fmath::fabs;

/// Accepted mutations a genome holds: as many as still fit a tweet in text
/// form (4 prefix bytes + base64url of 4 + 2·101 bytes = 279 characters).
pub const MAX_MUTATIONS: usize = 101;
/// The longest text form, bytes.
pub const TEXT_MAX: usize = 280;

/// The largest net this module builds. A shape is checked against these
/// when it is made ([`Shape::new`]).
pub const MAX_IN: usize = 32;
pub const MAX_HIDDEN: usize = 32;
pub const MAX_OUT: usize = 8;
pub const MAX_WEIGHTS: usize = (MAX_IN + 1) * MAX_HIDDEN + (MAX_HIDDEN + 1) * MAX_OUT;

/// A mutation's step, picked by its seed's top two bits.
const SIGMA: [f32; 4] = [0.05, 0.1, 0.2, 0.4];

/// One hidden layer: `nin` inputs, `nhid` softsign units, `nout` softsign
/// outputs. Weights are laid out unit by unit, each unit's bias first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shape {
    nin: u8,
    nhid: u8,
    nout: u8,
}

impl Shape {
    /// `None` past the module's caps or with an empty layer.
    pub const fn new(nin: usize, nhid: usize, nout: usize) -> Option<Self> {
        if nin == 0 || nhid == 0 || nout == 0 {
            return None;
        }
        if nin > MAX_IN || nhid > MAX_HIDDEN || nout > MAX_OUT {
            return None;
        }
        Some(Self {
            nin: nin as u8,
            nhid: nhid as u8,
            nout: nout as u8,
        })
    }

    pub const fn inputs(self) -> usize {
        self.nin as usize
    }

    pub const fn outputs(self) -> usize {
        self.nout as usize
    }

    /// The weight count: `(nin + 1)·nhid + (nhid + 1)·nout`.
    pub const fn weights(self) -> usize {
        self.first() + (self.nhid as usize + 1) * self.nout as usize
    }

    /// Where the output layer starts.
    const fn first(self) -> usize {
        (self.nin as usize + 1) * self.nhid as usize
    }

    /// Weight `i`'s scale, 1/√fan-in, so a fresh net neither saturates nor
    /// sleeps.
    fn scale(self, i: usize) -> f32 {
        let fan = if i < self.first() {
            self.nin as f32 + 1.0
        } else {
            self.nhid as f32 + 1.0
        };
        1.0 / fan.sqrt()
    }
}

/// A recipe: the base seed, then one seed per accepted mutation. Fixed
/// size, so it is `Copy` and a pool of them is a flat array.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Genome {
    pub base: u32,
    n: u8,
    muts: [u16; MAX_MUTATIONS],
}

/// Why a text form was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GenomeError {
    /// It does not start with the prefix this shape expects.
    Prefix,
    /// A character outside base64url.
    Alphabet,
    /// Not 4 + 2n bytes, or a length base64 cannot produce.
    Length,
    /// More mutations than [`MAX_MUTATIONS`].
    TooLong,
}

impl Genome {
    /// The unmutated net from `base`.
    pub const fn new(base: u32) -> Self {
        Self {
            base,
            n: 0,
            muts: [0; MAX_MUTATIONS],
        }
    }

    pub fn muts(&self) -> &[u16] {
        &self.muts[..self.n as usize]
    }

    pub fn len(&self) -> usize {
        self.n as usize
    }

    pub fn is_empty(&self) -> bool {
        self.n == 0
    }

    /// Room for another mutation.
    pub fn full(&self) -> bool {
        self.n as usize >= MAX_MUTATIONS
    }

    /// This genome with one more mutation; `None` once it is full.
    pub fn child(&self, m: u16) -> Option<Self> {
        if self.full() {
            return None;
        }
        let mut c = *self;
        c.muts[c.n as usize] = m;
        c.n += 1;
        Some(c)
    }

    /// The text form: `prefix` then base64url, little-endian base and
    /// seeds, no padding. Returns the bytes written to `out`.
    pub fn encode(&self, prefix: &[u8; 4], out: &mut [u8; TEXT_MAX]) -> usize {
        let mut raw = [0u8; 4 + 2 * MAX_MUTATIONS];
        raw[..4].copy_from_slice(&self.base.to_le_bytes());
        for (i, m) in self.muts().iter().enumerate() {
            raw[4 + 2 * i..6 + 2 * i].copy_from_slice(&m.to_le_bytes());
        }
        out[..4].copy_from_slice(prefix);
        4 + b64_encode(&raw[..4 + 2 * self.len()], &mut out[4..])
    }

    /// Read a text form back. Surrounding whitespace is ignored.
    pub fn decode(prefix: &[u8; 4], text: &[u8]) -> Result<Self, GenomeError> {
        let text = text.trim_ascii();
        let body = text
            .strip_prefix(prefix.as_slice())
            .ok_or(GenomeError::Prefix)?;
        let mut raw = [0u8; 4 + 2 * MAX_MUTATIONS + 2];
        let n = b64_decode(body, &mut raw)?;
        if n < 4 || !n.is_multiple_of(2) {
            return Err(GenomeError::Length);
        }
        let muts = (n - 4) / 2;
        if muts > MAX_MUTATIONS {
            return Err(GenomeError::TooLong);
        }
        let mut g = Self::new(u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]));
        for i in 0..muts {
            g.muts[i] = u16::from_le_bytes([raw[4 + 2 * i], raw[5 + 2 * i]]);
        }
        g.n = muts as u8;
        Ok(g)
    }
}

/// A net's weights. Built from a genome and never written by anything else,
/// so the genome is the whole of what a bred animal is.
#[derive(Clone, Debug, PartialEq)]
pub struct Net {
    pub shape: Shape,
    pub w: [f32; MAX_WEIGHTS],
}

impl Net {
    /// The net `genome` describes, in `shape`.
    pub fn build(shape: Shape, genome: &Genome) -> Self {
        let mut net = Self {
            shape,
            w: [0.0; MAX_WEIGHTS],
        };
        net.rebuild(genome);
        net
    }

    /// Rewrite these weights from `genome`, in place.
    pub fn rebuild(&mut self, genome: &Genome) {
        let nw = self.shape.weights();
        let mut u = Draw(genome.base);
        for i in 0..nw {
            self.w[i] = noise(&mut u) * self.shape.scale(i);
        }
        self.w[nw..].fill(0.0);
        for (g, &m) in genome.muts().iter().enumerate() {
            self.mutate(genome.base, g, m);
        }
    }

    /// Apply mutation number `g` (from zero) with seed `m`: what
    /// [`Genome::child`] adds, so a child's net is its parent's plus this
    /// one step rather than a rebuild from the base.
    pub fn mutate(&mut self, base: u32, g: usize, m: u16) {
        let shape = self.shape;
        perturb(
            &mut self.w[..shape.weights()],
            |i| shape.scale(i),
            base,
            g,
            m,
        );
    }

    /// Run the net on `x` (at least `shape.inputs()` long) into `out` (at
    /// least `shape.outputs()` long). Every output is in (−1, 1).
    pub fn forward(&self, x: &[f32], out: &mut [f32]) {
        let (ni, nh, no) = (
            self.shape.nin as usize,
            self.shape.nhid as usize,
            self.shape.nout as usize,
        );
        let w = &self.w;
        let mut h = [0.0f32; MAX_HIDDEN];
        let mut k = 0;
        for hj in h.iter_mut().take(nh) {
            let mut s = w[k];
            k += 1;
            for &xi in &x[..ni] {
                s += w[k] * xi;
                k += 1;
            }
            *hj = soft(s);
        }
        for oj in out.iter_mut().take(no) {
            let mut s = w[k];
            k += 1;
            for &hi in &h[..nh] {
                s += w[k] * hi;
                k += 1;
            }
            *oj = soft(s);
        }
    }

    /// The index of the largest output; the first on a tie.
    pub fn choose(&self, x: &[f32]) -> u8 {
        let mut out = [0.0f32; MAX_OUT];
        self.forward(x, &mut out);
        let mut best = 0;
        for i in 1..self.shape.nout as usize {
            if out[i] > out[best] {
                best = i;
            }
        }
        best as u8
    }

    /// FNV-1a over the weights' little-endian bytes: a fingerprint two
    /// builds of the same genome must agree on.
    pub fn digest(&self) -> u32 {
        let mut h: u32 = 0x811c_9dc5;
        for w in &self.w[..self.shape.weights()] {
            for b in w.to_bits().to_le_bytes() {
                h ^= b as u32;
                h = h.wrapping_mul(0x0100_0193);
            }
        }
        h
    }
}

/// One mutation step over any weights: mutation number `g` (from zero)
/// with seed `m`, on a genome whose base is `base`, each weight moved by
/// the seed's step size times `scale(i)` times a fresh draw. What
/// [`Net::mutate`] does to a net, and what a bred student (`server`'s
/// `distill.rs`) does to its layers, so both breed by the one recipe.
pub fn perturb(w: &mut [f32], scale: impl Fn(usize) -> f32, base: u32, g: usize, m: u16) {
    let mut u = Draw(mix(base, g as u32, (m & 0x3fff) as u32));
    let sig = SIGMA[(m >> 14) as usize];
    for (i, wi) in w.iter_mut().enumerate() {
        *wi += sig * scale(i) * noise(&mut u);
    }
}

/// Softsign: tanh's shape from one division.
pub fn soft(x: f32) -> f32 {
    x / (1.0 + fabs(x))
}

/// splitmix32 for the bits, then 24 of them as an exact f32 in [0, 1).
#[derive(Clone, Copy)]
struct Draw(u32);

impl Draw {
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_add(0x9e37_79b9);
        let mut z = self.0;
        z = (z ^ (z >> 16)).wrapping_mul(0x85eb_ca6b);
        z = (z ^ (z >> 13)).wrapping_mul(0xc2b2_ae35);
        ((z ^ (z >> 16)) >> 8) as f32 * (1.0 / 16_777_216.0)
    }
}

/// Three uniforms, centred and scaled to variance 1.
fn noise(u: &mut Draw) -> f32 {
    let (a, b, c) = (u.next(), u.next(), u.next());
    (a + b + c - 1.5) * 2.0
}

/// Mutation `g`'s draw seed: the base, the mutation's place and its own
/// seed, so the same seed at another place is another step.
fn mix(base: u32, g: u32, s: u32) -> u32 {
    (base ^ 0x5bd1_e995).wrapping_mul(0x27d4_eb2d)
        ^ g.wrapping_add(1).wrapping_mul(0x9e37_79b9)
        ^ s.wrapping_add(1).wrapping_mul(0x85eb_ca6b)
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

fn b64_encode(raw: &[u8], out: &mut [u8]) -> usize {
    let mut n = 0;
    for chunk in raw.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let v = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        for k in 0..chunk.len() + 1 {
            out[n] = B64[((v >> (18 - 6 * k)) & 63) as usize];
            n += 1;
        }
    }
    n
}

fn b64_decode(text: &[u8], out: &mut [u8]) -> Result<usize, GenomeError> {
    if text.len() % 4 == 1 {
        return Err(GenomeError::Length);
    }
    let bytes = text.len() * 3 / 4;
    if bytes > out.len() {
        return Err(GenomeError::TooLong);
    }
    let mut n = 0;
    for chunk in text.chunks(4) {
        let mut v = 0u32;
        for (k, &c) in chunk.iter().enumerate() {
            let d = B64
                .iter()
                .position(|&a| a == c)
                .ok_or(GenomeError::Alphabet)?;
            v |= (d as u32) << (18 - 6 * k);
        }
        for k in 0..chunk.len() - 1 {
            out[n] = (v >> (16 - 8 * k)) as u8;
            n += 1;
        }
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The golfer's shape: `brain.mjs` NIN 14, NHID 16, NOUT 2.
    fn golfer() -> Shape {
        Shape::new(14, 16, 2).unwrap()
    }

    /// One mutation at each step size, and the largest seed.
    fn sample() -> Genome {
        let mut g = Genome::new(0xdead_beef);
        for m in [1, 0x4002, 0x8003, 0xc004, 0xffff] {
            g = g.child(m).unwrap();
        }
        g
    }

    /// Every number below was printed by Quake Golf's own `brain.mjs` (node
    /// 22): the port is held to the original, not to itself.
    #[test]
    fn it_rebuilds_the_golfers_weights_to_the_bit() {
        let mut u = Draw(7);
        assert_eq!(u.next() as f64, 0.1370641589164734);
        assert_eq!(u.next() as f64, 0.4510822296142578);
        assert_eq!(noise(&mut Draw(7)) as f64, 0.17249536514282227);
        assert_eq!(mix(0xdead_beef, 3, 0x1234), 2_497_828_529);

        let net = Net::build(golfer(), &sample());
        assert_eq!(golfer().weights(), 274);
        assert_eq!(net.w[0].to_bits(), 0x3e39_dfab);
        assert_eq!(net.w[273].to_bits(), 0xbeec_d9be);
        assert_eq!(net.digest(), 0x3739_3ef9);

        let x: [f32; 14] = core::array::from_fn(|i| (i as f32 - 6.0) / 4.0);
        let mut out = [0.0; 2];
        net.forward(&x, &mut out);
        assert_eq!(out[0].to_bits(), 0x3ea7_d996);
        assert_eq!(out[1].to_bits(), 0x3ad5_f5f1);
    }

    #[test]
    fn the_text_form_is_the_golfers_and_round_trips() {
        let g = sample();
        let mut buf = [0u8; TEXT_MAX];
        let n = g.encode(b"qg2:", &mut buf);
        assert_eq!(&buf[..n], b"qg2:776t3gEAAkADgATA__8");
        assert_eq!(Genome::decode(b"qg2:", &buf[..n]), Ok(g));
        assert_eq!(Genome::decode(b"gw1:", &buf[..n]), Err(GenomeError::Prefix));
        assert_eq!(
            Genome::decode(b"qg2:", b"qg2:77*t"),
            Err(GenomeError::Alphabet)
        );
        assert_eq!(
            Genome::decode(b"qg2:", b"qg2:776t3"),
            Err(GenomeError::Length)
        );
        assert_eq!(
            Genome::decode(b"qg2:", b"qg2:776t3gE"),
            Err(GenomeError::Length)
        );
    }

    #[test]
    fn a_full_genome_still_fits_a_tweet() {
        let mut g = Genome::new(u32::MAX);
        while let Some(c) = g.child(0xffff) {
            g = c;
        }
        assert_eq!(g.len(), MAX_MUTATIONS);
        let mut buf = [0u8; TEXT_MAX];
        let n = g.encode(b"gw1:", &mut buf);
        assert!(n <= TEXT_MAX, "{n}");
        assert_eq!(Genome::decode(b"gw1:", &buf[..n]), Ok(g));
    }

    #[test]
    fn a_child_is_its_parent_plus_one_step() {
        let parent = sample();
        let child = parent.child(0x9abc).unwrap();
        let mut stepped = Net::build(golfer(), &parent);
        stepped.mutate(parent.base, parent.len(), 0x9abc);
        assert_eq!(stepped, Net::build(golfer(), &child));
    }

    #[test]
    fn a_shape_past_the_caps_is_refused() {
        assert!(Shape::new(0, 16, 2).is_none());
        assert!(Shape::new(MAX_IN + 1, 16, 2).is_none());
        assert!(Shape::new(MAX_IN, MAX_HIDDEN, MAX_OUT).is_some());
        assert_eq!(
            Shape::new(MAX_IN, MAX_HIDDEN, MAX_OUT).unwrap().weights(),
            MAX_WEIGHTS
        );
    }
}
