//! What the Proving ground answers to one request: a page family's page on
//! that family's host names, and the Effect its form carries out, and on any other host name its own task list
//! (`GET /tasks`), oracle (`POST /oracle`), certificate authority
//! (`GET /certificate-authority`), observations (`GET /observations`) and
//! the revocation of the portal's sessions (`POST /revoke-sessions`), which
//! a harness asks for while a tool is stopped.

use std::collections::BTreeMap;
use std::sync::Arc;

use hyper::body::Bytes;
use hyper::{Method, StatusCode};

use crate::catalog;
use crate::exchange::{Reply, Request};
use crate::observation::{self, Observations};
use crate::oracle::{self, Submission};
use crate::portal::{self, Portal};
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
    portal: Portal,
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
            portal: Portal::new(seed),
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
        hosts.extend(
            portal::hosts()
                .into_iter()
                .map(|(host, task)| (host, Some(task))),
        );
        hosts
    }

    /// Every family's tasks, in the task list's order.
    fn tasks(&self) -> Vec<Task> {
        let mut tasks = catalog::tasks(self.seed, self.port);
        tasks.extend(portal::tasks(self.seed, self.port));
        tasks
    }

    /// Answers one request, given its `Host` header.
    pub(crate) fn respond(&self, host: &str, request: &Request<'_>) -> Reply {
        if let Some((reply, effect)) = self.portal.respond(&observation::host_name(host), request) {
            if let Some(effect) = effect {
                self.observations.effect(host, effect);
            }
            return reply;
        }
        if self.hosts.contains_key(&observation::host_name(host)) {
            return match (request.method, request.path) {
                (&Method::POST, "/orders") => match catalog::order(request.body) {
                    Some((effect, page)) => {
                        self.observations.effect(host, effect);
                        Reply::html(page)
                    }
                    None => Reply::error(StatusCode::BAD_REQUEST, "unknown_product"),
                },
                (&Method::GET, _) => catalog::page(self.seed, request.path)
                    .map_or_else(Reply::not_found, Reply::html),
                _ => Reply::not_found(),
            };
        }
        match (request.method, request.path) {
            (&Method::GET, "/tasks") => Reply::json(
                StatusCode::OK,
                &TaskList::new(VERSION, self.seed, &self.tasks()),
            ),
            (&Method::POST, "/oracle") => self.judge(request.body),
            (&Method::POST, "/revoke-sessions") => {
                self.portal.revoke();
                Reply::no_content()
            }
            (&Method::GET, "/observations") => Reply {
                status: StatusCode::OK,
                content_type: "application/json",
                headers: Vec::new(),
                body: self.observations.document().into(),
            },
            (&Method::GET, "/certificate-authority") => Reply {
                status: StatusCode::OK,
                content_type: "application/x-pem-file",
                headers: Vec::new(),
                body: self.certificate_authority.clone().into(),
            },
            _ => Reply::not_found(),
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
