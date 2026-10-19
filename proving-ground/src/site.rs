//! What the Proving ground answers to one request: a page family's page on
//! that family's host names, and the Effect its form carries out, and on any other host name its own task list
//! (`GET /tasks`), oracle (`POST /oracle`), certificate authority
//! (`GET /certificate-authority`), observations (`GET /observations`) and
//! time step (`PUT /time-step`).

use std::collections::BTreeMap;
use std::sync::Arc;

use hyper::body::Bytes;
use hyper::{Method, StatusCode};
use serde::{Deserialize, Serialize};

use crate::catalog;
use crate::observation::{self, Observations};
use crate::oracle::{self, Submission};
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
        }
    }

    /// Every host name a page family is served on, each with the task its
    /// requests belong to, if any: the family's own, and each task's. Each
    /// has its own leaf certificate, and only these are observed.
    pub(crate) fn hosts(seed: u64) -> BTreeMap<String, Option<String>> {
        let mut hosts = BTreeMap::from([(catalog::HOST.to_owned(), None)]);
        hosts.extend(
            catalog::tasks(seed, 0)
                .into_iter()
                .map(|task| (task.host, Some(task.entry.id))),
        );
        hosts
    }

    /// Every family's tasks, in the task list's order.
    fn tasks(&self) -> Vec<Task> {
        catalog::tasks(self.seed, self.port)
    }

    /// Answers one request, given its `Host` header, path and body.
    pub(crate) fn respond(&self, method: &Method, host: &str, path: &str, body: &Bytes) -> Reply {
        if self.hosts.contains_key(&observation::host_name(host)) {
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

    /// Sets the time step, `{"step": n}`, which the connection record stamps
    /// each later entry with.
    fn set_time_step(&self, body: &Bytes) -> Reply {
        #[derive(Deserialize, Serialize)]
        #[serde(deny_unknown_fields)]
        struct TimeStep {
            step: u64,
        }
        let Ok(time_step) = serde_json::from_slice::<TimeStep>(body) else {
            return Reply::error(StatusCode::BAD_REQUEST, "malformed_time_step");
        };
        self.observations.set_time_step(time_step.step);
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
