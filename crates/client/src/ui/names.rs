//! **What a player is called on screen.** The shard tells every client who
//! each player id is (`EventMsg::Tag`, `ClientCore::tag`): the proven wallet
//! and the name and picture that wallet set on its Elo Pros account page.
//! One rule turns that into a label, so chat, the kill feed, the death screen
//! and the nametag cannot disagree about somebody — including about whether
//! it is a program: a player that declared itself an agent (`EventMsg::Agent`,
//! v107) carries ` (agent)` everywhere its name appears. A player who wears a
//! guild tag on Elo Pros (v152) is drawn `[TAG] name` everywhere it is drawn.

use client_core::core::{ClientCore, Tag};

/// How far a nametag reaches (`DECISIONS.md`: "nametag range | 8 m,
/// aim-only"). Past it, or not aimed at, a player has no name.
pub const NAMETAG_REACH_M: f32 = 8.0;

/// Where names and pictures live when nothing better is known: the desktop
/// client derives its origin from `--servers` when given one.
pub const PLATFORM_ORIGIN: &str = "https://elopros.com";

/// The label for player `id`: their platform name, else their short
/// address, else `#id` (a guest, or a tag that has not landed yet), with
/// ` (agent)` after it for a player that declared itself one — the
/// spectate line's `(guest)` style.
pub fn label(tag: Option<&Tag>, agent: bool, id: u32) -> String {
    let name = match tag {
        Some(t) if !t.name.is_empty() => t.name.as_str().to_string(),
        Some(t) if !t.address.is_guest() => super::spectate::short(&t.address),
        _ => format!("#{id}"),
    };
    if agent {
        format!("{name} (agent)")
    } else {
        name
    }
}

/// [`label`] for player `id`, as `core` knows them: the real name, for what
/// is kept (the contacts book) and matched (`/mute`). What is DRAWN goes
/// through [`shown`].
pub fn of(core: &ClientCore, id: u32) -> String {
    label(core.tag(id), core.is_agent(id), id)
}

/// The name drawn for player `id`: [`of`] behind their guild tag, or a
/// made-up one in streamer mode (`ui::streamer`). Every surface that puts a
/// player's name on screen calls this.
pub fn shown(core: &ClientCore, id: u32) -> String {
    shown_if(super::streamer::on(), core.tag(id), core.is_agent(id), id)
}

/// [`shown`] with the switch given, for a test.
pub fn shown_if(streamer: bool, tag: Option<&Tag>, agent: bool, id: u32) -> String {
    if !streamer {
        let name = label(tag, agent, id);
        // The tag is drawn, never kept or matched: `of` stays the bare name.
        return match tag.map(|t| t.guild).filter(|g| !g.is_none()) {
            Some(g) => format!("[{}] {name}", g.tag()),
            None => name,
        };
    }
    // A guild tag would say who a streamer's players are: none is drawn.
    let key = match tag {
        Some(t) if !t.address.is_guest() => {
            let hex = t.address.to_hex();
            core::str::from_utf8(&hex).unwrap_or("").to_string()
        }
        _ => format!("#{id}"),
    };
    let name = super::streamer::alias(&key);
    if agent {
        format!("{name} (agent)")
    } else {
        name
    }
}

/// A name that arrived as text (a standings row, a friend, a spectate
/// feed), keyed by its wallet when there is one: itself, or its alias in
/// streamer mode.
pub fn shown_text(name: &str, wallet: Option<&str>) -> String {
    if !super::streamer::on() {
        return name.to_string();
    }
    super::streamer::alias(wallet.filter(|w| !w.is_empty()).unwrap_or(name))
}

/// The picture's url, if this player set one. `?v=` is the picture's
/// revision, so a changed picture is a different url to every cache.
pub fn pic_url(origin: &str, tag: &Tag) -> Option<String> {
    if tag.pic == 0 || tag.address.is_guest() {
        return None;
    }
    let hex = tag.address.to_hex();
    let wallet = core::str::from_utf8(&hex).ok()?;
    Some(format!(
        "{}/api/face/{wallet}/pic.png?v={:08x}",
        origin.trim_end_matches('/'),
        tag.pic
    ))
}

/// The platform origin (`scheme://host[:port]`) behind a url such as the
/// `--servers` list, or [`PLATFORM_ORIGIN`] when there is none.
pub fn origin_of(url: Option<&str>) -> String {
    url.and_then(|u| {
        let (scheme, rest) = u.split_once("://")?;
        let host = rest.split('/').next().filter(|h| !h.is_empty())?;
        Some(format!("{scheme}://{host}"))
    })
    .unwrap_or_else(|| PLATFORM_ORIGIN.to_string())
}

/// A `/api/face/{wallet}` body → (name, picture revision): the same reading
/// the shard makes (`server/faces.rs`). A name the wire's `Name` refuses is
/// no name; `None` for a body that is not the platform's answer.
pub fn parse_face(body: &[u8]) -> Option<(String, u32)> {
    let v: serde_json::Value = serde_json::from_slice(body).ok()?;
    let obj = v.as_object()?;
    let name = obj
        .get("name")
        .and_then(|n| n.as_str())
        .map(str::trim)
        .filter(|n| protocol::Name::new(n).is_some())
        .unwrap_or("")
        .to_string();
    let pic = obj
        .get("pic")
        .and_then(|p| p.as_str())
        .filter(|p| p.len() == 8)
        .and_then(|p| u32::from_str_radix(p, 16).ok())
        .map_or(0, |p| p.max(1));
    Some((name, pic))
}

/// Whether nothing solid stands between `from` and `to` — a wall, a door,
/// a rock, the ground. A nametag is a thing you can see, so a player behind
/// a wall has none, even aimed at. The walk is the explorer's
/// (`server/explorer.rs::clear_line`) with the built pieces added: one
/// sample per `ARROW_STEP_MM`, the step every shot in the sim takes.
pub fn clear_line(
    core: &mut client_core::core::ClientCore,
    from: (f32, f32, f32),
    to: (f32, f32, f32),
) -> bool {
    use sim_core::{collide, terrain};
    let d = (to.0 - from.0, to.1 - from.1, to.2 - from.2);
    let len = (d.0 * d.0 + d.1 * d.1 + d.2 * d.2).sqrt();
    let steps = ((len * 1000.0 / sim_core::limits::ARROW_STEP_MM as f32).ceil() as usize).max(1);
    let at = |i: usize| {
        let t = i as f32 / steps as f32;
        (from.0 + d.0 * t, from.1 + d.1 * t, from.2 + d.2 * t)
    };
    let seed = core.island().0;
    {
        let haven = core.haven();
        let cols = core.pieces.cols();
        let mut prev = (from.0, from.2);
        for i in 1..=steps {
            let (x, y, z) = at(i);
            if y <= terrain::ground(seed, haven, x, z)
                // An arrowhead's radius, not zero: the shot walk tests a
                // floor or a roof only for a positive one, and a zero-width
                // look saw a name through the slab above it.
                || collide::shot_blocked(seed, haven, cols, prev.0, prev.1, x, z, y, 0.05)
                || collide::deploy_stop(seed, haven, cols, x, z, y, 0.0).is_some()
            {
                return false;
            }
            prev = (x, z);
        }
    }
    let (seed, mut island) = core.island();
    (1..=steps).all(|i| {
        let (x, y, z) = at(i);
        !island.blocks_volume(seed, x, z, y, 0.0, 0.0)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag(name: &str, pic: u32) -> Tag {
        Tag {
            id: 259,
            address: protocol::Address::from_hex(b"0x7e5f4552091a69125d5dfcb7b8c2659029395bdf")
                .unwrap(),
            name: protocol::Name::new(name).unwrap(),
            pic,
            guild: protocol::Guild::NONE,
        }
    }

    #[test]
    fn a_guild_tag_is_drawn_before_the_name_but_never_kept_or_streamed() {
        let mut t = tag("Ash", 0);
        t.guild = protocol::Guild::new("OWL", Some("teal")).unwrap();
        assert_eq!(shown_if(false, Some(&t), false, 259), "[OWL] Ash");
        assert_eq!(shown_if(false, Some(&t), true, 259), "[OWL] Ash (agent)");
        assert_eq!(label(Some(&t), false, 259), "Ash");
        assert!(!shown_if(true, Some(&t), false, 259).contains("OWL"));
        assert_eq!(shown_if(false, Some(&tag("Ash", 0)), false, 259), "Ash");
    }

    #[test]
    fn a_name_then_an_address_then_an_id() {
        assert_eq!(label(Some(&tag("Ash", 0)), false, 259), "Ash");
        assert_eq!(label(Some(&tag("", 0)), false, 259), "0x7e5f…5bdf");
        assert_eq!(label(None, false, 259), "#259");
    }

    #[test]
    fn streamer_mode_draws_a_made_up_name_and_keeps_the_agent_mark() {
        // A name no alias is made of (`streamer::FIRST`/`SECOND` hold "Ash").
        let t = tag("Quillon", 0);
        assert_eq!(shown_if(false, Some(&t), false, 259), "Quillon");
        let a = shown_if(true, Some(&t), false, 259);
        assert!(!a.contains("Quillon") && !a.contains("0x"), "{a}");
        assert_eq!(
            a,
            shown_if(true, Some(&tag("", 0)), false, 259),
            "keyed by wallet, not by name"
        );
        assert!(shown_if(true, Some(&t), true, 259).ends_with(" (agent)"));
        assert_ne!(
            shown_if(true, None, false, 259),
            shown_if(true, None, false, 260)
        );
    }

    #[test]
    fn an_agent_says_so_wherever_it_is_named() {
        assert_eq!(label(Some(&tag("jev", 0)), true, 259), "jev (agent)");
        assert_eq!(label(None, true, 259), "#259 (agent)");
    }

    #[test]
    fn a_picture_url_carries_its_revision_and_none_without_one() {
        assert_eq!(
            pic_url("https://elopros.com/", &tag("Ash", 0x1a2b)).as_deref(),
            Some("https://elopros.com/api/face/0x7e5f4552091a69125d5dfcb7b8c2659029395bdf/pic.png?v=00001a2b")
        );
        assert_eq!(pic_url("https://elopros.com", &tag("Ash", 0)), None);
        assert_eq!(
            origin_of(Some("https://elopros.com/api/launcher/servers/gates")),
            "https://elopros.com"
        );
        assert_eq!(origin_of(None), PLATFORM_ORIGIN);
    }

    #[test]
    fn the_platform_answer_reads_as_the_shard_reads_it() {
        let body = br#"{"wallet":"0xab","name":" Ash ","pic":"1a2b3c4d","picture":"/x"}"#;
        assert_eq!(parse_face(body), Some(("Ash".into(), 0x1a2b_3c4d)));
        assert_eq!(
            parse_face(br#"{"name":null,"pic":null}"#),
            Some((String::new(), 0))
        );
        assert_eq!(
            parse_face(r#"{"name":"Zoë"}"#.as_bytes()),
            Some((String::new(), 0))
        );
        assert_eq!(parse_face(b"<html>"), None);
    }
}
