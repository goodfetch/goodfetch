//! The exchanges the protocol and page suites' case IDs name, each judged
//! against what `goodfetch-protocol` says the protocol is: its JSON-RPC
//! failures, its Error codes and its operations' types. A crawl's delivery
//! cases are `delivery`'s, a page's formats cases `formats`'s, its route
//! cases `routes`'s, a Run's Session cases `sessions`', their proxy cases
//! `proxies`' and `proxy_limits`', its events and bodies cases `events`'s, its `page.request`
//! cases `page_request`'s, its dialog cases `dialogs`', its popup cases
//! `popups`', its page function cases `page_functions`', its Page check
//! cases `page_check`'s, its robots.txt reading cases `robots_txt`'s, its
//! hostile-input cases `hostile_input`'s, its `page.read` cases `read`'s,
//! the reset gate's cases `reset`'s, the start table's `run_start`'s and
//! the fencing of a Run's two copies `two_copies`'.
//!
//! Size: one concern, the table of every case the suites name: each ID to
//! its case, each case to the module that runs it, and the Site and checks
//! they share, which change together as a case is added.

mod blocking_off;
mod blocks;
mod caller_secrets;
mod clean_up;
mod consent;
mod delivery;
mod dialogs;
mod events;
mod exchange;
mod fetch_edge;
mod fetch_tiers;
mod formats;
mod guard;
mod hostile_input;
mod launch;
mod navigation;
mod page_check;
mod page_functions;
mod page_request;
mod poll;
mod popups;
mod proxies;
mod proxy_limits;
mod read;
mod reset;
mod robots_txt;
mod routes;
mod run_start;
mod run_trace;
mod service_workers;
mod sessions;
mod two_copies;
mod waits;

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fmt;

use goodfetch_protocol::catalog::{PROTOCOL_VERSION, ProtocolVersion};
use goodfetch_protocol::connection::{
    CALL_CANCELLED, CANCEL, HelloParams, HelloResult, PROTOCOL_VERSION_UNSUPPORTED, Surface,
};
use goodfetch_protocol::engine::{ENGINE_INFO, EngineInfo};
use goodfetch_protocol::events::EVENTS_SUBSCRIBE;
use goodfetch_protocol::formats::USER_CONFIGURATION;
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::page::{Navigated, PAGE_GOTO, PageOpened, RUN_NEW_PAGE};
use goodfetch_protocol::result_document::{CaseCheck, OracleAnswer};
use goodfetch_protocol::run::{RUN_FINISH, RUN_START};
use serde_json::{Value, json};

use self::guard::Step;
use super::connection::{Connection, Gone, code_of};
use super::fabricated;
use super::target::{DEADLINE, EngineTarget};
use crate::proving_ground::{
    ProvingGround, ProvingGroundError, ProxyCredentials, ReceivedRequests, TaskAccount, TaskList,
};

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
    WaitsArmedBeforeBackPass,
    UnresolvedNameFailsTheNavigation,
    SupersededNavigationIsInterrupted,
    NavigationPastItsDeadline,
    /// A fetch edge or response decoding case, Collie's lines.
    FetchEdge(fetch_edge::Case),
    MarkdownMatchesTheOracle,
    FormatNotNamedIsLeftOut,
    NoMainContentIsNone,
    TextRenderedAfterLoadIsRead,
    ClosedShadowRootTextIsRead,
    RulesTouchOnlyTheMainContent,
    EachFormatsCallIsInTheRunTrace,
    EveryFormMatchesTheOracle,
    RawHtmlIsContentBeforeTheRules,
    FormatsDefaultsAndUnknownOptions,
    /// A links, images, media and tables case (#786).
    References(formats::references::Case),
    /// A metadata and contacts case (#787).
    Structured(formats::structured::Case),
    /// A `content.extractor` case (#788).
    Extractor(formats::extractor::Case),
    HeadlessIdentity,
    BackgroundTimer,
    ClickWaitsOutACover,
    TransientDuplicateIsWaitedOut,
    LocatorSurvivesARerender,
    TypedTextReachesAControlledField,
    KeyReachesAField,
    StillAmbiguous,
    NeverActionable,
    BusyPage,
    /// A Clean-up case.
    CleanUp(clean_up::Case),
    /// A Consent prompt case.
    Consent(consent::Case),
    /// A route case.
    Route(routes::Case),
    /// A Session case, its proxy and Page check cases among them.
    Session(sessions::Case),
    /// A page events or bodies case.
    Events(events::Case),
    /// A reset gate case.
    Reset(reset::Case),
    /// A start table case.
    RunStart(run_start::Case),
    /// A case of a Run's two copies, one fenced.
    TwoCopies(two_copies::Case),
    /// A service worker case.
    ServiceWorker(service_workers::Case),
    /// A `page.request` case, the hostile-input cases among them.
    PageRequest(page_request::Case),
    /// A fetch tier case, Foxhound's.
    FetchTier(fetch_tiers::Case),
    /// A dialog case.
    Dialog(dialogs::Case),
    /// A popup case.
    Popup(popups::Case),
    /// A page function case.
    PageFunction(page_functions::Case),
    /// A `page.read` case.
    Read(read::Case),
    /// An app-state wait case.
    Wait(waits::Case),
    /// A robots.txt reading case.
    RobotsTxt(robots_txt::Case),
}

/// What a case reaches of the Proving ground: a page of its catalog family,
/// each task's start URL, the origin of its access family, the port it
/// serves on, the `resolve` mapping of its hosts to its address, and the
/// Proving ground itself, whose time step a case may set, whose oracle a
/// case may ask and whose own observations a case may read.
pub(crate) struct Site {
    page_url: Option<String>,
    start_urls: BTreeMap<String, String>,
    /// Each task's account, by its ID, for the tasks that log in.
    accounts: BTreeMap<String, TaskAccount>,
    access_origin: Option<String>,
    port: String,
    resolve: Value,
    /// Each forward proxy's address, by name, when the Proving ground
    /// serves them.
    proxies: BTreeMap<String, String>,
    /// The credentials its proxies that ask for them take, when it serves
    /// any.
    proxy_credentials: Option<ProxyCredentials>,
    proving_ground: ProvingGround,
    /// What a case expected of the Proving ground and could not read, with
    /// why, since the suite last took it: each fails the case.
    unread: RefCell<Vec<String>>,
}

impl Site {
    /// The site of `task_list`, served by `proving_ground`.
    ///
    /// # Errors
    ///
    /// When the Proving ground cannot say which network fixtures it
    /// serves.
    pub(crate) fn of(
        task_list: &TaskList,
        proving_ground: &ProvingGround,
    ) -> Result<Self, ProvingGroundError> {
        let fixtures = proving_ground.network_fixtures()?;
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
        Ok(Self {
            page_url,
            start_urls: task_list
                .tasks
                .iter()
                .map(|task| (task.id.as_str().to_owned(), task.start_url.clone()))
                .collect(),
            accounts: task_list
                .tasks
                .iter()
                .filter_map(|task| Some((task.id.as_str().to_owned(), task.account.clone()?)))
                .collect(),
            access_origin,
            port,
            resolve: Value::Object(resolve),
            proxies: fixtures
                .as_ref()
                .map(|fixtures| fixtures.proxies.clone())
                .unwrap_or_default(),
            proxy_credentials: fixtures.and_then(|fixtures| fixtures.proxy_credentials),
            proving_ground: proving_ground.clone(),
            unread: RefCell::new(Vec::new()),
        })
    }

    /// The requests the Proving ground's page hosts received; none when it
    /// cannot list them, which fails the case naming why.
    pub(crate) fn received_requests(&self) -> ReceivedRequests {
        self.proving_ground
            .received_requests()
            .unwrap_or_else(|why| {
                self.unread(format!(
                    "the Proving ground lists the requests its page hosts received ({why})"
                ));
                ReceivedRequests::default()
            })
    }

    /// The Proving ground's own observations; none when it cannot serve
    /// them, which fails the case naming why.
    pub(crate) fn observations(&self) -> Option<Value> {
        self.proving_ground
            .observations()
            .map_err(|why| {
                self.unread(format!(
                    "the Proving ground's observations can be read ({why})"
                ));
            })
            .ok()
    }

    /// Notes, once, that the case could not read `what` of the Proving
    /// ground.
    fn unread(&self, what: String) {
        let mut unread = self.unread.borrow_mut();
        if !unread.contains(&what) {
            unread.push(what);
        }
    }

    /// What the case just run expected of the Proving ground and could not
    /// read, each a failed check naming why.
    pub(crate) fn take_unread(&self) -> Vec<CaseCheck> {
        self.unread
            .take()
            .into_iter()
            .map(|name| CaseCheck {
                name,
                passed: false,
            })
            .collect()
    }

    /// `settings` with the site's `network.resolve` mapping, and, unless
    /// `for_model` or `settings` list ranges, its hosts listed in
    /// `egress.private_ranges`: the egress relay judges a mapping's answers
    /// as any other (spec #563's reading of #325), so a case reaches the
    /// Proving ground's loopback address by listing its hosts, the
    /// resolved-address check left on, and `localhost` and literal
    /// loopback addresses stay refused. A Client acting for a model never
    /// sets the key; its cases list the hosts in user configuration
    /// ([`Site::configured_hosts`]).
    pub(crate) fn mapped(&self, settings: &Value, for_model: bool) -> Value {
        let mut settings = settings.clone();
        if !settings.is_object() {
            settings = json!({});
        }
        if !settings["network"].is_object() {
            settings["network"] = json!({});
        }
        settings["network"]["resolve"] = self.resolve.clone();
        if for_model {
            return settings;
        }
        if !settings["egress"].is_object() {
            settings["egress"] = json!({});
        }
        if settings["egress"].get("private_ranges").is_none() {
            settings["egress"]["private_ranges"] = json!(self.hosts());
        }
        settings
    }

    /// The hosts the site's mapping names.
    fn hosts(&self) -> Vec<&String> {
        self.resolve
            .as_object()
            .into_iter()
            .flat_map(serde_json::Map::keys)
            .collect()
    }

    /// The user configuration that lists the site's hosts in
    /// `egress.private_ranges`, for a Client acting for a model, which
    /// never sets the key itself.
    pub(crate) fn configured_hosts(&self) -> String {
        let hosts: Vec<String> = self
            .hosts()
            .iter()
            .map(|host| format!("\"{host}\""))
            .collect();
        user_configuration(&format!(
            "[settings.egress]\nprivate_ranges = [{}]\n",
            hosts.join(", ")
        ))
    }

    /// The credentials the Proving ground's proxies that ask for them
    /// take, if it serves any.
    pub(crate) fn proxy_credentials(&self) -> Option<&ProxyCredentials> {
        self.proxy_credentials.as_ref()
    }

    /// The address of the Proving ground's forward proxy `name`, such as
    /// `open`, if it serves one.
    fn proxy(&self, name: &str) -> Option<&str> {
        self.proxies.get(name).map(String::as_str)
    }

    /// The start URL of the task `id`, such as `statuses/1`.
    fn start_url(&self, id: &str) -> Option<&str> {
        self.start_urls.get(id).map(String::as_str)
    }

    /// The account the task `id` logs in with, such as `portal/2`'s.
    fn account(&self, id: &str) -> Option<&TaskAccount> {
        self.accounts.get(id)
    }

    /// The plain HTTP URL of `path` on `host`, one of the Proving ground's
    /// host names that belong to no task, on its port.
    fn named_url(&self, host: &str, path: &str) -> String {
        format!("http://{host}:{}{path}", self.port)
    }

    /// [`Site::named_url`] over TLS, which the same port serves.
    fn named_secure_url(&self, host: &str, path: &str) -> String {
        format!("https://{host}:{}{path}", self.port)
    }
}

/// A user configuration file, at the format's version, that sets
/// `settings`, its TOML tables.
pub(crate) fn user_configuration(settings: &str) -> String {
    format!("version = \"{}\"\n\n{settings}", USER_CONFIGURATION.version)
}

/// The Proving ground's host names that belong to no task and that a case
/// reaches, which `resolve` maps too: the fetch edge's (ADR 0118), and the
/// scope family's site and off-site host the route cases load, the
/// Clean-up pages' two hosts and the payment script's (ADR 0047), the
/// consent pages' two hosts (ADR 0048), the `body-limits` family's host
/// whose robots.txt is past the size limit, the `access-policy` family's
/// host whose robots.txt answers late, and the `bombs` family's own
/// host, whose zstd frame asks for a window past 8 MB (ADR 0067); and the
/// delivery cases' plain host, which an engine would otherwise look up.
const NAMED_HOSTS: [&str; 12] = [
    delivery::PLAIN_HOST,
    "fetch-edge.test",
    "scope.test",
    "elsewhere.test",
    clean_up::HOST,
    clean_up::FRAME_HOST,
    clean_up::PAYMENT_HOST,
    consent::HOST,
    consent::FRAME_HOST,
    robots_txt::PAST_THE_SIZE_HOST,
    robots_txt::SLOW_ROBOTS_HOST,
    hostile_input::BOMBS_HOST,
];

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

    /// `found`, checked to be read: a check named `expected`, which fails
    /// naming why when the case could not read it.
    fn read<T>(&mut self, expected: &str, found: Result<T, impl fmt::Display>) -> Option<T> {
        match found {
            Ok(found) => {
                self.check(expected, true);
                Some(found)
            }
            Err(why) => {
                self.check(format!("{expected} ({why})"), false);
                None
            }
        }
    }

    /// Checks that `answer`, to a call the case makes on its way to what it
    /// claims, is a result: a failed check naming `call` and the failure
    /// when it is not, and none when it is.
    fn answered(&mut self, call: &str, answer: &Value) {
        if let Some(error) = answer.get("error") {
            self.check(format!("`{call}` answers with a result ({error})"), false);
        }
    }

    /// `found`, or, when the case could not read what it needs, a failed
    /// check that names what it expected and why it has none.
    fn needs<T>(&mut self, expected: &str, found: Result<T, impl fmt::Display>) -> Option<T> {
        match found {
            Ok(found) => Some(found),
            Err(why) => {
                self.check(format!("{expected} ({why})"), false);
                None
            }
        }
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
            "a-wait-armed-before-going-back-passes" => Self::WaitsArmedBeforeBackPass,
            "a-name-that-does-not-resolve-fails-apart-from-a-refused-connection" => {
                Self::UnresolvedNameFailsTheNavigation
            }
            "a-navigation-superseded-before-it-committed-is-interrupted" => {
                Self::SupersededNavigationIsInterrupted
            }
            "a-navigation-whose-document-does-not-load-in-time-raises-deadline-exceeded" => {
                Self::NavigationPastItsDeadline
            }
            "markdown-matches-the-oracle-on-the-markdown-pages" => Self::MarkdownMatchesTheOracle,
            "a-format-the-request-does-not-name-is-left-out" => Self::FormatNotNamedIsLeftOut,
            "a-page-with-no-main-content-answers-empty-markdown-and-none" => {
                Self::NoMainContentIsNone
            }
            "text-a-script-renders-after-load-and-an-open-shadow-roots-text-are-in-the-markdown" => {
                Self::TextRenderedAfterLoadIsRead
            }
            "a-closed-shadow-roots-text-is-in-the-markdown-on-chromium" => {
                Self::ClosedShadowRootTextIsRead
            }
            "an-ad-a-cosmetic-rule-hides-stays-in-the-document-and-a-locator-finds-it-but-not-in-the-markdown" => {
                Self::RulesTouchOnlyTheMainContent
            }
            "the-run-trace-holds-each-formats-call-with-the-rules-version-and-what-they-removed" => {
                Self::EachFormatsCallIsInTheRunTrace
            }
            "each-markdown-content-html-kind-and-the-text-match-the-oracle-and-fit-is-shorter-than-main" => {
                Self::EveryFormMatchesTheOracle
            }
            "raw-html-is-what-content-returns-and-a-hidden-ad-is-in-it-but-not-in-cleaned-html-or-markdown" => {
                Self::RawHtmlIsContentBeforeTheRules
            }
            "markdown-and-html-default-to-the-main-content-and-an-unknown-option-is-invalid-params" => {
                Self::FormatsDefaultsAndUnknownOptions
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
            "an-action-on-a-busy-page-raises-deadline-exceeded-at-its-deadline" => Self::BusyPage,
            _ => {
                return fetch_edge::Case::from_id(id)
                    .map(Self::FetchEdge)
                    .or_else(|| routes::Case::from_id(id).map(Self::Route))
                    .or_else(|| sessions::Case::from_id(id).map(Self::Session))
                    .or_else(|| clean_up::Case::from_id(id).map(Self::CleanUp))
                    .or_else(|| events::Case::from_id(id).map(Self::Events))
                    .or_else(|| consent::Case::from_id(id).map(Self::Consent))
                    .or_else(|| reset::Case::from_id(id).map(Self::Reset))
                    .or_else(|| run_start::Case::from_id(id).map(Self::RunStart))
                    .or_else(|| two_copies::Case::from_id(id).map(Self::TwoCopies))
                    .or_else(|| service_workers::Case::from_id(id).map(Self::ServiceWorker))
                    .or_else(|| page_request::Case::from_id(id).map(Self::PageRequest))
                    .or_else(|| fetch_tiers::Case::from_id(id).map(Self::FetchTier))
                    .or_else(|| dialogs::Case::from_id(id).map(Self::Dialog))
                    .or_else(|| popups::Case::from_id(id).map(Self::Popup))
                    .or_else(|| page_functions::Case::from_id(id).map(Self::PageFunction))
                    .or_else(|| read::Case::from_id(id).map(Self::Read))
                    .or_else(|| waits::Case::from_id(id).map(Self::Wait))
                    .or_else(|| robots_txt::Case::from_id(id).map(Self::RobotsTxt))
                    .or_else(|| formats::references::Case::from_id(id).map(Self::References))
                    .or_else(|| formats::structured::Case::from_id(id).map(Self::Structured))
                    .or_else(|| formats::extractor::Case::from_id(id).map(Self::Extractor));
            }
        })
    }

    /// Runs the case on the open Connection to `target`, whose `hello` the
    /// engine answered with `hello`, against `site`.
    #[expect(
        clippy::too_many_lines,
        reason = "one arm per case, or per kind of case, of the suites: a dispatch table read as one"
    )]
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
                speaks_the_protocol_major(hello, checks);
                Ok(())
            }
            Self::EngineInfo => engine_info(connection, hello, checks),
            Self::UnknownMethod => unknown_method(connection, checks),
            Self::InvalidParams => invalid_params(connection, checks),
            Self::CancelInFlight => cancel_in_flight(connection, checks),
            Self::CancelAnswered => cancel_answered(connection, checks),
            Self::FabricatedHello => fabricated_hello(connection, target, hello, checks),
            Self::AtLeastOnce => delivery::at_least_once(connection, site, checks),
            Self::ExactlyOnce => delivery::exactly_once(connection, site, checks),
            Self::LateCommit => delivery::late_commit(connection, site, checks),
            Self::PagesShareTheDefaultSession => default_session(connection, site, checks),
            Self::GotoAnswersTheResponse => goto(connection, site, checks),
            Self::ErrorStatusesAreResponses => navigation::error_statuses(connection, site, checks),
            Self::HistoryAnswersResponsesAcrossSites => {
                navigation::history_across_sites(connection, site, checks)
            }
            Self::BackLoadsTheDocumentAgain => {
                navigation::back_loads_again(connection, site, checks)
            }
            Self::WaitsArmedBeforeBackPass => {
                navigation::waits_armed_before_back(connection, site, checks)
            }
            Self::UnresolvedNameFailsTheNavigation => {
                navigation::unresolved_name(connection, site, checks)
            }
            Self::SupersededNavigationIsInterrupted => {
                navigation::superseded(connection, site, checks)
            }
            Self::NavigationPastItsDeadline => {
                navigation::past_its_deadline(connection, site, checks)
            }
            Self::FetchEdge(case) => case.run(connection, site, checks),
            Self::PageRequest(case) => case.run(connection, target, site, checks),
            Self::Dialog(case) => case.run(connection, site, checks),
            Self::Popup(case) => case.run(connection, site, checks),
            Self::PageFunction(case) => case.run(connection, target, site, checks),
            Self::Read(case) => case.run(connection, site, checks),
            Self::Wait(case) => case.run(connection, site, checks),
            Self::RobotsTxt(case) => case.run(connection, target, site, checks),
            Self::MarkdownMatchesTheOracle => {
                formats::markdown_matches_the_oracle(connection, site, None, checks)
            }

            Self::FormatNotNamedIsLeftOut => {
                formats::a_format_not_named_is_left_out(connection, site, checks)
            }
            Self::NoMainContentIsNone => formats::no_main_content_is_none(connection, site, checks),
            Self::TextRenderedAfterLoadIsRead => {
                formats::text_rendered_after_load_is_read(connection, site, checks)
            }
            Self::ClosedShadowRootTextIsRead => {
                formats::closed_shadow_root_text_is_read(connection, site, checks)
            }
            Self::RulesTouchOnlyTheMainContent => {
                formats::the_rules_touch_only_the_main_content(connection, site, checks)
            }
            Self::EachFormatsCallIsInTheRunTrace => {
                formats::each_call_is_in_the_run_trace(connection, target, site, checks)
            }
            Self::EveryFormMatchesTheOracle => {
                formats::content_modes::every_form_matches_the_oracle(
                    connection, site, None, checks,
                )
            }
            Self::Extractor(case) => case.run(connection, site, checks),
            Self::RawHtmlIsContentBeforeTheRules => {
                formats::content_modes::raw_html_is_content_before_the_rules(
                    connection, site, checks,
                )
            }
            Self::FormatsDefaultsAndUnknownOptions => {
                formats::content_modes::defaults_and_unknown_options(connection, site, checks)
            }
            Self::References(case) => case.run(connection, site, checks),
            Self::Structured(case) => case.run(connection, site, checks),
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
            Self::BusyPage => guard::busy_page(connection, site, checks),
            Self::Route(case) => case.run(connection, site, checks),
            Self::Session(case) => case.run(connection, target, site, checks),
            Self::CleanUp(case) => case.run(connection, target, site, checks),
            Self::Events(case) => case.run(connection, site, checks),
            Self::Reset(case) => case.run(connection, target, site, checks),
            Self::RunStart(case) => case.run(connection, target, site, checks),
            Self::TwoCopies(case) => case.run(connection, target, site, checks),
            Self::Consent(case) => case.run(
                connection,
                target.environment.get("GOODFETCH_HOME").map(String::as_str),
                site,
                checks,
            ),
            Self::ServiceWorker(case) => case.run(connection, target, site, checks),
            Self::FetchTier(case) => case.run(connection, site, checks),
        }
    }
}

fn speaks_the_protocol_major(hello: &HelloResult, checks: &mut Checks) {
    checks.check(
        format!(
            "the engine speaks the protocol's major {}",
            PROTOCOL_VERSION.major
        ),
        hello.protocol_version.major == PROTOCOL_VERSION.major,
    );
}

fn unknown_method(connection: &mut Connection, checks: &mut Checks) -> Result<(), Gone> {
    let id = connection.request(UNKNOWN_METHOD, &json!({}))?;
    let answer = connection.answer(id)?.message;
    refused_as(&answer, ProtocolFailure::MethodNotFound, checks);
    Ok(())
}

fn invalid_params(connection: &mut Connection, checks: &mut Checks) -> Result<(), Gone> {
    let id = connection.request(ENGINE_INFO.name, &json!({ "unknown": true }))?;
    let answer = connection.answer(id)?.message;
    refused_as(&answer, ProtocolFailure::InvalidParams, checks);
    Ok(())
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
        needs: Some(hello.product),
        ..other.hello_params(Surface::Rust)
    };
    let refused = (|| {
        let (id, first) = fabricated::with_a_call_behind(&mut other, &params)?;
        checks.check(
            "the first message answers the `hello`",
            first.get("method").is_none() && first["id"] == json!(id),
        );
        checks.check(
            format!("it refuses it with `{}`", PROTOCOL_VERSION_UNSUPPORTED.name),
            code_of(&first) == PROTOCOL_VERSION_UNSUPPORTED.name,
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
    let settings = site.mapped(settings, connection.acts_for_model());
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
    let answer = connection.call(RUN_NEW_PAGE.name, &json!({ "run_id": run_id }))?;
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
        let answer = connection.call(
            PAGE_GOTO.name,
            &json!({ "page_id": page.page_id, "url": url }),
        )?;
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
