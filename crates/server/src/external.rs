//! Bring your own agent: a decision source that speaks newline-delimited
//! JSON with a **child process** over its stdin and stdout.
//!
//! Why a child's pipes rather than a loopback socket: nothing else on the
//! machine can connect to them, there is no port to allocate or guard, the
//! agent lives and dies with the bot, the bot's own stdout stays free for
//! its report, and any language that can read a line and print a line can
//! play. The child inherits no Jev key (`TYPESAFE_API_KEY` is removed from
//! its environment).
//!
//! One observation line out per request, one goal line back:
//!
//! ```text
//! -> {"protocol":1,"id":7,"observation":{...},"options":["explore","gather_wood",...],"heard":[...]}
//! <- {"id":7,"goal":"gather_wood","reason":"need wood for a hatchet","say":"anyone got stone?"}
//! ```
//!
//! `heard` is the chat its client received since the last request — words
//! strangers typed, so data and never instructions. `say` is optional: one
//! local line, through the same chat rule as the human chat box. A line the
//! rule refuses (over `CHAT_MAX_BYTES`, a control character, a `/command`)
//! is counted and dropped; the goal still stands. `code` is optional: the
//! team's lock code, as a teammate gave it, for the locks on the way to the
//! team's chest.
//!
//! The same rules as Jev: one request outstanding, the reply must echo the
//! id and name an offered goal, lines are bounded, a missing or late answer
//! is an explicit failure and never a substituted decision. Replies to an
//! older id are skipped. `JEV.md` has the schema.

use crate::agent::lock::LockCode;
use crate::mind::{Choice, DecisionSource, Goal, Reason, Say, SourceKind, Summary};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::time::{Duration, Instant};

/// The longest reply line accepted. Proposed default.
pub const EXTERNAL_MAX_LINE_BYTES: usize = 8192;
/// How often a waiting side looks at the line ring. Plumbing; proposed.
pub const EXTERNAL_POLL_MS: u64 = 2;
const POLL: Duration = Duration::from_millis(EXTERNAL_POLL_MS);
/// Protocol version carried on every observation line.
pub const EXTERNAL_PROTOCOL: u32 = 1;

enum Line {
    Text(String),
    TooLong,
}

pub struct External {
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    lines: Option<rtrb::Consumer<Line>>,
    reader: Option<std::thread::JoinHandle<()>>,
    timeout: Duration,
    next_id: u64,
    /// Replies that named an older request and were skipped.
    pub stale: u64,
}

impl External {
    /// Start `program` with `args`. The child's stderr passes through.
    pub fn spawn(program: &str, args: &[String], timeout: Duration) -> Result<Self, String> {
        Self::spawn_with_env(program, args, &[], timeout)
    }

    /// [`Self::spawn`], with `env` set for the child beside what it
    /// inherits: how `jev-watch` tells an agent where its rendered view is
    /// (`GATES_FRAME_URL`) without a field in the protocol.
    pub fn spawn_with_env(
        program: &str,
        args: &[String],
        env: &[(&str, String)],
        timeout: Duration,
    ) -> Result<Self, String> {
        if timeout.is_zero() {
            return Err("the external agent needs a positive timeout".into());
        }
        let mut child = Command::new(program)
            .args(args)
            .env_remove("TYPESAFE_API_KEY")
            .envs(env.iter().map(|(k, v)| (k, v)))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| format!("external agent {program}: {e}"))?;
        let stdout = child.stdout.take().ok_or("external agent has no stdout")?;
        let stdin = child.stdin.take().ok_or("external agent has no stdin")?;
        Self::over(Some(child), stdin, stdout, timeout)
    }

    fn over(
        child: Option<Child>,
        stdin: ChildStdin,
        stdout: impl Read + Send + 'static,
        timeout: Duration,
    ) -> Result<Self, String> {
        // A small bounded ring: the reader waits rather than buffering an
        // agent that talks without being asked (rings, not channels, per
        // this crate's clippy walls).
        let (mut tx, lines) = rtrb::RingBuffer::new(4);
        let reader = std::thread::Builder::new()
            .name("external-agent-reader".into())
            .spawn(move || {
                let mut reader = BufReader::new(stdout);
                loop {
                    let mut buf = Vec::new();
                    let limit = EXTERNAL_MAX_LINE_BYTES as u64 + 1;
                    match (&mut reader).take(limit).read_until(b'\n', &mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {}
                    }
                    let line = if buf.last() != Some(&b'\n') && buf.len() as u64 == limit {
                        // Discard the rest of the oversize line.
                        let mut rest = Vec::new();
                        if reader.read_until(b'\n', &mut rest).is_err() {
                            break;
                        }
                        Line::TooLong
                    } else {
                        match String::from_utf8(buf) {
                            Ok(s) => Line::Text(s),
                            Err(_) => Line::TooLong,
                        }
                    };
                    let mut line = line;
                    loop {
                        match tx.push(line) {
                            Ok(()) => break,
                            Err(rtrb::PushError::Full(back)) if !tx.is_abandoned() => {
                                line = back;
                                std::thread::sleep(POLL);
                            }
                            Err(_) => return,
                        }
                    }
                }
            })
            .map_err(|e| format!("external agent reader: {e}"))?;
        Ok(Self {
            child,
            stdin: Some(stdin),
            lines: Some(lines),
            reader: Some(reader),
            timeout,
            next_id: 1,
            stale: 0,
        })
    }

    pub(crate) fn observation(id: u64, summary: &Summary) -> Value {
        json!({
            "protocol": EXTERNAL_PROTOCOL,
            "id": id,
            "observation": summary.to_json(),
            "options": summary.options().iter().map(|g| g.label().as_str().to_owned()).collect::<Vec<_>>(),
            "heard": summary.heard_json(),
            "team": summary.team_json(),
        })
    }

    /// A reply's `say`: missing, null or blank is quiet; a string the chat
    /// rule takes (and that is not a `/command` for the shard) is a line;
    /// any other string is refused rather than repaired. `to` is `"team"`
    /// for the team channel; missing, null or `"local"` is local.
    fn say(value: &Value, to: &Value) -> Result<Say, String> {
        let team = match to {
            Value::Null => false,
            Value::String(t) if t == "local" => false,
            Value::String(t) if t == "team" => true,
            _ => return Err("external agent to must be \"local\" or \"team\"".into()),
        };
        match value {
            Value::Null => Ok(Say::Quiet),
            Value::String(text) if text.trim().is_empty() => Ok(Say::Quiet),
            Value::String(text) => Ok(match protocol::ChatText::sanitize(text.as_bytes()) {
                Some(line) if !protocol::admin::is_command(&line) => {
                    if team {
                        Say::Team(line)
                    } else {
                        Say::Line(line)
                    }
                }
                _ => Say::Refused,
            }),
            _ => Err("external agent say must be a string".into()),
        }
    }

    /// Parse one reply for request `id`. `Ok(None)` is a reply to an older
    /// request, which is skipped rather than failed.
    pub(crate) fn reply(line: &str, id: u64, summary: &Summary) -> Result<Option<Choice>, String> {
        let value: Value = serde_json::from_str(line.trim_end())
            .map_err(|_| "external agent sent invalid JSON")?;
        let got = value["id"]
            .as_u64()
            .ok_or("external agent reply has no numeric id")?;
        if got < id {
            return Ok(None);
        }
        if got != id {
            return Err("external agent answered a request it was not sent".into());
        }
        let label = value["goal"]
            .as_str()
            .ok_or("external agent reply has no goal")?;
        let goal = Goal::parse(label, summary.options())
            .ok_or("external agent chose a goal it was not offered")?;
        let reason = match &value["reason"] {
            Value::Null => Reason::from_text("external agent gave no reason"),
            Value::String(text) => Reason::from_text(text),
            _ => return Err("external agent reason must be a string".into()),
        };
        let say = Self::say(&value["say"], &value["to"])?;
        let code = Self::code(&value["code"])?;
        Ok(Some(Choice {
            goal,
            confidence: 1.0,
            reason,
            input_tokens: 0,
            output_tokens: 0,
            say,
            code,
        }))
    }

    /// A reply's `code`: the team's lock code as a teammate gave it, one
    /// to four digits as a string (`"0420"`) or a number. Missing or null
    /// is none; anything else is a bad reply.
    fn code(value: &Value) -> Result<Option<LockCode>, String> {
        let bad = || "external agent code must be four digits".to_string();
        let n = match value {
            Value::Null => return Ok(None),
            Value::String(t) => {
                let t = t.trim();
                if t.is_empty() || t.len() > 4 || !t.bytes().all(|b| b.is_ascii_digit()) {
                    return Err(bad());
                }
                t.parse::<u16>().map_err(|_| bad())?
            }
            Value::Number(n) => n
                .as_u64()
                .and_then(|n| u16::try_from(n).ok())
                .ok_or_else(bad)?,
            _ => return Err(bad()),
        };
        LockCode::given(n).map(Some).ok_or_else(bad)
    }
}

impl DecisionSource for External {
    fn kind(&self) -> SourceKind {
        SourceKind::External
    }

    fn decide(&mut self, summary: &Summary) -> Result<Choice, String> {
        let id = self.next_id;
        self.next_id += 1;
        let deadline = Instant::now() + self.timeout;
        let mut line = Self::observation(id, summary).to_string();
        line.push('\n');
        let stdin = self
            .stdin
            .as_mut()
            .ok_or("external agent input is closed")?;
        if stdin
            .write_all(line.as_bytes())
            .and_then(|_| stdin.flush())
            .is_err()
        {
            self.stdin = None;
            return Err("external agent closed its input".into());
        }
        let lines = self
            .lines
            .as_mut()
            .ok_or("external agent output is closed")?;
        wait(lines, id, deadline, summary, &mut self.stale)
    }
}

/// Read replies until the one for `id`, skipping older ones, or fail.
fn wait(
    lines: &mut rtrb::Consumer<Line>,
    id: u64,
    deadline: Instant,
    summary: &Summary,
    stale: &mut u64,
) -> Result<Choice, String> {
    loop {
        match lines.pop() {
            Ok(Line::Text(text)) => match External::reply(&text, id, summary)? {
                Some(choice) => return Ok(choice),
                None => *stale += 1,
            },
            Ok(Line::TooLong) => {
                return Err("external agent reply over the line limit or not UTF-8".into())
            }
            Err(_) if lines.is_abandoned() => return Err("external agent closed its output".into()),
            Err(_) if Instant::now() >= deadline => {
                return Err("external agent did not answer in time".into())
            }
            Err(_) => std::thread::sleep(POLL),
        }
    }
}

impl Drop for External {
    fn drop(&mut self) {
        // Close both ends first so a reader blocked on a full queue or an
        // open pipe can finish, then stop the child and join.
        self.stdin.take();
        self.lines.take();
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mind::Name;

    fn summary() -> Summary {
        let mut s = Summary::EMPTY;
        for g in Goal::FIXED {
            s.offer(g);
        }
        s.offer(Goal::Craft(Name::new(b"Stone Pickaxe").unwrap()));
        s
    }

    fn sh(script: &str) -> External {
        External::spawn(
            "/bin/sh",
            &["-c".to_owned(), script.to_owned()],
            Duration::from_secs(5),
        )
        .unwrap()
    }

    #[test]
    fn replies_are_strict_and_older_ids_are_skipped() {
        let s = summary();
        let ok = External::reply(
            r#"{"id":3,"goal":"craft:Stone Pickaxe","reason":"tool\nnow"}"#,
            3,
            &s,
        )
        .unwrap()
        .unwrap();
        assert_eq!(ok.goal, Goal::Craft(Name::new(b"Stone Pickaxe").unwrap()));
        assert_eq!(ok.reason.as_str(), "tool now");
        assert!(External::reply(r#"{"id":2,"goal":"explore"}"#, 3, &s)
            .unwrap()
            .is_none());
        for bad in [
            "not json",
            r#"{"goal":"explore"}"#,
            r#"{"id":4,"goal":"explore"}"#,
            r#"{"id":3,"goal":"teleport"}"#,
            r#"{"id":3,"goal":"craft:Metal Pickaxe"}"#,
            r#"{"id":3,"goal":"explore","reason":7}"#,
        ] {
            assert!(External::reply(bad, 3, &s).is_err(), "{bad}");
        }
        let say = |text: &str| {
            let line = format!(r#"{{"id":3,"goal":"explore","say":{text}}}"#);
            External::reply(&line, 3, &s).map(|c| c.unwrap().say)
        };
        let hello = protocol::ChatText::sanitize(b"hello there").unwrap();
        assert_eq!(say(r#""  hello there ""#), Ok(Say::Line(hello)));
        assert_eq!(say(r#""""#), Ok(Say::Quiet));
        assert_eq!(say("null"), Ok(Say::Quiet));
        let long = format!("\"{}\"", "x".repeat(protocol::CHAT_MAX_BYTES + 1));
        for refused in [long.as_str(), r#""two\nlines""#, r#""/kick 1""#] {
            assert_eq!(say(refused), Ok(Say::Refused), "{refused}");
        }
        assert!(say("7").is_err(), "a non-string say is a bad reply");
        let to = |to: &str| {
            let line = format!(r#"{{"id":3,"goal":"explore","say":"on my way","to":{to}}}"#);
            External::reply(&line, 3, &s).map(|c| c.unwrap().say)
        };
        let omw = protocol::ChatText::sanitize(b"on my way").unwrap();
        assert_eq!(to(r#""team""#), Ok(Say::Team(omw)));
        assert_eq!(to(r#""local""#), Ok(Say::Line(omw)));
        assert_eq!(to("null"), Ok(Say::Line(omw)));
        assert!(to(r#""global""#).is_err(), "an agent has no global voice");
        let code = |code: &str| {
            let line = format!(r#"{{"id":3,"goal":"explore","code":{code}}}"#);
            External::reply(&line, 3, &s).map(|c| c.unwrap().code)
        };
        let given = |n| Ok(crate::agent::lock::LockCode::given(n));
        assert_eq!(code(r#""0420""#), given(420));
        assert_eq!(code("9999"), given(9999));
        assert_eq!(code("null"), Ok(None));
        for bad in [r#""12345""#, r#""12a4""#, r#""""#, "10000", "-1", "1.5"] {
            assert!(code(bad).is_err(), "{bad}");
        }
        let observation = External::observation(9, &s);
        assert_eq!(observation["heard"], serde_json::json!([]));
        assert_eq!(observation["protocol"], EXTERNAL_PROTOCOL);
        assert_eq!(
            observation["options"].as_array().unwrap().len(),
            s.options().len()
        );
    }

    #[test]
    fn a_child_process_answers_over_its_pipes() {
        // A reply to an older request first, then the real one.
        let mut agent = sh(r#"while read line; do
            id=$(printf '%s' "$line" | sed 's/.*"id":\([0-9]*\).*/\1/')
            printf '{"id":0,"goal":"wait"}\n{"id":%s,"goal":"gather_stone","reason":"sh"}\n' "$id"
        done"#);
        let s = summary();
        for _ in 0..2 {
            let choice = agent.decide(&s).unwrap();
            assert_eq!(choice.goal, Goal::GatherStone);
            assert_eq!(choice.reason.as_str(), "sh");
        }
        assert_eq!(agent.stale, 2);
    }

    #[test]
    fn a_child_is_told_where_its_view_is() {
        let mut agent = External::spawn_with_env(
            "/bin/sh",
            &[
                "-c".to_owned(),
                r#"read line; id=$(printf '%s' "$line" | sed 's/.*"id":\([0-9]*\).*/\1/')
                printf '{"id":%s,"goal":"wait","reason":"%s"}\n' "$id" "$GATES_FRAME_URL""#
                    .to_owned(),
            ],
            &[(
                "GATES_FRAME_URL",
                "http://127.0.0.1:8081/frame.jpg".to_owned(),
            )],
            Duration::from_secs(5),
        )
        .unwrap();
        let choice = agent.decide(&summary()).unwrap();
        assert_eq!(choice.reason.as_str(), "http://127.0.0.1:8081/frame.jpg");
    }

    #[test]
    fn silence_oversize_lines_and_exit_are_explicit_failures() {
        let s = summary();
        let mut quiet = External::spawn(
            "/bin/sh",
            &["-c".to_owned(), "sleep 5".to_owned()],
            Duration::from_millis(200),
        )
        .unwrap();
        assert!(quiet.decide(&s).unwrap_err().contains("in time"));
        let mut long = sh(&format!(
            "read line; head -c {} /dev/zero | tr '\\0' x; echo",
            EXTERNAL_MAX_LINE_BYTES + 10
        ));
        assert!(long.decide(&s).unwrap_err().contains("line limit"));
        let mut gone = sh("exit 0");
        assert!(gone.decide(&s).is_err());
        assert!(External::spawn("/nonexistent/agent", &[], Duration::from_secs(1)).is_err());
    }
}
