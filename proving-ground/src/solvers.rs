//! The solver-service stand-ins (ADRs 0035 and 0076): the paid services a
//! Solver calls, each on its own host name, for the Proving ground's own
//! CAPTCHA-marked pages. A harness opens an account on one and tells it what
//! its calls meet: a solution the page accepts, an error, a stall or a body
//! that is not JSON. Every call is recorded with its error and with the
//! price of the solution it spends.

mod sensor_service;
mod token_service;

use std::collections::{BTreeMap, VecDeque};
use std::time::Duration;

use hyper::body::Bytes;
use hyper::{Method, StatusCode, Uri};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::exchange::{Reply, Request};
use crate::generator;
use crate::observation;
use crate::protection;
use crate::sync::Mutex;
use sensor_service::{Named, Sensor};
use token_service::{Refusal, SERVICES, Service};

/// The generator places of the stand-ins' values: their own first, then the
/// value's, then the account's, task's or sensor's number and the half
/// drawn.
const PLACE: u64 = 43;
const KEY_PLACE: u64 = 1;
const TASK_PLACE: u64 = 2;

/// How long a stalled sensor call is held, far past any client's timeout.
const STALL: Duration = Duration::from_hours(1);

/// How many accounts are opened; a later one is refused.
const MOST_ACCOUNTS: usize = 10_000;

/// How many tasks are kept; a later one drops the oldest.
const MOST_TASKS: usize = 10_000;

/// How many calls are recorded; later ones are counted in
/// `unrecorded_calls`.
const MOST_CALLS: usize = 10_000;

/// How many characters of a call's task type are recorded.
const LONGEST_KIND: usize = 64;

/// What an account's calls meet, as `POST /solver-accounts` tells it.
#[derive(Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Told {
    /// A task is solved at its second poll, a sensor call at once.
    Solve,
    /// A task stays unsolved, however often it is polled; a sensor call is
    /// not answered.
    Stall,
    /// Every task type, or product, is one the service does not support.
    Unsupported,
    /// A task's second poll, or a sensor call, answers JSON cut short.
    Malformed,
    /// Every create-task or sensor call answers this error code.
    CreateError(String),
    /// A task's second poll answers this error code.
    ResultError(String),
}

/// The service an account is on.
#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(untagged)]
enum Provider {
    Token(Service),
    Sensor(sensor_service::Service),
}

impl Provider {
    const fn host(self) -> &'static str {
        match self {
            Self::Token(service) => service.host(),
            Self::Sensor(_) => sensor_service::HOST,
        }
    }

    /// The key of the account whose drawn halves are `high` and `low`.
    fn key(self, high: u64, low: u64) -> String {
        match self {
            Self::Token(service) => service.key(high, low),
            Self::Sensor(_) => uuid(high, low),
        }
    }
}

/// An error a service answers, with its status.
struct Failed {
    status: StatusCode,
    body: Value,
}

/// The body of `POST /solver-accounts`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Opened {
    service: Provider,
    told: Told,
}

/// An account on a service.
struct Account {
    name: String,
    told: Told,
}

/// A task an account created.
struct Created {
    account: String,
    id: Value,
    /// The token its solution holds.
    token: String,
    /// How many polls it was given.
    polls: u64,
    /// Its create-task call in the record, if recorded.
    call: Option<usize>,
}

/// Which of a service's calls a request is.
#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Operation {
    CreateTask,
    GetTaskResult,
    /// The one call a sensor service answers a product with.
    Generate,
}

/// A call a service received, as `GET /solver-calls` lists it.
#[derive(Serialize)]
struct Call {
    sequence: u64,
    /// The account its key names, if any.
    account: Option<String>,
    service: Provider,
    #[serde(rename = "call")]
    operation: Operation,
    /// The task type a create-task call names, or a sensor service's
    /// product.
    kind: Option<String>,
    /// The task it created or polled, if the account has it.
    task: Option<Value>,
    /// The status it was answered with; `None` when it stalls.
    status: Option<u16>,
    /// The error it was answered with.
    error: Option<Value>,
    /// What its solution costs, in dollars, once solved.
    price: Option<&'static str>,
}

#[derive(Default)]
struct State {
    /// The accounts by service host and key.
    accounts: BTreeMap<(&'static str, String), Account>,
    opened: u64,
    tasks: VecDeque<Created>,
    created: u64,
    /// How many sensors were made.
    sensors: u64,
    calls: Vec<Call>,
    unrecorded_calls: u64,
}

/// The body of a create-task call.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateTask {
    client_key: String,
    task: TaskData,
}

/// What a create-task call says of its task.
#[derive(Deserialize)]
struct TaskData {
    #[serde(rename = "type")]
    kind: String,
    #[serde(rename = "websiteURL", default)]
    website_url: String,
    #[serde(rename = "websiteKey", default)]
    website_key: String,
}

/// The body of a task-result call.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GetTaskResult {
    client_key: String,
    task_id: Value,
}

/// What a call is answered with, and what the record keeps of it.
struct Answered {
    operation: Operation,
    account: Option<String>,
    kind: Option<String>,
    task: Option<Value>,
    outcome: Outcome,
    /// What the solution it was answered with costs, in dollars.
    price: Option<&'static str>,
}

/// What a call meets.
enum Outcome {
    Answered(Bytes),
    Failed(Failed),
    /// No answer before [`STALL`] is over.
    Stalled,
}

/// The stand-ins' accounts, tasks and record of calls.
#[derive(Default)]
pub(crate) struct Solvers(Mutex<State>);

/// Every host name a stand-in answers on.
pub(crate) fn hosts() -> impl Iterator<Item = &'static str> {
    SERVICES
        .into_iter()
        .map(Service::host)
        .chain([sensor_service::HOST])
}

impl Solvers {
    /// Opens the account `body` describes, on its service, and answers its
    /// name and key.
    pub(crate) fn open(&self, seed: u64, body: &Bytes) -> Reply {
        let Ok(Opened { service, told }) = serde_json::from_slice::<Opened>(body) else {
            return Reply::error(StatusCode::BAD_REQUEST, "malformed_solver_account");
        };
        if let (Provider::Sensor(_), Told::ResultError(_)) = (service, &told) {
            return Reply::error(StatusCode::BAD_REQUEST, "malformed_solver_account");
        }
        let mut state = self.0.lock();
        if state.accounts.len() >= MOST_ACCOUNTS {
            return Reply::error(StatusCode::TOO_MANY_REQUESTS, "too_many_solver_accounts");
        }
        state.opened += 1;
        let number = state.opened;
        let half = |half| generator::draw(seed, &[PLACE, KEY_PLACE, number, half]);
        let key = service.key(half(0), half(1));
        let name = format!("account-{number}");
        let reply = Reply::json(StatusCode::OK, &json!({ "account": name, "key": key }));
        state
            .accounts
            .insert((service.host(), key), Account { name, told });
        reply
    }

    /// Every call the stand-ins received, in order, as the JSON of
    /// `GET /solver-calls`.
    pub(crate) fn document(&self) -> Reply {
        let state = self.0.lock();
        Reply::json(
            StatusCode::OK,
            &json!({ "calls": state.calls, "unrecorded_calls": state.unrecorded_calls }),
        )
    }

    /// Answers a call on a stand-in's host name; `None` when `host` is none
    /// of theirs.
    pub(crate) fn respond(&self, seed: u64, host: &str, request: &Request<'_>) -> Option<Reply> {
        if host == sensor_service::HOST {
            if request.method != Method::POST {
                return Some(Reply::not_found());
            }
            let mut state = self.0.lock();
            let answered = state.generate(seed, request);
            let provider = Provider::Sensor(sensor_service::Service::HyperSolutions);
            return Some(state.record(provider, answered));
        }
        let service = SERVICES
            .into_iter()
            .find(|service| service.host() == host)?;
        let mut state = self.0.lock();
        let answered = match (request.method, request.path) {
            (&Method::POST, "/createTask") => state.create(seed, service, request.body),
            (&Method::POST, "/getTaskResult") => state.poll(service, request.body),
            _ => return Some(Reply::not_found()),
        };
        Some(state.record(Provider::Token(service), answered))
    }

    /// How long a call on a stand-in's host name is held, if it is: a
    /// sensor call whose account was told to stall.
    pub(crate) fn hold(&self, host: &str, request: &Request<'_>) -> Option<Duration> {
        if host != sensor_service::HOST || request.method != Method::POST {
            return None;
        }
        let key = request.headers.get(sensor_service::KEY_HEADER)?;
        let key = key.to_str().ok()?.to_owned();
        let state = self.0.lock();
        let account = state.accounts.get(&(sensor_service::HOST, key))?;
        (matches!(account.told, Told::Stall) && request.path == sensor_service::SENSOR)
            .then_some(STALL)
    }
}

impl State {
    /// Creates the task `body` asks `service` for.
    fn create(&mut self, seed: u64, service: Service, body: &Bytes) -> Answered {
        let mut answered = Answered {
            operation: Operation::CreateTask,
            account: None,
            kind: None,
            task: None,
            outcome: Outcome::Failed(service.refuse(Refusal::Malformed)),
            price: None,
        };
        let Ok(CreateTask { client_key, task }) = serde_json::from_slice(body) else {
            return answered;
        };
        answered.kind = Some(kind(&task.kind));
        let Some(account) = self.accounts.get(&(service.host(), client_key)) else {
            answered.outcome = Outcome::Failed(service.refuse(Refusal::Key));
            return answered;
        };
        answered.account = Some(account.name.clone());
        let supported = service.solves(&task.kind) && !matches!(account.told, Told::Unsupported);
        let page = task.website_url.parse::<Uri>().ok();
        let page = page
            .as_ref()
            .and_then(|url| Some((url.host()?, url.path())));
        let refused = match (&account.told, page) {
            (Told::CreateError(code), _) => Some(service.fail(code)),
            _ if !supported => Some(service.refuse(Refusal::Type)),
            (_, None) => Some(service.refuse(Refusal::PageUrl)),
            _ if task.website_key.is_empty() => Some(service.refuse(Refusal::SiteKey)),
            _ => None,
        };
        if let Some(refused) = refused {
            answered.outcome = Outcome::Failed(refused);
            return answered;
        }
        let Some((host, path)) = page else {
            return answered;
        };
        let name = account.name.clone();
        self.created += 1;
        let number = self.created;
        let half = |half| generator::draw(seed, &[PLACE, TASK_PLACE, number, half]);
        let id = service.task_id(number, half(0), half(1));
        answered.outcome = Outcome::Answered(bytes(&service.created(&id)));
        answered.task = Some(id.clone());
        if self.tasks.len() >= MOST_TASKS {
            self.tasks.pop_front();
        }
        self.tasks.push_back(Created {
            account: name,
            id,
            token: protection::turnstile_token(
                seed,
                &task.website_key,
                &observation::host_name(host),
                path,
                number,
            ),
            polls: 0,
            call: (self.calls.len() < MOST_CALLS).then_some(self.calls.len()),
        });
        answered
    }

    /// Polls the task `body` names at `service`.
    fn poll(&mut self, service: Service, body: &Bytes) -> Answered {
        let mut answered = Answered {
            operation: Operation::GetTaskResult,
            account: None,
            kind: None,
            task: None,
            outcome: Outcome::Failed(service.refuse(Refusal::Malformed)),
            price: None,
        };
        let Ok(GetTaskResult {
            client_key,
            task_id,
        }) = serde_json::from_slice(body)
        else {
            return answered;
        };
        let Some(account) = self.accounts.get(&(service.host(), client_key)) else {
            answered.outcome = Outcome::Failed(service.refuse(Refusal::Key));
            return answered;
        };
        answered.account = Some(account.name.clone());
        let Some(task) = self
            .tasks
            .iter_mut()
            .find(|task| task.account == account.name && task.id == task_id)
        else {
            answered.outcome = Outcome::Failed(service.refuse(Refusal::Task));
            return answered;
        };
        answered.task = Some(task_id);
        task.polls += 1;
        answered.outcome = match &account.told {
            Told::ResultError(code) if task.polls > 1 => Outcome::Failed(service.fail(code)),
            Told::Stall | Told::ResultError(_) => {
                Outcome::Answered(bytes(&service.processing(&task.id)))
            }
            _ if task.polls == 1 => Outcome::Answered(bytes(&service.processing(&task.id))),
            told => {
                if let Some(call) = task.call.and_then(|at| self.calls.get_mut(at)) {
                    call.price = Some(service.price());
                }
                solution(told, &service.ready(&task.id, &task.token))
            }
        };
        answered
    }

    /// Answers `request`, a call to the sensor service.
    fn generate(&mut self, seed: u64, request: &Request<'_>) -> Answered {
        let mut answered = Answered {
            operation: Operation::Generate,
            account: None,
            kind: Some(kind(request.path.trim_start_matches('/'))),
            task: None,
            outcome: Outcome::Stalled,
            price: None,
        };
        let key = request.headers.get(sensor_service::KEY_HEADER);
        let key = key.map(|key| key.to_str().unwrap_or_default());
        let account = match key {
            None => Err("missing_api_key"),
            Some(key) if !sensor_service::is_key(key) => Err("malformed_api_key"),
            Some(key) => self
                .accounts
                .get(&(sensor_service::HOST, key.to_owned()))
                .ok_or("invalid_api_key"),
        };
        let account = match account {
            Ok(account) => account,
            Err(code) => {
                answered.outcome = Outcome::Failed(sensor_service::fail(code));
                return answered;
            }
        };
        answered.account = Some(account.name.clone());
        answered.outcome = match &account.told {
            Told::CreateError(code) => Outcome::Failed(sensor_service::fail(code)),
            told if request.path != sensor_service::SENSOR || matches!(told, Told::Unsupported) => {
                Outcome::Failed(sensor_service::fail("product_not_enabled"))
            }
            Told::Stall => Outcome::Stalled,
            told => match sensor(seed, self.sensors + 1, request.body) {
                Ok(solved) => {
                    self.sensors += 1;
                    answered.price = Some(sensor_service::PRICE);
                    solution(told, &solved)
                }
                Err(refused) => Outcome::Failed(refused),
            },
        };
        answered
    }

    /// Records a call to `service` and gives its reply.
    fn record(&mut self, service: Provider, answered: Answered) -> Reply {
        let (status, body, error) = match answered.outcome {
            Outcome::Answered(body) => (Some(StatusCode::OK), body, None),
            Outcome::Failed(Failed { status, body }) => (Some(status), bytes(&body), Some(body)),
            Outcome::Stalled => (None, Bytes::new(), None),
        };
        if self.calls.len() < MOST_CALLS {
            self.calls.push(Call {
                sequence: self.calls.len() as u64,
                account: answered.account,
                service,
                operation: answered.operation,
                kind: answered.kind,
                task: answered.task,
                status: status.map(|status| status.as_u16()),
                error,
                price: answered.price,
            });
        } else {
            self.unrecorded_calls += 1;
        }
        Reply {
            status: status.unwrap_or(StatusCode::GATEWAY_TIMEOUT),
            content_type: "application/json",
            headers: Vec::new(),
            body,
        }
    }
}

/// The sensor `number` for the sensor call `body`, as the service answers
/// it, or why it refuses the call.
fn sensor(seed: u64, number: u64, body: &Bytes) -> Result<Value, Failed> {
    let Ok(mut call) = serde_json::from_slice::<Sensor>(body) else {
        return Err(sensor_service::invalid("The body misses a field."));
    };
    if !matches!(call.version.as_str(), "2" | "3") {
        return Err(sensor_service::invalid("version is neither 2 nor 3"));
    }
    let script_hash = match call.named()? {
        Named::Script(script) => protection::hash(&script),
        Named::Hash(hash) if hash == protection::script_hash(seed) => hash,
        Named::Hash(_) => return Err(sensor_service::script_required()),
        Named::Context(context) => match context.split_once('.') {
            Some((hash, _)) if context == self::context(seed, hash) => hash.to_owned(),
            _ => return Err(sensor_service::invalid("context is no earlier call's")),
        },
    };
    let page = call.page_url.parse::<Uri>().ok();
    let Some((host, path)) = page
        .as_ref()
        .and_then(|url| Some((url.host()?, url.path())))
    else {
        return Err(sensor_service::invalid("pageUrl is not a URL"));
    };
    let sensed = protection::Sensed {
        abck: &call.abck,
        bm_sz: &call.bmsz,
        host: &observation::host_name(host),
        path,
        user_agent: &call.user_agent,
        script_hash: &script_hash,
    };
    let data = protection::sensor_data(seed, &sensed, number);
    Ok(sensor_service::answered(
        &data,
        &self::context(seed, &script_hash),
    ))
}

/// The context a later sensor call names the script of `script_hash` by.
fn context(seed: u64, script_hash: &str) -> String {
    let mac = protection::hash(&format!("{seed}.{script_hash}"));
    format!("{script_hash}.{mac}")
}

/// What an account told `told` is answered for a solved call.
fn solution(told: &Told, solved: &Value) -> Outcome {
    let solved = bytes(solved);
    Outcome::Answered(if matches!(told, Told::Malformed) {
        solved.slice(..solved.len() / 2)
    } else {
        solved
    })
}

/// A task type or product as recorded.
fn kind(named: &str) -> String {
    named.chars().take(LONGEST_KIND).collect()
}

/// The version 4 UUID whose drawn halves are `high` and `low`.
fn uuid(high: u64, low: u64) -> String {
    format!(
        "{:08x}-{:04x}-4{:03x}-{:04x}-{:012x}",
        high >> 32,
        (high >> 16) & 0xFFFF,
        high & 0xFFF,
        0x8000 | (low >> 50),
        low & 0xFFFF_FFFF_FFFF
    )
}

fn bytes(value: &Value) -> Bytes {
    value.to_string().into()
}
