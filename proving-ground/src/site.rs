//! What the Proving ground answers to one request: a page family's page on
//! that family's host names, and the Effect its form carries out, and on any other host name its own task list
//! (`GET /tasks`), oracle (`POST /oracle`), certificate authority
//! (`GET /certificate-authority`), observations (`GET /observations`) and
//! time step (`PUT /time-step`), which a task's harness sets and drifting
//! pages follow.

use std::collections::BTreeMap;
use std::sync::Arc;

use hyper::body::Bytes;
use hyper::http::uri::PathAndQuery;
use hyper::{Method, StatusCode, Uri};
use serde::{Deserialize, Serialize};

use crate::agent;
use crate::catalog;
use crate::model::{self, Candidate, FakeModel};
use crate::observation::{self, Observations};
use crate::oracle::{self, Submission};
use crate::replay::{self, Refusals};
use crate::sync::Mutex;
use crate::task::{Task, TaskList};
use crate::version::VERSION;

/// The site of one `serve`: its seed and the port it bound.
pub(crate) struct Site {
    seed: u64,
    port: u16,
    /// The family's host names: its own and each of its tasks'.
    hosts: BTreeMap<String, Option<String>>,
    /// The seed's certificate authority, as PEM.
    certificate_authority: String,
    observations: Arc<Observations>,
    time_step: Mutex<u64>,
    model: FakeModel,
    refusals: Refusals,
}

/// The Proving ground's time step, as `PUT /time-step` sets it.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct TimeStep {
    step: u64,
}

/// A reply's status, media type and body.
pub(crate) struct Reply {
    pub(crate) status: StatusCode,
    pub(crate) content_type: &'static str,
    pub(crate) body: Bytes,
}

impl Reply {
    fn json(status: StatusCode, value: &impl Serialize) -> Self {
        Self {
            status,
            content_type: "application/json",
            body: serde_json::to_vec(value)
                .expect("invariant: the Proving ground's own values serialize")
                .into(),
        }
    }

    fn html(page: String) -> Self {
        Self {
            status: StatusCode::OK,
            content_type: "text/html; charset=utf-8",
            body: page.into(),
        }
    }

    /// A JSON error naming its code.
    pub(crate) fn error(status: StatusCode, code: &str) -> Self {
        Self::json(status, &serde_json::json!({ "error": code }))
    }
}

impl Site {
    pub(crate) fn new(
        seed: u64,
        port: u16,
        certificate_authority: String,
        observations: Arc<Observations>,
    ) -> Self {
        Self {
            seed,
            port,
            hosts: Self::hosts(seed),
            certificate_authority,
            observations,
            time_step: Mutex::new(0),
            model: FakeModel::new(seed),
            refusals: Refusals::default(),
        }
    }

    /// Every host name a page family is served on, each with the task its
    /// requests belong to, if any: the family's own, and each task's. Each
    /// has its own leaf certificate, and only these are observed.
    pub(crate) fn hosts(seed: u64) -> BTreeMap<String, Option<String>> {
        let mut hosts = BTreeMap::from([
            (catalog::HOST.to_owned(), None),
            (agent::HOST.to_owned(), None),
            (model::HOST.to_owned(), None),
        ]);
        hosts.extend(
            Self::all_tasks(seed, 0)
                .into_iter()
                .map(|task| (task.host, Some(task.entry.id))),
        );
        hosts
    }

    /// Every family's tasks, in the task list's order.
    fn tasks(&self) -> Vec<Task> {
        Self::all_tasks(self.seed, self.port)
    }

    fn all_tasks(seed: u64, port: u16) -> Vec<Task> {
        let mut tasks = catalog::tasks(seed, port);
        tasks.extend(agent::tasks(seed, port));
        tasks.extend(replay::tasks());
        tasks
    }

    /// Answers one request, given its `Host` header, path and body.
    pub(crate) fn respond(&self, method: &Method, host: &str, path: &str, body: &Bytes) -> Reply {
        let name = observation::host_name(host);
        if name == model::HOST {
            // The Fake model answers its own paths (`Self::model`).
            return Reply::error(StatusCode::NOT_FOUND, "not_found");
        }
        if replay::serves(&name) {
            return self.replay(method, &name, path);
        }
        if self.hosts.contains_key(&name) && agent::serves(&name) {
            return self.agent(method, host, &name, path, body);
        }
        if self.hosts.contains_key(&name) {
            return match (method, path) {
                (&Method::POST, "/orders") => match catalog::order(body) {
                    Some((effect, page)) => {
                        self.observations.effect(host, effect);
                        Reply::html(page)
                    }
                    None => Reply::error(StatusCode::BAD_REQUEST, "unknown_product"),
                },
                (&Method::GET, _) => catalog::page(self.seed, path).map_or_else(
                    || Reply::error(StatusCode::NOT_FOUND, "not_found"),
                    Reply::html,
                ),
                _ => Reply::error(StatusCode::NOT_FOUND, "not_found"),
            };
        }
        match (method, path) {
            (&Method::GET, "/tasks") => Reply::json(
                StatusCode::OK,
                &TaskList::new(VERSION, self.seed, &self.tasks()),
            ),
            (&Method::POST, "/oracle") => self.judge(body),
            (&Method::PUT, "/time-step") => self.set_time_step(body),
            (&Method::GET, "/replay") => Reply {
                status: StatusCode::OK,
                content_type: "application/json",
                body: self.refusals.document().into(),
            },
            (&Method::GET, "/model-calls") => Reply {
                status: StatusCode::OK,
                content_type: "application/json",
                body: self.model.document().into(),
            },
            (&Method::GET, "/observations") => Reply {
                status: StatusCode::OK,
                content_type: "application/json",
                body: self.observations.document().into(),
            },
            (&Method::GET, "/certificate-authority") => Reply {
                status: StatusCode::OK,
                content_type: "application/x-pem-file",
                body: self.certificate_authority.clone().into(),
            },
            _ => Reply::error(StatusCode::NOT_FOUND, "not_found"),
        }
    }

    /// Answers a request to the `agent` family: its pages at the current
    /// time step, and the Effects their script posts, recorded with it.
    fn agent(&self, method: &Method, host: &str, name: &str, path: &str, body: &Bytes) -> Reply {
        let step = *self.time_step.lock();
        match (method, path) {
            (&Method::POST, agent::EFFECTS_PATH) => match agent::effect(body, step) {
                Some(effect) => {
                    self.observations.effect(host, effect);
                    Reply::json(StatusCode::OK, &serde_json::json!({ "time_step": step }))
                }
                None => Reply::error(StatusCode::BAD_REQUEST, "malformed_effect"),
            },
            (&Method::GET, _) => agent::page(self.seed, name, step, path).map_or_else(
                || Reply::error(StatusCode::NOT_FOUND, "not_found"),
                Reply::html,
            ),
            _ => Reply::error(StatusCode::NOT_FOUND, "not_found"),
        }
    }

    /// The Fake model's answer to a `POST` on its host name to `target`
    /// (path and query); `None` for any other request, which [`Self::respond`]
    /// answers.
    pub(crate) fn model(
        &self,
        method: &Method,
        host: &str,
        target: &str,
        body: &Bytes,
    ) -> Option<model::Answer> {
        if method != Method::POST || observation::host_name(host) != model::HOST {
            return None;
        }
        let step = *self.time_step.lock();
        let candidates: Vec<Candidate> = self
            .tasks()
            .into_iter()
            .map(|task| {
                let id = task.entry.id;
                let answer = agent::answer(self.seed, &id, step).unwrap_or_else(|| {
                    let effects = task.expected.effects.iter().map(serde_json::to_value);
                    task.expected
                        .records
                        .iter()
                        .map(serde_json::to_value)
                        .chain(effects)
                        .filter_map(|value| match value {
                            Ok(serde_json::Value::Object(fields)) => Some(fields),
                            _ => None,
                        })
                        .collect()
                });
                Candidate {
                    fault: agent::fault(self.seed, &id),
                    id,
                    description: task.entry.description,
                    host: task.host,
                    answer,
                }
            })
            .collect();
        self.model.answer(target, body, &candidates)
    }

    /// Answers a request to an archived host on its own name, as through a
    /// tunnel: its archived page, else a refusal it records.
    fn replay(&self, method: &Method, host: &str, path: &str) -> Reply {
        self.archived(method, host, path, || format!("{host}{path}"))
    }

    /// Answers a request in absolute form, as a browser sends one to its
    /// proxy: an archived page, else a refusal it records.
    pub(crate) fn proxy(&self, method: &Method, uri: &Uri) -> Reply {
        let host = uri.host().unwrap_or_default();
        let target = uri.path_and_query().map_or("/", PathAndQuery::as_str);
        self.archived(method, host, target, || uri.to_string())
    }

    /// The archived page a `GET` to `host` and `target` asks for; else a
    /// refusal of the request `named`, recorded.
    fn archived(
        &self,
        method: &Method,
        host: &str,
        target: &str,
        named: impl FnOnce() -> String,
    ) -> Reply {
        if method == Method::GET
            && let Some(page) = replay::page(host, target)
        {
            return Reply::html(page.to_owned());
        }
        self.refusals.record(method.as_str(), named());
        Reply::error(StatusCode::NOT_FOUND, "not_in_archive")
    }

    /// Whether a `CONNECT` to `authority` may open its tunnel: only to an
    /// archived host's port 443. A refused one is recorded.
    pub(crate) fn tunnel(&self, authority: &str) -> bool {
        let open = replay::tunnels(authority);
        if !open {
            self.refusals.record("CONNECT", authority.to_owned());
        }
        open
    }

    /// Sets the time step drifting pages follow.
    fn set_time_step(&self, body: &Bytes) -> Reply {
        let Ok(time_step) = serde_json::from_slice::<TimeStep>(body) else {
            return Reply::error(StatusCode::BAD_REQUEST, "malformed_time_step");
        };
        *self.time_step.lock() = time_step.step;
        Reply::json(StatusCode::OK, &time_step)
    }

    /// Judges a submission against its task's expected result.
    fn judge(&self, body: &Bytes) -> Reply {
        let Ok(submission) = serde_json::from_slice::<Submission>(body) else {
            return Reply::error(StatusCode::BAD_REQUEST, "malformed_submission");
        };
        let tasks = self.tasks();
        match tasks.iter().find(|task| task.entry.id == submission.task) {
            Some(task) => {
                let effects = self.observations.effects_of(&task.entry.id);
                Reply::json(
                    StatusCode::OK,
                    &oracle::judge(&task.expected, submission, &effects),
                )
            }
            None => Reply::error(StatusCode::NOT_FOUND, "unknown_task"),
        }
    }
}
