//! What the Proving ground answers to one request: a page family's page on
//! that family's host names, and the Effect its form carries out, and on any
//! other host name its own task list (`GET /tasks`), oracle (`POST /oracle`),
//! time step (`PUT /time-step`), which a task's harness sets and scheduled
//! content and drifting pages follow, certificate authority
//! (`GET /certificate-authority`), observations (`GET /observations`), the
//! replay proxy's refusals (`GET /replay`), the Fake model's calls
//! (`GET /model-calls`) and the revocation of the portal's sessions
//! (`POST /revoke-sessions`), which a harness asks for while a tool is
//! stopped.

use std::collections::BTreeMap;
use std::sync::Arc;

use hyper::body::Bytes;
use hyper::http::uri::PathAndQuery;
use hyper::{Method, StatusCode, Uri};
use serde::{Deserialize, Serialize};

use crate::agent;
use crate::catalog;
use crate::encodings;
use crate::exchange::{Reply, Request};
use crate::extraction;
use crate::files;
use crate::handmade;
use crate::languages;
use crate::login::{self, Logins};
use crate::model::{self, Candidate, FakeModel};
use crate::observation::{self, Observations};
use crate::oracle::{self, Submission};
use crate::portal::{self, Portal};
use crate::rendered;
use crate::replay::{self, Refusals};
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
    model: FakeModel,
    refusals: Refusals,
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
            model: FakeModel::new(seed),
            refusals: Refusals::default(),
        }
    }

    /// Every host name a page family is served on, each with the task its
    /// requests belong to, if any: the family's own, and each task's. Each
    /// has its own leaf certificate, and only these are observed.
    pub(crate) fn hosts(seed: u64) -> BTreeMap<String, Option<String>> {
        let mut hosts: BTreeMap<_, _> = FAMILY_HOSTS
            .into_iter()
            .chain([
                agent::HOST,
                model::HOST,
                encodings::HOST,
                extraction::HOST,
                files::HOST,
                languages::HOST,
            ])
            .map(|host| (host.to_owned(), None))
            .collect();
        hosts.extend(
            Self::every_task(seed, None)
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

    /// Every family's tasks, as this `serve` lists them.
    fn tasks(&self) -> Vec<Task> {
        Self::every_task(self.seed, Some(self.port))
    }

    /// Every family's tasks for `seed`, in the task list's order, their start
    /// URLs on `port`, or on none when listed without serving.
    pub(crate) fn every_task(seed: u64, port: Option<u16>) -> Vec<Task> {
        let mut tasks = catalog::tasks(seed, port);
        tasks.extend(rendered::tasks(seed, port));
        tasks.extend(login::tasks(seed, port));
        tasks.extend(schedule::tasks(seed, port));
        tasks.extend(portal::tasks(seed, port));
        tasks.extend(handmade::tasks(seed, port));
        tasks.extend(agent::tasks(seed, port));
        tasks.extend(replay::tasks());
        tasks.extend(extraction::tasks(seed, port));
        tasks.extend(encodings::tasks(seed, port));
        tasks.extend(languages::tasks(seed, port));
        tasks.extend(files::tasks(seed, port));
        tasks
    }

    /// The reply of an Extraction family (#165) to a request on `name`, one
    /// of its host names; `None` when `name` is another family's.
    fn extraction(&self, name: &str, request: &Request<'_>) -> Option<Reply> {
        let (method, path, query) = (request.method, request.path, request.query);
        let of = |family: &str| name == family || name.ends_with(&format!(".{family}"));
        let ok = |(content_type, body)| (StatusCode::OK, content_type, body);
        let html = |page: String| ok(("text/html; charset=utf-8", page.into_bytes()));
        let page = if of(extraction::HOST) {
            extraction::page(self.seed, path).map(ok)
        } else if of(encodings::HOST) {
            encodings::page(self.seed, path).map(ok)
        } else if of(languages::HOST) {
            languages::page(self.seed, name, path, query).map(html)
        } else if of(files::HOST) {
            files::page(self.seed, path).map(|file| (file.status, file.content_type, file.body))
        } else {
            return None;
        };
        Some(match (method, page) {
            (&Method::GET, Some((status, content_type, body))) => Reply {
                status,
                content_type,
                headers: Vec::new(),
                body: body.into(),
            },
            _ => Reply::not_found(),
        })
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
        if name == model::HOST {
            // The Fake model answers its own paths (`Self::model`).
            return Reply::not_found();
        }
        if replay::serves(&name) {
            return self.replay(request.method, &name, request.path);
        }
        if self.hosts.contains_key(&name) && agent::serves(&name) {
            return self.agent(host, &name, request);
        }
        if self.hosts.contains_key(&name) {
            if let Some(reply) = self.extraction(&name, request) {
                return reply;
            }
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
            (&Method::GET, "/replay") => Reply {
                status: StatusCode::OK,
                content_type: "application/json",
                headers: Vec::new(),
                body: self.refusals.document().into(),
            },
            (&Method::GET, "/model-calls") => Reply {
                status: StatusCode::OK,
                content_type: "application/json",
                headers: Vec::new(),
                body: self.model.document().into(),
            },
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

    /// Answers a request to the `agent` family: its pages at the current
    /// time step, and the Effects their script posts, recorded with it.
    fn agent(&self, host: &str, name: &str, request: &Request<'_>) -> Reply {
        let step = *self.time_step.lock();
        match (request.method, request.path) {
            (&Method::POST, agent::EFFECTS_PATH) => match agent::effect(request.body, step) {
                Some(effect) => {
                    self.observations.effect(host, effect);
                    Reply::json(StatusCode::OK, &serde_json::json!({ "time_step": step }))
                }
                None => Reply::error(StatusCode::BAD_REQUEST, "malformed_effect"),
            },
            (&Method::GET, path) => {
                agent::page(self.seed, name, step, path).map_or_else(Reply::not_found, Reply::html)
            }
            _ => Reply::not_found(),
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

    /// Sets the time step scheduled content and drifting pages follow.
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
