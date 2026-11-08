//! The fetch edge cases of the `http` tier's redirect chains (ADR 0118;
//! spec #579; pitfalls U12 and U13), each a crawl under `fetch.tier: http`
//! of the Proving ground's `fetch-edge.test` pages over plain HTTP/1.1, the
//! harness its handler: a loop fails with `too_many_redirects` at a cost of
//! `fetch.redirect.max` + 1 requests; each redirect status sends its next
//! hop with Fetch's method and body; a 3xx Fetch does not follow fails with
//! `response_not_ok` unless `crawl.handle_statuses` lists it; each hop goes
//! through the navigation gate (robots.txt, the domain's delay) and the
//! Frontier (a duplicate key, the scope); a hop leaving the scope, to a
//! host the domain policy refuses or to an address the address check
//! refuses ends its request `skipped`, live in `run.outcomes`; a redirect's `Retry-After` delays
//! its next hop, or past the crawl's bound fails the request with
//! `response_unavailable`; and a refresh stub within
//! `fetch.refresh.max_delay` hands the handler its target. A cookie jar,
//! the `browser` tier and `Authorization` on a crawl request join these
//! cases with their tickets.

use serde_json::{Value, json};

use super::{CRAWL_REQUEST_HANDLE, CRAWL_START, Checks, Connection, Gone, Site};
use crate::protocol::target::DEADLINE;
use crate::proving_ground::ObservedRequest;
use goodfetch_protocol::page::PAGE_FORMATS;
use goodfetch_protocol::run::RUN_FINISH;

/// The fetch edge pages' host.
const HOST: &str = "fetch-edge.test";

/// `run.outcomes`, Foxhound's.
const RUN_OUTCOMES: &str = "run.outcomes";

/// `run.add_seed`, Foxhound's.
const RUN_ADD_SEED: &str = "run.add_seed";

/// A domain's delay between request starts, which every hop waits out.
const DOMAIN_DELAY_MS: u64 = 1_500;

/// What a crawl's seeds ended with: each handler request's crawl request
/// URL and the URL of the page it was handed, and every outcome.
struct Crawled {
    handled: Vec<(String, String)>,
    outcomes: Vec<Value>,
}

impl Crawled {
    /// The outcome of the request for `url`.
    fn of(&self, url: &str) -> Option<&Value> {
        self.outcomes
            .iter()
            .find(|outcome| outcome["request"]["url"] == json!(url))
    }

    /// The URL of the page the handler of `url` was handed.
    fn page_of(&self, url: &str) -> Option<&str> {
        self.handled
            .iter()
            .find(|(seed, _)| seed == url)
            .map(|(_, page)| page.as_str())
    }
}

/// A seed of `path` on `host`: a `GET`, or `method` with `body` (Base64).
fn seed(site: &Site, host: &str, path: &str, method: Option<(&str, &str)>) -> Value {
    let mut seed = json!({ "url": site.named_url(host, path), "label": "page" });
    if let Some((method, body)) = method {
        seed["method"] = json!(method);
        seed["body"] = json!(body);
    }
    seed
}

/// Crawls `seeds` under `run_id` with `settings` beside `fetch.tier: http`
/// and the Proving ground's names, `handlers` of them reaching the handler,
/// which reads each page's URL and answers no Record; every outcome read,
/// the Run finished. `None` when the Proving ground serves none of them.
fn crawl(
    connection: &mut Connection,
    site: &Site,
    run_id: &str,
    seeds: &[Value],
    handlers: usize,
    mut settings: Value,
    checks: &mut Checks,
) -> Result<Option<Crawled>, Gone> {
    settings["fetch"]["tier"] = json!("http");
    let settings = site.mapped(&settings, connection.acts_for_model());
    let id = connection.request(
        CRAWL_START,
        &json!({ "run_id": run_id, "seeds": seeds, "settings": settings }),
    )?;
    let answer = connection.answer(id)?;
    checks.check(
        "`crawl.start` starts a new Run",
        answer.message["result"]["started_as"] == json!("new"),
    );
    let mut queued: Vec<Value> = answer.others;
    let mut handled = Vec::new();
    while handled.len() < handlers {
        let message = if queued.is_empty() {
            connection.next_within(DEADLINE * 4)?
        } else {
            queued.remove(0)
        };
        if message["method"] != json!(CRAWL_REQUEST_HANDLE) {
            continue;
        }
        let read = connection.request(
            PAGE_FORMATS.name,
            &json!({ "page_id": message["params"]["page_id"], "formats": { "markdown": true } }),
        )?;
        let formats = connection.answer(read)?;
        queued.extend(formats.others);
        handled.push((
            message["params"]["crawl_request"]["url"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            formats.message["result"]["url"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
        ));
        connection.reply(&message["id"], &json!({ "records": [] }))?;
    }
    let mut outcomes = Vec::new();
    let mut after: Option<u64> = None;
    while outcomes.len() < seeds.len() {
        let mut params = json!({ "run_id": run_id });
        if let Some(position) = after {
            params["after"] = json!(position);
        }
        let id = connection.request(RUN_OUTCOMES, &params)?;
        let answer = connection.answer(id)?;
        queued.extend(answer.others);
        let page = &answer.message["result"];
        let read = page["outcomes"].as_array().cloned().unwrap_or_default();
        if read.is_empty() && page["ended"] == json!(true) {
            break;
        }
        after = page["next"].as_u64();
        outcomes.extend(read);
    }
    checks.check(
        "no other handler is asked",
        !queued
            .iter()
            .any(|message| message["method"] == json!(CRAWL_REQUEST_HANDLE)),
    );
    let finish = connection.request(RUN_FINISH.name, &json!({ "run_id": run_id }))?;
    let finished = connection.answer(finish)?.message;
    checks.check(
        "the Run finishes",
        finished["result"]["lifecycle"] == json!("finished"),
    );
    Ok(Some(Crawled { handled, outcomes }))
}

/// The requests the Proving ground received on `host`, in arrival order,
/// since `serve` started: earlier cases' included.
fn host_requests(site: &Site, host: &str) -> Vec<ObservedRequest> {
    site.received_requests()
        .every()
        .filter(|request| request.host.split(':').next() == Some(host))
        .cloned()
        .collect()
}

/// The requests the Proving ground received on `host` for `path`, in
/// arrival order, since `serve` started: earlier cases' included.
fn requests_for(site: &Site, host: &str, path: &str) -> Vec<ObservedRequest> {
    host_requests(site, host)
        .into_iter()
        .filter(|request| request.target == path)
        .collect()
}

/// Whether `request` carries the header `name`.
fn has_header(request: &ObservedRequest, name: &str) -> bool {
    request.header(name).is_some()
}

/// Whether `outcome` ended `kind` with the error or skip reason `code`
/// after one attempt.
fn ended(outcome: Option<&Value>, kind: &str, code: &str) -> bool {
    outcome.is_some_and(|outcome| {
        outcome["kind"] == json!(kind)
            && (outcome["error"]["code"] == json!(code) || outcome["reason"] == json!(code))
    })
}

/// A hop as the host received it: the request whose answer sent it, the
/// hop's own request, and the host's request just before that one.
#[derive(Debug, PartialEq)]
struct Hop<'a> {
    from: &'a ObservedRequest,
    next: &'a ObservedRequest,
    before: &'a ObservedRequest,
}

impl Hop<'_> {
    /// The time from the arrival of the host's request before the hop to
    /// the hop's, in microseconds: what the domain's delay spaces, since it
    /// counts between request starts (ADR 0086).
    fn after_the_request_before(&self) -> Option<u64> {
        arrived(self.next).checked_sub(arrived(self.before))
    }

    /// The time from the answer that sent the hop to the hop's arrival, in
    /// microseconds: what a `Retry-After` delays, since it counts from the
    /// answer.
    fn after_the_answer(&self) -> Option<u64> {
        arrived(self.next).checked_sub(self.from.timing.answered_microseconds?)
    }
}

/// When `request` arrived, in microseconds since `serve` started.
fn arrived(request: &ObservedRequest) -> u64 {
    request.timing.arrived_microseconds
}

/// The hop the first request for `from` among `requests`, a host's in
/// arrival order, led to: the first request for `to` after it. The host's
/// next request is not always the hop: another seed of the crawl may come
/// between them.
fn hop<'a>(requests: &'a [ObservedRequest], from: &str, to: &str) -> Option<Hop<'a>> {
    let start = requests.iter().position(|request| request.target == from)?;
    let at = start
        + requests[start..]
            .iter()
            .position(|request| request.target == to)?;
    Some(Hop {
        from: &requests[start],
        next: &requests[at],
        before: &requests[at.checked_sub(1)?],
    })
}

/// Under `fetch.redirect.max: 5`, a redirect loop fails with
/// `too_many_redirects`, not retried, after 5 hops and at a cost of 6
/// requests; a chain of 5 redirects reaches its handler.
pub(super) fn redirect_loop(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let seeds: Vec<Value> = [
        seed(site, HOST, "/redirect/loop", None),
        seed(site, HOST, "/redirect/hops/5", None),
    ]
    .into_iter()
    .collect();
    let settings = json!({ "fetch": { "redirect": { "max": 5 } } });
    let Some(crawled) = crawl(
        connection,
        site,
        "conformance-fetch-edge-redirect-loop",
        &seeds,
        1,
        settings,
        checks,
    )?
    else {
        return Ok(());
    };
    let loop_url = site.named_url(HOST, "/redirect/loop");
    let outcome = crawled.of(&loop_url);
    checks.check(
        "the loop ends `failed` with `too_many_redirects`, not retried",
        ended(outcome, "failed", "too_many_redirects")
            && outcome.is_some_and(|outcome| outcome["attempts"] == json!(1)),
    );
    checks.check(
        "its error names the 5 hops and `fetch.redirect.max`",
        outcome.is_some_and(|outcome| {
            let details = &outcome["error"]["details"];
            details["hops"].as_array().map(Vec::len) == Some(5)
                && details["limit"]
                    == json!({ "kind": "setting", "name": "fetch.redirect.max", "count": 5 })
        }),
    );
    checks.check(
        "the loop cost the site `fetch.redirect.max` + 1 requests",
        requests_for(site, HOST, "/redirect/loop").len() == 6,
    );
    let chain = site.named_url(HOST, "/redirect/hops/5");
    checks.check(
        "a chain of 5 redirects hands its handler the page it ends on",
        crawled
            .page_of(&chain)
            .is_some_and(|page| page.ends_with("/redirect/hops/0")),
    );
    Ok(())
}

/// A `POST` answered with 302 reaches its target as a `GET` with no body,
/// and one answered with 308 as a `POST` with its body.
pub(super) fn redirect_methods(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    // "q=1", in Base64.
    let body = Some(("POST", "cT0x"));
    let earlier = requests_for(site, HOST, "/target").len();
    let seeds: Vec<Value> = [
        seed(site, HOST, "/redirect/302", body),
        seed(site, HOST, "/redirect/308", body),
    ]
    .into_iter()
    .collect();
    let Some(crawled) = crawl(
        connection,
        site,
        "conformance-fetch-edge-redirect-methods",
        &seeds,
        2,
        json!({}),
        checks,
    )?
    else {
        return Ok(());
    };
    checks.check(
        "both handlers are handed the target page",
        crawled.handled.len() == 2
            && crawled
                .handled
                .iter()
                .all(|(_, page)| page.ends_with("/target")),
    );
    let target = requests_for(site, HOST, "/target").split_off(earlier);
    let as_get = target
        .iter()
        .filter(|request| request.method == "GET")
        .collect::<Vec<_>>();
    let as_post = target
        .iter()
        .filter(|request| request.method == "POST")
        .collect::<Vec<_>>();
    checks.check(
        "the 302's hop arrives as a `GET` with no body",
        as_get.len() == 1 && !has_header(as_get[0], "content-length"),
    );
    checks.check(
        "the 308's hop arrives as a `POST` with its body",
        as_post.len() == 1 && has_header(as_post[0], "content-length"),
    );
    Ok(())
}

/// A 300, a redirect with no `Location` and one to `ftp:`, `file:`,
/// `gopher:` or `data:` fail at once with `response_not_ok`, so no crawl
/// fetches another scheme, while a 301 `crawl.handle_statuses` lists
/// reaches the handler unfollowed. A listed response is judged by
/// `crawl.content_types` too (ADR 0119): the listed 301 with neither a
/// type nor a body reaches the handler, and the one typed `text/plain`,
/// which the built-in list does not take, ends `skipped` with
/// `content_type` at once, its handler never asked.
pub(super) fn not_followed(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let failing = [
        "/redirect/300",
        "/redirect/no-location",
        "/redirect/ftp",
        "/redirect/file",
        "/redirect/gopher",
        "/redirect/data",
    ];
    let seeds: Vec<Value> = failing
        .iter()
        .chain(&["/redirect/301/untyped", "/redirect/301"])
        .map(|path| seed(site, HOST, path, None))
        .collect();
    let settings = json!({ "crawl": { "handle_statuses": [301] } });
    let earlier = requests_for(site, HOST, "/target").len();
    let Some(crawled) = crawl(
        connection,
        site,
        "conformance-fetch-edge-not-followed",
        &seeds,
        1,
        settings,
        checks,
    )?
    else {
        return Ok(());
    };
    for path in failing {
        let url = site.named_url(HOST, path);
        let outcome = crawled.of(&url);
        checks.check(
            format!("{path} ends `failed` with `response_not_ok` at once"),
            ended(outcome, "failed", "response_not_ok")
                && outcome.is_some_and(|outcome| outcome["attempts"] == json!(1)),
        );
    }
    let listed = site.named_url(HOST, "/redirect/301/untyped");
    checks.check(
        "the listed 301 with neither a type nor a body reaches its handler unfollowed",
        crawled.page_of(&listed) == Some(listed.as_str()),
    );
    let typed = site.named_url(HOST, "/redirect/301");
    let outcome = crawled.of(&typed);
    checks.check(
        "the listed 301 typed `text/plain` ends `skipped` with `content_type` at once",
        ended(outcome, "skipped", "content_type")
            && outcome.is_some_and(|outcome| outcome["attempts"] == json!(1)),
    );
    checks.check(
        "no redirect's target is requested",
        requests_for(site, HOST, "/target").len() == earlier,
    );
    Ok(())
}

/// Each hop goes through the navigation gate: one to a path robots.txt
/// disallows ends the request `skipped` with `robots_txt`, never sent, and
/// a hop waits out the domain's delay after the request before it.
pub(super) fn gated_hops(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let seeds: Vec<Value> = [
        seed(site, HOST, "/redirect/robots", None),
        seed(site, HOST, "/redirect/307", None),
    ]
    .into_iter()
    .collect();
    let settings = json!({ "politeness": { "domain": { "delay": DOMAIN_DELAY_MS } } });
    let Some(crawled) = crawl(
        connection,
        site,
        "conformance-fetch-edge-gated-hops",
        &seeds,
        1,
        settings,
        checks,
    )?
    else {
        return Ok(());
    };
    let robots = site.named_url(HOST, "/redirect/robots");
    checks.check(
        "a hop robots.txt disallows ends its request `skipped` with `robots_txt`",
        ended(crawled.of(&robots), "skipped", "robots_txt"),
    );
    checks.check(
        "the disallowed hop is never sent",
        requests_for(site, HOST, "/private/target").is_empty(),
    );
    // The delay counts between starts, and a start reaches the site a
    // little later each time: arrivals may come up to a tenth of the delay
    // closer than their starts (fix-queue 62).
    checks.check(
        "a hop waits out the domain's delay after the request before it",
        hop(&host_requests(site, HOST), "/redirect/307", "/target")
            .as_ref()
            .and_then(Hop::after_the_request_before)
            .is_some_and(|gap| gap >= DOMAIN_DELAY_MS * 900),
    );
    Ok(())
}

/// A redirect to a page admitted for another request ends `skipped` with
/// `duplicate`, its handler never asked, and one leaving the crawl's scope
/// ends `skipped` with `out_of_scope`, its target never requested.
pub(super) fn frontier_hops(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let seeds: Vec<Value> = [
        seed(site, HOST, "/target", None),
        seed(site, HOST, "/redirect/302", None),
        seed(site, "scope.test", "/moved", None),
    ]
    .into_iter()
    .collect();
    let earlier = requests_for(site, HOST, "/target").len();
    let Some(crawled) = crawl(
        connection,
        site,
        "conformance-fetch-edge-frontier-hops",
        &seeds,
        1,
        json!({}),
        checks,
    )?
    else {
        return Ok(());
    };
    let redirect = site.named_url(HOST, "/redirect/302");
    checks.check(
        "a redirect to a page admitted for another request ends `skipped` with `duplicate`",
        ended(crawled.of(&redirect), "skipped", "duplicate"),
    );
    checks.check(
        "the admitted page is requested once",
        requests_for(site, HOST, "/target").len() == earlier + 1,
    );
    let moved = site.named_url("scope.test", "/moved");
    checks.check(
        "a redirect leaving the scope ends `skipped` with `out_of_scope`",
        ended(crawled.of(&moved), "skipped", "out_of_scope"),
    );
    checks.check(
        "the off-site target is never requested",
        requests_for(site, "elsewhere.test", "/offsite").is_empty(),
    );
    Ok(())
}

/// The redirects a crawl's own checks refuse, under `crawl.scope.strategy:
/// all` narrowed by `crawl.scope.exclude` and `egress.blocked_domains`
/// naming `bücher.test` (ADRs 0017, 0093, 0118 and 0120): one leaving the
/// scope ends `skipped` with `out_of_scope`; one to `bücher.test` spelled
/// in punycode, and one to the Proving ground's loopback address, dotted or
/// in hex, which the address check refuses at that hop, end `skipped` with
/// `domain_policy`; none of their targets is requested, and an in-scope
/// chain reaches its handler (the positive control). The seeds are added
/// to the live crawl after a `run.outcomes` call with `kinds: ["skipped",
/// "blocked"]` and `after: "now"` is made, and each refusal reaches it as
/// it happens with its reason; the Run report counts each reason.
pub(super) fn refused_hops(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-fetch-edge-refused-hops";
    let control = site.named_url(HOST, "/redirect/hops/1");
    let refused: Vec<(String, &str)> = vec![
        (site.named_url("scope.test", "/moved"), "out_of_scope"),
        (site.named_url(HOST, "/redirect/punycode"), "domain_policy"),
        (site.named_url(HOST, "/redirect/address"), "domain_policy"),
        (
            site.named_url(HOST, "/redirect/address/hex"),
            "domain_policy",
        ),
    ];
    let earlier_offsite = requests_for(site, "elsewhere.test", "/offsite").len();
    let earlier_target = requests_for(site, "xn--bcher-kva.test", "/target").len();
    let settings = site.mapped(
        &json!({
            "fetch": { "tier": "http" },
            "crawl": {
                "keep_alive": true,
                "scope": { "strategy": "all", "exclude": ["**/offsite"] },
            },
            "egress": { "blocked_domains": ["bücher.test"] },
        }),
        connection.acts_for_model(),
    );
    handled_page(connection, run_id, &control, &settings, checks)?;
    let urls: Vec<&str> = refused.iter().map(|(url, _)| url.as_str()).collect();
    let outcomes = added_live(connection, run_id, &urls)?;
    for (url, reason) in &refused {
        let outcome = outcomes
            .iter()
            .find(|outcome| outcome["request"]["url"] == json!(url));
        checks.check(
            format!("`run.outcomes` after `now` yields {url} `skipped` with `{reason}`"),
            ended(outcome, "skipped", reason),
        );
    }
    checks.check(
        "`run.outcomes` yields only the refusals",
        outcomes.len() == refused.len(),
    );
    checks.check(
        "no refused hop's target is requested",
        requests_for(site, "elsewhere.test", "/offsite").len() == earlier_offsite
            && requests_for(site, "xn--bcher-kva.test", "/target").len() == earlier_target
            && addressed(site) == 0,
    );
    let finish = connection.request(RUN_FINISH.name, &json!({ "run_id": run_id }))?;
    let report = connection.answer(finish)?.message;
    checks.check(
        "the Run finishes",
        report["result"]["lifecycle"] == json!("finished"),
    );
    let skips = &report["result"]["requests"]["by_skip_reason"];
    checks.check(
        "the Run report counts 3 `domain_policy` and 1 `out_of_scope` skips",
        skips.as_array().is_some_and(|skips| {
            skips.contains(&json!({ "name": "domain_policy", "count": 3 }))
                && skips.contains(&json!({ "name": "out_of_scope", "count": 1 }))
        }),
    );
    Ok(())
}

/// Starts the crawl `run_id` under `settings` with the one seed `url`, an
/// in-scope chain to `/redirect/hops/0`, and answers its handler once it
/// checked the page it was handed is the chain's end.
fn handled_page(
    connection: &mut Connection,
    run_id: &str,
    url: &str,
    settings: &Value,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let control = url;
    let id = connection.request(
        CRAWL_START,
        &json!({
            "run_id": run_id,
            "seeds": [{ "url": control, "label": "page" }],
            "settings": settings,
        }),
    )?;
    let answer = connection.answer(id)?;
    checks.check(
        "`crawl.start` starts a new Run",
        answer.message["result"]["started_as"] == json!("new"),
    );
    let mut queued: Vec<Value> = answer.others;
    let handle = loop {
        let message = if queued.is_empty() {
            connection.next_within(DEADLINE * 4)?
        } else {
            queued.remove(0)
        };
        if message["method"] == json!(CRAWL_REQUEST_HANDLE) {
            break message;
        }
    };
    let read = connection.request(
        PAGE_FORMATS.name,
        &json!({ "page_id": handle["params"]["page_id"], "formats": { "markdown": true } }),
    )?;
    let formats = connection.answer(read)?;
    checks.check(
        "an in-scope chain reaches its handler",
        formats.message["result"]["url"]
            .as_str()
            .is_some_and(|url| url.ends_with("/redirect/hops/0")),
    );
    connection.reply(&handle["id"], &json!({ "records": [] }))?;
    Ok(())
}

/// The outcomes of kinds `skipped` and `blocked` a `run.outcomes` call
/// made with `after: "now"` yields while each of `urls` is added to the
/// live crawl `run_id` as a seed, until it yields as many as `urls`.
fn added_live(
    connection: &mut Connection,
    run_id: &str,
    urls: &[&str],
) -> Result<Vec<Value>, Gone> {
    let live = connection.request(
        RUN_OUTCOMES,
        &json!({ "run_id": run_id, "kinds": ["skipped", "blocked"], "after": "now" }),
    )?;
    for url in urls {
        let add = connection.request(
            RUN_ADD_SEED,
            &json!({ "run_id": run_id, "seed": { "url": url, "label": "page" } }),
        )?;
        connection.answer(add)?;
    }
    let mut outcomes: Vec<Value> = Vec::new();
    let mut page = connection.answer(live)?.message;
    loop {
        let read = page["result"]["outcomes"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        outcomes.extend(read);
        if outcomes.len() >= urls.len() || page["result"]["ended"] == json!(true) {
            return Ok(outcomes);
        }
        let next = connection.request(
            RUN_OUTCOMES,
            &json!({
                "run_id": run_id,
                "kinds": ["skipped", "blocked"],
                "after": page["result"]["next"],
            }),
        )?;
        page = connection.answer(next)?.message;
    }
}

/// The requests the Proving ground received for `/address/target`, on any
/// host.
fn addressed(site: &Site) -> usize {
    site.received_requests()
        .of_the_case()
        .filter(|request| request.target == "/address/target")
        .count()
}

/// A 302's `Retry-After` of 2 s delays its next hop by 2 s; one asking for
/// an hour, past the crawl's bound, ends the request `failed` with
/// `response_unavailable` carrying the 302, not retried.
pub(super) fn redirect_retry_after(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let seeds: Vec<Value> = [
        seed(site, HOST, "/retry-after/302/seconds", None),
        seed(site, HOST, "/retry-after/302/imf-fixdate/long", None),
    ]
    .into_iter()
    .collect();
    let settings = json!({
        "politeness": { "retry_after": { "max": 3_000 }, "backoff": { "max": 3_000 } },
    });
    let Some(crawled) = crawl(
        connection,
        site,
        "conformance-fetch-edge-redirect-retry-after",
        &seeds,
        1,
        settings,
        checks,
    )?
    else {
        return Ok(());
    };
    let waited = site.named_url(HOST, "/retry-after/302/seconds");
    checks.check(
        "the 302's handler is handed its target",
        crawled
            .page_of(&waited)
            .is_some_and(|page| page.ends_with("/target")),
    );
    checks.check(
        "its next hop waits the 2 s its `Retry-After` asks",
        hop(
            &host_requests(site, HOST),
            "/retry-after/302/seconds",
            "/target",
        )
        .as_ref()
        .and_then(Hop::after_the_answer)
        .is_some_and(|gap| gap >= 1_800_000),
    );
    let long = site.named_url(HOST, "/retry-after/302/imf-fixdate/long");
    let outcome = crawled.of(&long);
    checks.check(
        "one past the bound ends `failed` with `response_unavailable`, not retried",
        ended(outcome, "failed", "response_unavailable")
            && outcome.is_some_and(|outcome| outcome["attempts"] == json!(1)),
    );
    checks.check(
        "its error carries the 302 and the time asked",
        outcome.is_some_and(|outcome| {
            let details = &outcome["error"]["details"];
            details["status"] == json!(302) && details["retry_after"].is_string()
        }),
    );
    Ok(())
}

/// At the default `fetch.refresh.max_delay`, a refresh stub of 5 s hands
/// the handler its target at once, through the gate, while one of 200 s,
/// one with no URL, one to itself and one inside `<noscript>` hand it the
/// stub; at `null` a 200 s refresh is followed, and at `0` a 5 s one is
/// not while an instant one is.
pub(super) fn refresh_stubs(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let stays = [
        "/refresh/meta/200",
        "/refresh/no-url",
        "/refresh/self",
        "/refresh/noscript",
    ];
    let runs: [(&str, Value, &[&str], &[&str]); 3] = [
        (
            "default",
            json!({ "politeness": { "domain": { "delay": DOMAIN_DELAY_MS } } }),
            &["/refresh/meta/5"],
            &stays,
        ),
        (
            "any-delay",
            json!({ "fetch": { "refresh": { "max_delay": null } } }),
            &["/refresh/meta/200"],
            &[],
        ),
        (
            "instant-only",
            json!({ "fetch": { "refresh": { "max_delay": 0 } } }),
            &["/refresh/meta/0"],
            &["/refresh/meta/5"],
        ),
    ];
    for (name, settings, followed, unfollowed) in runs {
        let seeds: Vec<Value> = followed
            .iter()
            .chain(unfollowed)
            .map(|path| seed(site, HOST, path, None))
            .collect();
        let Some(crawled) = crawl(
            connection,
            site,
            &format!("conformance-fetch-edge-refresh-{name}"),
            &seeds,
            followed.len() + unfollowed.len(),
            settings,
            checks,
        )?
        else {
            return Ok(());
        };
        for path in followed {
            let url = site.named_url(HOST, path);
            checks.check(
                format!("under `{name}`, {path}'s handler is handed its target"),
                crawled
                    .page_of(&url)
                    .is_some_and(|page| page.ends_with("/target")),
            );
        }
        for path in unfollowed {
            let url = site.named_url(HOST, path);
            checks.check(
                format!("under `{name}`, {path}'s handler is handed the stub"),
                crawled.page_of(&url) == Some(url.as_str()),
            );
        }
    }
    checks.check(
        "a refresh hop waits out the domain's delay, through the gate",
        hop(&host_requests(site, HOST), "/refresh/meta/5", "/target")
            .as_ref()
            .and_then(Hop::after_the_request_before)
            .is_some_and(|gap| gap >= DOMAIN_DELAY_MS * 900),
    );
    Ok(())
}

#[cfg(test)]
mod tests;
