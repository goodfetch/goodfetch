#![expect(
    clippy::disallowed_methods,
    reason = "the tests read and write a temporary folder outside any engine, on the test Clock (ADR 0059)"
)]

use std::collections::BTreeMap;
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use goodfetch_clean_up_rules::rules::{FilterLists, ListTexts};
use goodfetch_engine::clock::Clock;
use goodfetch_engine::home::Home;
use goodfetch_engine::response_limits::Limits;
use goodfetch_engine::rule_sources::Sourcing;
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_protocol::clean_up_rules::{
    CleanUpRulesManifest, MANIFEST_FILE, RulesLists, RulesSource,
};
use goodfetch_protocol::formats::CLEAN_UP_RULES;
use goodfetch_test_support::TestEnv;
use serde_json::json;

mod ahead;

use super::upstream::Fetches;
use super::{
    KEPT_VERSIONS, Kept, Sources, Unobtained, Version, Versions, manifest_file, now_of, replace,
};

/// Covers: story:5-97
#[test]
fn the_sources_state_is_replaced_whole_through_a_partial_file_that_does_not_stay() {
    let env = goodfetch_test_support::TestEnv::new();
    let state = env.folder().join("0.0").join("clean-up-rules.json");

    replace(&state, b"{\"upstream\":1}").unwrap();
    replace(&state, b"{\"upstream\":2}").unwrap();

    let mut left: Vec<String> = std::fs::read_dir(state.parent().unwrap())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    left.sort();
    assert_eq!(
        (std::fs::read_to_string(&state).unwrap(), left),
        (
            "{\"upstream\":2}".to_owned(),
            vec!["clean-up-rules.json".to_owned()]
        )
    );
}

/// A version is stale from 14 days after its date on, whatever the
/// calendar says when the test runs: the rule takes "now" (TESTING.md ›
/// Time), so the embedded part's date only moves where it goes stale.
/// Covers: story:5-97
#[test]
fn a_version_goes_stale_14_days_after_its_date_on_any_calendar() {
    let embedded = Version::embedded();
    let published = jiff::Timestamp::try_from(embedded.manifest.published_at).unwrap();
    let stale_from = published + jiff::SignedDuration::from_hours(14 * 24);
    let second = jiff::SignedDuration::from_secs(1);

    assert_eq!(
        [
            published - jiff::SignedDuration::from_hours(24),
            published,
            stale_from - second,
            stale_from,
            jiff::Timestamp::MAX,
        ]
        .map(|now| embedded.stale(now)),
        [false, false, false, true, true]
    );
}

/// The test Clock's start.
fn clock() -> Clock {
    Clock::paused(UNIX_EPOCH + Duration::from_hours(491_001))
}

/// The sources of `env`'s home whose Filter lists are all at `origin`.
fn sources(env: &TestEnv, clock: Clock, origin: &str) -> Sources {
    let home = Home::new(env.home().to_path_buf());
    let lists: BTreeMap<String, String> = ["ads", "trackers", "annoyances", "unbreak"]
        .into_iter()
        .map(|list| (list.to_owned(), format!("https://{origin}/{list}.txt")))
        .collect();
    Sources {
        sourcing: Sourcing::new(clock),
        kept: Kept::in_home(&home, TaskOwner::root(BlockingPolicy::Inline)),
        state: env.folder().join("clean-up-rules.json"),
        channel: String::new(),
        lists,
        limits: Limits::default(),
    }
}

/// A version of the lists `sources` names, fetched before `now` and
/// expired since, kept and named by their state as the last one fetched.
async fn expired_version(sources: &Sources, now: jiff::Timestamp) -> Version {
    let manifest = CleanUpRulesManifest {
        format_version: CLEAN_UP_RULES.version,
        source: RulesSource::Upstream,
        published_at: SystemTime::from(now - jiff::SignedDuration::from_hours(48)),
        expires_at: Some(SystemTime::from(now - jiff::SignedDuration::from_hours(24))),
        lists: RulesLists {
            ads: "easylist.txt".to_owned(),
            trackers: "easyprivacy.txt".to_owned(),
            annoyances: "fanboy-annoyance.txt".to_owned(),
            unbreak: "brave-unbreak.txt".to_owned(),
        },
    };
    let mut files: Vec<(String, Vec<u8>)> = manifest
        .lists
        .files()
        .into_iter()
        .map(|(_, file)| (file.to_owned(), b"||ads.example^\n".to_vec()))
        .collect();
    files.push((MANIFEST_FILE.to_owned(), manifest_file(&manifest)));
    let (version, _) = sources
        .kept
        .keep(RulesSource::Upstream, files, now)
        .await
        .unwrap();
    sources
        .write_state(json!({ "upstream": { "version": version.digest, "lists": sources.lists } }))
        .await;
    version
}

/// When the re-fetch of an expired version stalls past
/// `clean_up.fetch_timeout`, a new Run holds to the last version fetched,
/// as it does when the re-fetch is refused, never to the embedded part:
/// the timeout bounds the fetch, not the reading of what the machine has.
/// Covers: story:5-97
#[tokio::test(start_paused = true)]
async fn a_refetch_that_stalls_past_the_timeout_keeps_the_last_version_fetched() {
    let env = TestEnv::new();
    // Connections queue on it and are never answered.
    let stalled = TcpListener::bind("127.0.0.1:0").unwrap();
    let clock = clock();
    let now = now_of(clock);
    let sources = sources(&env, clock, &stalled.local_addr().unwrap().to_string());
    let last = expired_version(&sources, now).await;

    let newest = Fetches::default()
        .newest(&sources, now, clock, Duration::from_secs(2))
        .await;

    assert_eq!(
        newest
            .map(|version| version.digest)
            .map_err(|failure| failure.to_string()),
        Ok(last.digest)
    );
}

/// Runs that start together with no version fetched share one fetch of
/// the lists, and its outcome: here one connection to a source that hangs
/// up, and the same failure for both.
#[tokio::test]
async fn runs_starting_together_share_one_fetch_of_the_lists() {
    let env = TestEnv::new();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = listener.local_addr().unwrap().to_string();
    let connections = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&connections);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            counted.fetch_add(1, Ordering::SeqCst);
            drop(stream);
        }
    });
    let clock = clock();
    let now = now_of(clock);
    let (first, second) = (sources(&env, clock, &origin), sources(&env, clock, &origin));
    let fetches = Fetches::default();
    let limit = Duration::from_secs(30);

    let both = tokio::join!(
        fetches.newest(&first, now, clock, limit),
        fetches.newest(&second, now, clock, limit),
    );

    let reason = |newest: Result<Version, Unobtained>| newest.map(|_| ()).map_err(|f| f.reason());
    assert_eq!(
        (
            connections.load(Ordering::SeqCst),
            reason(both.0),
            reason(both.1)
        ),
        (1, Err("unreachable"), Err("unreachable"))
    );
}

/// A version whose files no longer read, which several of a Run's first
/// pages ask for at once, is loaded once, and every one of them gets its
/// failure; a page asking later tries again.
#[tokio::test(start_paused = true)]
async fn concurrent_loads_of_a_version_that_does_not_read_fail_together() {
    let versions = Versions::new(|_| false);
    let clock = clock();
    let loads = AtomicUsize::new(0);
    let load = || async {
        loads.fetch_add(1, Ordering::SeqCst);
        clock.sleep(Duration::from_secs(1)).await;
        Err::<goodfetch_clean_up_rules::rules::FilterLists, _>(Unobtained::NotLoaded(
            "it is gone".to_owned(),
        ))
    };
    let digest = format!("sha256:{}", "0".repeat(64));

    let (first, second) = tokio::join!(
        versions.loaded_once(&digest, load),
        versions.loaded_once(&digest, load),
    );
    let together = loads.load(Ordering::SeqCst);
    let later = versions.loaded_once(&digest, load).await;

    let reason = |loaded: Result<Arc<_>, Unobtained>| loaded.map(|_| ()).map_err(|f| f.reason());
    assert_eq!(
        (
            together,
            reason(first),
            reason(second),
            loads.load(Ordering::SeqCst),
            reason(later)
        ),
        (
            1,
            Err("not_loaded"),
            Err("not_loaded"),
            2,
            Err("not_loaded")
        )
    );
}

/// Compiles no rule, counting each load on `loads`.
fn empty_lists(loads: &AtomicUsize) -> FilterLists {
    loads.fetch_add(1, Ordering::SeqCst);
    FilterLists::compile(&ListTexts {
        ads: "",
        trackers: "",
        annoyances: "",
        unbreak: "",
    })
}

/// A digest of its own for each of `n`.
fn digest_of(n: usize) -> String {
    format!("sha256:{n:064}")
}

/// A version whose Runs all ended is not loaded again for the next Run
/// that holds to it: the engine keeps it, within its bound.
#[tokio::test]
async fn the_next_run_on_a_version_no_run_holds_reuses_its_lists() {
    let versions = Versions::new(|_| false);
    let loads = AtomicUsize::new(0);
    let load = || std::future::ready(Ok::<_, Unobtained>(empty_lists(&loads)));

    let first = versions.loaded_once(&digest_of(1), load).await.unwrap();
    drop(first);
    let next = versions.loaded_once(&digest_of(1), load).await;

    assert_eq!((next.is_ok(), loads.load(Ordering::SeqCst)), (true, 1));
}

/// The engine keeps at most `KEPT_VERSIONS` versions no Run holds, the
/// most recently used: an older one is loaded again.
#[tokio::test]
async fn versions_no_run_holds_are_kept_within_their_bound() {
    let versions = Versions::new(|_| false);
    let loads = AtomicUsize::new(0);
    let load = || std::future::ready(Ok::<_, Unobtained>(empty_lists(&loads)));

    for n in 0..=KEPT_VERSIONS {
        drop(versions.loaded_once(&digest_of(n), load).await.unwrap());
    }
    let newest = versions.loaded_once(&digest_of(KEPT_VERSIONS), load).await;
    let after_newest = loads.load(Ordering::SeqCst);
    let oldest = versions.loaded_once(&digest_of(0), load).await;

    assert_eq!(
        (
            newest.is_ok(),
            after_newest,
            oldest.is_ok(),
            loads.load(Ordering::SeqCst)
        ),
        (true, KEPT_VERSIONS + 1, true, KEPT_VERSIONS + 2)
    );
}
