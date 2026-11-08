//! The learned body (`server::body`) against torch: a tiny randomly-weighted
//! body and a window of real records, written with torch's answers by
//! `ops/ticks/export.py --fixture` (`fixtures/body-tiny.json`). The Rust
//! port must read the same inputs off the same record bytes and compute the
//! same logits at every position.

use server::body::{LearnedBody, Weights};
use server::ticks::column;
use sim_core::input::InputFrame;

fn fixture() -> serde_json::Value {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/body-tiny.json");
    serde_json::from_slice(&std::fs::read(path).expect("the fixture")).expect("JSON")
}

fn hex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

fn floats(v: &serde_json::Value) -> Vec<f32> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_f64().unwrap() as f32)
        .collect()
}

/// The frame a record says it sent.
fn sent(row: &[u8]) -> InputFrame {
    let at = |n: &str| column(n).unwrap().0;
    InputFrame {
        seq: 0,
        buttons: row[at("buttons")],
        yaw: u16::from_le_bytes([row[at("f_yaw")], row[at("f_yaw") + 1]]),
        pitch: row[at("f_pitch")],
        pitch_fine: row[at("f_pitch_fine")] as i8,
        move_x: row[at("f_move_x")] as i8,
        move_z: row[at("f_move_z")] as i8,
        sel: row[at("f_sel")],
    }
}

fn close(what: &str, got: &[f32], want: &[f32], tol: f32) {
    assert_eq!(got.len(), want.len(), "{what}: length");
    for (i, (g, w)) in got.iter().zip(want).enumerate() {
        assert!(
            (g - w).abs() <= tol * (1.0 + w.abs()),
            "{what}[{i}]: rust {g}, torch {w}"
        );
    }
}

#[test]
fn the_port_reads_and_computes_what_torch_does() {
    let v = fixture();
    let w = std::sync::Arc::new(Weights::from_json(&v).expect("the fixture's body loads"));
    let f = &v["fixture"];
    assert_eq!(
        f["row_bytes"].as_u64(),
        Some(server::ticks::row_bytes() as u64),
        "the fixture's records are this build's records"
    );
    let rows: Vec<Vec<u8>> = f["records"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| hex(r.as_str().unwrap()))
        .collect();
    let labels: Vec<Option<&str>> = f["goal_labels"]
        .as_array()
        .unwrap()
        .iter()
        .map(|g| g.as_str())
        .collect();
    let n = f["obs"].as_array().unwrap().len();
    assert_eq!(rows.len(), n + 2, "two records before the window");
    let mut body = LearnedBody::new(w.clone());
    let mut obs = vec![0.0; w.n_obs];
    let mut fin = vec![0.0; w.n_frame];
    for p in 0..n {
        let r = p + 2;
        w.observation(&rows[r], w.goal_index(labels[r]), &mut obs);
        close(&format!("obs@{p}"), &obs, &floats(&f["obs"][p]), 1e-5);
        let (last, before) = (sent(&rows[r - 1]), sent(&rows[r - 2]));
        w.frame_input(Some(&last), Some(&before), &mut fin);
        close(
            &format!("frame_prev@{p}"),
            &fin,
            &floats(&f["frame_prev"][p]),
            1e-5,
        );
        body.step(&obs, &fin);
        close(
            &format!("frame_logits@{p}"),
            &body.frame_logits,
            &floats(&f["frame_logits"][p]),
            1e-3,
        );
        close(
            &format!("plan_logits@{p}"),
            &body.plan_logits,
            &floats(&f["plan_logits"][p]),
            1e-3,
        );
    }
}

/// A full window starts again from its newest half: the position after it
/// is computed exactly as a fresh body fed that half and then it would.
#[test]
fn a_full_window_starts_again_from_its_newest_half() {
    let v = fixture();
    let w = std::sync::Arc::new(Weights::from_json(&v).unwrap());
    let f = &v["fixture"];
    let ctx = w.ctx;
    let inputs: Vec<(Vec<f32>, Vec<f32>)> = (0..=ctx)
        .map(|i| {
            let k = i % f["obs"].as_array().unwrap().len();
            let mut obs = floats(&f["obs"][k]);
            obs[w.goals.len() + 1] += i as f32 * 0.01; // distinct inputs
            (obs, floats(&f["frame_prev"][k]))
        })
        .collect();
    let mut long = LearnedBody::new(w.clone());
    for (o, fr) in &inputs {
        long.step(o, fr);
    }
    let mut fresh = LearnedBody::new(w.clone());
    for (o, fr) in &inputs[ctx - ctx / 2..] {
        fresh.step(o, fr);
    }
    assert_eq!(long.frame_logits, fresh.frame_logits);
    assert_eq!(long.plan_logits, fresh.plan_logits);
}
