//! The languages captions are translated into (wire v135).
//!
//! A language is a small number on the wire: 0 is none (no translation, or
//! a caption in the words as said), and `1..=LANGS.len()` index [`LANGS`].
//! A player picks one with `/lang <code>` (`admin::AdminCmd::Lang`); the
//! client sends its system language that way at join, so nobody has to.
//!
//! The list is the set the shard will translate into, not the set it can
//! hear: a transcript is in whatever language was spoken.

/// Width of a language on the wire. Five bits hold 31 languages.
pub const LANG_BITS: u32 = 5;

/// `(code, name)`: the ISO 639-1 code a player types (`fil` has none) and
/// the language's English name, which the translator is told. **Order is
/// the wire**: append only.
pub const LANGS: [(&str, &str); 30] = [
    ("en", "English"),
    ("es", "Spanish"),
    ("pt", "Portuguese"),
    ("fr", "French"),
    ("de", "German"),
    ("it", "Italian"),
    ("nl", "Dutch"),
    ("pl", "Polish"),
    ("ru", "Russian"),
    ("uk", "Ukrainian"),
    ("tr", "Turkish"),
    ("ar", "Arabic"),
    ("hi", "Hindi"),
    ("id", "Indonesian"),
    ("vi", "Vietnamese"),
    ("th", "Thai"),
    ("zh", "Chinese"),
    ("ja", "Japanese"),
    ("ko", "Korean"),
    ("sv", "Swedish"),
    ("cs", "Czech"),
    ("ro", "Romanian"),
    ("hu", "Hungarian"),
    ("fil", "Filipino"),
    ("el", "Greek"),
    ("fi", "Finnish"),
    ("da", "Danish"),
    ("no", "Norwegian"),
    ("he", "Hebrew"),
    ("ms", "Malay"),
];

const _: () = assert!(LANGS.len() < 1 << LANG_BITS);

/// The highest language number.
pub const LANG_MAX: u8 = LANGS.len() as u8;

/// A language's number from a code or a locale: `es`, `pt-BR`,
/// `zh_Hans_CN.UTF-8`, `EN`. `None` for one not in [`LANGS`] (and for `C`
/// and `POSIX`, which name no language).
pub fn lang_of(locale: &str) -> Option<u8> {
    let primary = locale
        .split(['-', '_', '.', '@'])
        .next()?
        .trim()
        .to_ascii_lowercase();
    // Old and sibling codes for the same language.
    let code = match primary.as_str() {
        "tl" => "fil",
        "nb" | "nn" => "no",
        "iw" => "he",
        "in" => "id",
        c => c,
    };
    LANGS
        .iter()
        .position(|(c, _)| *c == code)
        .map(|i| i as u8 + 1)
}

/// The code for language `lang`, `None` for 0 or past the list.
pub fn lang_code(lang: u8) -> Option<&'static str> {
    LANGS.get((lang as usize).checked_sub(1)?).map(|(c, _)| *c)
}

/// The English name of language `lang`.
pub fn lang_name(lang: u8) -> Option<&'static str> {
    LANGS.get((lang as usize).checked_sub(1)?).map(|(_, n)| *n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locales_name_their_language() {
        assert_eq!(lang_of("en"), Some(1));
        assert_eq!(lang_of("pt-BR"), lang_of("pt"));
        assert_eq!(lang_of("zh_Hans_CN.UTF-8"), lang_of("zh"));
        assert_eq!(lang_of("ES"), Some(2));
        assert_eq!(lang_of("nb-NO"), lang_of("no"));
        assert_eq!(lang_of("tl"), lang_of("fil"));
        assert_eq!(lang_of("C"), None);
        assert_eq!(lang_of(""), None);
        assert_eq!(lang_of("xx"), None);
    }

    #[test]
    fn codes_round_trip() {
        for n in 1..=LANG_MAX {
            assert_eq!(lang_of(lang_code(n).unwrap()), Some(n));
            assert!(lang_name(n).is_some());
        }
        assert_eq!(lang_code(0), None);
        assert_eq!(lang_code(LANG_MAX + 1), None);
    }
}
