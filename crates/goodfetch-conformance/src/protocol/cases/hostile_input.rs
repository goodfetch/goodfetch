//! The page suite's hostile-input cases (ADR 0067, SC-05, ticket #644): a
//! header value holding a CR, an LF or a NUL, which `page.request` and a
//! route's answers refuse with `header_value_invalid` before anything
//! reaches the browser, judged by the Proving ground's record of what it
//! received; and a body past the Run's limits on what the engine reads,
//! which `page.request` refuses with `response_limit_exceeded` naming the
//! setting: a decompression bomb and a body one byte past 64 MiB; and a
//! gzip body within the ratio that decodes past the 256 MiB a browser's
//! pipe frames in one message, which `page.request` and a route's
//! `route.fetch` refuse naming `network.max_body_size` while the browser's
//! connection goes on. Each refusal pairs with its positive control: a
//! plain value is sent, and a page compressed within the ratio is kept.
//! Over TLS, where a browser offers `br` and `zstd`, `page.request` refuses
//! the Brotli and zstd bombs at the ratio, and fails a zstd frame whose
//! window is past RFC 9659's 8 MB as a decoding failure.

use std::path::Path;

use goodfetch_protocol::hostile_input::HEADER_VALUE_INVALID;
use goodfetch_protocol::page::request::PAGE_REQUEST;
use goodfetch_protocol::page::route::{
    PAGE_ROUTE, ROUTE_CONTINUE, ROUTE_FALLBACK, ROUTE_FETCH, ROUTE_FULFILL,
};
use goodfetch_protocol::response_limits::{
    NETWORK_MAX_BODY_SIZE, NETWORK_MAX_DECOMPRESSION_RATIO, RESPONSE_LIMIT_EXCEEDED,
};
use serde::Deserialize;
use serde_json::{Value, json};

use super::consent::walls;
use super::navigation::{call, failed_with};
use super::robots_txt::PAST_THE_SIZE_HOST;
use super::routes::{Act, Routes, glob, goto};
use super::{Checks, Site, finish_run, new_page, start_run, user_configuration};
use crate::protocol::connection::{Connection, Gone, code_of};
use crate::protocol::target::EngineTarget;
use crate::proving_ground::ObservedRequest;

/// The `bombs` family's own host, which serves the zstd frame whose window
/// is past 8 MB at [`PAST_THE_WINDOW`] and belongs to no task.
pub(super) const BOMBS_HOST: &str = "bombs.test";

/// The path of that frame.
const PAST_THE_WINDOW: &str = "/past-the-window";

/// The browser's network error for a zstd frame whose window is past 8 MB.
const ZSTD_WINDOW_SIZE_TOO_BIG: &str = "net::ERR_ZSTD_WINDOW_SIZE_TOO_BIG";

/// One hostile-input case of the page suite.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Case {
    RequestHeaders,
    RouteHeaders,
    RequestLimits,
    CodedPastTheFrame,
    CodedBombsOverTls,
}

impl Case {
    /// The case a suite's case ID names.
    pub(crate) fn from_id(id: &str) -> Option<Self> {
        Some(match id {
            "page-request-refuses-a-header-value-with-a-line-break-or-a-nul-before-the-browser" => {
                Self::RequestHeaders
            }
            "a-routes-header-value-with-a-line-break-or-a-nul-is-refused-before-the-browser" => {
                Self::RouteHeaders
            }
            "page-request-refuses-a-body-past-the-runs-limits-naming-the-setting" => {
                Self::RequestLimits
            }
            "a-coded-body-past-the-browsers-frame-is-refused-at-max-body-size-and-the-browser-goes-on" => {
                Self::CodedPastTheFrame
            }
            "over-tls-page-request-refuses-the-brotli-and-zstd-bombs-at-the-ratio-and-a-zstd-window-past-8-mb-as-undecodable" => {
                Self::CodedBombsOverTls
            }
            _ => return None,
        })
    }

    /// Runs the case on the open Connection to `target` against `site`.
    pub(crate) fn run(
        self,
        connection: &mut Connection,
        target: &EngineTarget,
        site: &Site,
        checks: &mut Checks,
    ) -> Result<(), Gone> {
        match self {
            Self::RequestHeaders => request_headers(connection, site, checks),
            Self::RouteHeaders => route_headers(connection, site, checks),
            Self::RequestLimits => request_limits(connection, site, checks),
            Self::CodedPastTheFrame => coded_past_the_frame(connection, site, checks),
            Self::CodedBombsOverTls => coded_bombs_over_tls(connection, target, site, checks),
        }
    }
}

/// The shared table of header values no header may carry (ADR 0067), read
/// whole so a case it gains runs in the suite too.
const HEADER_VALUES: &str =
    include_str!("../../../../../test-support/fixtures/hostile/header_values.toml");

/// What the suite reads of the header-value table.
#[derive(Deserialize)]
struct HeaderValues {
    cases: Vec<HeaderValueCase>,
}

/// One case of the header-value table.
#[derive(Deserialize)]
struct HeaderValueCase {
    name: String,
    input: HeaderValueInput,
}

/// Where a case's value is set, and the value.
#[derive(Deserialize)]
struct HeaderValueInput {
    source: String,
    value: String,
}

/// The header values the table gives the source `source` (`page_request`
/// or `route`), each with the name of its case.
fn hostile_values(source: &str) -> Vec<(String, String)> {
    let table: HeaderValues =
        toml::from_str(HEADER_VALUES).expect("invariant: the embedded header-value table is valid");
    table
        .cases
        .into_iter()
        .filter(|case| case.input.source == source)
        .map(|case| (case.name, case.input.value))
        .collect()
}

/// The header every case sets, and the plain value its positive control
/// sends.
const HEADER: &str = "X-Note";
const PLAIN: &str = "a plain note";

/// The requests to `host` whose target starts with `prefix` the Proving
/// ground received.
fn received(site: &Site, host: &str, prefix: &str) -> Vec<ObservedRequest> {
    site.received_requests()
        .of_the_case()
        .filter(|request| request.host == host && request.target.starts_with(prefix))
        .cloned()
        .collect()
}

/// The values of the header `name` the request `request` carried.
fn header_values<'a>(request: &'a ObservedRequest, name: &str) -> Vec<&'a str> {
    request
        .headers
        .iter()
        .filter(|(sent, _)| sent.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
        .collect()
}

/// `page.request` with a CR, an LF or a NUL in a header value raises
/// `header_value_invalid` and sends nothing; the same header with a plain
/// value reaches the site.
fn request_headers(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-hostile-request-headers";
    let target = site.named_url("fetch-edge.test", "/target");
    start_run(connection, site, run_id, &json!({}), checks)?;
    let Some(page) = new_page(connection, run_id, checks)?.map(|page| json!(page.page_id)) else {
        return finish_run(connection, run_id);
    };
    for (case, value) in hostile_values("page_request") {
        let url = format!("{target}?header={case}");
        let refused = call(
            connection,
            PAGE_REQUEST.name,
            &page,
            &json!({ "url": url, "headers": { HEADER: value } }),
        )?;
        checks.check(
            format!(
                "`page.request` with the header value of {case} raises `{}`",
                HEADER_VALUE_INVALID.name
            ),
            code_of(&refused) == &json!(HEADER_VALUE_INVALID.name),
        );
        checks.check(
            format!("naming the header, never its value, for {case}"),
            refused["error"]["data"]["name"] == json!(HEADER)
                && !refused.to_string().contains("X-Injected"),
        );
        checks.check(
            format!("the request of {case} never reaches the site"),
            received(site, "fetch-edge.test", &format!("/target?header={case}")).is_empty(),
        );
    }
    let sent = call(
        connection,
        PAGE_REQUEST.name,
        &page,
        &json!({ "url": format!("{target}?header=plain"), "headers": { HEADER: PLAIN } }),
    )?;
    checks.check(
        "`page.request` with a plain header value answers 200",
        sent["result"]["status"] == json!(200),
    );
    let plain = received(site, "fetch-edge.test", "/target?header=plain");
    checks.check(
        "the site received the plain value",
        plain.len() == 1 && header_values(&plain[0], HEADER) == [PLAIN],
    );
    finish_run(connection, run_id)
}

/// A route's `continue`, `fallback` and `fulfill` with a CR, an LF or a NUL
/// in a header value each raise `header_value_invalid` and leave the
/// request paused; the handler's next `continue`, with a plain value, sends
/// it, and the site receives that value alone.
fn route_headers(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-hostile-route-headers";
    let url = site.named_url("scope.test", "/about?route=hostile-headers");
    start_run(connection, site, run_id, &json!({}), checks)?;
    let Some(page) = new_page(connection, run_id, checks)? else {
        return finish_run(connection, run_id);
    };
    let values = hostile_values("route");
    let hostile = values.len();
    let mut routes = Routes::new(move |_| {
        let mut acts: Vec<Act> = values
            .iter()
            .map(|(_, value)| {
                Act::Call(ROUTE_CONTINUE.name, json!({ "headers": { HEADER: value } }))
            })
            .collect();
        acts.push(Act::Call(
            ROUTE_FALLBACK.name,
            json!({ "headers": { HEADER: "a\r\nX-Injected: yes" } }),
        ));
        acts.push(Act::Call(
            ROUTE_FULFILL.name,
            json!({ "headers": { HEADER: "a\u{0}b" }, "body": "" }),
        ));
        acts.push(Act::Call(
            ROUTE_CONTINUE.name,
            json!({ "headers": { HEADER: PLAIN } }),
        ));
        acts.push(Act::End);
        acts
    });
    Routes::route(
        connection,
        PAGE_ROUTE.name,
        &json!({ "page_id": page.page_id, "url": glob("**/about*") }),
    )?;
    let loaded = goto(&mut routes, connection, &page, &url)?;
    let made = routes.made(connection)?;
    let refused = made.len().saturating_sub(1);
    checks.check(
        format!(
            "each `route.continue`, `route.fallback` and `route.fulfill` with a CR, an LF or a NUL in a header value raises `{}`",
            HEADER_VALUE_INVALID.name
        ),
        made.len() == hostile + 3
            && made[..refused]
                .iter()
                .all(|(_, answer)| code_of(answer) == &json!(HEADER_VALUE_INVALID.name)),
    );
    checks.check(
        "the `route.continue` with a plain value after them is answered",
        made.last()
            .is_some_and(|(_, answer)| answer.get("result").is_some()),
    );
    checks.check(
        "the page loads",
        loaded["result"]["response"]["status"] == json!(200),
    );
    let reached = received(site, "scope.test", "/about?route=hostile-headers");
    checks.check(
        "the site received the request once, with the plain value alone",
        reached.len() == 1
            && header_values(&reached[0], HEADER) == [PLAIN]
            && header_values(&reached[0], "X-Injected").is_empty(),
    );
    finish_run(connection, run_id)
}

/// `page.request` to a decompression bomb raises `response_limit_exceeded`
/// naming `network.max_decompression_ratio`, and to a body one byte past
/// 64 MiB, naming `network.max_body_size`; a page compressed within the
/// ratio answers 200.
fn request_limits(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-hostile-request-limits";
    let (bomb, safe, past_the_size) = (
        site.named_url("1.bombs.test", "/"),
        site.named_url("6.bombs.test", "/"),
        site.named_url("1.body-limits.test", "/"),
    );
    start_run(connection, site, run_id, &json!({}), checks)?;
    let Some(page) = new_page(connection, run_id, checks)?.map(|page| json!(page.page_id)) else {
        return finish_run(connection, run_id);
    };
    for (url, setting, limit) in [
        (&bomb, NETWORK_MAX_DECOMPRESSION_RATIO.name, 100),
        (&past_the_size, NETWORK_MAX_BODY_SIZE.name, 67_108_864),
    ] {
        let refused = call(connection, PAGE_REQUEST.name, &page, &json!({ "url": url }))?;
        checks.check(
            format!(
                "`page.request` to {url} raises `{}` naming `{setting}`",
                RESPONSE_LIMIT_EXCEEDED.name
            ),
            code_of(&refused) == &json!(RESPONSE_LIMIT_EXCEEDED.name)
                && refused["error"]["data"]["setting"] == json!(setting)
                && refused["error"]["data"]["limit"] == json!(limit),
        );
    }
    let kept = call(
        connection,
        PAGE_REQUEST.name,
        &page,
        &json!({ "url": safe }),
    )?;
    checks.check(
        "`page.request` to a page compressed within the ratio answers 200",
        kept["result"]["status"] == json!(200),
    );
    finish_run(connection, run_id)
}

/// The Run's `network.max_body_size` in the coded body's case: 1 MiB, so a
/// streamed read reaches it within a few chunks however loaded the machine,
/// while the body still decodes far past the browser pipe's frame.
const MAX_BODY_SIZE: u64 = 1024 * 1024;

/// Whether `answer` raises `response_limit_exceeded` naming
/// `network.max_body_size` and [`MAX_BODY_SIZE`].
fn past_the_size(answer: &Value) -> bool {
    code_of(answer) == &json!(RESPONSE_LIMIT_EXCEEDED.name)
        && answer["error"]["data"]["setting"] == NETWORK_MAX_BODY_SIZE.name
        && answer["error"]["data"]["limit"] == json!(MAX_BODY_SIZE)
}

/// The `route.fulfill` params a handler answers after its `route.fetch`:
/// an empty page, as the fetch got no body to pass on.
fn empty_page(_fetched: &Value) -> Value {
    json!({ "status": 200, "content_type": "text/plain", "body": "" })
}

/// On a Run whose `network.max_body_size` is 1 MiB, `page.request` to a
/// gzip body within the ratio that decodes to 272 MiB, past the 256 MiB a
/// browser's pipe frames in one message (`body-limits/6`), raises
/// `response_limit_exceeded` naming `network.max_body_size`, and so does a route's `route.fetch` of the same
/// body on a navigation; the browser's connection goes on, so a later
/// `page.request` answers 200.
fn coded_past_the_frame(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-hostile-coded-past-the-frame";
    let settings = json!({ "network": { "max_body_size": MAX_BODY_SIZE } });
    let (coded, small) = (
        site.named_url("6.body-limits.test", "/"),
        site.named_url(PAST_THE_SIZE_HOST, "/page"),
    );
    start_run(connection, site, run_id, &settings, checks)?;
    let Some(page) = new_page(connection, run_id, checks)? else {
        return finish_run(connection, run_id);
    };
    let page_id = json!(page.page_id);
    let refused = call(
        connection,
        PAGE_REQUEST.name,
        &page_id,
        &json!({ "url": coded }),
    )?;
    checks.check(
        format!(
            "`page.request` to a coded body past the browser's frame raises `{}` naming `network.max_body_size` and {MAX_BODY_SIZE}",
            RESPONSE_LIMIT_EXCEEDED.name
        ),
        past_the_size(&refused),
    );
    let mut routes = Routes::new(|_| vec![Act::FetchThenFulfill(empty_page)]);
    Routes::route(
        connection,
        PAGE_ROUTE.name,
        &json!({ "page_id": page.page_id, "url": glob("**") }),
    )?;
    let answer = goto(&mut routes, connection, &page, &coded)?;
    checks.check(
        "the `goto` whose fetched body passed the limit answers",
        answer.get("result").is_some() || answer.get("error").is_some(),
    );
    let made = routes.made(connection)?;
    checks.check(
        format!(
            "`route.fetch` of it raises `{}` naming `network.max_body_size` and {MAX_BODY_SIZE}",
            RESPONSE_LIMIT_EXCEEDED.name
        ),
        made.first()
            .is_some_and(|(method, answer)| *method == ROUTE_FETCH.name && past_the_size(answer)),
    );
    let kept = call(
        connection,
        PAGE_REQUEST.name,
        &page_id,
        &json!({ "url": small }),
    )?;
    checks.check(
        "the browser's connection goes on: a later `page.request` answers 200",
        kept["result"]["status"] == json!(200),
    );
    finish_run(connection, run_id)
}

/// The name, in the engine's per-major folder, of the copy of the Proving
/// ground's authority that user configuration trusts for [`BOMBS_HOST`].
const AUTHORITY_FILE: &str = "conformance-bombs-authority.pem";

/// Writes `pem` to `file`, or removes `file` when `pem` is `None`.
#[expect(
    clippy::disallowed_methods,
    reason = "the harness is synchronous and runs no async worker"
)]
fn place(file: &Path, pem: Option<&str>) -> std::io::Result<()> {
    match pem {
        Some(pem) => std::fs::write(file, pem),
        None => std::fs::remove_file(file),
    }
}

/// Over `https://`, where a browser offers `br` and `zstd` (it offers
/// neither over `http://` to a host other than localhost), `page.request`
/// to the Brotli and zstd bombs (`bombs/3`, `bombs/4`) raises
/// `response_limit_exceeded` naming `network.max_decompression_ratio`, and
/// to a zstd frame whose window is past RFC 9659's 8 MB fails as a decoding
/// failure, `navigation_failed` naming the browser's
/// `net::ERR_ZSTD_WINDOW_SIZE_TOO_BIG`, never as a cut body. User
/// configuration trusts the Proving ground's authority for `bombs.test`
/// (`tls.trusted_cas`, which no Run sets; ADR 0042), so the engine's relay
/// terminates its hosts, while the case's Run goes.
fn coded_bombs_over_tls(
    connection: &mut Connection,
    target: &EngineTarget,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let (Some(brotli), Some(zstd), past_the_window) = (
        site.start_url("bombs/3").map(str::to_owned),
        site.start_url("bombs/4").map(str::to_owned),
        site.named_secure_url(BOMBS_HOST, PAST_THE_WINDOW),
    ) else {
        checks.check("the Proving ground serves the case's pages", false);
        return Ok(());
    };
    checks.check(
        "the Brotli and zstd bombs start over `https://`",
        brotli.starts_with("https://") && zstd.starts_with("https://"),
    );
    let first = "conformance-hostile-coded-bombs-home";
    start_run(connection, site, first, &json!({}), checks)?;
    finish_run(connection, first)?;
    let major = target
        .environment
        .get("GOODFETCH_HOME")
        .and_then(|home| walls::major_of(home, first));
    let authority = checks.needs(
        "the Proving ground serves its certificate authority",
        site.proving_ground.certificate_authority(),
    );
    let (Some(major), Some(authority)) = (major, authority) else {
        checks.check(
            "the engine's per-major folder and the Proving ground's authority are found",
            false,
        );
        return Ok(());
    };
    let file = major.join(AUTHORITY_FILE);
    checks.read(
        "the Proving ground's authority is written beside the user configuration",
        place(&file, Some(&authority)),
    );
    let Some(before) = checks.needs(
        "user configuration trusts the Proving ground's authority for `bombs.test`",
        walls::configure(
            &major,
            Some(&user_configuration(&format!(
                "[settings.tls.trusted_cas]\n\"{BOMBS_HOST}\" = [{}]\n",
                json!(file.display().to_string())
            ))),
        ),
    ) else {
        checks.needs("the authority's copy is removed", place(&file, None));
        return Ok(());
    };
    let outcome = coded_bombs_on(connection, site, [&brotli, &zstd], &past_the_window, checks);
    checks.needs(
        "user configuration is put back",
        walls::configure(&major, before.as_deref()),
    );
    checks.needs("the authority's copy is removed", place(&file, None));
    outcome
}

/// [`coded_bombs_over_tls`]'s Run, once user configuration trusts the
/// Proving ground's authority for `bombs.test`.
fn coded_bombs_on(
    connection: &mut Connection,
    site: &Site,
    bombs: [&str; 2],
    past_the_window: &str,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-hostile-coded-bombs";
    start_run(connection, site, run_id, &json!({}), checks)?;
    let Some(page) = new_page(connection, run_id, checks)?.map(|page| json!(page.page_id)) else {
        return finish_run(connection, run_id);
    };
    for url in bombs {
        let refused = call(connection, PAGE_REQUEST.name, &page, &json!({ "url": url }))?;
        checks.check(
            format!(
                "`page.request` to {url} raises `{}` naming `{}`",
                RESPONSE_LIMIT_EXCEEDED.name, NETWORK_MAX_DECOMPRESSION_RATIO.name
            ),
            code_of(&refused) == &json!(RESPONSE_LIMIT_EXCEEDED.name)
                && refused["error"]["data"]["setting"] == NETWORK_MAX_DECOMPRESSION_RATIO.name
                && refused["error"]["data"]["limit"] == json!(100),
        );
    }
    let undecodable = call(
        connection,
        PAGE_REQUEST.name,
        &page,
        &json!({ "url": past_the_window }),
    )?;
    failed_with(
        &undecodable,
        "`page.request` of a zstd frame whose window is past 8 MB",
        ZSTD_WINDOW_SIZE_TOO_BIG,
        checks,
    );
    finish_run(connection, run_id)
}

#[cfg(test)]
mod tests;
