//! Standard base-64 encode / decode.
//!
//! Used by both the JSON coding layer (`interp::coding`) and
//! `tswift-foundation`'s `Data` methods (`base64EncodedString`, `base64EncodedData`,
//! `Data(base64Encoded:)`).  Keeping one copy here avoids duplicating ~60 lines
//! across two crates while respecting the `tswift-foundation → tswift-core`
//! dependency direction.

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Encode `bytes` as a standard base64 string (with `=` padding).
pub fn encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[(triple >> 18 & 0x3F) as usize] as char);
        out.push(ALPHABET[(triple >> 12 & 0x3F) as usize] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(triple >> 6 & 0x3F) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[(triple & 0x3F) as usize] as char
        } else {
            '='
        });
    }
    out
}

/// Decode a standard base64 string to bytes. Returns `None` on invalid input.
///
/// Whitespace in the input is stripped before decoding (matching Foundation's
/// `Data(base64Encoded:)` lenient mode).  The length after stripping must be a
/// whole number of 4-character groups; padding `=` characters are only valid in
/// the final group.
pub fn decode(input: &str) -> Option<Vec<u8>> {
    fn val(c: u8) -> Option<u32> {
        match c {
            b'A'..=b'Z' => Some((c - b'A') as u32),
            b'a'..=b'z' => Some((c - b'a' + 26) as u32),
            b'0'..=b'9' => Some((c - b'0' + 52) as u32),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let cleaned: Vec<u8> = input.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    // Empty input decodes to empty Data (matching Foundation); other lengths
    // must be a whole number of 4-char groups.
    if !cleaned.len().is_multiple_of(4) {
        return None;
    }
    let chunk_count = cleaned.len() / 4;
    let mut out = Vec::with_capacity(chunk_count * 3);
    for (chunk_index, chunk) in cleaned.chunks(4).enumerate() {
        let pad = chunk.iter().filter(|&&c| c == b'=').count();
        // Padding is only ever valid (1 or 2 chars) in the final chunk, and the
        // pad must be a trailing run.
        if pad > 0 && (chunk_index != chunk_count - 1 || pad > 2) {
            return None;
        }
        if pad > 0 && chunk[4 - pad..].iter().any(|&c| c != b'=') {
            return None;
        }
        let mut acc = 0u32;
        for &c in &chunk[..4 - pad] {
            acc = (acc << 6) | val(c)?;
        }
        acc <<= 6 * pad;
        out.push((acc >> 16 & 0xFF) as u8);
        if pad < 2 {
            out.push((acc >> 8 & 0xFF) as u8);
        }
        if pad < 1 {
            out.push((acc & 0xFF) as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_empty() {
        assert_eq!(encode(b""), "");
    }

    #[test]
    fn encode_one_byte_pads_two() {
        assert_eq!(encode(b"H"), "SA==");
    }

    #[test]
    fn encode_two_bytes_pads_one() {
        assert_eq!(encode(b"Hi"), "SGk=");
    }

    #[test]
    fn encode_three_bytes_no_padding() {
        assert_eq!(encode(b"Man"), "TWFu");
    }

    #[test]
    fn encode_hello() {
        // "Hello" = [72, 101, 108, 108, 111]
        assert_eq!(encode(b"Hello"), "SGVsbG8=");
    }

    #[test]
    fn decode_empty() {
        assert_eq!(decode("").unwrap(), b"");
    }

    #[test]
    fn decode_hello_round_trip() {
        let encoded = encode(b"Hello");
        assert_eq!(decode(&encoded).unwrap(), b"Hello");
    }

    #[test]
    fn decode_invalid_length_returns_none() {
        assert!(decode("SGk").is_none()); // not a multiple of 4
    }

    #[test]
    fn decode_invalid_chars_returns_none() {
        assert!(decode("!!!!=").is_none());
    }
}
