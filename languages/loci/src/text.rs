//! Text helpers that follow the rules of Python strings, as loci-core uses them.

use std::sync::OnceLock;

use regex::Regex;
use unicode_general_category::{GeneralCategory, get_general_category};
use unicode_normalization::UnicodeNormalization;

/// The characters that `str.isspace` accepts in Python.
pub(crate) fn is_py_space(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

/// `str.strip()`.
pub(crate) fn py_strip(s: &str) -> &str {
    s.trim_matches(is_py_space)
}

/// `str.split()` joined by one space.
pub(crate) fn collapse_space(s: &str) -> String {
    s.split(is_py_space)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// `str.casefold()`.
pub(crate) fn casefold(s: &str) -> String {
    caseless::default_case_fold_str(s)
}

pub(crate) fn nfc(s: &str) -> String {
    s.nfc().collect()
}

/// `urllib.parse.unquote`: decode `%XX` as UTF-8 and replace bad bytes.
pub(crate) fn unquote(s: &str) -> String {
    percent_encoding::percent_decode_str(s)
        .decode_utf8_lossy()
        .into_owned()
}

/// The first character of the general category name, as `unicodedata.category(c)[0]`.
fn category_class(c: char) -> char {
    use GeneralCategory::*;
    match get_general_category(c) {
        UppercaseLetter | LowercaseLetter | TitlecaseLetter | ModifierLetter | OtherLetter => 'L',
        NonspacingMark | SpacingMark | EnclosingMark => 'M',
        DecimalNumber | LetterNumber | OtherNumber => 'N',
        ConnectorPunctuation | DashPunctuation | OpenPunctuation | ClosePunctuation
        | InitialPunctuation | FinalPunctuation | OtherPunctuation => 'P',
        MathSymbol | CurrencySymbol | ModifierSymbol | OtherSymbol => 'S',
        SpaceSeparator | LineSeparator | ParagraphSeparator => 'Z',
        Control | Format | Surrogate | PrivateUse | Unassigned => 'C',
        _ => 'C',
    }
}

/// True when Python's `unicodedata.category(c)[0]` is one of `classes`.
pub(crate) fn category_in(c: char, classes: &str) -> bool {
    classes.contains(category_class(c))
}

fn link_in_heading() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"!?\[([^\]]*)\]\([^)]*\)").expect("valid regex"))
}

/// The GitHub heading slug. A Markdown link in the heading counts as its text.
pub(crate) fn github_slug(text: &str) -> String {
    let text = link_in_heading().replace_all(text, "$1").to_lowercase();
    text.chars()
        .filter(|&c| {
            let alnum = matches!(category_class(c), 'L' | 'N');
            alnum || c == '_' || c == '-' || c == ' ' || category_class(c) == 'M'
        })
        .map(|c| if c == ' ' { '-' } else { c })
        .collect()
}

/// The heading text with the characters that a link cannot carry turned into spaces.
pub(crate) fn loose_heading(text: &str, case_sensitive: bool) -> String {
    let spaced: String = nfc(text)
        .chars()
        .map(|c| if "#|^:%[]\\".contains(c) { ' ' } else { c })
        .collect();
    let folded = collapse_space(&spaced);
    if case_sensitive {
        folded
    } else {
        casefold(&folded)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_follows_github() {
        assert_eq!(github_slug("My Heading!"), "my-heading");
        assert_eq!(github_slug("A [link](x) here"), "a-link-here");
    }

    #[test]
    fn loose_heading_ignores_marks() {
        assert_eq!(loose_heading("a:  b|c", false), "a b c");
    }

    #[test]
    fn strip_follows_python() {
        assert_eq!(py_strip("\u{1c} a \u{a0}"), "a");
    }
}
