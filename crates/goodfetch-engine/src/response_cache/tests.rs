//! The Response cache through its interface, on real SQLite and body files
//! in the test's folder: a fresh entry is served with no request from one
//! lookup and one body file, a miss opens none, a stale one is revalidated
//! and a 304 serves its stored body, partitions and variants are kept
//! apart, and an index of a version this engine cannot read is neither
//! read nor written while the engine runs on without it, the generated
//! refusal cases and the `head/` set included. Its bounds too: past
//! `cache.max_size` the least recently used entries leave with their body
//! files, past `cache.max_age_ceiling` an entry is served in no mode, and
//! at a `low` or `critical` storage level nothing new is stored.
#![expect(
    clippy::disallowed_methods,
    reason = "the tests read the corpus files and the cache's folder, and relabel its index as a newer engine would"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use bytes::Bytes;
use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_protocol::formats::Compatibility;
use goodfetch_protocol::run::RunId;
use goodfetch_protocol::storage::StorageLevel;
use goodfetch_test_support::TestEnv;
use rusqlite::Connection;

use super::{Bounds, CacheRequest, Hit, Lookup, Partition, ResponseCache, ResponseHead};
use crate::governor::{Filesystem, Governor, StorageSample};
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
        .replayed(&ada(), &form("q=shoes&tag=a&tag=b", "en"), at(1))
        .await;
    let counted = (work.read(Row::CacheLookups), work.read(Row::CacheFileOpens));
    let values_swapped = cache
        .replayed(&ada(), &form("q=shoes&tag=b&tag=a", "en"), at(1))
        .await;
    let french = cache
        .replayed(&ada(), &form("q=shoes&tag=a&tag=b", "fr"), at(1))
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

const MIB: usize = 1024 * 1024;

/// A body of one MiB of `byte`.
fn mebibyte(byte: u8) -> Bytes {
    Bytes::from(vec![byte; MIB])
}

/// `env`'s cache under `max_size` bytes and `max_age_ceiling`.
fn bounded(env: &TestEnv, max_size: u64, max_age_ceiling: Duration) -> ResponseCache {
    cache(env).bounded(Bounds {
        max_size,
        max_age_ceiling,
    })
}

/// `body` stored under `replay` for `path` at `now`.
async fn replay_stored(cache: &ResponseCache, path: &str, body: Bytes, now: SystemTime) -> bool {
    cache
        .store_replayed(&ada(), &get(path, &[]), &minute(), body, None, now)
        .await
}

/// The byte that the body `replay` serves for `path` at `now`, one made by
/// [`mebibyte`], repeats; none when none is served.
async fn replayed_body(cache: &ResponseCache, path: &str, now: SystemTime) -> Option<u8> {
    let body = cache.replayed(&ada(), &get(path, &[]), now).await?.body;
    assert!(body.len() == MIB && body.iter().all(|byte| *byte == body[0]));
    Some(body[0])
}

fn sha256(body: &[u8]) -> String {
    data_encoding::HEXLOWER.encode(&<sha2::Sha256 as sha2::Digest>::digest(body))
}

fn sorted(mut names: Vec<String>) -> Vec<String> {
    names.sort();
    names
}

/// Past `cache.max_size`, the least recently used entries leave, a hit
/// counting as a use, and their body files with them: of three MiB under
/// a cap of two and a half, the one stored second and never served again
/// leaves.
/// Covers: story:6-42, setting:cache.max_size
#[tokio::test]
async fn past_the_size_cap_the_least_recently_used_entry_leaves_with_its_body_file() {
    let env = TestEnv::new();
    let cache = bounded(&env, (5 * MIB / 2) as u64, Duration::from_secs(3600));
    assert!(replay_stored(&cache, "/a", mebibyte(b'a'), start()).await);
    assert!(replay_stored(&cache, "/b", mebibyte(b'b'), at(1)).await);
    assert_eq!(replayed_body(&cache, "/a", at(2)).await, Some(b'a'));

    assert!(replay_stored(&cache, "/c", mebibyte(b'c'), at(3)).await);

    assert_eq!(
        (
            replayed_body(&cache, "/a", at(4)).await,
            replayed_body(&cache, "/b", at(4)).await,
            replayed_body(&cache, "/c", at(4)).await,
        ),
        (Some(b'a'), None, Some(b'c'))
    );
    assert_eq!(
        body_files(&env),
        sorted(vec![sha256(&mebibyte(b'a')), sha256(&mebibyte(b'c'))])
    );
}

/// A body two entries share counts once toward the cap, and its file
/// leaves only with the last entry holding it.
/// Covers: story:6-42, setting:cache.max_size
#[tokio::test]
async fn a_shared_body_leaves_with_the_last_entry_holding_it() {
    let env = TestEnv::new();
    let cache = bounded(&env, (5 * MIB / 2) as u64, Duration::from_secs(3600));
    assert!(replay_stored(&cache, "/a", mebibyte(b'x'), start()).await);
    assert!(replay_stored(&cache, "/b", mebibyte(b'x'), at(1)).await);
    assert!(replay_stored(&cache, "/c", mebibyte(b'c'), at(2)).await);
    let shared = body_files(&env);

    assert!(replay_stored(&cache, "/d", mebibyte(b'd'), at(3)).await);

    assert_eq!(shared.len(), 2);
    assert_eq!(
        (
            replayed_body(&cache, "/a", at(4)).await,
            replayed_body(&cache, "/b", at(4)).await,
        ),
        (None, None)
    );
    assert_eq!(
        body_files(&env),
        sorted(vec![sha256(&mebibyte(b'c')), sha256(&mebibyte(b'd'))])
    );
}

/// A body larger than the cap is never stored.
/// Covers: story:6-42, setting:cache.max_size
#[tokio::test]
async fn a_body_larger_than_the_cap_is_not_stored() {
    let env = TestEnv::new();
    let cache = bounded(&env, (MIB / 2) as u64, Duration::from_secs(3600));

    assert!(!replay_stored(&cache, "/a", mebibyte(b'a'), start()).await);
    assert_eq!(body_files(&env), Vec::<String>::new());
}

/// Past `cache.max_age_ceiling`, an entry is served in neither mode,
/// however fresh its headers say it is, and the next store deletes it
/// with its body file.
/// Covers: story:6-43, setting:cache.max_age_ceiling
#[tokio::test]
async fn past_the_age_ceiling_an_entry_is_served_in_neither_mode_and_leaves() {
    let env = TestEnv::new();
    let hour = Duration::from_secs(3600);
    let cache = bounded(&env, u64::MAX, hour);
    assert!(replay_stored(&cache, "/replayed", mebibyte(b'r'), start()).await);
    stored(&cache, &get("/stale", &[])).await;
    let within = replayed_body(&cache, "/replayed", start() + hour).await;

    let later = start() + hour + Duration::from_secs(1);
    let replayed = replayed_body(&cache, "/replayed", later).await;
    let revalidated = cache.lookup(&ada(), &get("/stale", &[]), later).await;
    assert!(replay_stored(&cache, "/new", body(), later).await);

    assert_eq!(within, Some(b'r'));
    assert_eq!((replayed, revalidated), (None, Lookup::Miss));
    assert_eq!(
        rows(&env)
            .into_iter()
            .map(|(url, _)| url)
            .collect::<Vec<_>>(),
        ["https://cache.test/new"]
    );
    assert_eq!(body_files(&env), [sha256(&body())]);
}

/// A sample with `folder`'s filesystem at `level`.
fn sample(folder: PathBuf, level: StorageLevel) -> Arc<StorageSample> {
    Arc::new(StorageSample {
        number: 1,
        filesystems: vec![Filesystem {
            folders: vec![folder],
            level,
            free: 0,
            free_inodes: None,
            limit: None,
        }],
    })
}

/// While its filesystem is at `low` or `critical`, the cache stores
/// nothing new and evicts nothing, and still serves what it holds
/// (ADR 0122).
/// Covers: story:6-42
#[rstest::rstest]
#[case::low(StorageLevel::Low)]
#[case::critical(StorageLevel::Critical)]
#[tokio::test]
async fn at_a_low_storage_level_nothing_new_is_stored_and_what_is_held_is_served(
    #[case] level: StorageLevel,
) {
    let env = TestEnv::new();
    let (samples, storage) = tokio::sync::watch::channel(Arc::new(StorageSample::default()));
    let cache = bounded(&env, (3 * MIB / 2) as u64, Duration::from_secs(3600))
        .watching(Governor::unsampled().sample(folder(&env)), storage);
    assert!(cache.storing());
    assert!(replay_stored(&cache, "/a", mebibyte(b'a'), start()).await);
    samples.send_replace(sample(folder(&env), level));

    let stored_while_low = replay_stored(&cache, "/b", mebibyte(b'b'), at(1)).await;

    assert!(!cache.storing());
    assert!(!stored_while_low);
    assert_eq!(replayed_body(&cache, "/a", at(2)).await, Some(b'a'));
    assert_eq!(body_files(&env), [sha256(&mebibyte(b'a'))]);
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
    // Stale by now, and within `cache.max_age_ceiling`'s 30 days.
    let much_later = at(24 * 60 * 60);
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
            much_later,
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
