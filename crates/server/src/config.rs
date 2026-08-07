//! Shard boot config, read from `shard.toml` (CLAUDE.md commands). Parsed
//! by hand — three keys don't earn a serde dependency; unknown keys are
//! refused so a typo can't silently run defaults.

use std::net::SocketAddr;

#[derive(Clone, Debug)]
pub struct ShardConfig {
    /// UDP bind address. Port 0 binds ephemeral (the test path).
    pub bind: SocketAddr,
    /// World seed — the whole island derives from it (TERRAIN.md §0).
    pub seed: u64,
    /// Dev-only fixed spawn `"x,z"` in meters (DECISIONS.md §open row
    /// "dev spawn override"). Unset is the shipping default; never set it
    /// on a public shard — every joiner lands on the same point.
    pub dev_spawn: Option<(f32, f32)>,
    /// TLS identity for a PUBLIC shard: paths to a real certificate chain
    /// and its private key. Both or neither — set, the shard serves that
    /// identity and browsers trust it outright (no `serverCertificateHashes`,
    /// which is the dev flow and needs a short-lived cert). Unset is the
    /// shipping default and self-signs for loopback, which is what every
    /// test and the local dev flow use.
    pub cert_pem: Option<String>,
    pub key_pem: Option<String>,
    /// Whether a session token is REQUIRED to join.
    ///
    /// `false` (the shipping default) is a shard that takes guests: a player
    /// with no launcher plays anyway, which is the same posture
    /// `scry::Player::Anonymous` has always taken on the client. `true`
    /// refuses a joiner whose token is absent or not good, with
    /// `REFUSE_AUTH`.
    ///
    /// **It is a knob and not a wall because a shard's admission policy is
    /// the operator's**, and the two real cases both exist: a public armed
    /// shard wants everyone identified, and a local dev shard must be
    /// joinable with nothing running but the binary. Every test in this repo
    /// depends on the second.
    /// Proposed default `false`, DECISIONS.md §open ("scry session auth v0").
    pub require_auth: bool,
    /// Where `content/*.toml` lives (CLAUDE.md wall 7). Default `content`
    /// resolves against the CWD, which the repo commands make the repo
    /// root. The shard binary refuses to boot on invalid content.
    pub content_dir: String,
    /// The player save file (`store.rs`), or `None` for **a shard that
    /// remembers nothing past its own process**.
    ///
    /// Unset is the shipping default, and with `require_auth` also at its
    /// default it is today's behaviour exactly: no identity, so no key, so
    /// every join builds a fresh character. That is what keeps every test in
    /// this repo hermetic — no test writes a file it did not ask for — and it
    /// is why arming persistence is one deliberate line rather than something
    /// a shard does to its working directory by surprise.
    ///
    /// **Precisely, because the middle case is real:** the store's index is
    /// in memory and the file is only how it survives a restart. So a shard
    /// with `require_auth = true` and no `save_file` remembers a player for
    /// the life of its process — a reconnect after a network blip keeps your
    /// inventory, a restart does not. That is a strictly better default than
    /// "forget on every disconnect" and it costs nothing, but it is not
    /// persistence, and an operator who wants a base to outlive a deploy sets
    /// this key.
    ///
    /// Set, the file is created if absent and **validated against this
    /// shard's seed and content hash at boot**: a mismatch refuses to boot
    /// rather than handing a hundred players an empty inventory. See
    /// `store::open` for the refusals and what each one asks the operator to
    /// do about it.
    ///
    /// Persistence also needs an *identity* to file a save under, which is
    /// `require_auth`'s business and not this knob's: a guest is admitted and
    /// remembered by nobody. So a shard that wants players to keep their
    /// things sets both.
    /// Proposed default `None`, DECISIONS.md §open ("player persistence v0").
    pub save_file: Option<String>,
}

impl ShardConfig {
    /// Loopback + ephemeral port: what tests and local bots use.
    pub fn ephemeral(seed: u64) -> Self {
        Self {
            bind: "127.0.0.1:0".parse().expect("static addr"),
            seed,
            dev_spawn: None,
            cert_pem: None,
            key_pem: None,
            require_auth: false,
            content_dir: "content".into(),
            save_file: None,
        }
    }
}

/// Parse `key = value` lines; `#` comments and blanks skipped; string
/// values may be double-quoted. Refuses unknown keys, missing keys, and
/// unparseable values.
pub fn parse_shard_toml(text: &str) -> Result<ShardConfig, String> {
    let mut bind: Option<SocketAddr> = None;
    let mut seed: Option<u64> = None;
    let mut dev_spawn: Option<(f32, f32)> = None;
    let mut content_dir: Option<String> = None;
    let mut require_auth: Option<bool> = None;
    let mut cert_pem: Option<String> = None;
    let mut key_pem: Option<String> = None;
    let mut save_file: Option<String> = None;
    for (n, line) in text.lines().enumerate() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let (key, value) = line
            .split_once('=')
            .ok_or_else(|| format!("shard.toml line {}: expected key = value", n + 1))?;
        let key = key.trim();
        let value = value.trim().trim_matches('"');
        match key {
            "bind" => {
                bind = Some(
                    value
                        .parse()
                        .map_err(|e| format!("shard.toml line {}: bad bind: {e}", n + 1))?,
                );
            }
            "seed" => {
                seed = Some(
                    value
                        .parse()
                        .map_err(|e| format!("shard.toml line {}: bad seed: {e}", n + 1))?,
                );
            }
            "dev_spawn" => {
                let (x, z) = value
                    .split_once(',')
                    .ok_or_else(|| format!("shard.toml line {}: dev_spawn wants \"x,z\"", n + 1))?;
                let x: f32 = x
                    .trim()
                    .parse()
                    .map_err(|e| format!("shard.toml line {}: bad dev_spawn x: {e}", n + 1))?;
                let z: f32 = z
                    .trim()
                    .parse()
                    .map_err(|e| format!("shard.toml line {}: bad dev_spawn z: {e}", n + 1))?;
                let island = sim_core::terrain::ISLAND_SIZE;
                if !(x.is_finite()
                    && z.is_finite()
                    && (0.0..=island).contains(&x)
                    && (0.0..=island).contains(&z))
                {
                    return Err(format!(
                        "shard.toml line {}: dev_spawn ({x},{z}) outside the {island} m island",
                        n + 1
                    ));
                }
                dev_spawn = Some((x, z));
            }
            "cert_pem" | "key_pem" => {
                if value.is_empty() {
                    return Err(format!("shard.toml line {}: empty {key}", n + 1));
                }
                if key == "cert_pem" {
                    cert_pem = Some(value.to_string());
                } else {
                    key_pem = Some(value.to_string());
                }
            }
            "require_auth" => match value {
                "true" => require_auth = Some(true),
                "false" => require_auth = Some(false),
                // Refused rather than coerced: a shard that read `yes` as
                // false would take guests while its operator believed it
                // did not, and admission policy is the one setting where a
                // silent default is a security posture nobody chose.
                other => {
                    return Err(format!(
                        "shard.toml line {}: require_auth must be true or false, got `{other}`",
                        n + 1
                    ))
                }
            },
            "content_dir" => {
                if value.is_empty() {
                    return Err(format!("shard.toml line {}: empty content_dir", n + 1));
                }
                content_dir = Some(value.to_string());
            }
            // Empty is refused rather than read as "off": a line the operator
            // wrote is a line they meant, and `save_file = ""` would be a
            // shard that silently remembers nobody while its config says it
            // should. Off is the key being absent.
            "save_file" => {
                if value.is_empty() {
                    return Err(format!(
                        "shard.toml line {}: empty save_file — omit the key to run \
                         without persistence, which is the default",
                        n + 1
                    ));
                }
                save_file = Some(value.to_string());
            }
            other => return Err(format!("shard.toml line {}: unknown key `{other}`", n + 1)),
        }
    }
    // Both or neither: half a TLS identity is a shard that self-signs while
    // its operator believes it is public.
    if cert_pem.is_some() != key_pem.is_some() {
        return Err("shard.toml: cert_pem and key_pem must be set together".into());
    }
    Ok(ShardConfig {
        bind: bind.ok_or("shard.toml: missing `bind`")?,
        seed: seed.ok_or("shard.toml: missing `seed`")?,
        dev_spawn,
        cert_pem,
        key_pem,
        require_auth: require_auth.unwrap_or(false),
        content_dir: content_dir.unwrap_or_else(|| "content".into()),
        save_file,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_refuses() {
        let cfg = parse_shard_toml("# shard\nbind = \"127.0.0.1:4433\"\nseed = 7\n").unwrap();
        assert_eq!(cfg.bind.port(), 4433);
        assert_eq!(cfg.seed, 7);
        assert_eq!(cfg.dev_spawn, None);
        assert_eq!(cfg.content_dir, "content");
        let cfg = parse_shard_toml(
            "bind = \"127.0.0.1:1\"\nseed = 7\ncontent_dir = \"/srv/gates/content\"\n",
        )
        .unwrap();
        assert_eq!(cfg.content_dir, "/srv/gates/content");
        assert!(
            parse_shard_toml("bind = \"127.0.0.1:1\"\nseed = 7\ncontent_dir = \"\"\n").is_err()
        );
        assert!(parse_shard_toml("bind = \"127.0.0.1:1\"").is_err()); // missing seed
        assert!(parse_shard_toml("bind = \"127.0.0.1:1\"\nseed = 1\nwat = 2").is_err());
        // TLS identity: both or neither, and neither may be empty.
        let base = "bind = \"127.0.0.1:1\"\nseed = 1\n";
        let pub_cfg = parse_shard_toml(&format!(
            "{base}cert_pem = \"/c/fullchain.pem\"\nkey_pem = \"/c/privkey.pem\"\n"
        ))
        .unwrap();
        assert_eq!(pub_cfg.cert_pem.as_deref(), Some("/c/fullchain.pem"));
        assert_eq!(pub_cfg.key_pem.as_deref(), Some("/c/privkey.pem"));
        assert!(parse_shard_toml(&format!("{base}cert_pem = \"/c/f.pem\"\n")).is_err());
        assert!(parse_shard_toml(&format!("{base}key_pem = \"/c/k.pem\"\n")).is_err());
        assert!(parse_shard_toml(&format!("{base}cert_pem = \"\"\nkey_pem = \"x\"\n")).is_err());
    }

    #[test]
    fn dev_spawn_parses_and_refuses() {
        let base = "bind = \"127.0.0.1:1\"\nseed = 1\n";
        let ok = parse_shard_toml(&format!("{base}dev_spawn = \"1024, 1024\"\n")).unwrap();
        assert_eq!(ok.dev_spawn, Some((1024.0, 1024.0)));
        for bad in [
            "\"1024\"",
            "\"1024,\"",
            "\"-1,5\"",
            "\"9999,5\"",
            "\"nan,5\"",
        ] {
            assert!(
                parse_shard_toml(&format!("{base}dev_spawn = {bad}\n")).is_err(),
                "accepted dev_spawn = {bad}"
            );
        }
    }
}

#[cfg(test)]
mod auth_cfg_tests {
    use super::*;

    /// The default is guests-welcome, and that is load-bearing: every test
    /// in this repo joins a shard with no launcher running.
    #[test]
    fn require_auth_defaults_off() {
        assert!(!ShardConfig::ephemeral(1).require_auth);
        let cfg = parse_shard_toml("bind = \"127.0.0.1:1\"\nseed = 7\n").expect("parses");
        assert!(!cfg.require_auth);
    }

    #[test]
    fn require_auth_reads_both_ways() {
        let on = parse_shard_toml("bind = \"127.0.0.1:1\"\nseed = 7\nrequire_auth = true\n")
            .expect("parses");
        assert!(on.require_auth);
        let off = parse_shard_toml("bind = \"127.0.0.1:1\"\nseed = 7\nrequire_auth = false\n")
            .expect("parses");
        assert!(!off.require_auth);
    }

    /// Anything else is refused, not coerced. A shard that read `yes` as
    /// false would take guests while its operator believed it did not —
    /// admission is the one setting where a silent default is a posture
    /// nobody chose.
    #[test]
    fn a_fuzzy_require_auth_is_refused() {
        for bad in ["yes", "1", "on", "True", ""] {
            let src = alloc_line(bad);
            assert!(
                parse_shard_toml(&src).is_err(),
                "require_auth = `{bad}` must be refused, not coerced"
            );
        }
    }

    /// `format!` is disallowed in this crate; build the line by hand.
    fn alloc_line(v: &str) -> std::string::String {
        let mut s = std::string::String::from("bind = \"127.0.0.1:1\"\nseed = 7\nrequire_auth = ");
        s.push_str(v);
        s.push('\n');
        s
    }
}
