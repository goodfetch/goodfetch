//! The page suite's Response cache cases (ADR 0046, ticket #796): the
//! browser tier asks the Run's Response cache at step 5 of a page's
//! pipeline, so a later Run's page takes from it what an earlier Run's
//! page stored, with no request to the site. The page is the Proving
//! ground's `cache.test` page `/page/<name>`, never stored itself, which
//! loads the script `/asset/<name>.js` and frames `/fresh/<name>`, both
//! kept an hour; the script writes the number of its answer on the
//! document, so the page tells which answer it ran, and the Proving
//! ground counts the requests the site saw. The service worker line of the
//! same ticket is a service worker case.

use goodfetch_protocol::page::functions::PAGE_EVALUATE;
use goodfetch_protocol::page::session::{RUN_NEW_SESSION, SESSION_CLOSE, SESSION_NEW_PAGE};
use goodfetch_protocol::page::{BLOCKED, PAGE_CHECK_FAILED, PageOpened};
use goodfetch_protocol::response_cache::CACHE_STATS;
use goodfetch_protocol::run::RUN_START;
use serde_json::{Value, json};

use super::fetch_tiers::cached::HOST;
use super::navigation::{answered, call};
use super::proxies::server as proxy_server;
use super::routes::received;
use super::{Checks, Site, finish_run, new_page, run_trace, start_run};
use crate::protocol::connection::{Connection, Gone, code_of};
use crate::protocol::target::EngineTarget;

/// The fetch-edge host serving cut bodies.
const EDGE_HOST: &str = "fetch-edge.test";

/// One Response cache case of the page suite.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Case {
    SubresourcesAreServed,
    PartitionsIsolate,
    ExclusionsNeverCached,
    HitsMarkResults,
}

impl Case {
    /// The case a suite's case ID names.
    pub(crate) fn from_id(id: &str) -> Option<Self> {
        Some(match id {
            "a-later-runs-page-takes-its-script-and-frame-from-the-response-cache" => {
                Self::SubresourcesAreServed
            }
            "partitions-isolate-logged-in-sessions-countries-languages-and-devices" => {
                Self::PartitionsIsolate
            }
            "blocked-failed-page-check-cut-body-and-invalid-cert-responses-are-never-cached" => {
                Self::ExclusionsNeverCached
            }
            "cache-hits-mark-provenance-completeness-usage-and-replay-report" => {
                Self::HitsMarkResults
            }
            _ => return None,
        })
    }

    /// Runs the case on the open Connection against `site`.
    pub(crate) fn run(
        self,
        connection: &mut Connection,
        target: &EngineTarget,
        site: &Site,
        checks: &mut Checks,
    ) -> Result<(), Gone> {
        match self {
            Self::SubresourcesAreServed => subresources_are_served(connection, site, checks),
            Self::PartitionsIsolate => partitions_isolate(connection, site, checks),
            Self::ExclusionsNeverCached => exclusions_never_cached(connection, site, checks),
            Self::HitsMarkResults => hits_mark_results(connection, target, site, checks),
        }
    }
}

/// The settings of these cases' Runs: the Response cache on.
fn settings() -> Value {
    json!({ "cache": { "mode": "revalidate" } })
}

/// Loads `url` on a page of a new Run `run_id` under [`settings`] and
/// finishes the Run: what the page's script wrote, `None` when the page did
/// not open.
fn load(
    connection: &mut Connection,
    site: &Site,
    (run_id, url): (&str, &str),
    checks: &mut Checks,
) -> Result<Option<Value>, Gone> {
    start_run(connection, site, run_id, &settings(), checks)?;
    let Some(page) = new_page(connection, run_id, checks)? else {
        finish_run(connection, run_id)?;
        return Ok(None);
    };
    let page_id = json!(page.page_id);
    let loaded = connection.goto(&page, url)?;
    answered(&loaded, &format!("`{run_id}`'s `goto`"), url, 200, checks);
    let written = call(
        connection,
        PAGE_EVALUATE.name,
        &page_id,
        &json!({ "function": "() => document.documentElement.dataset.answer ?? null" }),
    )?;
    finish_run(connection, run_id)?;
    Ok(Some(written["result"]["value"].clone()))
}

/// Under `cache.mode: revalidate`, a second Run's load of a page the site
/// never lets be stored reaches the site again, while its script and its
/// frame, kept an hour, come from the cache the first Run's load filled:
/// the site saw one request for each, and the script the second page ran
/// is the site's first answer.
fn subresources_are_served(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let name = "browser-served";
    let page = format!("/page/{name}");
    let url = site.named_url(HOST, &page);
    let first = load(
        connection,
        site,
        ("conformance-response-cache-1", &url),
        checks,
    )?;
    checks.check(
        "the first Run's page ran the script's first answer",
        first == Some(json!("1")),
    );
    let second = load(
        connection,
        site,
        ("conformance-response-cache-2", &url),
        checks,
    )?;
    checks.check(
        "the second Run's page ran the script's first answer, from the cache",
        second == Some(json!("1")),
    );
    checks.check(
        "the site saw the page twice, never stored, and its script and its frame once each",
        (
            received(site, HOST, &page),
            received(site, HOST, &format!("/asset/{name}.js")),
            received(site, HOST, &format!("/fresh/{name}")),
        ) == (2, 1, 1),
    );
    Ok(())
}

/// Opens a new Session on `run_id` with `session_settings` and opens one page
/// in it, returning `(session_id, page)`.
fn open_session_page(
    connection: &mut Connection,
    run_id: &str,
    session_settings: &Value,
    label: &str,
    checks: &mut Checks,
) -> Result<Option<(String, PageOpened)>, Gone> {
    let session = connection.call(
        RUN_NEW_SESSION.name,
        &json!({ "run_id": run_id, "settings": session_settings }),
    )?;
    let Some(session_id) = session["result"]["session_id"].as_str().map(str::to_owned) else {
        checks.check(format!("{label}: `run.new_session` opens"), false);
        return Ok(None);
    };
    let opened = connection.call(SESSION_NEW_PAGE.name, &json!({ "session_id": session_id }))?;
    let page = serde_json::from_value::<PageOpened>(opened["result"].clone()).ok();
    checks.check(format!("{label}: `session.new_page` opens"), page.is_some());
    Ok(page.map(|page| (session_id, page)))
}

/// Reads `document.body.innerText` of `page`.
fn body_text(connection: &mut Connection, page: &PageOpened) -> Result<String, Gone> {
    let answer = call(
        connection,
        PAGE_EVALUATE.name,
        &json!(page.page_id),
        &json!({ "function": "() => document.body.innerText" }),
    )?;
    Ok(answer["result"]["value"]
        .as_str()
        .unwrap_or_default()
        .to_owned())
}

/// Closes `session_id`.
fn close_session(connection: &mut Connection, session_id: &str) -> Result<(), Gone> {
    let _ = connection.call(SESSION_CLOSE.name, &json!({ "session_id": session_id }))?;
    Ok(())
}

/// Two Sessions that must not share a Response cache entry at `path`.
struct PartitionPair<'a> {
    path: &'a str,
    first: (&'a str, Value),
    second: (&'a str, Value),
    recheck_first: bool,
    assertion: &'a str,
}

/// Checks that two Sessions with different settings never share an entry at
/// `pair.path` on `HOST`.
fn check_partition_pair(
    connection: &mut Connection,
    site: &Site,
    run_id: &str,
    pair: &PartitionPair<'_>,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let url = site.named_url(HOST, pair.path);
    let (Some((sid_first, page_first)), Some((sid_second, page_second))) = (
        open_session_page(connection, run_id, &pair.first.1, pair.first.0, checks)?,
        open_session_page(connection, run_id, &pair.second.1, pair.second.0, checks)?,
    ) else {
        return Ok(());
    };
    let loaded_first = connection.goto(&page_first, &url)?;
    answered(&loaded_first, pair.first.0, &url, 200, checks);
    let reused_first = if pair.recheck_first {
        let again = connection.goto(&page_first, &url)?;
        answered(
            &again,
            &format!("{} reload", pair.first.0),
            &url,
            200,
            checks,
        );
        body_text(connection, &page_first)?.contains("Answer 1")
    } else {
        true
    };
    let loaded_second = connection.goto(&page_second, &url)?;
    answered(&loaded_second, pair.second.0, &url, 200, checks);
    let second_text = body_text(connection, &page_second)?;
    checks.check(
        pair.assertion,
        reused_first && second_text.contains("Answer 2") && received(site, HOST, pair.path) == 2,
    );
    close_session(connection, &sid_first)?;
    close_session(connection, &sid_second)?;
    Ok(())
}

/// Checks that a `Set-Cookie` response stays inside its Session's own
/// partition, is never served to an anonymous Session, and is forgotten when
/// its Session closes.
fn check_session_cookie_isolation(
    connection: &mut Connection,
    site: &Site,
    run_id: &str,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let cookie_path = "/set-cookie/session-life";
    let cookie_url = site.named_url(HOST, cookie_path);
    let (Some((owner_sid, owner_page)), Some((anon_sid, anon_page))) = (
        open_session_page(connection, run_id, &json!({}), "session A", checks)?,
        open_session_page(connection, run_id, &json!({}), "session B", checks)?,
    ) else {
        return Ok(());
    };
    let owner_first = connection.goto(&owner_page, &cookie_url)?;
    answered(
        &owner_first,
        "session A first load of set-cookie page",
        &cookie_url,
        200,
        checks,
    );
    let first_body = body_text(connection, &owner_page)?;
    let owner_second = connection.goto(&owner_page, &cookie_url)?;
    answered(
        &owner_second,
        "session A second load of set-cookie page",
        &cookie_url,
        200,
        checks,
    );
    let second_body = body_text(connection, &owner_page)?;
    checks.check(
        "session A's second load is served from its own Session partition",
        first_body.contains("Answer 1")
            && second_body.contains("Answer 1")
            && received(site, HOST, cookie_path) == 1,
    );

    let anon_load = connection.goto(&anon_page, &cookie_url)?;
    answered(
        &anon_load,
        "anonymous session B load of set-cookie page",
        &cookie_url,
        200,
        checks,
    );
    let anon_body = body_text(connection, &anon_page)?;
    checks.check(
        "anonymous session B never sees session A's set-cookie entry",
        anon_body.contains("Answer 2") && received(site, HOST, cookie_path) == 2,
    );

    let owner_partition = format!("session:{run_id}:{owner_sid}");
    let has_partition = |stats: &Value| {
        stats["result"]["partitions"]
            .as_array()
            .is_some_and(|parts| {
                parts
                    .iter()
                    .any(|part| part["partition"] == json!(owner_partition))
            })
    };
    let stats_before = connection.call(CACHE_STATS.name, &json!({}))?;
    checks.check(
        "`cache.stats` lists session A's partition while open",
        has_partition(&stats_before),
    );
    close_session(connection, &owner_sid)?;
    let stats_after = connection.call(CACHE_STATS.name, &json!({}))?;
    checks.check(
        "closing session A forgets its Session partition from the Response cache",
        !has_partition(&stats_after),
    );
    close_session(connection, &anon_sid)?;
    Ok(())
}

/// Verifies Response cache partitioning (ADR 0046, #795):
/// - a response setting a cookie is kept for the life of a Session with no
///   Saved identity, never served to another Session, and gone when the
///   Session ends;
/// - two proxy exit countries (`US` vs `DE`), two languages (`en-US` vs
///   `fr-FR`), and two device classes (`desktop` vs `mobile`) never share an
///   entry.
fn partitions_isolate(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let Some(open_proxy) = proxy_server(site, "open", checks) else {
        return Ok(());
    };
    let run_id = "conformance-response-cache-partitions";
    let mapped = site.mapped(&settings(), connection.acts_for_model());
    let started = connection.call(
        RUN_START.name,
        &json!({
            "run_id": run_id,
            "settings": mapped,
            "project": {
                "path": "goodfetch.toml",
                "settings": {
                    "proxy": {
                        "providers": {
                            "list": {
                                "servers": [
                                    { "server": open_proxy, "kind": "datacenter", "country": "US" },
                                    { "server": open_proxy, "kind": "datacenter", "country": "DE" },
                                ]
                            }
                        }
                    }
                }
            }
        }),
    )?;
    checks.check(
        "`run.start` opens the partitioning Run with list proxy providers",
        started["result"]["run_id"] == json!(run_id),
    );
    if started["result"]["run_id"] != json!(run_id) {
        return Ok(());
    }

    check_session_cookie_isolation(connection, site, run_id, checks)?;
    for pair in [
        PartitionPair {
            path: "/fresh/geo-iso",
            first: (
                "US proxy session",
                json!({ "proxy": { "provider": "list", "country": "US" } }),
            ),
            second: (
                "DE proxy session",
                json!({ "proxy": { "provider": "list", "country": "DE" } }),
            ),
            recheck_first: true,
            assertion: "US reuses its cached entry while DE misses and reaches the site",
        },
        PartitionPair {
            path: "/fresh/lang-iso",
            first: ("en-US session", json!({ "browser": { "locale": "en-US" } })),
            second: ("fr-FR session", json!({ "browser": { "locale": "fr-FR" } })),
            recheck_first: false,
            assertion: "two languages never share a cached entry",
        },
        PartitionPair {
            path: "/fresh/dev-iso",
            first: (
                "desktop session",
                json!({ "browser": { "is_mobile": false } }),
            ),
            second: (
                "mobile session",
                json!({ "browser": { "is_mobile": true } }),
            ),
            recheck_first: false,
            assertion: "two device classes never share a cached entry",
        },
    ] {
        check_partition_pair(connection, site, run_id, &pair, checks)?;
    }

    finish_run(connection, run_id)?;
    Ok(())
}

/// Checks that a Block cause and a failed Page check never enter the cache.
fn check_blocked_and_page_check_exclusions(
    connection: &mut Connection,
    site: &Site,
    run_id: &str,
    page: &PageOpened,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let blocked_path = "/blocked/no-cache";
    let blocked_url = site.named_url(HOST, blocked_path);
    let first_block = connection.goto(page, &blocked_url)?;
    if let Some((second_sid, second_page)) = open_session_page(
        connection,
        run_id,
        &json!({}),
        "second session for blocked check",
        checks,
    )? {
        let second_block = connection.goto(&second_page, &blocked_url)?;
        checks.check(
            "a Block cause raises `blocked` both times and is never served from cache",
            code_of(&first_block).as_str() == Some(BLOCKED.name)
                && code_of(&second_block).as_str() == Some(BLOCKED.name)
                && received(site, HOST, blocked_path) == 2,
        );
        close_session(connection, &second_sid)?;
    }

    let check_path = "/fresh/page-check-fail";
    let check_url = site.named_url(HOST, check_path);
    if let Some((checked_sid, checked_page)) = open_session_page(
        connection,
        run_id,
        &json!({
            "session": {
                "page_check": {
                    "expected_page": { "kind": "css", "selector": "#must-exist-never-there" }
                }
            }
        }),
        "checked session",
        checks,
    )? {
        let failed_nav = connection.goto(&checked_page, &check_url)?;
        checks.check(
            "a page failing `session.page_check` raises `page_check_failed`",
            code_of(&failed_nav).as_str() == Some(PAGE_CHECK_FAILED.name)
                && received(site, HOST, check_path) == 1,
        );
        close_session(connection, &checked_sid)?;
        let unchecked = connection.goto(page, &check_url)?;
        answered(
            &unchecked,
            "unchecked session load after failed page check",
            &check_url,
            200,
            checks,
        );
        let text = body_text(connection, page)?;
        checks.check(
            "a response whose Page check failed is never served from cache",
            text.contains("Answer 2") && received(site, HOST, check_path) == 2,
        );
    }
    Ok(())
}

/// Verifies Response cache never-store exclusions under `cache.mode: replay`
/// (ADRs 0046, 0118; #795, #871):
/// - a Block cause (`story:8-65`, `story:6-40`) is never stored or replayed;
/// - a failed Page check (`story:6-40`) is never stored or replayed;
/// - a cut body (`story:6-40`) is never stored or replayed;
/// - a response fetched under `tls.accept_invalid_certs` (`story:6-40`) is
///   never stored or replayed.
fn exclusions_never_cached(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-response-cache-exclusions";
    let replay_settings = json!({
        "cache": { "mode": "replay" },
        "tls": { "accept_invalid_certs": { HOST: null } },
    });
    start_run(connection, site, run_id, &replay_settings, checks)?;
    let Some(page) = new_page(connection, run_id, checks)? else {
        finish_run(connection, run_id)?;
        return Ok(());
    };

    check_blocked_and_page_check_exclusions(connection, site, run_id, &page, checks)?;

    let cut_path = "/cut/content-length";
    let cut_url = site.named_url(EDGE_HOST, cut_path);
    let cut_before = received(site, EDGE_HOST, cut_path);
    let cut_first = connection.goto(&page, &cut_url)?;
    let cut_second = connection.goto(&page, &cut_url)?;
    checks.check(
        "a cut body fails and reaches the site again on the next load",
        cut_first["error"].is_object()
            && cut_second["error"].is_object()
            && received(site, EDGE_HOST, cut_path) == cut_before + 2,
    );

    let tls_path = "/fresh/tls-except";
    let tls_url = site.named_secure_url(HOST, tls_path);
    let tls_first = connection.goto(&page, &tls_url)?;
    answered(
        &tls_first,
        "first HTTPS load under accept_invalid_certs",
        &tls_url,
        200,
        checks,
    );
    if let Some((second_sid, second_page)) = open_session_page(
        connection,
        run_id,
        &json!({}),
        "second session for accept_invalid_certs check",
        checks,
    )? {
        let tls_second = connection.goto(&second_page, &tls_url)?;
        answered(
            &tls_second,
            "second HTTPS load under accept_invalid_certs",
            &tls_url,
            200,
            checks,
        );
        let second_text = body_text(connection, &second_page)?;
        checks.check(
            "a response under `tls.accept_invalid_certs` is never stored or replayed",
            second_text.contains("Answer 2") && received(site, HOST, tls_path) == 2,
        );
        close_session(connection, &second_sid)?;
    }

    finish_run(connection, run_id)?;
    Ok(())
}

/// Under `cache.mode: replay`, a cache hit is recorded in Provenance with its
/// original `fetched_at` and `run_id`, counts as a fetch for Completeness, is
/// counted in Usage (`cache_hits`) apart from `requests`, and the Run report
/// carries `cache_mode: "replay"` (ADRs 0005 and 0046, issue 803).
fn hits_mark_results(
    connection: &mut Connection,
    target: &EngineTarget,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let name = "hits-mark-results";
    let page_path = format!("/page/{name}");
    let url = site.named_url(HOST, &page_path);
    let replay_settings = json!({ "cache": { "mode": "replay" } });

    let run1 = "conformance-cache-results-1";
    start_run(connection, site, run1, &replay_settings, checks)?;
    if let Some(page) = new_page(connection, run1, checks)? {
        let loaded = connection.goto(&page, &url)?;
        answered(&loaded, "run 1 replay load", &url, 200, checks);
    }
    let finished1 = connection.call(
        goodfetch_protocol::run::RUN_FINISH.name,
        &json!({ "run_id": run1 }),
    )?;
    let report1 = &finished1["result"];
    checks.check(
        "first replay Run finishes complete and marks cache_mode replay in its report",
        report1["completeness"] == json!("complete") && report1["cache_mode"] == json!("replay"),
    );

    let hits_before = received(site, HOST, &page_path);
    let run2 = "conformance-cache-results-2";
    start_run(connection, site, run2, &replay_settings, checks)?;
    if let Some(page) = new_page(connection, run2, checks)? {
        let loaded = connection.goto(&page, &url)?;
        answered(&loaded, "run 2 replay cache-hit load", &url, 200, checks);
    }
    let finished2 = connection.call(
        goodfetch_protocol::run::RUN_FINISH.name,
        &json!({ "run_id": run2 }),
    )?;
    let report2 = &finished2["result"];
    let cache_hits = report2["usage"]["cache_hits"].as_u64().unwrap_or(0);
    checks.check(
        "second replay Run serves from cache, stays complete, counts cache_hits in Usage, and marks replay in its report",
        received(site, HOST, &page_path) == hits_before
            && report2["completeness"] == json!("complete")
            && report2["cache_mode"] == json!("replay")
            && cache_hits >= 1,
    );

    let trace = checks.run_trace(run_trace::provenance(target, run2));
    let cache_entry = trace.iter().find(|entry| {
        entry["action"] == json!("response_cache")
            && entry["origin"]["kind"] == json!("response_cache")
            && entry["origin"]["run_id"] == json!(run1)
            && entry["origin"]["fetched_at"]
                .as_str()
                .is_some_and(|ts| goodfetch_protocol::point_in_time::from_rfc3339(ts).is_ok())
    });
    checks.check(
        "Provenance marks the cache hit with its original fetched_at and fetching run_id",
        cache_entry.is_some(),
    );
    Ok(())
}
