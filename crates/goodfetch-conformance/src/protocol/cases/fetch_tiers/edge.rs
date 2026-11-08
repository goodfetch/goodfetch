//! The fetch edge cases of the `http` tier (ADR 0118; spec #579), each a
//! crawl under `fetch.tier: http` of the Proving ground's `fetch-edge.test`
//! pages over plain HTTP/1.1, the harness its handler: a cut body fails its
//! attempt with `response_truncated`, is retried, backs no domain off and
//! never reaches the handler (pitfall H15); a body its connection's close
//! delimits reaches it whole; and a request the site never processed is
//! sent once more on a new connection, counted as no retry and no
//! Politeness start (pitfall H05); and requests a site's `Retry-After`
//! past `politeness.retry_after.max` held end `failed` with
//! `response_unavailable`, which `run.retry_failed` re-admits once the site
//! is back, in the same Run and Dataset (ADR 0102). No Response cache or Response recording
//! exists yet, so "never cached" and the `WARC-Truncated` record join these
//! cases with them (#793, #1291).

use std::time::Duration;

use goodfetch_protocol::catalog::ErrorGroup;
use goodfetch_protocol::jsonrpc::ErrorData;
use serde_json::{Map, Value, json};

use super::{Checks, Connection, Gone, Site};
use crate::protocol::foxhound::{
    CRAWL_REQUEST_ENQUEUE, CRAWL_REQUEST_HANDLE, CRAWL_START, RUN_FAILED_REQUESTS, RUN_OUTCOMES,
    RUN_RETRY_FAILED,
};
use crate::proving_ground::ObservedRequest;
use goodfetch_protocol::page::PAGE_FORMATS;
use goodfetch_protocol::page::RESPONSE_TRUNCATED;
use goodfetch_protocol::run::RUN_FINISH;

/// The fetch edge pages' host.
const HOST: &str = "fetch-edge.test";

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
    settings: Value,
    checks: &mut Checks,
) -> Result<Option<Started>, Gone> {
    start_declaring(connection, site, (run_id, &[]), paths, settings, checks)
}

/// [`start`], the crawl declaring the Datasets `datasets` beside `run_id`.
fn start_declaring(
    connection: &mut Connection,
    site: &Site,
    (run_id, datasets): (&str, &[&str]),
    paths: &[&str],
    mut settings: Value,
    checks: &mut Checks,
) -> Result<Option<Started>, Gone> {
    let urls: Vec<String> = paths
        .iter()
        .map(|path| site.named_url(HOST, path))
        .collect();
    settings["fetch"] = json!({ "tier": "http" });
    let settings = site.mapped(&settings, connection.acts_for_model());
    let seeds: Vec<Value> = urls
        .iter()
        .map(|url| json!({ "url": url, "label": "page" }))
        .collect();
    let mut params = json!({ "run_id": run_id, "seeds": seeds, "settings": settings });
    if !datasets.is_empty() {
        params["datasets"] = datasets
            .iter()
            .map(|name| json!({ "name": name }))
            .collect();
    }
    let id = connection.request(CRAWL_START, &params)?;
    let answer = connection.answer(id)?;
    checks.check(
        "`crawl.start` starts a new Run",
        answer.message["result"]["started_as"] == json!("new"),
    );
    Ok(Some((urls, answer.others)))
}

/// The requests the Proving ground received for `url`, in arrival order.
fn requests_for(site: &Site, url: &str) -> Vec<ObservedRequest> {
    let Some(path) = url
        .split_once(HOST)
        .and_then(|(_, rest)| rest.find('/').map(|slash| &rest[slash..]))
    else {
        return Vec::new();
    };
    site.received_requests()
        .of_the_case()
        .filter(|request| request.host.split(':').next() == Some(HOST) && request.target == path)
        .cloned()
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
                    && outcome["error"]["code"] == RESPONSE_TRUNCATED.name
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
            .and_then(|first| first.next_request.as_ref()?["after_microseconds"].as_u64());
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
    let reused_url = site.named_url(HOST, "/reuse/close");
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
        requests.len() == 2 && requests[0].connection != requests[1].connection,
    );
    let gap = requests
        .first()
        .and_then(|first| first.next_request.as_ref()?["after_microseconds"].as_u64());
    checks.check(
        "the resend waits out no domain delay: it is no Politeness start",
        gap.is_some_and(|gap| gap < DOMAIN_DELAY_MS * 1_000 / 2),
    );
    Ok(())
}

/// `/retry-after/503/seconds` answers 503 asking for 2 s, past the crawl's
/// `politeness.retry_after.max` of 1 s: its handler, the harness, fails
/// with `response_unavailable` naming the status and the time asked, as an
/// SDK's handler does, so it ends `failed`, not retried, and
/// `/held/after-503`, admitted while the site's time is still ahead, ends
/// the same way unsent, with `status` null, no handler asked. The engine
/// judges the bound when the site asked, so however long it and the
/// harness take to settle the 503, both fail; the case only needs the 503
/// settled before the site's 2 s passed, after which sending is right
/// (ADR 0118). Once the site
/// is back, `run.retry_failed` re-admits both in the same Run and answers
/// 2: each is fetched and handed to the handler under its ID, commits its
/// one Record in `pages`, and its later `succeeded` outcome follows its
/// `failed` one, nothing listed failed after and no Record committed twice.
pub(super) fn held_past_the_bound(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-fetch-edge-held-past-the-bound";
    let settings = json!({
        "crawl": { "retry": { "max": 1 }, "handler": { "concurrency": 1 } },
        "politeness": {
            "retry_after": { "max": 1_000 },
            "backoff": { "initial": 500, "max": 1_000 },
        },
    });
    let Some((urls, others)) = start_declaring(
        connection,
        site,
        (run_id, &["pages"]),
        &["/retry-after/503/seconds", "/held/after-503"],
        settings.clone(),
        checks,
    )?
    else {
        return Ok(());
    };
    let mut handled = Vec::new();
    let finished = served(connection, others, RUN_FINISH.name, run_id, &mut handled)?;
    checks.check(
        "the Run finishes `incomplete`, its handler asked for the 503 alone",
        finished["result"]["lifecycle"] == json!("finished")
            && finished["result"]["completeness"] == json!("incomplete")
            && handled.len() == 1
            && handled[0].0 == json!(urls[0]),
    );
    let failed = failed_requests(connection, run_id)?;
    let unavailable = |url: &str, status: &Value| {
        failed.iter().any(|outcome| {
            outcome["request"]["url"] == json!(url)
                && outcome["kind"] == json!("failed")
                && outcome["error"]["code"] == json!("response_unavailable")
                && outcome["error"]["details"]["status"] == *status
                && outcome["error"]["details"]["retry_after"].is_string()
                && outcome["attempts"] == json!(1)
        })
    };
    checks.check(
        "the 503 past the bound and the request held meanwhile end `failed` with `response_unavailable`, not retried",
        failed.len() == 2
            && unavailable(&urls[0], &json!(503))
            && unavailable(&urls[1], &Value::Null),
    );
    wait_until_back(&failed);
    let mut settings = settings;
    settings["fetch"] = json!({ "tier": "http" });
    let settings = site.mapped(&settings, connection.acts_for_model());
    let retry = connection.request(
        RUN_RETRY_FAILED,
        &json!({ "run_id": run_id, "settings": settings }),
    )?;
    let retried = connection.answer(retry)?;
    checks.check(
        "`run.retry_failed` re-admits both and answers 2",
        retried.message["result"] == json!({ "readmitted": 2 }),
    );
    handled.clear();
    let refinished = served(
        connection,
        retried.others,
        RUN_FINISH.name,
        run_id,
        &mut handled,
    )?;
    checks.check(
        "the same Run finishes again `complete`, its two Records committed once in `pages`",
        refinished["result"]["lifecycle"] == json!("finished")
            && refinished["result"]["completeness"] == json!("complete")
            && refinished["result"]["records"]
                == json!([{ "dataset": "pages", "committed": 2, "dropped": 0, "rejected": 0 }]),
    );
    let id = connection.request(RUN_OUTCOMES, &json!({ "run_id": run_id }))?;
    let outcomes = connection.answer(id)?.message;
    checks.check(
        "each is handed to its handler under its ID, and its `succeeded` outcome follows its `failed` one",
        superseded(&outcomes, &urls, &handled),
    );
    checks.check(
        "nothing is listed failed after",
        failed_requests(connection, run_id)?.is_empty(),
    );
    Ok(())
}

/// Waits until the latest time any of the `failed` outcomes' sites asked
/// for has passed: the site is back.
#[expect(
    clippy::disallowed_methods,
    reason = "the synchronous harness waits on its own thread, outside any async worker, for the site's wall-clock time"
)]
fn wait_until_back(failed: &[Value]) {
    let asked = failed
        .iter()
        .filter_map(|outcome| outcome["error"]["details"]["retry_after"].as_str())
        .filter_map(unix_seconds_of)
        .max();
    if let Some(asked) = asked {
        let back = std::time::UNIX_EPOCH + Duration::from_secs(asked + 1);
        if let Ok(wait) = back.duration_since(std::time::SystemTime::now()) {
            std::thread::sleep(wait);
        }
    }
}

/// Whether each of `urls` ended `failed` then `succeeded` in `outcomes`
/// under one ID, the one its handler was handed in `handled`.
fn superseded(outcomes: &Value, urls: &[String], handled: &[(Value, Value)]) -> bool {
    let endings = |url: &str| -> Vec<(Value, Value)> {
        outcomes["result"]["outcomes"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|outcome| outcome["request"]["url"] == json!(url))
            .map(|outcome| (outcome["kind"].clone(), outcome["request"]["id"].clone()))
            .collect()
    };
    handled.len() == urls.len()
        && urls.iter().all(|url| {
            let ended = endings(url);
            let given = handled
                .iter()
                .find(|(handed, _)| handed == &json!(url))
                .map(|(_, id)| id.clone());
            ended.len() == 2
                && ended[0].0 == json!("failed")
                && ended[1].0 == json!("succeeded")
                && ended[0].1 == ended[1].1
                && given.as_ref() == Some(&ended[0].1)
        })
}

/// Calls `method` on the Run `run_id` and serves its crawl requests until
/// it answers, `queued` first: a 503 asking for a wait fails its attempt
/// with `response_unavailable` naming the status and the time that wait
/// ends, counted from when the harness was handed it ([`received_at`]), as
/// an SDK's handler does, and any other page commits one Record of `pages`.
/// Each request handed is noted in `handled` with its crawl request's ID.
fn served(
    connection: &mut Connection,
    mut queued: Vec<Value>,
    method: &str,
    run_id: &str,
    handled: &mut Vec<(Value, Value)>,
) -> Result<Value, Gone> {
    let id = connection.request(method, &json!({ "run_id": run_id }))?;
    loop {
        let message = if queued.is_empty() {
            connection.next_within(crate::protocol::target::DEADLINE * 2)?
        } else {
            queued.remove(0)
        };
        if message["id"] == json!(id) && message.get("method").is_none() {
            return Ok(message);
        }
        if message["method"] != json!(CRAWL_REQUEST_HANDLE) {
            continue;
        }
        let params = &message["params"];
        let request = &params["crawl_request"];
        handled.push((request["url"].clone(), request["id"].clone()));
        let response = &params["response"];
        let asked = response["headers"]["retry-after"]
            .as_str()
            .and_then(|seconds| seconds.parse::<u64>().ok());
        match asked {
            Some(seconds) if response["status"] == json!(503) => {
                let details = Map::from_iter([
                    ("status".to_owned(), json!(503)),
                    (
                        "retry_after".to_owned(),
                        json!(rfc3339(received_at() + seconds)),
                    ),
                ]);
                connection.fail_with(
                    &message["id"],
                    ErrorGroup::Page,
                    ErrorData {
                        code: "response_unavailable".to_owned(),
                        retryable: true,
                        details,
                    },
                    "the site is failing for now",
                )?;
            }
            _ => connection.reply(
                &message["id"],
                &json!({ "records": [{ "dataset": "pages", "data": { "url": request["url"] } }] }),
            )?,
        }
    }
}

/// The latest outcome of each request of `run_id` listed failed.
fn failed_requests(connection: &mut Connection, run_id: &str) -> Result<Vec<Value>, Gone> {
    let id = connection.request(RUN_FAILED_REQUESTS, &json!({ "run_id": run_id }))?;
    let answer = connection.answer(id)?.message;
    Ok(answer["result"]["requests"]
        .as_array()
        .cloned()
        .unwrap_or_default())
}

/// Days since 1970-01-01 of a proleptic Gregorian date (Hinnant's
/// `days_from_civil`).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let of_era = year - era * 400;
    let of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let of_cycle = of_era * 365 + of_era / 4 - of_era / 100 + of_year;
    era * 146_097 + of_cycle - 719_468
}

/// The date of `days` since 1970-01-01 (Hinnant's `civil_from_days`).
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let days = days + 719_468;
    let era = days.div_euclid(146_097);
    let of_era = days - era * 146_097;
    let of_cycle = (of_era - of_era / 1_460 + of_era / 36_524 - of_era / 146_096) / 365;
    let of_year = of_era - (365 * of_cycle + of_cycle / 4 - of_cycle / 100);
    let shifted = (5 * of_year + 2) / 153;
    let day = of_year - (153 * shifted + 2) / 5 + 1;
    let month = if shifted < 10 {
        shifted + 3
    } else {
        shifted - 9
    };
    let year = of_cycle + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// Seconds since the Unix epoch of `year`-`month`-`day` at `time`
/// (`HH:MM:SS`).
fn unix_seconds(year: i64, month: i64, day: i64, time: &str) -> Option<u64> {
    let mut parts = time.split(':').map(|part| part.parse::<i64>().ok());
    let (hours, minutes, seconds) = (parts.next()??, parts.next()??, parts.next()??);
    u64::try_from(
        days_from_civil(year, month, day) * 86_400 + hours * 3_600 + minutes * 60 + seconds,
    )
    .ok()
}

/// The harness's clock now, in whole seconds since the Unix epoch rounded
/// up: the moment a handler request it was handed counts a `Retry-After`
/// in seconds from, as a site's response it carries was received no later
/// (RFC 9110, §10.2.3), so the time it names is never sooner than the site
/// asked. The response's `Date` names whole seconds rounded down, which
/// would cut up to a second off the site's wait.
#[expect(
    clippy::disallowed_methods,
    reason = "the harness, a handler, reads its own wall clock to name the time a site asked for"
)]
fn received_at() -> u64 {
    let since = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    since.as_secs() + u64::from(since.subsec_nanos() > 0)
}

/// Seconds since the Unix epoch of an RFC 3339 time in UTC, such as
/// `2026-10-27T00:30:11Z` or with a fraction of a second, which is cut.
fn unix_seconds_of(time: &str) -> Option<u64> {
    let (date, clock) = time.strip_suffix('Z')?.split_once('T')?;
    let mut date = date.split('-').map(|part| part.parse::<i64>().ok());
    let (year, month, day) = (date.next()??, date.next()??, date.next()??);
    unix_seconds(year, month, day, clock.split('.').next()?)
}

/// `seconds` since the Unix epoch as an RFC 3339 time in UTC.
fn rfc3339(seconds: u64) -> String {
    let seconds = i64::try_from(seconds).unwrap_or(i64::MAX);
    let (year, month, day) = civil_from_days(seconds.div_euclid(86_400));
    let of_day = seconds.rem_euclid(86_400);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        of_day / 3_600,
        of_day % 3_600 / 60,
        of_day % 60
    )
}
