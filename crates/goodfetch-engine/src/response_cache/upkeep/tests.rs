//! The Response cache's upkeep through its interface, on real SQLite and
//! body files in the test's folder: stats count one person's entries by
//! partition, a clear deletes what its filters keep, by Run, domain,
//! partition or all of a person's, with the body files no entry holds any
//! more, and leaves every other entry served; a verify lists the entries
//! whose body file is missing or changed. None creates a cache never
//! written, each refuses an index of a newer version, and a planted canary
//! is found nowhere in the cache's files once its entry is cleared.
#![expect(
    clippy::disallowed_methods,
    reason = "the tests read and tamper with the cache's folder, and relabel its index as a newer engine would"
)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use bytes::Bytes;
use goodfetch_counted_work::CountedWork;
use goodfetch_protocol::response_cache::{
    CacheBodyProblem, CacheCleared, CacheDamage, CachePartitionStats, CacheStats, CacheVerified,
};
use goodfetch_protocol::run::RunId;
use goodfetch_test_support::{Canaries, Output, SecretKind, TestEnv};
use rusqlite::Connection;

use super::Clearing;
use crate::response_cache::{
    CacheRequest, Lookup, Partition, ResponseCache, ResponseCacheError, ResponseHead,
};
use crate::tasks::{BlockingPolicy, TaskOwner};

fn start() -> SystemTime {
    goodfetch_protocol::point_in_time::from_rfc3339("2026-01-05T09:00:00Z").unwrap()
}

fn folder(env: &TestEnv) -> PathBuf {
    env.home().join("0.1").join("response-cache")
}

fn cache(env: &TestEnv) -> ResponseCache {
    ResponseCache::of(
        folder(env),
        None,
        Arc::new(TaskOwner::root(BlockingPolicy::Inline)),
        CountedWork::default(),
    )
}

fn person(name: &str) -> Partition {
    Partition {
        person: name.to_owned(),
        ..Partition::default()
    }
}

fn ada() -> Partition {
    person("ada")
}

fn get(url: &str) -> CacheRequest {
    CacheRequest::new("GET", &url.parse().unwrap(), &[], Bytes::new())
}

/// A response fresh for an hour.
fn hour() -> ResponseHead {
    ResponseHead {
        status: 200,
        headers: vec![("cache-control".to_owned(), "max-age=3600".to_owned())],
    }
}

fn run(name: &str) -> RunId {
    RunId::parse(name).unwrap()
}

/// `body` stored for `url` in `partition` at the start, by the Run `run_id`.
async fn kept(cache: &ResponseCache, partition: &Partition, url: &str, body: &str, run_id: &str) {
    assert!(
        cache
            .store(
                partition,
                &get(url),
                &hour(),
                Bytes::from(body.to_owned()),
                Some(&run(run_id)),
                start(),
            )
            .await
    );
}

/// Whether `url` is served fresh to `partition` a minute after the start.
async fn served(cache: &ResponseCache, partition: &Partition, url: &str) -> bool {
    matches!(
        cache
            .lookup(partition, &get(url), start() + Duration::from_secs(60))
            .await,
        Lookup::Fresh(_)
    )
}

fn sha256(body: &str) -> String {
    data_encoding::HEXLOWER.encode(&<sha2::Sha256 as sha2::Digest>::digest(body.as_bytes()))
}

fn body_files(env: &TestEnv) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(folder(env).join("bodies"))
        .map(|entries| {
            entries
                .map(|entry| entry.unwrap().file_name().into_string().unwrap())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

fn cleared(entries: u64, bodies: u64, bytes: u64) -> CacheCleared {
    CacheCleared {
        entries,
        bodies,
        bytes,
    }
}

/// Stats count one person's entries, by partition the largest first, the
/// bodies they hold once each, and no other person's; a cache never
/// written counts nothing and is not created.
#[tokio::test]
async fn stats_count_one_persons_entries_by_partition() {
    let env = TestEnv::new();
    let cache = cache(&env);
    assert_eq!(cache.stats("ada").await.unwrap(), CacheStats::default());
    assert!(!folder(&env).exists());
    let in_france = Partition {
        country: "FR".to_owned(),
        ..ada()
    };
    kept(&cache, &ada(), "https://cache.test/a", "shared", "first").await;
    kept(&cache, &ada(), "https://cache.test/b", "shared", "first").await;
    kept(
        &cache,
        &in_france,
        "https://cache.test/c",
        "french",
        "first",
    )
    .await;
    kept(
        &cache,
        &person("grace"),
        "https://cache.test/d",
        "hers",
        "first",
    )
    .await;

    let stats = cache.stats("ada").await.unwrap();

    let partition = |country: &str, entries| CachePartitionStats {
        partition: "public".to_owned(),
        country: country.to_owned(),
        language: String::new(),
        device: String::new(),
        entries,
    };
    assert_eq!(
        (
            stats.entries,
            stats.bodies,
            stats.body_bytes,
            stats.partitions
        ),
        (3, 2, 12, vec![partition("", 2), partition("FR", 1)])
    );
    assert!(stats.held_bytes >= 16, "{}", stats.held_bytes);
}

/// A Run's clear deletes the entries it fetched for every person and the
/// body files only they held; another Run's entries stay served, with the
/// body they share.
#[tokio::test]
async fn a_runs_clear_deletes_its_entries_and_no_other_runs() {
    let env = TestEnv::new();
    let cache = cache(&env);
    kept(
        &cache,
        &ada(),
        "https://cache.test/a",
        "first only",
        "first",
    )
    .await;
    kept(
        &cache,
        &person("grace"),
        "https://cache.test/b",
        "shared",
        "first",
    )
    .await;
    kept(&cache, &ada(), "https://cache.test/c", "shared", "second").await;

    let answer = cache.clear(Clearing::of_run(&run("first"))).await.unwrap();

    assert_eq!(answer, cleared(2, 1, 10));
    assert_eq!(body_files(&env), [sha256("shared")]);
    assert!(!served(&cache, &ada(), "https://cache.test/a").await);
    assert!(!served(&cache, &person("grace"), "https://cache.test/b").await);
    assert!(served(&cache, &ada(), "https://cache.test/c").await);
}

/// A domain's clear deletes the entries of the host and its subdomains,
/// and none of another host, even one ending in the same letters.
#[tokio::test]
async fn a_domains_clear_deletes_its_host_and_subdomains_only() {
    let env = TestEnv::new();
    let cache = cache(&env);
    let urls = [
        "https://cache.test/a",
        "https://www.cache.test/b",
        "https://notcache.test/c",
        "https://other.test/d",
    ];
    for url in urls {
        kept(&cache, &ada(), url, url, "first").await;
    }

    let answer = cache
        .clear(Clearing {
            person: Some("ada".to_owned()),
            domain: Some("cache.test".to_owned()),
            ..Clearing::default()
        })
        .await
        .unwrap();

    assert_eq!((answer.entries, answer.bodies), (2, 2));
    let mut left = Vec::new();
    for url in urls {
        left.push(served(&cache, &ada(), url).await);
    }
    assert_eq!(left, [false, false, true, true]);
}

/// A partition's clear deletes the entries of that partition only: the
/// same person's other partitions and another person's same partition
/// stay served.
#[tokio::test]
async fn a_partitions_clear_leaves_the_other_partitions_served() {
    let env = TestEnv::new();
    let cache = cache(&env);
    let in_france = Partition {
        country: "FR".to_owned(),
        ..ada()
    };
    let grace_in_france = Partition {
        person: "grace".to_owned(),
        ..in_france.clone()
    };
    for partition in [&ada(), &in_france, &grace_in_france] {
        kept(
            &cache,
            partition,
            "https://cache.test/page",
            "page",
            "first",
        )
        .await;
    }

    let answer = cache
        .clear(Clearing {
            person: Some("ada".to_owned()),
            partition: Some("public".to_owned()),
            country: Some("FR".to_owned()),
            ..Clearing::default()
        })
        .await
        .unwrap();

    assert_eq!(answer, cleared(1, 0, 0));
    let mut left = Vec::new();
    for partition in [&ada(), &in_france, &grace_in_france] {
        left.push(served(&cache, partition, "https://cache.test/page").await);
    }
    assert_eq!(left, [true, false, true]);
}

/// A clear with no filter but the person deletes everything of that
/// person's, and no other person's; one of a cache never written is not
/// created.
#[tokio::test]
async fn clearing_everything_deletes_one_persons_entries() {
    let env = TestEnv::new();
    let cache = cache(&env);
    let everything = || Clearing {
        person: Some("ada".to_owned()),
        ..Clearing::default()
    };
    assert_eq!(cache.clear(everything()).await.unwrap(), cleared(0, 0, 0));
    assert!(!folder(&env).exists());
    kept(&cache, &ada(), "https://cache.test/a", "one", "first").await;
    kept(&cache, &ada(), "https://other.test/b", "two", "second").await;
    kept(
        &cache,
        &person("grace"),
        "https://cache.test/a",
        "hers",
        "third",
    )
    .await;

    let answer = cache.clear(everything()).await.unwrap();

    assert_eq!(answer, cleared(2, 2, 6));
    assert_eq!(body_files(&env), [sha256("hers")]);
    assert_eq!(cache.stats("ada").await.unwrap().entries, 0);
    assert!(served(&cache, &person("grace"), "https://cache.test/a").await);
}

/// A verify lists the entries whose body file was changed or is missing,
/// and no intact one; it changes nothing, and one of a cache never written
/// does not create it.
#[tokio::test]
async fn a_verify_reports_a_tampered_body_and_a_missing_one() {
    let env = TestEnv::new();
    let cache = cache(&env);
    assert_eq!(
        cache.verify("ada").await.unwrap(),
        CacheVerified {
            entries: 0,
            bodies: 0,
            damaged: Vec::new(),
        }
    );
    assert!(!folder(&env).exists());
    kept(
        &cache,
        &ada(),
        "https://cache.test/intact",
        "intact",
        "first",
    )
    .await;
    kept(
        &cache,
        &ada(),
        "https://cache.test/tampered",
        "tampered",
        "first",
    )
    .await;
    kept(
        &cache,
        &ada(),
        "https://cache.test/missing",
        "missing",
        "first",
    )
    .await;
    let bodies = folder(&env).join("bodies");
    std::fs::write(bodies.join(sha256("tampered")), "tamperex").unwrap();
    std::fs::remove_file(bodies.join(sha256("missing"))).unwrap();

    let verified = cache.verify("ada").await.unwrap();

    let damage = |path: &str, problem| CacheDamage {
        partition: "public".to_owned(),
        method: "GET".to_owned(),
        url: format!("https://cache.test/{path}"),
        run_id: Some(run("first")),
        body_sha256: sha256(path),
        problem,
    };
    assert_eq!(
        verified,
        CacheVerified {
            entries: 3,
            bodies: 2,
            damaged: vec![
                damage("missing", CacheBodyProblem::Missing),
                damage("tampered", CacheBodyProblem::Changed),
            ],
        }
    );
    assert_eq!(
        std::fs::read_to_string(bodies.join(sha256("tampered"))).unwrap(),
        "tamperex"
    );
}

/// Stats, a clear and a verify each refuse an index of a newer version,
/// and the clear deletes nothing.
/// Covers: story:6-49, format:response-cache
#[tokio::test]
async fn each_refuses_an_index_of_a_newer_version() {
    let env = TestEnv::new();
    kept(&cache(&env), &ada(), "https://cache.test/a", "one", "first").await;
    Connection::open(folder(&env).join("index.sqlite"))
        .unwrap()
        .execute("UPDATE format SET version = ?1", ["0.2"])
        .unwrap();
    let newer = cache(&env);
    let refused = |error| matches!(error, ResponseCacheError::Format { .. });

    assert!(refused(newer.stats("ada").await.unwrap_err()));
    assert!(refused(
        newer
            .clear(Clearing::of_run(&run("first")))
            .await
            .unwrap_err()
    ));
    assert!(refused(newer.verify("ada").await.unwrap_err()));
    assert_eq!(body_files(&env), [sha256("one")]);
}

/// The canaries an entry's URL and body carried are found nowhere in the
/// cache's files once its Run's entries are cleared (spec #379): neither
/// the index, its write-ahead log nor a body file keeps them, while
/// another Run's entry keeps its own.
#[tokio::test]
async fn a_cleared_runs_canaries_stay_nowhere_in_the_cache() {
    let env = TestEnv::new();
    let mut canaries = Canaries::new(env.seed());
    let in_url = canaries.plant(SecretKind::EndpointToken).value().to_owned();
    let in_body = canaries.plant(SecretKind::TypedPassword).value().to_owned();
    let kept_body = canaries.plant(SecretKind::SessionCookie).value().to_owned();
    let query: String = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("token", &in_url)
        .finish();
    let cache = cache(&env);
    let discarded = format!("https://cache.test/page?{query}");
    kept(
        &cache,
        &ada(),
        &discarded,
        &format!("<p>{in_body}</p>"),
        "first",
    )
    .await;
    kept(
        &cache,
        &ada(),
        "https://cache.test/kept",
        &kept_body,
        "second",
    )
    .await;
    assert!(served(&cache, &ada(), &discarded).await);

    cache.clear(Clearing::of_run(&run("first"))).await.unwrap();

    let found: Vec<&str> = canaries
        .search(&[Output::Path(&folder(&env))])
        .unwrap()
        .into_iter()
        .map(|hit| hit.kind.name())
        .collect();
    assert!(
        found
            .iter()
            .all(|kind| *kind == SecretKind::SessionCookie.name())
            && !found.is_empty(),
        "{found:?}"
    );
}
