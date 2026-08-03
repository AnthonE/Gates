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
    /// Where `content/*.toml` lives (CLAUDE.md wall 7). Default `content`
    /// resolves against the CWD, which the repo commands make the repo
    /// root. The shard binary refuses to boot on invalid content.
    pub content_dir: String,
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
            content_dir: "content".into(),
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
    let mut cert_pem: Option<String> = None;
    let mut key_pem: Option<String> = None;
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
            "content_dir" => {
                if value.is_empty() {
                    return Err(format!("shard.toml line {}: empty content_dir", n + 1));
                }
                content_dir = Some(value.to_string());
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
        content_dir: content_dir.unwrap_or_else(|| "content".into()),
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
