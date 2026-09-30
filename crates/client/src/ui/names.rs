//! **What a player is called on screen.** The shard tells every client who
//! each player id is (`EventMsg::Tag`, `ClientCore::tag`): the proven wallet
//! and the name and picture that wallet set on its Elo Pros account page.
//! One rule turns that into a label, so chat, the kill feed, the death screen
//! and the nametag cannot disagree about somebody.

use client_core::core::Tag;

/// How far a nametag reaches (`DECISIONS.md`: "nametag range | 8 m,
/// aim-only"). Past it, or not aimed at, a player has no name.
pub const NAMETAG_REACH_M: f32 = 8.0;

/// Where names and pictures live when nothing better is known: the desktop
/// client derives its origin from `--servers` when given one.
pub const PLATFORM_ORIGIN: &str = "https://elopros.com";

/// The label for player `id`: their platform name, else their short
/// address, else `#id` (a guest, or a tag that has not landed yet).
pub fn label(tag: Option<&Tag>, id: u32) -> String {
    match tag {
        Some(t) if !t.name.is_empty() => t.name.as_str().to_string(),
        Some(t) if !t.address.is_guest() => super::spectate::short(&t.address),
        _ => format!("#{id}"),
    }
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
                || collide::shot_blocked(seed, haven, cols, prev.0, prev.1, x, z, y, 0.0)
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
        }
    }

    #[test]
    fn a_name_then_an_address_then_an_id() {
        assert_eq!(label(Some(&tag("Ash", 0)), 259), "Ash");
        assert_eq!(label(Some(&tag("", 0)), 259), "0x7e5f…5bdf");
        assert_eq!(label(None, 259), "#259");
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
