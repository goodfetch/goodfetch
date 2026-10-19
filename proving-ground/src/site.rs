//! What the Proving ground answers to one request: a page family's page on
//! that family's host names, and the Effect its form carries out, and on any other host name its own task list
//! (`GET /tasks`), oracle (`POST /oracle`), certificate authority
//! (`GET /certificate-authority`) and observations (`GET /observations`).

use std::collections::BTreeMap;
use std::net::IpAddr;
use std::sync::Arc;

use hyper::body::Bytes;
use hyper::{Method, StatusCode};
use serde::Serialize;

use crate::catalog;
use crate::crawl::{self, Families};
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
    /// What the Crawl families saw crawls do.
    crawl: Families,
}

/// A reply's status, media type, extra headers and body.
pub(crate) struct Reply {
    pub(crate) status: StatusCode,
    pub(crate) content_type: &'static str,
    /// Headers beyond `Content-Type`, such as `Location` and `Retry-After`.
    pub(crate) headers: Vec<(&'static str, String)>,
    pub(crate) body: Bytes,
    /// Set when the server holds the reply a while before it sends it.
    pub(crate) held: Option<crawl::Held>,
}

impl From<crawl::Page> for Reply {
    fn from(page: crawl::Page) -> Self {
        Self {
            status: page.status,
            content_type: page.content_type,
            headers: page.headers,
            body: page.body,
            held: page.held,
        }
    }
}

impl Reply {
    fn json(status: StatusCode, value: &impl Serialize) -> Self {
        Self {
            status,
            content_type: "application/json",
            headers: Vec::new(),
            body: serde_json::to_vec(value)
                .expect("invariant: the Proving ground's own values serialize")
                .into(),
            held: None,
        }
    }

    fn html(page: String) -> Self {
        Self {
            status: StatusCode::OK,
            content_type: "text/html; charset=utf-8",
            headers: Vec::new(),
            body: page.into(),
            held: None,
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
            crawl: Families::new(),
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
        hosts.extend(crawl::hosts(seed));
        hosts
    }

    /// Every family's tasks, in the task list's order.
    fn tasks(&self) -> Vec<Task> {
        let mut tasks = catalog::tasks(self.seed, self.port);
        tasks.extend(crawl::tasks(self.seed, self.port));
        tasks
    }

    /// Answers one request, given its `Host` header, path, query (empty when
    /// it has none), the address it came from, and its body.
    pub(crate) fn respond(
        &self,
        method: &Method,
        host: &str,
        path: &str,
        query: &str,
        peer: IpAddr,
        body: &Bytes,
    ) -> Reply {
        let name = observation::host_name(host);
        if self.hosts.contains_key(&name) {
            let request = crawl::Request {
                method,
                host: &name,
                authority: host,
                path,
                query,
                peer,
            };
            if let Some(page) = self.crawl.respond(self.seed, &request, &self.observations) {
                return page.into();
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
                headers: Vec::new(),
                body: self.observations.document().into(),
                held: None,
            },
            (&Method::GET, "/certificate-authority") => Reply {
                status: StatusCode::OK,
                content_type: "application/x-pem-file",
                headers: Vec::new(),
                body: self.certificate_authority.clone().into(),
                held: None,
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
                let findings = self.observations.findings_of(&task.entry.id);
                Reply::json(
                    StatusCode::OK,
                    &oracle::judge(&task.expected, submission, &effects, findings),
                )
            }
            None => Reply::error(StatusCode::NOT_FOUND, "unknown_task"),
        }
    }
}
