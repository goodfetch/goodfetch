//! The Response cache cases of the `http` tier (ADR 0046; pitfall H19),
//! each a series of crawls under `fetch.tier: http` of one page of the
//! Proving ground's `cache.test` over plain HTTP/1.1, the harness their
//! handler, on the target's one Goodfetch home, whose cache the crawls
//! share. Each page names which answer of its path it is, so a page
//! handed to the handler tells whether the site sent it or the cache
//! served it, and the Proving ground counts the requests the site saw: with
//! `cache.mode` at its default, `off`, every crawl reaches the site; under
//! `revalidate` a fresh response is served with no request, a stale one is
//! revalidated with `If-None-Match` and `If-Modified-Since` and a `304`
//! serves its stored body, and a response varying on `Cookie` is served
//! only to a request with the cookie it was stored for, while a `POST`
//! reaches the site each time; under `replay` a `POST` the site says never
//! to store is kept, and served again to the same JSON or form body sent
//! in another order.

use serde_json::{Value, json};

use super::{Checks, Connection, Gone, Site};
use crate::protocol::foxhound::{CRAWL_REQUEST_HANDLE, CRAWL_START};
use crate::protocol::target::DEADLINE;
use crate::proving_ground::ObservedRequest;
use goodfetch_protocol::page::PAGE_FORMATS;
use goodfetch_protocol::run::RUN_FINISH;

/// The Response cache's host.
pub(crate) const HOST: &str = "cache.test";

/// When the host's stale pages were last modified.
const LAST_MODIFIED: &str = "Thu, 01 Jan 2026 00:00:00 GMT";

/// The settings of a crawl under `cache.mode`, its default when `None`.
fn settings(mode: Option<&str>) -> Value {
    let mut settings = json!({ "fetch": { "tier": "http" } });
    if let Some(mode) = mode {
        settings["cache"] = json!({ "mode": mode });
    }
    settings
}

/// Crawls `path` on the cache host under `run_id` with `settings` and the
/// Proving ground's names, its handler reading the page's Markdown and
/// answering no Record; the Run finished. The Markdown, empty when no
/// handler was asked.
fn crawl(
    connection: &mut Connection,
    site: &Site,
    run_id: &str,
    path: &str,
    settings: &Value,
    checks: &mut Checks,
) -> Result<String, Gone> {
    let seed = json!({ "url": site.named_url(HOST, path), "label": "page" });
    crawl_seed(connection, site, run_id, &seed, settings, checks)
}

/// Crawls a `POST` of `body` to `path` on the cache host, sent with no
/// `Content-Type`, as [`crawl`] crawls a `GET`.
fn post(
    connection: &mut Connection,
    site: &Site,
    run_id: &str,
    (path, body): (&str, &str),
    settings: &Value,
    checks: &mut Checks,
) -> Result<String, Gone> {
    let seed = json!({
        "url": site.named_url(HOST, path),
        "label": "page",
        "method": "POST",
        "body": data_encoding::BASE64.encode(body.as_bytes()),
    });
    crawl_seed(connection, site, run_id, &seed, settings, checks)
}

/// Crawls `seed` as [`crawl`] crawls a path.
fn crawl_seed(
    connection: &mut Connection,
    site: &Site,
    run_id: &str,
    seed: &Value,
    settings: &Value,
    checks: &mut Checks,
) -> Result<String, Gone> {
    let settings = site.mapped(settings, connection.acts_for_model());
    let id = connection.request(
        CRAWL_START,
        &json!({
            "run_id": run_id,
            "seeds": [seed],
            "labels": { "page": {} },
            "settings": settings,
        }),
    )?;
    let answer = connection.answer(id)?;
    checks.check(
        format!("`crawl.start` starts the new Run `{run_id}`"),
        answer.message["result"]["started_as"] == json!("new"),
    );
    let mut queued = answer.others.into_iter();
    let handle = match queued.next() {
        Some(message) => message,
        None => connection.next_within(DEADLINE)?,
    };
    let asked = handle["method"] == json!(CRAWL_REQUEST_HANDLE);
    checks.check(
        format!("the handler of `{run_id}` is asked for its page"),
        asked,
    );
    let mut markdown = String::new();
    if asked {
        let read = connection.request(
            PAGE_FORMATS.name,
            &json!({ "page_id": handle["params"]["page_id"], "formats": { "markdown": true } }),
        )?;
        let formats = connection.answer(read)?.message;
        formats["result"]["markdown"]
            .as_str()
            .unwrap_or_default()
            .clone_into(&mut markdown);
        connection.reply(&handle["id"], &json!({ "records": [] }))?;
    }
    let finish = connection.request(RUN_FINISH.name, &json!({ "run_id": run_id }))?;
    let finished = connection.answer(finish)?.message;
    checks.check(
        format!("`{run_id}` finishes"),
        finished["result"]["lifecycle"] == json!("finished"),
    );
    Ok(markdown)
}

/// The requests the cache host received for `path`, in arrival order.
fn requests_for(site: &Site, path: &str) -> Vec<ObservedRequest> {
    site.received_requests()
        .every()
        .filter(|request| request.host == HOST && request.target == path)
        .cloned()
        .collect()
}

/// Whether `markdown` is the site's answer `answer` to `path`.
fn is_answer(markdown: &str, answer: u64, path: &str) -> bool {
    markdown.contains(&format!("Answer {answer} of {path}."))
}

/// Under `cache.mode`'s default, `off`, two crawls of a page the site lets
/// be kept an hour both reach the site, and the second is handed its
/// second answer.
pub(super) fn off(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let path = "/fresh/off";
    let unset = settings(None);
    crawl(
        connection,
        site,
        "conformance-cache-off-1",
        path,
        &unset,
        checks,
    )?;
    let second = crawl(
        connection,
        site,
        "conformance-cache-off-2",
        path,
        &unset,
        checks,
    )?;
    checks.check(
        "the second crawl is handed the site's second answer",
        is_answer(&second, 2, path),
    );
    checks.check(
        "the site saw both requests",
        requests_for(site, path).len() == 2,
    );
    Ok(())
}

/// Under `cache.mode: revalidate`, a second crawl of a page the site lets
/// be kept an hour is handed the first answer, and the site sees one
/// request.
pub(super) fn fresh(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let path = "/fresh/kept";
    let revalidate = settings(Some("revalidate"));
    crawl(
        connection,
        site,
        "conformance-cache-fresh-1",
        path,
        &revalidate,
        checks,
    )?;
    let second = crawl(
        connection,
        site,
        "conformance-cache-fresh-2",
        path,
        &revalidate,
        checks,
    )?;
    checks.check(
        "the second crawl is handed the stored first answer",
        is_answer(&second, 1, path),
    );
    checks.check(
        "the site saw one request",
        requests_for(site, path).len() == 1,
    );
    Ok(())
}

/// Under `cache.mode: revalidate`, a second crawl of a page stale at once
/// sends its request with the stored `ETag` in `If-None-Match` and its
/// `Last-Modified` in `If-Modified-Since`; the site answers `304`, and the
/// handler is handed the stored first answer.
pub(super) fn stale(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let path = "/stale/kept";
    let revalidate = settings(Some("revalidate"));
    crawl(
        connection,
        site,
        "conformance-cache-stale-1",
        path,
        &revalidate,
        checks,
    )?;
    let second = crawl(
        connection,
        site,
        "conformance-cache-stale-2",
        path,
        &revalidate,
        checks,
    )?;
    let requests = requests_for(site, path);
    let revalidation = requests.get(1);
    checks.check(
        "the site saw two requests, the first unconditional",
        requests.len() == 2
            && requests[0].header("if-none-match").is_none()
            && requests[0].header("if-modified-since").is_none(),
    );
    checks.check(
        "the second names the stored validators",
        revalidation.is_some_and(|request| {
            request.header("if-none-match") == Some("\"kept-v1\"")
                && request.header("if-modified-since") == Some(LAST_MODIFIED)
        }),
    );
    checks.check(
        "the site answered it `304`",
        revalidation.is_some_and(|request| request.status == Some(304)),
    );
    checks.check(
        "the second crawl is handed the stored first answer",
        is_answer(&second, 1, path),
    );
    Ok(())
}

/// Under `cache.mode: revalidate`, a page varying on `Cookie`, stored for
/// a request with no cookie, is not served to one with a cookie: a crawl
/// whose first hop sets `visitor` reaches the site for the page with it and
/// is handed the site's second answer, naming the cookie. A later crawl
/// with no cookie is handed the stored first answer, and the site saw two
/// requests for the page.
pub(super) fn vary_cookie(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let path = "/vary/keyed";
    let revalidate = settings(Some("revalidate"));
    crawl(
        connection,
        site,
        "conformance-cache-vary-1",
        path,
        &revalidate,
        checks,
    )?;
    let with_cookie = crawl(
        connection,
        site,
        "conformance-cache-vary-2",
        "/cookie/keyed",
        &revalidate,
        checks,
    )?;
    let without = crawl(
        connection,
        site,
        "conformance-cache-vary-3",
        path,
        &revalidate,
        checks,
    )?;
    let requests = requests_for(site, path);
    checks.check(
        "the site saw two requests for the page, the second with the cookie",
        requests.len() == 2
            && requests[0].header("cookie").is_none()
            && requests[1].header("cookie") == Some("visitor=keyed"),
    );
    checks.check(
        "the crawl with the cookie is handed the site's second answer",
        is_answer(&with_cookie, 2, path) && with_cookie.contains("Cookie: visitor=keyed."),
    );
    checks.check(
        "the crawl with no cookie is handed the stored first answer",
        is_answer(&without, 1, path) && without.contains("Cookie: none."),
    );
    Ok(())
}

/// Under `cache.mode: replay`, each crawl `POST`s `bodies` in turn to
/// `path`, a page the site says never to store; the Markdown each crawl is
/// handed, and the requests the site saw.
fn replayed(
    connection: &mut Connection,
    site: &Site,
    run: &str,
    path: &str,
    bodies: &[&str],
    checks: &mut Checks,
) -> Result<(Vec<String>, usize), Gone> {
    let replay = settings(Some("replay"));
    let mut handed = Vec::new();
    for (crawl, body) in bodies.iter().enumerate() {
        let run_id = format!("conformance-cache-{run}-{}", crawl + 1);
        handed.push(post(
            connection,
            site,
            &run_id,
            (path, body),
            &replay,
            checks,
        )?);
    }
    Ok((handed, requests_for(site, path).len()))
}

/// Under `cache.mode: replay`, a `POST` of a JSON body to a page the site
/// says never to store is kept: a later crawl sending the same object with
/// its keys in another order is handed the stored first answer with no
/// request, and one sending another value reaches the site.
pub(super) fn replay_json(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let path = "/api/search";
    let (handed, requests) = replayed(
        connection,
        site,
        "replay-json",
        path,
        &[
            r#"{"q":"shoes","page":1}"#,
            r#"{"page":1,"q":"shoes"}"#,
            r#"{"page":2,"q":"shoes"}"#,
        ],
        checks,
    )?;
    checks.check(
        "the first crawl is handed the site's answer to its `POST`",
        handed.first().is_some_and(|first| {
            is_answer(first, 1, path) && first.contains("POST with 22 bytes.")
        }),
    );
    checks.check(
        "the crawl sending the same object in another order is handed the stored first answer",
        handed
            .get(1)
            .is_some_and(|second| is_answer(second, 1, path)),
    );
    checks.check(
        "the crawl sending another value is handed the site's second answer",
        handed.get(2).is_some_and(|third| is_answer(third, 2, path)),
    );
    checks.check("the site saw two requests", requests == 2);
    Ok(())
}

/// Under `cache.mode: replay`, a form `POST`ed with its fields in another
/// order is handed the stored first answer, a repeated name's values in
/// their order; with those values swapped it is another body, and reaches
/// the site.
pub(super) fn replay_form(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let path = "/api/filter";
    let (handed, requests) = replayed(
        connection,
        site,
        "replay-form",
        path,
        &[
            "tag=a&q=red+shoes&tag=b",
            "q=red+shoes&tag=a&tag=b",
            "tag=b&q=red+shoes&tag=a",
        ],
        checks,
    )?;
    checks.check(
        "the crawl sending the fields in another order is handed the stored first answer",
        handed
            .get(1)
            .is_some_and(|second| is_answer(second, 1, path)),
    );
    checks.check(
        "the crawl sending a name's values in another order is handed the site's second answer",
        handed.get(2).is_some_and(|third| is_answer(third, 2, path)),
    );
    checks.check("the site saw two requests", requests == 2);
    Ok(())
}

/// Under `cache.mode: revalidate`, which stores only `GET` and `HEAD`, two
/// crawls `POST`ing one body both reach the site.
pub(super) fn revalidate_post(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let path = "/api/revalidated";
    let revalidate = settings(Some("revalidate"));
    let body = r#"{"q":"shoes"}"#;
    post(
        connection,
        site,
        "conformance-cache-revalidate-post-1",
        (path, body),
        &revalidate,
        checks,
    )?;
    let second = post(
        connection,
        site,
        "conformance-cache-revalidate-post-2",
        (path, body),
        &revalidate,
        checks,
    )?;
    checks.check(
        "the second crawl is handed the site's second answer",
        is_answer(&second, 2, path),
    );
    checks.check(
        "the site saw both requests",
        requests_for(site, path).len() == 2,
    );
    Ok(())
}
