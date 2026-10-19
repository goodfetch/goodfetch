//! What the Proving ground answers to one request: a page family's page on
//! that family's host names, and the Effect its form carries out, and on any other host name its own task list
//! (`GET /tasks`), oracle (`POST /oracle`), certificate authority
//! (`GET /certificate-authority`) and observations (`GET /observations`).

use std::collections::BTreeMap;
use std::sync::Arc;

use hyper::body::Bytes;
use hyper::{Method, StatusCode};

use crate::adversarial;
use crate::catalog;
use crate::exchange::{Reply, Request};
use crate::observation::{self, Observations};
use crate::oracle::{self, Finding, Submission};
use crate::protection;
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
        hosts.extend(adversarial::hosts());
        hosts.extend(protection::hosts());
        hosts
    }

    /// Every family's tasks, in the task list's order.
    fn tasks(&self) -> Vec<Task> {
        let mut tasks = catalog::tasks(self.seed, self.port);
        tasks.extend(adversarial::tasks(self.seed, self.port));
        tasks.extend(protection::tasks(self.seed, self.port));
        tasks
    }

    /// Answers one request, given its `Host` header.
    pub(crate) fn respond(&self, host: &str, request: &Request<'_>) -> Reply {
        let name = observation::host_name(host);
        if let Some((reply, recorded)) = adversarial::respond(self.seed, host, &name, request)
            .or_else(|| protection::respond(self.seed, &name, request))
        {
            if let Some(recorded) = recorded {
                self.observations.record(host, recorded);
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
                let submitted = submission.record_ids();
                let answer = oracle::judge(&task.expected, submission, &effects)
                    .refuted_by(self.reported(&task.entry.id));
                if task.entry.id == protection::TIER_MOVE_TASK {
                    let answer =
                        protection::judge_tier_move(answer, &submitted, &self.observations);
                    return Reply::json(StatusCode::OK, &answer);
                }
                Reply::json(StatusCode::OK, &answer)
            }
            None => Reply::error(StatusCode::NOT_FOUND, "unknown_task"),
        }
    }

    /// What the pages of `task` reported beyond having run: each a finding,
    /// since a page's script reports only what must not have worked.
    fn reported(&self, task: &str) -> Vec<Finding> {
        self.observations
            .reports_of(task)
            .into_iter()
            .filter(|report| report.check != observation::RAN)
            .map(|report| Finding {
                check: report.check,
                at: report.page,
                expected: None,
                observed: report.observed.into(),
            })
            .collect()
    }
}
