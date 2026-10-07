//! Speakers and inscriptions (`ARC.md` F5, F6): the people who talk, and the
//! ancients' writing on the island's stones.
//!
//! **The sim holds where they stand and what reading teaches, never the
//! words.** No `String` comes near the sim (wall 3). A word with a speaker
//! or a read at a stone is a reach-checked verb (`Command::Arc` with
//! [`OP_TALK`] or [`OP_READ`]) that announces itself (`EV_ARC_DID`). The
//! server answers it with the text from content, composed against the
//! world: a speaker's line can depend on which works burn, and an
//! inscription's hint carries this wipe's puzzle solution (`mech.rs`). So a
//! hint reaches only the player who walked to the stone.
//!
//! **Glyphs.** The ancients' script is a fixed alphabet (`content/arc.toml`
//! `[glyphs]`), so learning it is lasting mastery. Each player carries a mask
//! of the glyphs they can read ([`Player::glyphs`]). A bilingual stone
//! teaches the glyphs it names. A client draws a known glyph as its letter
//! and an unknown one as the glyph.
//!
//! [`Player::glyphs`]: crate::world::Player::glyphs

use crate::limits::{MAX_INSCRIPTIONS, MAX_SPEAKERS};
use crate::spot::Spot;
use crate::terrain::Haven;
use crate::works::{REFUSE_A_KIND, REFUSE_A_REACH};
use crate::world::{EventQueue, Player, EV_ARC_DID, EV_ARC_REFUSED};

/// `Command::Arc` op: say topic `arg` to speaker `target`.
pub const OP_TALK: u8 = 2;
/// `Command::Arc` op: read inscription `target`.
pub const OP_READ: u8 = 3;

/// How close to a speaker or a stone a word or a read must be, metres.
pub const LORE_REACH_M: f32 = 4.0;
/// Topics one speaker has.
pub const MAX_TOPICS: usize = 6;

/// One speaker: where they stand, and how many topics they answer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SpeakerDef {
    pub spot: Spot,
    pub topics: u8,
}

/// One inscription: where it stands, and the glyphs reading it teaches.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InscriptionDef {
    pub spot: Spot,
    pub teaches: u64,
}

/// Every speaker and inscription on the shard. `EMPTY` holds none.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoreContent {
    pub speakers: [SpeakerDef; MAX_SPEAKERS],
    pub n_speakers: u8,
    pub inscriptions: [InscriptionDef; MAX_INSCRIPTIONS],
    pub n_inscriptions: u8,
}

impl LoreContent {
    pub const EMPTY: LoreContent = LoreContent {
        speakers: [SpeakerDef {
            spot: Spot {
                site: 0,
                nth: 0,
                x_cm: 0,
                y_cm: 0,
                z_cm: 0,
            },
            topics: 0,
        }; MAX_SPEAKERS],
        n_speakers: 0,
        inscriptions: [InscriptionDef {
            spot: Spot {
                site: 0,
                nth: 0,
                x_cm: 0,
                y_cm: 0,
                z_cm: 0,
            },
            teaches: 0,
        }; MAX_INSCRIPTIONS],
        n_inscriptions: 0,
    };

    /// One speaker with two topics and one stone teaching glyphs 0 and 4,
    /// both at the town's middle, for the gates.
    pub fn probe_fixture() -> Self {
        let mut c = Self::EMPTY;
        let spot = Spot {
            site: crate::spot::SITE_TOWN,
            ..Spot::default()
        };
        c.speakers[0] = SpeakerDef { spot, topics: 2 };
        c.n_speakers = 1;
        c.inscriptions[0] = InscriptionDef {
            spot,
            teaches: 0b1_0001,
        };
        c.n_inscriptions = 1;
        c
    }
}

/// A word or a read from `p`. Refusals are events.
#[allow(clippy::too_many_arguments)]
pub fn act(
    lc: &LoreContent,
    haven: &Haven,
    p: &mut Player,
    op: u8,
    target: u8,
    arg: u8,
    events: &mut EventQueue,
) {
    let pid = p.id;
    let packed = (target as u32) << 8 | arg as u32;
    let refuse = |events: &mut EventQueue, code: u32| {
        events.push(EV_ARC_REFUSED, pid, code, (op as u32) << 8 | target as u32);
    };
    if p.dead || p.sleeping || p.wounded {
        return refuse(events, REFUSE_A_KIND);
    }
    let spot = match op {
        OP_TALK => match lc.speakers.get(target as usize) {
            Some(s) if (target as usize) < lc.n_speakers as usize && arg < s.topics => s.spot,
            _ => return refuse(events, REFUSE_A_KIND),
        },
        OP_READ => match lc.inscriptions.get(target as usize) {
            Some(i) if (target as usize) < lc.n_inscriptions as usize => i.spot,
            _ => return refuse(events, REFUSE_A_KIND),
        },
        _ => return refuse(events, REFUSE_A_KIND),
    };
    let px = p.body.qx as f32 * crate::movement::POS_XZ_Q;
    let pz = p.body.qz as f32 * crate::movement::POS_XZ_Q;
    let feet = p.body.qy as f32 * crate::movement::POS_Y_Q;
    if !crate::spot::within(haven, &spot, px, feet, pz, LORE_REACH_M) {
        return refuse(events, REFUSE_A_REACH);
    }
    if op == OP_READ {
        p.glyphs |= lc.inscriptions[target as usize].teaches;
    }
    events.push(EV_ARC_DID, pid, op as u32, packed);
}
