//! Extended grapheme-cluster segmentation (a pragmatic UAX #29 subset).
//!
//! Swift counts and indexes a `String` by extended grapheme cluster. The
//! offline-build constraint rules out the `unicode-segmentation` crate, so this
//! module ships a self-contained segmenter covering combining marks, variation
//! selectors, CRLF, ZWJ emoji sequences, and regional-indicator flag pairs.
//! It lives in core because both the interpreter (String subscripting,
//! ADR-0006) and `qswift-std` (the String method surface) need it.

const ZWJ: char = '\u{200D}';

/// Segment a string into extended grapheme clusters.
pub fn graphemes(s: &str) -> Vec<String> {
    let chars: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let start = i;
        i += 1;
        loop {
            if i >= chars.len() {
                break;
            }
            let prev = chars[i - 1];
            let cur = chars[i];
            // CRLF stays together.
            if prev == '\r' && cur == '\n' {
                i += 1;
                continue;
            }
            // Extend (combining marks / variation selectors) and ZWJ join.
            if is_extend(cur) || cur == ZWJ {
                i += 1;
                continue;
            }
            // A ZWJ glues whatever follows (emoji sequences).
            if prev == ZWJ {
                i += 1;
                continue;
            }
            // Pair regional indicators (flags): join only when the run from the
            // cluster start is currently odd in length.
            if is_regional(prev) && is_regional(cur) {
                let run = chars[start..i]
                    .iter()
                    .rev()
                    .take_while(|c| is_regional(**c))
                    .count();
                if run % 2 == 1 {
                    i += 1;
                    continue;
                }
            }
            break;
        }
        out.push(chars[start..i].iter().collect());
    }
    out
}

fn is_regional(c: char) -> bool {
    ('\u{1F1E6}'..='\u{1F1FF}').contains(&c)
}

/// Combining marks and variation selectors that extend a grapheme cluster.
fn is_extend(c: char) -> bool {
    matches!(c as u32,
        0x0300..=0x036F   // combining diacritical marks
        | 0x0483..=0x0489
        | 0x0591..=0x05BD
        | 0x0610..=0x061A
        | 0x064B..=0x065F
        | 0x0670
        | 0x06D6..=0x06DC
        | 0x0E31 | 0x0E34..=0x0E3A
        | 0x1AB0..=0x1AFF // combining diacritical marks extended
        | 0x1DC0..=0x1DFF // combining diacritical marks supplement
        | 0x20D0..=0x20FF // combining diacritical marks for symbols
        | 0xFE00..=0xFE0F // variation selectors
        | 0xFE20..=0xFE2F // combining half marks
        | 0xE0100..=0xE01EF // variation selectors supplement
    )
}
