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

fn page() -> CacheRequest {
    CacheRequest {
        method: "GET".to_owned(),
        url: "https://cache.test/page".to_owned(),
        headers: Vec::new(),
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
    let other = CacheRequest {
        url: "https://cache.test/other".to_owned(),
        ..page()
    };
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
