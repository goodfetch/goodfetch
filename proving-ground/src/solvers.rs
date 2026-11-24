//! The solver-service stand-ins (ADRs 0035 and 0076): the paid services a
//! Solver calls, each on its own host name, for the Proving ground's own
//! CAPTCHA-marked pages. A harness opens an account on one and tells it what
//! its calls meet: a solution the page accepts, an error, a stall or a body
//! that is not JSON. Every call is recorded with its error and with the
//! price of the solution it spends.

mod token_service;

use std::collections::{BTreeMap, VecDeque};

use hyper::body::Bytes;
use hyper::{Method, StatusCode, Uri};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::exchange::{Reply, Request};
use crate::generator;
use crate::observation;
use crate::protection;
use crate::sync::Mutex;
use token_service::{Failed, Refusal, SERVICES, Service};

/// The generator places of the stand-ins' values: their own first, then the
/// value's, then the account's or task's number and the half drawn.
const PLACE: u64 = 43;
const KEY_PLACE: u64 = 1;
const TASK_PLACE: u64 = 2;

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
    /// A task is solved at its second poll.
    Solve,
    /// A task stays unsolved, however often it is polled.
    Stall,
    /// Every task type is one the service does not support.
    Unsupported,
    /// A task's second poll answers JSON cut short.
    Malformed,
    /// Every create-task call answers this error code.
    CreateError(String),
    /// A task's second poll answers this error code.
    ResultError(String),
}

/// The body of `POST /solver-accounts`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Opened {
    service: Service,
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
}

/// A call a service received, as `GET /solver-calls` lists it.
#[derive(Serialize)]
struct Call {
    sequence: u64,
    /// The account its key names, if any.
    account: Option<String>,
    service: Service,
    #[serde(rename = "call")]
    operation: Operation,
    /// The task type a create-task call names.
    kind: Option<String>,
    /// The task it created or polled, if the account has it.
    task: Option<Value>,
    status: u16,
    /// The error it was answered with.
    error: Option<Value>,
    /// What its task's solution costs, in dollars, once solved.
    price: Option<&'static str>,
}

#[derive(Default)]
struct State {
    /// The accounts by service host and key.
    accounts: BTreeMap<(&'static str, String), Account>,
    opened: u64,
    tasks: VecDeque<Created>,
    created: u64,
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
    outcome: Result<Bytes, Failed>,
}

/// The stand-ins' accounts, tasks and record of calls.
#[derive(Default)]
pub(crate) struct Solvers(Mutex<State>);

/// Every host name a stand-in answers on.
pub(crate) fn hosts() -> impl Iterator<Item = &'static str> {
    SERVICES.into_iter().map(Service::host)
}

impl Solvers {
    /// Opens the account `body` describes, on its service, and answers its
    /// name and key.
    pub(crate) fn open(&self, seed: u64, body: &Bytes) -> Reply {
        let Ok(Opened { service, told }) = serde_json::from_slice::<Opened>(body) else {
            return Reply::error(StatusCode::BAD_REQUEST, "malformed_solver_account");
        };
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
        let service = SERVICES
            .into_iter()
            .find(|service| service.host() == host)?;
        let mut state = self.0.lock();
        let answered = match (request.method, request.path) {
            (&Method::POST, "/createTask") => state.create(seed, service, request.body),
            (&Method::POST, "/getTaskResult") => state.poll(service, request.body),
            _ => return Some(Reply::not_found()),
        };
        Some(state.record(service, answered))
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
            outcome: Err(service.refuse(Refusal::Malformed)),
        };
        let Ok(CreateTask { client_key, task }) = serde_json::from_slice(body) else {
            return answered;
        };
        answered.kind = Some(task.kind.chars().take(LONGEST_KIND).collect());
        let Some(account) = self.accounts.get(&(service.host(), client_key)) else {
            answered.outcome = Err(service.refuse(Refusal::Key));
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
            answered.outcome = Err(refused);
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
        answered.outcome = Ok(bytes(&service.created(&id)));
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
            outcome: Err(service.refuse(Refusal::Malformed)),
        };
        let Ok(GetTaskResult {
            client_key,
            task_id,
        }) = serde_json::from_slice(body)
        else {
            return answered;
        };
        let Some(account) = self.accounts.get(&(service.host(), client_key)) else {
            answered.outcome = Err(service.refuse(Refusal::Key));
            return answered;
        };
        answered.account = Some(account.name.clone());
        let Some(task) = self
            .tasks
            .iter_mut()
            .find(|task| task.account == account.name && task.id == task_id)
        else {
            answered.outcome = Err(service.refuse(Refusal::Task));
            return answered;
        };
        answered.task = Some(task_id);
        task.polls += 1;
        answered.outcome = match &account.told {
            Told::ResultError(code) if task.polls > 1 => Err(service.fail(code)),
            Told::Stall | Told::ResultError(_) => Ok(bytes(&service.processing(&task.id))),
            _ if task.polls == 1 => Ok(bytes(&service.processing(&task.id))),
            told => {
                if let Some(call) = task.call.and_then(|at| self.calls.get_mut(at)) {
                    call.price = Some(service.price());
                }
                let solved = bytes(&service.ready(&task.id, &task.token));
                Ok(if matches!(told, Told::Malformed) {
                    solved.slice(..solved.len() / 2)
                } else {
                    solved
                })
            }
        };
        answered
    }

    /// Records a call to `service` and gives its reply.
    fn record(&mut self, service: Service, answered: Answered) -> Reply {
        let (status, body, error) = match answered.outcome {
            Ok(body) => (StatusCode::OK, body, None),
            Err(Failed { status, body }) => (status, bytes(&body), Some(body)),
        };
        if self.calls.len() < MOST_CALLS {
            self.calls.push(Call {
                sequence: self.calls.len() as u64,
                account: answered.account,
                service,
                operation: answered.operation,
                kind: answered.kind,
                task: answered.task,
                status: status.as_u16(),
                error,
                price: None,
            });
        } else {
            self.unrecorded_calls += 1;
        }
        Reply {
            status,
            content_type: "application/json",
            headers: Vec::new(),
            body,
        }
    }
}

fn bytes(value: &Value) -> Bytes {
    value.to_string().into()
}
