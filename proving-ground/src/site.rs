//! What the Proving ground answers to one request: a page family's page on
//! that family's host name, and on any other host name its own task list
//! (`GET /tasks`) and oracle (`POST /oracle`).

use hyper::body::Bytes;
use hyper::{Method, StatusCode};
use serde::Serialize;

use crate::catalog;
use crate::task::{Task, TaskList};
use crate::version::VERSION;

/// The site of one `serve`: its seed and the port it bound.
pub(crate) struct Site {
    seed: u64,
    port: u16,
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
    pub(crate) fn new(seed: u64, port: u16) -> Self {
        Self { seed, port }
    }

    /// Every family's tasks, in the task list's order.
    fn tasks(&self) -> Vec<Task> {
        catalog::tasks(self.seed, self.port)
    }

    /// Answers one request, given its `Host` header, path and body.
    pub(crate) fn respond(&self, method: &Method, host: &str, path: &str, _body: &Bytes) -> Reply {
        let name = host.rsplit_once(':').map_or(host, |(name, _)| name);
        if name.eq_ignore_ascii_case(catalog::HOST) {
            return match (method, catalog::page(self.seed, path)) {
                (&Method::GET, Some(page)) => Reply::html(page),
                _ => Reply::error(StatusCode::NOT_FOUND, "not_found"),
            };
        }
        match (method, path) {
            (&Method::GET, "/tasks") => Reply::json(
                StatusCode::OK,
                &TaskList::new(VERSION, self.seed, &self.tasks()),
            ),
            _ => Reply::error(StatusCode::NOT_FOUND, "not_found"),
        }
    }
}
