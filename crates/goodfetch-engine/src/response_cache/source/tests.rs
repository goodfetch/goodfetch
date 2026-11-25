//! `ResponseSource` through its interface, on a real cache in the test's
//! folder and a recording stand-in: a fulfilling route answers before the
//! Run's Response recording, which answers before the cache, which answers
//! before the network; with `cache.mode: off` the cache is never read nor
//! written.
#![expect(
    clippy::disallowed_methods,
    reason = "Tokio's test runtime runs the tests, as in the cache's own tests"
)]

use std::sync::{Arc, PoisonError};
use std::time::{Duration, SystemTime};

use bytes::Bytes;
use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_protocol::response_cache::CacheMode;
use goodfetch_test_support::TestEnv;

use super::{Recording, Replaying, ResponseSource, Served, Source};
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
        Box::pin(async { Some(served("recording")) })
    }
}

fn body(source: &Source) -> Option<&[u8]> {
    match source {
        Source::Routed(served) | Source::Recorded(served) => Some(&served.body),
        Source::Fresh(hit) => Some(&hit.body),
        Source::Stale(_) | Source::Network => None,
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
        .source(Some(served("route")), &ada(), &page(), now)
        .await;
    let asked_after_route = recording.asked.lock().unwrap().len();
    let lookups_after_route = work.read(Row::CacheLookups);
    let replayed = recorded.source(None, &ada(), &page(), now).await;
    let lookups_after_recording = work.read(Row::CacheLookups);
    let from_cache = unrecorded.source(None, &ada(), &page(), now).await;
    let other = request("GET", "/other", &[], "");
    let sent = unrecorded.source(None, &ada(), &other, now).await;

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

    let storable = source.stores(&page(), &response.head);
    let stored = source
        .store(
            &ada(),
            &page(),
            &response.head,
            response.body,
            None,
            start(),
        )
        .await;
    let answered = source.source(None, &ada(), &page(), start()).await;

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

    let judged = source.stores(&first, &response.head);
    let kept = source
        .store(&ada(), &first, &response.head, response.body, None, start())
        .await;
    let a_day_later = start() + Duration::from_hours(24);
    let reordered = source
        .source(
            None,
            &ada(),
            &search(r#"{ "page": 1, "q": "shoes" }"#),
            a_day_later,
        )
        .await;
    let other = source
        .source(
            None,
            &ada(),
            &search(r#"{"q":"boots","page":1}"#),
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

    let post_judged = revalidate.stores(&post, &response.head);
    let post_kept = revalidate
        .store(
            &ada(),
            &post,
            &response.head,
            response.body.clone(),
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
                start()
            )
            .await
    );
    let page_replayed = replay.source(None, &ada(), &page(), start()).await;
    let replayed_page = request("GET", "/replayed", &[], "");
    assert!(
        replay
            .store(&ada(), &replayed_page, &kept.head, kept.body, None, start())
            .await
    );
    assert!(
        replay
            .store(&ada(), &post, &response.head, response.body, None, start())
            .await
    );
    let page_revalidated = revalidate
        .source(None, &ada(), &replayed_page, start())
        .await;
    let post_revalidated = revalidate.source(None, &ada(), &post, start()).await;

    assert_eq!((post_judged, post_kept), (false, false));
    assert_eq!(page_replayed, Source::Network);
    assert_eq!(page_revalidated, Source::Network);
    assert_eq!(post_revalidated, Source::Network);
}
