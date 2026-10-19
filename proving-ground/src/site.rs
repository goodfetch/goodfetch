//! What the Proving ground answers to one request: a page family's page on
//! that family's host name, and on any other host name its own task list
//! (`GET /tasks`), oracle (`POST /oracle`) and time step (`PUT /time-step`),
//! which a task's harness sets and scheduled content follows.

use hyper::body::Bytes;
use hyper::{Method, StatusCode};
use serde::{Deserialize, Serialize};

use crate::catalog;
use crate::exchange::{Reply, Request};
use crate::login::{self, Logins};
use crate::oracle::{self, Submission};
use crate::rendered;
use crate::schedule;
use crate::sync::Mutex;
use crate::task::{Task, TaskList};
use crate::version::VERSION;

/// The site of one `serve`: its seed, the port it bound, and the state its
/// families keep.
pub(crate) struct Site {
    seed: u64,
    port: u16,
    logins: Logins,
    time_step: Mutex<u64>,
}

/// The Proving ground's time step, as `PUT /time-step` sets it.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct TimeStep {
    step: u64,
}

impl Site {
    pub(crate) fn new(seed: u64, port: u16) -> Self {
        Self {
            seed,
            port,
            logins: Logins::default(),
            time_step: Mutex::new(0),
        }
    }

    /// Every family's tasks, in the task list's order.
    fn tasks(&self) -> Vec<Task> {
        let mut tasks = catalog::tasks(self.seed, self.port);
        tasks.extend(rendered::tasks(self.seed, self.port));
        tasks.extend(login::tasks(self.seed, self.port));
        tasks.extend(schedule::tasks(self.seed, self.port));
        tasks
    }

    /// Answers one request, given its `Host` header.
    pub(crate) fn respond(&self, host: &str, request: &Request<'_>) -> Reply {
        let name = host.rsplit_once(':').map_or(host, |(name, _)| name);
        if name.eq_ignore_ascii_case(catalog::HOST) {
            return match (request.method, catalog::page(self.seed, request.path)) {
                (&Method::GET, Some(page)) => Reply::html(page),
                _ => Reply::not_found(),
            };
        }
        if name.eq_ignore_ascii_case(rendered::HOST) {
            return rendered::respond(self.seed, request);
        }
        if name.eq_ignore_ascii_case(login::HOST) {
            return login::respond(self.seed, &self.logins, request);
        }
        if name.eq_ignore_ascii_case(schedule::HOST) {
            return schedule::respond(self.seed, *self.time_step.lock(), request);
        }
        match (request.method, request.path) {
            (&Method::GET, "/tasks") => Reply::json(
                StatusCode::OK,
                &TaskList::new(VERSION, self.seed, &self.tasks()),
            ),
            (&Method::POST, "/oracle") => self.judge(request.body),
            (&Method::PUT, "/time-step") => self.set_time_step(request.body),
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
            Some(task) => Reply::json(StatusCode::OK, &oracle::judge(&task.expected, submission)),
            None => Reply::error(StatusCode::NOT_FOUND, "unknown_task"),
        }
    }
}
