//! Reassembly of `Transfer-Encoding: chunked` bodies.
//!
//! Three HTTP clients in this crate need this — the scan's banner grabbing, the
//! UniFi controller client, and the assistant's API client — and each had grown
//! its own copy. They agreed on the easy parts and shared the same defect: chunk
//! lengths count *bytes*, so slicing the `&str` at one panics whenever a chunk
//! boundary lands inside a multi-byte character. That is not a hypothetical for
//! a controller returning SSIDs and device aliases, or for a model's prose.

/// Reassembles a chunked body, returning whatever could be decoded.
///
/// A truncated final chunk is the expected case rather than a malformed server:
/// every caller reads a bounded prefix of the response, so the read limit cuts
/// the last chunk short routinely. Returning the partial body lets the caller
/// report a useful error instead of an empty one.
pub(crate) fn decode(body: &str) -> String {
    let bytes = body.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut pos = 0usize;

    while pos < bytes.len() {
        let Some(offset) = bytes[pos..].windows(2).position(|w| w == b"\r\n") else {
            break;
        };

        // A chunk length may carry extensions: "1a;name=value".
        let line = &bytes[pos..pos + offset];
        let token = line.split(|b| *b == b';').next().unwrap_or_default();
        let Ok(token) = std::str::from_utf8(token) else {
            break;
        };
        let Ok(size) = usize::from_str_radix(token.trim(), 16) else {
            break;
        };
        if size == 0 {
            break; // terminating chunk
        }

        let start = pos + offset + 2;
        let end = start.saturating_add(size).min(bytes.len());
        out.extend_from_slice(&bytes[start..end]);

        if end == bytes.len() {
            break; // cut short by the read limit
        }
        pos = end + 2; // step over the chunk's trailing CRLF
    }

    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reassembles_a_body_split_across_chunks() {
        let raw = "15\r\n{\"tag_name\":\"v1.9.0\",\r\ne\r\n\"draft\":false}\r\n0\r\n\r\n";
        assert_eq!(decode(raw), "{\"tag_name\":\"v1.9.0\",\"draft\":false}");
    }

    #[test]
    fn chunk_extensions_and_uppercase_hex_are_understood() {
        assert_eq!(
            decode("A;name=value\r\n0123456789\r\n0\r\n\r\n"),
            "0123456789"
        );
    }

    #[test]
    fn a_body_cut_short_by_the_read_limit_keeps_what_arrived() {
        assert_eq!(decode("ff\r\ntruncated here"), "truncated here");
    }

    #[test]
    fn a_chunk_boundary_inside_a_multibyte_character_does_not_panic() {
        // The failure this exists to prevent: an em dash is three bytes, and
        // slicing a &str at a byte index inside it panics. A controller
        // returning a device named "Marek's — office AP" was enough.
        let text = "café — naïve";
        let bytes = text.as_bytes();
        let split = 4; // lands inside the é

        let raw = format!(
            "{:x}\r\n{}\r\n{:x}\r\n{}\r\n0\r\n\r\n",
            split,
            String::from_utf8_lossy(&bytes[..split]),
            bytes.len() - split,
            String::from_utf8_lossy(&bytes[split..]),
        );

        // Lossy conversion of the split halves changes their byte lengths, so
        // the exact output is not the point — not panicking is.
        let _ = decode(&raw);
    }

    #[test]
    fn a_multibyte_character_whole_within_a_chunk_survives_intact() {
        let text = "café — naïve";
        let raw = format!("{:x}\r\n{text}\r\n0\r\n\r\n", text.len());
        assert_eq!(decode(&raw), text);
    }

    #[test]
    fn junk_is_returned_as_far_as_it_parsed_rather_than_panicking() {
        assert_eq!(decode(""), "");
        assert_eq!(decode("not-hex\r\nbody"), "");
        assert_eq!(decode("\r\n\r\n"), "");
    }
}
