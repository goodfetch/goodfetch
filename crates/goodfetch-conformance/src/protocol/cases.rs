//! The exchanges the protocol and page suites' case IDs name, each judged
//! against what `goodfetch-protocol` says the protocol is: its JSON-RPC
//! failures, its Error codes and its operations' types. A crawl's delivery
//! cases are `delivery`'s, a page's formats cases `formats`'s and its
//! route cases `routes`'s.

mod delivery;
mod fetch_edge;
mod formats;
mod guard;
mod launch;
mod navigation;
mod routes;

use std::collections::BTreeMap;

use goodfetch_protocol::catalog::{PROTOCOL_VERSION, ProtocolVersion};
use goodfetch_protocol::connection::{
    CALL_CANCELLED, CANCEL, HELLO, HelloParams, HelloResult, PROTOCOL_VERSION_UNSUPPORTED,
    ProductVersion, Surface,
};
use goodfetch_protocol::engine::{ENGINE_INFO, EngineInfo};
use goodfetch_protocol::events::EVENTS_SUBSCRIBE;
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::page::{Navigated, PAGE_GOTO, PageOpened, RUN_NEW_PAGE};
use goodfetch_protocol::result_document::{CaseCheck, OracleAnswer};
use goodfetch_protocol::run::{RUN_FINISH, RUN_START};
use serde_json::{Value, json};

use self::guard::Step;
use super::connection::{Connection, Gone};
use super::target::{DEADLINE, EngineTarget, START_DEADLINE};
use crate::proving_ground::{ProvingGround, TaskList};

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
    FabricatedHello,
    AtLeastOnce,
    ExactlyOnce,
    LateCommit,
    PagesShareTheDefaultSession,
    GotoAnswersTheResponse,
    ErrorStatusesAreResponses,
    HistoryAnswersResponsesAcrossSites,
    BackLoadsTheDocumentAgain,
    UnresolvedNameFailsTheNavigation,
    SupersededNavigationIsInterrupted,
    /// A fetch edge or response decoding case, Collie's lines.
    FetchEdge(fetch_edge::Case),
    MarkdownMatchesTheOracle,
    FormatNotNamedIsLeftOut,
    NoMainContentIsNone,
    HeadlessIdentity,
    BackgroundTimer,
    ClickWaitsOutACover,
    TransientDuplicateIsWaitedOut,
    LocatorSurvivesARerender,
    TypedTextReachesAControlledField,
    KeyReachesAField,
    StillAmbiguous,
    NeverActionable,
    /// A route case.
    Route(routes::Case),
}

/// What a case reaches of the Proving ground: a page of its catalog family,
/// each task's start URL, the origin of its access family, the port it
/// serves on, the `resolve` mapping of its hosts to its address, and the
/// Proving ground itself, whose time step a case may set, whose oracle a
/// case may ask and whose own observations a case may read.
pub(crate) struct Site {
    page_url: Option<String>,
    start_urls: BTreeMap<String, String>,
    access_origin: Option<String>,
    port: String,
    resolve: Value,
    proving_ground: ProvingGround,
}

impl Site {
    /// The site of `task_list`, served by `proving_ground`.
    pub(crate) fn of(task_list: &TaskList, proving_ground: &ProvingGround) -> Self {
        let address = proving_ground.address();
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
        let access_origin = task_list
            .tasks
            .iter()
            .find(|task| task.family == "access")
            .and_then(|task| {
                let rest = task.start_url.strip_prefix("http://")?;
                let authority = rest.split('/').next()?;
                Some(format!("http://{authority}"))
            });
        let port = address
            .rsplit_once(':')
            .map_or("80", |(_, port)| port)
            .to_owned();
        let mut resolve = serde_json::Map::new();
        for task in &task_list.tasks {
            if let Some(host) = host_of(&task.start_url) {
                resolve.insert(host.to_owned(), json!([ip]));
            }
        }
        for host in NAMED_HOSTS {
            resolve.insert(host.to_owned(), json!([ip]));
        }
        // The access family's secure host, which no task names: a browser
        // would resolve it to every loopback address, the Proving ground's
        // among them, and this keeps it on that one.
        resolve.insert(launch::IDENTITY_HOST.to_owned(), json!([ip]));
        Self {
            page_url,
            start_urls: task_list
                .tasks
                .iter()
                .map(|task| (task.id.as_str().to_owned(), task.start_url.clone()))
                .collect(),
            access_origin,
            port,
            resolve: Value::Object(resolve),
            proving_ground: proving_ground.clone(),
        }
    }

    /// The start URL of the task `id`, such as `statuses/1`.
    fn start_url(&self, id: &str) -> Option<&str> {
        self.start_urls.get(id).map(String::as_str)
    }

    /// The plain HTTP URL of `path` on `host`, one of the Proving ground's
    /// host names that belong to no task, on its port.
    fn named_url(&self, host: &str, path: &str) -> Option<String> {
        let (_, port) = self.proving_ground.address().rsplit_once(':')?;
        Some(format!("http://{host}:{port}{path}"))
    }
}

/// The Proving ground's host names that belong to no task and that a case
/// reaches, which `resolve` maps too: the fetch edge's (ADR 0118), and the
/// scope family's site and off-site host the route cases load.
const NAMED_HOSTS: [&str; 3] = ["fetch-edge.test", "scope.test", "elsewhere.test"];

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

/// The checks a case made so far, and the oracle's answers it was given.
#[derive(Default)]
pub(crate) struct Checks {
    pub(crate) checks: Vec<CaseCheck>,
    pub(crate) oracle_answers: Vec<OracleAnswer>,
}

impl Checks {
    fn check(&mut self, name: impl Into<String>, passed: bool) {
        self.checks.push(CaseCheck {
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
            "a-fabricated-hello-is-refused-before-any-other-message-and-the-connection-closes" => {
                Self::FabricatedHello
            }
            "a-work-item-whose-lease-ends-unanswered-is-delivered-again" => Self::AtLeastOnce,
            "a-work-item-answered-in-time-commits-once" => Self::ExactlyOnce,
            "an-answer-after-the-lease-ended-is-a-late-commit-and-refused" => Self::LateCommit,
            "pages-open-in-the-runs-default-session" => Self::PagesShareTheDefaultSession,
            "goto-answers-the-documents-response" => Self::GotoAnswersTheResponse,
            "an-error-status-is-the-documents-response" => Self::ErrorStatusesAreResponses,
            "back-and-forward-across-sites-answer-the-documents-response" => {
                Self::HistoryAnswersResponsesAcrossSites
            }
            "going-back-loads-the-document-again" => Self::BackLoadsTheDocumentAgain,
            "a-name-that-does-not-resolve-fails-apart-from-a-refused-connection" => {
                Self::UnresolvedNameFailsTheNavigation
            }
            "a-navigation-superseded-before-it-committed-is-interrupted" => {
                Self::SupersededNavigationIsInterrupted
            }
            "markdown-matches-the-oracle-on-the-markdown-pages" => Self::MarkdownMatchesTheOracle,
            "a-format-the-request-does-not-name-is-left-out" => Self::FormatNotNamedIsLeftOut,
            "a-page-with-no-main-content-answers-empty-markdown-and-none" => {
                Self::NoMainContentIsNone
            }
            "a-headless-page-shows-the-site-its-windows-identity" => Self::HeadlessIdentity,
            "a-page-behind-another-keeps-its-timer-and-its-focus" => Self::BackgroundTimer,
            "a-click-on-a-covered-target-lands-once-the-cover-leaves" => Self::ClickWaitsOutACover,
            "an-ambiguous-target-that-becomes-unique-is-acted-on" => {
                Self::TransientDuplicateIsWaitedOut
            }
            "a-locator-survives-a-re-render-that-replaces-its-element" => {
                Self::LocatorSurvivesARerender
            }
            "typed-text-reaches-a-controlled-field-as-trusted-input" => {
                Self::TypedTextReachesAControlledField
            }
            "a-pressed-key-reaches-a-field-as-a-trusted-key" => Self::KeyReachesAField,
            "a-target-still-ambiguous-raises-target-not-unique" => Self::StillAmbiguous,
            "a-target-never-actionable-raises-deadline-exceeded-naming-the-step" => {
                Self::NeverActionable
            }
            _ => {
                return fetch_edge::Case::from_id(id)
                    .map(Self::FetchEdge)
                    .or_else(|| routes::Case::from_id(id).map(Self::Route));
            }
        })
    }

    /// Runs the case on the open Connection to `target`, whose `hello` the
    /// engine answered with `hello`, against `site`.
    pub(crate) fn run(
        self,
        connection: &mut Connection,
        target: &EngineTarget,
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
            Self::FabricatedHello => fabricated_hello(connection, target, hello, checks),
            Self::AtLeastOnce => delivery::at_least_once(connection, checks),
            Self::ExactlyOnce => delivery::exactly_once(connection, checks),
            Self::LateCommit => delivery::late_commit(connection, checks),
            Self::PagesShareTheDefaultSession => default_session(connection, site, checks),
            Self::GotoAnswersTheResponse => goto(connection, site, checks),
            Self::ErrorStatusesAreResponses => navigation::error_statuses(connection, site, checks),
            Self::HistoryAnswersResponsesAcrossSites => {
                navigation::history_across_sites(connection, site, checks)
            }
            Self::BackLoadsTheDocumentAgain => {
                navigation::back_loads_again(connection, site, checks)
            }
            Self::UnresolvedNameFailsTheNavigation => {
                navigation::unresolved_name(connection, site, checks)
            }
            Self::SupersededNavigationIsInterrupted => {
                navigation::superseded(connection, site, checks)
            }
            Self::FetchEdge(case) => case.run(connection, site, checks),
            Self::MarkdownMatchesTheOracle => {
                formats::markdown_matches_the_oracle(connection, site, checks)
            }
            Self::FormatNotNamedIsLeftOut => {
                formats::a_format_not_named_is_left_out(connection, site, checks)
            }
            Self::NoMainContentIsNone => formats::no_main_content_is_none(connection, site, checks),
            Self::HeadlessIdentity => launch::headless_identity(connection, site, checks),
            Self::BackgroundTimer => launch::background_timer(connection, site, checks),
            Self::ClickWaitsOutACover => guard::lands(
                connection,
                site,
                "actionability/2",
                "conformance-cover",
                &[Step::Click("#target")],
                checks,
            ),
            Self::TransientDuplicateIsWaitedOut => guard::lands(
                connection,
                site,
                "actionability/27",
                "conformance-transient-duplicate",
                &[Step::Click("#target")],
                checks,
            ),
            Self::LocatorSurvivesARerender => guard::lands(
                connection,
                site,
                "actionability/6",
                "conformance-rerender",
                &[Step::Click("#target")],
                checks,
            ),
            Self::TypedTextReachesAControlledField => guard::lands(
                connection,
                site,
                "actionability/28",
                "conformance-controlled-field",
                &[Step::Fill("#name", "Goodfetch"), Step::Click("#save")],
                checks,
            ),
            Self::KeyReachesAField => guard::lands(
                connection,
                site,
                "actionability/15",
                "conformance-key",
                &[Step::Press("#field", "Enter")],
                checks,
            ),
            Self::StillAmbiguous => guard::still_ambiguous(connection, site, checks),
            Self::NeverActionable => guard::never_actionable(connection, site, checks),
            Self::Route(case) => case.run(connection, site, checks),
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

/// On a Connection of its own, a `hello` naming a protocol major the engine
/// cannot speak, followed at once by another call, is refused with
/// `protocol_version_unsupported` before any other message, and the engine
/// then closes the Connection (ADR 0066).
fn fabricated_hello(
    connection: &mut Connection,
    target: &EngineTarget,
    hello: &HelloResult,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let mut other = connection
        .another(target)
        .map_err(|error| Gone::Failed(error.to_string()))?;
    let params = HelloParams {
        protocol_version: ProtocolVersion {
            major: PROTOCOL_VERSION.major + 1,
            minor: 0,
        },
        surface: Surface::Rust,
        form: None,
        token: other.token().map(ToOwned::to_owned),
        needs: Some(hello.product),
        product_version: ProductVersion::parse(env!("CARGO_PKG_VERSION"))
            .expect("invariant: the workspace's version is a Product version"),
        log_level: None,
    };
    let refused = (|| {
        let id = other.request(
            HELLO,
            &serde_json::to_value(params).expect("invariant: `hello` serializes"),
        )?;
        other.request(ENGINE_INFO.name, &json!({}))?;
        let first = other.next_within(START_DEADLINE)?;
        checks.check(
            "the first message answers the `hello`",
            first.get("method").is_none() && first["id"] == json!(id),
        );
        checks.check(
            format!("it refuses it with `{}`", PROTOCOL_VERSION_UNSUPPORTED.name),
            first["error"]["data"]["code"] == json!(PROTOCOL_VERSION_UNSUPPORTED.name),
        );
        checks.check(
            "nothing else is answered and the engine closes the Connection",
            other.next_within(DEADLINE) == Err(Gone::Closed),
        );
        Ok(())
    })();
    connection.keep(other.take_exchanged());
    other.close();
    refused
}

/// Starts the Run `run_id` with `settings`, its `network.resolve` mapping
/// the site's hosts.
pub(crate) fn start_run(
    connection: &mut Connection,
    site: &Site,
    run_id: &str,
    settings: &Value,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let mut settings = settings.clone();
    settings["network"] = json!({ "resolve": site.resolve });
    let id = connection.request(
        RUN_START.name,
        &json!({ "run_id": run_id, "settings": settings }),
    )?;
    let answer = connection.answer(id)?.message;
    checks.check("the Run starts", answer.get("result").is_some());
    Ok(())
}

/// Opens a page of the Run `run_id`.
pub(crate) fn new_page(
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

pub(crate) fn finish_run(connection: &mut Connection, run_id: &str) -> Result<(), Gone> {
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
    start_run(connection, site, run_id, &json!({}), checks)?;
    let first = new_page(connection, run_id, checks)?;
    let second = new_page(connection, run_id, checks)?;
    if let (Some(first), Some(second)) = (first, second) {
        checks.check(
            "the second page is in the first page's Session",
            first.session_id == second.session_id,
        );
        checks.check("the two pages are two", first.page_id != second.page_id);
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
    start_run(connection, site, run_id, &json!({}), checks)?;
    if let Some(page) = new_page(connection, run_id, checks)? {
        let id = connection.request(
            PAGE_GOTO.name,
            &json!({ "page_id": page.page_id, "url": url }),
        )?;
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
