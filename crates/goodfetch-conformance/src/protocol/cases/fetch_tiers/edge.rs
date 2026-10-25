//! The fetch edge cases of the `http` tier (ADR 0118; spec #579), each a
//! crawl under `fetch.tier: http` of the Proving ground's `fetch-edge.test`
//! pages over plain HTTP/1.1, the harness its handler: a cut body fails its
//! attempt with `response_truncated`, is retried, backs no domain off and
//! never reaches the handler (pitfall H15); a body its connection's close
//! delimits reaches it whole; and a request the site never processed is
//! sent once more on a new connection, counted as no retry and no
//! Politeness start (pitfall H05). No Response cache or Response recording
//! exists yet, so "never cached" and the `WARC-Truncated` record join these
//! cases with them (#793, #1291).

use serde_json::{Value, json};

use super::{CRAWL_REQUEST_HANDLE, CRAWL_START, Checks, Connection, Gone, Site};
use goodfetch_protocol::page::PAGE_FORMATS;
use goodfetch_protocol::run::RUN_FINISH;

/// The fetch edge pages' host.
const HOST: &str = "fetch-edge.test";

/// `run.outcomes`, Foxhound's.
const RUN_OUTCOMES: &str = "run.outcomes";

/// `crawl_request.enqueue`, Foxhound's.
const CRAWL_REQUEST_ENQUEUE: &str = "crawl_request.enqueue";

/// The cut pages over HTTP/1.1: a body shorter than its `Content-Length`,
/// a chunked one missing its last chunk, a gzip stream ending early in a
/// whole transfer, and one whose transfer ends early.
const CUT: [&str; 4] = [
    "/cut/content-length",
    "/cut/chunked",
    "/cut/gzip",
    "/cut/gzip-content-length",
];

/// A domain's first backoff wait set far above any retry's own delay, so a
/// retry that came within half of it did not wait out a backoff.
const BACKOFF_INITIAL_MS: u64 = 30_000;

/// A domain's delay between request starts, which a resend counted as a
/// Politeness start would wait out.
const DOMAIN_DELAY_MS: u64 = 3_000;

/// A crawl just started: its seeds' URLs, and every message the engine
/// sent before the start's answer.
type Started = (Vec<String>, Vec<Value>);

/// Starts a crawl of `paths` on the fetch edge host under `run_id` with
/// `settings` beside `fetch.tier: http` and the Proving ground's names;
/// `None` when the Proving ground serves none of them.
fn start(
    connection: &mut Connection,
    site: &Site,
    run_id: &str,
    paths: &[&str],
    mut settings: Value,
    checks: &mut Checks,
) -> Result<Option<Started>, Gone> {
    let urls: Option<Vec<String>> = paths
        .iter()
        .map(|path| site.named_url(HOST, path))
        .collect();
    checks.check(
        "the Proving ground serves the fetch edge pages",
        urls.is_some(),
    );
    let Some(urls) = urls else {
        return Ok(None);
    };
    settings["fetch"] = json!({ "tier": "http" });
    let settings = site.mapped(&settings, connection.acts_for_model());
    let seeds: Vec<Value> = urls
        .iter()
        .map(|url| json!({ "url": url, "label": "page" }))
        .collect();
    let id = connection.request(
        CRAWL_START,
        &json!({ "run_id": run_id, "seeds": seeds, "settings": settings }),
    )?;
    let answer = connection.answer(id)?;
    checks.check(
        "`crawl.start` starts a new Run",
        answer.message["result"]["started_as"] == json!("new"),
    );
    Ok(Some((urls, answer.others)))
}

/// The requests the Proving ground received for `url`, in arrival order.
fn requests_for(site: &Site, url: &str) -> Vec<Value> {
    let Some(path) = url
        .split_once(HOST)
        .and_then(|(_, rest)| rest.find('/').map(|slash| &rest[slash..]))
    else {
        return Vec::new();
    };
    site.proving_ground
        .observations()
        .ok()
        .and_then(|observations| observations["requests"].as_array().cloned())
        .unwrap_or_default()
        .into_iter()
        .filter(|request| {
            request["host"]
                .as_str()
                .is_some_and(|host| host.split(':').next() == Some(HOST))
                && request["target"] == json!(path)
        })
        .collect()
}

/// Each cut page fails both attempts `crawl.retry.max: 1` allows with
/// `response_truncated`, ends `failed`, its retry sent with no backoff
/// waited; no handler request is ever sent.
pub(super) fn cut_bodies(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-fetch-edge-cut-bodies";
    let settings = json!({
        "crawl": { "retry": { "max": 1 } },
        "politeness": { "backoff": { "initial": BACKOFF_INITIAL_MS } },
    });
    let Some((urls, mut seen)) = start(connection, site, run_id, &CUT, settings, checks)? else {
        return Ok(());
    };
    let mut outcomes = Vec::new();
    let mut after: Option<u64> = None;
    while outcomes.len() < urls.len() {
        let mut params = json!({ "run_id": run_id, "kinds": ["failed"] });
        if let Some(position) = after {
            params["after"] = json!(position);
        }
        let id = connection.request(RUN_OUTCOMES, &params)?;
        let answer = connection.answer(id)?;
        seen.extend(answer.others);
        let page = &answer.message["result"];
        let read = page["outcomes"].as_array().cloned().unwrap_or_default();
        if read.is_empty() && page["ended"] == json!(true) {
            break;
        }
        after = page["next"].as_u64();
        outcomes.extend(read);
    }
    let finish = connection.request(RUN_FINISH.name, &json!({ "run_id": run_id }))?;
    seen.extend(connection.answer(finish)?.others);
    for url in &urls {
        let outcome = outcomes
            .iter()
            .find(|outcome| outcome["request"]["url"] == json!(url));
        let errors: Vec<&Value> = outcome
            .and_then(|outcome| outcome["errors"].as_array())
            .map(|errors| errors.iter().map(|error| &error["code"]).collect())
            .unwrap_or_default();
        checks.check(
            format!("{url} ends `failed` with `response_truncated`"),
            outcome.is_some_and(|outcome| {
                outcome["kind"] == json!("failed")
                    && outcome["error"]["code"] == json!("response_truncated")
            }),
        );
        checks.check(
            format!("{url} is retried once, each attempt cut"),
            outcome.is_some_and(|outcome| outcome["attempts"] == json!(2))
                && errors == [&json!("response_truncated"); 2],
        );
        let requests = requests_for(site, url);
        let gap = requests
            .first()
            .and_then(|first| first["next_request"]["after_microseconds"].as_u64());
        checks.check(
            format!("{url}'s retry waits out no backoff of its domain"),
            requests.len() == 2 && gap.is_some_and(|gap| gap < BACKOFF_INITIAL_MS * 1_000 / 2),
        );
    }
    checks.check(
        "no handler is asked to handle a cut body",
        !seen
            .iter()
            .any(|message| message["method"] == json!(CRAWL_REQUEST_HANDLE)),
    );
    Ok(())
}

/// The handler is asked to handle the page whose body its connection's
/// close ends, and reads it whole.
pub(super) fn close_delimited(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-fetch-edge-close-delimited";
    let Some((urls, others)) = start(
        connection,
        site,
        run_id,
        &["/close-delimited"],
        json!({}),
        checks,
    )?
    else {
        return Ok(());
    };
    let handle = match others.into_iter().next() {
        Some(message) => message,
        None => connection.next_within(crate::protocol::target::DEADLINE)?,
    };
    checks.check(
        "the handler is asked to handle the page",
        handle["method"] == json!(CRAWL_REQUEST_HANDLE)
            && handle["params"]["crawl_request"]["url"] == json!(urls[0]),
    );
    let read = connection.request(
        PAGE_FORMATS.name,
        &json!({ "page_id": handle["params"]["page_id"], "formats": { "markdown": true } }),
    )?;
    let formats = connection.answer(read)?.message;
    let markdown = formats["result"]["markdown"].as_str().unwrap_or_default();
    checks.check(
        "its page holds the whole body, to its last line",
        markdown.contains("Line 40 of the fetch edge page at /close-delimited."),
    );
    connection.reply(&handle["id"], &json!({ "records": [] }))?;
    let finish = connection.request(RUN_FINISH.name, &json!({ "run_id": run_id }))?;
    let finished = connection.answer(finish)?.message;
    checks.check(
        "the Run finishes",
        finished["result"]["lifecycle"] == json!("finished"),
    );
    Ok(())
}

/// The handler of `/target` enqueues `/reuse/close`, which the HTTP tier
/// sends on the connection `/target` left idle; the site closes it unread,
/// so the tier sends it once more on a new connection, at once, though the
/// domain's delay is 3 s and `crawl.retry.max` is 0, and its handler is
/// asked once, for its first attempt.
pub(super) fn resent(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-fetch-edge-resend";
    let settings = json!({
        "crawl": { "retry": { "max": 0 } },
        "politeness": { "domain": { "delay": DOMAIN_DELAY_MS } },
    });
    let Some((urls, others)) = start(connection, site, run_id, &["/target"], settings, checks)?
    else {
        return Ok(());
    };
    let Some(reused_url) = site.named_url(HOST, "/reuse/close") else {
        return Ok(());
    };
    let first = match others.into_iter().next() {
        Some(message) => message,
        None => connection.next_within(crate::protocol::target::DEADLINE)?,
    };
    checks.check(
        "the handler is asked to handle the first page",
        first["method"] == json!(CRAWL_REQUEST_HANDLE)
            && first["params"]["crawl_request"]["url"] == json!(urls[0]),
    );
    let params = &first["params"];
    let enqueue = connection.request(
        CRAWL_REQUEST_ENQUEUE,
        &json!({
            "run_id": run_id,
            "crawl_request_id": params["crawl_request"]["id"],
            "lease_epoch": params["lease_epoch"],
            "url": reused_url,
            "label": "page",
        }),
    )?;
    let enqueued = connection.answer(enqueue)?;
    checks.check(
        "the handler enqueues the page the site closes its reused connection for",
        enqueued.message.get("result").is_some(),
    );
    connection.reply(&first["id"], &json!({ "records": [] }))?;
    let second = match enqueued
        .others
        .into_iter()
        .find(|message| message["method"] == json!(CRAWL_REQUEST_HANDLE))
    {
        Some(message) => message,
        None => connection.next_within(crate::protocol::target::DEADLINE * 2)?,
    };
    checks.check(
        "its handler is asked once, for its first attempt",
        second["method"] == json!(CRAWL_REQUEST_HANDLE)
            && second["params"]["crawl_request"]["url"] == json!(reused_url)
            && second["params"]["attempt"] == json!(1),
    );
    connection.reply(&second["id"], &json!({ "records": [] }))?;
    let finish = connection.request(RUN_FINISH.name, &json!({ "run_id": run_id }))?;
    let finished = connection.answer(finish)?.message;
    checks.check(
        "the Run finishes",
        finished["result"]["lifecycle"] == json!("finished"),
    );
    let requests = requests_for(site, &reused_url);
    checks.check(
        "the site saw the page twice, on two connections",
        requests.len() == 2 && requests[0]["connection"] != requests[1]["connection"],
    );
    let gap = requests
        .first()
        .and_then(|first| first["next_request"]["after_microseconds"].as_u64());
    checks.check(
        "the resend waits out no domain delay: it is no Politeness start",
        gap.is_some_and(|gap| gap < DOMAIN_DELAY_MS * 1_000 / 2),
    );
    Ok(())
}
