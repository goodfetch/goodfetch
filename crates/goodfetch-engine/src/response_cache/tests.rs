//! The Response cache through its interface, on real SQLite and body files
//! in the test's folder: a fresh entry is served with no request from one
//! lookup and one body file, a miss opens none, a stale one is revalidated
//! and a 304 serves its stored body, partitions and variants are kept
//! apart, and an index of a version this engine cannot read is neither
//! read nor written while the engine runs on without it, the generated
//! refusal cases and the `head/` set included.
#![expect(
    clippy::disallowed_methods,
    reason = "the tests read the corpus files and the cache's folder, and relabel its index as a newer engine would"
)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use bytes::Bytes;
use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_protocol::formats::Compatibility;
use goodfetch_protocol::run::RunId;
use goodfetch_test_support::TestEnv;
use rusqlite::Connection;

use super::{CacheRequest, Hit, Lookup, Partition, ResponseCache, ResponseHead};
use crate::tasks::{BlockingPolicy, TaskOwner};

/// ADR 0059's test `Clock` start.
fn start() -> SystemTime {
    goodfetch_protocol::point_in_time::from_rfc3339("2026-01-05T09:00:00Z").unwrap()
}

fn at(seconds: u64) -> SystemTime {
    start() + Duration::from_secs(seconds)
}

fn pairs(headers: &[(&str, &str)]) -> Vec<(String, String)> {
    headers
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn folder(env: &TestEnv) -> PathBuf {
    env.home().join("0.1").join("response-cache")
}

fn cache_with(env: &TestEnv, vfs: Option<String>, work: &CountedWork) -> ResponseCache {
    ResponseCache::of(
        folder(env),
        vfs,
        Arc::new(TaskOwner::root(BlockingPolicy::Inline)),
        work.clone(),
    )
}

fn cache(env: &TestEnv) -> ResponseCache {
    cache_with(env, None, &CountedWork::default())
}

fn ada() -> Partition {
    Partition {
        person: "ada".to_owned(),
        ..Partition::default()
    }
}

fn get(path: &str, headers: &[(&str, &str)]) -> CacheRequest {
    request("GET", path, headers, "")
}

fn request(method: &str, path: &str, headers: &[(&str, &str)], body: &str) -> CacheRequest {
    CacheRequest::new(
        method,
        &format!("https://cache.test{path}").parse().unwrap(),
        &pairs(headers),
        Bytes::from(body.to_owned()),
    )
}

/// A response fresh for a minute, with an `ETag` to revalidate.
fn minute() -> ResponseHead {
    ResponseHead {
        status: 200,
        headers: pairs(&[
            ("content-type", "text/html"),
            ("cache-control", "max-age=60"),
            ("etag", "\"v1\""),
        ]),
    }
}

fn body() -> Bytes {
    Bytes::from_static(b"<p>cached</p>")
}

/// `minute()` stored for `request` at the start, by the Run `first`.
async fn stored(cache: &ResponseCache, request: &CacheRequest) {
    assert!(
        cache
            .store(
                &ada(),
                request,
                &minute(),
                body(),
                Some(&RunId::parse("first").unwrap()),
                start()
            )
            .await
    );
}

fn hit(lookup: Lookup) -> Hit {
    match lookup {
        Lookup::Fresh(hit) => hit,
        other => panic!("not fresh: {other:?}"),
    }
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

fn rows(env: &TestEnv) -> Vec<(String, String)> {
    let connection = Connection::open(folder(env).join("index.sqlite")).unwrap();
    let mut statement = connection
        .prepare("SELECT url, response_headers FROM entries ORDER BY url")
        .unwrap();
    statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// A fresh entry is served with no request: its stored head with an `Age`
/// of its own, its body and the Run that fetched it, from one lookup of
/// the index and one body file opened.
/// Covers: story:6-37, setting:cache.mode
#[tokio::test]
async fn a_fresh_entry_is_served_from_one_lookup_and_one_body_file() {
    let env = TestEnv::new();
    let work = CountedWork::default();
    let cache = cache_with(&env, None, &work);
    let request = get("/page", &[("accept", "text/html")]);
    stored(&cache, &request).await;

    let served = hit(cache.lookup(&ada(), &request, at(30)).await);

    assert_eq!(
        served,
        Hit {
            head: ResponseHead {
                status: 200,
                headers: pairs(&[
                    ("content-type", "text/html"),
                    ("cache-control", "max-age=60"),
                    ("etag", "\"v1\""),
                    ("age", "30"),
                ]),
            },
            body: body(),
            run_id: Some(RunId::parse("first").unwrap()),
        }
    );
    assert_eq!(
        (work.read(Row::CacheLookups), work.read(Row::CacheFileOpens)),
        (1, 1)
    );
}

/// Under `replay`, a form POST's answer is served as stored, no `Age`
/// added, from one lookup and one body file, to the form with its fields
/// in another order and each name's values in theirs; a form whose
/// repeated values come in another order, or another variant of what the
/// answer's `Vary` names, misses with no body file opened.
/// Covers: story:6-36, setting:cache.mode
#[tokio::test]
async fn a_replayed_form_post_is_served_as_stored_for_its_fields_and_variant() {
    let env = TestEnv::new();
    let work = CountedWork::default();
    let cache = cache_with(&env, None, &work);
    let form = |body: &str, language: &str| {
        request(
            "POST",
            "/api/filter",
            &[
                ("content-type", "application/x-www-form-urlencoded"),
                ("accept-language", language),
            ],
            body,
        )
    };
    let answer = ResponseHead {
        status: 201,
        headers: pairs(&[("content-type", "text/html"), ("vary", "Accept-Language")]),
    };
    assert!(
        cache
            .store_replayed(
                &ada(),
                &form("tag=a&q=shoes&tag=b", "en"),
                &answer,
                body(),
                None,
                start()
            )
            .await
    );

    let reordered = cache
        .replayed(&ada(), &form("q=shoes&tag=a&tag=b", "en"))
        .await;
    let counted = (work.read(Row::CacheLookups), work.read(Row::CacheFileOpens));
    let values_swapped = cache
        .replayed(&ada(), &form("q=shoes&tag=b&tag=a", "en"))
        .await;
    let french = cache
        .replayed(&ada(), &form("q=shoes&tag=a&tag=b", "fr"))
        .await;

    assert_eq!(
        reordered,
        Some(Hit {
            head: answer,
            body: body(),
            run_id: None,
        })
    );
    assert_eq!(counted, (1, 1));
    assert_eq!((values_swapped, french), (None, None));
    assert_eq!(
        (work.read(Row::CacheLookups), work.read(Row::CacheFileOpens)),
        (3, 1)
    );
}

/// A miss opens no body file: a URL the cache holds nothing for costs one
/// lookup and nothing more, and a cache never written is not even looked
/// up, nor created.
/// Covers: story:6-37
#[tokio::test]
async fn a_miss_opens_no_body_file() {
    let env = TestEnv::new();
    let work = CountedWork::default();
    let cache = cache_with(&env, None, &work);

    let before = cache.lookup(&ada(), &get("/page", &[]), start()).await;
    let created = folder(&env).exists();
    stored(&cache, &get("/other", &[])).await;
    let missed = cache.lookup(&ada(), &get("/page", &[]), at(1)).await;

    assert_eq!((before, created), (Lookup::Miss, false));
    assert_eq!(missed, Lookup::Miss);
    assert_eq!(
        (work.read(Row::CacheLookups), work.read(Row::CacheFileOpens)),
        (1, 0)
    );
}

/// A stale entry is revalidated with its validators; a 304 serves its
/// stored body with the headers the 304 updates, which keep it fresh from
/// then on, and a 304 for another `ETag` serves nothing.
/// Covers: story:6-37
#[tokio::test]
async fn a_304_answering_a_stale_entry_serves_its_stored_body() {
    let env = TestEnv::new();
    let cache = cache(&env);
    let request = get("/page", &[]);
    stored(&cache, &request).await;

    let Lookup::Stale(stale) = cache.lookup(&ada(), &request, at(120)).await else {
        panic!("not stale");
    };
    let conditions = stale.conditions.clone();
    let other = cache
        .revalidated(
            stale.clone(),
            &request,
            &ResponseHead {
                status: 304,
                headers: pairs(&[("etag", "\"v2\"")]),
            },
            at(121),
        )
        .await;
    let revalidated = cache
        .revalidated(
            stale,
            &request,
            &ResponseHead {
                status: 304,
                headers: pairs(&[("etag", "\"v1\""), ("cache-control", "max-age=600")]),
            },
            at(121),
        )
        .await
        .unwrap();
    let later = hit(cache.lookup(&ada(), &request, at(300)).await);

    assert_eq!(conditions, pairs(&[("if-none-match", "\"v1\"")]));
    assert_eq!(other, None);
    assert_eq!(revalidated.body, body());
    assert_eq!(
        revalidated.head.headers,
        pairs(&[
            ("content-type", "text/html"),
            ("cache-control", "max-age=600"),
            ("etag", "\"v1\""),
            ("age", "0"),
        ])
    );
    assert_eq!(
        (later.body, later.head.headers.last().cloned()),
        (body(), Some(("age".to_owned(), "179".to_owned())))
    );
}

/// An entry answers its own partition only: another person, exit country,
/// language or device class misses, and so does a request with a cookie,
/// which the public partition never answers.
/// Covers: story:6-38
#[tokio::test]
async fn an_entry_answers_its_own_partition_only() {
    let env = TestEnv::new();
    let cache = cache(&env);
    let request = get("/page", &[]);
    stored(&cache, &request).await;
    let others = [
        Partition {
            person: "grace".to_owned(),
            ..ada()
        },
        Partition {
            country: "FR".to_owned(),
            ..ada()
        },
        Partition {
            language: "fr-FR".to_owned(),
            ..ada()
        },
        Partition {
            device: "mobile".to_owned(),
            ..ada()
        },
    ];

    for other in others {
        assert_eq!(
            cache.lookup(&other, &request, at(1)).await,
            Lookup::Miss,
            "{other:?}"
        );
    }
    assert_eq!(
        cache
            .lookup(&ada(), &get("/page", &[("cookie", "session=1")]), at(1))
            .await,
        Lookup::Miss
    );
}

/// A response that varies on `Cookie`, stored for a request with none, is
/// keyed on its absence: a request with a cookie never gets it, and one
/// without does.
/// Covers: story:6-38
#[tokio::test]
async fn a_response_varying_on_cookie_is_keyed_on_it() {
    let env = TestEnv::new();
    let cache = cache(&env);
    let request = get("/page", &[]);
    let varying = ResponseHead {
        status: 200,
        headers: pairs(&[("cache-control", "max-age=60"), ("vary", "Cookie")]),
    };
    assert!(
        cache
            .store(&ada(), &request, &varying, body(), None, start())
            .await
    );

    assert_eq!(
        cache
            .lookup(&ada(), &get("/page", &[("cookie", "a=1")]), at(1))
            .await,
        Lookup::Miss
    );
    assert_eq!(
        hit(cache.lookup(&ada(), &request, at(1)).await).body,
        body()
    );
}

/// One body shared by two entries is stored once, under its SHA-256; an
/// entry whose body file is gone misses.
#[tokio::test]
async fn a_body_is_stored_once_under_its_sha256() {
    let env = TestEnv::new();
    let work = CountedWork::default();
    let cache = cache_with(&env, None, &work);
    stored(&cache, &get("/a", &[])).await;
    stored(&cache, &get("/b", &[])).await;

    let files = body_files(&env);
    std::fs::remove_file(folder(&env).join("bodies").join(&files[0])).unwrap();
    let sha256 = data_encoding::HEXLOWER.encode(&<sha2::Sha256 as sha2::Digest>::digest(body()));

    assert_eq!(files, [sha256]);
    // The bodies' folder made, and the one body file written.
    assert_eq!(work.read(Row::CacheBodyWrites), 2);
    assert_eq!(
        cache.lookup(&ada(), &get("/a", &[]), at(1)).await,
        Lookup::Miss
    );
}

/// The index at `env`'s cache, relabelled `version` as a newer engine
/// would have written it.
fn relabel(env: &TestEnv, version: &str) {
    Connection::open(folder(env).join("index.sqlite"))
        .unwrap()
        .execute("UPDATE format SET version = ?1", [version])
        .unwrap();
}

/// An index of a version this engine cannot read is neither read nor
/// written: every lookup misses, every store is skipped, no body file is
/// written, and the engine runs on without the cache.
/// Covers: story:6-49, format:response-cache
#[tokio::test]
async fn an_index_of_a_newer_version_is_neither_read_nor_written() {
    let env = TestEnv::new();
    stored(&cache(&env), &get("/page", &[])).await;
    relabel(&env, "0.2");
    let (rows_before, bodies_before) = (rows(&env), body_files(&env));
    let work = CountedWork::default();
    let newer = cache_with(&env, None, &work);

    let looked = newer.lookup(&ada(), &get("/page", &[]), at(1)).await;
    let other = ResponseHead {
        status: 200,
        headers: pairs(&[("cache-control", "max-age=60")]),
    };
    let wrote = newer
        .store(
            &ada(),
            &get("/new", &[]),
            &other,
            Bytes::from_static(b"new"),
            None,
            at(1),
        )
        .await;

    assert_eq!(
        (looked, wrote, newer.running_without()),
        (Lookup::Miss, false, true)
    );
    assert_eq!((rows(&env), body_files(&env)), (rows_before, bodies_before));
    assert_eq!(
        (
            work.read(Row::CacheFileOpens),
            work.read(Row::CacheBodyWrites)
        ),
        (0, 0)
    );
}

/// Each generated refusal case of the Response cache (ADR 0066): an index
/// relabelled with the fabricated version is served from when the rule's
/// table makes it compatible, and otherwise run without.
/// Covers: format:response-cache, story:6-49
#[rstest::rstest]
#[tokio::test]
async fn each_generated_refusal_of_the_response_cache_gets_its_verdict(
    #[files("../goodfetch-protocol/generated/refusals/response-cache/*.json")]
    #[mode = str]
    case: &str,
) {
    let case: goodfetch_protocol::refusals::RefusalCase = serde_json::from_str(case).unwrap();
    let env = TestEnv::new();
    let request = get("/page", &[]);
    stored(&cache(&env), &request).await;
    relabel(&env, &case.found.to_string());
    let reader = cache(&env);

    let looked = reader.lookup(&ada(), &request, at(1)).await;

    match case.compatibility {
        Compatibility::Compatible => assert_eq!(hit(looked).body, body(), "{case:#?}"),
        _ => assert_eq!(
            (looked, reader.running_without()),
            (Lookup::Miss, true),
            "{case:#?}"
        ),
    }
}

/// The `head/` set the corpus writer wrote, read by today's engine (ADR
/// 0066): its entries are found in their partitions, stale by now, with
/// their validators, and it takes another.
/// Covers: format:response-cache
#[tokio::test]
async fn the_head_sets_index_opens_and_takes_another_entry() {
    let env = TestEnv::new();
    let head = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../compat/response-cache/head");
    std::fs::create_dir_all(folder(&env)).unwrap();
    Connection::open(folder(&env).join("index.sqlite"))
        .unwrap()
        .execute_batch(&std::fs::read_to_string(head.join("response-cache.sql")).unwrap())
        .unwrap();
    // The set holds the index alone: the replayed entry's body is put back
    // under its SHA-256.
    let body = br#"{"results":[]}"#;
    let bodies = folder(&env).join(super::BODIES);
    std::fs::create_dir_all(&bodies).unwrap();
    std::fs::write(
        bodies.join(data_encoding::HEXLOWER.encode(&<sha2::Sha256 as sha2::Digest>::digest(body))),
        body,
    )
    .unwrap();
    let cache = cache(&env);
    let much_later = at(365 * 24 * 60 * 60);
    let os_user = Partition {
        person: "os_user".to_owned(),
        ..Partition::default()
    };

    let Lookup::Stale(stale) = cache
        .lookup(
            &os_user,
            &get("/revalidate", &[("accept-language", "en")]),
            much_later,
        )
        .await
    else {
        panic!("the head set's entry is not found stale");
    };
    let replayed = cache
        .replayed(
            &os_user,
            &request(
                "POST",
                "/api/search",
                &[("content-type", "application/json")],
                r#"{"page":1,"q":"shoes"}"#,
            ),
        )
        .await;
    stored(&cache, &get("/added", &[])).await;

    assert_eq!(
        stale.conditions,
        pairs(&[
            ("if-none-match", "\"v1\""),
            ("if-modified-since", "Sun, 04 Jan 2026 09:00:00 GMT"),
        ])
    );
    assert_eq!(replayed.map(|hit| hit.body), Some(Bytes::from_static(body)));
    assert_eq!(rows(&env).len(), 4);
}

/// An engine's cache opens its index once for its writes and once for its
/// lookups, however many of each it makes, through the VFS its value
/// names (ADR 0060).
#[tokio::test]
async fn the_index_opens_once_for_its_writes_and_once_for_its_lookups_through_its_vfs() {
    let env = TestEnv::new();
    let vfs = goodfetch_test_support::CountingVfs::register();
    let cache = cache_with(&env, Some(vfs.name().to_owned()), &CountedWork::default());

    stored(&cache, &get("/a", &[])).await;
    stored(&cache, &get("/b", &[])).await;
    for _ in 0..3 {
        hit(cache.lookup(&ada(), &get("/a", &[]), at(1)).await);
    }

    assert_eq!(vfs.opens(), 2);
}
