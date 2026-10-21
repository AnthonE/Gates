//! Contacts (Rust's): who goes in the book, when a meeting counts again, and
//! what a kill does to the mark.

use client::contacts::{
    crop_mugshot, mugshot_rect, Book, Filter, Rel, Sighting, Watch, ENCOUNTER_GAP_S, MAX_CONTACTS,
    MUGSHOT_PX, MUGSHOT_REFRESH_S, SEEN_S,
};
use protocol::Address;

fn addr(n: u16) -> Address {
    let mut a = [0u8; 20];
    a[18] = (n >> 8) as u8;
    a[19] = n as u8;
    a[0] = 1;
    Address(a)
}

/// A passer-by is not a contact: only a few seconds' company is, and stepping
/// away starts the count over.
#[test]
fn a_contact_is_someone_you_stood_near() {
    let (a, b) = (addr(1), addr(2));
    let mut w = Watch::default();
    let dt = SEEN_S / 3.0 + 0.01;
    assert!(w.step(dt, &[a, b]).is_empty());
    assert!(w.step(dt, &[a]).is_empty(), "b stepped away");
    assert_eq!(
        w.step(dt, &[a, b]),
        vec![(a, true)],
        "a crossed; b started over"
    );
    assert_eq!(
        w.step(dt, &[a, b]),
        vec![(a, false)],
        "still near, crossed once"
    );
}

/// A meeting counts again only after a gap; a guest is never booked.
#[test]
fn the_book_counts_meetings_not_frames() {
    let mut book = Book::default();
    let a = addr(1);
    assert_eq!(book.seen(&a, "ALICE", false, 1_000), Sighting::New);
    assert_eq!(book.seen(&a, "ALICE", false, 1_010), Sighting::Still);
    assert_eq!(book.get(&a).unwrap().met, 1);
    let later = 1_010 + ENCOUNTER_GAP_S;
    assert_eq!(book.seen(&a, "Alice B", false, later), Sighting::Again);
    let c = book.get(&a).unwrap();
    assert_eq!((c.met, c.last_seen, c.first_seen), (2, later, 1_000));
    assert_eq!(c.name, "Alice B", "the newest name wins");
    assert_eq!(
        book.seen(&Address::GUEST, "guest", false, 5),
        Sighting::Full
    );
    assert_eq!(book.count(Filter::All), 1);
}

/// Whoever kills you goes in as an enemy — unless you marked them a friend.
#[test]
fn a_killer_is_an_enemy_unless_a_friend() {
    let mut book = Book::default();
    let (a, b) = (addr(1), addr(2));
    book.killed_by(&a, "RAIDER", false, 50);
    let c = book.get(&a).unwrap();
    assert_eq!((c.rel, c.killed_me), (Rel::Enemy, 1));
    book.seen(&b, "PAL", false, 50);
    book.set_rel(&b, Rel::Friendly);
    book.killed_by(&b, "PAL", false, 60);
    assert_eq!(book.get(&b).unwrap().rel, Rel::Friendly);
    assert_eq!(book.shown(Filter::Enemy).len(), 1);
    assert!(book.forget(&a) && book.get(&a).is_none());
}

/// A full book drops the longest-unseen unmarked contact, never a marked one,
/// and survives a round trip through its file.
#[test]
fn a_full_book_forgets_strangers_first() {
    let mut book = Book::default();
    for i in 0..MAX_CONTACTS as u16 {
        book.seen(&addr(i), "x", false, 100 + i as u64);
    }
    book.set_rel(&addr(0), Rel::Enemy);
    assert_eq!(book.seen(&addr(999), "new", false, 9_999), Sighting::New);
    assert_eq!(book.count(Filter::All), MAX_CONTACTS);
    assert!(book.get(&addr(0)).is_some(), "the marked one stays");
    assert!(book.get(&addr(1)).is_none(), "the oldest stranger went");
    assert_eq!(Book::parse(book.to_json().as_bytes()), book);
    assert_eq!(Book::parse(b"not json"), Book::default());

    assert!(book.wants_mugshot(&addr(999), 10_000));
    book.shot(&addr(999), 10_000);
    assert!(!book.wants_mugshot(&addr(999), 10_001));
    assert!(book.wants_mugshot(&addr(999), 10_000 + MUGSHOT_REFRESH_S));
}

/// The mugshot is the square around the body on screen, kept inside the
/// frame, and nobody far away gets one.
#[test]
fn a_mugshot_is_the_square_around_them() {
    let (x, y, side) = mugshot_rect((640.0, 500.0), (640.0, 300.0), 1280, 720).unwrap();
    assert_eq!(side, 210);
    assert_eq!((x, y), (535, 275), "centred above the waist");
    assert!(
        mugshot_rect((640.0, 330.0), (640.0, 300.0), 1280, 720).is_none(),
        "too far"
    );
    let (_, y, _) = mugshot_rect((640.0, 760.0), (640.0, 560.0), 1280, 720).unwrap();
    assert_eq!(y, 720 - 210, "clamped inside the frame");

    // A 4x4 frame, blue in BGRA, cut whole: every pixel comes out blue RGBA.
    let frame: Vec<u8> = [255u8, 0, 0, 7].repeat(16);
    let px = crop_mugshot(&frame, 4, true, (0, 0, 4)).unwrap();
    assert_eq!(px.len(), (MUGSHOT_PX * MUGSHOT_PX * 4) as usize);
    assert_eq!(&px[..4], &[0, 0, 255, 255]);
    assert!(
        crop_mugshot(&frame, 4, true, (2, 2, 4)).is_none(),
        "off the frame"
    );
}
