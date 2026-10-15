//! Proximity voice's codec: a talker turns microphone samples into 20 ms
//! Opus frames, a listener turns each talker's frames back into samples.
//!
//! - [`Talker`] encodes mono [`RATE`] samples into [`FRAME`]-sample frames,
//!   numbering them, and flushes the last partial frame with the end flag
//!   when the talk key comes up.
//! - [`Decoder`] is one talker as heard: it decodes in arrival order at the
//!   listener's rate, conceals a short gap, and drops what arrives late.
//! - [`Resampler`] moves samples between rates (a device's 44.1/48 kHz and
//!   the codec's 16 kHz; a speech service's 24 kHz).
//!
//! Nothing here is the wire (`protocol::voice`) or the mix (`sound`).

use rusty_opus::{Application, OpusDecoder, OpusEncoder};

/// The codec's rate: wideband speech, Opus's SILK mode.
pub const RATE: u32 = 16_000;
/// One frame: 20 ms at [`RATE`].
pub const FRAME: usize = RATE as usize / 50;
/// Target bitrate. Wideband SILK sounds clean here, and a frame is ~60 B.
pub const BITRATE: i32 = 24_000;
/// Largest packet the encoder may write (the wire's cap is 160 B).
pub const MAX_PACKET: usize = 160;
/// Frames a gap may conceal before the rest is treated as silence.
pub const MAX_CONCEAL: u16 = 3;

/// The talking half: frames, numbers and encodes.
pub struct Talker {
    enc: OpusEncoder,
    buf: [f32; FRAME],
    fill: usize,
    seq: u16,
    out: [u8; MAX_PACKET],
    talking: bool,
}

impl Talker {
    pub fn new() -> Self {
        let mut enc = OpusEncoder::new(RATE as i32, 1, Application::Voip).expect("16 kHz mono");
        enc.bitrate_bps = BITRATE;
        Self {
            enc,
            buf: [0.0; FRAME],
            fill: 0,
            seq: 0,
            out: [0; MAX_PACKET],
            talking: false,
        }
    }

    /// Whether frames have gone out since the last [`Talker::finish`].
    pub fn talking(&self) -> bool {
        self.talking
    }

    /// Feed mono samples at [`RATE`]; `emit(seq, end, packet)` runs once
    /// per finished frame (never with `end` set: that is `finish`'s).
    pub fn push(&mut self, mut pcm: &[f32], mut emit: impl FnMut(u16, bool, &[u8])) {
        while !pcm.is_empty() {
            let n = (FRAME - self.fill).min(pcm.len());
            self.buf[self.fill..self.fill + n].copy_from_slice(&pcm[..n]);
            self.fill += n;
            pcm = &pcm[n..];
            if self.fill == FRAME {
                self.encode(false, &mut emit);
            }
        }
    }

    /// The talk key came up: the partial frame, padded with silence, goes
    /// out with the end flag. Nothing goes out if nothing was said.
    pub fn finish(&mut self, mut emit: impl FnMut(u16, bool, &[u8])) {
        if !self.talking && self.fill == 0 {
            return;
        }
        self.buf[self.fill..].fill(0.0);
        self.fill = FRAME;
        self.encode(true, &mut emit);
        self.talking = false;
    }

    fn encode(&mut self, end: bool, emit: &mut impl FnMut(u16, bool, &[u8])) {
        for s in self.buf.iter_mut() {
            *s = s.clamp(-1.0, 1.0);
        }
        self.fill = 0;
        match self.enc.encode(&self.buf, FRAME, &mut self.out) {
            Ok(n) if n > 0 && n <= MAX_PACKET => {
                let seq = self.seq;
                self.seq = self.seq.wrapping_add(1);
                self.talking = true;
                emit(seq, end, &self.out[..n]);
            }
            _ => {}
        }
    }
}

impl Default for Talker {
    fn default() -> Self {
        Self::new()
    }
}

/// Whether an Opus packet is one 20 ms frame (the only shape a talker
/// sends). Read off the TOC byte (RFC 6716 §3.1): code 0, one frame, and a
/// configuration whose frame is 20 ms. Anything else is concealed rather
/// than decoded, so a forged packet cannot make a listener decode 120 ms.
pub fn is_one_frame(packet: &[u8]) -> bool {
    let Some(&toc) = packet.first() else {
        return false;
    };
    if toc & 3 != 0 {
        return false;
    }
    let config = toc >> 3;
    match config {
        0..=11 => config % 4 == 1,
        12..=15 => config % 2 == 1,
        _ => config % 4 == 3,
    }
}

/// One talker as heard: decodes at `out_rate`, in order.
pub struct Decoder {
    dec: OpusDecoder,
    /// The rate the codec decodes at: `out_rate` when Opus can make it,
    /// else 48 kHz and [`Decoder::rs`] takes it the rest of the way.
    rate: u32,
    rs: Option<Resampler>,
    next: Option<u16>,
    scratch: Vec<f32>,
}

impl Decoder {
    pub fn new(out_rate: u32) -> Self {
        let native = [8_000, 12_000, 16_000, 24_000, 48_000].contains(&out_rate);
        let rate = if native { out_rate } else { 48_000 };
        Self {
            dec: OpusDecoder::new(rate as i32, 1).expect("an Opus rate"),
            rate,
            rs: (!native).then(|| Resampler::new(48_000, out_rate)),
            next: None,
            scratch: vec![0.0; rate as usize / 50],
        }
    }

    /// Samples one frame decodes to, before any resampling.
    fn frame(&self) -> usize {
        self.rate as usize / 50
    }

    /// Decode frame `seq`, appending the samples to `out`. A gap of up to
    /// [`MAX_CONCEAL`] frames is concealed first; a frame older than the
    /// last one decoded is dropped. `end` closes the utterance, so the next
    /// frame starts fresh whatever its number.
    pub fn push(&mut self, seq: u16, end: bool, packet: &[u8], out: &mut Vec<f32>) {
        if let Some(next) = self.next {
            let ahead = seq.wrapping_sub(next);
            if ahead >= 0x8000 {
                return; // late or a duplicate
            }
            for _ in 0..ahead.min(MAX_CONCEAL) {
                self.decode(&[], out);
            }
        }
        let packet = if is_one_frame(packet) { packet } else { &[] };
        self.decode(packet, out);
        self.next = if end { None } else { Some(seq.wrapping_add(1)) };
    }

    fn decode(&mut self, packet: &[u8], out: &mut Vec<f32>) {
        let n = self.frame();
        let got = match self.dec.decode(packet, n, &mut self.scratch[..n]) {
            Ok(got) => got.min(n),
            Err(_) => {
                self.scratch[..n].fill(0.0);
                n
            }
        };
        match self.rs.as_mut() {
            Some(rs) => rs.process(&self.scratch[..got], |s| out.push(s)),
            None => out.extend_from_slice(&self.scratch[..got]),
        }
    }
}

/// A second-order low-pass (RBJ's cookbook), for the band edge before a
/// rate comes down.
#[derive(Clone, Copy)]
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl Biquad {
    fn low_pass(rate: f32, cutoff: f32) -> Self {
        let w = std::f32::consts::TAU * cutoff / rate;
        let alpha = w.sin() / (2.0 * std::f32::consts::FRAC_1_SQRT_2);
        let c = w.cos();
        let a0 = 1.0 + alpha;
        Self {
            b0: (1.0 - c) / 2.0 / a0,
            b1: (1.0 - c) / a0,
            b2: (1.0 - c) / 2.0 / a0,
            a1: -2.0 * c / a0,
            a2: (1.0 - alpha) / a0,
            z1: 0.0,
            z2: 0.0,
        }
    }

    fn run(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }
}

/// Streaming linear resampler. Coming down in rate it low-passes first
/// (two cascaded biquads at 0.45 of the new rate), so a 48 kHz microphone
/// does not fold its top octave into a 16 kHz frame.
pub struct Resampler {
    step: f64,
    t: f64,
    prev: f32,
    lp: Option<[Biquad; 2]>,
}

impl Resampler {
    pub fn new(from: u32, to: u32) -> Self {
        let lp = (to < from).then(|| [Biquad::low_pass(from as f32, 0.45 * to as f32); 2]);
        Self {
            step: from as f64 / to as f64,
            t: 0.0,
            prev: 0.0,
            lp,
        }
    }

    /// Feed `input`; `emit` runs once per output sample.
    pub fn process(&mut self, input: &[f32], mut emit: impl FnMut(f32)) {
        for &x in input {
            let x = match self.lp.as_mut() {
                Some([a, b]) => b.run(a.run(x)),
                None => x,
            };
            while self.t < 1.0 {
                emit(self.prev + (x - self.prev) * self.t as f32);
                self.t += self.step;
            }
            self.t -= 1.0;
            self.prev = x;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(rate: u32, hz: f32, secs: f32) -> Vec<f32> {
        (0..(rate as f32 * secs) as usize)
            .map(|i| 0.3 * (std::f32::consts::TAU * hz * i as f32 / rate as f32).sin())
            .collect()
    }

    fn rms(s: &[f32]) -> f32 {
        (s.iter().map(|x| x * x).sum::<f32>() / s.len().max(1) as f32).sqrt()
    }

    #[test]
    fn a_push_round_trips_at_any_rate() {
        let mut talker = Talker::new();
        let mut frames = Vec::new();
        let pcm = tone(RATE, 300.0, 0.5);
        talker.push(&pcm[..pcm.len() - 100], |seq, end, p| {
            frames.push((seq, end, p.to_vec()))
        });
        talker.finish(|seq, end, p| frames.push((seq, end, p.to_vec())));
        assert_eq!(frames.len(), 25);
        assert!(frames
            .iter()
            .all(|(_, _, p)| is_one_frame(p) && p.len() <= MAX_PACKET));
        assert!(
            frames.last().unwrap().1,
            "the last frame carries the end flag"
        );
        assert!(!talker.talking());
        // Nothing said, nothing sent.
        let mut extra = 0;
        talker.finish(|_, _, _| extra += 1);
        assert_eq!(extra, 0);

        for rate in [16_000, 44_100, 48_000] {
            let mut dec = Decoder::new(rate);
            let mut out = Vec::new();
            for (seq, end, p) in &frames {
                dec.push(*seq, *end, p, &mut out);
            }
            let want = 25 * rate as usize / 50;
            assert!(
                out.len().abs_diff(want) <= 2,
                "{rate}: {} samples",
                out.len()
            );
            let mid = &out[out.len() / 4..out.len() * 3 / 4];
            assert!(
                rms(mid) > 0.12,
                "{rate}: the tone came through ({})",
                rms(mid)
            );
        }
    }

    #[test]
    fn gaps_conceal_and_late_frames_drop() {
        let mut talker = Talker::new();
        let mut frames = Vec::new();
        talker.push(&tone(RATE, 220.0, 0.2), |s, e, p| {
            frames.push((s, e, p.to_vec()))
        });
        let mut dec = Decoder::new(RATE);
        let mut out = Vec::new();
        dec.push(frames[0].0, false, &frames[0].2, &mut out);
        // Frames 1 and 2 lost: concealed when 3 arrives.
        dec.push(frames[3].0, false, &frames[3].2, &mut out);
        assert_eq!(out.len(), 4 * FRAME);
        // Frame 1 arriving now is late.
        dec.push(frames[1].0, false, &frames[1].2, &mut out);
        assert_eq!(out.len(), 4 * FRAME);
        // A forged 60 ms packet is concealed as one frame, not decoded.
        dec.push(frames[4].0, false, &[0b0001_1000, 1, 2, 3], &mut out);
        assert_eq!(out.len(), 5 * FRAME);
    }

    #[test]
    fn resampler_keeps_rate_and_level() {
        for (from, to) in [
            (48_000, 16_000),
            (44_100, 16_000),
            (24_000, 16_000),
            (16_000, 48_000),
        ] {
            let pcm = tone(from, 440.0, 1.0);
            let mut out = Vec::new();
            Resampler::new(from, to).process(&pcm, |s| out.push(s));
            assert!(
                out.len().abs_diff(to as usize) <= 2,
                "{from}->{to}: {}",
                out.len()
            );
            let level = rms(&out[out.len() / 4..]);
            assert!((level - rms(&pcm)).abs() < 0.02, "{from}->{to}: {level}");
        }
        // Above the new band edge, a tone is gone rather than folded down.
        let mut out = Vec::new();
        Resampler::new(48_000, 16_000).process(&tone(48_000, 12_000.0, 1.0), |s| out.push(s));
        assert!(rms(&out[out.len() / 4..]) < 0.03);
    }
}
