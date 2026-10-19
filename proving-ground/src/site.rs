//! What the Proving ground answers to one request: a page family's page on
//! that family's host names, and the Effect its form carries out, and on any other host name its own task list
//! (`GET /tasks`), oracle (`POST /oracle`), certificate authority
//! (`GET /certificate-authority`) and observations (`GET /observations`).

use std::collections::BTreeMap;
use std::sync::Arc;

use hyper::body::Bytes;
use hyper::{Method, StatusCode};
use serde::Serialize;

use crate::catalog;
use crate::encodings;
use crate::languages;
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
        let mut hosts = BTreeMap::from([
            (catalog::HOST.to_owned(), None),
            (encodings::HOST.to_owned(), None),
            (languages::HOST.to_owned(), None),
        ]);
        hosts.extend(
            Self::tasks_of(seed, 0)
                .into_iter()
                .map(|task| (task.host, Some(task.entry.id))),
        );
        hosts
    }

    /// Every family's tasks, in the task list's order.
    fn tasks(&self) -> Vec<Task> {
        Self::tasks_of(self.seed, self.port)
    }

    /// Every family's tasks for `seed` on `port`, in the task list's order.
    fn tasks_of(seed: u64, port: u16) -> Vec<Task> {
        let mut tasks = catalog::tasks(seed, port);
        tasks.extend(encodings::tasks(seed, port));
        tasks.extend(languages::tasks(seed, port));
        tasks
    }

    /// The reply of an Extraction family (#165) to a request on `name`, one
    /// of its host names; `None` when `name` is another family's.
    fn extraction(&self, name: &str, method: &Method, path: &str, query: &str) -> Option<Reply> {
        let of = |family: &str| name == family || name.ends_with(&format!(".{family}"));
        let html = |page: String| ("text/html; charset=utf-8", page.into_bytes());
        let page = if of(encodings::HOST) {
            encodings::page(self.seed, path)
        } else if of(languages::HOST) {
            languages::page(self.seed, name, path, query).map(html)
        } else {
            return None;
        };
        Some(match (method, page) {
            (&Method::GET, Some((content_type, body))) => Reply {
                status: StatusCode::OK,
                content_type,
                body: body.into(),
            },
            _ => Reply::error(StatusCode::NOT_FOUND, "not_found"),
        })
    }

    /// Answers one request, given its `Host` header, path, query (empty
    /// when it has none) and body.
    pub(crate) fn respond(
        &self,
        method: &Method,
        host: &str,
        path: &str,
        query: &str,
        body: &Bytes,
    ) -> Reply {
        let name = observation::host_name(host);
        if self.hosts.contains_key(&name) {
            if let Some(reply) = self.extraction(&name, method, path, query) {
                return reply;
            }
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
