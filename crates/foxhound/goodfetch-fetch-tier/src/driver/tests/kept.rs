//! A body read for its first bytes alone, as robots.txt and the AI-use
//! signal files are (RFC 9309's 500 KiB; ADR 0067): the read stops once
//! they are decoded, so a gzip bomb or an endless body gives its start and
//! nothing more. A scripted transport answers on Tokio's paused clock, its
//! body never ending past the bytes it was given unless it ends.
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
use url::Url;

use crate::driver::{Body, FetchBounds, Hop, Reuse, Sending, Sent, Transport, fetch_start};

/// What robots.txt keeps.
const KEEP: usize = 500 * 1024;

/// A transport whose one answer is a `200` under `encoding`, its body
/// `chunks` and then nothing more ever.
struct Endless {
    encoding: Option<&'static str>,
    chunks: Vec<Bytes>,
}

/// The body [`Endless`] answers.
struct Chunks(VecDeque<Bytes>);

impl Body for Chunks {
    fn chunk(&mut self) -> Sending<'_, Option<Bytes>> {
        Box::pin(async move {
            match self.0.pop_front() {
                Some(chunk) => Ok(Some(chunk)),
                None => std::future::pending().await,
            }
        })
    }
}

impl Transport for Endless {
    fn send<'a>(&'a self, _hop: &'a Hop, _reuse: Reuse) -> Sending<'a, Sent> {
        Box::pin(async move {
            Ok(Sent {
                status: 200,
                headers: self
                    .encoding
                    .map(|coding| ("content-encoding".to_owned(), coding.to_owned()))
                    .into_iter()
                    .collect(),
                body: Box::new(Chunks(self.chunks.iter().cloned().collect())),
                reused: false,
                decoded: false,
                certificate_exception: None,
            })
        })
    }
}

/// The length of what a fetch of `transport`'s answer keeps of its first
/// [`KEEP`] bytes, `timeout.navigation` at 30 s, or its Error code.
async fn kept(transport: &Endless) -> Result<usize, &'static str> {
    let bounds = FetchBounds {
        first_byte: Deadline::given(&TIMEOUT_FIRST_BYTE, 30_000, SettingsLayer::Run),
        navigation: Deadline::given(&TIMEOUT_NAVIGATION, 30_000, SettingsLayer::Run),
        limits: Limits::default(),
    };
    let clock = Clock::paused(UNIX_EPOCH + Duration::from_hours(491_001));
    let hop = Hop::get(Url::parse("http://site.test/robots.txt").unwrap());
    fetch_start(transport, &hop, &bounds, &clock, KEEP)
        .await
        .map(|fetched| fetched.body.len())
        .map_err(|failure| failure.error().code())
}

/// A gzip robots.txt of about 100 KB that decodes to 100 MiB gives its
/// first 500 KiB, its stream read no further, though it never ends.
#[tokio::test(start_paused = true)]
async fn a_gzip_bomb_read_for_its_first_bytes_gives_them() {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::best());
    let mebibyte = vec![b'#'; 1024 * 1024];
    for _ in 0..100 {
        encoder.write_all(&mebibyte).unwrap();
    }
    let gzipped = encoder.finish().unwrap();
    let transport = Endless {
        encoding: Some("gzip"),
        chunks: vec![Bytes::from(gzipped)],
    };

    assert_eq!(kept(&transport).await, Ok(KEEP));
}

/// A plain body that never ends gives its first 500 KiB once they are in.
#[tokio::test(start_paused = true)]
async fn an_endless_body_read_for_its_first_bytes_stops_there() {
    let transport = Endless {
        encoding: None,
        chunks: (0..10)
            .map(|_| Bytes::from(vec![b'#'; 64 * 1024]))
            .collect(),
    };

    assert_eq!(kept(&transport).await, Ok(KEEP));
}
