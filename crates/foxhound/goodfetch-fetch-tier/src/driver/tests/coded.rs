//! A coded body decoded by the engine's one decoder as its bytes arrive
//! (spec #2164, issue 2187): a zstd frame asking for a window past 8 MiB
//! fails whatever `network.max_body_size` allows, as the engine's other
//! readers and Chromium fail it; a cut stream is `response_truncated`
//! (ADR 0118); and a stream that cannot decode fails as soon as its bytes
//! show it, its body never read to its end. A scripted transport answers
//! on Tokio's paused clock.
#![expect(
    clippy::disallowed_methods,
    reason = "tests run on the test Clock and Tokio's paused clock (ADR 0059)"
)]

use std::collections::VecDeque;
use std::io::Write as _;
use std::time::{Duration, UNIX_EPOCH};

use bytes::Bytes;
use flate2::Compression;
use flate2::write::GzEncoder;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::deadline::Deadline;
use goodfetch_engine::response_limits::Limits;
use goodfetch_protocol::settings::{SettingsLayer, TIMEOUT_FIRST_BYTE, TIMEOUT_NAVIGATION};
use rstest::rstest;
use serde_json::Value;
use url::Url;

use crate::driver::{Body, FetchBounds, Hop, Reuse, Sending, Sent, Transport, fetch};

/// A transport whose one answer is a `200` under `encoding`, its body
/// `chunks`, which then ends, or never does when it is `endless`.
struct Coded {
    encoding: &'static str,
    chunks: Vec<Bytes>,
    endless: bool,
}

/// The body [`Coded`] answers.
struct Chunks {
    chunks: VecDeque<Bytes>,
    endless: bool,
}

impl Body for Chunks {
    fn chunk(&mut self) -> Sending<'_, Option<Bytes>> {
        Box::pin(async move {
            match self.chunks.pop_front() {
                Some(chunk) => Ok(Some(chunk)),
                None if self.endless => std::future::pending().await,
                None => Ok(None),
            }
        })
    }
}

impl Transport for Coded {
    fn send<'a>(&'a self, _hop: &'a Hop, _reuse: Reuse) -> Sending<'a, Sent> {
        Box::pin(async move {
            Ok(Sent {
                status: 200,
                headers: vec![
                    ("content-type".to_owned(), "text/html".to_owned()),
                    ("content-encoding".to_owned(), self.encoding.to_owned()),
                ],
                body: Box::new(Chunks {
                    chunks: self.chunks.iter().cloned().collect(),
                    endless: self.endless,
                }),
                reused: false,
                decoded: false,
                certificate_exception: None,
            })
        })
    }
}

/// What a fetch of `transport`'s answer gives under a 1 GiB
/// `network.max_body_size` and a ratio that lets any body through: the
/// body's length, or its Error code and network error.
async fn fetched(transport: &Coded) -> Result<usize, (&'static str, Option<String>)> {
    let bounds = FetchBounds {
        first_byte: Deadline::given(&TIMEOUT_FIRST_BYTE, 30_000, SettingsLayer::Run),
        navigation: Deadline::given(&TIMEOUT_NAVIGATION, 30_000, SettingsLayer::Run),
        limits: Limits {
            max_body_size: 1 << 30,
            max_decompression_ratio: 1_000_000,
            ..Limits::default()
        },
    };
    let clock = Clock::paused(UNIX_EPOCH + Duration::from_hours(491_001));
    let hop = Hop::get(Url::parse("http://site.test/page").unwrap());
    fetch(transport, &hop, &bounds, &clock)
        .await
        .map(|fetched| fetched.body.len())
        .map_err(|failure| {
            let error = failure.error();
            let network = error
                .details()
                .and_then(|details| details.get("error"))
                .and_then(Value::as_str)
                .map(str::to_owned);
            (error.code(), network)
        })
}

/// A zstd frame of `size` bytes asking for a window of `2^window_log`.
fn zstd_windowed(window_log: u32, size: u32) -> Vec<u8> {
    let text: Vec<u8> = (0..size)
        .map(|index| u8::try_from(index.wrapping_mul(2_654_435_761) >> 24).unwrap())
        .collect();
    let mut encoder = zstd::stream::Encoder::new(Vec::new(), 1).unwrap();
    encoder.window_log(window_log).unwrap();
    encoder.include_contentsize(false).unwrap();
    encoder.write_all(&text).unwrap();
    encoder.finish().unwrap()
}

/// RFC 9659's window, 8 MiB, decodes; a larger one does not, however much
/// `network.max_body_size` allows.
#[rstest]
#[case::a_window_of_8_mib(23, Ok(9 * 1024 * 1024))]
#[case::a_window_of_16_mib(
    24,
    Err(("navigation_failed", Some("net::ERR_CONTENT_DECODING_FAILED".to_owned())))
)]
#[tokio::test(start_paused = true)]
async fn a_zstd_frame_may_ask_for_a_window_of_eight_mebibytes_at_most(
    #[case] window_log: u32,
    #[case] expected: Result<usize, (&'static str, Option<String>)>,
) {
    let frame = zstd_windowed(window_log, 9 * 1024 * 1024);
    let transport = Coded {
        encoding: "zstd",
        chunks: frame
            .chunks(64 * 1024)
            .map(Bytes::copy_from_slice)
            .collect(),
        endless: false,
    };

    assert_eq!(fetched(&transport).await, expected);
}

/// A gzip stream that stops before its end, its body ended, was cut.
#[tokio::test(start_paused = true)]
async fn a_cut_gzip_stream_is_response_truncated() {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&[b'#'; 4096]).unwrap();
    let mut gzipped = encoder.finish().unwrap();
    gzipped.truncate(gzipped.len() / 2);
    let transport = Coded {
        encoding: "gzip",
        chunks: vec![Bytes::from(gzipped)],
        endless: false,
    };

    assert_eq!(fetched(&transport).await, Err(("response_truncated", None)));
}

/// A body labelled gzip that is no gzip stream fails on its first bytes,
/// though it never ends.
#[tokio::test(start_paused = true)]
async fn a_body_that_cannot_decode_fails_before_it_ends() {
    let transport = Coded {
        encoding: "gzip",
        chunks: vec![Bytes::from_static(b"<!doctype html><p>plain")],
        endless: true,
    };

    assert_eq!(
        fetched(&transport).await,
        Err((
            "navigation_failed",
            Some("net::ERR_CONTENT_DECODING_FAILED".to_owned())
        ))
    );
}
