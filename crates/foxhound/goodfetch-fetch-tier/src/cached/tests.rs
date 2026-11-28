//! The cached transport through the driver's fetch, over a scripted site
//! that records each request it receives, a real Response cache in the
//! test's folder and the test Clock on Tokio's paused clock: a fresh
//! response is served with no request, a stale one is revalidated and a
//! 304 serves its stored body, a `Vary: Cookie` response is keyed on the
//! cookie, a pool Session's logged-in page is its own until it retires,
//! and a cut body is never stored.
#![expect(
    clippy::disallowed_methods,
    reason = "tests run on the test Clock and Tokio's paused clock (ADR 0059)"
)]
#![expect(
    clippy::disallowed_types,
    reason = "the scripted site's answers and requests are each read or written in one step, never across an `.await`"
)]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use goodfetch_counted_work::CountedWork;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::deadline::Deadline;
use goodfetch_engine::response_cache::{Caching, ResponseCache, ResponseSource};
use goodfetch_engine::response_limits::Limits;
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_protocol::page::SessionId;
use goodfetch_protocol::response_cache::CacheMode;
use goodfetch_protocol::run::RunId;
use goodfetch_protocol::settings::{SettingsLayer, TIMEOUT_FIRST_BYTE, TIMEOUT_NAVIGATION};
use goodfetch_test_support::TestEnv;
use url::Url;

use super::over;
use crate::driver::{
    Body, FetchBounds, Fetched, Hop, Reuse, Sending, Sent, Transport, TransportFailure, fetch,
};

/// One answer of the scripted site: its status, its headers and its body,
/// cut after its bytes when `cut`.
struct Answer {
    status: u16,
    headers: Vec<(&'static str, &'static str)>,
    body: &'static [u8],
    cut: bool,
}

/// A site answering its scripted answers in order, one per request, and
/// recording each request's headers.
#[derive(Default)]
struct Site {
    answers: Mutex<VecDeque<Answer>>,
    received: Mutex<Vec<Vec<(String, String)>>>,
}

impl Site {
    fn answering(answers: Vec<Answer>) -> Arc<Self> {
        Arc::new(Self {
            answers: Mutex::new(answers.into()),
            received: Mutex::default(),
        })
    }

    fn received(&self) -> Vec<Vec<(String, String)>> {
        self.received.lock().unwrap().clone()
    }
}

/// A body of one chunk, then its end or a cut.
struct Chunks(VecDeque<Result<Option<Bytes>, TransportFailure>>);

impl Body for Chunks {
    fn chunk(&mut self) -> Sending<'_, Option<Bytes>> {
        let next = self.0.pop_front().unwrap_or(Ok(None));
        Box::pin(async move { next })
    }
}

impl Transport for Site {
    fn send<'a>(&'a self, hop: &'a Hop, _reuse: Reuse) -> Sending<'a, Sent> {
        self.received.lock().unwrap().push(hop.headers.clone());
        let answer = self
            .answers
            .lock()
            .unwrap()
            .pop_front()
            .expect("the site was sent no more request than it answers");
        Box::pin(async move {
            let end = if answer.cut {
                Err(TransportFailure::ConnectionClosed)
            } else {
                Ok(None)
            };
            Ok(Sent {
                status: answer.status,
                headers: answer
                    .headers
                    .iter()
                    .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                    .collect(),
                body: Box::new(Chunks(
                    [Ok(Some(Bytes::from_static(answer.body))), end].into(),
                )),
                reused: false,
                decoded: false,
                certificate_exception: None,
                origin: None,
            })
        })
    }
}

fn ok(headers: Vec<(&'static str, &'static str)>, body: &'static [u8]) -> Answer {
    Answer {
        status: 200,
        headers,
        body,
        cut: false,
    }
}

/// A page fresh for a minute, with validators to revalidate it by.
fn minute(body: &'static [u8]) -> Answer {
    ok(
        vec![
            ("content-type", "text/html"),
            ("cache-control", "max-age=60"),
            ("etag", "\"v1\""),
            ("last-modified", "Mon, 05 Jan 2026 08:00:00 GMT"),
        ],
        body,
    )
}

/// The test's clock, its cache under `revalidate` and the cached
/// transport over `site`.
struct Fetching {
    clock: Clock,
    caching: Arc<Caching>,
    site: Arc<dyn Transport>,
    transport: Arc<dyn Transport>,
    _env: TestEnv,
}

impl Fetching {
    fn over(site: &Arc<Site>) -> Self {
        let env = TestEnv::new();
        let cache = ResponseCache::of(
            env.home().join("0.1").join("response-cache"),
            None,
            Arc::new(TaskOwner::root(BlockingPolicy::Inline)),
            CountedWork::default(),
        );
        let clock = Clock::paused(
            goodfetch_protocol::point_in_time::from_rfc3339("2026-01-05T09:00:00Z").unwrap(),
        );
        let caching = Caching::of(
            ResponseSource::of(&cache, CacheMode::Revalidate),
            "ada",
            RunId::parse("run").unwrap(),
            clock,
        )
        .expect("the cache is on");
        let site = Arc::clone(site) as Arc<dyn Transport>;
        Self {
            clock,
            transport: over(&caching, Arc::clone(&site)),
            caching,
            site,
            _env: env,
        }
    }

    /// The cached transport of the Run's pool Session `session`.
    fn of_session(&self, session: &SessionId) -> Arc<dyn Transport> {
        over(&self.caching.of_session(session), Arc::clone(&self.site))
    }

    async fn fetch(&self, hop: &Hop) -> Result<Fetched, &'static str> {
        self.fetch_on(&self.transport, hop).await
    }

    async fn fetch_on(
        &self,
        transport: &Arc<dyn Transport>,
        hop: &Hop,
    ) -> Result<Fetched, &'static str> {
        let bounds = FetchBounds {
            first_byte: Deadline::given(&TIMEOUT_FIRST_BYTE, 30_000, SettingsLayer::Run),
            navigation: Deadline::given(&TIMEOUT_NAVIGATION, 30_000, SettingsLayer::Run),
            limits: Limits::default(),
        };
        fetch(transport.as_ref(), hop, &bounds, &self.clock)
            .await
            .map_err(|failure| failure.error().code())
    }
}

fn page() -> Hop {
    Hop::get(Url::parse("https://cache.test/page").unwrap())
}

fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(header, _)| header == name)
        .map(|(_, value)| value.as_str())
}

/// A fresh response is served with no request: the site receives the
/// first request alone, and the second fetch gets its body.
/// Covers: story:6-37, setting:cache.mode
#[tokio::test(start_paused = true)]
async fn a_fresh_response_is_served_with_no_request() {
    let site = Site::answering(vec![minute(b"<p>first</p>")]);
    let fetching = Fetching::over(&site);

    let first = fetching.fetch(&page()).await.unwrap();
    tokio::time::advance(Duration::from_secs(30)).await;
    let second = fetching.fetch(&page()).await.unwrap();

    assert_eq!(site.received().len(), 1);
    assert_eq!(
        (second.status, &second.body[..]),
        (200, &b"<p>first</p>"[..])
    );
    assert_eq!(first.body, second.body);
    assert_eq!(header(&second.headers, "age"), Some("30"));
}

/// A stale response is revalidated with `If-None-Match` and
/// `If-Modified-Since`, and the site's 304 serves the stored body.
/// Covers: story:6-37
#[tokio::test(start_paused = true)]
async fn a_stale_response_is_revalidated_and_a_304_serves_its_stored_body() {
    let site = Site::answering(vec![
        minute(b"<p>stored</p>"),
        Answer {
            status: 304,
            headers: vec![("etag", "\"v1\""), ("cache-control", "max-age=60")],
            body: b"",
            cut: false,
        },
    ]);
    let fetching = Fetching::over(&site);

    fetching.fetch(&page()).await.unwrap();
    tokio::time::advance(Duration::from_secs(120)).await;
    let revalidated = fetching.fetch(&page()).await.unwrap();

    let received = site.received();
    assert_eq!(received.len(), 2);
    assert_eq!(
        (
            header(&received[1], "if-none-match"),
            header(&received[1], "if-modified-since")
        ),
        (Some("\"v1\""), Some("Mon, 05 Jan 2026 08:00:00 GMT"))
    );
    assert_eq!(header(&received[0], "if-none-match"), None);
    assert_eq!(
        (revalidated.status, &revalidated.body[..]),
        (200, &b"<p>stored</p>"[..])
    );
}

/// A `Vary: Cookie` response is keyed on the cookie: a request carrying a
/// cookie is sent to the site, never answered with the response stored
/// for the request with none, which is served again to a request with none.
/// Covers: story:6-37
#[tokio::test(start_paused = true)]
async fn a_response_varying_on_cookie_is_keyed_on_it() {
    let varying = |body| {
        ok(
            vec![
                ("content-type", "text/html"),
                ("cache-control", "max-age=60"),
                ("vary", "Cookie"),
            ],
            body,
        )
    };
    let site = Site::answering(vec![varying(b"<p>anyone</p>"), varying(b"<p>ada</p>")]);
    let fetching = Fetching::over(&site);
    let with_cookie = Hop {
        headers: vec![("cookie".to_owned(), "session=ada".to_owned())],
        ..page()
    };

    fetching.fetch(&page()).await.unwrap();
    let ada = fetching.fetch(&with_cookie).await.unwrap();
    let anyone = fetching.fetch(&page()).await.unwrap();

    assert_eq!(site.received().len(), 2);
    assert_eq!(&ada.body[..], b"<p>ada</p>");
    assert_eq!(&anyone.body[..], b"<p>anyone</p>");
}

/// A page a pool Session fetched with its cookie is served to it again
/// with no request, never to another Session of the Run nor to the Run's
/// own transport, and no longer to it once it retired.
/// Covers: story:6-39
#[tokio::test(start_paused = true)]
async fn a_sessions_logged_in_page_is_its_own_until_it_retires() {
    let site = Site::answering(vec![
        minute(b"<p>ada</p>"),
        minute(b"<p>bob</p>"),
        minute(b"<p>run</p>"),
        minute(b"<p>ada again</p>"),
    ]);
    let fetching = Fetching::over(&site);
    let (first, second) = (SessionId("s1".to_owned()), SessionId("s2".to_owned()));
    let (ada, bob) = (fetching.of_session(&first), fetching.of_session(&second));
    let logged_in = |who: &str| Hop {
        headers: vec![("cookie".to_owned(), format!("session={who}"))],
        ..page()
    };

    let mut bodies = Vec::new();
    for (transport, who) in [(&ada, "ada"), (&ada, "ada"), (&bob, "bob")] {
        bodies.push(
            fetching
                .fetch_on(transport, &logged_in(who))
                .await
                .unwrap()
                .body,
        );
    }
    bodies.push(fetching.fetch(&logged_in("ada")).await.unwrap().body);
    fetching.caching.forget(&first).await;
    bodies.push(
        fetching
            .fetch_on(&ada, &logged_in("ada"))
            .await
            .unwrap()
            .body,
    );

    assert_eq!(site.received().len(), 4);
    assert_eq!(
        bodies.iter().map(|body| &body[..]).collect::<Vec<_>>(),
        [
            &b"<p>ada</p>"[..],
            b"<p>ada</p>",
            b"<p>bob</p>",
            b"<p>run</p>",
            b"<p>ada again</p>"
        ]
    );
}

/// A body cut before its end is never stored: the next fetch of the page
/// goes to the site.
/// Covers: story:6-37
#[tokio::test(start_paused = true)]
async fn a_cut_body_is_not_stored() {
    let site = Site::answering(vec![
        Answer {
            cut: true,
            ..minute(b"<p>cut")
        },
        minute(b"<p>whole</p>"),
    ]);
    let fetching = Fetching::over(&site);

    let cut = fetching.fetch(&page()).await;
    let whole = fetching.fetch(&page()).await.unwrap();

    assert!(cut.is_err());
    assert_eq!(site.received().len(), 2);
    assert_eq!(&whole.body[..], b"<p>whole</p>");
}

/// A Run whose source only sends, `cache.mode: off`, gets no cached
/// transport: its hops leave on its own transport alone.
/// Covers: story:6-38, setting:cache.mode
#[tokio::test(start_paused = true)]
async fn a_run_with_the_cache_off_gets_no_cached_transport() {
    let caching = Caching::of(
        ResponseSource::default(),
        "ada",
        RunId::parse("run").unwrap(),
        Clock::paused(std::time::UNIX_EPOCH),
    );

    assert!(caching.is_none());
}

/// The HTTP tier answers from a WARC recording without calling its
/// underlying transport, refuses a missing request with
/// `response_not_recorded` (not retryable), and fails a truncated recorded
/// response with `response_truncated` (ADR 0105).
///
const REPLAY_WARC: &str = concat!(
    "WARC/1.1\r\n",
    "WARC-Type: warcinfo\r\n",
    "WARC-Record-ID: <urn:uuid:00000000-0000-0000-0000-000000000001>\r\n",
    "WARC-Date: 2026-01-05T09:00:00Z\r\n",
    "Content-Type: application/warc-fields\r\n",
    "Content-Length: 23\r\n\r\n",
    "software: goodfetch/0.1\r\n\r\n",
    "WARC/1.1\r\n",
    "WARC-Type: response\r\n",
    "WARC-Record-ID: <urn:uuid:00000000-0000-0000-0000-000000000002>\r\n",
    "WARC-Date: 2026-01-05T09:00:01Z\r\n",
    "WARC-Target-URI: https://cache.test/page\r\n",
    "Content-Type: application/http; msgtype=response\r\n",
    "Content-Length: 55\r\n\r\n",
    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n\r\n<p>warc</p>\r\n\r\n",
    "WARC/1.1\r\n",
    "WARC-Type: response\r\n",
    "WARC-Record-ID: <urn:uuid:00000000-0000-0000-0000-000000000003>\r\n",
    "WARC-Date: 2026-01-05T09:00:02Z\r\n",
    "WARC-Target-URI: https://cache.test/cut\r\n",
    "WARC-Truncated: disconnect\r\n",
    "Content-Type: application/http; msgtype=response\r\n",
    "Content-Length: 50\r\n\r\n",
    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n\r\n<p>cut\r\n\r\n",
);

/// Covers: story:6-52, story:6-53
#[tokio::test(start_paused = true)]
async fn a_replaying_http_tier_answers_recorded_fails_missing_and_truncates_cut() {
    use goodfetch_engine::mask::Masks;
    use goodfetch_engine::response_recording::Recordings;
    use goodfetch_protocol::response_limits::DEFAULT_MAX_DECOMPRESSION_RATIO;

    let env = TestEnv::new();
    let warc_file = env.folder().join("site.warc");
    std::fs::write(&warc_file, REPLAY_WARC.as_bytes()).unwrap();
    let warc_path = warc_file.to_string_lossy().into_owned();
    let owner = TaskOwner::root(BlockingPolicy::Inline);
    let recordings = Recordings::default();
    let replayed = recordings
        .open_replay(
            &warc_path,
            Masks::new(),
            DEFAULT_MAX_DECOMPRESSION_RATIO,
            &owner,
        )
        .await
        .unwrap();
    let run_id = RunId::parse("replay-http").unwrap();
    recordings.attach(run_id.clone(), replayed);

    let site = Site::answering(vec![minute(b"<p>live</p>")]);
    let caching = Caching::of(
        ResponseSource::default().recording(recordings.get(&run_id).unwrap()),
        "ada",
        run_id.clone(),
        Clock::paused(std::time::UNIX_EPOCH),
    )
    .unwrap();
    let transport = super::over(&caching, Arc::clone(&site) as Arc<dyn Transport>);
    let bounds = FetchBounds {
        first_byte: Deadline::given(&TIMEOUT_FIRST_BYTE, 30_000, SettingsLayer::Run),
        navigation: Deadline::given(&TIMEOUT_NAVIGATION, 30_000, SettingsLayer::Run),
        limits: Limits::default(),
    };
    let clock = Clock::paused(std::time::UNIX_EPOCH);

    let hit = crate::driver::fetch(
        transport.as_ref(),
        &Hop::get("https://cache.test/page".parse().unwrap()),
        &bounds,
        &clock,
    )
    .await
    .unwrap();
    assert_eq!(&hit.body[..], b"<p>warc</p>");
    assert!(matches!(
        hit.origin,
        Some(goodfetch_protocol::foxhound::provenance::ResponseOrigin::ResponseRecording { .. })
    ));

    let missing = crate::driver::fetch(
        transport.as_ref(),
        &Hop::get("https://cache.test/page?extra=1".parse().unwrap()),
        &bounds,
        &clock,
    )
    .await
    .unwrap_err();
    assert_eq!(missing.error().code(), "response_not_recorded");
    assert!(!missing.error().retryable());
    assert_eq!(
        missing.error().details().unwrap()["nearest"],
        serde_json::json!(["https://cache.test/page"])
    );

    let truncated = crate::driver::fetch(
        transport.as_ref(),
        &Hop::get("https://cache.test/cut".parse().unwrap()),
        &bounds,
        &clock,
    )
    .await
    .unwrap_err();
    assert_eq!(truncated.error().code(), "response_truncated");

    // The underlying live transport saw zero requests.
    assert_eq!(site.received(), [] as [Vec<(String, String)>; 0]);

    let report = recordings.take_report_of(&run_id).unwrap();
    assert_eq!(report.mode, "replay");
    assert_eq!(report.responses_used, 2);
    assert_eq!(report.misses, vec!["GET https://cache.test/page?extra=1"]);
}

#[derive(Debug, Default)]
struct CapturedList(Mutex<Vec<(String, bool, Bytes)>>);

impl goodfetch_engine::response_cache::Capture for CapturedList {
    fn capture(
        &self,
        request: goodfetch_engine::response_cache::CacheRequest,
        _head: goodfetch_engine::response_cache::ResponseHead,
        body: Bytes,
        truncated: bool,
        _now: std::time::SystemTime,
    ) -> goodfetch_engine::response_cache::Capturing<'_> {
        let mut guard = self.0.lock().unwrap();
        guard.push((request.url, truncated, body));
        let seq = guard.len() as u64;
        Box::pin(async move { seq })
    }

    fn mark_truncated(&self, _sequence: u64) {}
}

fn recording_test_answers() -> Vec<Answer> {
    vec![
        Answer {
            status: 200,
            headers: vec![("content-type", "text/plain")],
            body: b"live-0",
            cut: false,
        },
        Answer {
            status: 200,
            headers: vec![("content-type", "text/plain")],
            body: b"live-1",
            cut: false,
        },
        Answer {
            status: 200,
            headers: vec![("content-length", "40")],
            body: b"half",
            cut: true,
        },
    ]
}

/// An HTTP tier in `mode: "record"` records both whole and cut responses,
/// reads no cache entry while still storing into the cache (ADR 0105, #800).
/// Covers: story:6-55, story:6-57
#[tokio::test]
async fn a_recording_http_tier_captures_whole_and_cut_responses_and_bypasses_cache_read() {
    use goodfetch_engine::response_cache::Capture;

    let env = TestEnv::new();
    let cache = ResponseCache::of(
        env.home().join("0.1").join("response-cache"),
        None,
        Arc::new(TaskOwner::root(BlockingPolicy::Inline)),
        CountedWork::default(),
    );
    let clock = Clock::paused(
        goodfetch_protocol::point_in_time::from_rfc3339("2026-01-05T09:00:00Z").unwrap(),
    );
    let captured = Arc::new(CapturedList::default());
    let source = ResponseSource::of(&cache, CacheMode::Replay)
        .capturing(Arc::clone(&captured) as Arc<dyn Capture>);
    let caching = Caching::of(source, "ada", RunId::parse("record-http").unwrap(), clock).unwrap();

    let site = Site::answering(recording_test_answers());
    let transport = over(&caching, Arc::clone(&site) as Arc<dyn Transport>);
    let bounds = FetchBounds {
        first_byte: Deadline::given(&TIMEOUT_FIRST_BYTE, 30_000, SettingsLayer::Run),
        navigation: Deadline::given(&TIMEOUT_NAVIGATION, 30_000, SettingsLayer::Run),
        limits: Limits::default(),
    };

    let live_url: url::Url = "https://cache.test/live".parse().unwrap();
    let first = fetch(
        transport.as_ref(),
        &Hop::get(live_url.clone()),
        &bounds,
        &clock,
    )
    .await
    .unwrap();
    assert_eq!(first.body.as_ref(), b"live-0");

    // Second fetch in record mode bypasses the cache read and fetches live-1.
    let second = fetch(
        transport.as_ref(),
        &Hop::get(live_url.clone()),
        &bounds,
        &clock,
    )
    .await
    .unwrap();
    assert_eq!(second.body.as_ref(), b"live-1");

    // Cut response fails with response_truncated AND is recorded as truncated.
    let cut_err = fetch(
        transport.as_ref(),
        &Hop::get("https://cache.test/cut".parse().unwrap()),
        &bounds,
        &clock,
    )
    .await
    .unwrap_err();
    assert_eq!(cut_err.error().code(), "response_truncated");

    assert_eq!(
        captured.0.lock().unwrap().clone(),
        vec![
            (
                "https://cache.test/live".to_owned(),
                false,
                Bytes::from_static(b"live-0"),
            ),
            (
                "https://cache.test/live".to_owned(),
                false,
                Bytes::from_static(b"live-1"),
            ),
            (
                "https://cache.test/cut".to_owned(),
                true,
                Bytes::from_static(b"half"),
            ),
        ]
    );

    // Meanwhile, a subsequent cache-replay Run serves what the record Run stored.
    let cache_only = Caching::of(
        ResponseSource::of(&cache, CacheMode::Replay),
        "ada",
        RunId::parse("after-record").unwrap(),
        clock,
    )
    .unwrap();
    let cached_transport = over(&cache_only, Arc::clone(&site) as Arc<dyn Transport>);
    let hit = fetch(
        cached_transport.as_ref(),
        &Hop::get(live_url),
        &bounds,
        &clock,
    )
    .await
    .unwrap();
    assert_eq!(hit.body.as_ref(), b"live-1");
    assert_eq!(site.received().len(), 3);
}
