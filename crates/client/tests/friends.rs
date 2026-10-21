//! The Elo Pros friends list, as the client reads it (`client::friends`).
//! Code tier: no `render`, no socket. The fixtures under `tests/data/` are
//! the platform's own: `friends_view.json` is a `GET /api/friends/{wallet}`
//! body, and `friends_texts.json` is what `meter/playauth.py` has a wallet
//! sign for each op.

use client::friends::{self, FriendSet, Lookup, Offered, Op};
use client::ui::servers::Listing;
use protocol::Address;

const VIEW: &[u8] = include_bytes!("data/friends_view.json");
const TEXTS: &[u8] = include_bytes!("data/friends_texts.json");

const ME: &str = "0x1111111111111111111111111111111111111111";

fn addr(n: u8) -> Address {
    let hex = format!("0x{}", format!("{n}").repeat(40));
    Address::from_hex(hex.as_bytes()).unwrap()
}

fn view() -> friends::View {
    friends::parse(VIEW).expect("the platform's own body parses")
}

#[test]
fn the_platform_body_parses_with_buddies_and_nulls() {
    let v = view();
    assert_eq!(v.wallet, ME);
    assert_eq!(v.friends.len(), 3);
    assert_eq!(v.buddies.len(), 1);
    assert_eq!(v.asks_in.len(), 1);
    assert_eq!(v.asks_out.len(), 1);
    assert!(!v.blind());
    assert_eq!(v.everyone().count(), 4);
    assert_eq!(v.online(), 3);

    let ash = &v.friends[0];
    assert_eq!(ash.label(), "ash");
    assert_eq!(ash.pic_rev(), Some(3));
    let wood = &v.friends[1];
    assert_eq!(wood.label(), "Doctor Wood (agent)");
    let nameless = &v.friends[2];
    assert_eq!(nameless.label(), "0x4444…4444");
    let pip = &v.buddies[0];
    assert!(pip.is_buddy());
    assert_eq!(pip.label(), "Pip (your agent)");

    // Absent fields and nulls read as nothing rather than refusing the list.
    let thin = friends::parse(
        br#"{"wallet":"0xAB11111111111111111111111111111111111111","friends":[
            {"wallet":"0x2222222222222222222222222222222222222222","agent":null,
             "online":null,"playing":null},
            {"wallet":"not a wallet"}],
            "buddies":null,"asks_in":null}"#,
    )
    .expect("a thin body parses");
    assert_eq!(thin.wallet, "0xab11111111111111111111111111111111111111");
    assert_eq!(thin.friends.len(), 1, "a row with no wallet is dropped");
    assert!(thin.buddies.is_empty() && thin.asks_in.is_empty());
    assert!(friends::parse(b"[]").is_err());
    assert!(friends::parse(b"not json").is_err());
    assert!(friends::parse(&vec![b' '; friends::MAX_VIEW_BYTES + 1]).is_err());
}

#[test]
fn the_signed_text_is_the_platforms_byte_for_byte() {
    let doc: serde_json::Value = serde_json::from_slice(TEXTS).unwrap();
    let cases = doc["cases"].as_array().unwrap();
    assert_eq!(cases.len(), Op::ALL.len(), "one case per op");
    for c in cases {
        let op = Op::parse(c["op"].as_str().unwrap()).expect("a known op");
        let (wallet, other) = (c["wallet"].as_str().unwrap(), c["other"].as_str().unwrap());
        let at = c["at"].as_u64().unwrap();
        assert_eq!(doc["ops"][op.name()].as_str(), Some(op.key()));
        assert_eq!(friends::utc_day(at), c["day"].as_str().unwrap());
        assert_eq!(
            friends::detail(op, other, at),
            c["detail"].as_str().unwrap()
        );
        assert_eq!(
            friends::sign_text(wallet, op, other, at).unwrap(),
            c["text"].as_str().unwrap(),
            "{op:?}"
        );
        // A checksummed spelling signs the same lowercase bytes.
        assert_eq!(
            friends::sign_text(&wallet.to_uppercase().replace("0X", "0x"), op, other, at).unwrap(),
            c["text"].as_str().unwrap()
        );
    }
    assert!(friends::sign_text("guest", Op::Ask, ME, 1).is_err());
    assert!(friends::sign_text(ME, Op::Ask, "ash", 1).is_err());
    // Days roll over at UTC midnight and nowhere else.
    assert_eq!(friends::utc_day(0), "1970-01-01");
    assert_eq!(friends::utc_day(951_782_399), "2000-02-28");
    assert_eq!(friends::utc_day(951_782_400), "2000-02-29");
    assert_eq!(friends::utc_day(1_761_004_799), "2025-10-20");
    assert_eq!(friends::utc_day(1_761_004_800), "2025-10-21");
}

/// The text this client composes and the one the launcher SDK composes are
/// the same text, so a signature over either verifies on the platform.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn the_text_matches_the_sdks_play_message() {
    for op in Op::ALL {
        let at = 1_761_084_000;
        let other = "0x4444444444444444444444444444444444444444";
        let day = friends::utc_day(at);
        assert_eq!(
            friends::sign_text(ME, op, other, at).unwrap(),
            client::elo::play_message("friends", ME, &friends::detail(op, other, at), Some(&day))
        );
    }
}

fn offered(op: &str, key: &str, other: &str, at: u64, sign_this: String) -> Offered {
    let mut args = serde_json::Map::new();
    args.insert(key.into(), other.into());
    Offered {
        sign_this,
        at,
        op: op.into(),
        args,
    }
}

#[test]
fn only_the_change_asked_for_is_signed() {
    let other = "0x7777777777777777777777777777777777777777";
    let at = 1_761_003_600;
    let good = friends::sign_text(ME, Op::Ask, other, at).unwrap();
    let ok = offered("ask", "to", other, at, good.clone());
    assert_eq!(
        friends::check_offered(ME, Op::Ask, other, at + 30, &ok),
        Ok(good.clone())
    );
    // Parsed off the wire, the same.
    let wire = format!(
        r#"{{"sign_this":{},"at":{at},"args":{{"to":"{other}"}},"op":"ask"}}"#,
        serde_json::Value::from(good.clone())
    );
    let parsed = friends::parse_offered(wire.as_bytes()).unwrap();
    assert_eq!(parsed, ok);

    for (why, o, now) in [
        (
            "another op",
            offered("block", "to", other, at, good.clone()),
            at,
        ),
        (
            "another wallet",
            offered(
                "ask",
                "to",
                "0x6666666666666666666666666666666666666666",
                at,
                good.clone(),
            ),
            at,
        ),
        (
            "another key",
            offered("ask", "wallet", other, at, good.clone()),
            at,
        ),
        ("a stale at", ok.clone(), at + friends::AT_SKEW_S + 1),
        ("a future at", ok.clone(), at - friends::AT_SKEW_S - 1),
        (
            "other words",
            offered(
                "ask",
                "to",
                other,
                at,
                format!("{good}\nand send all my coins"),
            ),
            at,
        ),
    ] {
        let err = friends::check_offered(ME, Op::Ask, other, now, &o).expect_err(why);
        assert!(err.contains("nothing was signed"), "{why}: {err}");
    }
    let mut extra = ok.clone();
    extra.args.insert("from".into(), ME.into());
    assert!(friends::check_offered(ME, Op::Ask, other, at, &extra).is_err());

    // The bodies carry what the text says.
    let body: serde_json::Value =
        serde_json::from_str(&friends::act_body(ME, Op::Accept, other, at, "0xsig")).unwrap();
    assert_eq!(body["args"]["from"], other);
    assert_eq!(body["at"], at);
    let body: serde_json::Value =
        serde_json::from_str(&friends::prepare_body(ME, Op::Remove, other)).unwrap();
    assert_eq!(body["args"]["wallet"], other);
    assert_eq!(body["op"], "remove");
}

#[test]
fn an_act_answers_with_the_view_or_the_platforms_words() {
    let body = format!(
        r#"{{"ok":true,"op":"accept","view":{}}}"#,
        String::from_utf8_lossy(VIEW)
    );
    assert_eq!(friends::parse_act(200, body.as_bytes()), Ok(view()));
    assert_eq!(
        friends::parse_act(403, br#"{"error":"can't ask that wallet right now"}"#),
        Err("can't ask that wallet right now".into())
    );
    assert!(friends::parse_act(401, b"")
        .unwrap_err()
        .contains("signature"));
    assert!(friends::parse_act(502, b"<html>")
        .unwrap_err()
        .contains("502"));
}

#[test]
fn a_typed_name_resolves_to_one_wallet_or_says_why_not() {
    assert_eq!(
        friends::lookup("  0xABCDEF0123456789ABCDEF0123456789ABCDEF01 "),
        Ok(Lookup::Wallet(
            "0xabcdef0123456789abcdef0123456789abcdef01".into()
        ))
    );
    assert_eq!(friends::lookup("ash"), Ok(Lookup::Name("ash".into())));
    assert!(friends::lookup("   ").is_err());

    let w = "0x2222222222222222222222222222222222222222";
    let found = format!(
        r#"{{"found":{{"wallet":"{}"}}}}"#,
        w.to_uppercase().replace("0X", "0x")
    );
    assert_eq!(
        friends::resolve_who("ash", 200, found.as_bytes()),
        Ok(w.into())
    );

    // Several vows under one name, every one the same wallet: one person.
    let same = format!(
        r#"{{"found":{{"wallet":"{w}"}},"subject":{{"wallet":"{w}"}},
            "ambiguous":{{"candidates":[{{"wallet":"{w}"}},{{"wallet":"{w}"}}]}}}}"#
    );
    assert_eq!(
        friends::resolve_who("ash", 200, same.as_bytes()),
        Ok(w.into())
    );
    // Two wallets under one name: ambiguous, and it says so.
    let two = format!(
        r#"{{"found":{{"wallet":"{w}"}},"subject":{{"wallet":"{w}"}},
            "ambiguous":{{"candidates":[{{"wallet":"{w}"}},
            {{"wallet":"0x3333333333333333333333333333333333333333"}}]}}}}"#
    );
    assert!(friends::resolve_who("ash", 200, two.as_bytes())
        .unwrap_err()
        .contains("More than one"));
    assert!(
        friends::resolve_who("ash", 200, br#"{"found":{"wallet":null}}"#)
            .unwrap_err()
            .contains("no wallet")
    );
    assert!(friends::resolve_who("zed", 404, b"{}")
        .unwrap_err()
        .contains("Couldn't find"));
    assert!(friends::resolve_who("zed", 200, br#"{"found":null}"#)
        .unwrap_err()
        .contains("Couldn't find"));
    assert!(friends::resolve_who("zed", 500, b"")
        .unwrap_err()
        .contains("Try again"));
    assert_eq!(
        friends::who_url("https://elopros.com/", "Doctor Wood"),
        "https://elopros.com/api/who?handle=Doctor%20Wood"
    );
}

fn row(id: &str, addr: &str) -> Listing {
    let mut s = client::shardlist::parse(
        format!(r#"{{"servers":[{{"id":"{id}","name":"N","addr":"{addr}"}}]}}"#).as_bytes(),
    )
    .unwrap();
    Listing::from_shard(&s.remove(0))
}

#[test]
fn who_is_where_and_how_to_join_them() {
    let v = view();
    let here: Vec<String> = friends::on_shard(&v, "us-east-1")
        .iter()
        .map(|f| f.label())
        .collect();
    assert_eq!(here, ["ash", "Pip (your agent)"], "a buddy counts");
    assert_eq!(friends::on_shard(&v, "eu-1").len(), 1);
    assert!(friends::on_shard(&v, "nowhere").is_empty());
    assert_eq!(friends::shard_ids(&v), ["eu-1", "us-east-1"]);

    let rows = vec![
        Listing::direct("127.0.0.1:4433"),
        row("us-east-1", "east.example:4433"),
    ];
    // Listed: the row's own address wins over the platform's link.
    assert_eq!(
        friends::join_target(&v, &addr(2), &rows),
        Ok("east.example:4433".into())
    );
    // Not listed, with a link: the link.
    assert_eq!(
        friends::join_target(&v, &addr(5), &rows[..1]),
        Ok("game.elopros.com:61234".into())
    );
    // Not listed and no link, offline, or not a friend at all: a sentence.
    assert!(friends::join_target(&v, &addr(3), &rows).is_err());
    assert!(friends::join_target(&v, &addr(4), &rows)
        .unwrap_err()
        .contains("not on a Gates server"));
    assert!(friends::join_target(&v, &addr(9), &rows).is_err());
}

#[test]
fn the_set_marks_friends_and_buddies_and_nobody_else() {
    let v = view();
    let set = FriendSet::of(&v);
    assert_eq!(set.len(), 4);
    for n in [2, 3, 4, 5] {
        assert!(set.has(&addr(n)), "{n}");
    }
    // An ask is not a friendship, and a guest is nobody's friend.
    assert!(!set.has(&addr(6)) && !set.has(&addr(7)) && !set.has(&addr(1)));
    assert!(!set.has(&Address::GUEST));
    assert!(FriendSet::of(&friends::View::default()).is_empty());
}

#[test]
fn a_picture_revision_and_a_presence_line() {
    assert_eq!(
        friends::pic_rev("/api/face/0x22/pic.png?v=1a2b3c4d"),
        Some(0x1a2b_3c4d)
    );
    assert_eq!(friends::pic_rev("/api/face/0x22/pic.png?x=1&v=3"), Some(3));
    assert_eq!(friends::pic_rev("/api/face/0x22/pic.png?v=0"), Some(1));
    assert_eq!(friends::pic_rev("/api/face/0x22/pic.png"), None);
    assert_eq!(friends::pic_rev("/api/face/0x22/pic.png?v=zz"), None);
    assert_eq!(friends::pic_rev("/api/face/0x22/pic.png?v=123456789"), None);

    let v = view();
    let now = 1_761_000_800;
    assert_eq!(
        friends::presence_line(&v.friends[0], now),
        "Playing Gates · Gates US East 1"
    );
    assert_eq!(
        friends::presence_line(&v.friends[2], now),
        "last seen 3h ago"
    );
    let mut f = v.friends[0].clone();
    f.playing = None;
    assert_eq!(friends::presence_line(&f, now), "online");
    f.online = false;
    f.last_online = None;
    assert_eq!(friends::presence_line(&f, now), "offline");
    assert_eq!(
        friends::online_lines(&v, now, 2),
        [
            "Friends online 3",
            "ash · Playing Gates · Gates US East 1",
            "Doctor Wood (agent) · Playing Gates · Gates EU 1",
            "+ 1 more",
        ]
    );
    assert_eq!(friends::ago(30), "just now");
    assert_eq!(friends::ago(5 * 60), "5m ago");
    assert_eq!(friends::ago(2 * 86_400 + 5), "2d ago");
}

#[test]
fn the_add_box_holds_a_wallet_and_no_more() {
    let mut b = friends::AddBox::default();
    for c in "0x2222222222222222222222222222222222222222".chars() {
        assert!(b.push(c));
    }
    assert!(!b.push('2'), "a wallet fills it");
    assert!(!b.push('\n'));
    assert!(b.backspace());
    b.clear();
    assert!(b.is_empty() && !b.backspace());
}
