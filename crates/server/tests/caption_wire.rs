//! Captions on the wire (v138): `/lang` sets a player's language quietly or
//! with an answer, the scribe is handed what was said with the languages of
//! who heard it, and a caption reaches whoever hears its talker, a
//! translation only those who read its language. The scribe's thread and
//! services are `scribe.rs`'s; here its rings are driven by hand.

use client_core::core::ClientCore;
use protocol::{Caption, Channel, ChatMsg, ChatText, EventMsg, ItemCatalog};
use server::core::{Lane, ShardCore};
use server::scribe::{Said, ToScribe};
use server::stats::ShardStats;
use sim_core::gather::GatherContent;

const SEED: u64 = 20_261_021;
const SPAWN: (f32, f32) = (2048.0, 2048.0);
const Q_CM: i32 = 3;

fn id_of(slot: usize) -> u32 {
    (1 << 8) | slot as u32
}

fn line(s: &str) -> ChatText {
    ChatText::sanitize(s.as_bytes()).expect("a clean line")
}

/// One tick; every event the server put on the lane, decoded, by slot.
fn pump(
    core: &mut ShardCore,
    stats: &ShardStats,
    clients: &mut [ClientCore],
) -> Vec<(usize, EventMsg)> {
    let mut events: Vec<(usize, Vec<u8>)> = Vec::new();
    core.tick_bare(stats, |lane, slot, bytes| {
        if lane == Lane::Event {
            events.push((slot, bytes.to_vec()));
        }
        true
    });
    let mut seen = Vec::new();
    for (slot, bytes) in events {
        seen.push((slot, protocol::decode_event(&bytes).expect("events decode")));
        clients[slot].on_stream(&bytes).expect("events decode");
    }
    seen
}

fn captions_to(seen: &[(usize, EventMsg)], slot: usize) -> Vec<Caption> {
    seen.iter()
        .filter(|(s, _)| *s == slot)
        .filter_map(|(_, m)| match m {
            EventMsg::Caption(c) => Some(*c),
            _ => None,
        })
        .collect()
}

fn server_lines_to(seen: &[(usize, EventMsg)], slot: usize) -> Vec<String> {
    seen.iter()
        .filter(|(s, _)| *s == slot)
        .filter_map(|(_, m)| match m {
            EventMsg::Chat { from: 0, text, .. } => {
                Some(String::from_utf8(text.as_bytes().to_vec()).unwrap())
            }
            _ => None,
        })
        .collect()
}

fn say(core: &mut ShardCore, slot: usize, channel: Channel, text: &str) {
    core.push_chat(
        slot,
        ChatMsg {
            channel,
            text: line(text),
        },
    );
}

#[test]
fn captions_reach_who_hears_and_translations_who_reads() {
    let stats = ShardStats::default();
    let mut core = Box::new(ShardCore::new(SEED));
    core.world.gather = GatherContent::probe_fixture();
    core.world.dev_spawn = Some(SPAWN);
    core.catalog = ItemCatalog::EMPTY;
    for slot in 0..3 {
        assert!(core.connect(slot, id_of(slot)));
    }
    let mut clients: Vec<ClientCore> = (0..3)
        .map(|slot| ClientCore::new(SEED, id_of(slot), 0))
        .collect();
    let (scribe, mut to_scribe, mut to_sim) = server::scribe::pair();
    core.scribe = scribe;
    pump(&mut core, &stats, &mut clients);
    pump(&mut core, &stats, &mut clients);
    // Slot 2 walks out of earshot, past the voice radius.
    let w2 = core
        .world
        .players
        .iter()
        .position(|p| p.active && p.id == id_of(2))
        .unwrap();
    core.world.players[w2].body.qx += 20_000 / Q_CM;

    let es = protocol::lang_of("es").unwrap();
    let en = protocol::lang_of("en").unwrap();
    // The client's own `/lang` at join is quiet; a player's is answered.
    say(&mut core, 1, Channel::Local, "/lang en-US auto");
    let seen = pump(&mut core, &stats, &mut clients);
    assert!(server_lines_to(&seen, 1).is_empty(), "{seen:?}");
    say(&mut core, 0, Channel::Local, "/lang es");
    let seen = pump(&mut core, &stats, &mut clients);
    let answer = server_lines_to(&seen, 0);
    assert_eq!(answer.len(), 1, "{seen:?}");
    assert!(answer[0].contains("Spanish"), "{answer:?}");

    // A typed line goes to the scribe with its readers' languages; the
    // distant slot 2 was not reached and reads none.
    say(&mut core, 0, Channel::Local, "hola a todos");
    pump(&mut core, &stats, &mut clients);
    match to_scribe.pop() {
        Ok(ToScribe::Chat {
            slot,
            id,
            channel,
            text,
            langs,
            ..
        }) => {
            assert_eq!((slot, id, channel), (0, id_of(0), Channel::Local));
            assert_eq!(text, line("hola a todos"));
            assert_eq!(langs, (1 << es) | (1 << en));
        }
        _ => panic!("the line should have reached the scribe"),
    }

    // Voice relayed from slot 0 goes to the scribe with its listeners'
    // languages; voice nobody heard does not.
    let mut hears = [false; server::slot::MAX_CONNS];
    core.voice_reach(0, &mut hears).expect("slot 0 can talk");
    let frame = protocol::VoiceFrame::new(1, 0, &[1, 2, 3]).unwrap();
    core.scribe_hear(0, &[frame], &hears, &stats);
    match to_scribe.pop() {
        Ok(ToScribe::Voice { slot, langs, .. }) => {
            assert_eq!(slot, 0);
            assert_eq!(langs, 1 << en, "slot 1 heard it; slot 2 is out of range");
        }
        _ => panic!("the frame should have reached the scribe"),
    }
    core.scribe_hear(0, &[frame], &[false; server::slot::MAX_CONNS], &stats);
    assert!(to_scribe.pop().is_err());

    // Words said out loud: to everyone in voice range, the talker included.
    let said = Caption {
        from: id_of(0),
        line: 1,
        voice: true,
        more: false,
        channel: Channel::Local,
        lang: 0,
        text: line("¡sígueme!"),
    };
    to_sim
        .push(Said {
            slot: 0,
            to_lang: 0,
            caption: said,
        })
        .unwrap();
    // Their translation: only to the English reader.
    let translated = Caption {
        lang: es,
        text: line("follow me!"),
        ..said
    };
    to_sim
        .push(Said {
            slot: 0,
            to_lang: en,
            caption: translated,
        })
        .unwrap();
    // A typed global line, translated: every English reader, wherever.
    let typed = Caption {
        line: protocol::caption_key(&line("hola a todos")),
        voice: false,
        channel: Channel::Global,
        lang: es,
        text: line("hello everyone"),
        ..said
    };
    to_sim
        .push(Said {
            slot: 0,
            to_lang: en,
            caption: typed,
        })
        .unwrap();
    // A caption from somebody no longer in the slot goes nowhere.
    to_sim
        .push(Said {
            slot: 0,
            to_lang: 0,
            caption: Caption {
                from: id_of(9),
                ..said
            },
        })
        .unwrap();
    let seen = pump(&mut core, &stats, &mut clients);
    assert_eq!(captions_to(&seen, 0), vec![said]);
    assert_eq!(captions_to(&seen, 1), vec![said, translated, typed]);
    assert!(captions_to(&seen, 2).is_empty());
    // And the client holds them for its chat log.
    assert_eq!(clients[1].pop_caption(), Some(said));
    assert_eq!(clients[1].pop_caption(), Some(translated));
    assert_eq!(clients[1].pop_caption(), Some(typed));
    assert_eq!(clients[1].pop_caption(), None);
}
