//! The shipped content's layout, pinned (`Content::layout_hash`).
//!
//! The saves file items, recipes, pieces, deployables, works and glyphs by
//! index, so a row added, removed, renamed or reordered makes every save on
//! the live shard unreadable: **a layout change is a wipe**, the same as a
//! worldgen change (`sim-core/tests/terrain_golden.rs`). Balance edits leave
//! the layout alone (`content.rs`, `layout_holds_through_a_balance_edit...`).
//!
//! Pinned so the change shows in the diff: `ci/deploy_kind.py` reads this
//! constant to tell a PR that needs a wipe from one that can ship mid-wipe.
//! If this fails on purpose, paste the `left` value below. The PR is then a
//! wipe: it goes to `next-wipe`, not `main`.

use content::Content;
use std::path::Path;

const GOLDEN_LAYOUT_HASH: u64 = 700728033408663343;

#[test]
fn content_layout_golden() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content");
    let c = Content::load_dir(&dir).expect("shipped content must load");
    assert_eq!(
        c.layout_hash(),
        GOLDEN_LAYOUT_HASH,
        "the content layout moved (a row added, removed, renamed or reordered). \
         That is a wipe: paste the new hash into GOLDEN_LAYOUT_HASH and send the \
         PR to next-wipe"
    );
}
