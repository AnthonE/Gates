//! Proximity voice end to end (wire v113): one body talks into a real shard
//! over WebTransport and another body standing beside it hears the talker —
//! every frame from the talker's id, in order, with the end flag, decoding
//! to the tone that was said. The relay, the radius, both datagram lanes
//! and the codec, in one pass.

use protocol::VoiceFrame;
use server::botclient::{bot_endpoint, run_driven_bot, BotDriver};
use sim_core::input::InputFrame;
use std::sync::atomic::Ordering;
use std::time::Duration;

const SEED: u64 = 20_261_015;
/// Frames the talker says: two seconds.
const FRAMES: usize = 100;

/// Says a 300 Hz tone for two seconds once a second has passed.
struct Talker {
    frames: Vec<VoiceFrame>,
    ticks: u32,
    sent: usize,
    player_id: u32,
}

impl Talker {
    fn new() -> Self {
        let rate = voice::RATE as f32;
        let tone: Vec<f32> = (0..FRAMES * voice::FRAME - 50)
            .map(|i| 0.3 * (std::f32::consts::TAU * 300.0 * i as f32 / rate).sin())
            .collect();
        let mut enc = voice::Talker::new();
        let mut frames = Vec::new();
        enc.push(&tone, |seq, _, p| {
            frames.push(VoiceFrame::new(seq, 0, p).unwrap())
        });
        enc.finish(|seq, _, p| frames.push(VoiceFrame::new(seq, protocol::VOICE_END, p).unwrap()));
        assert_eq!(frames.len(), FRAMES);
        Self {
            frames,
            ticks: 0,
            sent: 0,
            player_id: 0,
        }
    }
}

impl BotDriver for Talker {
    fn frame(&mut self, _: &server::view::ClientView, player: u32, seq: u16) -> InputFrame {
        self.ticks += 1;
        self.player_id = player;
        InputFrame {
            seq,
            ..InputFrame::default()
        }
    }

    fn voice_out(&mut self, out: &mut [VoiceFrame]) -> usize {
        // 30 ticks a second, 50 frames a second: a tick owes 5/3 frames.
        if self.ticks < 30 {
            return 0;
        }
        let due = ((self.ticks - 30) as usize * 5 / 3 + 1).min(FRAMES);
        let mut n = 0;
        while self.sent < due && n < out.len() {
            out[n] = self.frames[self.sent];
            self.sent += 1;
            n += 1;
        }
        n
    }
}

/// Keeps every frame it hears.
#[derive(Default)]
struct Listener {
    heard: Vec<VoiceFrame>,
}

impl BotDriver for Listener {
    fn frame(&mut self, _: &server::view::ClientView, _: u32, seq: u16) -> InputFrame {
        InputFrame {
            seq,
            ..InputFrame::default()
        }
    }

    fn voice_in(&mut self, f: &VoiceFrame) {
        self.heard.push(*f);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_body_beside_a_talker_hears_them_through_the_shard() {
    let content = content::Content::load_dir(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content"),
    )
    .unwrap();
    let mut config = server::config::ShardConfig::ephemeral(SEED);
    config.dev_spawn = Some(sim_core::world::World::new(SEED).spawn_pos(0));
    let shard = server::net::spawn_shard(
        config,
        server::net::bake_all(&content).unwrap(),
        server::store::Saves::off(),
        server::worldfile::WorldBoot::off(),
    )
    .await
    .unwrap();
    let endpoint = bot_endpoint().unwrap();
    let mut talker = Talker::new();
    let mut listener = Listener::default();
    let walk = Duration::from_secs(5);
    let (a, b) = tokio::join!(
        run_driven_bot(&endpoint, shard.local_addr, walk, &mut talker),
        run_driven_bot(&endpoint, shard.local_addr, walk, &mut listener),
    );
    shard.shutdown.store(true, Ordering::Relaxed);
    endpoint.close(0u32.into(), b"test done");
    let (a, b) = (a.unwrap(), b.unwrap());
    assert_eq!(a.decode_errors + b.decode_errors, 0);
    assert_eq!(talker.sent, FRAMES, "the talker said it all");

    let heard = &listener.heard;
    assert!(
        heard.len() >= FRAMES * 9 / 10,
        "heard {} of {FRAMES}",
        heard.len()
    );
    assert!(
        heard.iter().all(|f| f.from == a.player_id),
        "every frame is the talker's"
    );
    assert!(heard.windows(2).all(|w| w[1].seq > w[0].seq), "in order");
    assert!(heard.last().unwrap().is_end(), "the end flag arrived");

    let mut dec = voice::Decoder::new(voice::RATE);
    let mut pcm = Vec::new();
    for f in heard {
        dec.push(f.seq, f.is_end(), f.payload(), &mut pcm);
    }
    let mid = &pcm[pcm.len() / 4..pcm.len() * 3 / 4];
    let rms = (mid.iter().map(|s| s * s).sum::<f32>() / mid.len() as f32).sqrt();
    assert!(rms > 0.12, "the tone came through ({rms})");
}
