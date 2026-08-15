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
    /// Dev-only spawn kit override: `"item.id:count, item.id:count, …"`,
    /// **replacing** `content/balance.toml`'s `[[spawn_kit]]` wholesale for
    /// this shard's fresh spawns and respawns. Unset is the shipping
    /// default, which is the content's own kit — a rock and a torch
    /// (DECISIONS.md 2026-08-15).
    ///
    /// **Why this exists at all.** The nine-entry kit it replaces (900
    /// wood, 500 stone, 100 metal frags, a plan and a hammer) was never
    /// balance: its own comment in `balance.toml` called it *"testing
    /// scaffolding"*, and it shipped because a naked spawn made the build
    /// wheel read `350 Wood (0)` and left the whole building interaction
    /// unreachable to anyone testing it. Deleting it from content without
    /// replacing the capability would have made every future build/hammer
    /// pass start by hand-editing `content/`, which is worse than a flag:
    /// an edited content tree is a set nobody declared, and `CLAUDE.md`'s
    /// packager entry is what that costs.
    ///
    /// **It is `dev_spawn`'s shape on purpose**, and it inherits that key's
    /// caveats exactly. Both are shard-local perturbations of the sim's
    /// inputs that no header records, so a shard run with either is not
    /// reproducible from its seed and content hash alone — set it on a dev
    /// box and never on a public shard. It is deliberately NOT a boolean
    /// naming a second kit baked into the binary: the counts live here,
    /// where an operator typed them for one session, rather than in
    /// `crates/`, which is what wall 7 is about.
    ///
    /// Ids and counts are validated against the loaded content at boot
    /// (`dev_spawn_kit`), so a typo refuses the boot rather than quietly
    /// granting nothing.
    pub dev_spawn_kit: Option<Vec<(String, u32)>>,
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
    /// **How many resident inhabitants the shard seats itself**
    /// (`population.rs`). Bots, dialled over this shard's own wire after the
    /// bind, running the raid profile — half owners building and locking,
    /// half attackers planting charges.
    ///
    /// **Zero is the shipping default**, and it is the honest one for a
    /// public shard: a population is scaffolding for a world that does not
    /// have enough players yet, and an operator who has players does not want
    /// bots eating their slots. It exists because the opposite state was
    /// worse — three consecutive judges ranked "a shard has no inhabitants"
    /// the largest playable gap in the game, and until this key every
    /// opponent in the tree lived inside a `#[test]`.
    ///
    /// Capped at [`crate::population::MAX_POPULATION`] and refused above it,
    /// so at least one seat is always a person's. Refused outright when
    /// `require_auth` is set, because an inhabitant handshakes as a guest.
    pub population: usize,
    /// Which congestion controller quinn runs (`cc = "cubic" | "bbr"`).
    ///
    /// **CUBIC is the default and stays the default.** `NETCODE.md` §2.1
    /// names the reason this is worth a key at all: CUBIC halves cwnd on
    /// loss, and a collapsed window at 100 ms RTT can fall below our 30 Hz
    /// send rate — so the controller is one of the few transport choices
    /// that can change how the game feels. BBR is loss-insensitive and is
    /// labelled experimental in quinn, which is exactly why it is an
    /// operator's A/B rather than a new default.
    ///
    /// §2.2 promised this as a `--cc` flag from the day it was written and
    /// it did not exist until 2026-08-15, which meant "measure before
    /// trusting" had never been runnable. `net_congestion_events` is the
    /// measurement it unblocks.
    pub congestion: Congestion,
}

/// Congestion controller selection (`shard.toml` `cc`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Congestion {
    /// quinn's default. Ours too, until somebody measures otherwise.
    #[default]
    Cubic,
    /// Loss-insensitive; experimental in quinn.
    Bbr,
}

impl ShardConfig {
    /// Loopback + ephemeral port: what tests and local bots use.
    pub fn ephemeral(seed: u64) -> Self {
        Self {
            bind: "127.0.0.1:0".parse().expect("static addr"),
            congestion: Congestion::Cubic,
            seed,
            dev_spawn: None,
            dev_spawn_kit: None,
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
            population: 0,
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
    let mut dev_spawn_kit: Option<Vec<(String, u32)>> = None;
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
    let mut population: Option<usize> = None;
    let mut congestion: Option<Congestion> = None;
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
            // `"item.id:count, item.id:count, …"`. Shape only here — that
            // an id exists and that a count fits its stack are questions
            // for the loaded content, and this function has none
            // (`dev_spawn_kit`, below, is where the boot asks them).
            "dev_spawn_kit" => {
                let mut stacks = Vec::new();
                for part in value.split(',') {
                    let part = part.trim();
                    if part.is_empty() {
                        continue;
                    }
                    let (id, count) = part.split_once(':').ok_or_else(|| {
                        format!(
                            "shard.toml line {}: dev_spawn_kit wants \
                             \"item.id:count, …\", got `{part}`",
                            n + 1
                        )
                    })?;
                    let count: u32 = count.trim().parse().map_err(|e| {
                        format!(
                            "shard.toml line {}: bad dev_spawn_kit count for `{id}`: {e}",
                            n + 1
                        )
                    })?;
                    stacks.push((id.trim().to_string(), count));
                }
                if stacks.is_empty() {
                    return Err(format!(
                        "shard.toml line {}: dev_spawn_kit is empty — delete the \
                         key to use the content's own kit, or write \
                         `dev_spawn_kit = \"\"` nowhere at all",
                        n + 1
                    ));
                }
                if stacks.len() > sim_core::limits::MAX_SPAWN_KIT {
                    return Err(format!(
                        "shard.toml line {}: dev_spawn_kit has {} stacks, past the \
                         sim's {}-slot kit",
                        n + 1,
                        stacks.len(),
                        sim_core::limits::MAX_SPAWN_KIT
                    ));
                }
                dev_spawn_kit = Some(stacks);
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
            // Refused above the cap rather than clamped, which is wall 4's
            // "a bound with a stated overflow policy" at a config line: a
            // shard that seated 99 of the 200 an operator asked for would be
            // running a population nobody chose, and the operator would read
            // the number they typed as the number that is playing.
            "population" => {
                let v: usize = value.parse().map_err(|_| {
                    format!(
                        "shard.toml line {}: population must be a whole number of \
                         inhabitants (0 seats none, which is the default)",
                        n + 1
                    )
                })?;
                if v > crate::population::MAX_POPULATION {
                    return Err(format!(
                        "shard.toml line {}: population {v} is over the {} cap — a \
                         shard whose every slot is a bot refuses the player it was \
                         seated for",
                        n + 1,
                        crate::population::MAX_POPULATION
                    ));
                }
                population = Some(v);
            }
            // Refused rather than defaulted on an unknown value: an
            // operator who typed `cc = "bbr2"` wanted something other than
            // CUBIC, and silently giving them CUBIC is the A/B reporting a
            // result for a controller it never ran.
            "cc" => match value {
                "cubic" => congestion = Some(Congestion::Cubic),
                "bbr" => congestion = Some(Congestion::Bbr),
                other => {
                    return Err(format!(
                        "shard.toml line {}: cc must be \"cubic\" or \"bbr\", got \
                         `{other}` (cubic is the default; bbr is experimental in \
                         quinn and is an A/B, not a recommendation)",
                        n + 1
                    ))
                }
            },
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
    // An inhabitant handshakes as `Address::GUEST` (`botclient::run_bot`), so
    // a shard that refuses guests refuses every one of them — and the
    // supervisor would re-man each post forever against its own front door,
    // a reconnect storm a shard runs against itself. Refused here, with the
    // two keys named, rather than discovered as a `refused_auth` counter
    // climbing at 0.5 Hz on a shard whose config asked for company.
    if population.unwrap_or(0) > 0 && require_auth.unwrap_or(false) {
        return Err(
            "shard.toml: population needs require_auth = false — an inhabitant \
                    joins as a guest, so an authenticating shard refuses every one \
                    of them and the population re-dials its own closed door"
                .into(),
        );
    }
    Ok(ShardConfig {
        congestion: congestion.unwrap_or_default(),
        bind: bind.ok_or("shard.toml: missing `bind`")?,
        seed: seed.ok_or("shard.toml: missing `seed`")?,
        dev_spawn,
        dev_spawn_kit,
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
        // Unset ⇒ 0, a shard that seats nobody but the players who dial it.
        population: population.unwrap_or(0),
    })
}

/// Resolve `dev_spawn_kit` against the content this shard actually loaded,
/// or say which entry is wrong.
///
/// `None` in, `None` out — the shard then runs `content.bake_spawn_kit()`,
/// which is the shipping path and the only one a public shard ever takes.
///
/// **Every refusal `bake_spawn_kit` makes, this makes too**, and by asking
/// the same questions of the same tables rather than by trusting the key:
/// an unknown id, a zero count, a count past the item's own stack size, and
/// a repeat (`grant_kit` writes slots and never merges, so a repeated id is
/// a typo that silently halves what the operator meant). A dev shard that
/// boots is therefore a dev shard whose kit the sim can actually represent
/// — the alternative is a flag that grants nothing and reads as a defect in
/// whatever verb the session was there to test.
///
/// It does NOT re-ask the content's own balance questions, because it is not
/// balance: this kit never reaches a player who did not type it.
///
/// ⚠ **That sentence is about who sees the kit, not about how often.** Since
/// 2026-08-15 `World::wake` grants the kit on every respawn, so whatever is
/// typed here is minted per DEATH, not once per character — the documented
/// example in `shard.toml.example` re-mints 900 wood, 500 stone and 100
/// metal frags every time a tester dies. That is the exact arithmetic
/// `world.rs`'s grant comment cites as the reason the old shipped kit could
/// not be re-granted, re-opened on the dev path alone and deliberately: the
/// key exists so a build/hammer session is not blocked, and a tester who
/// dies five times wanting materials is not a balance failure. It IS a
/// reason not to reach for this key on anything a stranger can join.
pub fn dev_spawn_kit(
    spec: Option<&[(String, u32)]>,
    content: &content::Content,
) -> Result<Option<sim_core::inventory::SpawnKit>, String> {
    let Some(spec) = spec else {
        return Ok(None);
    };
    let mut kit = sim_core::inventory::SpawnKit::EMPTY;
    for (i, (id, count)) in spec.iter().enumerate() {
        let item = content
            .item(id)
            .ok_or_else(|| format!("dev_spawn_kit: no such item `{id}`"))?;
        if *count == 0 {
            return Err(format!(
                "dev_spawn_kit: `{id}` grants 0, which is a slot that draws empty"
            ));
        }
        if *count as u64 > item.stack as u64 {
            return Err(format!(
                "dev_spawn_kit: `{id}` grants {count} past its own stack size {}",
                item.stack
            ));
        }
        if spec[..i].iter().any(|(prev, _)| prev == id) {
            return Err(format!(
                "dev_spawn_kit: `{id}` granted twice — the kit writes slots and \
                 never merges, so the second entry overwrites nothing and the \
                 first is what you get"
            ));
        }
        let idx = content
            .item_index(id)
            .ok_or_else(|| format!("dev_spawn_kit: `{id}` vanished between two lookups"))?;
        let stack = sim_core::gather::ItemStack {
            item: idx,
            count: u16::try_from(*count)
                .map_err(|_| format!("dev_spawn_kit: `{id}` count {count} overflows u16"))?,
        };
        if !kit.set(i, stack) {
            return Err(format!(
                "dev_spawn_kit: `{id}` did not fit the sim's {}-slot kit",
                sim_core::limits::MAX_SPAWN_KIT
            ));
        }
    }
    Ok(Some(kit))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A congestion controller an operator did not choose is an A/B that
    /// reports a result for the wrong arm. So: the two names parse, the
    /// default is CUBIC (unwritten and written), and **anything else is a
    /// boot failure rather than a silent fallback** — the failure mode this
    /// refuses is a `shard.toml` reading `cc = "bbr2"` while the shard runs
    /// CUBIC and the operator reads the measurement as BBR's.
    #[test]
    fn the_congestion_knob_parses_or_refuses_but_never_guesses() {
        let base = "bind = \"127.0.0.1:0\"\nseed = 1\n";
        let of = |s: &str| parse_shard_toml(s).map(|c| c.congestion);

        assert_eq!(of(base).unwrap(), Congestion::Cubic, "default is cubic");
        assert_eq!(
            of(&format!("{base}cc = \"cubic\"\n")).unwrap(),
            Congestion::Cubic
        );
        assert_eq!(
            of(&format!("{base}cc = \"bbr\"\n")).unwrap(),
            Congestion::Bbr
        );

        let err = of(&format!("{base}cc = \"bbr2\"\n")).unwrap_err();
        assert!(
            err.contains("cc must be"),
            "an unknown controller must name itself in the error, got: {err}"
        );
    }

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

    /// The dev kit override parses its own shape and refuses the rest.
    ///
    /// Unset is the shipping default and means "the content's kit", which is
    /// the assertion that matters most: a public shard that never writes this
    /// key must be indistinguishable from one built before the key existed.
    ///
    /// Proven red by deleting the `stacks.is_empty()` refusal (the `""` case
    /// then parses to an empty kit, which seats a naked spawn on a shard whose
    /// operator asked for a fat one) and by deleting the `split_once(':')`
    /// error (`"item.wood"` with no count then panics on unwrap rather than
    /// naming the line).
    #[test]
    fn dev_spawn_kit_parses_and_refuses() {
        let base = "bind = \"127.0.0.1:1\"\nseed = 1\n";
        assert_eq!(
            parse_shard_toml(base).unwrap().dev_spawn_kit,
            None,
            "an unset key must mean the content's own kit"
        );

        let ok = parse_shard_toml(&format!(
            "{base}dev_spawn_kit = \"item.wood:900, item.stone:500\"\n"
        ))
        .unwrap();
        assert_eq!(
            ok.dev_spawn_kit,
            Some(vec![
                ("item.wood".to_string(), 900),
                ("item.stone".to_string(), 500)
            ]),
            "order is load-bearing — the first entries land on the hotbar"
        );

        for bad in [
            "\"\"",                     // empty: delete the key instead
            "\"item.wood\"",            // no count
            "\"item.wood:\"",           // no count
            "\"item.wood:lots\"",       // not a number
            "\"item.wood:-1\"",         // not a count
            "\"item.wood:4294967296\"", // past u32
        ] {
            assert!(
                parse_shard_toml(&format!("{base}dev_spawn_kit = {bad}\n")).is_err(),
                "accepted dev_spawn_kit = {bad}"
            );
        }

        // Past the sim's kit width, refused here rather than truncated into a
        // kit the operator did not write.
        let long: Vec<String> = (0..sim_core::limits::MAX_SPAWN_KIT + 1)
            .map(|i| format!("item.x{i}:1"))
            .collect();
        assert!(
            parse_shard_toml(&format!("{base}dev_spawn_kit = \"{}\"\n", long.join(", "))).is_err(),
            "a kit past MAX_SPAWN_KIT was accepted"
        );
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
