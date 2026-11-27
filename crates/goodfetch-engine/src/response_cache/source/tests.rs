//! `ResponseSource` through its interface, on a real cache in the test's
//! folder and a recording stand-in: a fulfilling route answers before the
//! Run's Response recording, which answers before the cache, which answers
//! before the network; with `cache.mode: off` the cache is never read nor
//! written; what a fetch's outcome keeps out is never stored, and a Paid
//! fetch never served.
#![expect(
    clippy::disallowed_methods,
    reason = "Tokio's test runtime runs the tests, as in the cache's own tests"
)]

use std::sync::{Arc, PoisonError};
use std::time::{Duration, SystemTime};

use bytes::Bytes;
use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_protocol::response_cache::CacheMode;
use goodfetch_test_support::{Canaries, Output, SecretKind, TestEnv};
use rstest::rstest;

use super::{
    KeptOut, NotRecorded, Recorded as RecordingAnswer, Recording, Replaying, ResponseSource,
    Served, Source,
};
use crate::response_cache::{CacheRequest, Partition, ResponseCache, ResponseHead};
use crate::sync::Mutex;
use crate::tasks::{BlockingPolicy, TaskOwner};

fn start() -> SystemTime {
    goodfetch_protocol::point_in_time::from_rfc3339("2026-01-05T09:00:00Z").unwrap()
}

fn cache(env: &TestEnv, work: &CountedWork) -> ResponseCache {
    ResponseCache::of(
        env.home().join("0.1").join("response-cache"),
        None,
        Arc::new(TaskOwner::root(BlockingPolicy::Inline)),
        work.clone(),
    )
}

fn ada() -> Partition {
    Partition {
        person: "ada".to_owned(),
        ..Partition::default()
    }
}

fn request(method: &str, path: &str, headers: &[(&str, &str)], body: &str) -> CacheRequest {
    let headers: Vec<(String, String)> = headers
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    CacheRequest::new(
        method,
        &format!("https://cache.test{path}").parse().unwrap(),
        &headers,
        Bytes::from(body.to_owned()),
    )
}

fn page() -> CacheRequest {
    request("GET", "/page", &[], "")
}

/// A POST of `body` as JSON to the search API.
fn search(body: &str) -> CacheRequest {
    request(
        "POST",
        "/api/search",
        &[("content-type", "application/json")],
        body,
    )
}

/// What the search API answered: no freshness at all.
fn answered(from: &str) -> Served {
    Served {
        head: ResponseHead {
            status: 200,
            headers: vec![
                ("content-type".to_owned(), "application/json".to_owned()),
                ("cache-control".to_owned(), "no-store".to_owned()),
            ],
        },
        body: Bytes::from(format!("{{\"from\":\"{from}\"}}")),
    }
}

fn served(from: &str) -> Served {
    Served {
        head: ResponseHead {
            status: 200,
            headers: vec![
                ("content-type".to_owned(), "text/html".to_owned()),
                ("cache-control".to_owned(), "max-age=60".to_owned()),
            ],
        },
        body: Bytes::from(format!("<p>{from}</p>")),
    }
}

/// A recording stand-in that recorded `served("recording")` for every
/// request, and remembers the URLs it was asked for.
#[derive(Debug, Default)]
struct Recorded {
    asked: Mutex<Vec<String>>,
}

impl Recording for Recorded {
    fn recorded<'a>(&'a self, request: &'a CacheRequest) -> Replaying<'a> {
        self.asked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request.url.clone());
        Box::pin(async { RecordingAnswer::Answered(Arc::new(served("recording"))) })
    }
}

/// A recording stand-in that misses every request.
#[derive(Debug)]
struct Missing;

impl Recording for Missing {
    fn recorded<'a>(&'a self, request: &'a CacheRequest) -> Replaying<'a> {
        let miss = NotRecorded {
            method: request.method.clone(),
            url: request.url.clone(),
            recording: "missing.warc".to_owned(),
            nearest: vec!["https://cache.test/page?a=1".to_owned()],
        };
        Box::pin(async move { RecordingAnswer::Missed(miss) })
    }
}

fn body(source: &Source) -> Option<&[u8]> {
    match source {
        Source::Routed(served) => Some(&served.body),
        Source::Recorded(served) | Source::RecordedCut(served) => Some(&served.body),
        Source::Fresh(hit) => Some(&hit.body),
        Source::NotRecorded(_) | Source::Stale(_) | Source::Network => None,
    }
}

/// The order of ADR 0046: a fulfilling route answers first, the Run's
/// Response recording second, the cache third and the network last; an
/// earlier answer leaves the later sources unasked.
/// Covers: story:6-37, setting:cache.mode
#[tokio::test]
async fn a_route_then_the_recording_then_the_cache_then_the_network_answers() {
    let env = TestEnv::new();
    let work = CountedWork::default();
    let cache = cache(&env, &work);
    let cached = served("cache");
    assert!(
        cache
            .store(&ada(), &page(), &cached.head, cached.body, None, start())
            .await
    );
    let recording = Arc::new(Recorded::default());
    let recorded =
        ResponseSource::of(&cache, CacheMode::Revalidate).recording(Arc::clone(&recording) as _);
    let unrecorded = ResponseSource::of(&cache, CacheMode::Revalidate);
    let now = start() + Duration::from_secs(1);

    let routed = recorded
        .source(Some(served("route")), &ada(), &page(), None, now)
        .await;
    let asked_after_route = recording.asked.lock().unwrap().len();
    let lookups_after_route = work.read(Row::CacheLookups);
    let replayed = recorded.source(None, &ada(), &page(), None, now).await;
    let lookups_after_recording = work.read(Row::CacheLookups);
    let from_cache = unrecorded.source(None, &ada(), &page(), None, now).await;
    let other = request("GET", "/other", &[], "");
    let sent = unrecorded.source(None, &ada(), &other, None, now).await;

    assert_eq!(body(&routed), Some(&b"<p>route</p>"[..]));
    assert_eq!((asked_after_route, lookups_after_route), (0, 0));
    assert!(matches!(replayed, Source::Recorded(_)));
    assert_eq!(body(&replayed), Some(&b"<p>recording</p>"[..]));
    assert_eq!(lookups_after_recording, 0);
    assert!(matches!(from_cache, Source::Fresh(_)));
    assert_eq!(body(&from_cache), Some(&b"<p>cache</p>"[..]));
    assert_eq!(sent, Source::Network);
}

/// With `cache.mode: off`, the default, the cache is neither looked up nor
/// written: every request goes to the network and nothing is stored.
/// Covers: story:6-38, setting:cache.mode
#[tokio::test]
async fn with_the_cache_off_nothing_is_looked_up_or_stored() {
    let env = TestEnv::new();
    let work = CountedWork::default();
    let cache = cache(&env, &work);
    let source = ResponseSource::of(&cache, CacheMode::Off);
    let response = served("network");

    let storable = source.stores(&ada(), &page(), &response.head, None);
    let stored = source
        .store(
            &ada(),
            &page(),
            &response.head,
            response.body,
            None,
            None,
            start(),
        )
        .await;
    let answered = source.source(None, &ada(), &page(), None, start()).await;

    assert_eq!((storable, stored), (false, false));
    assert_eq!(answered, Source::Network);
    assert!(source.sends_only());
    assert_eq!(
        (
            work.read(Row::CacheLookups),
            work.read(Row::CacheBodyWrites)
        ),
        (0, 0)
    );
    assert!(!env.home().join("0.1").join("response-cache").exists());
}

/// Under `replay`, a POST's answer, which no freshness lets be kept, is
/// stored and served again a day later with no request, to the same JSON
/// with its keys in another order; a POST of another body is not served,
/// and each answer takes one lookup.
/// Covers: story:6-36, setting:cache.mode
#[tokio::test]
async fn replay_serves_a_post_by_its_canonical_body_whatever_its_age() {
    let env = TestEnv::new();
    let work = CountedWork::default();
    let source = ResponseSource::of(&cache(&env, &work), CacheMode::Replay);
    let first = search(r#"{"q":"shoes","page":1}"#);
    let response = answered("site");

    let judged = source.stores(&ada(), &first, &response.head, None);
    let kept = source
        .store(
            &ada(),
            &first,
            &response.head,
            response.body,
            None,
            None,
            start(),
        )
        .await;
    let a_day_later = start() + Duration::from_hours(24);
    let reordered = source
        .source(
            None,
            &ada(),
            &search(r#"{ "page": 1, "q": "shoes" }"#),
            None,
            a_day_later,
        )
        .await;
    let other = source
        .source(
            None,
            &ada(),
            &search(r#"{"q":"boots","page":1}"#),
            None,
            a_day_later,
        )
        .await;

    assert_eq!((judged, kept), (true, true));
    assert!(matches!(reordered, Source::Fresh(_)));
    assert_eq!(body(&reordered), Some(&b"{\"from\":\"site\"}"[..]));
    assert_eq!(other, Source::Network);
    assert_eq!(work.read(Row::CacheLookups), 2);
}

/// Under `revalidate` the same POST is neither stored nor served, and
/// what `replay` stored never answers `revalidate`, nor the reverse.
/// Covers: story:6-36, setting:cache.mode
#[tokio::test]
async fn revalidate_and_replay_never_answer_each_other() {
    let env = TestEnv::new();
    let work = CountedWork::default();
    let cache = cache(&env, &work);
    let revalidate = ResponseSource::of(&cache, CacheMode::Revalidate);
    let replay = ResponseSource::of(&cache, CacheMode::Replay);
    let post = search(r#"{"q":"shoes"}"#);
    let response = answered("site");
    let kept = served("cache");

    let post_judged = revalidate.stores(&ada(), &post, &response.head, None);
    let post_kept = revalidate
        .store(
            &ada(),
            &post,
            &response.head,
            response.body.clone(),
            None,
            None,
            start(),
        )
        .await;
    assert!(
        revalidate
            .store(
                &ada(),
                &page(),
                &kept.head,
                kept.body.clone(),
                None,
                None,
                start()
            )
            .await
    );
    let page_replayed = replay.source(None, &ada(), &page(), None, start()).await;
    let replayed_page = request("GET", "/replayed", &[], "");
    assert!(
        replay
            .store(
                &ada(),
                &replayed_page,
                &kept.head,
                kept.body,
                None,
                None,
                start()
            )
            .await
    );
    assert!(
        replay
            .store(
                &ada(),
                &post,
                &response.head,
                response.body,
                None,
                None,
                start()
            )
            .await
    );
    let page_revalidated = revalidate
        .source(None, &ada(), &replayed_page, None, start())
        .await;
    let post_revalidated = revalidate.source(None, &ada(), &post, None, start()).await;

    assert_eq!((post_judged, post_kept), (false, false));
    assert_eq!(page_replayed, Source::Network);
    assert_eq!(page_revalidated, Source::Network);
    assert_eq!(post_revalidated, Source::Network);
}

/// A response its fetch's own outcome keeps out is never stored, under
/// either mode, so the next request for it goes to the network.
/// Covers: story:6-40
#[rstest]
#[case::blocked(CacheMode::Revalidate, KeptOut::Blocked)]
#[case::soft_blocked(CacheMode::Revalidate, KeptOut::SoftBlocked)]
#[case::failed_page_check(CacheMode::Revalidate, KeptOut::FailedPageCheck)]
#[case::cut_body(CacheMode::Revalidate, KeptOut::CutBody)]
#[case::paid(CacheMode::Revalidate, KeptOut::Paid)]
#[case::resolved_secret(CacheMode::Revalidate, KeptOut::ResolvedSecret)]
#[case::certificate_exception(CacheMode::Revalidate, KeptOut::CertificateException)]
#[case::replayed_block(CacheMode::Replay, KeptOut::Blocked)]
#[case::replayed_resolved_secret(CacheMode::Replay, KeptOut::ResolvedSecret)]
#[tokio::test]
async fn a_response_its_outcome_keeps_out_is_never_stored(
    #[case] mode: CacheMode,
    #[case] kept_out: KeptOut,
) {
    let env = TestEnv::new();
    let work = CountedWork::default();
    let source = ResponseSource::of(&cache(&env, &work), mode);
    let response = served("site");

    let judged = source.stores(&ada(), &page(), &response.head, Some(kept_out));
    let stored = source
        .store(
            &ada(),
            &page(),
            &response.head,
            response.body,
            Some(kept_out),
            None,
            start(),
        )
        .await;
    let next = source.source(None, &ada(), &page(), None, start()).await;

    assert_eq!((judged, stored), (false, false));
    assert_eq!(next, Source::Network);
    assert_eq!(work.read(Row::CacheBodyWrites), 0);
}

/// A request carrying a resolved secret, in its URL and echoed in its
/// response's body, leaves no trace in the cache's files under either
/// mode (ADR 0067): neither the index, its write-ahead log nor a body
/// file holds the canary, while the entry stored beside it holds its own.
/// Covers: story:6-40
#[rstest]
#[case::revalidate(CacheMode::Revalidate)]
#[case::replay(CacheMode::Replay)]
#[tokio::test]
async fn a_resolved_secrets_canary_is_nowhere_in_the_cache(#[case] mode: CacheMode) {
    let env = TestEnv::new();
    let mut canaries = Canaries::new(env.seed());
    let secret = canaries.plant(SecretKind::EndpointToken).value().to_owned();
    let control = canaries.plant(SecretKind::SessionCookie).value().to_owned();
    let source = ResponseSource::of(&cache(&env, &CountedWork::default()), mode);
    let query: String = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("token", &secret)
        .finish();
    let carrying = request("GET", &format!("/page?{query}"), &[], "");
    let echoed = Served {
        body: Bytes::from(format!("<p>{secret}</p>")),
        ..served("site")
    };
    let beside = Served {
        body: Bytes::from(format!("<p>{control}</p>")),
        ..served("site")
    };

    let stored = source
        .store(
            &ada(),
            &carrying,
            &echoed.head,
            echoed.body,
            Some(KeptOut::ResolvedSecret),
            None,
            start(),
        )
        .await;
    let control_stored = source
        .store(
            &ada(),
            &page(),
            &beside.head,
            beside.body,
            None,
            None,
            start(),
        )
        .await;

    let found: Vec<&str> = canaries
        .search(&[Output::Path(&env.home().join("0.1").join("response-cache"))])
        .unwrap()
        .into_iter()
        .map(|hit| hit.kind.name())
        .collect();
    assert_eq!((stored, control_stored), (false, true));
    assert!(
        !found.is_empty()
            && found
                .iter()
                .all(|kind| *kind == SecretKind::SessionCookie.name()),
        "{found:?}"
    );
}

/// A Paid fetch is never answered from the cache, even by a fresh entry
/// another fetch stored, and does not look it up; the fetch that is not
/// Paid is answered from it.
/// Covers: story:6-40
#[tokio::test]
async fn a_paid_fetch_is_never_served_from_the_cache() {
    let env = TestEnv::new();
    let work = CountedWork::default();
    let source = ResponseSource::of(&cache(&env, &work), CacheMode::Revalidate);
    let kept = served("cache");
    assert!(
        source
            .store(&ada(), &page(), &kept.head, kept.body, None, None, start())
            .await
    );
    let now = start() + Duration::from_secs(1);

    let paid = source
        .source(None, &ada(), &page(), Some(KeptOut::Paid), now)
        .await;
    let lookups_after_paid = work.read(Row::CacheLookups);
    let unpaid = source.source(None, &ada(), &page(), None, now).await;

    assert_eq!(paid, Source::Network);
    assert_eq!(lookups_after_paid, 0);
    assert_eq!(body(&unpaid), Some(&b"<p>cache</p>"[..]));
}

/// A browser pauses a response to keep it only where one may be stored:
/// a GET under `revalidate`, a POST too under `replay`, nothing with the
/// cache off or for a request an outcome keeps out.
/// Covers: story:6-41
#[rstest]
#[case::revalidate_get(CacheMode::Revalidate, "GET", None, true)]
#[case::revalidate_post(CacheMode::Revalidate, "POST", None, false)]
#[case::replay_post(CacheMode::Replay, "POST", None, true)]
#[case::off(CacheMode::Off, "GET", None, false)]
#[case::secret(CacheMode::Revalidate, "GET", Some(KeptOut::ResolvedSecret), false)]
fn a_response_is_paused_only_where_it_may_be_stored(
    #[case] mode: CacheMode,
    #[case] method: &str,
    #[case] kept_out: Option<KeptOut>,
    #[case] paused: bool,
) {
    let env = TestEnv::new();
    let source = ResponseSource::of(&cache(&env, &CountedWork::default()), mode);

    assert_eq!(
        source.may_store(&request(method, "/page", &[], ""), kept_out),
        paused
    );
}

/// A transport asks the sources about a request, and reads what it needs
/// for that, only where its method may be answered or stored: a GET or a
/// HEAD under `revalidate`, any method under `replay` or with a recording,
/// none with the cache off and no recording.
/// Covers: story:6-41
#[rstest]
#[case::revalidate_get(CacheMode::Revalidate, false, "GET", true)]
#[case::revalidate_head(CacheMode::Revalidate, false, "HEAD", true)]
#[case::revalidate_post(CacheMode::Revalidate, false, "POST", false)]
#[case::replay_post(CacheMode::Replay, false, "POST", true)]
#[case::off(CacheMode::Off, false, "GET", false)]
#[case::recording_post(CacheMode::Off, true, "POST", true)]
fn a_request_is_asked_about_only_where_its_method_may_be_answered(
    #[case] mode: CacheMode,
    #[case] recorded: bool,
    #[case] method: &str,
    #[case] asked: bool,
) {
    let env = TestEnv::new();
    let source = ResponseSource::of(&cache(&env, &CountedWork::default()), mode);
    let source = if recorded {
        source.recording(Arc::new(Recorded::default()))
    } else {
        source
    };

    assert_eq!(source.takes(method), asked);
}

/// While replaying a Response recording, a miss returns `Source::NotRecorded`
/// without consulting the cache or falling through to the network, and
/// nothing is stored into the cache.
#[tokio::test]
async fn a_recording_miss_never_falls_through_to_the_cache_or_network() {
    let env = TestEnv::new();
    let work = CountedWork::default();
    let cache = cache(&env, &work);
    let cached = served("cache");
    assert!(
        cache
            .store(
                &ada(),
                &page(),
                &cached.head,
                cached.body.clone(),
                None,
                start()
            )
            .await
    );
    let replaying = ResponseSource::of(&cache, CacheMode::Revalidate).recording(Arc::new(Missing));
    let now = start() + Duration::from_secs(1);

    let answer = replaying.source(None, &ada(), &page(), None, now).await;

    assert!(replaying.replaying());
    assert!(!replaying.may_store(&page(), None));
    assert!(!replaying.stores(&ada(), &page(), &cached.head, None));
    assert_eq!(work.read(Row::CacheLookups), 0);
    assert_eq!(
        answer,
        Source::NotRecorded(NotRecorded {
            method: "GET".to_owned(),
            url: "https://cache.test/page".to_owned(),
            recording: "missing.warc".to_owned(),
            nearest: vec!["https://cache.test/page?a=1".to_owned()],
        })
    );
}

#[derive(Debug, Default)]
struct NoopCapture;

impl super::Capture for NoopCapture {
    fn capture(
        &self,
        _request: CacheRequest,
        _head: ResponseHead,
        _body: Bytes,
        _truncated: bool,
        _now: SystemTime,
    ) -> super::Capturing<'_> {
        Box::pin(async { 1 })
    }

    fn mark_truncated(&self, _sequence: u64) {}
}

/// A `record` Run reads no cache entry (even when fresh or under `replay`)
/// while still allowing the cache to store what it fetched (ADR 0105).
#[rstest]
#[case::revalidate(CacheMode::Revalidate)]
#[case::replay(CacheMode::Replay)]
#[tokio::test]
async fn a_record_run_reads_no_cache_entry_but_may_store_into_the_cache(#[case] mode: CacheMode) {
    let env = TestEnv::new();
    let work = CountedWork::default();
    let cache = cache(&env, &work);
    let cached = served("from-cache");
    assert!(
        cache
            .store(
                &ada(),
                &page(),
                &cached.head,
                cached.body.clone(),
                None,
                start()
            )
            .await
    );
    let recording = ResponseSource::of(&cache, mode).capturing(Arc::new(NoopCapture));
    let now = start() + Duration::from_secs(1);

    let answer = recording.source(None, &ada(), &page(), None, now).await;

    assert!(recording.recording_live());
    assert!(!recording.replaying());
    assert_eq!(answer, Source::Network);
    assert_eq!(work.read(Row::CacheLookups), 0);
    assert!(recording.may_store(&page(), None));
    assert!(recording.stores(&ada(), &page(), &cached.head, None));
    let fresh = served("fresh-from-site");
    assert!(
        recording
            .store(
                &ada(),
                &page(),
                &fresh.head,
                fresh.body.clone(),
                None,
                None,
                now
            )
            .await
    );
}
