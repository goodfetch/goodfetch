//! What the Proving ground answers to one request: a page family's page on
//! that family's host names, and the Effect its form carries out, and on any
//! other host name its own task list (`GET /tasks`), oracle (`POST /oracle`),
//! time step (`PUT /time-step`), which a task's harness sets and scheduled
//! content follows, certificate authority (`GET /certificate-authority`),
//! observations (`GET /observations`) and the revocation of the portal's
//! sessions (`POST /revoke-sessions`), which a harness asks for while a tool
//! is stopped.

use std::collections::BTreeMap;
use std::sync::Arc;

use hyper::body::Bytes;
use hyper::{Method, StatusCode};
use serde::{Deserialize, Serialize};

use crate::catalog;
use crate::exchange::{Reply, Request};
use crate::handmade;
use crate::login::{self, Logins};
use crate::observation::{self, Observations};
use crate::oracle::{self, Submission};
use crate::portal::{self, Portal};
use crate::rendered;
use crate::schedule;
use crate::sync::Mutex;
use crate::task::{Task, TaskList};
use crate::version::VERSION;

/// The site of one `serve`: its seed and the port it bound.
pub(crate) struct Site {
    seed: u64,
    port: u16,
    /// The families' host names: each family's own and each of its tasks'.
    hosts: BTreeMap<String, Option<String>>,
    /// The seed's certificate authority, as PEM.
    certificate_authority: String,
    observations: Arc<Observations>,
    logins: Logins,
    time_step: Mutex<u64>,
    portal: Portal,
}

/// The Proving ground's time step, as `PUT /time-step` sets it.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct TimeStep {
    step: u64,
}

/// The families' own host names, each family's tasks served on
/// `<ordinal>.<host>`.
const FAMILY_HOSTS: [&str; 4] = [catalog::HOST, rendered::HOST, login::HOST, schedule::HOST];

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
            logins: Logins::default(),
            time_step: Mutex::new(0),
            portal: Portal::new(seed),
        }
    }

    /// Every host name a page family is served on, each with the task its
    /// requests belong to, if any: the family's own, and each task's. Each
    /// has its own leaf certificate, and only these are observed.
    pub(crate) fn hosts(seed: u64) -> BTreeMap<String, Option<String>> {
        let mut hosts: BTreeMap<_, _> = FAMILY_HOSTS
            .into_iter()
            .map(|host| (host.to_owned(), None))
            .collect();
        hosts.extend(
            Self::tasks_of(seed, 0)
                .into_iter()
                .map(|task| (task.host, Some(task.entry.id))),
        );
        hosts.extend(
            portal::hosts()
                .into_iter()
                .chain(handmade::hosts())
                .map(|(host, task)| (host, Some(task))),
        );
        hosts
    }

    /// Every family's tasks, in the task list's order.
    fn tasks(&self) -> Vec<Task> {
        Self::tasks_of(self.seed, self.port)
    }

    fn tasks_of(seed: u64, port: u16) -> Vec<Task> {
        let mut tasks = catalog::tasks(seed, port);
        tasks.extend(rendered::tasks(seed, port));
        tasks.extend(login::tasks(seed, port));
        tasks.extend(schedule::tasks(seed, port));
        tasks.extend(portal::tasks(seed, port));
        tasks.extend(handmade::tasks(seed, port));
        tasks
    }

    /// Answers one request, given its `Host` header.
    pub(crate) fn respond(&self, host: &str, request: &Request<'_>) -> Reply {
        let name = observation::host_name(host);
        if let Some((reply, effect)) = self
            .portal
            .respond(&name, request)
            .or_else(|| handmade::respond(self.seed, self.port, &name, request))
        {
            if let Some(effect) = effect {
                self.observations.effect(host, effect);
            }
            return reply;
        }
        if self.hosts.contains_key(&name) {
            let family = FAMILY_HOSTS
                .into_iter()
                .find(|family| {
                    name == *family
                        || name
                            .strip_suffix(family)
                            .is_some_and(|prefix| prefix.ends_with('.'))
                })
                .expect("invariant: every served host name is a family's or its task's");
            return match family {
                catalog::HOST => self.catalog(host, request),
                rendered::HOST => rendered::respond(self.seed, request),
                login::HOST => login::respond(self.seed, &self.logins, request),
                _ => schedule::respond(self.seed, *self.time_step.lock(), request),
            };
        }
        match (request.method, request.path) {
            (&Method::GET, "/tasks") => Reply::json(
                StatusCode::OK,
                &TaskList::new(VERSION, self.seed, &self.tasks()),
            ),
            (&Method::POST, "/oracle") => self.judge(request.body),
            (&Method::PUT, "/time-step") => self.set_time_step(request.body),
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

    /// Answers a request to the catalog family, recording the Effect of an
    /// order for the request's task.
    fn catalog(&self, host: &str, request: &Request<'_>) -> Reply {
        match (request.method, request.path) {
            (&Method::POST, "/orders") => match catalog::order(request.body) {
                Some((effect, page)) => {
                    self.observations.effect(host, effect);
                    Reply::html(page)
                }
                None => Reply::error(StatusCode::BAD_REQUEST, "unknown_product"),
            },
            (&Method::GET, path) => {
                catalog::page(self.seed, path).map_or_else(Reply::not_found, Reply::html)
            }
            _ => Reply::not_found(),
        }
    }

    /// Sets the time step scheduled content follows.
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
                let now = self.portal.expected_now(&task.entry.id);
                let expected = now.as_ref().unwrap_or(&task.expected);
                Reply::json(
                    StatusCode::OK,
                    &oracle::judge(expected, submission, &effects),
                )
            }
            None => Reply::error(StatusCode::NOT_FOUND, "unknown_task"),
        }
    }
}
