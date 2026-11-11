//! A TLS `ClientHello` read from the first bytes of a connection, and its
//! JA4 fingerprint (the specification of `FoxIO`), ported from the lab's TLS
//! fingerprint relay (`rnd/lab/fixtures/tls-fingerprint.ts`).

use serde::Serialize;

/// What a `ClientHello` says of its client, GREASE values left out.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct ClientHello {
    pub(crate) ja4: String,
    /// The JA4 without `pre_shared_key` and `early_data`, which a resumed
    /// handshake adds, so one client compares with itself across
    /// resumptions.
    pub(crate) ja4_base: String,
    pub(crate) resumed: bool,
    /// The server name the client asked for, if any.
    pub(crate) sni: Option<String>,
    pub(crate) alpn: Vec<String>,
    /// The highest version offered: `13`, `12`, `11`, `10`, or `00`.
    pub(crate) version: String,
    pub(crate) ciphers: Vec<u16>,
    /// Extension types in the order sent.
    pub(crate) extensions: Vec<u16>,
    pub(crate) signature_algorithms: Vec<u16>,
}

/// The JA4s, without what a resumed handshake adds, of the Chrome for
/// Testing build Collie pins (156.0.8078.4, `browsers.toml`), measured on
/// the Proving ground: as Collie starts it, with
/// `--disable-field-trial-config`, and with the field-trial configuration
/// built into the binary, which adds one extension (`0x12e0`). JA4 sorts
/// the extensions, so Chrome's shuffling of them changes neither. They move
/// with the pin: the rung site's test of its browser-identity page fails
/// when the pinned build sends another.
const PINNED_BROWSER_JA4: [&str; 2] = [
    "t13d1517h2_8daaf6152771_cb7bf5808d99",
    "t13d1518h2_8daaf6152771_4980c97edce0",
];

impl ClientHello {
    /// Whether the client's TLS fingerprint is the pinned browser's.
    pub(crate) fn of_the_pinned_browser(&self) -> bool {
        PINNED_BROWSER_JA4.contains(&self.ja4_base.as_str())
    }
}

/// Why the first bytes are no `ClientHello`.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct NotClientHello;

/// Parses the first TLS record of a connection. `Ok(None)` until the whole
/// record has arrived.
pub(crate) fn parse(buffer: &[u8]) -> Result<Option<ClientHello>, NotClientHello> {
    const HANDSHAKE: u8 = 22;
    const CLIENT_HELLO: u8 = 1;
    let Some(&kind) = buffer.first() else {
        return Ok(None);
    };
    if kind != HANDSHAKE {
        return Err(NotClientHello);
    }
    let Some(length) = buffer.get(3..5) else {
        return Ok(None);
    };
    let record_end = 5 + usize::from(u16::from_be_bytes([length[0], length[1]]));
    let Some(record) = buffer.get(5..record_end) else {
        return Ok(None);
    };
    let mut reader = Reader(record);
    if reader.u8()? != CLIENT_HELLO {
        return Err(NotClientHello);
    }
    reader.skip(3)?;
    let legacy_version = reader.u16()?;
    reader.skip(32)?;
    let session_id = reader.u8()?;
    reader.skip(usize::from(session_id))?;
    let cipher_length = reader.u16()?;
    let mut cipher_reader = Reader(reader.take(usize::from(cipher_length))?);
    let mut ciphers = Vec::new();
    while !cipher_reader.0.is_empty() {
        ciphers.push(cipher_reader.u16()?);
    }
    let compression = reader.u8()?;
    reader.skip(usize::from(compression))?;
    let mut hello = Extensions::default();
    // A ClientHello with no extensions ends here.
    if !reader.0.is_empty() {
        let length = reader.u16()?;
        // The lab reads to the shorter of the extensions and the record.
        let available = usize::from(length).min(reader.0.len());
        let mut extensions = Reader(reader.take(available)?);
        while !extensions.0.is_empty() {
            let kind = extensions.u16()?;
            let length = extensions.u16()?;
            let data = extensions.take(usize::from(length))?;
            if !grease(kind) {
                hello.read(kind, data);
            }
        }
    }
    Ok(Some(hello.finish(legacy_version, &ciphers)))
}

/// GREASE values (RFC 8701), which a client draws anew for each connection.
fn grease(value: u16) -> bool {
    value & 0x0f0f == 0x0a0a
}

/// A bounds-checked reader over a complete record: running out of bytes
/// means the record is no `ClientHello`.
struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, length: usize) -> Result<&'a [u8], NotClientHello> {
        if length > self.0.len() {
            return Err(NotClientHello);
        }
        let (taken, rest) = self.0.split_at(length);
        self.0 = rest;
        Ok(taken)
    }

    fn skip(&mut self, length: usize) -> Result<(), NotClientHello> {
        self.take(length).map(|_| ())
    }

    fn u8(&mut self) -> Result<u8, NotClientHello> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, NotClientHello> {
        let bytes = self.take(2)?;
        Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
    }
}

/// What the extensions say, read in order.
#[derive(Default)]
struct Extensions {
    types: Vec<u16>,
    sni: Option<String>,
    alpn: Vec<String>,
    versions: Vec<u16>,
    signature_algorithms: Vec<u16>,
}

impl Extensions {
    const SERVER_NAME: u16 = 0x0000;
    const ALPN: u16 = 0x0010;
    const SUPPORTED_VERSIONS: u16 = 0x002b;
    const SIGNATURE_ALGORITHMS: u16 = 0x000d;
    const PRE_SHARED_KEY: u16 = 0x0029;
    const EARLY_DATA: u16 = 0x002a;

    /// Reads one extension; a malformed one's data is left out, as the lab
    /// does.
    fn read(&mut self, kind: u16, data: &[u8]) {
        self.types.push(kind);
        match kind {
            Self::SERVER_NAME => {
                if data.len() > 5 {
                    let length = usize::from(u16::from_be_bytes([data[3], data[4]]));
                    let name = data.get(5..5 + length).unwrap_or(&data[5..]);
                    self.sni = Some(String::from_utf8_lossy(name).into_owned());
                }
            }
            Self::ALPN => {
                let mut at = 2;
                while let Some(&length) = data.get(at) {
                    let end = (at + 1 + usize::from(length)).min(data.len());
                    self.alpn
                        .push(String::from_utf8_lossy(&data[at + 1..end]).into_owned());
                    at = end;
                }
            }
            Self::SUPPORTED_VERSIONS => {
                self.versions
                    .extend(pairs(data.get(1..).unwrap_or_default()));
            }
            Self::SIGNATURE_ALGORITHMS => {
                self.signature_algorithms
                    .extend(pairs(data.get(2..).unwrap_or_default()));
            }
            _ => {}
        }
    }

    fn finish(self, legacy_version: u16, ciphers: &[u16]) -> ClientHello {
        let top = self
            .versions
            .iter()
            .copied()
            .filter(|version| !grease(*version))
            .max()
            .unwrap_or(legacy_version);
        let version = match top {
            0x0304 => "13",
            0x0303 => "12",
            0x0302 => "11",
            0x0301 => "10",
            _ => "00",
        };
        let ciphers: Vec<u16> = ciphers
            .iter()
            .copied()
            .filter(|cipher| !grease(*cipher))
            .collect();
        // GREASE is ignored everywhere, signature algorithms included: Chrome
        // draws a new value for each connection.
        let signature_algorithms: Vec<u16> = self
            .signature_algorithms
            .into_iter()
            .filter(|algorithm| !grease(*algorithm))
            .collect();
        let alpn_code = self.alpn.first().map_or_else(
            || "00".to_owned(),
            |first| {
                let mut characters = first.chars();
                match (characters.next(), characters.last()) {
                    (Some(only), None) => format!("{only}{only}"),
                    (Some(first), Some(last)) => format!("{first}{last}"),
                    _ => "00".to_owned(),
                }
            },
        );
        let mut sorted_ciphers = ciphers.clone();
        sorted_ciphers.sort_unstable();
        let cipher_hash = sha12(&hex_list(&sorted_ciphers));
        let fingerprint = |extensions: &[u16]| {
            let mut sorted: Vec<u16> = extensions
                .iter()
                .copied()
                .filter(|kind| *kind != Self::SERVER_NAME && *kind != Self::ALPN)
                .collect();
            sorted.sort_unstable();
            let mut hashed = hex_list(&sorted);
            if !signature_algorithms.is_empty() {
                hashed.push('_');
                hashed.push_str(&hex_list(&signature_algorithms));
            }
            format!(
                "t{version}{}{:02}{:02}{alpn_code}_{cipher_hash}_{}",
                if self.sni.is_some() { 'd' } else { 'i' },
                ciphers.len().min(99),
                extensions.len().min(99),
                sha12(&hashed)
            )
        };
        let unresumed: Vec<u16> = self
            .types
            .iter()
            .copied()
            .filter(|kind| *kind != Self::PRE_SHARED_KEY && *kind != Self::EARLY_DATA)
            .collect();
        ClientHello {
            ja4: fingerprint(&self.types),
            ja4_base: fingerprint(&unresumed),
            resumed: self.types.contains(&Self::PRE_SHARED_KEY),
            sni: self.sni,
            alpn: self.alpn,
            version: version.to_owned(),
            ciphers,
            extensions: self.types,
            signature_algorithms,
        }
    }
}

/// Big-endian `u16` pairs, a trailing odd byte left out.
fn pairs(data: &[u8]) -> impl Iterator<Item = u16> + '_ {
    data.as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_be_bytes(*pair))
}

fn hex_list(values: &[u16]) -> String {
    values
        .iter()
        .map(|value| format!("{value:04x}"))
        .collect::<Vec<_>>()
        .join(",")
}

/// The first 12 hex digits of a SHA-256.
fn sha12(text: &str) -> String {
    use sha2::{Digest as _, Sha256};
    let digest = Sha256::digest(text.as_bytes());
    digest[..6].iter().fold(String::new(), |mut text, byte| {
        use std::fmt::Write as _;
        write!(text, "{byte:02x}").expect("invariant: writing to a String succeeds");
        text
    })
}

#[cfg(test)]
mod tests {
    use rstest::rstest;
    use sha2::{Digest as _, Sha256};

    use super::*;

    fn u16s(value: u16) -> [u8; 2] {
        value.to_be_bytes()
    }

    fn extension(kind: u16, data: &[u8]) -> Vec<u8> {
        let length = u16::try_from(data.len()).unwrap();
        [&u16s(kind)[..], &u16s(length), data].concat()
    }

    /// The lab's minimal TLS 1.3 `ClientHello`: GREASE in the ciphers and the
    /// extensions, SNI, ALPN `h2`, `supported_versions` and signature
    /// algorithms.
    fn hello(signature_grease: u16, pre_shared_key: bool) -> Vec<u8> {
        let name = b"localhost";
        let sni = [&u16s(12)[..], &[0], &u16s(9), name].concat();
        let alpn = [&u16s(3)[..], &[2], b"h2"].concat();
        let mut extensions = [
            extension(0x1a1a, &[]),
            extension(0x0000, &sni),
            extension(0x0010, &alpn),
            extension(0x002b, &[2, 0x03, 0x04]),
            extension(
                0x000d,
                &[
                    &u16s(6)[..],
                    &u16s(signature_grease),
                    &u16s(0x0403),
                    &u16s(0x0804),
                ]
                .concat(),
            ),
        ]
        .concat();
        if pre_shared_key {
            extensions.extend(extension(0x0029, &[0, 0]));
        }
        let extensions_length = u16::try_from(extensions.len()).unwrap();
        let body = [
            &u16s(0x0303)[..],
            &[7; 32],
            &[0],
            &u16s(6),
            &u16s(0x0a0a),
            &u16s(0x1302),
            &u16s(0x1301),
            &[1, 0],
            &u16s(extensions_length),
            &extensions,
        ]
        .concat();
        let body_length = u16::try_from(body.len()).unwrap();
        let handshake = [&[1, 0][..], &u16s(body_length), &body].concat();
        let handshake_length = u16::try_from(handshake.len()).unwrap();
        [&[22, 3, 1][..], &u16s(handshake_length), &handshake].concat()
    }

    /// The first 12 hex digits of a SHA-256, as JA4 truncates its hashes.
    fn sha12(text: &str) -> String {
        Sha256::digest(text.as_bytes())[..6]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<Vec<_>>()
            .concat()
    }

    #[test]
    fn ja4_is_computed_without_grease_from_sorted_ciphers_and_extensions() {
        let parsed = parse(&hello(0x0a0a, false)).unwrap().unwrap();

        assert_eq!(parsed.sni.as_deref(), Some("localhost"));
        assert_eq!(parsed.alpn, ["h2"]);
        assert_eq!(
            parsed.ja4,
            format!(
                "t13d0204h2_{}_{}",
                sha12("1301,1302"),
                sha12("000d,002b_0403,0804")
            )
        );
    }

    #[test]
    fn an_incomplete_record_is_not_parsed_yet() {
        assert_eq!(parse(&hello(0x0a0a, false)[..20]), Ok(None));
    }

    #[test]
    fn bytes_that_are_no_handshake_are_refused() {
        assert_eq!(parse(b"GET / HTTP/1.1\r\n"), Err(NotClientHello));
    }

    #[rstest]
    #[case::grease_2a2a_and_5a5a(0x2a2a, 0x5a5a)]
    #[case::grease_0a0a_and_fafa(0x0a0a, 0xfafa)]
    fn grease_in_the_signature_algorithms_does_not_change_ja4(
        #[case] first: u16,
        #[case] second: u16,
    ) {
        assert_eq!(
            parse(&hello(first, false)).unwrap().unwrap().ja4,
            parse(&hello(second, false)).unwrap().unwrap().ja4
        );
    }

    #[test]
    fn a_resumed_handshake_changes_ja4_but_keeps_the_base_fingerprint() {
        let fresh = parse(&hello(0x0a0a, false)).unwrap().unwrap();
        let resumed = parse(&hello(0x0a0a, true)).unwrap().unwrap();

        assert!(resumed.resumed);
        assert!(!fresh.resumed);
        assert_ne!(resumed.ja4, fresh.ja4);
        assert_eq!(resumed.ja4_base, fresh.ja4);
    }

    proptest::proptest! {
        #[test]
        fn any_bytes_parse_or_are_refused_without_a_panic(bytes in proptest::collection::vec(proptest::num::u8::ANY, 0..512)) {
            let _ = parse(&bytes);
        }

        #[test]
        fn a_handshake_cut_anywhere_parses_or_waits_without_a_panic(cut in 0usize..200, tail in proptest::collection::vec(proptest::num::u8::ANY, 0..64)) {
            let mut bytes = hello(0x0a0a, true);
            bytes.truncate(cut.min(bytes.len()));
            bytes.extend(tail);
            let _ = parse(&bytes);
        }
    }
}
