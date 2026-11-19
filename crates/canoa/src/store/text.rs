//! What text another wrote may hold before a person or an agent reads it.

/// Whether `c` can hide, reorder or smuggle text: direction marks and isolates, zero-width and invisible format characters,
/// line and paragraph separators, the soft hyphen, variation selectors, the tag block, the Hangul and Khmer fillers and the combining
/// grapheme joiner (which draw as blanks), the musical formatting characters and the annotation marks. Control characters are not here;
/// each caller decides which of those it keeps.
#[must_use]
pub fn hides_text(c: char) -> bool {
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
            | '\u{115f}'
            | '\u{1160}'
            | '\u{3164}'
            | '\u{ffa0}'
            | '\u{034f}'
            | '\u{17b4}'..='\u{17b5}'
            | '\u{1d173}'..='\u{1d17a}'
            | '\u{fff9}'..='\u{fffb}'
    )
}
