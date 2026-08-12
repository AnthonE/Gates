//! Shard boot config, read from `shard.toml` (CLAUDE.md commands). Parsed
//! by hand — three keys don't earn a serde dependency; unknown keys are
//! refused so a typo can't silently run defaults.

use std::net::SocketAddr;

/// How often the world is written, in ticks — 60 s at 30 Hz.
///
/// Chosen against what a crash costs rather than against what a save
/// costs, which is the inversion `reference/SAVES.md` §4 makes available:
/// their knob's other end is a shard-wide freeze, so they cannot go below
/// ten minutes. Ours has no freeze to trade against — the encode is a
/// bounded pass on the sim thread and the write is on the store thread — so
/// the only cost of a shorter interval is disk, and the only cost of a
/// longer one is a player's hour.
///
/// Proposed default, DECISIONS.md §open ("world persistence v0").
pub const DEFAULT_WORLD_SAVE_INTERVAL_TICKS: u64 = 1_800;

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
    /// Whether a proven identity is REQUIRED to join.
    ///
    /// `false` (the shipping default) is a shard that takes guests: a player
    /// with no wallet and no launcher plays anyway, which is the same posture
    /// `scry::Player::Anonymous` has always taken on the client. `true`
    /// refuses a joiner who offers no address, or whose SIWE signature does
    /// not verify, with `REFUSE_AUTH` (`auth::verify`).
    ///
    /// **It is a knob and not a wall because a shard's admission policy is
    /// the operator's**, and the two real cases both exist: a public armed
    /// shard wants everyone identified, and a local dev shard must be
    /// joinable with nothing running but the binary. Every test in this repo
    /// depends on the second.
    /// Default `false`, DECISIONS.md §open ("siwe identity v1").
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
    /// Path to the **world** file: bases, boxes, bags, fuses, stumps, and
    /// the bodies standing in it. Unset ⇒ the world is generated fresh from
    /// the seed on every boot, which is what every shard did before world
    /// persistence and what every test still runs.
    ///
    /// **A different key from `save_file`, deliberately** — that is
    /// `reference/SAVES.md` §5's split and it is the one that matters at
    /// wipe time: a wipe destroys the world and keeps the player store
    /// (which is where blueprints will live), so they have to be two files
    /// an operator can delete independently. Setting one without the other
    /// is legal and both halves are useful alone: a world with no player
    /// store forgets who you are but keeps your base standing as a sleeper
    /// you can walk back into; a player store with no world hands you your
    /// inventory on a fresh island.
    ///
    /// Proposed default `None`, DECISIONS.md §open ("world persistence v0").
    pub world_file: Option<String>,
    /// How often the world is written, in ticks. The reference game's
    /// `server.saveinterval` is 600 s and its two ends are "how much a crash
    /// costs" against "how often everyone freezes" (`reference/SAVES.md`
    /// §4) — ours has no second end, because the walk is off the sim thread
    /// and the write is off it again, so this trades crash cost against
    /// nothing but disk writes.
    ///
    /// Proposed default 1800 ticks = 60 s at 30 Hz, DECISIONS.md §open
    /// ("world persistence v0").
    pub world_save_interval_ticks: u64,
    /// Where the status endpoint listens (`status.rs`): a plain HTTP
    /// responder answering `GET /status.json` with integers read off
    /// `ShardStats` atomics — `players`, `max_players`, `tick` — on its own
    /// thread, never touching the sim. It is what lights the shard list's
    /// `players` column (`ci/shardlist.py` is the eventual consumer).
    ///
    /// Unset is the shipping default and serves **nothing**, which is the
    /// honest state: a live shard changes nothing until its operator says
    /// where to listen, and publishing the resulting URL anywhere is a
    /// separate operator act again. Port 0 binds ephemeral (the test path,
    /// same as `bind`). DECISIONS.md §open ("shard status endpoint v0").
    pub status_addr: Option<SocketAddr>,
    /// The shard's name for SIWE domain binding — what goes in the signed
    /// message, and what a player sees in their wallet prompt.
    ///
    /// **It must be the host the client dialled**, because that is the whole
    /// of the domain binding: the client builds the message from the address
    /// it connected to and the server from this key, so a signature
    /// collected by `evil.example` cannot be presented at `gates.example` —
    /// the two messages differ and the recovered signer will not match.
    /// Defaults to the bind host, which is right for a dev shard and wrong
    /// for anything behind a name, so a public shard sets it.
    pub domain: String,
    /// The ticket door (`entitle.rs`): does a joiner own a copy of this game?
    ///
    /// [`entitle::Config::off`] is the shipping default and checks **nothing**
    /// — the same honest state `status_addr` takes. Community and training
    /// shards run this way on purpose (`DECISIONS.md` 2026-08-04: one build,
    /// two populations — the armed set is the perimeter), and so does every
    /// test in this repo.
    ///
    /// Armed, it requires `require_auth = true`, because a guest has no wallet
    /// to ask about; `parse_shard_toml` refuses the pair rather than warning.
    pub entitle: crate::entitle::Config,
    /// The oldest client **release** this shard will admit, packed by
    /// [`protocol::version::pack`]. A joiner below it meets `REFUSE_BUILD`.
    ///
    /// **Zero is the shipping default and admits everything**, which is the
    /// same honest state `status_addr` and `entitle` take: a shard requires
    /// nothing until its operator says so. That is not a hole — `PROTO_VER`
    /// is still exact and still refuses any client whose bytes disagree, so
    /// the floor at 0 means "any client that can talk to me may", not "any
    /// client at all".
    ///
    /// It exists for the case the protocol number cannot see: a release that
    /// parses fine and is wrong about something that is not a byte — a
    /// prediction rule that moved, a number that changed in `content/`. The
    /// operator raises the floor when they ship one of those, which is a
    /// judgement about a release and belongs in a config file rather than in
    /// a constant somebody has to rebuild to change.
    ///
    /// Written in `shard.toml` as a semver string (`min_client = "0.1.0"`),
    /// because a packed integer in a config file is a number nobody can
    /// check by eye.
    pub min_client: u32,
    /// Wallets trusted with the admin lane (`admin_wallets`, admin v0).
    /// **Empty by default: nobody is an admin** — `entitle.rs`'s
    /// one-build-two-populations posture applied to privilege.
    pub admins: crate::admin::Admins,
    /// Where the anomaly log is appended (`anomaly_file`). `None` ⇒ no log
    /// and every push is a no-op, which is what a test shard runs.
    pub anomaly_file: Option<String>,
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
            world_file: None,
            world_save_interval_ticks: DEFAULT_WORLD_SAVE_INTERVAL_TICKS,
            status_addr: None,
            domain: "127.0.0.1".into(),
            entitle: crate::entitle::Config::off(),
            min_client: 0,
            admins: crate::admin::Admins::none(),
            anomaly_file: None,
        }
    }
}

/// `"0.1.0"` → the packed [`protocol::version::VER`] form. Rejects anything
/// that is not three plain numbers, and rejects a component ≥ 1000 with the
/// reason rather than wrapping it into the field above — the packing's own
/// cap, restated at the one place an operator types a version by hand.
///
/// A prerelease suffix is refused rather than ignored: `min_client =
/// "0.2.0-rc1"` looks like it means something and the decimal packing cannot
/// express it (`protocol::version`'s header says why), so accepting it would
/// silently apply a floor of 0.2.0 — the ignored-field-becomes-a-supported-one
/// failure, arriving as a shard admitting builds its operator meant to refuse.
pub fn parse_min_client(s: &str) -> Result<u32, String> {
    let mut parts = s.split('.');
    let mut got = [0u32; 3];
    for (i, name) in ["major", "minor", "patch"].iter().enumerate() {
        let raw = parts.next().ok_or_else(|| {
            format!("`{s}` is not a version — want three numbers, e.g. \"0.1.0\"")
        })?;
        got[i] = raw
            .parse::<u32>()
            .map_err(|_| format!("`{s}`: the {name} part `{raw}` is not a plain number"))?;
    }
    if parts.next().is_some() {
        return Err(format!(
            "`{s}` has more than three parts — want e.g. \"0.1.0\""
        ));
    }
    if got[1] >= 1_000 || got[2] >= 1_000 {
        return Err(format!("`{s}`: minor and patch must each be below 1000"));
    }
    Ok(protocol::version::pack(got[0], got[1], got[2]))
}

/// Parse `key = value` lines; `#` comments and blanks skipped; string
/// values may be double-quoted. Refuses unknown keys, missing keys, and
/// unparseable values.
pub fn parse_shard_toml(text: &str) -> Result<ShardConfig, String> {
    let mut bind: Option<SocketAddr> = None;
    let mut seed: Option<u64> = None;
    let mut dev_spawn: Option<(f32, f32)> = None;
    let mut min_client: Option<u32> = None;
    let mut content_dir: Option<String> = None;
    let mut require_auth: Option<bool> = None;
    let mut cert_pem: Option<String> = None;
    let mut key_pem: Option<String> = None;
    let mut save_file: Option<String> = None;
    let mut world_file: Option<String> = None;
    let mut world_save_interval_ticks: u64 = DEFAULT_WORLD_SAVE_INTERVAL_TICKS;
    let mut status_addr: Option<SocketAddr> = None;
    let mut domain: Option<String> = None;
    let mut entitle_origin: Option<String> = None;
    let mut entitle_slug: Option<String> = None;
    let mut admins: Option<crate::admin::Admins> = None;
    let mut anomaly_file: Option<String> = None;
    let mut entitle_timeout_secs: Option<u64> = None;
    let mut entitle_sweep_secs: Option<u64> = None;
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
            // Empty is refused rather than read as "off", the same rule
            // `save_file` states: a line the operator wrote is a line they
            // meant, and off is the key being absent.
            "world_file" => {
                if value.is_empty() {
                    return Err(format!(
                        "shard.toml line {}: empty world_file — omit the key to \
                         generate a fresh world every boot, which is the default",
                        n + 1
                    ));
                }
                world_file = Some(value.to_string());
            }
            "world_save_interval_ticks" => {
                let v: u64 = value.parse().map_err(|_| {
                    format!(
                        "shard.toml line {}: world_save_interval_ticks must be a \
                         whole number of ticks",
                        n + 1
                    )
                })?;
                // Zero would be a save every tick — a write of up to
                // `WORLD_SAVE_MAX_BYTES` thirty times a second, which is not
                // a configuration anybody wants and is much more likely a
                // typo for "off" (which is omitting `world_file`).
                if v == 0 {
                    return Err(format!(
                        "shard.toml line {}: world_save_interval_ticks = 0 would \
                         write the world every tick; omit `world_file` to turn \
                         world persistence off",
                        n + 1
                    ));
                }
                world_save_interval_ticks = v;
            }
            // A `SocketAddr` like `bind`, and refused when it does not parse
            // — an empty value fails the same parse, so the "a line the
            // operator wrote is a line they meant" rule holds without a
            // separate check. Off is the key being absent.
            "status_addr" => {
                status_addr = Some(
                    value
                        .parse()
                        .map_err(|e| format!("shard.toml line {}: bad status_addr: {e}", n + 1))?,
                );
            }
            "domain" => {
                if value.is_empty() || value.len() > protocol::DOMAIN_MAX {
                    return Err(format!(
                        "shard.toml line {}: domain must be 1..={} bytes — it is \
                         the host players dial, and it goes in what they sign",
                        n + 1,
                        protocol::DOMAIN_MAX
                    ));
                }
                domain = Some(value.to_string());
            }
            // The ticket door (`entitle.rs`). Absent ⇒ this shard checks no
            // copies, which is what every test, every community shard and
            // the local dev flow run on.
            "entitle_origin" => {
                if value.is_empty() {
                    return Err(format!(
                        "shard.toml line {}: entitle_origin is empty — omit the key \
                         to check no copies, rather than naming nowhere",
                        n + 1
                    ));
                }
                if !value.starts_with("https://") && !value.starts_with("http://") {
                    return Err(format!(
                        "shard.toml line {}: entitle_origin needs a scheme, e.g. \
                         https://scry.moreright.xyz",
                        n + 1
                    ));
                }
                // A trailing slash would build `…//api/…`, which some origins
                // serve and some 404. Trimmed here rather than at every call.
                entitle_origin = Some(value.trim_end_matches('/').to_string());
            }
            "entitle_slug" => entitle_slug = Some(value.to_string()),
            "entitle_timeout_secs" => {
                entitle_timeout_secs = Some(value.parse().map_err(|e| {
                    format!("shard.toml line {}: bad entitle_timeout_secs: {e}", n + 1)
                })?);
            }
            "entitle_sweep_secs" => {
                entitle_sweep_secs = Some(value.parse().map_err(|e| {
                    format!("shard.toml line {}: bad entitle_sweep_secs: {e}", n + 1)
                })?);
            }
            "admin_wallets" => {
                // Refused whole on any bad entry (`Admins::parse`): a
                // typo'd admin address is an operator expecting a
                // privilege they do not have, and boot is the cheap place
                // to find out.
                admins = Some(
                    crate::admin::Admins::parse(value)
                        .map_err(|e| format!("shard.toml line {}: {e}", n + 1))?,
                );
            }
            "anomaly_file" => {
                if value.is_empty() {
                    return Err(format!("shard.toml line {}: anomaly_file is empty", n + 1));
                }
                anomaly_file = Some(value.to_string());
            }
            "min_client" => {
                min_client = Some(
                    parse_min_client(value)
                        .map_err(|e| format!("shard.toml line {}: bad min_client: {e}", n + 1))?,
                );
            }
            other => return Err(format!("shard.toml line {}: unknown key `{other}`", n + 1)),
        }
    }
    // Both or neither: half a TLS identity is a shard that self-signs while
    // its operator believes it is public.
    if cert_pem.is_some() != key_pem.is_some() {
        return Err("shard.toml: cert_pem and key_pem must be set together".into());
    }
    let entitle = crate::entitle::Config {
        origin: entitle_origin,
        slug: entitle_slug.unwrap_or_else(|| crate::ENTITLE_SLUG.to_string()),
        timeout: entitle_timeout_secs
            .map(std::time::Duration::from_secs)
            .unwrap_or(crate::entitle::DEFAULT_TIMEOUT),
        sweep: entitle_sweep_secs
            .map(std::time::Duration::from_secs)
            .unwrap_or(crate::entitle::DEFAULT_SWEEP),
    };
    // Refused here rather than clamped at the call site: a cadence nobody
    // chose is worse than a boot that says which line to fix.
    entitle.sane().map_err(|e| format!("shard.toml: {e}"))?;
    // A ticket door over an open door is a shard that checks copies and then
    // admits everyone who offers no address at all — the check has nothing to
    // hang on, because a guest has no wallet to ask about. Refused rather
    // than warned: unlike the save_file case below it, this one silently
    // gives the game away.
    if entitle.armed() && !require_auth.unwrap_or(false) {
        return Err(
            "shard.toml: entitle_origin needs require_auth = true — a guest has \
                    no wallet to check, so a ticket door over an open door checks nobody"
                .into(),
        );
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
        world_file,
        world_save_interval_ticks,
        status_addr,
        // Unset ⇒ 0, which admits every client that can talk to this shard at
        // all. `PROTO_VER` is still exact; this floor is the operator's extra.
        min_client: min_client.unwrap_or(0),
        // Unset ⇒ the bind host, which is what a dev shard on loopback
        // wants and what a shard behind a DNS name must override.
        domain: domain.unwrap_or_else(|| {
            let b = bind.expect("checked above");
            b.ip().to_string()
        }),
        entitle,
        admins: admins.unwrap_or_else(crate::admin::Admins::none),
        anomaly_file,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The floor an operator types has to mean what it looks like, and the
    /// packing is the one place a typo turns into a shard admitting builds it
    /// meant to refuse.
    #[test]
    fn the_client_floor_parses_as_a_version_or_says_why_not() {
        assert_eq!(
            parse_min_client("0.1.0").unwrap(),
            protocol::version::pack(0, 1, 0)
        );
        assert_eq!(
            parse_min_client("1.2.3").unwrap(),
            protocol::version::pack(1, 2, 3)
        );
        assert_eq!(parse_min_client("0.0.0").unwrap(), 0);

        // Ordered the way semver is, which is what makes the shard's gate one
        // `<`. This is the property, not an implementation detail.
        assert!(parse_min_client("0.1.0").unwrap() < parse_min_client("0.2.0").unwrap());
        assert!(parse_min_client("0.9.9").unwrap() < parse_min_client("1.0.0").unwrap());
        assert!(parse_min_client("0.1.9").unwrap() < parse_min_client("0.1.10").unwrap());

        // Each refusal names the thing that is wrong, because the operator is
        // reading it at boot with the file open.
        for bad in [
            "",
            "0.1",
            "0.1.0.0",
            "1",
            "v0.1.0",
            "0.x.0",
            "0.1.0-rc1",
            "latest",
        ] {
            assert!(
                parse_min_client(bad).is_err(),
                "`{bad}` should not parse as a version"
            );
        }
        // A component that would carry into the field above it: 0.1.1000 and
        // 0.2.0 must not become the same number.
        assert!(parse_min_client("0.1.1000").is_err());
        assert!(parse_min_client("0.1000.0").is_err());
    }

    /// Unset ⇒ 0 ⇒ admits everything, and the key is refused rather than
    /// ignored when it is malformed — an ignored floor is a shard whose
    /// operator believes it is gating and is not.
    #[test]
    fn the_client_floor_defaults_open_and_refuses_nonsense() {
        let cfg = parse_shard_toml("bind = \"127.0.0.1:4433\"\nseed = 7\n").unwrap();
        assert_eq!(cfg.min_client, 0, "an unset floor must admit everything");

        let cfg = parse_shard_toml("bind = \"127.0.0.1:4433\"\nseed = 7\nmin_client = \"0.2.0\"\n")
            .unwrap();
        assert_eq!(cfg.min_client, protocol::version::pack(0, 2, 0));

        let err = parse_shard_toml("bind = \"127.0.0.1:4433\"\nseed = 7\nmin_client = \"soon\"\n")
            .expect_err("a floor that is not a version must refuse the boot");
        assert!(
            err.contains("min_client"),
            "the error must name the key: {err}"
        );
    }

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
            let src = format!("bind = \"127.0.0.1:1\"\nseed = 7\nrequire_auth = {bad}\n");
            assert!(
                parse_shard_toml(&src).is_err(),
                "require_auth = `{bad}` must be refused, not coerced"
            );
        }
    }
}
