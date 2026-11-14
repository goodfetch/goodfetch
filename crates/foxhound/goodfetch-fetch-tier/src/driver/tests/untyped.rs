//! An untyped response under a crawl request's `crawl.content_types` (ADR
//! 0119; ticket #950): its type is Chromium's sniffer's over at most the
//! first 1,024 bytes of its decoded body, so only that much is read before
//! a type the Label does not take drops the rest unread. A scripted
//! transport answers on Tokio's paused clock, its body never ending past
//! the bytes it was given.
//!
//! Covers: story:6-264
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

use crate::content_types::Accepts;
use crate::driver::{
    Body, FetchBounds, Fetched, Hop, OwnDocument, Reuse, Sending, Sent, Transport, fetch_accepting,
};

/// A PDF's opening, which the sniffer names `application/pdf`, padded past
/// the sniffer's 1,024 bytes.
fn pdf() -> Vec<u8> {
    let mut bytes = b"%PDF-1.7\n".to_vec();
    bytes.resize(1_500, b'x');
    bytes
}

/// A transport whose one answer is a `200` naming no type, or
/// `content_type` when given, its body `chunks` and then, unless `ends`,
/// nothing more ever.
struct Untyped {
    encoding: Option<&'static str>,
    content_type: Option<&'static str>,
    chunks: Vec<Bytes>,
    ends: bool,
}

/// The body [`Untyped`] answers.
struct Chunks {
    chunks: VecDeque<Bytes>,
    ends: bool,
}

impl Body for Chunks {
    fn chunk(&mut self) -> Sending<'_, Option<Bytes>> {
        Box::pin(async move {
            match self.chunks.pop_front() {
                Some(chunk) => Ok(Some(chunk)),
                None if self.ends => Ok(None),
                None => std::future::pending().await,
            }
        })
    }
}

impl Transport for Untyped {
    fn send<'a>(&'a self, _hop: &'a Hop, _reuse: Reuse) -> Sending<'a, Sent> {
        Box::pin(async move {
            Ok(Sent {
                status: 200,
                headers: self
                    .encoding
                    .map(|coding| ("content-encoding".to_owned(), coding.to_owned()))
                    .into_iter()
                    .chain(
                        self.content_type
                            .map(|value| ("content-type".to_owned(), value.to_owned())),
                    )
                    .collect(),
                body: Box::new(Chunks {
                    chunks: self.chunks.iter().cloned().collect(),
                    ends: self.ends,
                }),
                reused: false,
                decoded: false,
                certificate_exception: None,
            })
        })
    }
}

/// A fetch of `transport`'s answer under the built-in
/// `crawl.content_types`, `timeout.navigation` at 30 s.
async fn fetched(transport: &Untyped) -> Result<Fetched, goodfetch_engine::CallError> {
    let bounds = FetchBounds {
        first_byte: Deadline::given(&TIMEOUT_FIRST_BYTE, 30_000, SettingsLayer::Run),
        navigation: Deadline::given(&TIMEOUT_NAVIGATION, 30_000, SettingsLayer::Run),
        limits: Limits::default(),
    };
    let clock = Clock::paused(UNIX_EPOCH + Duration::from_hours(491_001));
    let hop = Hop::get(Url::parse("http://files.test/download").unwrap());
    let own = OwnDocument {
        accepts: &Accepts::default(),
        encoding: None,
    };
    fetch_accepting(transport, &hop, &bounds, &clock, Some(own))
        .await
        .map_err(crate::driver::FetchFailure::into_error)
}

/// The judgment of a fetch: its type when refused, and its body's length.
fn judged(fetched: &Result<Fetched, goodfetch_engine::CallError>) -> Option<(Option<&str>, usize)> {
    fetched
        .as_ref()
        .ok()
        .map(|fetched| (fetched.unaccepted.as_deref(), fetched.body.len()))
}

#[tokio::test(start_paused = true)]
async fn an_untyped_body_the_label_refuses_is_left_unread_past_its_first_kilobyte() {
    let transport = Untyped {
        encoding: None,
        content_type: None,
        chunks: vec![Bytes::from(pdf())],
        ends: false,
    };

    let fetched = fetched(&transport).await;

    assert_eq!(judged(&fetched), Some((Some("application/pdf"), 0)));
}

#[tokio::test(start_paused = true)]
async fn an_untyped_body_is_judged_on_its_decoded_bytes() {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&pdf()).unwrap();
    let gzipped = encoder.finish().unwrap();
    let (first, rest) = gzipped.split_at(gzipped.len() / 2);
    let transport = Untyped {
        encoding: Some("gzip"),
        content_type: None,
        chunks: vec![Bytes::copy_from_slice(first), Bytes::copy_from_slice(rest)],
        ends: false,
    };

    let fetched = fetched(&transport).await;

    assert_eq!(judged(&fetched), Some((Some("application/pdf"), 0)));
}

#[tokio::test(start_paused = true)]
async fn an_untyped_body_the_label_takes_is_read_whole() {
    let mut html = b"<!DOCTYPE html><p>".to_vec();
    html.resize(3_000, b'a');
    let transport = Untyped {
        encoding: None,
        content_type: None,
        chunks: html.chunks(700).map(Bytes::copy_from_slice).collect(),
        ends: true,
    };

    let fetched = fetched(&transport).await;

    assert_eq!(judged(&fetched), Some((None, 3_000)));
}

#[tokio::test(start_paused = true)]
async fn an_untyped_body_shorter_than_a_kilobyte_is_judged_whole_at_its_end() {
    let html = b"<!DOCTYPE html><p>short</p>".to_vec();
    let transport = Untyped {
        encoding: None,
        content_type: None,
        chunks: vec![Bytes::from(html.clone())],
        ends: true,
    };

    let fetched = fetched(&transport).await;

    assert_eq!(judged(&fetched), Some((None, html.len())));
}

/// A `Content-Type` that names no type counts as none (ADR 0119; the WHATWG
/// MIME Sniffing standard, section 7): its head refuses nothing, and the
/// type the sniffer gives its body decides, a PDF refused and an HTML page
/// read whole.
#[rstest::rstest]
#[case::any("*/*")]
#[case::unknown_unknown("unknown/unknown")]
#[case::application_unknown("application/unknown")]
#[case::empty("")]
#[case::no_slash("html")]
#[tokio::test(start_paused = true)]
async fn a_content_type_naming_no_type_is_sniffed(#[case] content_type: &'static str) {
    let mut html = b"<!DOCTYPE html><p>".to_vec();
    html.resize(1_500, b'a');
    let page = Untyped {
        encoding: None,
        content_type: Some(content_type),
        chunks: vec![Bytes::from(html)],
        ends: true,
    };
    let document = Untyped {
        encoding: None,
        content_type: Some(content_type),
        chunks: vec![Bytes::from(pdf())],
        ends: false,
    };

    let (page, document) = (fetched(&page).await, fetched(&document).await);

    assert_eq!(
        (judged(&page), judged(&document)),
        (Some((None, 1_500)), Some((Some("application/pdf"), 0)))
    );
}
