//! The Response cache's writer for the Compatibility corpus (ADR 0066):
//! through the engine's own cache, at ADR 0059's test `Clock` start, stores
//! for the OS user a response that varies on `Accept-Language` and carries
//! both validators, fetched outside a Run, and for a person fetching through
//! a French exit with a mobile Identity profile one fetched by a Run, and
//! under `replay` a JSON POST's answer for the OS user; then
//! writes the index, as SQL text, into the folder given as the only
//! argument, as `response-cache.sql`. `xtask compat write` runs it.
#![expect(
    clippy::disallowed_methods,
    reason = "a synchronous corpus writer that writes files (ADR 0066)"
)]

use std::path::PathBuf;
use std::sync::Arc;

use bytes::Bytes;
use goodfetch_counted_work::CountedWork;
use goodfetch_engine::response_cache::{
    CacheRequest, EntryScope, Partition, ResponseCache, ResponseHead,
};
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_protocol::run::RunId;

/// ADR 0059's test `Clock` start.
const START: &str = "2026-01-05T09:00:00Z";

fn pairs(headers: &[(&str, &str)]) -> Vec<(String, String)> {
    headers
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

/// The OS user's partition, with no exit, language or device.
fn os_user() -> Partition {
    Partition {
        person: "os_user".to_owned(),
        ..Partition::default()
    }
}

/// Non-empty approval scope exercised by the `head/` compatibility fixture.
fn example_scope() -> EntryScope {
    EntryScope {
        wall_domain: Some("cache.test".to_owned()),
        wall_refetch: true,
        consent_domain: Some("cache.test".to_owned()),
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .expect("invariant: xtask compat passes the output folder"),
    );
    let scratch = tempfile::tempdir()?;
    let folder = scratch.path().join("response-cache");
    let cache = ResponseCache::of(
        folder.clone(),
        None,
        Arc::new(TaskOwner::root(BlockingPolicy::Inline)),
        CountedWork::default(),
    );
    let start = goodfetch_protocol::point_in_time::from_rfc3339(START)?;
    goodfetch_test_support::runtime().block_on(async {
        let revalidated = cache
            .store(
                &os_user(),
                &CacheRequest::new(
                    "GET",
                    &"https://cache.test/revalidate".parse()?,
                    &pairs(&[("accept-language", "en")]),
                    Bytes::new(),
                ),
                &ResponseHead {
                    status: 200,
                    headers: pairs(&[
                        ("content-type", "text/html"),
                        ("cache-control", "max-age=60"),
                        ("vary", "Accept-Language"),
                        ("etag", "\"v1\""),
                        ("last-modified", "Sun, 04 Jan 2026 09:00:00 GMT"),
                    ]),
                },
                Bytes::from_static(b"<p>revalidate me</p>"),
                None,
                start,
            )
            .await;
        let by_a_run = cache
            .store_scoped(
                &Partition {
                    person: "grace".to_owned(),
                    country: "FR".to_owned(),
                    language: "fr-FR".to_owned(),
                    device: "mobile".to_owned(),
                    ..Partition::default()
                },
                &CacheRequest::new(
                    "GET",
                    &"https://cache.test/fresh".parse()?,
                    &[],
                    Bytes::new(),
                ),
                &ResponseHead {
                    status: 200,
                    headers: pairs(&[("cache-control", "max-age=3600")]),
                },
                Bytes::from_static(b"fresh for an hour"),
                &example_scope(),
                Some(&RunId::parse("filled-the-cache")?),
                start,
            )
            .await;
        let replayed = cache
            .store_replayed(
                &os_user(),
                &CacheRequest::new(
                    "POST",
                    &"https://cache.test/api/search".parse()?,
                    &pairs(&[("content-type", "application/json")]),
                    Bytes::from_static(br#"{"q":"shoes","page":1}"#),
                ),
                &ResponseHead {
                    status: 200,
                    headers: pairs(&[
                        ("content-type", "application/json"),
                        ("cache-control", "no-store"),
                    ]),
                },
                Bytes::from_static(br#"{"results":[]}"#),
                Some(&RunId::parse("filled-the-cache")?),
                start,
            )
            .await;
        if !(revalidated && by_a_run && replayed) {
            return Err("the cache stored not every response".into());
        }
        Ok::<_, Box<dyn std::error::Error>>(())
    })?;
    let connection = rusqlite::Connection::open(folder.join("index.sqlite"))?;
    std::fs::create_dir_all(&output)?;
    std::fs::write(
        output.join("response-cache.sql"),
        goodfetch_test_support::sql_dump(&connection)?,
    )?;
    Ok(())
}
