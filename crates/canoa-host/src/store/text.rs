//! What text another wrote may hold before a person or an agent reads it.

/// Whether `c` can hide, reorder or smuggle text: direction marks and isolates, zero-width and invisible format characters,
/// line and paragraph separators, the soft hyphen, variation selectors and the tag block. Control characters are not here;
/// each caller decides which of those it keeps.
#[must_use]
pub(crate) fn hides_text(c: char) -> bool {
    matches!(
        c,
        '\u{202a}'..='\u{202e}'
            | '\u{2066}'..='\u{206f}'
            | '\u{200b}'..='\u{200f}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{2060}'
            | '\u{061c}'
            | '\u{feff}'
            | '\u{00ad}'
            | '\u{180e}'
            | '\u{2061}'..='\u{2064}'
            | '\u{fe00}'..='\u{fe0f}'
            | '\u{e0000}'..='\u{e007f}'
            | '\u{e0100}'..='\u{e01ef}'
    )
}
