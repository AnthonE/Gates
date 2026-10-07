//! The ancients' glyphs, as shapes (`ARC.md` F6): each glyph is a pattern on
//! a 3 × 5 grid, fixed forever, so a player who learns them can read them
//! off a screenshot next wipe. A glyph this player reads is drawn as its
//! letter; one they do not is drawn as its pattern.
//!
//! Pure, so `tests/ui.rs` and the tests below can hold the patterns apart.

/// Columns and rows of a glyph's grid.
pub const GLYPH_COLS: u32 = 3;
pub const GLYPH_ROWS: u32 = 5;

/// Glyph `g`'s pattern: bit `r * 3 + c` lights cell (`c`, `r`). Every glyph
/// has its top-middle cell (the stroke they all hang from), at least five
/// cells, and no two glyphs share a pattern (`patterns_are_distinct`).
pub fn pattern(g: usize) -> u16 {
    // Walked in order, each glyph taking the first candidate no earlier one
    // wears: 64 glyphs at most, so the walk is a few thousand steps.
    let mut taken = [0u16; 64];
    let g = g.min(63);
    for h in 0..=g {
        let mut seed = (h as u32).wrapping_add(0x9E37).wrapping_mul(0x85EB_CA6B);
        taken[h] = loop {
            seed ^= seed >> 13;
            seed = seed.wrapping_mul(0xC2B2_AE35);
            seed ^= seed >> 16;
            let bits = (seed as u16 & 0x7FFF) | 0b010;
            if bits.count_ones() >= 5 && !taken[..h].contains(&bits) {
                break bits;
            }
        };
    }
    taken[g]
}

/// Whether cell (`c`, `r`) of glyph `g` is lit.
pub fn lit(g: usize, c: u32, r: u32) -> bool {
    c < GLYPH_COLS && r < GLYPH_ROWS && pattern(g) & (1 << (r * GLYPH_COLS + c)) != 0
}

/// `text` split into lines of at most `width` characters, at spaces.
pub fn wrap(text: &str, width: usize) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut last_space = None;
    for (i, ch) in text.char_indices() {
        if ch == ' ' {
            last_space = Some(i);
        }
        if i - start >= width {
            if let Some(sp) = last_space.filter(|&sp| sp > start) {
                out.push(&text[start..sp]);
                start = sp + 1;
                last_space = None;
            }
        }
    }
    if start < text.len() {
        out.push(&text[start..]);
    }
    out
}

/// How many of the alphabet's glyphs `mask` reads.
pub fn known_count(mask: u64, alphabet_len: usize) -> u32 {
    let full = if alphabet_len >= 64 {
        u64::MAX
    } else {
        (1u64 << alphabet_len) - 1
    };
    (mask & full).count_ones()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patterns_are_distinct() {
        let all: Vec<u16> = (0..64).map(pattern).collect();
        for (i, a) in all.iter().enumerate() {
            assert!(a.count_ones() >= 5, "glyph {i} is too thin");
            assert!(a & 0b010 != 0, "glyph {i} lacks its top stroke");
            for (j, b) in all.iter().enumerate().skip(i + 1) {
                assert_ne!(a, b, "glyphs {i} and {j} look the same");
            }
        }
    }

    #[test]
    fn wrap_breaks_at_spaces() {
        assert_eq!(
            wrap("THE FIRE SLEEPS UNDER", 9),
            ["THE FIRE", "SLEEPS", "UNDER"]
        );
        assert_eq!(wrap("SHORT", 20), ["SHORT"]);
    }
}
