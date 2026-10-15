//! An agent's ears and voice (proximity voice, wire v111): what the players
//! near it say out loud becomes text it hears, and what it says becomes
//! speech they hear.
//!
//! - **Ears.** Every relayed frame decodes into its talker's buffer at
//!   16 kHz. An utterance ends on the talker's end flag (their talk key came
//!   up), a pause, or [`MAX_UTTERANCE_S`], and goes to a speech-to-text
//!   service on the `agent-ears` thread. The text comes back as a heard
//!   line marked `voice`, with the talker's id, so the mind knows who said
//!   it (its friend included) exactly as it knows who typed a line.
//! - **Voice.** A line the mind says (`speak` in the external protocol, or
//!   its chat line) goes to a text-to-speech service on the `agent-mouth`
//!   thread; the samples come back, are encoded into 20 ms Opus frames and
//!   go out at the pace a microphone would send them.
//!
//! The services are any OpenAI-compatible `audio/transcriptions` and
//! `audio/speech` endpoints (OpenAI itself, a local server, a proxy), and
//! ElevenLabs for the voice when its key is set ([`VoiceConfig::from_env`]).
//! Both threads are bounded rings away from the bot loop, which never waits
//! on either: a slow service costs a late line, never a stalled body. Each
//! direction has a spend ceiling per hour.

use protocol::VoiceFrame;
use serde_json::{json, Value};
use std::time::{Duration, Instant};

/// The longest utterance sent to be transcribed; a longer one is cut.
pub const MAX_UTTERANCE_S: f32 = 15.0;
/// Shorter than this is a cough or a key tap, not speech.
const MIN_UTTERANCE_S: f32 = 0.35;
/// Quieter than this (RMS) is an open microphone, not speech.
const MIN_RMS: f32 = 0.006;
/// A talker silent this long has finished, end flag or not.
const PAUSE: Duration = Duration::from_millis(700);
/// Talkers listened to at once; past it the one quiet longest is dropped.
const EARS: usize = 8;
/// The longest line spoken, in characters.
pub const MAX_SPEECH_CHARS: usize = 240;
/// Frames sent ahead of real time, so a listener's buffer fills promptly.
const LEAD_FRAMES: u32 = 3;
/// How long a service call may take before the line is given up.
const SERVICE_TIMEOUT: Duration = Duration::from_secs(20);
/// The biggest answer read from a service (30 s of 24 kHz speech).
const MAX_AUDIO_BYTES: u64 = 2 * 24_000 * 30;

/// Where speech goes to be heard and comes from to be said.
#[derive(Clone, Debug)]
pub struct VoiceConfig {
    /// `…/v1`: `audio/transcriptions` and `audio/speech` hang off it.
    pub base: String,
    pub key: String,
    pub stt_model: String,
    pub tts_model: String,
    /// The OpenAI-compatible voice name.
    pub tts_voice: String,
    /// How the voice should sound (`instructions`, for models that take it).
    pub tts_style: String,
    /// ElevenLabs instead, when both are set: `(api key, voice id, model)`.
    pub eleven: Option<(String, String, String)>,
    /// Words the transcriber should expect (its `prompt`): names, places.
    pub vocabulary: String,
    pub max_heard_hour: u32,
    pub max_said_hour: u32,
}

impl VoiceConfig {
    /// Whether it has ears: a transcriber's key.
    pub fn hears(&self) -> bool {
        !self.key.is_empty()
    }

    /// From the environment: `GATES_VOICE_KEY` (else `OPENAI_API_KEY`),
    /// `GATES_VOICE_URL`, `GATES_STT_MODEL`, `GATES_TTS_MODEL`,
    /// `GATES_TTS_VOICE`, `GATES_TTS_STYLE`; `ELEVENLABS_API_KEY` with
    /// `GATES_ELEVEN_VOICE` (and `GATES_ELEVEN_MODEL`) for an ElevenLabs
    /// voice. With only an ElevenLabs voice it talks and does not hear
    /// (`key` empty); `Err` with neither.
    pub fn from_env() -> Result<Self, String> {
        let var = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
        let key = var("GATES_VOICE_KEY").or_else(|| var("OPENAI_API_KEY"));
        let eleven = match (var("ELEVENLABS_API_KEY"), var("GATES_ELEVEN_VOICE")) {
            (Some(k), Some(v)) => Some((
                k,
                v,
                var("GATES_ELEVEN_MODEL").unwrap_or_else(|| "eleven_flash_v2_5".into()),
            )),
            _ => None,
        };
        if key.is_none() && eleven.is_none() {
            return Err(
                "--voice needs GATES_VOICE_KEY or OPENAI_API_KEY (speech to text), \
                 or ELEVENLABS_API_KEY with GATES_ELEVEN_VOICE (a voice alone)"
                    .into(),
            );
        }
        let key = key.unwrap_or_default();
        let number = |k: &str, d: u32| var(k).and_then(|v| v.parse().ok()).unwrap_or(d);
        Ok(Self {
            base: var("GATES_VOICE_URL")
                .unwrap_or_else(|| "https://api.openai.com/v1".into())
                .trim_end_matches('/')
                .to_string(),
            key,
            stt_model: var("GATES_STT_MODEL").unwrap_or_else(|| "gpt-4o-mini-transcribe".into()),
            tts_model: var("GATES_TTS_MODEL").unwrap_or_else(|| "gpt-4o-mini-tts".into()),
            tts_voice: var("GATES_TTS_VOICE").unwrap_or_else(|| "ash".into()),
            tts_style: var("GATES_TTS_STYLE").unwrap_or_default(),
            eleven,
            vocabulary: var("GATES_STT_PROMPT").unwrap_or_else(|| {
                "Gates, Doctor Wood, Triple T, tung tung tung, THE GATE, junk, scrap.".into()
            }),
            max_heard_hour: number("GATES_VOICE_HEARD_HOUR", 240),
            max_said_hour: number("GATES_VOICE_SAID_HOUR", 240),
        })
    }
}

/// One talker being listened to.
struct Ear {
    from: u32,
    dec: ::voice::Decoder,
    pcm: Vec<f32>,
    last: Instant,
}

/// What the voice did, for the run's report.
#[derive(Clone, Copy, Debug, Default)]
pub struct VoiceStats {
    pub frames_heard: u64,
    pub utterances: u64,
    pub transcribed: u64,
    pub heard_dropped: u64,
    pub lines_spoken: u64,
    pub said_dropped: u64,
    pub frames_sent: u64,
    pub failures: u64,
}

/// An agent's ears and voice. Owned by the body (`Survivor`); everything
/// here is a ring push or pop, so the bot loop never waits.
pub struct AgentVoice {
    ears: Vec<Ear>,
    to_ears: rtrb::Producer<(u32, Vec<f32>)>,
    from_ears: rtrb::Consumer<(u32, String)>,
    to_mouth: rtrb::Producer<String>,
    from_mouth: rtrb::Consumer<Vec<f32>>,
    /// Lines the mind asked to have spoken (`External`'s `speak`).
    speech: rtrb::Consumer<String>,
    talker: ::voice::Talker,
    /// The line being sent: its frames, and when its first was due.
    out: std::collections::VecDeque<VoiceFrame>,
    out_start: Instant,
    out_sent: u32,
    /// It has a transcriber; without one, what it hears goes by.
    hears: bool,
    heard_window: Window,
    said_window: Window,
    pub stats: VoiceStats,
    /// What went wrong on either thread, said on the bot's stderr.
    errors: [rtrb::Consumer<String>; 2],
}

/// A spend ceiling per rolling hour.
struct Window {
    start: Instant,
    used: u32,
    cap: u32,
}

impl Window {
    fn new(cap: u32) -> Self {
        Self {
            start: Instant::now(),
            used: 0,
            cap,
        }
    }

    fn take(&mut self, now: Instant) -> bool {
        if now.duration_since(self.start) >= Duration::from_secs(3_600) {
            self.start = now;
            self.used = 0;
        }
        if self.used >= self.cap {
            return false;
        }
        self.used += 1;
        true
    }
}

/// The producer end the mind's external door pushes `speak` lines into.
pub type SpeechTx = rtrb::Producer<String>;

impl AgentVoice {
    /// Start the two service threads. The returned producer is where the
    /// mind's own lines go (`External::with_speech`).
    pub fn spawn(cfg: VoiceConfig) -> Result<(Self, SpeechTx), String> {
        let (to_ears, mut ears_rx) = rtrb::RingBuffer::<(u32, Vec<f32>)>::new(4);
        let (mut ears_tx, from_ears) = rtrb::RingBuffer::<(u32, String)>::new(8);
        let (to_mouth, mut mouth_rx) = rtrb::RingBuffer::<String>::new(2);
        let (mut mouth_tx, from_mouth) = rtrb::RingBuffer::<Vec<f32>>::new(2);
        let (speech_tx, speech) = rtrb::RingBuffer::<String>::new(2);
        let (mut err_ears, ears_errors) = rtrb::RingBuffer::<String>::new(8);
        let (mut err_mouth, mouth_errors) = rtrb::RingBuffer::<String>::new(8);
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(SERVICE_TIMEOUT))
            .http_status_as_error(false)
            .build()
            .into();
        {
            let cfg = cfg.clone();
            let agent = agent.clone();
            std::thread::Builder::new()
                .name("agent-ears".into())
                .spawn(move || loop {
                    match ears_rx.pop() {
                        Ok((from, pcm)) => match transcribe(&agent, &cfg, &pcm) {
                            Ok(text) if !text.is_empty() => {
                                let _ = ears_tx.push((from, text));
                            }
                            Ok(_) => {}
                            Err(e) => {
                                let _ = err_ears.push(e);
                            }
                        },
                        Err(_) if ears_rx.is_abandoned() => return,
                        Err(_) => std::thread::sleep(Duration::from_millis(20)),
                    }
                })
                .map_err(|e| format!("agent-ears: {e}"))?;
        }
        let (max_heard, max_said, hears) = (cfg.max_heard_hour, cfg.max_said_hour, cfg.hears());
        std::thread::Builder::new()
            .name("agent-mouth".into())
            .spawn(move || loop {
                match mouth_rx.pop() {
                    Ok(text) => match synthesize(&agent, &cfg, &text) {
                        Ok(pcm) => {
                            let _ = mouth_tx.push(pcm);
                        }
                        Err(e) => {
                            let _ = err_mouth.push(e);
                        }
                    },
                    Err(_) if mouth_rx.is_abandoned() => return,
                    Err(_) => std::thread::sleep(Duration::from_millis(20)),
                }
            })
            .map_err(|e| format!("agent-mouth: {e}"))?;
        let now = Instant::now();
        Ok((
            Self {
                ears: Vec::with_capacity(EARS),
                to_ears,
                from_ears,
                to_mouth,
                from_mouth,
                speech,
                talker: ::voice::Talker::new(),
                out: std::collections::VecDeque::new(),
                out_start: now,
                out_sent: 0,
                hears,
                heard_window: Window::new(max_heard),
                said_window: Window::new(max_said),
                stats: VoiceStats::default(),
                errors: [ears_errors, mouth_errors],
            },
            speech_tx,
        ))
    }

    /// A frame relayed from a player near this body.
    pub fn hear(&mut self, f: &VoiceFrame, now: Instant) {
        self.stats.frames_heard += 1;
        if !self.hears {
            return;
        }
        let i = match self.ears.iter().position(|e| e.from == f.from) {
            Some(i) => i,
            None => {
                if self.ears.len() == EARS {
                    let quiet = (0..EARS).min_by_key(|&i| self.ears[i].last).unwrap_or(0);
                    self.ears.swap_remove(quiet);
                }
                self.ears.push(Ear {
                    from: f.from,
                    dec: ::voice::Decoder::new(::voice::RATE),
                    pcm: Vec::new(),
                    last: now,
                });
                self.ears.len() - 1
            }
        };
        let ear = &mut self.ears[i];
        ear.last = now;
        ear.dec.push(f.seq, f.is_end(), f.payload(), &mut ear.pcm);
        let full = ear.pcm.len() as f32 >= MAX_UTTERANCE_S * ::voice::RATE as f32;
        if f.is_end() || full {
            self.finish(i, now);
        }
    }

    fn finish(&mut self, i: usize, now: Instant) {
        let ear = &mut self.ears[i];
        let pcm = std::mem::take(&mut ear.pcm);
        let from = ear.from;
        let secs = pcm.len() as f32 / ::voice::RATE as f32;
        let rms = (pcm.iter().map(|s| s * s).sum::<f32>() / pcm.len().max(1) as f32).sqrt();
        if secs < MIN_UTTERANCE_S || rms < MIN_RMS {
            return;
        }
        self.stats.utterances += 1;
        if !self.heard_window.take(now) || self.to_ears.push((from, pcm)).is_err() {
            self.stats.heard_dropped += 1;
        }
    }

    /// Once a tick: close utterances that paused, start the next line, and
    /// hand back what was heard. `heard(from, text)` runs per transcript.
    pub fn tick(&mut self, now: Instant, mut heard: impl FnMut(u32, &str)) {
        for i in (0..self.ears.len()).rev() {
            if !self.ears[i].pcm.is_empty() && now.duration_since(self.ears[i].last) >= PAUSE {
                self.finish(i, now);
            }
        }
        while let Ok((from, text)) = self.from_ears.pop() {
            self.stats.transcribed += 1;
            heard(from, &text);
        }
        for errors in &mut self.errors {
            while let Ok(e) = errors.pop() {
                self.stats.failures += 1;
                eprintln!("voice: {e}");
            }
        }
        while let Ok(text) = self.speech.pop() {
            self.say(&text, now);
        }
        // The next line's samples, once the last line has gone out.
        if self.out.is_empty() {
            if let Ok(pcm) = self.from_mouth.pop() {
                let out = &mut self.out;
                self.talker.push(&pcm, |seq, _, p| {
                    if let Some(f) = VoiceFrame::new(seq, 0, p) {
                        out.push_back(f);
                    }
                });
                self.talker.finish(|seq, _, p| {
                    if let Some(f) = VoiceFrame::new(seq, protocol::VOICE_END, p) {
                        out.push_back(f);
                    }
                });
                self.out_start = now;
                self.out_sent = 0;
                self.stats.lines_spoken += 1;
            }
        }
    }

    /// Have `text` spoken. A line while another waits replaces it.
    pub fn say(&mut self, text: &str, now: Instant) {
        let text = clean(text);
        if text.is_empty() {
            return;
        }
        if !self.said_window.take(now) || self.to_mouth.push(text).is_err() {
            self.stats.said_dropped += 1;
        }
    }

    /// Whether a line is being sent or waits to be.
    pub fn busy(&self) -> bool {
        !self.out.is_empty()
    }

    /// The frames due by `now` (a few ahead of it), oldest first.
    pub fn due(&mut self, now: Instant, out: &mut [VoiceFrame]) -> usize {
        let elapsed = now.duration_since(self.out_start).as_millis() as u32;
        let due = elapsed / 20 + LEAD_FRAMES;
        let mut n = 0;
        while n < out.len() && self.out_sent < due {
            let Some(f) = self.out.pop_front() else {
                break;
            };
            out[n] = f;
            n += 1;
            self.out_sent += 1;
            self.stats.frames_sent += 1;
        }
        n
    }
}

/// One printable line, cut at a word within [`MAX_SPEECH_CHARS`].
pub fn clean(text: &str) -> String {
    let line: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let line = line.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() <= MAX_SPEECH_CHARS {
        return line;
    }
    let cut: String = line.chars().take(MAX_SPEECH_CHARS).collect();
    match cut.rfind(' ') {
        Some(i) => cut[..i].to_string(),
        None => cut,
    }
}

/// 16-bit mono WAV around `pcm` at the codec's rate.
fn wav(pcm: &[f32]) -> Vec<u8> {
    let rate = ::voice::RATE;
    let data = (pcm.len() * 2) as u32;
    let mut b = Vec::with_capacity(44 + data as usize);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&rate.to_le_bytes());
    b.extend_from_slice(&(rate * 2).to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data.to_le_bytes());
    for s in pcm {
        b.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32_767.0) as i16).to_le_bytes());
    }
    b
}

/// What was said in `pcm`, as text (one line, at most a few hundred bytes).
fn transcribe(agent: &ureq::Agent, cfg: &VoiceConfig, pcm: &[f32]) -> Result<String, String> {
    let boundary = format!("gates{:016x}", pcm.len() as u64 ^ 0x5eed_f00d_cafe_d00d);
    let mut body = Vec::new();
    let mut field = |name: &str, value: &str| {
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
            )
            .as_bytes(),
        );
    };
    field("model", &cfg.stt_model);
    field("response_format", "json");
    if !cfg.vocabulary.is_empty() {
        field("prompt", &cfg.vocabulary);
    }
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"speech.wav\"\r\nContent-Type: audio/wav\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(&wav(pcm));
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    let mut res = agent
        .post(format!("{}/audio/transcriptions", cfg.base))
        .header("Authorization", format!("Bearer {}", cfg.key))
        .content_type(format!("multipart/form-data; boundary={boundary}"))
        .send(&body[..])
        .map_err(|e| format!("transcription failed: {e}"))?;
    let status = res.status().as_u16();
    let text = res
        .body_mut()
        .with_config()
        .limit(64 * 1024)
        .read_to_string()
        .map_err(|_| "transcription unreadable")?;
    if status != 200 {
        return Err(format!("transcription HTTP {status}: {}", head(&text)));
    }
    let v: Value = serde_json::from_str(&text).map_err(|_| "transcription not JSON")?;
    Ok(clean(v["text"].as_str().unwrap_or("")))
}

/// `text` spoken, as mono samples at the codec's rate.
fn synthesize(agent: &ureq::Agent, cfg: &VoiceConfig, text: &str) -> Result<Vec<f32>, String> {
    let (req, rate) = match &cfg.eleven {
        Some((key, voice, model)) => (
            agent
                .post(format!(
                    "https://api.elevenlabs.io/v1/text-to-speech/{voice}?output_format=pcm_16000"
                ))
                .header("xi-api-key", key)
                .content_type("application/json")
                .send(json!({ "text": text, "model_id": model }).to_string()),
            16_000,
        ),
        None => {
            let mut body = json!({
                "model": cfg.tts_model,
                "voice": cfg.tts_voice,
                "input": text,
                "response_format": "pcm",
            });
            if !cfg.tts_style.is_empty() {
                body["instructions"] = Value::String(cfg.tts_style.clone());
            }
            (
                agent
                    .post(format!("{}/audio/speech", cfg.base))
                    .header("Authorization", format!("Bearer {}", cfg.key))
                    .content_type("application/json")
                    .send(body.to_string()),
                24_000,
            )
        }
    };
    let mut res = req.map_err(|e| format!("speech failed: {e}"))?;
    let status = res.status().as_u16();
    let bytes = res
        .body_mut()
        .with_config()
        .limit(MAX_AUDIO_BYTES)
        .read_to_vec()
        .map_err(|_| "speech unreadable or too long")?;
    if status != 200 {
        return Err(format!(
            "speech HTTP {status}: {}",
            head(&String::from_utf8_lossy(&bytes))
        ));
    }
    let pcm: Vec<f32> = bytes
        .chunks_exact(2)
        .map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32_768.0)
        .collect();
    if rate == ::voice::RATE {
        return Ok(pcm);
    }
    let mut out = Vec::with_capacity(pcm.len() * ::voice::RATE as usize / rate as usize + 1);
    ::voice::Resampler::new(rate, ::voice::RATE).process(&pcm, |s| out.push(s));
    Ok(out)
}

fn head(text: &str) -> String {
    text.chars().take(200).collect()
}

/// A transcript as heard lines: cut at words into pieces the chat rule
/// takes, the first `false` and the rest `true` (each continues the last).
pub fn pieces(text: &str) -> Vec<(protocol::ChatText, bool)> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for word in text.split_whitespace() {
        let word: String = word.chars().take(protocol::CHAT_MAX_BYTES / 4).collect();
        if !cur.is_empty() && cur.len() + 1 + word.len() > protocol::CHAT_MAX_BYTES {
            if let Some(t) = protocol::ChatText::sanitize(cur.as_bytes()) {
                out.push(t);
            }
            cur.clear();
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(&word);
    }
    if let Some(t) = protocol::ChatText::sanitize(cur.as_bytes()) {
        out.push(t);
    }
    out.into_iter()
        .enumerate()
        .map(|(i, t)| (t, i > 0))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_keeps_one_line_and_cuts_at_a_word() {
        assert_eq!(clean("  hi\nthere\t friend "), "hi there friend");
        let long = "word ".repeat(100);
        let c = clean(&long);
        assert!(c.chars().count() <= MAX_SPEECH_CHARS && c.ends_with("word"));
    }

    #[test]
    fn a_transcript_splits_into_chat_sized_pieces() {
        let text =
            "doc follow me to the big rock by the river and bring the bat we are raiding tonight";
        let p = pieces(text);
        assert!(p.len() >= 2);
        assert!(!p[0].1 && p[1..].iter().all(|(_, more)| *more));
        let joined: Vec<&str> = p
            .iter()
            .map(|(t, _)| core::str::from_utf8(t.as_bytes()).unwrap())
            .collect();
        assert_eq!(joined.join(" "), text);
        assert!(pieces("").is_empty());
    }

    #[test]
    fn wav_has_a_header_and_every_sample() {
        let w = wav(&[0.0, 0.5, -0.5]);
        assert_eq!(&w[..4], b"RIFF");
        assert_eq!(w.len(), 44 + 6);
    }

    /// A stand-in for the speech services on loopback: transcriptions
    /// answer one line, speech answers 0.4 s of 24 kHz tone. Each request's
    /// path and body go back to the test on a ring.
    fn mock(mut seen: rtrb::Producer<(String, Vec<u8>)>) -> std::net::SocketAddr {
        use std::io::{BufRead, BufReader, Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { return };
                let mut r = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                r.read_line(&mut line).unwrap();
                let path = line.split(' ').nth(1).unwrap_or("").to_string();
                let mut len = 0;
                loop {
                    let mut h = String::new();
                    r.read_line(&mut h).unwrap();
                    if h.trim().is_empty() {
                        break;
                    }
                    if let Some(v) = h.to_ascii_lowercase().strip_prefix("content-length:") {
                        len = v.trim().parse().unwrap();
                    }
                }
                let mut body = vec![0; len];
                r.read_exact(&mut body).unwrap();
                let answer: Vec<u8> = if path.ends_with("/audio/transcriptions") {
                    br#"{"text":"  doc, follow me!  "}"#.to_vec()
                } else {
                    (0..9_600)
                        .flat_map(|i| {
                            let s =
                                0.3 * (std::f32::consts::TAU * 200.0 * i as f32 / 24_000.0).sin();
                            ((s * 32_767.0) as i16).to_le_bytes()
                        })
                        .collect()
                };
                let _ = seen.push((path, body));
                let mut w = stream;
                let _ = write!(
                    w,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    answer.len()
                );
                let _ = w.write_all(&answer);
            }
        });
        addr
    }

    #[test]
    fn it_hears_a_talker_and_speaks_a_line_through_the_services() {
        let (seen_tx, mut seen) = rtrb::RingBuffer::new(8);
        let addr = mock(seen_tx);
        let cfg = VoiceConfig {
            base: format!("http://{addr}/v1"),
            key: "test".into(),
            stt_model: "stt".into(),
            tts_model: "tts".into(),
            tts_voice: "ash".into(),
            tts_style: "warm".into(),
            eleven: None,
            vocabulary: "Doctor Wood".into(),
            max_heard_hour: 10,
            max_said_hour: 10,
        };
        let (mut v, mut speech) = AgentVoice::spawn(cfg).unwrap();
        // A friend says half a second of something, then lets go of the key.
        let tone: Vec<f32> = (0..8_000)
            .map(|i| 0.3 * (std::f32::consts::TAU * 250.0 * i as f32 / 16_000.0).sin())
            .collect();
        let mut talker = ::voice::Talker::new();
        let mut frames = Vec::new();
        talker.push(&tone, |seq, _, p| {
            frames.push(VoiceFrame::new(seq, 0, p).unwrap())
        });
        talker
            .finish(|seq, _, p| frames.push(VoiceFrame::new(seq, protocol::VOICE_END, p).unwrap()));
        let now = Instant::now();
        for mut f in frames {
            f.from = 77;
            v.hear(&f, now);
        }
        // The mind asks for a line.
        speech.push("Hold still, friend!".into()).unwrap();

        let mut heard = Vec::new();
        let mut out = Vec::new();
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(10)
            && (heard.is_empty() || out.last().is_none_or(|f: &VoiceFrame| !f.is_end()))
        {
            let now = Instant::now();
            v.tick(now, |from, text| heard.push((from, text.to_string())));
            let mut buf = [VoiceFrame::EMPTY; 4];
            let n = v.due(now, &mut buf);
            out.extend_from_slice(&buf[..n]);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(heard, vec![(77, "doc, follow me!".to_string())]);
        // 0.4 s of speech is 20 frames, then the end flag's own frame.
        assert_eq!(out.len(), 21, "{:?}", v.stats);
        assert!(out.last().unwrap().is_end());
        let mut paths = Vec::new();
        while let Ok((path, body)) = seen.pop() {
            if path.ends_with("transcriptions") {
                let body = String::from_utf8_lossy(&body);
                assert!(body.contains("name=\"model\"\r\n\r\nstt"));
                assert!(body.contains("Doctor Wood") && body.contains("RIFF"));
            } else {
                let body: Value = serde_json::from_slice(&body).unwrap();
                assert_eq!(body["input"], "Hold still, friend!");
                assert_eq!(body["instructions"], "warm");
            }
            paths.push(path);
        }
        paths.sort();
        assert_eq!(paths, ["/v1/audio/speech", "/v1/audio/transcriptions"]);
    }

    #[test]
    fn a_window_caps_an_hour() {
        let mut w = Window::new(2);
        let now = Instant::now();
        assert!(w.take(now) && w.take(now) && !w.take(now));
        assert!(w.take(now + Duration::from_secs(3_601)));
    }
}
