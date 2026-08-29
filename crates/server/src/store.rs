//! The player store — **what makes "your base" a true sentence**, or the
//! first half of it.
//!
//! A shard that forgets you on every disconnect cannot be authenticated
//! into meaning anything: you could prove who you are perfectly and still
//! lose everything when your connection drops. So this is the slice that
//! had to come first, and it needs nothing from elo to land.
//!
//! ## The key is opaque, and that is the point
//!
//! A save is filed under a [`PlayerKey`]: bytes the admission seam returned
//! (`auth::verify`), which this module never interprets. Whether they come
//! from a SIWE signature, an elo player id or a dev flag is that seam's
//! business — change it and nothing here moves. The debate about *which*
//! identity scheme wins was therefore never a blocker for persistence; it
//! is one function's return type, and SIWE won (`DECISIONS.md` 2026-08-07).
//!
//! ## What this stores, and what elo stores
//!
//! This holds where your body stood and what was in its hands. It holds no
//! profile, no balance, no item entitlement: those live in the launcher's
//! world, and a shard that cached them would be a second, stale copy of
//! somebody else's ledger. The division is the same one `elo_overlay.rs`
//! draws for keys — the game process never holds what it is not the
//! authority for.
//!
//! ## The file: fixed slots, written in place
//!
//! Header, then `MAX_SAVED_PLAYERS` fixed records addressed by index. Not
//! an append log, and that is a deliberate reversal: an append log needs
//! compaction, and the autosave sweep would grow one by hundreds of
//! megabytes a day. A fixed table has a constant size, needs no compaction,
//! loads in one pass, and gives a save the same durability an append gives
//! — a torn write costs that one player's last record, which the per-record
//! checksum detects at load.
//!
//! Every capacity has a bound and a stated overflow policy (wall 4): the
//! table holds `MAX_SAVED_PLAYERS` distinct players and a full table
//! **evicts the least recently saved**, counted. Their next join is a fresh
//! character, which is the honest outcome — the alternative is refusing to
//! remember anyone new, and a shard that has been up long enough to fill
//! this table is exactly the shard where a new player matters.
//!
//! ## Boot validation is a refusal, never a silent wipe
//!
//! The header pins the seed and the content hash. A mismatch means the file
//! describes bodies on another island, or inventories whose item rows have
//! moved — so the shard **refuses to boot** and says which, rather than
//! quietly handing a hundred players an empty inventory. Moving the file
//! aside is the operator's way of saying "wipe", and it is a thing they
//! already know how to do. This is wall 7's rule ("a replay replays the
//! content it was played under") applied to a save.

use sim_core::limits::MAX_PLAYERS;
use sim_core::persist::{PlayerSave, PLAYER_SAVE_BYTES};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use xxhash_rust::xxh3::xxh3_64;

/// Longest key the store files a save under.
///
/// Covers every identity anyone has proposed for this game with room over:
/// a lowercase `0x…` Ethereum address (42 bytes — what `auth::verify`
/// actually returns), a 32-byte elo player id, a UUID, or a dev shard's
/// bare name. Deliberately too small to hold a self-describing credential —
/// a key is a lookup handle, and a store that could hold a JWT would invite
/// somebody to read claims out of one.
///
/// The assert below is what fixes the *number*: the key IS the address
/// today, so a cap under 42 would leave every verified player unnameable —
/// persistence silently doing nothing for everybody, with every gate green.
/// It stays at 48 rather than shrinking to 42 because the key is
/// deliberately opaque to everything but `auth.rs` — the day elo maps an
/// address to a player id, that id lands here and nothing else moves.
/// Proposed default, DECISIONS.md §open ("player persistence v0").
pub const PLAYER_KEY_MAX_BYTES: usize = 48;

const _: () = assert!(
    PLAYER_KEY_MAX_BYTES >= 42,
    "a key holds a lowercase 0x-prefixed Ethereum address, which is exactly 42 \
     bytes: a smaller cap would leave every verified player unnameable"
);

/// How many distinct players one shard remembers.
///
/// Sized against the shard rather than the population: `MAX_PLAYERS` is 100,
/// so this is forty wipes' worth of distinct visitors at full occupancy, and
/// it costs a ~1 MB file and one ~1 MB allocation at boot. The static assert
/// below is the part that matters — a table smaller than the shard's own
/// player cap could not hold everyone standing in the world at once.
/// Proposed default, DECISIONS.md §open ("player persistence v0").
pub const MAX_SAVED_PLAYERS: usize = 4096;

const _: () = assert!(
    MAX_SAVED_PLAYERS >= MAX_PLAYERS,
    "the save table must hold at least everyone the shard can seat at once"
);

/// How many numbered backups sit beside the live file: `<path>.1` … `<path>.N`,
/// higher number = older. Rotated at boot.
///
/// **2, matching the reference game's `server.saveBackupCount`, which
/// documents 2 as both its default and its minimum** — and the reason for the
/// minimum is the part worth copying rather than the number. Their recovery
/// documentation warns that `.sav.1` may itself be corrupt, because corruption
/// can predate the newest backup by several save cycles, so the operator walks
/// back to `.sav.2`. A depth of one gives you nowhere to walk.
///
/// Costs `SAVE_BACKUP_COUNT` × the file (~1 MB each) on disk and one copy at
/// boot. Nothing rotates during a run: our writes are in place and per-record
/// checksummed, so the failure a rotation defends against is a *file*, and a
/// file only turns over at a boot. `reference/SAVES.md` §6.
/// Proposed default, DECISIONS.md §open ("player persistence v0").
pub const SAVE_BACKUP_COUNT: usize = 2;

const _: () = assert!(
    SAVE_BACKUP_COUNT >= 2,
    "reference/SAVES.md §6: two is the reference game's documented MINIMUM, not \
     just its default — the newest backup can share the corruption when the \
     corruption predates it by a few save cycles, so a depth of one leaves an \
     operator nowhere to walk back to"
);

/// File magic. Read before anything else, so pointing `save_file` at the
/// wrong file says "not a save file" rather than misreading its first bytes
/// as a seed.
pub const SAVE_MAGIC: [u8; 8] = *b"GATESAV\0";

/// On-disk format version.
///
/// **Bump this in the same commit as any change to the record layout**, the
/// header layout, or `PLAYER_SAVE_BYTES` — the wall-6 discipline, applied to
/// a file instead of a packet. The header check below is what makes a
/// forgotten bump a loud refusal instead of a silent reinterpretation of
/// somebody's inventory.
///
/// **1 → 2 at research v0**: `PlayerSave` grew the blueprint mask, so the
/// record went 260 → 268 bytes. There is no migrator by design — a save
/// written by an older build is moved aside, which is what a wipe is.
/// **3 — an inventory slot carries its condition** (item durability v0):
/// `PlayerSave`'s slot stride 4 → 6 B, 196 → 256 per record, and the
/// canonical-empty rule widens with it (`persist.rs`).
/// **4 — a body saves what it is wearing** (armor v0): `PlayerSave` grew
/// `worn`, `WEAR_SLOTS` stacks at the inventory's own six-byte stride, so
/// the record went 256 → 268 per player. The layout moved and nothing on
/// disk announces it, which is the whole reason this number exists.
pub const SAVE_FORMAT: u16 = 4;

/// Header size. Fixed so record `i` is at a computable offset.
pub const SAVE_HEADER_BYTES: usize = 48;

/// One record: stamp, key, the save, and a checksum over all of it.
pub const SAVE_RECORD_BYTES: usize = 8 + 1 + PLAYER_KEY_MAX_BYTES + 7 + PLAYER_SAVE_BYTES + 8;

/// Field offsets inside one record. Named rather than inlined because the
/// trap list is explicit that a positional payload is where this class of
/// code actually bleeds — the right value at the wrong offset, which no
/// round-trip test can see.
const REC_STAMP: usize = 0;
const REC_KEY_LEN: usize = 8;
const REC_KEY: usize = 9;
const REC_SAVE: usize = REC_KEY + PLAYER_KEY_MAX_BYTES + 7;
const REC_SUM: usize = REC_SAVE + PLAYER_SAVE_BYTES;

/// An opaque player identity, as far as this game is concerned.
///
/// Stored **verbatim**, never hashed. Two reasons, and the second is the
/// load-bearing one: equal keys mean equal identities by construction, so
/// there is no collision question to reason about; and a hash would make the
/// store's contents unreadable to the operator who has to support it without
/// buying any secrecy, because a key is not a secret. That last clause is a
/// *requirement on the admission seam*, not an assumption — see
/// `auth::verify`, which returns a public address and must never return a
/// credential.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct PlayerKey {
    bytes: [u8; PLAYER_KEY_MAX_BYTES],
    len: u8,
}

/// Prints its length and never its bytes. A key is not a credential, but it
/// *names a person*, and this type crosses `Debug` boundaries in test output
/// and error reporting often enough that the derive would eventually put
/// somebody's identity in a log.
impl std::fmt::Debug for PlayerKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PlayerKey({} bytes)", self.len)
    }
}

impl PlayerKey {
    /// Wrap identity bytes. `None` ⇒ empty or over the cap: an empty key is
    /// how a record says "this slot holds nobody", so it cannot also be a
    /// player, and a truncated key would file two different players under
    /// one save.
    pub fn new(raw: &[u8]) -> Option<Self> {
        if raw.is_empty() || raw.len() > PLAYER_KEY_MAX_BYTES {
            return None;
        }
        let mut bytes = [0; PLAYER_KEY_MAX_BYTES];
        bytes[..raw.len()].copy_from_slice(raw);
        Some(Self {
            bytes,
            len: raw.len() as u8,
        })
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len as usize]
    }

    /// A key that names nobody, for sizing a buffer the caller is about to
    /// overwrite. **Not a valid key** — `new` refuses an empty one, and an
    /// empty key is how a record on disk says "this slot holds nobody" — so
    /// it can never be mistaken for a player. It exists because the world
    /// save's identity buffer is preallocated on the sim thread (wall 2)
    /// and `PlayerKey` has no `Default`, deliberately: a type that named
    /// somebody by accident is exactly what this one is designed against.
    pub const PLACEHOLDER: Self = Self {
        bytes: [0; PLAYER_KEY_MAX_BYTES],
        len: 0,
    };
}

/// What a load found. Reported at boot rather than counted in `ShardStats`,
/// because a corrupt record is a boot-time fact about a file and not a
/// running rate — an operator wants it in the line that says the shard came
/// up, once.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SaveLoad {
    /// Records loaded and admitted.
    pub live: usize,
    /// Records refused: a bad checksum (a torn write), a field the sim's
    /// own validator would not accept, or a condition the loaded content
    /// could not have minted (`crate::cond` — the decoder runs without the
    /// content tables, so the ceiling check has to happen here, where they
    /// are in scope). The slot is treated as empty and is free to be
    /// reused, so a corrupt record costs one player their save and nothing
    /// else.
    pub corrupt: usize,
    /// True when the file did not exist and was created empty.
    pub created: bool,
}

/// One table entry. `key: None` ⇒ free.
#[derive(Clone, Copy)]
struct Entry {
    key: Option<PlayerKey>,
    stamp: u64,
    save: PlayerSave,
}

/// The index: key → save, owned by exactly one thread.
///
/// **One owner, named in code rather than in a comment** — the accept loop,
/// which is the only thing that both needs to read it (at a join) and hears
/// about every write (off the sim's ring). That is why there is no lock in
/// here and none is owed: `crates/server/clippy.toml` bans `Mutex` crate-wide
/// and the ban held, because the ownership question had an answer. The file
/// half ([`SaveFile`]) is a separate type for the same reason — it is owned
/// by the storage thread, and the two never touch each other's state.
pub struct SaveStore {
    slots: Box<[Entry]>,
}

/// Where a `put` landed, and whether somebody was pushed out to make room.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Put {
    /// Record index the storage thread must write.
    pub index: usize,
    /// A different player's save was overwritten (the table was full).
    pub evicted: bool,
}

impl Default for SaveStore {
    fn default() -> Self {
        Self::new()
    }
}

impl SaveStore {
    /// An empty table. What a shard with no `save_file` runs: every lookup
    /// misses, so every join is a fresh character — today's behaviour,
    /// unchanged, which is what keeps every existing test hermetic.
    pub fn new() -> Self {
        let mut v = Vec::with_capacity(MAX_SAVED_PLAYERS);
        v.resize(
            MAX_SAVED_PLAYERS,
            Entry {
                key: None,
                stamp: 0,
                save: PlayerSave::EMPTY,
            },
        );
        Self {
            slots: v.into_boxed_slice(),
        }
    }

    /// The save filed under `key`, if any. A linear scan, deliberately: it
    /// runs once per join (not per tick), it needs no hasher and therefore
    /// cannot be made nondeterministic by one, and 4096 32-byte compares is
    /// noise beside the QUIC handshake it happens behind.
    pub fn find(&self, key: &PlayerKey) -> Option<PlayerSave> {
        self.slots
            .iter()
            .find(|e| e.key.as_ref() == Some(key))
            .map(|e| e.save)
    }

    /// File `save` under `key` at `stamp`, and say which record to write.
    ///
    /// Overflow policy, stated where it is implemented: the player's own
    /// slot if they have one, else a free slot, else **the least recently
    /// saved slot in the table**. Eviction always succeeds — the table is
    /// non-empty and `stamp` is now — so this cannot fail, and a caller has
    /// no refusal to handle.
    pub fn put(&mut self, key: &PlayerKey, stamp: u64, save: PlayerSave) -> Put {
        let mine = self.slots.iter().position(|e| e.key.as_ref() == Some(key));
        let (index, evicted) = match mine {
            Some(i) => (i, false),
            None => match self.slots.iter().position(|e| e.key.is_none()) {
                Some(i) => (i, false),
                None => {
                    // Least recently saved. `min_by_key` over a slice is a
                    // deterministic scan — no map iteration anywhere near
                    // this, which is what wall 1's sibling ban is about.
                    let i = self
                        .slots
                        .iter()
                        .enumerate()
                        .min_by_key(|(_, e)| e.stamp)
                        .map(|(i, _)| i)
                        .expect("the table is never empty");
                    (i, true)
                }
            },
        };
        self.slots[index] = Entry {
            key: Some(*key),
            stamp,
            save,
        };
        Put { index, evicted }
    }

    /// How many players this shard remembers.
    pub fn live(&self) -> usize {
        self.slots.iter().filter(|e| e.key.is_some()).count()
    }
}

/// The file half: one open handle, owned by the storage thread and nothing
/// else. `None` ⇒ persistence is off, and every write is a no-op rather
/// than an error — a shard with no `save_file` is a shard that was told not
/// to remember, not a shard that is failing to.
pub struct SaveFile {
    file: Option<File>,
}

impl Default for SaveFile {
    fn default() -> Self {
        Self::closed()
    }
}

impl SaveFile {
    /// Persistence off.
    pub fn closed() -> Self {
        Self { file: None }
    }

    pub fn is_open(&self) -> bool {
        self.file.is_some()
    }

    /// Write one record in place and flush it. `sync_data` per record and
    /// not per batch: a save is written at a leave, a shutdown, or an
    /// autosave sweep tick — never in the tick's hot path and never at a
    /// rate a disk notices — and the failure this guards against is the
    /// shard's own crash, which is the one moment a buffered write is worth
    /// nothing.
    ///
    /// Returns whether bytes actually reached a disk. `Ok(false)` ⇒ there is
    /// no file: a no-op rather than an error, because a shard told not to
    /// remember is not a shard failing to. **The bool is the whole reason this
    /// is not `Ok(())`** — a caller that counted every `Ok` would report
    /// `saves_written` climbing on a shard that has never opened a file, and a
    /// counter that lies about persistence is worse than no counter.
    pub fn write(
        &mut self,
        index: usize,
        key: &PlayerKey,
        stamp: u64,
        save: &PlayerSave,
    ) -> std::io::Result<bool> {
        let Some(file) = self.file.as_mut() else {
            return Ok(false);
        };
        let mut rec = [0u8; SAVE_RECORD_BYTES];
        encode_record(&mut rec, key, stamp, save);
        file.seek(SeekFrom::Start(record_offset(index)))?;
        file.write_all(&rec)?;
        file.sync_data()?;
        Ok(true)
    }
}

/// The two halves together, as one boot artifact. Split by owner the moment
/// the shard starts: the index goes to the accept loop, the file to the
/// storage thread, and neither can reach the other's state again.
pub struct Saves {
    pub store: SaveStore,
    pub file: SaveFile,
}

/// Counts and a flag, never contents — the same rule `PlayerKey`'s `Debug`
/// follows one level down. Hand-written rather than derived because a derive
/// here would print `MAX_SAVED_PLAYERS` players' inventories into whatever
/// asked, and the thing that asks is usually a failing test.
impl std::fmt::Debug for Saves {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Saves({} remembered, file {})",
            self.store.live(),
            if self.file.is_open() { "open" } else { "off" }
        )
    }
}

impl Default for Saves {
    fn default() -> Self {
        Self::off()
    }
}

impl Saves {
    /// Persistence off — an empty index and no file. What every shard with
    /// no `save_file` runs, and what every test in this repo runs.
    pub fn off() -> Self {
        Self {
            store: SaveStore::new(),
            file: SaveFile::closed(),
        }
    }
}

fn record_offset(index: usize) -> u64 {
    (SAVE_HEADER_BYTES + index * SAVE_RECORD_BYTES) as u64
}

fn encode_record(
    rec: &mut [u8; SAVE_RECORD_BYTES],
    key: &PlayerKey,
    stamp: u64,
    save: &PlayerSave,
) {
    rec[REC_STAMP..REC_STAMP + 8].copy_from_slice(&stamp.to_le_bytes());
    rec[REC_KEY_LEN] = key.len;
    rec[REC_KEY..REC_KEY + key.len as usize].copy_from_slice(key.as_bytes());
    let mut body = [0u8; PLAYER_SAVE_BYTES];
    save.write_le(&mut body);
    rec[REC_SAVE..REC_SAVE + PLAYER_SAVE_BYTES].copy_from_slice(&body);
    let sum = xxh3_64(&rec[..REC_SUM]);
    rec[REC_SUM..REC_SUM + 8].copy_from_slice(&sum.to_le_bytes());
}

/// Decode one record. `Ok(None)` ⇒ the slot holds nobody; `Err` ⇒ the bytes
/// are there but wrong, which is a counted corruption and not a boot
/// failure.
///
/// The checksum catches a torn write and nothing else: anybody who can edit
/// this file can recompute an xxh3. What protects the *sim* from a
/// hand-edited record is `PlayerSave::read_le`'s validation, which runs
/// after it.
fn decode_record(
    rec: &[u8; SAVE_RECORD_BYTES],
) -> Result<Option<(PlayerKey, u64, PlayerSave)>, ()> {
    let key_len = rec[REC_KEY_LEN] as usize;
    if key_len == 0 {
        // A free slot is all zeros, so it has no valid checksum and must be
        // recognised before one is asked for.
        return Ok(None);
    }
    if key_len > PLAYER_KEY_MAX_BYTES {
        return Err(());
    }
    let want = u64::from_le_bytes(
        rec[REC_SUM..REC_SUM + 8]
            .try_into()
            .expect("8 bytes from a fixed range"),
    );
    if xxh3_64(&rec[..REC_SUM]) != want {
        return Err(());
    }
    let key = PlayerKey::new(&rec[REC_KEY..REC_KEY + key_len]).ok_or(())?;
    let stamp = u64::from_le_bytes(
        rec[REC_STAMP..REC_STAMP + 8]
            .try_into()
            .expect("8 bytes from a fixed range"),
    );
    let mut body = [0u8; PLAYER_SAVE_BYTES];
    body.copy_from_slice(&rec[REC_SAVE..REC_SAVE + PLAYER_SAVE_BYTES]);
    let save = PlayerSave::read_le(&body).map_err(|_| ())?;
    Ok(Some((key, stamp, save)))
}

fn encode_header(seed: u64, content_hash: u64) -> [u8; SAVE_HEADER_BYTES] {
    let mut h = [0u8; SAVE_HEADER_BYTES];
    h[0..8].copy_from_slice(&SAVE_MAGIC);
    h[8..10].copy_from_slice(&SAVE_FORMAT.to_le_bytes());
    h[10..12].copy_from_slice(&(SAVE_RECORD_BYTES as u16).to_le_bytes());
    h[12..16].copy_from_slice(&(MAX_SAVED_PLAYERS as u32).to_le_bytes());
    h[16..24].copy_from_slice(&seed.to_le_bytes());
    h[24..32].copy_from_slice(&content_hash.to_le_bytes());
    h
}

/// Rotate the backups: `<path>.1` becomes `.2`, and the file we are about to
/// open becomes `.1`. Higher number = older, which is the reference game's
/// convention (`....sav.1`, `....sav.2`) and worth matching so an operator who
/// knows theirs knows ours.
///
/// **Best-effort by design.** Every failure here is ignored, because the shard
/// booting matters more than the backup existing: a read-only directory, a full
/// disk or a missing predecessor must not stop players getting in. The
/// consequence is that a backup is a *convenience*, never a guarantee — and the
/// per-record checksum, which is not best-effort, is what actually bounds the
/// damage from a torn write.
fn rotate_backups(path: &Path) {
    let at = |n: usize| {
        let mut s = path.as_os_str().to_os_string();
        s.push(format!(".{n}"));
        std::path::PathBuf::from(s)
    };
    // Oldest first, so nothing is overwritten before it has been moved up.
    for n in (1..SAVE_BACKUP_COUNT).rev() {
        let _ = std::fs::rename(at(n), at(n + 1));
    }
    let _ = std::fs::copy(path, at(1));
}

/// Open (or create) the save file and load its records.
///
/// Every refusal names what to do about it, because every one of them is an
/// operator mistake with a one-command fix and the alternative is a hundred
/// players silently starting over.
///
/// A boot rotates the backups first (`rotate_backups`), so `<path>.1` is the
/// world as the previous run left it. **The reason to have more than one is not
/// the obvious one:** the reference game's own recovery documentation warns that
/// `.sav.1` may itself be corrupt, because corruption can predate the newest
/// backup by several save cycles — which is why a depth of one is not worth
/// having and 2 is their documented minimum (`reference/SAVES.md` §6).
///
/// `gather` is the baked content this shard runs, and it is here for one
/// check the decoder cannot make: a record whose condition no command could
/// have minted (`crate::cond` carries the refuse-not-clamp policy) is
/// counted corrupt, exactly as a torn write is. `content_hash` already
/// pinned the content this file was written under, so the ceilings asked of
/// `gather` are the ceilings the save was played under.
pub fn open(
    path: &Path,
    seed: u64,
    content_hash: u64,
    gather: &sim_core::gather::GatherContent,
) -> Result<(Saves, SaveLoad), String> {
    // Before the handle is taken, and before any validation: a file this boot
    // is about to refuse is exactly the file an operator most wants a copy of.
    if path.exists() {
        rotate_backups(path);
    }
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(|e| format!("save file {}: {e}", path.display()))?;
    let len = file
        .metadata()
        .map_err(|e| format!("save file {}: {e}", path.display()))?
        .len();

    let mut store = SaveStore::new();
    let want_len = record_offset(MAX_SAVED_PLAYERS);

    if len == 0 {
        // First boot on this file: lay down the header and the empty table.
        // Written in full rather than left sparse so a later in-place record
        // write can never land past the end of the file.
        file.write_all(&encode_header(seed, content_hash))
            .map_err(|e| format!("save file {}: writing header: {e}", path.display()))?;
        let blank = [0u8; SAVE_RECORD_BYTES];
        for _ in 0..MAX_SAVED_PLAYERS {
            file.write_all(&blank)
                .map_err(|e| format!("save file {}: writing table: {e}", path.display()))?;
        }
        file.sync_data()
            .map_err(|e| format!("save file {}: {e}", path.display()))?;
        return Ok((
            Saves {
                store,
                file: SaveFile { file: Some(file) },
            },
            SaveLoad {
                live: 0,
                corrupt: 0,
                created: true,
            },
        ));
    }

    // Length before content, or a file too short to hold a header refuses as
    // "failed to fill whole buffer" — an io error about a read, for what is
    // actually a knob pointing at the wrong file.
    if len < SAVE_HEADER_BYTES as u64 {
        return Err(format!(
            "save file {} is {len} bytes, too short to be a gates save file \
             (a header is {SAVE_HEADER_BYTES}) — check the `save_file` path in \
             shard.toml before anything else",
            path.display()
        ));
    }
    let mut head = [0u8; SAVE_HEADER_BYTES];
    file.read_exact(&mut head)
        .map_err(|e| format!("save file {}: reading header: {e}", path.display()))?;
    if head[0..8] != SAVE_MAGIC {
        return Err(format!(
            "save file {} is not a gates save file (bad magic) — check the \
             `save_file` path in shard.toml before anything else",
            path.display()
        ));
    }
    let got_format = u16::from_le_bytes([head[8], head[9]]);
    if got_format != SAVE_FORMAT {
        return Err(format!(
            "save file {} was written in format {got_format}; this shard \
             speaks {SAVE_FORMAT}. There is no migrator: move the file aside \
             (that is what a wipe is) or run a build that speaks its format.",
            path.display()
        ));
    }
    let got_record = u16::from_le_bytes([head[10], head[11]]) as usize;
    if got_record != SAVE_RECORD_BYTES {
        return Err(format!(
            "save file {} has {got_record}-byte records; this build writes \
             {SAVE_RECORD_BYTES}. The record layout moved without the format \
             version — that is the bug, not the file (store.rs SAVE_FORMAT).",
            path.display()
        ));
    }
    let got_cap = u32::from_le_bytes([head[12], head[13], head[14], head[15]]) as usize;
    if got_cap != MAX_SAVED_PLAYERS {
        return Err(format!(
            "save file {} holds {got_cap} slots; this build holds \
             {MAX_SAVED_PLAYERS}. MAX_SAVED_PLAYERS moved, which changes \
             every record offset: move the file aside or restore the old cap.",
            path.display()
        ));
    }
    let got_seed = u64::from_le_bytes(head[16..24].try_into().expect("8 bytes"));
    if got_seed != seed {
        return Err(format!(
            "save file {} was written on seed {got_seed}; this shard runs \
             {seed}. Every saved position is a point on the other island — \
             run the old seed, or move the file aside to start this one clean.",
            path.display()
        ));
    }
    let got_content = u64::from_le_bytes(head[24..32].try_into().expect("8 bytes"));
    if got_content != content_hash {
        return Err(format!(
            "save file {} was written under content {got_content:016x}; this \
             shard loaded {content_hash:016x}. Item and recipe rows are \
             indices, so restoring these inventories under moved content \
             would hand players the wrong items. Restore that content set, or \
             move the file aside — a content change with saves in it is a \
             wipe, and it is the operator's to declare.",
            path.display()
        ));
    }
    if len != want_len {
        return Err(format!(
            "save file {} is {len} bytes; the header describes {want_len}. \
             Truncated or appended to — move it aside rather than reading \
             records out of whatever is there.",
            path.display()
        ));
    }

    let mut found = SaveLoad::default();
    let mut rec = [0u8; SAVE_RECORD_BYTES];
    for index in 0..MAX_SAVED_PLAYERS {
        file.read_exact(&mut rec)
            .map_err(|e| format!("save file {}: reading record {index}: {e}", path.display()))?;
        match decode_record(&rec) {
            Ok(None) => {}
            // The condition wall (`crate::cond`): the decoder bounded every
            // field it could see without content; this is the one it could
            // not. Refused as corrupt, never clamped — the module header
            // there says why, and per-record rather than per-file because
            // one edited save is one player's, not everyone's.
            Ok(Some((_, _, save))) if crate::cond::violation(&save.inv, gather).is_some() => {
                found.corrupt += 1;
            }
            Ok(Some((key, stamp, save))) => {
                found.live += 1;
                store.put(&key, stamp, save);
            }
            Err(()) => found.corrupt += 1,
        }
    }
    Ok((
        Saves {
            store,
            file: SaveFile { file: Some(file) },
        },
        found,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(s: &str) -> PlayerKey {
        PlayerKey::new(s.as_bytes()).expect("fits")
    }

    fn save_with(hp: u16) -> PlayerSave {
        let mut s = PlayerSave::EMPTY;
        s.hp = hp;
        s.hp_max = 100;
        s
    }

    #[test]
    fn a_key_refuses_empty_and_over_long() {
        assert!(PlayerKey::new(b"").is_none());
        assert!(PlayerKey::new(&[7u8; PLAYER_KEY_MAX_BYTES]).is_some());
        assert!(PlayerKey::new(&[7u8; PLAYER_KEY_MAX_BYTES + 1]).is_none());
    }

    /// A key names a person and lives in a file. `Debug` must not print it —
    /// it crosses test output and error reporting too often to be trusted.
    #[test]
    fn debug_never_prints_the_key() {
        let shown = format!("{:?}", key("anthone@example.com"));
        assert!(!shown.contains("anthone"), "key leaked into Debug: {shown}");
        assert!(shown.contains("19 bytes"), "expected a length: {shown}");
    }

    #[test]
    fn a_record_round_trips_through_its_bytes() {
        let (k, want) = (key("player-one"), save_with(71));
        let mut rec = [0u8; SAVE_RECORD_BYTES];
        encode_record(&mut rec, &k, 1_700_000_000, &want);
        let got = decode_record(&rec).expect("decodes").expect("occupied");
        assert_eq!(got.0, k);
        assert_eq!(got.1, 1_700_000_000);
        assert_eq!(got.2, want);
    }

    /// A zeroed record is a free slot, not a corrupt one. This is what a
    /// freshly created file is made of, so getting it wrong would report
    /// 4096 corruptions on every first boot.
    #[test]
    fn a_zeroed_record_reads_as_empty() {
        assert_eq!(decode_record(&[0u8; SAVE_RECORD_BYTES]), Ok(None));
    }

    /// One flipped bit anywhere in the record is caught. Walked across the
    /// whole record rather than at one offset, because a checksum over the
    /// wrong range is the defect this is really guarding against.
    #[test]
    fn a_torn_record_is_caught_at_every_offset() {
        let mut rec = [0u8; SAVE_RECORD_BYTES];
        encode_record(&mut rec, &key("player-one"), 42, &save_with(9));
        for off in 0..REC_SUM {
            let mut bent = rec;
            bent[off] ^= 0x01;
            if bent[REC_KEY_LEN] == 0 {
                continue; // flipping the length to zero says "free slot"
            }
            assert_eq!(
                decode_record(&bent),
                Err(()),
                "a flipped bit at {off} went unnoticed"
            );
        }
    }

    /// A record whose checksum is *right* and whose contents the sim would
    /// refuse is still refused. The checksum is an accident detector; the
    /// validator is the wall.
    #[test]
    fn a_checksummed_but_illegal_record_is_refused() {
        let mut rec = [0u8; SAVE_RECORD_BYTES];
        encode_record(&mut rec, &key("player-one"), 42, &save_with(9));
        // hp over hp_max, then re-checksummed the way an editor would.
        rec[REC_SAVE + 18..REC_SAVE + 20].copy_from_slice(&u16::MAX.to_le_bytes());
        let sum = xxh3_64(&rec[..REC_SUM]);
        rec[REC_SUM..REC_SUM + 8].copy_from_slice(&sum.to_le_bytes());
        assert_eq!(decode_record(&rec), Err(()));
    }

    #[test]
    fn a_put_finds_its_own_slot_again() {
        let mut store = SaveStore::new();
        let (a, b) = (key("a"), key("b"));
        assert_eq!(store.find(&a), None);
        let first = store.put(&a, 1, save_with(10));
        assert!(!first.evicted);
        store.put(&b, 2, save_with(20));
        assert_eq!(store.find(&a).map(|s| s.hp), Some(10));
        assert_eq!(store.find(&b).map(|s| s.hp), Some(20));
        assert_eq!(store.live(), 2);
        // The same key overwrites its own record rather than taking a second.
        let again = store.put(&a, 3, save_with(11));
        assert_eq!(again.index, first.index);
        assert!(!again.evicted);
        assert_eq!(store.find(&a).map(|s| s.hp), Some(11));
        assert_eq!(store.live(), 2);
    }

    /// The stated overflow policy, exercised at the bound: a full table
    /// evicts the least recently saved and says so.
    #[test]
    fn a_full_table_evicts_the_oldest() {
        let mut store = SaveStore::new();
        let mut first_key = None;
        for i in 0..MAX_SAVED_PLAYERS {
            let k = PlayerKey::new(&(i as u32).to_le_bytes()).expect("fits");
            // Stamps ascending, so slot 0 is the oldest.
            store.put(&k, i as u64 + 1, save_with(1));
            if i == 0 {
                first_key = Some(k);
            }
        }
        assert_eq!(store.live(), MAX_SAVED_PLAYERS);
        let oldest = first_key.expect("set");
        assert!(store.find(&oldest).is_some());
        let newcomer = key("someone-new");
        let put = store.put(&newcomer, u64::MAX, save_with(99));
        assert!(put.evicted, "a full table must report the eviction");
        assert_eq!(store.find(&oldest), None, "the oldest save must be gone");
        assert_eq!(store.find(&newcomer).map(|s| s.hp), Some(99));
        assert_eq!(store.live(), MAX_SAVED_PLAYERS);
    }

    /// The offsets add up to the declared record size, and the record is
    /// what the header advertises. Asserted rather than trusted: both
    /// numbers are derived, and a drift between them is exactly the
    /// wrong-offset defect the field constants exist to prevent.
    #[test]
    fn the_layout_is_the_size_the_header_declares() {
        assert_eq!(REC_SUM + 8, SAVE_RECORD_BYTES);
        assert_eq!(REC_SAVE, 64, "the save body's offset moved");
        // 268 → 328 at SAVE_FORMAT 3: the save body grew 60 bytes (an
        // inventory slot is six bytes since item durability v0). 328 → 340
        // at SAVE_FORMAT 4: two worn slots at the same stride (armor v0).
        assert_eq!(SAVE_RECORD_BYTES, 340);
        let head = encode_header(7, 0xdead_beef);
        assert_eq!(
            u16::from_le_bytes([head[10], head[11]]) as usize,
            SAVE_RECORD_BYTES
        );
    }
}
