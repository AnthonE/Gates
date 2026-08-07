//! The world's file: a whole-world blob, the identity table beside it, and
//! the boot that refuses both rather than half-trusting either.
//!
//! `store.rs` is the *player* file — a fixed-slot table, one record per
//! person, written in place. This is the other half of `reference/SAVES.md`
//! §5's split: the world blob and the player-keyed store are different
//! files, wiped by different operator acts, because one is what a wipe
//! destroys and the other is what a wipe is *for* (blueprints, when they
//! exist).
//!
//! ## Why this file is written whole and `store.rs` is written in place
//!
//! A player record is 188 fixed bytes at a computed offset, so an in-place
//! `write_all` + `sync_data` is atomic enough: the record is either the old
//! one or the new one, and a torn one is caught by its own checksum and
//! costs one player. A world blob is variable-length and self-referential —
//! a section count near the front describes bytes near the back — so a
//! partial overwrite is not one bad record, it is a file that decodes to a
//! world nobody ever played. So: **write to a temp file, fsync, rename.**
//! Rename is atomic on every filesystem this runs on, which buys the
//! property in-place writing cannot have at this size — at the cost of one
//! extra file existing for a few milliseconds, which is the whole
//! difference and it is worth it.
//!
//! ## What the header is for, and it is not the checksum
//!
//! The checksum catches a torn write. It catches nothing else, because
//! anybody who can edit this file can recompute an xxh3 — which is why the
//! sim-side validation in `worldsave.rs` exists and is where the real
//! defence lives. What the header catches is the *operator's* mistakes,
//! which are the common ones and each has a one-line fix:
//!
//! - **wrong magic** — the `world_file` key points at something else.
//! - **wrong format** — the blob layout moved under a running deployment.
//! - **wrong seed** — this world was generated on a different island, so
//!   every base in it stands in a place that is now sea.
//! - **wrong content hash** — item indices moved, so restoring a box would
//!   hand somebody a different item than the one they put in it. Exactly
//!   the refusal `store.rs` makes for the same reason.
//!
//! ## The identity table, and why it is here rather than in the blob
//!
//! A body in the world carries the id it had, and an id is
//! `generation << 8 | slot` — minted per connection, meaningless after a
//! restart. So a saved sleeper is unclaimable unless something remembers
//! *whose* it is, and the thing that knows is the shard's opaque
//! `PlayerKey`, which `persist.rs` and `worldsave.rs` are both explicit
//! must never enter `sim-core`.
//!
//! It lives in this file's own section, written by the server, read by the
//! server, handed to `ShardCore`'s sleeper index at boot. The blob stays
//! pure and the key stays out of the sim — the same seam `SaveMsg` already
//! draws, one level up.

use crate::store::{PlayerKey, PLAYER_KEY_MAX_BYTES, SAVE_BACKUP_COUNT};
use sim_core::limits::MAX_PLAYERS;
use sim_core::worldsave::{WORLD_SAVE_FORMAT, WORLD_SAVE_MAX_BYTES};
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use xxhash_rust::xxh3::xxh3_64;

/// `GATESWLD` — deliberately different from `store.rs`'s `GATESAV\0`, so
/// pointing `world_file` at the player save (or the reverse) is a named
/// refusal and not a very confusing decode.
pub const WORLD_MAGIC: [u8; 8] = *b"GATESWLD";

/// The container's own version, separate from the blob's
/// [`WORLD_SAVE_FORMAT`]. Two numbers because there are two things that can
/// change independently: this one turns when the header or the identity
/// table moves, that one turns when the sim's state layout does. Both are
/// checked; a file that fails either is refused.
pub const WORLD_FILE_FORMAT: u16 = 1;

/// The header, 50 bytes of fields padded to 64: magic (8), file format (2),
/// blob format (2), seed (8), content hash (8), tick (8), sleeper count (2),
/// blob length (4), checksum (8). Padded so a later field costs no layout
/// change and no version turn for the padding itself.
pub const WORLD_HEADER_BYTES: usize = 64;
const H_MAGIC: usize = 0;
const H_FILE_FORMAT: usize = 8;
const H_BLOB_FORMAT: usize = 10;
const H_SEED: usize = 12;
const H_CONTENT: usize = 20;
const H_TICK: usize = 28;
const H_SLEEPERS: usize = 36;
const H_BLOB_LEN: usize = 38;
const H_SUM: usize = 42;

/// One `(key, body id)` pair: whose body a saved sleeper is.
pub const IDENT_RECORD_BYTES: usize = 1 + PLAYER_KEY_MAX_BYTES + 4;

/// What a boot found. Counts and reasons, never contents.
#[derive(Clone, Copy, Debug, Default)]
pub struct WorldLoad {
    /// No file yet — a first boot, and not a failure.
    pub created: bool,
    /// Bodies the blob carried.
    pub bodies: usize,
    /// Bodies with a key beside them, so they can be claimed again.
    pub claimable: usize,
    /// The tick the world resumes at.
    pub tick: u64,
}

/// The identity table as the server holds it: index-aligned to nothing,
/// just pairs. Bounded by `MAX_PLAYERS` because a sleeper occupies a world
/// player slot and there are that many.
pub type Identities = Vec<(PlayerKey, u32)>;

/// Everything a boot hands the sim thread about the world it is resuming.
///
/// The blob travels as **bytes**, not as a loaded `World`, and the reason
/// is a division of labour worth stating. `bin/shard.rs` validates it —
/// header, checksum, and a full trial `World::load` — *before a port is
/// bound*, because that is where every other boot refusal in this repo
/// lands and an operator with a bad file should learn it from a message and
/// not from players connecting to a fresh island. The sim thread then loads
/// the same bytes into the world it actually runs, after its content tables
/// are installed and before its first tick.
///
/// So the decode happens twice, and that is deliberate: it costs about a
/// millisecond once per boot, and it buys the refusal being early *and* the
/// loaded world never having to cross a thread.
pub struct WorldBoot {
    pub file: WorldFile,
    pub idents: Identities,
    /// Empty ⇒ nothing to resume: persistence off, or a first boot.
    pub blob: Vec<u8>,
    pub interval_ticks: u64,
}

impl WorldBoot {
    /// World persistence off — what every test runs, and what a shard with
    /// no `world_file` key runs.
    pub fn off() -> Self {
        Self {
            file: WorldFile::closed(),
            idents: Identities::new(),
            blob: Vec::new(),
            interval_ticks: u64::MAX,
        }
    }
}

/// Rotate `<path>.1` → `.2` and copy `<path>` → `.1`, exactly as the player
/// store does and for the reason `reference/SAVES.md` §6 gives rather than
/// the obvious one: corruption can predate the newest backup, so a depth of
/// one is not worth having.
///
/// Best-effort, like the player store's: a read-only directory or a full
/// disk must not stop a shard booting. The consequence is that a backup is
/// a convenience and the checksum is the guarantee.
fn rotate_backups(path: &Path) {
    let at = |n: usize| {
        let mut s = path.as_os_str().to_os_string();
        s.push(format!(".{n}"));
        PathBuf::from(s)
    };
    for n in (1..SAVE_BACKUP_COUNT).rev() {
        let _ = std::fs::rename(at(n), at(n + 1));
    }
    let _ = std::fs::copy(path, at(1));
}

fn encode_header(
    seed: u64,
    content_hash: u64,
    tick: u64,
    sleepers: usize,
    blob_len: usize,
    body_sum: u64,
) -> [u8; WORLD_HEADER_BYTES] {
    let mut h = [0u8; WORLD_HEADER_BYTES];
    h[H_MAGIC..H_MAGIC + 8].copy_from_slice(&WORLD_MAGIC);
    h[H_FILE_FORMAT..H_FILE_FORMAT + 2].copy_from_slice(&WORLD_FILE_FORMAT.to_le_bytes());
    h[H_BLOB_FORMAT..H_BLOB_FORMAT + 2].copy_from_slice(&WORLD_SAVE_FORMAT.to_le_bytes());
    h[H_SEED..H_SEED + 8].copy_from_slice(&seed.to_le_bytes());
    h[H_CONTENT..H_CONTENT + 8].copy_from_slice(&content_hash.to_le_bytes());
    h[H_TICK..H_TICK + 8].copy_from_slice(&tick.to_le_bytes());
    h[H_SLEEPERS..H_SLEEPERS + 2].copy_from_slice(&(sleepers as u16).to_le_bytes());
    h[H_BLOB_LEN..H_BLOB_LEN + 4].copy_from_slice(&(blob_len as u32).to_le_bytes());
    h[H_SUM..H_SUM + 8].copy_from_slice(&body_sum.to_le_bytes());
    h
}

/// The world file's writer — the only thing in this process that touches
/// it, and like `SaveFile` it makes no decisions.
///
/// `None` ⇒ world persistence is off, and every write is a no-op rather
/// than an error, because a shard with no `world_file` was told not to
/// remember rather than failing to.
pub struct WorldFile {
    path: Option<PathBuf>,
    seed: u64,
    content_hash: u64,
}

impl WorldFile {
    pub fn closed() -> Self {
        Self {
            path: None,
            seed: 0,
            content_hash: 0,
        }
    }

    pub fn is_open(&self) -> bool {
        self.path.is_some()
    }

    /// Write a world. `Ok(false)` ⇒ there is no file, which is a no-op and
    /// not an error — the bool exists for the reason `SaveFile::write`'s
    /// does: a caller that counted every `Ok` would report a shard
    /// persisting a world it has never been given a path for.
    ///
    /// Temp-then-rename, and the `sync_data` is on the **temp file before
    /// the rename**, which is the ordering that matters: renaming a file
    /// whose bytes are still in the page cache would publish a name
    /// pointing at nothing after a power cut.
    pub fn write(
        &mut self,
        tick: u64,
        blob: &[u8],
        idents: &[(PlayerKey, u32)],
    ) -> std::io::Result<bool> {
        let Some(path) = self.path.as_ref() else {
            return Ok(false);
        };
        let mut body = Vec::with_capacity(idents.len() * IDENT_RECORD_BYTES + blob.len());
        for (key, id) in idents {
            let mut rec = [0u8; IDENT_RECORD_BYTES];
            let bytes = key.as_bytes();
            rec[0] = bytes.len() as u8;
            rec[1..1 + bytes.len()].copy_from_slice(bytes);
            rec[1 + PLAYER_KEY_MAX_BYTES..].copy_from_slice(&id.to_le_bytes());
            body.extend_from_slice(&rec);
        }
        body.extend_from_slice(blob);
        // One checksum over the identity table AND the blob together, not
        // one each: they are only meaningful as a pair — a key table from
        // one save against a world from another names bodies that are not
        // there — so there is no state in which accepting half of this is
        // better than refusing both.
        let sum = xxh3_64(&body);
        let head = encode_header(
            self.seed,
            self.content_hash,
            tick,
            idents.len(),
            blob.len(),
            sum,
        );

        let tmp = {
            let mut s = path.as_os_str().to_os_string();
            s.push(".tmp");
            PathBuf::from(s)
        };
        {
            let mut f = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(&tmp)?;
            f.write_all(&head)?;
            f.write_all(&body)?;
            f.sync_data()?;
        }
        std::fs::rename(&tmp, path)?;
        Ok(true)
    }
}

impl Default for WorldFile {
    fn default() -> Self {
        Self::closed()
    }
}

/// Counts and a flag, never contents — the posture `Saves`'s `Debug` takes,
/// for the same reason: the thing that prints this is usually a failing
/// test, and a derive here would dump every base on the shard into it.
impl std::fmt::Debug for WorldFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "WorldFile({})",
            if self.is_open() { "open" } else { "off" }
        )
    }
}

/// Open the world file and read it into `world`.
///
/// Every refusal names what to do about it, because every one of them is an
/// operator mistake with a one-command fix and the alternative is a shard
/// that quietly starts a new world on top of everyone's bases. The one
/// thing this will not do is start half a world: `World::load` decodes and
/// validates everything before it writes anything.
pub fn open(
    path: &Path,
    world: &mut sim_core::world::World,
    seed: u64,
    content_hash: u64,
    interval_ticks: u64,
) -> Result<(WorldBoot, WorldLoad), String> {
    // Before the handle and before any validation: a file this boot is
    // about to refuse is exactly the file an operator most wants a copy of.
    if path.exists() {
        rotate_backups(path);
    }
    let file = WorldFile {
        path: Some(path.to_path_buf()),
        seed,
        content_hash,
    };
    if !path.exists() {
        return Ok((
            WorldBoot {
                file,
                idents: Identities::new(),
                blob: Vec::new(),
                interval_ticks,
            },
            WorldLoad {
                created: true,
                ..Default::default()
            },
        ));
    }

    let mut f = File::open(path).map_err(|e| format!("world file {}: {e}", path.display()))?;
    let len = f
        .metadata()
        .map_err(|e| format!("world file {}: {e}", path.display()))?
        .len();
    // Length before content, or a file too short to hold a header refuses
    // as "failed to fill whole buffer" — an io error about a read, for what
    // is actually a knob pointing at the wrong file.
    if len < WORLD_HEADER_BYTES as u64 {
        return Err(format!(
            "world file {} is {len} bytes, too short to be a gates world file \
             (a header is {WORLD_HEADER_BYTES}) — check the `world_file` path in \
             shard.toml before anything else",
            path.display()
        ));
    }
    let mut head = [0u8; WORLD_HEADER_BYTES];
    f.read_exact(&mut head)
        .map_err(|e| format!("world file {}: reading header: {e}", path.display()))?;
    if head[H_MAGIC..H_MAGIC + 8] != WORLD_MAGIC {
        return Err(format!(
            "world file {} is not a gates world file (bad magic) — if it is the \
             player save, that is a different key: `save_file`",
            path.display()
        ));
    }
    let at16 = |o: usize| u16::from_le_bytes([head[o], head[o + 1]]);
    let at32 = |o: usize| u32::from_le_bytes([head[o], head[o + 1], head[o + 2], head[o + 3]]);
    let at64 = |o: usize| {
        u64::from_le_bytes([
            head[o],
            head[o + 1],
            head[o + 2],
            head[o + 3],
            head[o + 4],
            head[o + 5],
            head[o + 6],
            head[o + 7],
        ])
    };
    let file_format = at16(H_FILE_FORMAT);
    if file_format != WORLD_FILE_FORMAT {
        return Err(format!(
            "world file {} is format {file_format}, this build writes \
             {WORLD_FILE_FORMAT} — it was written by a different build. Move it \
             aside to start a fresh world; there is no converter",
            path.display()
        ));
    }
    let blob_format = at16(H_BLOB_FORMAT);
    if blob_format != WORLD_SAVE_FORMAT {
        return Err(format!(
            "world file {} holds a v{blob_format} world, this build reads \
             v{WORLD_SAVE_FORMAT} — the sim's state layout moved. Move it aside \
             to start a fresh world; there is no converter",
            path.display()
        ));
    }
    let got_seed = at64(H_SEED);
    if got_seed != seed {
        return Err(format!(
            "world file {} was generated on seed {got_seed}, this shard is seed \
             {seed} — every base in it stands somewhere that is now a different \
             island. Fix the `seed` key, or point `world_file` somewhere else",
            path.display()
        ));
    }
    let got_content = at64(H_CONTENT);
    if got_content != content_hash {
        return Err(format!(
            "world file {} was written under content hash {got_content:#x}, this \
             shard baked {content_hash:#x} — item indices have moved, so a box \
             restored from it would hand somebody a different item than the one \
             they put in. Restore the content that wrote it, or start a fresh world",
            path.display()
        ));
    }

    let sleepers = at16(H_SLEEPERS) as usize;
    let blob_len = at32(H_BLOB_LEN) as usize;
    if sleepers > MAX_PLAYERS {
        return Err(format!(
            "world file {} claims {sleepers} identities and a shard holds \
             {MAX_PLAYERS} — the file is corrupt",
            path.display()
        ));
    }
    if blob_len > WORLD_SAVE_MAX_BYTES {
        return Err(format!(
            "world file {} claims a {blob_len}-byte world and the ceiling is \
             {WORLD_SAVE_MAX_BYTES} — the file is corrupt",
            path.display()
        ));
    }
    let want = sleepers * IDENT_RECORD_BYTES + blob_len;
    if len as usize != WORLD_HEADER_BYTES + want {
        return Err(format!(
            "world file {} is {len} bytes; its header describes {} — it was \
             truncated, most likely by a disk filling mid-write. `{}.1` is the \
             previous run",
            path.display(),
            WORLD_HEADER_BYTES + want,
            path.display()
        ));
    }
    let mut body = vec![0u8; want];
    f.read_exact(&mut body)
        .map_err(|e| format!("world file {}: reading body: {e}", path.display()))?;
    if xxh3_64(&body) != at64(H_SUM) {
        return Err(format!(
            "world file {} failed its checksum — the shard was killed mid-write, \
             or the disk is going bad. `{}.1` is the previous run and `{}.2` the \
             one before it (the second exists because the first can share the \
             corruption)",
            path.display(),
            path.display(),
            path.display()
        ));
    }

    // The identity table, then the world. Decoded in that order and the
    // world last, because the world is the expensive half and a malformed
    // key table means the sleepers in it are unclaimable anyway.
    let mut idents = Identities::with_capacity(sleepers);
    for i in 0..sleepers {
        let rec = &body[i * IDENT_RECORD_BYTES..(i + 1) * IDENT_RECORD_BYTES];
        let key_len = rec[0] as usize;
        if key_len == 0 || key_len > PLAYER_KEY_MAX_BYTES {
            return Err(format!(
                "world file {}: identity {i} has a {key_len}-byte key (the cap is \
                 {PLAYER_KEY_MAX_BYTES}) — the file is corrupt",
                path.display()
            ));
        }
        let Some(key) = PlayerKey::new(&rec[1..1 + key_len]) else {
            return Err(format!(
                "world file {}: identity {i} has an unusable key — the file is corrupt",
                path.display()
            ));
        };
        let id = u32::from_le_bytes([
            rec[1 + PLAYER_KEY_MAX_BYTES],
            rec[2 + PLAYER_KEY_MAX_BYTES],
            rec[3 + PLAYER_KEY_MAX_BYTES],
            rec[4 + PLAYER_KEY_MAX_BYTES],
        ]);
        idents.push((key, id));
    }

    let blob = &body[sleepers * IDENT_RECORD_BYTES..];
    world.load(blob).map_err(|e| {
        format!(
            "world file {} was refused by the sim: {} — the checksum passed, so \
             the bytes are intact and the *contents* are wrong, which means the \
             file was edited or written by a build whose rules differ. `{}.1` is \
             the previous run",
            path.display(),
            e.reason(),
            path.display()
        )
    })?;

    let bodies = world.players.iter().filter(|p| p.active).count();
    // An identity naming a body the world does not have is dropped rather
    // than refused: the pair is checkable and the mismatch is survivable —
    // that player comes back through their store record, which is exactly
    // what the record is for (`reference/SAVES.md` §9.2).
    let claimable = idents
        .iter()
        .filter(|(_, id)| world.is_sleeper(*id))
        .count();
    let tick = world.tick;
    Ok((
        WorldBoot {
            file,
            idents,
            blob: blob.to_vec(),
            interval_ticks,
        },
        WorldLoad {
            created: false,
            bodies,
            claimable,
            tick,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The header's offsets are a hand-written layout, which is the shape
    /// `CLAUDE.md`'s positional-payload trap is about — the right value at
    /// the wrong offset, invisible to a round-trip because both halves move
    /// together. Pinned here against an independent reading.
    #[test]
    fn the_header_layout_is_pinned() {
        let h = encode_header(
            0x1122_3344_5566_7788,
            0x99AA_BBCC_DDEE_FF00,
            4242,
            7,
            99,
            0xDEAD,
        );
        assert_eq!(&h[0..8], b"GATESWLD", "magic at 0");
        assert_eq!(&h[8..10], &1u16.to_le_bytes(), "file format at 8");
        assert_eq!(
            &h[10..12],
            &WORLD_SAVE_FORMAT.to_le_bytes(),
            "blob fmt at 10"
        );
        assert_eq!(&h[12..20], &0x1122_3344_5566_7788u64.to_le_bytes(), "seed");
        assert_eq!(
            &h[20..28],
            &0x99AA_BBCC_DDEE_FF00u64.to_le_bytes(),
            "content"
        );
        assert_eq!(&h[28..36], &4242u64.to_le_bytes(), "tick at 28");
        assert_eq!(&h[36..38], &7u16.to_le_bytes(), "sleepers at 36");
        assert_eq!(&h[38..42], &99u32.to_le_bytes(), "blob len at 38");
        assert_eq!(&h[42..50], &0xDEADu64.to_le_bytes(), "checksum at 42");
        assert_eq!(h.len(), 64, "the header is padded to 64");
    }

    /// The two magics differ, so pointing one key at the other's file is a
    /// named refusal rather than a very confusing decode.
    #[test]
    fn the_two_files_cannot_be_mistaken_for_each_other() {
        assert_ne!(WORLD_MAGIC, crate::store::SAVE_MAGIC);
    }
}
