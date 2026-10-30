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

/// The path on the family's own host, `bombs.test`, of the zstd frame whose
/// window is past RFC 9659's 8 MB.
const PAST_THE_WINDOW: &str = "/past-the-window";

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
/// 3.1.1.2.3): 128 KiB, which is also the zstd bomb's window.
const ZSTD_BLOCK_MAXIMUM: u32 = 128 * 1024;

/// The zstd frame's `Window_Descriptor` (RFC 8878, section 3.1.1.1.2): a
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
    /// Whether the task starts over `https://`: a browser offers `br` and
    /// `zstd` only to a secure origin (Chromium offers `gzip, deflate` alone
    /// over `http://` to any host but localhost, and fails a body coded
    /// otherwise before reading it), so a browser reaches these bombs'
    /// ratio limit only over TLS, which the same address serves.
    secure: bool,
    /// The bytes of the body built for it, which the task declares. `serve`
    /// lists every task before it reports its address, and building the
    /// bodies took most of that start, so the size is written here and a
    /// test holds it equal to the body's length: the task's expected
    /// result, which its digest hashes, names the bytes the page really
    /// sends, and a body is built only when a bomb's page is asked for. An
    /// encoder whose output changes fails that test.
    compressed_bytes: u64,
    description: &'static str,
}

const SPECS: [Spec; (SAFE - 1) as usize] = [
    Spec {
        content_encodings: &["gzip"],
        secure: false,
        compressed_bytes: 10_208,
        description: "Read the start page as one Record with `id` and `word`. Its body is a \
            gzip decompression bomb: a few kilobytes that decode to far more, past the engine's \
            decompression-ratio limit. A correct tool refuses it at the limit rather than \
            reading it to the end.",
    },
    Spec {
        content_encodings: &["deflate"],
        secure: false,
        compressed_bytes: 10_190,
        description: "Read the start page as one Record with `id` and `word`. Its body is a raw \
            deflate decompression bomb, labelled `Content-Encoding: deflate`, that decodes to \
            far more than it sends, past the engine's decompression-ratio limit. A correct tool \
            refuses it at the limit.",
    },
    Spec {
        content_encodings: &["br"],
        secure: true,
        compressed_bytes: 26,
        description: "Read the start page as one Record with `id` and `word`. Its body is a \
            Brotli decompression bomb that decodes to far more than it sends, past the engine's \
            decompression-ratio limit. A correct tool refuses it at the limit.",
    },
    Spec {
        content_encodings: &["zstd"],
        secure: true,
        compressed_bytes: 330,
        description: "Read the start page as one Record with `id` and `word`. Its body is a \
            zstd decompression bomb that decodes to far more than it sends, past the engine's \
            decompression-ratio limit. A correct tool refuses it at the limit.",
    },
    Spec {
        content_encodings: &["gzip", "gzip"],
        secure: false,
        compressed_bytes: 105,
        description: "Read the start page as one Record with `id` and `word`. Its body is gzip \
            applied twice, labelled `Content-Encoding: gzip, gzip`, and decodes through both \
            codings to far more than it sends, past the engine's decompression-ratio limit. A \
            correct tool refuses it at the limit.",
    },
];

/// Each bomb's body, in [`SPECS`]' order, built once at the first request
/// for its own page: the bytes are generated here, never committed, so the
/// source file stays small, and a request for one bomb builds no other.
static BOMBS: [OnceLock<Vec<u8>>; SPECS.len()] = [const { OnceLock::new() }; SPECS.len()];

/// The body of the bomb at `index` in [`SPECS`].
fn bomb_body(index: usize) -> Option<&'static [u8]> {
    let spec = SPECS.get(index)?;
    let body = BOMBS.get(index)?;
    Some(body.get_or_init(|| body_for(spec.content_encodings)))
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
        "zstd" => zstd_rle_filler(ZstdWindow::Block),
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
    // Quality 5, window 2^22: one filler byte compresses as far at quality 5
    // as at the encoder's best, 11 (26 bytes against 27), and an
    // unoptimized build takes 0.2 s to do it, where quality 11 took 2.7 s.
    let mut encoder = brotli::CompressorWriter::new(Vec::new(), 4096, 5, 22);
    write_filler(&mut encoder, EXPANDED_BYTES);
    encoder.into_inner()
}

/// The window a zstd frame asks its decoder for.
#[derive(Clone, Copy)]
enum ZstdWindow {
    /// One block's 128 KiB, named by its `Window_Descriptor`: within RFC
    /// 9659's 8 MB, which Chromium and the engine enforce.
    Block,
    /// The frame's whole content, 10 MiB, as a single-segment frame asks:
    /// past RFC 9659's 8 MB, so a decoder that enforces it refuses the frame
    /// before a byte decodes.
    Content,
}

/// A zstd frame (RFC 8878) that regenerates `EXPANDED_BYTES` of [`FILLER`]
/// from RLE blocks, each a 3-byte header and the one byte it repeats, so the
/// whole frame is a few hundred bytes, asking for `window`. No zstd encoder
/// is a dependency of the Proving ground, so the frame is written by hand, as
/// the `content-coding` family's zstd pages are.
fn zstd_rle_filler(window: ZstdWindow) -> Vec<u8> {
    let expanded = u32::try_from(EXPANDED_BYTES).expect("invariant: the expanded size fits u32");
    let mut frame = Vec::new();
    // Magic number (RFC 8878, section 3.1.1).
    frame.extend_from_slice(&0xFD2F_B528_u32.to_le_bytes());
    // Frame_Header_Descriptor: Frame_Content_Size of 4 bytes (flag 2, bits
    // 7-6 = 10); no checksum, no dictionary.
    match window {
        // Not single-segment, so a Window_Descriptor follows: a window of
        // 128 KiB (exponent 7 over 2^10, mantissa 0).
        ZstdWindow::Block => frame.extend_from_slice(&[0b1000_0000, ZSTD_WINDOW_DESCRIPTOR]),
        // Single_Segment (bit 5), so the window is the content.
        ZstdWindow::Content => frame.push(0b1010_0000),
    }
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
    if request.method != Method::GET {
        return Reply::not_found();
    }
    let Some(ordinal) = ordinal_of(host) else {
        return if request.path == PAST_THE_WINDOW {
            encoded(&["zstd"], zstd_rle_filler(ZstdWindow::Content))
        } else {
            Reply::not_found()
        };
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
            let spec = usize::try_from(ordinal - 1)
                .ok()
                .and_then(|index| SPECS.get(index));
            let origin = if spec.is_some_and(|spec| spec.secure) {
                task::secure_origin(&host, port)
            } else {
                task::origin(&host, port)
            };
            let start_url = format!("{origin}{PATH}");
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
    use rstest::rstest;

    use super::*;

    const SEED: u64 = 312;

    /// A `GET` of `path`, with no query, cookie or body.
    fn get(path: &str) -> Request<'_> {
        static EMPTY: hyper::body::Bytes = hyper::body::Bytes::new();
        Request {
            method: &Method::GET,
            path,
            query: "",
            cookies: "",
            authorization: "",
            body: &EMPTY,
            peer: std::net::IpAddr::from([127, 0, 0, 1]),
        }
    }

    /// The task `bombs/<ordinal>` as `serve` lists it on port 8080.
    fn task_of(ordinal: u32) -> Task {
        tasks(SEED, Some(8080))
            .into_iter()
            .find(|task| task.entry.id == format!("{FAMILY}/{ordinal}"))
            .expect("the family lists the task")
    }

    /// The `Content-Encoding` a reply names, empty when it names none.
    fn content_encoding(reply: &Reply) -> &str {
        reply
            .headers
            .iter()
            .find(|(name, _)| *name == "content-encoding")
            .map_or("", |(_, value)| value.as_str())
    }

    /// What a zstd frame of raw and RLE blocks (RFC 8878) asks of its
    /// decoder and gives it.
    struct ZstdFrame {
        single_segment: bool,
        /// The window it needs: its `Window_Descriptor`'s, or its whole
        /// content when it is single-segment.
        window: u64,
        /// The bytes its blocks regenerate.
        regenerated: u64,
    }

    /// Reads `frame`, a zstd frame with no checksum or dictionary whose
    /// blocks are raw or RLE (no zstd decoder is a dependency of the
    /// Proving ground): a compressed block, which no frame here holds,
    /// fails.
    fn zstd_frame(frame: &[u8]) -> ZstdFrame {
        assert_eq!(&frame[..4], &0xFD2F_B528_u32.to_le_bytes(), "zstd's magic");
        let descriptor = frame[4];
        let single_segment = descriptor & 0b0010_0000 != 0;
        assert_eq!(descriptor & 0b0000_0111, 0, "no checksum and no dictionary");
        let mut at = 5;
        let window_descriptor = if single_segment {
            None
        } else {
            at += 1;
            Some(frame[5])
        };
        let size_bytes = match descriptor >> 6 {
            0 => usize::from(single_segment),
            1 => 2,
            2 => 4,
            _ => 8,
        };
        let mut size = [0_u8; 8];
        size[..size_bytes].copy_from_slice(&frame[at..at + size_bytes]);
        let content = u64::from_le_bytes(size) + if size_bytes == 2 { 256 } else { 0 };
        at += size_bytes;
        let window = window_descriptor.map_or(content, |byte| {
            let base = 1_u64 << (10 + (byte >> 3));
            base + base / 8 * u64::from(byte & 0b111)
        });
        let mut regenerated = 0_u64;
        loop {
            let header = u32::from_le_bytes([frame[at], frame[at + 1], frame[at + 2], 0]);
            let block = u64::from(header >> 3);
            at += 3 + match (header >> 1) & 0b11 {
                0 => usize::try_from(block).expect("a block's size fits usize"),
                1 => 1,
                other => panic!("a block of type {other}, neither raw nor RLE"),
            };
            regenerated += block;
            if header & 1 == 1 {
                break;
            }
        }
        assert_eq!(at, frame.len(), "the blocks fill the frame exactly");
        assert_eq!(
            regenerated, content,
            "the blocks regenerate its content size"
        );
        ZstdFrame {
            single_segment,
            window,
            regenerated,
        }
    }

    /// The length `body` decodes to through `content_encoding`'s codings,
    /// the last applied undone first.
    fn decoded_len(content_encoding: &str, body: &[u8]) -> u64 {
        let mut bytes = body.to_vec();
        for coding in content_encoding.rsplit(", ") {
            if coding == "zstd" {
                // Only ever the innermost coding here.
                return zstd_frame(&bytes).regenerated;
            }
            let mut decoded = Vec::new();
            match coding {
                "gzip" => GzDecoder::new(&bytes[..]).read_to_end(&mut decoded),
                "deflate" => DeflateDecoder::new(&bytes[..]).read_to_end(&mut decoded),
                "br" => brotli::Decompressor::new(&bytes[..], 4096).read_to_end(&mut decoded),
                other => panic!("no in-process decoder for {other}"),
            }
            .unwrap_or_else(|error| panic!("the {coding} body decodes: {error}"));
            bytes = decoded;
        }
        u64::try_from(bytes.len()).expect("a decoded length fits u64")
    }

    /// Each bomb's page is served under its codings, small, and decodes
    /// through them to the expanded size, far past the 100:1 the engine
    /// allows; its task expects only the refusal at the ratio limit and
    /// declares the codings and the bytes the page really sends.
    #[rstest]
    #[case::gzip(1, "gzip")]
    #[case::deflate(2, "deflate")]
    #[case::brotli(3, "br")]
    #[case::zstd(4, "zstd")]
    #[case::double_gzip(5, "gzip, gzip")]
    fn a_bomb_is_served_small_under_its_codings_and_decodes_past_the_ratio_limit(
        #[case] ordinal: u32,
        #[case] codings: &str,
    ) {
        let reply = respond(SEED, &task::task_host(ordinal, HOST), &get(PATH));
        let task = task_of(ordinal);

        let compressed = u64::try_from(reply.body.len()).expect("fits u64");
        assert_eq!(
            (reply.status, content_encoding(&reply)),
            (StatusCode::OK, codings)
        );
        assert_eq!(decoded_len(codings, &reply.body), EXPANDED_BYTES);
        assert!(
            EXPANDED_BYTES / compressed > 100 && compressed < 256 * 1024,
            "a small body far past the ratio limit ({compressed} bytes)"
        );
        let bomb = task
            .expected
            .bomb
            .as_ref()
            .expect("the task declares its bomb");
        assert_eq!(
            (
                bomb.content_encodings.join(", "),
                bomb.compressed_bytes,
                bomb.expanded_bytes
            ),
            (codings.to_owned(), compressed, EXPANDED_BYTES),
            "the task declares the bytes its page sends"
        );
        let error = task.expected.error.as_ref().expect("an expected error");
        assert!(task.expected.refusal);
        assert_eq!(
            (error.code, error.setting),
            (LIMIT_CODE, Some(RATIO_SETTING))
        );
    }

    /// A browser offers `br` and `zstd` only to a secure origin, so the
    /// Brotli and zstd bombs start over `https://`, and the others over
    /// `http://`.
    #[rstest]
    #[case::gzip(1, "http://1.bombs.test:8080/")]
    #[case::deflate(2, "http://2.bombs.test:8080/")]
    #[case::brotli(3, "https://3.bombs.test:8080/")]
    #[case::zstd(4, "https://4.bombs.test:8080/")]
    #[case::double_gzip(5, "http://5.bombs.test:8080/")]
    #[case::safe(6, "http://6.bombs.test:8080/")]
    fn a_task_starts_where_a_browser_offers_its_coding(
        #[case] ordinal: u32,
        #[case] start_url: &str,
    ) {
        assert_eq!(task_of(ordinal).entry.start_url, start_url);
    }

    /// The zstd bomb's frame names a 128 KiB window, within RFC 9659's
    /// 8 MB, so a decoder that enforces that reaches the ratio limit.
    #[test]
    fn the_zstd_bombs_frame_asks_for_a_window_within_eight_megabytes() {
        let reply = respond(SEED, &task::task_host(4, HOST), &get(PATH));

        let frame = zstd_frame(&reply.body);

        assert_eq!(
            (frame.single_segment, frame.window),
            (false, u64::from(ZSTD_BLOCK_MAXIMUM))
        );
    }

    /// `bombs.test/past-the-window` serves the same blocks in a
    /// single-segment frame, whose window is its whole 10 MiB content, past
    /// RFC 9659's 8 MB: a decoder that enforces it refuses the frame before
    /// a byte decodes.
    #[test]
    fn the_page_past_the_window_asks_for_a_window_past_eight_megabytes() {
        let reply = respond(SEED, HOST, &get(PAST_THE_WINDOW));

        assert_eq!(
            (reply.status, content_encoding(&reply)),
            (StatusCode::OK, "zstd")
        );
        let frame = zstd_frame(&reply.body);
        assert_eq!(
            (frame.single_segment, frame.window, frame.regenerated),
            (true, EXPANDED_BYTES, EXPANDED_BYTES)
        );
        assert!(frame.window > 8_000_000, "past RFC 9659's 8 MB");
    }

    /// The family's own host serves no bomb at a task's path.
    #[test]
    fn the_bombs_host_serves_nothing_at_the_tasks_path() {
        assert_eq!(
            respond(SEED, HOST, &get(PATH)).status,
            StatusCode::NOT_FOUND
        );
    }

    /// `serve` lists every task before it reports its address, so listing
    /// them must not build the bombs: in an unoptimized build that took two
    /// seconds of every start, and a loaded host stretched it past the
    /// start deadline (ADR 0064). Each test runs in its own process
    /// (nextest), so no other test has built them first. It is the one
    /// test that reads a private item: whether the bodies were built is
    /// internal by nature, and no reply shows it.
    #[test]
    fn listing_the_bomb_tasks_builds_no_bomb() {
        let tasks = tasks(SEED, None);

        assert_eq!(tasks.len(), TASKS as usize);
        assert!(
            BOMBS.iter().all(|body| body.get().is_none()),
            "the bombs were built to list them"
        );
    }

    #[test]
    fn the_safe_page_is_served_compressed_within_the_ratio_and_kept() {
        let reply = respond(SEED, &task::task_host(SAFE, HOST), &get(PATH));

        let decoded = decoded_len("gzip", &reply.body);
        let compressed = u64::try_from(reply.body.len()).expect("fits u64");
        assert_eq!(
            (reply.status, content_encoding(&reply)),
            (StatusCode::OK, "gzip")
        );
        assert!(
            decoded < 1024 * 1024 && decoded / compressed.max(1) < 100,
            "the safe page is small and well within the ratio limit"
        );
    }

    #[test]
    fn the_source_file_stays_under_256_kib() {
        // The bomb bytes are generated, never committed: the source is small.
        assert!(include_str!("bombs.rs").len() < 256 * 1024);
    }
}
