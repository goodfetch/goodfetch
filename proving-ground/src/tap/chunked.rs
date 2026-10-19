//! Where a chunked body ends (RFC 9112, section 7.1), read without keeping
//! its data.

/// The longest chunk-size or trailer line read; a longer one ends the body.
const LINE_LIMIT: usize = 8 * 1024;

/// A chunked body's progress.
#[derive(Debug)]
pub(super) enum Chunked {
    /// Reading a chunk-size line, this long so far.
    Size(Vec<u8>),
    /// Inside a chunk's data, with this many bytes left.
    Data(u64),
    /// The CRLF after a chunk's data, with this many bytes left.
    DataEnd(u8),
    /// Reading trailer lines, the current one this long so far.
    Trailer(usize),
}

impl Default for Chunked {
    fn default() -> Self {
        Self::Size(Vec::new())
    }
}

impl Chunked {
    /// Consumes what of `bytes` belongs to the body: how many, and whether
    /// the body ended there.
    pub(super) fn consume(&mut self, bytes: &[u8]) -> (usize, bool) {
        let mut used = 0;
        while used < bytes.len() {
            let rest = &bytes[used..];
            match self {
                Self::Size(line) => {
                    let Some(end) = rest.iter().position(|byte| *byte == b'\n') else {
                        line.extend_from_slice(rest);
                        used = bytes.len();
                        if line.len() > LINE_LIMIT {
                            return (used, true);
                        }
                        break;
                    };
                    line.extend_from_slice(&rest[..end]);
                    used += end + 1;
                    let text = String::from_utf8_lossy(line);
                    let digits = text.split(';').next().unwrap_or_default().trim();
                    *self = match u64::from_str_radix(digits, 16) {
                        Ok(0) => Self::Trailer(0),
                        Ok(size) => Self::Data(size),
                        // A malformed size ends the body here.
                        Err(_) => return (used, true),
                    };
                }
                Self::Data(left) => {
                    let taken = usize::try_from(*left).unwrap_or(usize::MAX).min(rest.len());
                    used += taken;
                    *left -= taken as u64;
                    if *left == 0 {
                        *self = Self::DataEnd(2);
                    }
                }
                Self::DataEnd(left) => {
                    let taken = usize::from(*left).min(rest.len());
                    used += taken;
                    *left -= u8::try_from(taken).expect("invariant: at most 2");
                    if *left == 0 {
                        *self = Self::default();
                    }
                }
                Self::Trailer(length) => {
                    let byte = rest[0];
                    used += 1;
                    match byte {
                        b'\n' if *length == 0 => return (used, true),
                        b'\n' => *length = 0,
                        b'\r' => {}
                        _ => {
                            *length += 1;
                            if *length > LINE_LIMIT {
                                return (used, true);
                            }
                        }
                    }
                }
            }
        }
        (used, false)
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    const BODY: &[u8] = b"4\r\nWiki\r\n5;name=value\r\npedia\r\n0\r\nExpires: never\r\n\r\n";

    #[rstest]
    #[case::whole(BODY.len())]
    #[case::byte_by_byte(1)]
    #[case::in_threes(3)]
    fn a_chunked_body_ends_after_its_last_chunk_and_trailers(#[case] piece: usize) {
        let mut chunked = Chunked::default();
        let mut consumed = 0;
        let mut ended = false;
        let input = [BODY, b"GET /next"].concat();
        for part in input.chunks(piece) {
            if ended {
                break;
            }
            let (used, done) = chunked.consume(part);
            consumed += used;
            ended = done;
        }

        assert!(ended);
        assert_eq!(consumed, BODY.len());
    }

    #[test]
    fn a_body_without_trailers_ends_at_its_empty_line() {
        let body = b"3\r\nabc\r\n0\r\n\r\nrest";

        let consumed = Chunked::default().consume(body);

        assert_eq!(consumed, (body.len() - 4, true));
    }
}
