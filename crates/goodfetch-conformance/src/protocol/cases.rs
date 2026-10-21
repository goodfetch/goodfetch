//! The exchanges the protocol and page suites' case IDs name, each judged
//! against what `goodfetch-protocol` says the protocol is: its JSON-RPC
//! failures, its Error codes and its operations' types.

use goodfetch_protocol::catalog::PROTOCOL_VERSION;
use goodfetch_protocol::connection::{CALL_CANCELLED, CANCEL, HelloResult};
use goodfetch_protocol::engine::{ENGINE_INFO, EngineInfo};
use goodfetch_protocol::events::EVENTS_SUBSCRIBE;
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::page::{Navigated, PAGE_GOTO, PageOpened, RUN_NEW_PAGE};
use goodfetch_protocol::result_document::CaseCheck;
use goodfetch_protocol::run::{RUN_FINISH, RUN_START};
use serde_json::{Value, json};

use super::connection::{Connection, Gone};
use crate::proving_ground::TaskList;

/// A method no Product carries, which every target must refuse.
const UNKNOWN_METHOD: &str = "conformance.unknown_method";

/// One case the protocol suite can declare.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Case {
    Hello,
    EngineInfo,
    UnknownMethod,
    InvalidParams,
    CancelInFlight,
    CancelAnswered,
    PagesShareTheDefaultSession,
    GotoAnswersTheResponse,
}

/// What a case reaches of the Proving ground: a page of its catalog family
/// and the `resolve` mapping of its hosts to the Proving ground's address.
pub(crate) struct Site {
    page_url: Option<String>,
    resolve: Value,
}

impl Site {
    /// The site of `task_list`, served at `address` (`host:port`).
    pub(crate) fn of(task_list: &TaskList, address: &str) -> Self {
        let ip = address
            .rsplit_once(':')
            .map_or(address, |(ip, _)| ip)
            .trim_start_matches('[')
            .trim_end_matches(']');
        let page_url = task_list
            .tasks
            .iter()
            .find(|task| task.family == "catalog")
            .map(|task| task.start_url.clone());
        let mut resolve = serde_json::Map::new();
        for task in &task_list.tasks {
            if let Some(host) = host_of(&task.start_url) {
                resolve.insert(host.to_owned(), json!([ip]));
            }
        }
        Self {
            page_url,
            resolve: Value::Object(resolve),
        }
    }
}

/// The host name of an `http` URL.
fn host_of(url: &str) -> Option<&str> {
    let rest = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))?;
    let authority = rest.split('/').next()?;
    Some(
        authority
            .rsplit_once(':')
            .map_or(authority, |(host, _)| host),
    )
}

/// The checks a case made so far.
#[derive(Default)]
pub(crate) struct Checks(pub(crate) Vec<CaseCheck>);

impl Checks {
    fn check(&mut self, name: impl Into<String>, passed: bool) {
        self.0.push(CaseCheck {
            name: name.into(),
            passed,
        });
    }
}

impl Case {
    /// The case a suite's case ID names.
    pub(crate) fn of(id: &str) -> Option<Self> {
        Some(match id {
            "hello-is-answered-with-the-engines-product" => Self::Hello,
            "engine-info-names-the-engine-hello-named" => Self::EngineInfo,
            "an-unknown-method-is-method-not-found" => Self::UnknownMethod,
            "params-the-operation-does-not-take-are-invalid-params" => Self::InvalidParams,
            "a-call-in-flight-cancelled-ends-with-call-cancelled" => Self::CancelInFlight,
            "cancelling-an-answered-call-leaves-the-connection-answering" => Self::CancelAnswered,
            "pages-open-in-the-runs-default-session" => Self::PagesShareTheDefaultSession,
            "goto-answers-the-documents-response" => Self::GotoAnswersTheResponse,
            _ => return None,
        })
    }

    /// Runs the case on the open Connection, whose `hello` the engine
    /// answered with `hello`, against `site`.
    pub(crate) fn run(
        self,
        connection: &mut Connection,
        hello: &HelloResult,
        site: &Site,
        checks: &mut Checks,
    ) -> Result<(), Gone> {
        match self {
            Self::Hello => {
                checks.check(
                    format!(
                        "the engine speaks the protocol's major {}",
                        PROTOCOL_VERSION.major
                    ),
                    hello.protocol_version.major == PROTOCOL_VERSION.major,
                );
                Ok(())
            }
            Self::EngineInfo => engine_info(connection, hello, checks),
            Self::UnknownMethod => {
                let id = connection.request(UNKNOWN_METHOD, &json!({}))?;
                let answer = connection.answer(id)?.message;
                refused_as(&answer, ProtocolFailure::MethodNotFound, checks);
                Ok(())
            }
            Self::InvalidParams => {
                let id = connection.request(ENGINE_INFO.name, &json!({ "unknown": true }))?;
                let answer = connection.answer(id)?.message;
                refused_as(&answer, ProtocolFailure::InvalidParams, checks);
                Ok(())
            }
            Self::CancelInFlight => cancel_in_flight(connection, checks),
            Self::CancelAnswered => cancel_answered(connection, checks),
            Self::PagesShareTheDefaultSession => default_session(connection, site, checks),
            Self::GotoAnswersTheResponse => goto(connection, site, checks),
        }
    }
}

fn engine_info(
    connection: &mut Connection,
    hello: &HelloResult,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let id = connection.request(ENGINE_INFO.name, &json!({}))?;
    let answer = connection.answer(id)?.message;
    let info = answer
        .get("result")
        .and_then(|result| serde_json::from_value::<EngineInfo>(result.clone()).ok());
    checks.check(
        "`engine.info` answers with what its result type holds",
        info.is_some(),
    );
    if let Some(info) = info {
        checks.check(
            "it names the Product `hello` named",
            info.product == hello.product,
        );
        checks.check(
            "it names the Product version `hello` named",
            info.product_version == hello.product_version,
        );
        checks.check(
            "it names the protocol version `hello` named",
            info.protocol_version == hello.protocol_version,
        );
        checks.check(
            "its Product major is its Product version's",
            info.product_major == info.product_version.major(),
        );
    }
    Ok(())
}

/// Checks that `answer` is the refusal JSON-RPC predefines for `failure`,
/// its `data.code` naming it.
fn refused_as(answer: &Value, failure: ProtocolFailure, checks: &mut Checks) {
    let error = &answer["error"];
    checks.check(
        format!(
            "the call is refused with JSON-RPC's code {}",
            failure.code()
        ),
        error["code"] == json!(failure.code()),
    );
    checks.check(
        format!("its `data.code` is `{}`", failure.name()),
        error["data"]["code"] == json!(failure.name()),
    );
}

/// A subscription stays in flight until its Client cancels it (ADR 0080),
/// then ends with `call_cancelled`, stopped by the Client (ADR 0113).
fn cancel_in_flight(connection: &mut Connection, checks: &mut Checks) -> Result<(), Gone> {
    let id = connection.request(EVENTS_SUBSCRIBE.name, &json!({}))?;
    connection.notify(CANCEL, &json!({ "id": id }))?;
    let error = &connection.answer(id)?.message["error"];
    checks.check(
        format!(
            "the call ends with the `{}` group's code {}",
            CALL_CANCELLED.group.as_str(),
            CALL_CANCELLED.group.json_rpc_class()
        ),
        error["code"] == json!(CALL_CANCELLED.group.json_rpc_class()),
    );
    checks.check(
        format!("its `data.code` is `{}`", CALL_CANCELLED.name),
        error["data"]["code"] == json!(CALL_CANCELLED.name),
    );
    checks.check(
        "it says no retry succeeds",
        error["data"]["retryable"] == json!(CALL_CANCELLED.retryable),
    );
    checks.check(
        "it says the Client stopped it",
        error["data"]["by"] == json!("client"),
    );
    Ok(())
}

/// A `$/cancel` of a call already answered gets no answer of its own, and
/// the next call is answered.
fn cancel_answered(connection: &mut Connection, checks: &mut Checks) -> Result<(), Gone> {
    let answered = connection.request(ENGINE_INFO.name, &json!({}))?;
    connection.answer(answered)?;
    connection.notify(CANCEL, &json!({ "id": answered }))?;
    let next = connection.request(ENGINE_INFO.name, &json!({}))?;
    let answer = connection.answer(next)?;
    checks.check(
        "the next call is answered with a result",
        answer.message.get("result").is_some(),
    );
    checks.check(
        "the cancel gets no answer of its own",
        answer
            .others
            .iter()
            .all(|message| message.get("id") != Some(&json!(answered))),
    );
    Ok(())
}

/// Starts the Run `run_id`, its `network.resolve` mapping the site's hosts.
fn start_run(
    connection: &mut Connection,
    site: &Site,
    run_id: &str,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let id = connection.request(
        RUN_START.name,
        &json!({ "run_id": run_id, "settings": { "network": { "resolve": site.resolve } } }),
    )?;
    let answer = connection.answer(id)?.message;
    checks.check("the Run starts", answer.get("result").is_some());
    Ok(())
}

/// Opens a page of the Run `run_id`.
fn new_page(
    connection: &mut Connection,
    run_id: &str,
    checks: &mut Checks,
) -> Result<Option<PageOpened>, Gone> {
    let id = connection.request(RUN_NEW_PAGE.name, &json!({ "run_id": run_id }))?;
    let answer = connection.browser_answer(id)?.message;
    let opened = answer
        .get("result")
        .and_then(|result| serde_json::from_value::<PageOpened>(result.clone()).ok());
    checks.check(
        format!(
            "`{}` answers with what its result type holds",
            RUN_NEW_PAGE.name
        ),
        opened.is_some(),
    );
    Ok(opened)
}

fn finish_run(connection: &mut Connection, run_id: &str) -> Result<(), Gone> {
    let id = connection.request(RUN_FINISH.name, &json!({ "run_id": run_id }))?;
    connection.browser_answer(id)?;
    Ok(())
}

/// Two pages of a Run open in one Session, its default (ADR 0092).
fn default_session(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-default-session";
    start_run(connection, site, run_id, checks)?;
    let first = new_page(connection, run_id, checks)?;
    let second = new_page(connection, run_id, checks)?;
    if let (Some(first), Some(second)) = (first, second) {
        checks.check(
            "the second page is in the first page's Session",
            first.session == second.session,
        );
        checks.check("the two pages are two", first.page != second.page);
    }
    finish_run(connection, run_id)
}

/// `page.goto` to a Proving ground page answers its document's response.
fn goto(connection: &mut Connection, site: &Site, checks: &mut Checks) -> Result<(), Gone> {
    let run_id = "conformance-goto";
    checks.check(
        "the Proving ground serves a catalog page",
        site.page_url.is_some(),
    );
    let Some(url) = site.page_url.clone() else {
        return Ok(());
    };
    start_run(connection, site, run_id, checks)?;
    if let Some(page) = new_page(connection, run_id, checks)? {
        let id = connection.request(PAGE_GOTO.name, &json!({ "page": page.page, "url": url }))?;
        let answer = connection.browser_answer(id)?.message;
        let response = answer
            .get("result")
            .and_then(|result| serde_json::from_value::<Navigated>(result.clone()).ok())
            .and_then(|navigated| navigated.response);
        checks.check(
            format!("`{}` answers the document's response", PAGE_GOTO.name),
            response.is_some(),
        );
        if let Some(response) = response {
            checks.check("its status is 200", response.status == 200);
            checks.check("its URL is the page's", response.url == url);
        }
    }
    finish_run(connection, run_id)
}
