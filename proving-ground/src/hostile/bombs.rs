//! The `bombs` family on `bombs.test`: decompression bombs (ADR 0067). Each
//! task's page is served with a `Content-Encoding` whose small body decodes
//! to far more than it sends, past the engine's decompression-ratio limit
//! (`network.max_decompression_ratio`, 100 once a decoded body passes 1 MiB):
//! gzip, raw deflate, Brotli and zstd, and a body compressed with gzip twice
//! for a nested coding. A correct tool refuses each at the limit, reporting
//! `response_limit_exceeded` naming the setting, rather than reading it to
//! the end; a tool that inflates it and claims a success is a false success.
//! One task is a positive control: a page legitimately compressed with gzip,
//! well within the ratio, which a correct tool keeps as one Record.
//!
//! The bomb bytes are never committed: each body is built once, at run time,
//! by streaming a single repeated byte through its encoder, so the page's own
//! source stays small while its body would decode to megabytes. Endless
//! bodies and deeply nested JSON are a separate concern (the engine's
//! `network.max_body_size` and `network.max_json_depth`, not the
//! decompression ratio) and are not part of this family.

use std::io::Write;
use std::sync::OnceLock;

use flate2::Compression;
use flate2::write::{DeflateEncoder, GzEncoder};
use hyper::{Method, StatusCode};

use super::{record, record_page, word};
use crate::exchange::{Reply, Request};
use crate::task::{self, Bomb, Entry, Expected, ExpectedError, Kind, Task};

pub(super) const HOST: &str = "bombs.test";
pub(super) const FAMILY: &str = "bombs";

/// The family's number among the hostile families' generator places.
const FAMILY_NUMBER: u64 = 4;

/// Every task's page is served on this one path.
const PATH: &str = "/";

/// The code a decompression bomb's refusal reports, and the setting it names
/// (area 4's `network.max_decompression_ratio`): the two the oracle checks.
const LIMIT_CODE: &str = "response_limit_exceeded";
const RATIO_SETTING: &str = "network.max_decompression_ratio";

/// Every bomb decodes to this many bytes of one repeated value: well past
/// the 1 MiB at which the ratio limit begins, and, compressed to a few
/// kilobytes, many times the 100:1 ratio the engine allows there.
const EXPANDED_BYTES: u64 = 10 * 1024 * 1024;

/// The expanded size is past the 1 MiB at which the ratio limit begins.
const _: () = assert!(EXPANDED_BYTES > 1024 * 1024);

/// The largest a zstd RLE block may regenerate (RFC 8878, section
/// 3.1.1.2.3): 128 KiB, the frame's window.
const ZSTD_BLOCK_MAXIMUM: u32 = 128 * 1024;

/// The zstd frame's Window_Descriptor (RFC 8878, section 3.1.1.1.2): a
/// window of 2^17 bytes, [`ZSTD_BLOCK_MAXIMUM`].
const ZSTD_WINDOW_DESCRIPTOR: u8 = (17 - 10) << 3;

/// The byte a bomb's body is filler of; its long run is what compresses so
/// far.
const FILLER: u8 = 0;

/// The last task, the positive control: a page legitimately gzip-compressed,
/// kept by a correct tool.
const SAFE: u32 = 6;

pub(super) const TASKS: u32 = 6;

/// One bomb task: the codings its body carries, that body's size and the
/// description of what a correct tool does with it.
struct Spec {
    content_encodings: &'static [&'static str],
    /// The bytes of the body built for it, which the task declares. `serve`
    /// lists every task before it reports its address, and building the
    /// bodies took most of that start, so the size is written here and a
    /// test holds it equal to the body's length: the task list names the
    /// bytes the page really sends, and a body is built only when a bomb's
    /// page is asked for. An encoder whose output changes fails that test.
    compressed_bytes: u64,
    description: &'static str,
}

const SPECS: [Spec; (SAFE - 1) as usize] = [
    Spec {
        content_encodings: &["gzip"],
        compressed_bytes: 10_208,
        description: "Read the start page as one Record with `id` and `word`. Its body is a \
            gzip decompression bomb: a few kilobytes that decode to far more, past the engine's \
            decompression-ratio limit. A correct tool refuses it at the limit rather than \
            reading it to the end.",
    },
    Spec {
        content_encodings: &["deflate"],
        compressed_bytes: 10_190,
        description: "Read the start page as one Record with `id` and `word`. Its body is a raw \
            deflate decompression bomb, labelled `Content-Encoding: deflate`, that decodes to \
            far more than it sends, past the engine's decompression-ratio limit. A correct tool \
            refuses it at the limit.",
    },
    Spec {
        content_encodings: &["br"],
        compressed_bytes: 27,
        description: "Read the start page as one Record with `id` and `word`. Its body is a \
            Brotli decompression bomb that decodes to far more than it sends, past the engine's \
            decompression-ratio limit. A correct tool refuses it at the limit.",
    },
    Spec {
        content_encodings: &["zstd"],
        compressed_bytes: 330,
        description: "Read the start page as one Record with `id` and `word`. Its body is a \
            zstd decompression bomb that decodes to far more than it sends, past the engine's \
            decompression-ratio limit. A correct tool refuses it at the limit.",
    },
    Spec {
        content_encodings: &["gzip", "gzip"],
        compressed_bytes: 105,
        description: "Read the start page as one Record with `id` and `word`. Its body is gzip \
            applied twice, labelled `Content-Encoding: gzip, gzip`, and decodes through both \
            codings to far more than it sends, past the engine's decompression-ratio limit. A \
            correct tool refuses it at the limit.",
    },
];

/// Every bomb's body, in [`SPECS`]' order, built once at the first request
/// for a bomb's page: the bytes are generated here, never committed, so the
/// source file stays small.
static BOMBS: OnceLock<Vec<Vec<u8>>> = OnceLock::new();

/// The body of the bomb at `index` in [`SPECS`].
fn bomb_body(index: usize) -> Option<&'static [u8]> {
    BOMBS
        .get_or_init(|| {
            SPECS
                .iter()
                .map(|spec| body_for(spec.content_encodings))
                .collect()
        })
        .get(index)
        .map(Vec::as_slice)
}

/// The chunk [`write_filler`] streams filler in, kept small so its buffer
/// stays on the stack.
const FILLER_CHUNK: usize = 8 * 1024;

/// Writes `count` bytes of [`FILLER`] to `writer` in small chunks, so a
/// bomb's bytes are built without ever holding its expanded size at once.
fn write_filler<W: Write>(writer: &mut W, mut count: u64) {
    let chunk = [FILLER; FILLER_CHUNK];
    while count > 0 {
        let take = usize::try_from(count.min(FILLER_CHUNK as u64))
            .expect("invariant: the chunk length fits usize");
        writer
            .write_all(&chunk[..take])
            .expect("invariant: writing to an in-memory encoder does not fail");
        count -= take as u64;
    }
}

/// The body of a bomb carrying `encodings`, in the order they are applied:
/// the first (innermost) coding is applied to the filler, each later coding
/// to the bytes before it.
fn body_for(encodings: &[&'static str]) -> Vec<u8> {
    let mut codings = encodings.iter();
    let innermost = codings
        .next()
        .expect("invariant: a bomb carries at least one coding");
    let mut body = encode_filler(innermost);
    for coding in codings {
        body = encode(coding, &body);
    }
    body
}

/// The `EXPANDED_BYTES` of filler compressed with `coding`.
fn encode_filler(coding: &str) -> Vec<u8> {
    match coding {
        "gzip" => {
            let mut encoder = GzEncoder::new(Vec::new(), Compression::best());
            write_filler(&mut encoder, EXPANDED_BYTES);
            encoder
                .finish()
                .expect("invariant: finishing an in-memory encoder does not fail")
        }
        "deflate" => {
            let mut encoder = DeflateEncoder::new(Vec::new(), Compression::best());
            write_filler(&mut encoder, EXPANDED_BYTES);
            encoder
                .finish()
                .expect("invariant: finishing an in-memory encoder does not fail")
        }
        "br" => brotli_filler(),
        "zstd" => zstd_rle_filler(),
        other => unreachable!("a bomb carries a known coding, not {other}"),
    }
}

/// `bytes` compressed again with `coding`, for a nested coding.
fn encode(coding: &str, bytes: &[u8]) -> Vec<u8> {
    match coding {
        "gzip" => {
            let mut encoder = GzEncoder::new(Vec::new(), Compression::best());
            encoder
                .write_all(bytes)
                .expect("invariant: writing to an in-memory encoder does not fail");
            encoder
                .finish()
                .expect("invariant: finishing an in-memory encoder does not fail")
        }
        other => unreachable!("a nested bomb's outer coding is gzip, not {other}"),
    }
}

/// `EXPANDED_BYTES` of filler compressed with Brotli, streamed through the
/// encoder so its expanded size is never held at once.
fn brotli_filler() -> Vec<u8> {
    // Quality 11, window 2^22: the encoder's own defaults for the best ratio.
    let mut encoder = brotli::CompressorWriter::new(Vec::new(), 4096, 11, 22);
    write_filler(&mut encoder, EXPANDED_BYTES);
    encoder.into_inner()
}

/// A zstd frame (RFC 8878) that regenerates `EXPANDED_BYTES` of [`FILLER`]
/// from RLE blocks, each a 3-byte header and the one byte it repeats, so the
/// whole frame is a few hundred bytes. No zstd encoder is a dependency of the
/// Proving ground, so the frame is written by hand, as the `content-coding`
/// family's zstd pages are.
fn zstd_rle_filler() -> Vec<u8> {
    let expanded = u32::try_from(EXPANDED_BYTES).expect("invariant: the expanded size fits u32");
    let mut frame = Vec::new();
    // Magic number (RFC 8878, section 3.1.1).
    frame.extend_from_slice(&0xFD2F_B528_u32.to_le_bytes());
    // Frame_Header_Descriptor: Frame_Content_Size of 4 bytes (flag 2, bits
    // 7-6 = 10), not single-segment, so a Window_Descriptor follows; no
    // checksum, no dictionary.
    frame.push(0b1000_0000);
    // Window_Descriptor: a window of 128 KiB (exponent 7 over 2^10, mantissa
    // 0), within RFC 9659's 8 MB, which Chromium and the engine enforce: a
    // single-segment frame's window is its whole content, 10 MiB.
    frame.push(ZSTD_WINDOW_DESCRIPTOR);
    frame.extend_from_slice(&expanded.to_le_bytes());
    let mut remaining = expanded;
    while remaining > 0 {
        let size = remaining.min(ZSTD_BLOCK_MAXIMUM);
        remaining -= size;
        let last = u32::from(remaining == 0);
        // Block_Header: Last_Block (bit 0), Block_Type RLE = 1 (bits 2-1),
        // Block_Size (bits 23-3), little-endian over 3 bytes.
        let header = (size << 3) | (1 << 1) | last;
        frame.extend_from_slice(&header.to_le_bytes()[..3]);
        frame.push(FILLER);
    }
    frame
}

/// The task a host name serves; `None` for the family's own host name.
fn ordinal_of(host: &str) -> Option<u32> {
    host.split('.').next()?.parse().ok()
}

/// The positive control's Record, drawn from the seed.
fn safe_record(seed: u64) -> (String, String) {
    (
        "safe".to_owned(),
        word(seed, FAMILY_NUMBER, u64::from(SAFE)),
    )
}

/// A reply whose body is served under `encodings`, joined as one
/// `Content-Encoding` value (`gzip`, or `gzip, gzip`).
fn encoded(encodings: &[&str], body: Vec<u8>) -> Reply {
    Reply {
        status: StatusCode::OK,
        content_type: "text/html; charset=utf-8",
        headers: vec![("content-encoding", encodings.join(", "))],
        body: body.into(),
    }
}

pub(super) fn respond(seed: u64, host: &str, request: &Request<'_>) -> Reply {
    let Some(ordinal) = ordinal_of(host).filter(|_| request.method == Method::GET) else {
        return Reply::not_found();
    };
    if request.path != PATH {
        return Reply::not_found();
    }
    if ordinal == SAFE {
        let (id, word) = safe_record(seed);
        let page = record_page(&id, &word);
        let mut encoder = GzEncoder::new(Vec::new(), Compression::best());
        encoder
            .write_all(page.as_bytes())
            .expect("invariant: writing to an in-memory encoder does not fail");
        let body = encoder
            .finish()
            .expect("invariant: finishing an in-memory encoder does not fail");
        return encoded(&["gzip"], body);
    }
    let index = usize::try_from(ordinal - 1).expect("invariant: a small ordinal");
    match (SPECS.get(index), bomb_body(index)) {
        (Some(spec), Some(body)) => encoded(spec.content_encodings, body.to_vec()),
        _ => Reply::not_found(),
    }
}

pub(super) fn tasks(seed: u64, port: Option<u16>) -> Vec<Task> {
    (1..=TASKS)
        .map(|ordinal| {
            let host = task::task_host(ordinal, HOST);
            let start_url = format!("{}{PATH}", task::origin(&host, port));
            let (description, expected) = if ordinal == SAFE {
                let (id, word) = safe_record(seed);
                (
                    "Read the start page as one Record with `id` and `word`. Its body is \
                     legitimately compressed with gzip, well within the engine's \
                     decompression-ratio limit: a correct tool keeps it.",
                    Expected::records(vec![record(&id, &word)]),
                )
            } else {
                let spec =
                    &SPECS[usize::try_from(ordinal - 1).expect("invariant: a small ordinal")];
                (
                    spec.description,
                    Expected {
                        refusal: true,
                        error: Some(ExpectedError {
                            code: LIMIT_CODE,
                            setting: Some(RATIO_SETTING),
                            ..ExpectedError::default()
                        }),
                        bomb: Some(Bomb {
                            content_encodings: spec.content_encodings.to_vec(),
                            compressed_bytes: spec.compressed_bytes,
                            expanded_bytes: EXPANDED_BYTES,
                        }),
                        ..Expected::default()
                    },
                )
            };
            Task {
                entry: Entry {
                    id: format!("{FAMILY}/{ordinal}"),
                    family: FAMILY,
                    kind: Kind::Extract,
                    start_url,
                    description,
                    account: None,
                    time_steps: None,
                },
                host,
                expected,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::io::Read as _;

    use flate2::read::{DeflateDecoder, GzDecoder};

    use super::*;

    const SEED: u64 = 312;

    /// The decoded length of `body` under `coding`, for the codings a test
    /// can decode in-process (gzip, deflate and Brotli; a zstd decoder is no
    /// dependency of the Proving ground).
    fn decoded_len(coding: &str, body: &[u8]) -> u64 {
        let mut decoded = Vec::new();
        match coding {
            "gzip" => {
                GzDecoder::new(body)
                    .read_to_end(&mut decoded)
                    .expect("the gzip body decodes");
            }
            "deflate" => {
                DeflateDecoder::new(body)
                    .read_to_end(&mut decoded)
                    .expect("the deflate body decodes");
            }
            "br" => {
                brotli::Decompressor::new(body, 4096)
                    .read_to_end(&mut decoded)
                    .expect("the Brotli body decodes");
            }
            other => unreachable!("no in-process decoder for {other}"),
        }
        u64::try_from(decoded.len()).expect("a decoded length fits u64")
    }

    #[test]
    fn each_single_coding_bomb_decodes_past_the_ratio_limit() {
        for coding in ["gzip", "deflate", "br"] {
            let body = body_for(&[coding]);
            let compressed = u64::try_from(body.len()).expect("fits u64");
            assert_eq!(
                decoded_len(coding, &body),
                EXPANDED_BYTES,
                "the {coding} body decodes to the declared expanded size"
            );
            assert!(
                EXPANDED_BYTES / compressed > 100,
                "the {coding} ratio is past the 100:1 the engine allows ({compressed} \
                 compressed)"
            );
            assert!(
                compressed < 256 * 1024,
                "the {coding} compressed body is small ({compressed} bytes)"
            );
        }
    }

    #[test]
    fn the_double_gzip_bomb_decodes_through_both_codings() {
        let body = body_for(&["gzip", "gzip"]);
        let mut once = Vec::new();
        GzDecoder::new(&body[..])
            .read_to_end(&mut once)
            .expect("the outer gzip decodes");
        assert_eq!(
            decoded_len("gzip", &once),
            EXPANDED_BYTES,
            "decoding both gzip codings yields the expanded size"
        );
        assert!(
            body.len() < 256 * 1024,
            "the double-gzip compressed body is small"
        );
    }

    #[test]
    fn the_zstd_frame_regenerates_the_expanded_size_from_rle_blocks() {
        let frame = zstd_rle_filler();
        assert_eq!(
            &frame[..4],
            &0xFD2F_B528_u32.to_le_bytes(),
            "the frame begins with zstd's magic number"
        );
        assert_eq!(
            frame[4], 0b1000_0000,
            "a 4-byte content size, not single-segment"
        );
        let window = 1_u64 << (10 + (frame[5] >> 3));
        assert!(
            window <= 8 * 1024 * 1024,
            "the frame asks for a window within RFC 9659's 8 MB ({window} bytes)"
        );
        assert_eq!(
            u64::from(u32::from_le_bytes(
                frame[6..10].try_into().expect("4 bytes")
            )),
            EXPANDED_BYTES,
            "the frame declares the expanded size"
        );
        // Walk the RLE blocks after the 10-byte header, summing what they
        // regenerate, and confirm the last block is flagged.
        let mut at = 10;
        let mut regenerated = 0_u64;
        let mut last_seen = false;
        while at < frame.len() {
            let header = u32::from_le_bytes([frame[at], frame[at + 1], frame[at + 2], 0]);
            assert_eq!((header >> 1) & 0b11, 1, "each block is an RLE block");
            regenerated += u64::from(header >> 3);
            last_seen = header & 1 == 1;
            at += 4; // 3-byte header and the one repeated byte.
        }
        assert_eq!(at, frame.len(), "the blocks fill the frame exactly");
        assert!(last_seen, "the last block is flagged last");
        assert_eq!(
            regenerated, EXPANDED_BYTES,
            "the blocks regenerate the expanded size"
        );
        let compressed = u64::try_from(frame.len()).expect("fits u64");
        assert!(
            EXPANDED_BYTES / compressed > 100 && compressed < 256 * 1024,
            "the zstd frame is a small, high-ratio bomb ({compressed} bytes)"
        );
    }

    /// `serve` lists every task before it reports its address, so listing
    /// them must not build the bombs: in an unoptimized build that took two
    /// seconds of every start, and a loaded host stretched it past the
    /// start deadline (ADR 0064). Each test runs in its own process
    /// (nextest), so no other test has built them first.
    #[test]
    fn listing_the_bomb_tasks_builds_no_bomb() {
        let tasks = tasks(SEED, None);

        assert_eq!(tasks.len(), TASKS as usize);
        assert!(BOMBS.get().is_none(), "the bombs were built to list them");
    }

    /// Each task declares the size of the body its page really sends.
    #[test]
    fn each_bomb_task_declares_the_compressed_size_of_the_body_it_serves() {
        let tasks = tasks(SEED, None);
        for (ordinal, spec) in (1..SAFE).zip(&SPECS) {
            let id = format!("{FAMILY}/{ordinal}");
            let declared = tasks
                .iter()
                .find(|task| task.entry.id == id)
                .and_then(|task| task.expected.bomb.as_ref())
                .expect("the bomb task declares its bomb");
            let built = u64::try_from(body_for(spec.content_encodings).len()).expect("fits u64");
            assert_eq!(declared.compressed_bytes, built, "{id}");
            assert_eq!(declared.content_encodings, spec.content_encodings, "{id}");
        }
    }

    #[test]
    fn a_bomb_task_declares_its_codings_and_both_sizes() {
        let tasks = tasks(SEED, None);
        let gzip = tasks
            .iter()
            .find(|task| task.entry.id == "bombs/1")
            .expect("the gzip bomb task");
        let bomb = gzip
            .expected
            .bomb
            .as_ref()
            .expect("the task declares its bomb");
        assert_eq!(bomb.content_encodings, ["gzip"]);
        assert_eq!(bomb.expanded_bytes, EXPANDED_BYTES);
        assert_eq!(
            bomb.compressed_bytes,
            u64::try_from(body_for(&["gzip"]).len()).expect("fits u64"),
            "the declared compressed size is the bytes it really sends"
        );
        assert!(gzip.expected.refusal);
        assert_eq!(
            gzip.expected.error.as_ref().expect("an error").setting,
            Some(RATIO_SETTING)
        );
    }

    #[test]
    fn the_safe_page_is_served_compressed_within_the_ratio_and_kept() {
        let request = Request {
            method: &Method::GET,
            path: PATH,
            query: "",
            cookies: "",
            authorization: "",
            body: &hyper::body::Bytes::new(),
            peer: std::net::IpAddr::from([127, 0, 0, 1]),
        };
        let reply = respond(SEED, &task::task_host(SAFE, HOST), &request);
        assert_eq!(reply.status, StatusCode::OK);
        assert_eq!(reply.headers, vec![("content-encoding", "gzip".to_owned())]);
        let decoded = decoded_len("gzip", &reply.body);
        let compressed = u64::try_from(reply.body.len()).expect("fits u64");
        assert!(
            decoded < 1024 * 1024 && decoded / compressed.max(1) < 100,
            "the safe page is small and well within the ratio limit"
        );
    }

    #[test]
    fn a_bomb_is_served_under_its_content_encoding() {
        let request = Request {
            method: &Method::GET,
            path: PATH,
            query: "",
            cookies: "",
            authorization: "",
            body: &hyper::body::Bytes::new(),
            peer: std::net::IpAddr::from([127, 0, 0, 1]),
        };
        let reply = respond(SEED, &task::task_host(5, HOST), &request);
        assert_eq!(
            reply.headers,
            vec![("content-encoding", "gzip, gzip".to_owned())],
            "the double-gzip bomb names both codings"
        );
    }

    #[test]
    fn the_source_file_stays_under_256_kib() {
        // The bomb bytes are generated, never committed: the source is small.
        assert!(include_str!("bombs.rs").len() < 256 * 1024);
    }
}
