//! Teams (wire v108): Rust's `RelationshipManager`, on the shard.
//!
//! What the reference does, and so what this does (`reference/MAP.md` §3 and
//! the 2018 Team UI devblog): the leader looks at a player within a few
//! metres and invites them; the invitee accepts or declines; the leader
//! kicks and promotes; a leader who leaves hands the team to the next member
//! and the last one out disbands it. Members see each other on the map
//! wherever they are, talk on their own channel, and carry a green mark. A
//! team **authorizes nothing**: no cupboard, door, lock or turret learns of
//! it, and friendly fire stays on, exactly as in Rust.
//!
//! **Not sim state.** A team is who you talk to and see on the map, the way
//! chat is who hears you, so it lives beside chat on the shard: never a
//! `Command`, never in `World`, the WAL or the hash. Members are player ids;
//! a returning player's woken body takes a new id, which
//! [`Teams::rekey`] carries over. A restart keeps them: each world save
//! writes the teams by player key beside the world file
//! (`<world_file>.teams`, [`Teams::save`]), and the boot reads them back
//! for the sleepers that world restored ([`Teams::load`]). A guest, who has
//! no key, is not kept; a wipe archives the file with the world.
//!
//! Fixed arrays throughout: this runs on the sim thread.

use crate::store::PlayerKey;
use protocol::{
    TeamMember, TeamView, TEAM_MAX, TEAM_OP_ACCEPT, TEAM_OP_DECLINE, TEAM_OP_INVITE, TEAM_OP_KICK,
    TEAM_OP_LEAVE, TEAM_OP_PROMOTE,
};
use sim_core::limits::MAX_PLAYERS;
use std::path::{Path, PathBuf};

/// How far the leader may be from the player they invite: Rust's 7 m
/// look-and-press reach, in centimetres.
pub const INVITE_REACH_CM: i64 = 700;
/// How long an invite waits for an answer, in ticks (60 s). Rust keeps
/// them until answered; a stale one here would only be a prompt nobody
/// remembers asking for.
pub const INVITE_TICKS: u64 = 60 * sim_core::limits::TICK_HZ as u64;
/// The fewest ticks between two views that differ only in where members
/// stand (half a second). A change of who, leader, state or invite goes at
/// once.
pub const MOVE_TICKS: u64 = 15;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Team {
    leader: u32,
    members: [u32; TEAM_MAX],
    len: u8,
}

impl Team {
    fn members(&self) -> &[u32] {
        &self.members[..self.len as usize]
    }

    fn has(&self, id: u32) -> bool {
        self.members().contains(&id)
    }

    fn add(&mut self, id: u32) {
        self.members[self.len as usize] = id;
        self.len += 1;
    }

    /// Take `id` out, keeping the order (the next leader is the oldest
    /// member left, as in Rust) and the tail zeroed.
    fn remove(&mut self, id: u32) {
        let n = self.len as usize;
        if let Some(i) = self.members[..n].iter().position(|&m| m == id) {
            self.members.copy_within(i + 1..n, i);
            self.members[n - 1] = 0;
            self.len -= 1;
            if self.leader == id {
                self.leader = self.members[0];
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Invite {
    to: u32,
    from: u32,
    until: u64,
}

/// What a verb did, for the shard to say back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Done {
    /// A team was made with the asker as leader, and the invite sent.
    Founded,
    Invited,
    Joined,
    Declined,
    Left,
    Kicked,
    Promoted,
}

/// Why a verb did nothing. Each is a line the asker is told.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// The shard runs with teams off (`team_max` 0 or 1).
    Off,
    NotLeader,
    Full,
    /// The invitee already has a team.
    Taken,
    /// The invitee is not here, not awake, not alive or not a player.
    NoOne,
    TooFar,
    NoInvite,
    /// The asker already has a team (accept) or has none (leave, kick).
    HaveTeam,
    NoTeam,
    NotMember,
    Unknown,
}

impl Refused {
    pub fn line(self) -> &'static str {
        match self {
            Refused::Off => "team: teams are off on this shard",
            Refused::NotLeader => "team: only the leader can do that",
            Refused::Full => "team: the team is full",
            Refused::Taken => "team: they are already on a team",
            Refused::NoOne => "team: nobody to do that with",
            Refused::TooFar => "team: get closer to them",
            Refused::NoInvite => "team: no invite waiting",
            Refused::HaveTeam => "team: leave your team first",
            Refused::NoTeam => "team: you are not on a team",
            Refused::NotMember => "team: they are not on your team",
            Refused::Unknown => "team: invite, accept, leave, kick, promote",
        }
    }
}

/// Every team on the shard and every invite waiting.
pub struct Teams {
    /// At most one team per player, so `MAX_PLAYERS` slots is enough by
    /// construction. A slot with `len == 0` is free.
    teams: Box<[Team; MAX_PLAYERS]>,
    /// One waiting invite per invitee: a new one replaces the old.
    invites: [Invite; MAX_PLAYERS],
    /// The most members a team may have: `shard.toml` `team_max`, at most
    /// the wire's `TEAM_MAX`. Below 2 there are no teams.
    cap: u8,
    /// Where the teams are kept across a restart, if the world is, and
    /// what was last written there.
    file: Option<PathBuf>,
    written: Option<String>,
}

/// The teams' file beside the world save.
pub fn teams_path(base: &Path) -> PathBuf {
    PathBuf::from(format!("{}.teams", base.display()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok())
        .collect()
}

impl Default for Teams {
    fn default() -> Self {
        Self::new(TEAM_MAX as u8)
    }
}

impl Teams {
    pub fn new(cap: u8) -> Self {
        Self {
            teams: Box::new([Team::default(); MAX_PLAYERS]),
            invites: [Invite::default(); MAX_PLAYERS],
            cap: cap.min(TEAM_MAX as u8),
            file: None,
            written: None,
        }
    }

    /// The same, kept in `file` across a restart.
    pub fn open(cap: u8, file: Option<PathBuf>) -> Self {
        Self {
            file,
            ..Self::new(cap)
        }
    }

    /// The teams by player key: one team per line, the leader first, each
    /// key in hex. A member `keys` has no key for is left out, and a team
    /// left with fewer than two is not written.
    pub fn to_text(&self, keys: &[(PlayerKey, u32)]) -> String {
        let key_of = |id: u32| keys.iter().find(|(_, have)| *have == id).map(|(k, _)| k);
        let mut out = String::new();
        for t in self.teams.iter().filter(|t| t.len > 1) {
            let leader = std::iter::once(t.leader);
            let rest = t.members().iter().copied().filter(|&m| m != t.leader);
            let line: Vec<String> = leader
                .chain(rest)
                .filter_map(key_of)
                .map(|k| hex(k.as_bytes()))
                .collect();
            if line.len() > 1 {
                out.push_str(&line.join(" "));
                out.push('\n');
            }
        }
        out
    }

    /// Rebuild the teams [`Teams::to_text`] wrote, for the players `keys`
    /// names now (the world's restored sleepers). A key nobody holds, a
    /// player already placed and any member past the cap are left out; a
    /// team left with fewer than two is not made. The first member left is
    /// the leader. Returns how many teams were made.
    pub fn restore(&mut self, text: &str, keys: &[(PlayerKey, u32)]) -> usize {
        if self.cap < 2 {
            return 0;
        }
        let mut made = 0;
        for line in text.lines() {
            let mut t = Team::default();
            for word in line.split_whitespace() {
                let Some(id) = unhex(word)
                    .and_then(|raw| PlayerKey::new(&raw))
                    .and_then(|k| keys.iter().find(|(have, _)| *have == k))
                    .map(|(_, id)| *id)
                else {
                    continue;
                };
                if t.len < self.cap && !t.has(id) && self.index_of(id).is_none() {
                    t.add(id);
                }
            }
            if t.len < 2 {
                continue;
            }
            t.leader = t.members[0];
            let Some(free) = self.teams.iter().position(|f| f.len == 0) else {
                break;
            };
            self.teams[free] = t;
            made += 1;
        }
        made
    }

    /// The teams' file for the store thread, if they changed since the last
    /// one handed over (`net.rs` take_world_save, on the world save's
    /// cadence). The sim thread makes the text; the store thread writes it
    /// (temp, fsync, rename). `None` with no file or no change. The caller
    /// checks the ring for room first, so a taken file is never dropped.
    pub fn take_write(&mut self, keys: &[(PlayerKey, u32)]) -> Option<crate::slot::FileMsg> {
        let path = self.file.clone()?;
        let text = self.to_text(keys);
        if self.written.as_ref() == Some(&text) {
            return None;
        }
        self.written = Some(text.clone());
        Some(crate::slot::FileMsg { path, text })
    }

    /// Read the teams back from their file at boot, for `keys`. No file
    /// is no teams.
    pub fn load(&mut self, keys: &[(PlayerKey, u32)]) -> std::io::Result<usize> {
        let Some(path) = &self.file else {
            return Ok(0);
        };
        match std::fs::read_to_string(path) {
            Ok(text) => {
                let made = self.restore(&text, keys);
                self.written = Some(text);
                Ok(made)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(0),
            Err(e) => Err(e),
        }
    }

    fn index_of(&self, id: u32) -> Option<usize> {
        if id == 0 {
            return None;
        }
        self.teams.iter().position(|t| t.len > 0 && t.has(id))
    }

    /// Is `a` on `b`'s team (and not alone)? A player is their own teammate.
    pub fn mates(&self, a: u32, b: u32) -> bool {
        a == b || self.index_of(a).is_some_and(|i| self.teams[i].has(b))
    }

    /// The members of `id`'s team, `id` included; empty for none.
    pub fn team_of(&self, id: u32) -> &[u32] {
        match self.index_of(id) {
            Some(i) => self.teams[i].members(),
            None => &[],
        }
    }

    fn invite_for(&self, to: u32, tick: u64) -> Option<&Invite> {
        self.invites
            .iter()
            .find(|v| v.to == to && v.to != 0 && tick < v.until)
    }

    /// Run one verb. `can_invite(target)` is the shard's word on the
    /// target: `Ok` for an awake, living player within reach, else why not.
    pub fn op(
        &mut self,
        actor: u32,
        op: u8,
        target: u32,
        tick: u64,
        can_invite: impl FnOnce(u32) -> Result<(), Refused>,
    ) -> Result<Done, Refused> {
        if actor == 0 {
            return Err(Refused::NoOne);
        }
        match op {
            TEAM_OP_INVITE => {
                if self.cap < 2 {
                    return Err(Refused::Off);
                }
                if target == 0 || target == actor {
                    return Err(Refused::NoOne);
                }
                let mine = self.index_of(actor);
                if let Some(i) = mine {
                    if self.teams[i].leader != actor {
                        return Err(Refused::NotLeader);
                    }
                    if self.teams[i].len >= self.cap {
                        return Err(Refused::Full);
                    }
                    if self.teams[i].has(target) {
                        return Err(Refused::Taken);
                    }
                }
                if self.index_of(target).is_some() {
                    return Err(Refused::Taken);
                }
                can_invite(target)?;
                let founded = if mine.is_none() {
                    let Some(free) = self.teams.iter().position(|t| t.len == 0) else {
                        return Err(Refused::Full);
                    };
                    let t = &mut self.teams[free];
                    *t = Team::default();
                    t.leader = actor;
                    t.add(actor);
                    true
                } else {
                    false
                };
                let slot = self
                    .invites
                    .iter()
                    .position(|v| v.to == target)
                    .or_else(|| {
                        self.invites
                            .iter()
                            .position(|v| v.to == 0 || tick >= v.until)
                    });
                // One slot per player, so a free one always exists while
                // fewer than `MAX_PLAYERS` are waiting; past that the
                // oldest goes, as Rust drops the oldest of its list.
                let slot = slot.unwrap_or_else(|| {
                    (0..MAX_PLAYERS)
                        .min_by_key(|&i| self.invites[i].until)
                        .unwrap_or(0)
                });
                self.invites[slot] = Invite {
                    to: target,
                    from: actor,
                    until: tick + INVITE_TICKS,
                };
                Ok(if founded {
                    Done::Founded
                } else {
                    Done::Invited
                })
            }
            TEAM_OP_ACCEPT => {
                if self.index_of(actor).is_some() {
                    return Err(Refused::HaveTeam);
                }
                let Some(inv) = self
                    .invite_for(actor, tick)
                    .copied()
                    .filter(|v| target == 0 || v.from == target)
                else {
                    return Err(Refused::NoInvite);
                };
                self.clear_invite(actor);
                let Some(i) = self.index_of(inv.from) else {
                    return Err(Refused::NoInvite);
                };
                if self.teams[i].len >= self.cap {
                    return Err(Refused::Full);
                }
                self.teams[i].add(actor);
                Ok(Done::Joined)
            }
            TEAM_OP_DECLINE => {
                if self.invite_for(actor, tick).is_none() {
                    return Err(Refused::NoInvite);
                }
                self.clear_invite(actor);
                Ok(Done::Declined)
            }
            TEAM_OP_LEAVE => {
                let Some(i) = self.index_of(actor) else {
                    return Err(Refused::NoTeam);
                };
                self.teams[i].remove(actor);
                self.settle(i);
                Ok(Done::Left)
            }
            TEAM_OP_KICK | TEAM_OP_PROMOTE => {
                let Some(i) = self.index_of(actor) else {
                    return Err(Refused::NoTeam);
                };
                if self.teams[i].leader != actor {
                    return Err(Refused::NotLeader);
                }
                if target == actor || !self.teams[i].has(target) {
                    return Err(Refused::NotMember);
                }
                if op == TEAM_OP_KICK {
                    self.teams[i].remove(target);
                    self.settle(i);
                    Ok(Done::Kicked)
                } else {
                    self.teams[i].leader = target;
                    Ok(Done::Promoted)
                }
            }
            _ => Err(Refused::Unknown),
        }
    }

    /// The last one out disbands the team. A leader left alone keeps it,
    /// as in Rust: a team of one can still invite.
    fn settle(&mut self, i: usize) {
        if self.teams[i].len == 0 {
            self.teams[i] = Team::default();
        }
    }

    fn clear_invite(&mut self, to: u32) {
        for v in self.invites.iter_mut() {
            if v.to == to {
                *v = Invite::default();
            }
        }
    }

    /// A woken body took a new id (`World::take_over`): it is still the same
    /// player, so their seat and any invite to or from them follow.
    pub fn rekey(&mut self, old: u32, new: u32) {
        if old == 0 || new == 0 || old == new {
            return;
        }
        for t in self.teams.iter_mut() {
            for m in t.members.iter_mut() {
                if *m == old {
                    *m = new;
                }
            }
            if t.leader == old {
                t.leader = new;
            }
        }
        for v in self.invites.iter_mut() {
            if v.to == old {
                v.to = new;
            }
            if v.from == old {
                v.from = new;
            }
        }
    }

    /// A body that is gone from the world (evicted, or a guest who left)
    /// leaves its team; `here(id)` is the shard's word on who still exists.
    pub fn prune(&mut self, tick: u64, mut here: impl FnMut(u32) -> bool) {
        for i in 0..MAX_PLAYERS {
            let n = self.teams[i].len as usize;
            if n == 0 {
                continue;
            }
            let mut gone = [0u32; TEAM_MAX];
            let mut k = 0;
            for &m in self.teams[i].members() {
                if !here(m) {
                    gone[k] = m;
                    k += 1;
                }
            }
            for &m in &gone[..k] {
                self.teams[i].remove(m);
            }
            if k > 0 {
                self.settle(i);
            }
        }
        for v in self.invites.iter_mut() {
            if v.to != 0 && (tick >= v.until || !here(v.to) || !here(v.from)) {
                *v = Invite::default();
            }
        }
    }

    /// What `id` is shown: their team, each member's state and position as
    /// `member(id)` reads them, and any invite waiting on them.
    pub fn view(
        &self,
        id: u32,
        tick: u64,
        mut member: impl FnMut(u32) -> (u8, u16, u16),
    ) -> TeamView {
        let mut v = TeamView::NONE;
        v.invite = self.invite_for(id, tick).map_or(0, |i| i.from);
        if let Some(i) = self.index_of(id) {
            let t = &self.teams[i];
            v.leader = t.leader;
            v.count = t.len;
            for (out, &m) in v.members.iter_mut().zip(t.members()) {
                let (state, x_m, z_m) = member(m);
                *out = TeamMember {
                    id: m,
                    state,
                    x_m,
                    z_m,
                };
            }
        }
        v
    }
}

/// Two views that differ only in where members stand: `MOVE_TICKS` may
/// hold the newer one back.
pub fn moved_only(a: &TeamView, b: &TeamView) -> bool {
    a.leader == b.leader
        && a.invite == b.invite
        && a.count == b.count
        && a.members()
            .iter()
            .zip(b.members())
            .all(|(x, y)| x.id == y.id && x.state == y.state)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Teams written by key come back for the same players under the ids
    /// a restored world gave them; a guest, and a player the world did not
    /// restore, are left out, and a team of one is not made.
    #[test]
    fn teams_come_back_by_key_after_a_restart() {
        let key = |s: &str| PlayerKey::new(s.as_bytes()).unwrap();
        let mut teams = Teams::new(4);
        let ok = |_| Ok(());
        // 1 leads 2 and 3; 4 leads 5 (a guest, no key).
        for (leader, member) in [(1, 2), (1, 3), (4, 5)] {
            teams.op(leader, TEAM_OP_INVITE, member, 0, ok).unwrap();
            teams.op(member, TEAM_OP_ACCEPT, leader, 0, ok).unwrap();
        }
        let before = [
            (key("0xaa"), 1),
            (key("0xbb"), 2),
            (key("0xcc"), 3),
            (key("0xdd"), 4),
        ];
        let text = teams.to_text(&before);
        assert_eq!(
            text.lines().count(),
            1,
            "the guest's team is one keyed player: {text}"
        );
        // A restart: new ids, and 0xcc's body is gone.
        let after = [(key("0xbb"), 20), (key("0xaa"), 10), (key("0xdd"), 40)];
        let mut back = Teams::new(4);
        assert_eq!(back.restore(&text, &after), 1);
        assert_eq!(back.team_of(10), &[10, 20]);
        assert!(back.team_of(40).is_empty());
        // Nothing to restore for an empty file or junk.
        assert_eq!(Teams::new(4).restore("", &after), 0);
        assert_eq!(Teams::new(4).restore("zz 0q\n", &after), 0);
        // Teams off: nothing is made.
        assert_eq!(Teams::new(1).restore(&text, &after), 0);
        // Through the file: saved, read back at boot.
        let dir = std::env::temp_dir().join(format!("gates-teams-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = teams_path(&dir.join("island.world"));
        let _ = std::fs::remove_file(&path);
        let mut kept = Teams::open(4, Some(path.clone()));
        assert_eq!(kept.load(&after).unwrap(), 0, "no file is no teams");
        teams.file = Some(path.clone());
        let msg = teams.take_write(&before).expect("the teams changed");
        assert!(
            teams.take_write(&before).is_none(),
            "unchanged: nothing to write"
        );
        assert_eq!(
            (msg.path.as_path(), msg.text.as_str()),
            (path.as_path(), text.as_str())
        );
        crate::gifts::write_atomic(&msg.path, &msg.text).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
        assert_eq!(kept.load(&after).unwrap(), 1);
        assert_eq!(kept.team_of(20), &[10, 20]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    const A: u32 = 0x0100;
    const B: u32 = 0x0201;
    const C: u32 = 0x0302;
    const D: u32 = 0x0403;

    fn ok(_: u32) -> Result<(), Refused> {
        Ok(())
    }

    fn team(t: &mut Teams, leader: u32, others: &[u32]) {
        for &o in others {
            t.op(leader, TEAM_OP_INVITE, o, 0, ok).unwrap();
            assert_eq!(t.op(o, TEAM_OP_ACCEPT, leader, 1, ok), Ok(Done::Joined));
        }
    }

    #[test]
    fn invite_founds_a_team_and_accept_joins_it() {
        let mut t = Teams::default();
        assert_eq!(t.op(A, TEAM_OP_INVITE, B, 0, ok), Ok(Done::Founded));
        assert_eq!(t.team_of(A), &[A]);
        assert_eq!(t.view(B, 1, |_| (0, 0, 0)).invite, A);
        assert_eq!(t.op(B, TEAM_OP_ACCEPT, 0, 1, ok), Ok(Done::Joined));
        assert_eq!(t.team_of(B), &[A, B]);
        assert!(t.mates(A, B) && t.mates(B, A));
        assert!(!t.mates(A, C));
        let v = t.view(B, 2, |id| (0, (id & 0xff) as u16, 7));
        assert_eq!((v.leader, v.count, v.invite), (A, 2, 0));
        assert_eq!(v.members()[1].x_m, 1);
    }

    #[test]
    fn only_the_leader_invites_and_a_full_team_refuses() {
        let mut t = Teams::new(3);
        team(&mut t, A, &[B, C]);
        assert_eq!(t.op(B, TEAM_OP_INVITE, D, 2, ok), Err(Refused::NotLeader));
        assert_eq!(t.op(A, TEAM_OP_INVITE, D, 2, ok), Err(Refused::Full));
        assert_eq!(t.op(D, TEAM_OP_INVITE, A, 2, ok), Err(Refused::Taken));
        let mut off = Teams::new(1);
        assert_eq!(off.op(A, TEAM_OP_INVITE, B, 0, ok), Err(Refused::Off));
    }

    #[test]
    fn the_shards_word_on_the_target_stops_an_invite_before_a_team_is_made() {
        let mut t = Teams::default();
        let far = |_| Err(Refused::TooFar);
        assert_eq!(t.op(A, TEAM_OP_INVITE, B, 0, far), Err(Refused::TooFar));
        assert!(t.team_of(A).is_empty());
    }

    #[test]
    fn invites_expire_and_decline_clears_them() {
        let mut t = Teams::default();
        t.op(A, TEAM_OP_INVITE, B, 0, ok).unwrap();
        assert_eq!(
            t.op(B, TEAM_OP_ACCEPT, A, INVITE_TICKS, ok),
            Err(Refused::NoInvite)
        );
        t.op(A, TEAM_OP_INVITE, B, 10, ok).unwrap();
        assert_eq!(t.op(B, TEAM_OP_ACCEPT, C, 11, ok), Err(Refused::NoInvite));
        assert_eq!(t.op(B, TEAM_OP_DECLINE, 0, 11, ok), Ok(Done::Declined));
        assert_eq!(t.op(B, TEAM_OP_ACCEPT, A, 12, ok), Err(Refused::NoInvite));
    }

    #[test]
    fn a_leader_who_leaves_hands_on_and_the_last_one_out_disbands() {
        let mut t = Teams::default();
        team(&mut t, A, &[B, C]);
        assert_eq!(t.op(A, TEAM_OP_LEAVE, 0, 3, ok), Ok(Done::Left));
        let v = t.view(C, 3, |_| (0, 0, 0));
        assert_eq!((v.leader, v.count), (B, 2));
        assert_eq!(t.op(B, TEAM_OP_KICK, C, 4, ok), Ok(Done::Kicked));
        // B alone keeps the team, and is the last one out.
        assert_eq!(t.team_of(B), &[B]);
        assert_eq!(t.op(B, TEAM_OP_LEAVE, 0, 5, ok), Ok(Done::Left));
        assert!(t.team_of(B).is_empty());
        assert_eq!(t.view(B, 5, |_| (0, 0, 0)), TeamView::NONE);
    }

    #[test]
    fn promote_hands_the_lead_and_kick_needs_it() {
        let mut t = Teams::default();
        team(&mut t, A, &[B]);
        assert_eq!(t.op(B, TEAM_OP_KICK, A, 2, ok), Err(Refused::NotLeader));
        assert_eq!(t.op(A, TEAM_OP_PROMOTE, C, 2, ok), Err(Refused::NotMember));
        assert_eq!(t.op(A, TEAM_OP_PROMOTE, B, 2, ok), Ok(Done::Promoted));
        assert_eq!(t.op(B, TEAM_OP_KICK, A, 3, ok), Ok(Done::Kicked));
        assert!(t.team_of(A).is_empty());
        assert_eq!(t.team_of(B), &[B]);
    }

    #[test]
    fn a_woken_body_keeps_its_seat_and_a_gone_one_loses_it() {
        let mut t = Teams::default();
        team(&mut t, A, &[B, C]);
        t.rekey(A, D);
        assert_eq!(t.team_of(B), &[D, B, C]);
        assert_eq!(t.view(B, 2, |_| (0, 0, 0)).leader, D);
        t.prune(3, |id| id != C);
        assert_eq!(t.team_of(B), &[D, B]);
        t.prune(4, |id| id != D);
        assert_eq!(t.team_of(B), &[B]);
        assert_eq!(t.view(B, 4, |_| (0, 0, 0)).leader, B);
    }

    #[test]
    fn a_view_that_only_moved_is_told_apart() {
        let mut t = Teams::default();
        team(&mut t, A, &[B]);
        let a = t.view(A, 2, |_| (0, 1, 1));
        let b = t.view(A, 2, |_| (0, 9, 9));
        let c = t.view(A, 2, |_| (protocol::TEAM_DEAD, 1, 1));
        assert!(moved_only(&a, &b));
        assert!(!moved_only(&a, &c));
    }
}
