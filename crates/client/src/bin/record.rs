//! `record <host:port> <out.rec> <seconds> [--walk x,z]` — tape a live
//! session for a trailer (`ci/film.sh`, `crate::film`).
//!
//! `record --scan <tape.rec> [every_s]` — play a tape back with no window and
//! print what is in it: where every body and animal is, how many pieces
//! stand, and when and where charges are planted and pieces come down. It is
//! how a shot list finds its moments.
//!
//! Joins as a guest player and stands still (after walking to `x,z`, if
//! asked — out of a raid's way, say), which makes it the camera crew:
//! a replay never draws its own body (`render::bodies` skips `player_id`), so
//! the tape holds everything within AOI of where it stood and shows none of
//! itself. Pumps at ~250 Hz so each snapshot is written down within a few ms
//! of arriving; the replay hands them back at those times.

use client::film::{self, Lane};
use client::{client_endpoint, Session};
use std::io::{BufWriter, Write};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// What the tap hands over between pumps, in arrival order.
type Taken = Arc<Mutex<Vec<(Lane, Vec<u8>)>>>;

fn die(why: impl std::fmt::Display) -> ! {
    eprintln!("record: {why}");
    std::process::exit(1)
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--scan") {
        let every: f64 = args.get(2).and_then(|v| v.parse().ok()).unwrap_or(5.0);
        scan(
            args.get(1).unwrap_or_else(|| die("--scan needs a tape")),
            every,
        );
        return;
    }
    let (server, out, secs, walk) = match args.as_slice() {
        [server, out, secs] => (server, out, secs, None),
        [server, out, secs, flag, at] if flag == "--walk" => {
            let xz: Vec<f32> = at
                .split(',')
                .filter_map(|v| v.trim().parse().ok())
                .collect();
            let [x, z] = xz[..] else {
                die(format!("--walk wants x,z, not `{at}`"));
            };
            (server, out, secs, Some((x, z)))
        }
        _ => {
            eprintln!("usage: record <host:port> <out.rec> <seconds> [--walk x,z]");
            std::process::exit(2);
        }
    };
    let secs: f64 = secs
        .parse()
        .unwrap_or_else(|_| die(format!("`{secs}` is not a number of seconds")));

    let endpoint = client_endpoint(server, None).unwrap_or_else(|e| die(e));
    let mut session = Session::connect(&endpoint, server, protocol::Address::GUEST, |_, _, _| None)
        .await
        .unwrap_or_else(|e| die(e));
    let w = session.welcome;
    println!(
        "record: in the world — player {} seed {} tick {}",
        w.player_id, w.seed, w.tick
    );

    let file = std::fs::File::create(out).unwrap_or_else(|e| die(format!("{out}: {e}")));
    let mut file = BufWriter::new(file);
    film::write_header(&mut file, &w).unwrap_or_else(|e| die(e));

    let taken: Taken = Arc::default();
    let sink = taken.clone();
    session.tap(move |lane, bytes| {
        if let Ok(mut v) = sink.lock() {
            v.push((lane, bytes.to_vec()));
        }
    });

    // The session clock: the sum of every `dt_ms` handed to `pump`, which is
    // the only clock the core ever sees. An entry is stamped with the clock
    // BEFORE the pump that drained it advanced it, so a replay that feeds
    // everything at or before its own pre-pump clock is never early.
    let mut clock_ms = 0.0f64;
    let mut counts = [0u64; 2];
    let mut last = Instant::now();
    let mut reported = Instant::now();
    let mut ticker = tokio::time::interval(Duration::from_millis(4));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    while clock_ms < secs * 1000.0 {
        ticker.tick().await;
        let now = Instant::now();
        let dt_ms = now.duration_since(last).as_secs_f64() * 1000.0;
        last = now;
        if let Some((x, z)) = walk {
            // Straight at it, and stop within a stride. Nothing clever: the
            // recorder's position only decides what the tape can see.
            let [ex, _, ez] = session.core.eye_position();
            let (dx, dz) = (x - ex, z - ez);
            let far = dx * dx + dz * dz > 1.5 * 1.5;
            let yaw = client::look::yaw_u16(dx.atan2(dz));
            let pitch = client::look::pitch_u8(0.0);
            session
                .core
                .set_input(0, yaw, pitch, 0, if far { 127 } else { 0 }, 0);
        }
        session.pump(dt_ms);
        let drained: Vec<_> = taken
            .lock()
            .map(|mut v| v.drain(..).collect())
            .unwrap_or_default();
        for (lane, bytes) in drained {
            film::write_entry(&mut file, clock_ms, lane, &bytes).unwrap_or_else(|e| die(e));
            counts[lane as usize] += 1;
        }
        clock_ms += dt_ms;
        if session.closed() {
            eprintln!("record: the shard hung up at {:.1} s", clock_ms / 1000.0);
            break;
        }
        if now.duration_since(reported) >= Duration::from_secs(15) {
            reported = now;
            let [x, y, z] = session.core.eye_position();
            println!(
                "record: {:>4.0} s · {} snapshots · {} events · standing at {x:.0},{y:.0},{z:.0}",
                clock_ms / 1000.0,
                counts[0],
                counts[1]
            );
        }
    }
    file.flush().unwrap_or_else(|e| die(e));
    let dropped = session.datagrams_dropped();
    println!(
        "record: {:.1} s taped to {out} — {} snapshots, {} events, {dropped} dropped",
        clock_ms / 1000.0,
        counts[0],
        counts[1]
    );
    if dropped > 0 {
        eprintln!(
            "record: snapshots were dropped before the tap saw them; the replay will stutter"
        );
    }
}

/// Print a tape's timeline: bodies every `every` seconds, charges and
/// removals as they happen. Positions are world metres, `x,z`.
fn scan(path: &str, every: f64) {
    let tape = std::fs::File::open(path)
        .and_then(|f| film::read(&mut std::io::BufReader::new(f)))
        .unwrap_or_else(|e| die(format!("{path}: {e}")));
    let end = tape.end_ms();
    let (mut session, mut replay) = Session::replay(tape);
    let step = client_core::clock::TICK_MS;
    let mut next_report = 0.0;
    let mut rs = client_core::interp::RemoteState::default();
    println!("scan: {path}, {:.1} s", end / 1000.0);
    while replay.clock_ms < end {
        while !replay.caught_up() {
            if replay.feed() == 0 {
                session.pump(0.0);
            }
        }
        session.pump(step);
        replay.clock_ms += step;
        let t = replay.clock_ms / 1000.0;
        let core = &mut session.core;
        if session.applied2 & client_core::core::APPLIED2_CHARGE != 0 {
            let (cx, cz, _, loc, _, fuse) = core.charge_placed;
            let (x, z) = sim_core::build::anchor(cx, cz, loc);
            println!(
                "{t:7.1}  CHARGE planted at {x:.0},{z:.0}, fuse {:.1} s",
                fuse as f64 / 30.0
            );
        }
        session.applied2 = 0;
        session.applied = 0;
        while let Some(r) = core.pop_removed() {
            let (x, z) = sim_core::build::anchor(r.cx, r.cz, r.loc);
            println!(
                "{t:7.1}  DOWN {} at {x:.0},{z:.0} level {}",
                if r.deploy { "deployable" } else { "piece" },
                r.level
            );
        }
        // The rest of the rings, emptied so they never back up.
        while core.pop_impact().is_some() {}
        while core.pop_placed().is_some() {}
        while core.pop_death().is_some() {}
        if t >= next_report {
            next_report += every;
            let at = core.render_tick();
            let mut people = Vec::new();
            let mut animals = 0;
            for id in core.interp.ids() {
                if id == core.player_id || !core.interp.sample(id, at, &mut rs) {
                    continue;
                }
                if sim_core::mob::slot_of_id(id).is_some() {
                    animals += 1;
                } else {
                    people.push(format!(
                        "{id}@{:.0},{:.0}{}",
                        rs.x,
                        rs.z,
                        if rs.dead { "†" } else { "" }
                    ));
                }
            }
            let [ex, _, ez] = core.eye_position();
            println!(
                "{t:7.1}  pieces {:3} · animals {animals} · recorder {ex:.0},{ez:.0} · {}",
                core.pieces.len(),
                people.join(" ")
            );
        }
    }
}
