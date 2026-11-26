//! The Response cache's upkeep cases of the `http` tier (ADRs 0001 and
//! 0046), crawls of `cache.test` under `cache.mode: revalidate` as
//! `cached`'s are, a page's next crawl telling whether the cache still
//! holds it: handed the site's first answer with no request, or reaching
//! the site. `run.discard` deletes the discarded Run's entries and no other
//! Run's; `cache.verify` lists an entry whose body file was changed on
//! disk; and `cache.clear` deletes the entries its filters keep, those of a
//! partition or of a domain, every other entry still served.

use std::path::Path;

use goodfetch_protocol::response_cache::{CACHE_CLEAR, CACHE_STATS, CACHE_VERIFY};
use serde_json::{Value, json};

use super::bounded::body_files;
use super::cached::{HOST, crawl, crawl_seed, is_answer, requests_for, settings};
use super::{Checks, Connection, Gone, Site};
use crate::protocol::cases::consent::walls::major_of;
use crate::protocol::cases::reset::discard as discarded_as;
use crate::protocol::cases::routes::received;

/// Another domain than the cache host, and its page the site sends fresh
/// for an hour, the query naming the case that crawls it.
const OTHER_HOST: &str = "scope.test";
const OTHER_PAGE: &str = "/cached?upkeep=domain-clear";

/// Crawls `path` under `revalidate` as the Run `run_id`, and answers
/// whether it was handed the site's answer `answer`, the site having seen
/// `requests` requests for it.
fn crawled(
    connection: &mut Connection,
    site: &Site,
    (run_id, path): (&str, &str),
    (answer, requests): (u64, usize),
    checks: &mut Checks,
) -> Result<bool, Gone> {
    let markdown = crawl(
        connection,
        site,
        run_id,
        path,
        &settings(Some("revalidate")),
        checks,
    )?;
    Ok(is_answer(&markdown, answer, path) && requests_for(site, path).len() == requests)
}

/// Crawls [`OTHER_PAGE`] under `revalidate` as the Run `run_id`, and
/// answers how many requests for it the site has received.
fn crawled_elsewhere(
    connection: &mut Connection,
    site: &Site,
    run_id: &str,
    checks: &mut Checks,
) -> Result<usize, Gone> {
    let seed = json!({ "url": site.named_url(OTHER_HOST, OTHER_PAGE), "label": "page" });
    let revalidate = settings(Some("revalidate"));
    crawl_seed(connection, site, run_id, &seed, &revalidate, checks)?;
    Ok(received(site, OTHER_HOST, OTHER_PAGE))
}

/// Asks `operation` with `params`, answering its result.
fn asked(connection: &mut Connection, operation: &str, params: &Value) -> Result<Value, Gone> {
    let id = connection.request(operation, params)?;
    Ok(connection.answer(id)?.message["result"].clone())
}

/// Whether `damaged`, a list `cache.verify` answers, names `url` as stored
/// by the Run `run_id` in the public partition, its body `changed`.
fn names_changed(damaged: &Value, url: &str, run_id: &str) -> bool {
    damaged.as_array().is_some_and(|damaged| {
        damaged.iter().any(|entry| {
            entry["url"] == json!(url)
                && entry["run_id"] == json!(run_id)
                && entry["partition"] == json!("public")
                && entry["problem"] == json!("changed")
        })
    })
}

/// Changes, in the body files the Response cache in `major` keeps, the one
/// holding `text`, keeping its length; answers whether one did.
#[expect(
    clippy::disallowed_methods,
    reason = "the harness is synchronous and runs no async worker"
)]
fn tamper(major: &Path, text: &str) -> std::io::Result<bool> {
    for file in body_files(major)? {
        let body = std::fs::read(&file)?;
        let Some(at) = body
            .windows(text.len())
            .position(|window| window == text.as_bytes())
        else {
            continue;
        };
        let mut changed = body;
        changed[at..at + text.len()].fill(b'X');
        std::fs::write(&file, changed)?;
        return Ok(true);
    }
    Ok(false)
}

/// Two Runs each store a page under `revalidate`; once `run.discard`
/// discarded the first, its page's next crawl reaches the site, while the
/// other Run's page is still served.
pub(super) fn discard(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let (gone, kept) = ("/fresh/discarded", "/fresh/kept-past-a-discard");
    let discarded = "conformance-cache-discarded";
    crawled(connection, site, (discarded, gone), (1, 1), checks)?;
    crawled(
        connection,
        site,
        ("conformance-cache-not-discarded", kept),
        (1, 1),
        checks,
    )?;
    discarded_as(connection, discarded, checks)?;
    let again = crawled(
        connection,
        site,
        ("conformance-cache-discarded-after", gone),
        (2, 2),
        checks,
    )?;
    checks.check(
        "the discarded Run's page is no longer served: its next crawl reaches the site",
        again,
    );
    let still = crawled(
        connection,
        site,
        ("conformance-cache-not-discarded-after", kept),
        (1, 1),
        checks,
    )?;
    checks.check("the other Run's page is still served", still);
    Ok(())
}

/// A page stored under `revalidate` has its body file changed on disk,
/// its length kept; `cache.verify` then lists its entry, and it alone, as
/// `changed`. Once `cache.clear` of its Run deleted it, `cache.verify`
/// lists nothing damaged.
pub(super) fn verify(
    connection: &mut Connection,
    home: Option<&str>,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let (run_id, path) = ("conformance-cache-tampered", "/fresh/tampered");
    crawled(connection, site, (run_id, path), (1, 1), checks)?;
    let before = asked(connection, CACHE_VERIFY.name, &json!({}))?;
    checks.check(
        "before any change, `cache.verify` lists no damaged entry",
        before["damaged"] == json!([]),
    );
    let Some(major) = home.and_then(|home| major_of(home, run_id)) else {
        checks.check("the engine's per-major folder is found", false);
        return Ok(());
    };
    let changed = checks.needs(
        "the page's body file is read and changed",
        tamper(&major, &format!("Answer 1 of {path}.")),
    );
    checks.check(
        "the cache holds a body file with the page's text",
        changed == Some(true),
    );
    let after = asked(connection, CACHE_VERIFY.name, &json!({}))?;
    checks.check(
        "`cache.verify` lists the page's entry, its Run and partition named, its body `changed`",
        names_changed(&after["damaged"], &site.named_url(HOST, path), run_id),
    );
    checks.check(
        "`cache.verify` lists no other entry",
        after["damaged"].as_array().map(Vec::len) == Some(1),
    );
    let cleared = asked(connection, CACHE_CLEAR.name, &json!({ "run_id": run_id }))?;
    checks.check(
        "`cache.clear` of the page's Run deletes its one entry and body file",
        cleared["entries"] == json!(1) && cleared["bodies"] == json!(1),
    );
    let last = asked(connection, CACHE_VERIFY.name, &json!({}))?;
    checks.check(
        "once it is cleared, `cache.verify` lists no damaged entry",
        last["damaged"] == json!([]),
    );
    Ok(())
}

/// A page stored under `revalidate` is in the public partition, which
/// `cache.stats` lists. `cache.clear` of a Saved identity's partition, or
/// of the public partition through a proxy exit country, deletes none of
/// it and the page is still served; `cache.clear` of the public partition,
/// narrowed to the page's Run, deletes it, and its next crawl reaches the
/// site.
pub(super) fn partition_clear(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let path = "/fresh/partitioned";
    let stored = "conformance-cache-partition-1";
    crawled(connection, site, (stored, path), (1, 1), checks)?;
    let stats = asked(connection, CACHE_STATS.name, &json!({}))?;
    checks.check(
        "`cache.stats` lists the public partition, with no country, language or device, holding entries",
        stats["partitions"].as_array().is_some_and(|partitions| {
            partitions.iter().any(|partition| {
                partition["partition"] == json!("public")
                    && partition["country"] == json!("")
                    && partition["language"] == json!("")
                    && partition["device"] == json!("")
                    && partition["entries"].as_u64().is_some_and(|entries| entries > 0)
            })
        }),
    );
    for (filters, what) in [
        (
            json!({ "partition": "shop-account" }),
            "a Saved identity's partition",
        ),
        (
            json!({ "partition": "public", "country": "FR" }),
            "the public partition through the exit country `FR`",
        ),
    ] {
        let cleared = asked(connection, CACHE_CLEAR.name, &filters)?;
        checks.check(
            format!("`cache.clear` of {what} deletes no entry"),
            cleared["entries"] == json!(0),
        );
    }
    let still = crawled(
        connection,
        site,
        ("conformance-cache-partition-2", path),
        (1, 1),
        checks,
    )?;
    checks.check("the public partition's page is still served", still);
    let cleared = asked(
        connection,
        CACHE_CLEAR.name,
        &json!({ "partition": "public", "run_id": stored }),
    )?;
    checks.check(
        "`cache.clear` of the public partition, narrowed to the page's Run, deletes its one entry",
        cleared["entries"] == json!(1),
    );
    let again = crawled(
        connection,
        site,
        ("conformance-cache-partition-3", path),
        (2, 2),
        checks,
    )?;
    checks.check(
        "once cleared, the page's next crawl reaches the site",
        again,
    );
    Ok(())
}

/// A page of `cache.test` and one of another domain stored under
/// `revalidate`: `cache.clear` of a domain holding nothing, or of a
/// subdomain of `cache.test`, deletes none of them and the page is still
/// served; `cache.clear` of `cache.test` deletes it, with the host's other
/// entries, and its next crawl reaches the site, while the other domain's
/// page is still served.
pub(super) fn domain_clear(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let path = "/fresh/by-domain";
    crawled(
        connection,
        site,
        ("conformance-cache-domain-1", path),
        (1, 1),
        checks,
    )?;
    let stored = crawled_elsewhere(connection, site, "conformance-cache-domain-other-1", checks)?;
    checks.check(
        format!("the page of `{OTHER_HOST}` reaches its site once"),
        stored == 1,
    );
    let subdomain = format!("sub.{HOST}");
    for domain in ["elsewhere.test", subdomain.as_str()] {
        let cleared = asked(connection, CACHE_CLEAR.name, &json!({ "domain": domain }))?;
        checks.check(
            format!("`cache.clear` of `{domain}` deletes no entry"),
            cleared["entries"] == json!(0),
        );
    }
    let still = crawled(
        connection,
        site,
        ("conformance-cache-domain-2", path),
        (1, 1),
        checks,
    )?;
    checks.check("the page of `cache.test` is still served", still);
    let cleared = asked(connection, CACHE_CLEAR.name, &json!({ "domain": HOST }))?;
    checks.check(
        format!("`cache.clear` of `{HOST}` deletes its entries"),
        cleared["entries"]
            .as_u64()
            .is_some_and(|entries| entries > 0),
    );
    let again = crawled(
        connection,
        site,
        ("conformance-cache-domain-3", path),
        (2, 2),
        checks,
    )?;
    checks.check(
        "once cleared, the page's next crawl reaches the site",
        again,
    );
    let kept = crawled_elsewhere(connection, site, "conformance-cache-domain-other-2", checks)?;
    checks.check(
        format!("the page of `{OTHER_HOST}` is still served: its site saw no new request"),
        kept == 1,
    );
    Ok(())
}
