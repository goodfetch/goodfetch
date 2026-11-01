//! The page suite's route cases (ADR 0085, ticket #702): `page.route` and
//! `session.route` under the rules of Selenium's `WebDriver BiDi` design note with
//! Playwright's names, judged by the answers the engine gives and by the
//! Proving ground's own record of the requests it received. The harness is
//! the routes' handler: it answers each `route.handle` request the engine
//! sends with the calls a case scripts, then replies that the handler ended
//! or failed.
//!
//! Size: one handler harness and the cases that script it, each reading
//! its private record of what the handler served and called.

use std::collections::BTreeMap;

use data_encoding::BASE64;
use goodfetch_protocol::connection::CLOSED;
use goodfetch_protocol::connection::{CALL_CANCELLED, CANCEL};
use goodfetch_protocol::egress::EGRESS_REFUSED;
use goodfetch_protocol::page::functions::PAGE_EVALUATE;
use goodfetch_protocol::page::route::{
    PAGE_HANDLER_FAILED, PAGE_ROUTE, ROUTE_ABORT, ROUTE_ALREADY_HANDLED, ROUTE_CONTINUE,
    ROUTE_FALLBACK, ROUTE_FETCH, ROUTE_FULFILL, ROUTE_HANDLE, SESSION_ROUTE,
};
use goodfetch_protocol::page::session_pages::PAGE_CLOSE;
use goodfetch_protocol::page::{NAVIGATION_FAILED, PAGE_GOTO, PageOpened};
use goodfetch_protocol::response_limits::RESPONSE_LIMIT_EXCEEDED;
use goodfetch_protocol::run::RUN_FINISH;
use serde_json::{Value, json};

use super::navigation::{answered, url_on};
use super::{Checks, Site, finish_run, new_page, start_run};
use crate::protocol::connection::{Connection, Gone};
use crate::protocol::target::BROWSER_DEADLINE;

/// The Error code the harness's handler fails with, as an SDK's does for a
/// handler that threw.
const THREW: &str = "handler_failed";

/// One route case of the page suite.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Case {
    Unanswered,
    HandlerFails,
    AnsweredTwice,
    Order,
    Answers,
    Fetch,
    StepOrder,
    Cache,
    SiblingClosed,
    PostData,
    FetchPageClosed,
    FetchPastLimit,
}

impl Case {
    /// The case a suite's case ID names.
    pub(crate) fn from_id(id: &str) -> Option<Self> {
        Some(match id {
            "an-unanswered-route-lets-the-request-through-and-the-page-loads" => Self::Unanswered,
            "a-route-whose-handler-fails-aborts-its-request-is-removed-and-the-waiting-call-raises-page-handler-failed" => {
                Self::HandlerFails
            }
            "a-second-answer-raises-route-already-handled" => Self::AnsweredTwice,
            "page-routes-come-before-session-routes-later-first-and-the-first-answer-wins" => {
                Self::Order
            }
            "continue-fulfill-and-abort-reach-the-server-as-its-record-shows" => Self::Answers,
            "fetch-hands-the-request-on-and-fulfill-answers-its-changed-response" => Self::Fetch,
            "what-a-route-sends-on-passes-the-domain-policy-and-cookies-come-after-every-route" => {
                Self::StepOrder
            }
            "adding-a-route-leaves-the-browsers-cache-serving-a-cached-response" => Self::Cache,
            "a-session-route-keeps-serving-its-other-pages-once-one-closes" => Self::SiblingClosed,
            "a-route-reads-the-body-the-page-posts" => Self::PostData,
            "a-fetch-whose-page-closes-before-its-response-raises-closed-and-the-run-finishes" => {
                Self::FetchPageClosed
            }
            "a-fetch-whose-body-passes-network-max-body-size-raises-response-limit-exceeded" => {
                Self::FetchPastLimit
            }
            _ => return None,
        })
    }

    /// Runs the case on the open Connection against `site`.
    pub(crate) fn run(
        self,
        connection: &mut Connection,
        site: &Site,
        checks: &mut Checks,
    ) -> Result<(), Gone> {
        match self {
            Self::Unanswered => unanswered(connection, site, checks),
            Self::HandlerFails => handler_fails(connection, site, checks),
            Self::AnsweredTwice => answered_twice(connection, site, checks),
            Self::Order => order(connection, site, checks),
            Self::Answers => continue_fulfill_abort(connection, site, checks),
            Self::Fetch => fetch_then_fulfill(connection, site, checks),
            Self::StepOrder => order_of_steps(connection, site, checks),
            Self::Cache => cache_kept(connection, site, checks),
            Self::SiblingClosed => sibling_closed(connection, site, checks),
            Self::PostData => post_data(connection, site, checks),
            Self::FetchPageClosed => fetch_page_closed(connection, site, checks),
            Self::FetchPastLimit => fetch_past_limit(connection, site, checks),
        }
    }
}

/// What the harness's handler does with one `route.handle` request, in
/// order.
pub(super) enum Act {
    /// Calls this `route.*` operation with these params, the request's
    /// `route_id` added.
    Call(&'static str, Value),
    /// Calls `route.fetch`, then, once it answers, `route.fulfill` with the
    /// params this function makes of the fetched response, then ends.
    FetchThenFulfill(fn(&Value) -> Value),
    /// Replies that the handler ended.
    End,
    /// Replies that the handler failed.
    Fail,
}

type Handler = Box<dyn FnMut(&Value) -> Vec<Act>>;

/// The harness as the routes' handler on one Connection: every answer it
/// read, by its request's id, and every `route.handle` request it served.
pub(super) struct Routes {
    handler: Handler,
    answers: BTreeMap<i64, Value>,
    /// The params of each `route.handle` request, in arrival order.
    pub(super) handled: Vec<Value>,
    /// The calls the handler made, in order: their method and id.
    made: Vec<(&'static str, i64)>,
    /// The `route.fetch` calls awaiting their answer, by their id.
    fetching: BTreeMap<i64, Fetching>,
}

/// A `route.fetch` call awaiting its answer.
struct Fetching {
    /// The request's `route_id`.
    route_id: Value,
    /// The id of the `route.handle` request it serves.
    handle: Value,
    /// What makes the `route.fulfill` params of the fetched response.
    change: fn(&Value) -> Value,
}

impl Routes {
    pub(super) fn new(handler: impl FnMut(&Value) -> Vec<Act> + 'static) -> Self {
        Self {
            handler: Box::new(handler),
            answers: BTreeMap::new(),
            handled: Vec::new(),
            made: Vec::new(),
            fetching: BTreeMap::new(),
        }
    }

    /// Subscribes `method` (`page.route` or `session.route`) with
    /// `params`, and returns the subscription's id.
    pub(super) fn route(
        connection: &mut Connection,
        method: &str,
        params: &Value,
    ) -> Result<i64, Gone> {
        connection.request(method, params)
    }

    /// Calls `method` with `params` and answers its answer, serving every
    /// `route.handle` request that comes meanwhile.
    pub(super) fn call(
        &mut self,
        connection: &mut Connection,
        method: &str,
        params: &Value,
    ) -> Result<Value, Gone> {
        let id = connection.request(method, params)?;
        self.answer(connection, id)
    }

    /// The answer to the request `id`, serving every `route.handle`
    /// request that comes before it.
    pub(super) fn answer(&mut self, connection: &mut Connection, id: i64) -> Result<Value, Gone> {
        loop {
            if let Some(answer) = self.answers.get(&id) {
                return Ok(answer.clone());
            }
            let message = connection.next_within(BROWSER_DEADLINE)?;
            self.take(connection, message)?;
        }
    }

    /// Serves every `route.handle` request that comes until `holds` is
    /// true of what the harness served and called.
    fn until(
        &mut self,
        connection: &mut Connection,
        holds: impl Fn(&Self) -> bool,
    ) -> Result<(), Gone> {
        while !holds(self) {
            let message = connection.next_within(BROWSER_DEADLINE)?;
            self.take(connection, message)?;
        }
        Ok(())
    }

    /// The answers to the calls the handler made, in order, each read once
    /// it came.
    pub(super) fn made(
        &mut self,
        connection: &mut Connection,
    ) -> Result<Vec<(&'static str, Value)>, Gone> {
        let made = self.made.clone();
        made.into_iter()
            .map(|(method, id)| Ok((method, self.answer(connection, id)?)))
            .collect()
    }

    fn take(&mut self, connection: &mut Connection, message: Value) -> Result<(), Gone> {
        if message["method"] == json!(ROUTE_HANDLE.name) && message.get("id").is_some() {
            return self.serve(connection, &message);
        }
        if message.get("method").is_some() {
            return Ok(());
        }
        let Some(id) = message["id"].as_i64() else {
            return Ok(());
        };
        if let Some(Fetching {
            route_id,
            handle,
            change,
        }) = self.fetching.remove(&id)
        {
            let mut params = change(&message["result"]);
            params["route_id"] = route_id;
            let fulfilled = connection.request(ROUTE_FULFILL.name, &params)?;
            self.made.push((ROUTE_FULFILL.name, fulfilled));
            connection.reply(&handle, &json!({}))?;
        }
        self.answers.insert(id, message);
        Ok(())
    }

    fn serve(&mut self, connection: &mut Connection, request: &Value) -> Result<(), Gone> {
        let params = request["params"].clone();
        self.handled.push(params.clone());
        let route_id = params["route_id"].clone();
        for act in (self.handler)(&params) {
            match act {
                Act::Call(method, mut call) => {
                    call["route_id"] = route_id.clone();
                    let id = connection.request(method, &call)?;
                    self.made.push((method, id));
                }
                Act::FetchThenFulfill(change) => {
                    let id =
                        connection.request(ROUTE_FETCH.name, &json!({ "route_id": route_id }))?;
                    self.made.push((ROUTE_FETCH.name, id));
                    self.fetching.insert(
                        id,
                        Fetching {
                            route_id,
                            handle: request["id"].clone(),
                            change,
                        },
                    );
                    return Ok(());
                }
                Act::End => connection.reply(&request["id"], &json!({}))?,
                Act::Fail => connection.fail(&request["id"], THREW, "the route's handler threw")?,
            }
        }
        Ok(())
    }
}

/// A glob matcher's wire form.
pub(super) fn glob(pattern: &str) -> Value {
    json!({ "kind": "glob", "pattern": pattern })
}

/// A body in Base64.
fn base64(text: &str) -> String {
    BASE64.encode(text.as_bytes())
}

/// An HTML page's `route.fulfill` params.
fn html(body: &str) -> Value {
    json!({ "status": 200, "content_type": "text/html", "body": base64(body) })
}

/// How many requests to `host` for `target` the Proving ground received.
pub(super) fn received(site: &Site, host: &str, target: &str) -> usize {
    requests_to(site, host, target).len()
}

/// The requests to `host` for `target` the Proving ground received.
fn requests_to(site: &Site, host: &str, target: &str) -> Vec<Value> {
    site.proving_ground
        .received_requests()
        .unwrap_or_default()
        .of_the_case()
        .filter(|request| request["host"] == json!(host) && request["target"] == json!(target))
        .cloned()
        .collect()
}

/// The path and query of `url`.
fn target_of(url: &str) -> &str {
    url.find("://")
        .and_then(|scheme| {
            url[scheme + 3..]
                .find('/')
                .map(|path| &url[scheme + 3 + path..])
        })
        .unwrap_or("/")
}

/// The Error code a failure answer names.
fn code_of(answer: &Value) -> &Value {
    &answer["error"]["data"]["code"]
}

/// Starts the Run `run_id` with `settings` and opens a page of it.
fn page(
    connection: &mut Connection,
    site: &Site,
    run_id: &str,
    settings: &Value,
    checks: &mut Checks,
) -> Result<Option<PageOpened>, Gone> {
    start_run(connection, site, run_id, settings, checks)?;
    new_page(connection, run_id, checks)
}

/// `page.goto` on `page` to `url`.
pub(super) fn goto(
    routes: &mut Routes,
    connection: &mut Connection,
    page: &PageOpened,
    url: &str,
) -> Result<Value, Gone> {
    routes.call(
        connection,
        PAGE_GOTO.name,
        &json!({ "page_id": page.page_id, "url": url }),
    )
}

/// Cancels the subscription `id` and reads its end.
fn unroute(routes: &mut Routes, connection: &mut Connection, id: i64) -> Result<Value, Gone> {
    connection.notify(CANCEL, &json!({ "id": id }))?;
    routes.answer(connection, id)
}

/// A route whose handler ends without answering falls back, past the last
/// route to the network: the page loads, and the Proving ground received
/// the request.
fn unanswered(connection: &mut Connection, site: &Site, checks: &mut Checks) -> Result<(), Gone> {
    let run_id = "conformance-route-unanswered";
    let (Some(url), Some(page)) = (
        site.named_url("scope.test", "/about?route=unanswered"),
        page(connection, site, run_id, &json!({}), checks)?,
    ) else {
        return finish_run(connection, run_id);
    };
    let mut routes = Routes::new(|_| vec![Act::End]);
    let route = Routes::route(
        connection,
        PAGE_ROUTE.name,
        &json!({ "page_id": page.page_id, "url": glob("**/about*") }),
    )?;
    let answer = goto(&mut routes, connection, &page, &url)?;
    answered(&answer, "the routed `goto`", &url, 200, checks);
    checks.check(
        "the route's handler was asked about the document's request",
        routes.handled.len() == 1 && routes.handled[0]["request"]["url"] == json!(url),
    );
    checks.check(
        "the Proving ground received the request once",
        received(site, "scope.test", target_of(&url)) == 1,
    );
    let ended = unroute(&mut routes, connection, route)?;
    checks.check(
        "cancelling the route ends its subscription with `call_cancelled`",
        code_of(&ended) == &json!(CALL_CANCELLED.name),
    );
    finish_run(connection, run_id)
}

/// A handler that fails aborts its request, which never reaches the site;
/// the `goto` waiting on it raises `page_handler_failed`, the route's
/// subscription ends with it, and the route is gone: the next `goto` loads
/// unrouted.
fn handler_fails(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-route-handler-fails";
    let (Some(url), Some(page)) = (
        site.named_url("scope.test", "/about?route=fails"),
        page(connection, site, run_id, &json!({}), checks)?,
    ) else {
        return finish_run(connection, run_id);
    };
    let mut routes = Routes::new(|_| vec![Act::Fail]);
    let route = Routes::route(
        connection,
        PAGE_ROUTE.name,
        &json!({ "page_id": page.page_id, "url": glob("**/about*") }),
    )?;
    let failed = goto(&mut routes, connection, &page, &url)?;
    checks.check(
        format!("the `goto` raises `{}`", PAGE_HANDLER_FAILED.name),
        code_of(&failed) == &json!(PAGE_HANDLER_FAILED.name),
    );
    checks.check(
        "its details name the route's handler and the request's URL",
        failed["error"]["data"]["handler"] == json!("route")
            && failed["error"]["data"]["url"] == json!(url),
    );
    checks.check(
        "the aborted request never reached the Proving ground",
        received(site, "scope.test", target_of(&url)) == 0,
    );
    let ended = routes.answer(connection, route)?;
    checks.check(
        format!(
            "the route's subscription ends with `{}`",
            PAGE_HANDLER_FAILED.name
        ),
        code_of(&ended) == &json!(PAGE_HANDLER_FAILED.name),
    );
    let again = goto(&mut routes, connection, &page, &url)?;
    answered(&again, "the next `goto`", &url, 200, checks);
    checks.check(
        "the removed route's handler is asked nothing more",
        routes.handled.len() == 1,
    );
    checks.check(
        "the next request reached the Proving ground",
        received(site, "scope.test", target_of(&url)) == 1,
    );
    finish_run(connection, run_id)
}

/// The first answer stands; a second raises `route_already_handled`, and
/// so does one after the handler ended.
fn answered_twice(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-route-answered-twice";
    let (Some(url), Some(page)) = (
        site.named_url("scope.test", "/about?route=twice"),
        page(connection, site, run_id, &json!({}), checks)?,
    ) else {
        return finish_run(connection, run_id);
    };
    let mut routes = Routes::new(|_| {
        vec![
            Act::Call(ROUTE_FULFILL.name, html("<p>first</p>")),
            Act::Call(ROUTE_FULFILL.name, html("<p>second</p>")),
            Act::End,
            Act::Call(ROUTE_ABORT.name, json!({})),
        ]
    });
    Routes::route(
        connection,
        PAGE_ROUTE.name,
        &json!({ "page_id": page.page_id, "url": glob("**/about*") }),
    )?;
    let answer = goto(&mut routes, connection, &page, &url)?;
    answered(&answer, "the fulfilled `goto`", &url, 200, checks);
    let made = routes.made(connection)?;
    let codes: Vec<&Value> = made.iter().map(|(_, answer)| code_of(answer)).collect();
    checks.check(
        "the first `route.fulfill` is answered",
        made.first()
            .is_some_and(|(_, answer)| answer.get("result").is_some()),
    );
    checks.check(
        format!(
            "the second, and an answer after the handler ended, raise `{}`",
            ROUTE_ALREADY_HANDLED.name
        ),
        codes.get(1..) == Some(&[&json!(ROUTE_ALREADY_HANDLED.name); 2][..]),
    );
    checks.check(
        "nothing reached the Proving ground",
        received(site, "scope.test", target_of(&url)) == 0,
    );
    finish_run(connection, run_id)
}

/// Page routes come before Session routes, later routes first, each
/// `fallback` handing the request, with its overrides, to the route before;
/// the first answer wins, and the routes after it are not asked.
fn order(connection: &mut Connection, site: &Site, checks: &mut Checks) -> Result<(), Gone> {
    let run_id = "conformance-route-order";
    let (Some(first), Some(wins), Some(page)) = (
        site.named_url("scope.test", "/about?route=order"),
        site.named_url("scope.test", "/about?route=first-wins"),
        page(connection, site, run_id, &json!({}), checks)?,
    ) else {
        return finish_run(connection, run_id);
    };
    let session = Routes::route(
        connection,
        SESSION_ROUTE.name,
        &json!({ "session_id": page.session_id, "url": glob("**/about*") }),
    )?;
    let earlier = Routes::route(
        connection,
        PAGE_ROUTE.name,
        &json!({ "page_id": page.page_id, "url": glob("**/about*") }),
    )?;
    let later = Routes::route(
        connection,
        PAGE_ROUTE.name,
        &json!({ "page_id": page.page_id, "url": glob("**/about*") }),
    )?;
    let mut routes = Routes::new(move |params| {
        let fulfilling = params["request"]["url"]
            .as_str()
            .is_some_and(|url| url.ends_with("first-wins"));
        match params["id"].as_i64() {
            Some(id) if id == later && fulfilling => {
                vec![
                    Act::Call(ROUTE_FULFILL.name, html("<p>later</p>")),
                    Act::End,
                ]
            }
            Some(id) if id == later => vec![
                Act::Call(
                    ROUTE_FALLBACK.name,
                    json!({ "headers": { "x-goodfetch-route": "later" } }),
                ),
                Act::End,
            ],
            Some(id) if id == earlier => {
                vec![Act::Call(ROUTE_FALLBACK.name, json!({})), Act::End]
            }
            _ => vec![Act::Call(ROUTE_CONTINUE.name, json!({})), Act::End],
        }
    });
    let answer = goto(&mut routes, connection, &page, &first)?;
    answered(
        &answer,
        "the `goto` all three routes fell through",
        &first,
        200,
        checks,
    );
    let asked: Vec<Value> = routes
        .handled
        .iter()
        .map(|params| params["id"].clone())
        .collect();
    checks.check(
        "the later page route is asked first, then the earlier, then the Session's",
        asked == [json!(later), json!(earlier), json!(session)],
    );
    checks.check(
        "a route after a `fallback` sees the request with its overrides",
        routes.handled.get(1).is_some_and(|params| {
            params["request"]["headers"]["x-goodfetch-route"] == json!("later")
        }),
    );
    let reached = requests_to(site, "scope.test", target_of(&first));
    checks.check(
        "the Proving ground received the request once, with the override",
        reached.len() == 1 && sends_header(&reached[0], "x-goodfetch-route", "later"),
    );
    let before = routes.handled.len();
    let won = goto(&mut routes, connection, &page, &wins)?;
    answered(
        &won,
        "the `goto` the later route fulfilled",
        &wins,
        200,
        checks,
    );
    checks.check(
        "once the later route answers, no other route is asked",
        routes.handled.len() == before + 1,
    );
    checks.check(
        "the fulfilled request never reached the Proving ground",
        received(site, "scope.test", target_of(&wins)) == 0,
    );
    finish_run(connection, run_id)
}

/// `continue` sends the request on, here to another URL, which the
/// Proving ground received in place of the page's; `fulfill` and `abort`
/// reach no network.
fn continue_fulfill_abort(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-route-answers";
    let (Some(continued), Some(rewritten), Some(fulfilled), Some(aborted), Some(page)) = (
        site.named_url("scope.test", "/about?route=continue"),
        site.named_url("scope.test", "/about?route=continued"),
        site.named_url("scope.test", "/about?route=fulfill"),
        site.named_url("scope.test", "/about?route=abort"),
        page(connection, site, run_id, &json!({}), checks)?,
    ) else {
        return finish_run(connection, run_id);
    };
    let to = rewritten.clone();
    let mut routes = Routes::new(move |params| {
        let url = params["request"]["url"].as_str().unwrap_or_default();
        if url.ends_with("=continue") {
            vec![
                Act::Call(ROUTE_CONTINUE.name, json!({ "url": to })),
                Act::End,
            ]
        } else if url.ends_with("=fulfill") {
            vec![
                Act::Call(ROUTE_FULFILL.name, html("<p>fulfilled</p>")),
                Act::End,
            ]
        } else {
            vec![
                Act::Call(
                    ROUTE_ABORT.name,
                    json!({ "error_code": "addressunreachable" }),
                ),
                Act::End,
            ]
        }
    });
    Routes::route(
        connection,
        PAGE_ROUTE.name,
        &json!({ "page_id": page.page_id, "url": glob("**/about*") }),
    )?;
    let answer = goto(&mut routes, connection, &page, &continued)?;
    checks.check(
        "the continued `goto` answers a document's response",
        answer["result"]["response"]["status"] == json!(200),
    );
    checks.check(
        "the Proving ground received the overridden URL and not the page's",
        received(site, "scope.test", target_of(&rewritten)) == 1
            && received(site, "scope.test", target_of(&continued)) == 0,
    );
    let answer = goto(&mut routes, connection, &page, &fulfilled)?;
    answered(&answer, "the fulfilled `goto`", &fulfilled, 200, checks);
    checks.check(
        "the fulfilled request never reached the Proving ground",
        received(site, "scope.test", target_of(&fulfilled)) == 0,
    );
    let answer = goto(&mut routes, connection, &page, &aborted)?;
    checks.check(
        format!("the aborted `goto` raises `{}`", NAVIGATION_FAILED.name),
        code_of(&answer) == &json!(NAVIGATION_FAILED.name),
    );
    checks.check(
        "naming the network error the route chose",
        answer["error"]["data"]["error"] == json!("net::ERR_ADDRESS_UNREACHABLE"),
    );
    checks.check(
        "the aborted request never reached the Proving ground",
        received(site, "scope.test", target_of(&aborted)) == 0,
    );
    let made = routes.made(connection)?;
    checks.check(
        "each answer is answered",
        made.iter()
            .all(|(_, answer)| answer.get("result").is_some()),
    );
    finish_run(connection, run_id)
}

/// The fulfill params of a fetched response with a header of the
/// program's own added.
fn with_header(fetched: &Value) -> Value {
    let mut headers = fetched["headers"].clone();
    headers["x-goodfetch-fulfilled"] = json!("yes");
    json!({ "status": fetched["status"], "headers": headers, "body": fetched["body"] })
}

/// `fetch` sends the request on and answers its response, body kept; the
/// handler's `fulfill` of that response, changed, is what the page gets.
fn fetch_then_fulfill(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-route-fetch";
    let (Some(url), Some(page)) = (
        site.named_url("scope.test", "/about?route=fetch"),
        page(connection, site, run_id, &json!({}), checks)?,
    ) else {
        return finish_run(connection, run_id);
    };
    let mut routes = Routes::new(|_| vec![Act::FetchThenFulfill(with_header)]);
    Routes::route(
        connection,
        PAGE_ROUTE.name,
        &json!({ "page_id": page.page_id, "url": glob("**/about*") }),
    )?;
    let answer = goto(&mut routes, connection, &page, &url)?;
    answered(
        &answer,
        "the `goto` whose response was fetched",
        &url,
        200,
        checks,
    );
    checks.check(
        "the page got the fulfilled response, its header added",
        answer["result"]["response"]["headers"]["x-goodfetch-fulfilled"] == json!("yes"),
    );
    let made = routes.made(connection)?;
    let fetched = made.first().map(|(_, answer)| answer["result"].clone());
    checks.check(
        "`route.fetch` answers the site's response with its body",
        fetched.as_ref().is_some_and(|fetched| {
            fetched["status"] == json!(200)
                && fetched["body"]
                    .as_str()
                    .and_then(|body| BASE64.decode(body.as_bytes()).ok())
                    .is_some_and(|body| String::from_utf8_lossy(&body).contains("S-ABOUT"))
        }),
    );
    checks.check(
        "the Proving ground received the request once",
        received(site, "scope.test", target_of(&url)) == 1,
    );
    finish_run(connection, run_id)
}

/// The interception's order, steps 2, 3 and 7 (ADR 0085), on one Run whose
/// domain policy allows the portal and the scope site: cookies come after
/// every route, and the domain policy judges what a route sends on.
fn order_of_steps(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-route-steps";
    let settings = json!({ "egress": { "allowed_domains": ["portal.test", "scope.test"] } });
    let Some(page) = page(connection, site, run_id, &settings, checks)? else {
        return finish_run(connection, run_id);
    };
    let (Some(login), Some(off)) = (
        url_on(site, "portal/1", "/login"),
        site.named_url("elsewhere.test", "/offsite?route=rewritten"),
    ) else {
        return finish_run(connection, run_id);
    };
    let to = off.clone();
    let mut routes = Routes::new(move |params| {
        let url = params["request"]["url"].as_str().unwrap_or_default();
        if url.contains("/reports") {
            vec![
                Act::Call(ROUTE_CONTINUE.name, json!({ "url": to })),
                Act::End,
            ]
        } else if url.contains("route=headers") {
            vec![
                Act::Call(
                    ROUTE_CONTINUE.name,
                    json!({ "headers": { "x-goodfetch-route": "steps" } }),
                ),
                Act::End,
            ]
        } else if url.contains("route=fulfilled") {
            vec![
                Act::Call(ROUTE_FULFILL.name, html("<p>offsite</p>")),
                Act::End,
            ]
        } else {
            vec![Act::End]
        }
    });
    for pattern in ["**/login*", "**/reports*", "**/offsite*"] {
        Routes::route(
            connection,
            PAGE_ROUTE.name,
            &json!({ "page_id": page.page_id, "url": glob(pattern) }),
        )?;
    }
    cookies_come_last(&mut routes, connection, site, &page, &login, checks)?;
    policy_judges_what_routes_send(&mut routes, connection, site, &page, &off, checks)?;
    finish_run(connection, run_id)
}

/// Whether the request `request` the Proving ground received carries the
/// header `name` with `value`.
fn sends_header(request: &Value, name: &str, value: &str) -> bool {
    request["headers"].as_array().is_some_and(|headers| {
        headers.iter().any(|header| {
            header[0]
                .as_str()
                .is_some_and(|sent| sent.eq_ignore_ascii_case(name))
                && header[1] == json!(value)
        })
    })
}

/// Whether the request `request` the Proving ground received carries the
/// cookie `name`, once.
fn carries_cookie(request: &Value, name: &str) -> bool {
    request["cookies"].as_array().is_some_and(|cookies| {
        cookies
            .iter()
            .filter(|cookie| cookie[0] == json!(name))
            .count()
            == 1
    })
}

/// Step 7 after step 2: a load of the portal's login page carries the
/// cookie the first load set, which no route sees; a route that replaces
/// the request's headers does not lose it.
fn cookies_come_last(
    routes: &mut Routes,
    connection: &mut Connection,
    site: &Site,
    page: &PageOpened,
    login: &str,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let portal = super::host_of(login).unwrap_or_default().to_owned();
    let replaced = format!("{login}?route=headers");
    goto(routes, connection, page, login)?;
    let answer = goto(routes, connection, page, login)?;
    answered(
        &answer,
        "the second `goto` to the login page",
        login,
        200,
        checks,
    );
    checks.check(
        "the route sees no cookie on the request",
        routes.handled.get(1).is_some_and(|params| {
            params["request"]["headers"]
                .as_object()
                .is_some_and(|headers| {
                    !headers
                        .keys()
                        .any(|name| name.eq_ignore_ascii_case("cookie"))
                })
        }),
    );
    let logins = requests_to(site, &portal, "/login");
    checks.check(
        "the Proving ground received the Session's cookie on the second load",
        logins.len() == 2 && carries_cookie(&logins[1], "pre"),
    );
    goto(routes, connection, page, &replaced)?;
    let reached = requests_to(site, &portal, target_of(&replaced));
    checks.check(
        "a route's own headers reach the Proving ground with the Session's cookie added",
        reached.len() == 1
            && carries_cookie(&reached[0], "pre")
            && sends_header(&reached[0], "x-goodfetch-route", "steps"),
    );
    Ok(())
}

/// Step 3 after step 2: the policy judges the URL a route sends on, so a
/// route's URL off the policy is refused before any connection; a route's
/// `fulfill` ends the request before the policy; and a request its route
/// falls through is judged too.
fn policy_judges_what_routes_send(
    routes: &mut Routes,
    connection: &mut Connection,
    site: &Site,
    page: &PageOpened,
    off: &str,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let (Some(reports), Some(fulfilled_off), Some(unrouted_off)) = (
        url_on(site, "portal/1", "/reports?page=1"),
        site.named_url("elsewhere.test", "/offsite?route=fulfilled"),
        site.named_url("elsewhere.test", "/offsite?route=none"),
    ) else {
        return Ok(());
    };
    let portal = super::host_of(&reports).unwrap_or_default().to_owned();
    let refused = goto(routes, connection, page, &reports)?;
    checks.check(
        format!(
            "a route's URL off the domain policy raises `{}`",
            EGRESS_REFUSED.name
        ),
        code_of(&refused) == &json!(EGRESS_REFUSED.name)
            && refused["error"]["data"]["url"] == json!(off),
    );
    checks.check(
        "neither the page's URL nor the route's reached the Proving ground",
        received(site, &portal, "/reports?page=1") == 0
            && received(site, "elsewhere.test", target_of(off)) == 0,
    );
    let answer = goto(routes, connection, page, &fulfilled_off)?;
    answered(
        &answer,
        "the `goto` a route fulfilled off the policy",
        &fulfilled_off,
        200,
        checks,
    );
    let refused = goto(routes, connection, page, &unrouted_off)?;
    checks.check(
        format!(
            "a URL off the policy its route falls through raises `{}`",
            EGRESS_REFUSED.name
        ),
        code_of(&refused) == &json!(EGRESS_REFUSED.name),
    );
    checks.check(
        "nothing reached the Proving ground's off-site host",
        received(site, "elsewhere.test", target_of(&fulfilled_off)) == 0
            && received(site, "elsewhere.test", target_of(&unrouted_off)) == 0,
    );
    Ok(())
}

/// A route changes no caching: with a route on it, as without one, the
/// browser's cache serves a second load of a page it may keep, so the
/// Proving ground receives one request each time, and the route still
/// sees both.
fn cache_kept(connection: &mut Connection, site: &Site, checks: &mut Checks) -> Result<(), Gone> {
    let run_id = "conformance-route-cache";
    let (Some(without_route), Some(with_route), Some(page)) = (
        site.named_url("scope.test", "/cached?route=none"),
        site.named_url("scope.test", "/cached?route=page"),
        page(connection, site, run_id, &json!({}), checks)?,
    ) else {
        return finish_run(connection, run_id);
    };
    let mut routes = Routes::new(|_| vec![Act::End]);
    for _ in 0..2 {
        goto(&mut routes, connection, &page, &without_route)?;
    }
    checks.check(
        "without a route, the second load came from the browser's cache",
        received(site, "scope.test", target_of(&without_route)) == 1,
    );
    Routes::route(
        connection,
        PAGE_ROUTE.name,
        &json!({ "page_id": page.page_id, "url": glob("**/cached*") }),
    )?;
    for _ in 0..2 {
        let answer = goto(&mut routes, connection, &page, &with_route)?;
        answered(
            &answer,
            "a routed load of the cached page",
            &with_route,
            200,
            checks,
        );
    }
    checks.check(
        "with a route, the second load came from the browser's cache too",
        received(site, "scope.test", target_of(&with_route)) == 1,
    );
    checks.check("the route saw both loads", routes.handled.len() == 2);
    finish_run(connection, run_id)
}

/// A Session route belongs to the Session, not to the page it first
/// served: once one of the Session's pages closes, the route still answers
/// the others' requests, its subscription goes on, and another Session
/// route can still be armed.
fn sibling_closed(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-route-sibling-closed";
    let (Some(url), Some(again), Some(closing)) = (
        site.named_url("scope.test", "/about?route=sibling"),
        site.named_url("scope.test", "/again?route=sibling"),
        page(connection, site, run_id, &json!({}), checks)?,
    ) else {
        return finish_run(connection, run_id);
    };
    let Some(open) = new_page(connection, run_id, checks)? else {
        return finish_run(connection, run_id);
    };
    let session = Routes::route(
        connection,
        SESSION_ROUTE.name,
        &json!({ "session_id": closing.session_id, "url": glob("**/about*") }),
    )?;
    let closed = connection.request(PAGE_CLOSE.name, &json!({ "page_id": closing.page_id }))?;
    let closed = connection.browser_answer(closed)?.message;
    checks.check(
        "one page of the Session closes",
        closed.get("result").is_some(),
    );
    let armed_after = Routes::route(
        connection,
        SESSION_ROUTE.name,
        &json!({ "session_id": closing.session_id, "url": glob("**/again*") }),
    )?;
    let mut routes = Routes::new(|_| {
        vec![
            Act::Call(ROUTE_FULFILL.name, html("<p>session</p>")),
            Act::End,
        ]
    });
    let answer = goto(&mut routes, connection, &open, &url)?;
    answered(&answer, "the other page's `goto`", &url, 200, checks);
    let answer = goto(&mut routes, connection, &open, &again)?;
    answered(
        &answer,
        "the other page's second `goto`",
        &again,
        200,
        checks,
    );
    let asked: Vec<Value> = routes
        .handled
        .iter()
        .map(|params| params["id"].clone())
        .collect();
    checks.check(
        "the Session route answers the other page's request, and one armed after the close the next",
        asked == [json!(session), json!(armed_after)],
    );
    checks.check(
        "neither Session route's subscription ended",
        !routes.answers.contains_key(&session) && !routes.answers.contains_key(&armed_after),
    );
    checks.check(
        "the fulfilled requests never reached the Proving ground",
        received(site, "scope.test", target_of(&url)) == 0
            && received(site, "scope.test", target_of(&again)) == 0,
    );
    finish_run(connection, run_id)
}

/// A handler inside `route.fetch` whose page closes before the response
/// comes is not left waiting: `route.fetch` raises `closed`, and the Run
/// still finishes. The incidents family's sixth host answers its second
/// page only after 8 seconds, so the page closes first.
fn fetch_page_closed(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-route-fetch-page-closed";
    let (Some(url), Some(page)) = (
        site.start_url("incidents/6")
            .map(|start| start.replace("/catalog/1", "/catalog/2")),
        page(connection, site, run_id, &json!({}), checks)?,
    ) else {
        return finish_run(connection, run_id);
    };
    let mut routes = Routes::new(|_| vec![Act::FetchThenFulfill(with_header)]);
    Routes::route(
        connection,
        PAGE_ROUTE.name,
        &json!({ "page_id": page.page_id, "url": glob("**/catalog/2") }),
    )?;
    connection.request(
        PAGE_GOTO.name,
        &json!({ "page_id": page.page_id, "url": url }),
    )?;
    routes.until(connection, |routes| !routes.made.is_empty())?;
    let closed = routes.call(
        connection,
        PAGE_CLOSE.name,
        &json!({ "page_id": page.page_id }),
    )?;
    checks.check(
        "the page closes while its route's handler is inside `route.fetch`",
        closed.get("result").is_some(),
    );
    let made = routes.made(connection)?;
    checks.check(
        format!("`route.fetch` raises `{}`", CLOSED.name),
        made.first().is_some_and(|(method, answer)| {
            *method == ROUTE_FETCH.name && code_of(answer) == &json!(CLOSED.name)
        }),
    );
    let finished = connection.request(RUN_FINISH.name, &json!({ "run_id": run_id }))?;
    let finished = connection.browser_answer(finished)?.message;
    checks.check("the Run finishes", finished.get("result").is_some());
    Ok(())
}

/// The body `route.fetch` reads is one the engine itself reads (ticket
/// #644): past the Run's `network.max_body_size` it raises
/// `response_limit_exceeded` naming the setting and its value, and the
/// handler's call holds no copy of it.
fn fetch_past_limit(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-route-fetch-past-limit";
    let settings = json!({ "network": { "max_body_size": 64 } });
    let (Some(url), Some(page)) = (
        site.named_url("scope.test", "/about?route=fetch-past-limit"),
        page(connection, site, run_id, &settings, checks)?,
    ) else {
        return finish_run(connection, run_id);
    };
    let mut routes = Routes::new(|_| vec![Act::FetchThenFulfill(with_header)]);
    Routes::route(
        connection,
        PAGE_ROUTE.name,
        &json!({ "page_id": page.page_id, "url": glob("**/about*") }),
    )?;
    let answer = goto(&mut routes, connection, &page, &url)?;
    checks.check(
        "the `goto` whose fetched body passed the limit answers",
        answer.get("result").is_some() || answer.get("error").is_some(),
    );
    let made = routes.made(connection)?;
    checks.check(
        format!(
            "`route.fetch` raises `{}` naming `network.max_body_size` and 64",
            RESPONSE_LIMIT_EXCEEDED.name
        ),
        made.first().is_some_and(|(method, answer)| {
            *method == ROUTE_FETCH.name
                && code_of(answer) == &json!(RESPONSE_LIMIT_EXCEEDED.name)
                && answer["error"]["data"]["setting"] == json!("network.max_body_size")
                && answer["error"]["data"]["limit"] == json!(64)
        }),
    );
    finish_run(connection, run_id)
}

/// A route sees the body a page posts, in Base64, as `route.request()`'s
/// `postData()` reads it; a request with none has none.
fn post_data(connection: &mut Connection, site: &Site, checks: &mut Checks) -> Result<(), Gone> {
    let run_id = "conformance-route-post-data";
    let (Some(url), Some(page)) = (
        site.named_url("scope.test", "/about?route=post-data"),
        page(connection, site, run_id, &json!({}), checks)?,
    ) else {
        return finish_run(connection, run_id);
    };
    let mut routes = Routes::new(|_| vec![Act::End]);
    goto(&mut routes, connection, &page, &url)?;
    Routes::route(
        connection,
        PAGE_ROUTE.name,
        &json!({ "page_id": page.page_id, "url": glob("**/about*") }),
    )?;
    let posted = routes.call(
        connection,
        PAGE_EVALUATE.name,
        &json!({
            "page_id": page.page_id,
            "function": "async () => (await fetch('/about?route=posted', { method: 'POST', body: 'a=1&b=2' })).status",
        }),
    )?;
    checks.check(
        "the page's POST goes on past its route and gets an answer",
        posted["result"]["value"].is_number(),
    );
    checks.check(
        "the route sees the posted body in Base64",
        routes.handled.first().is_some_and(|params| {
            params["request"]["method"] == json!("POST")
                && params["request"]["post_data"] == json!(base64("a=1&b=2"))
        }),
    );
    finish_run(connection, run_id)
}
