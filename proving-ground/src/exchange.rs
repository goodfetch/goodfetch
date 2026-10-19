//! One request a page family answers and the reply it gives: the parts of
//! HTTP the families read and write, apart from hyper's types.

use hyper::body::Bytes;
use hyper::{Method, StatusCode};
use serde::Serialize;

/// The parts of a request a page family reads.
pub(crate) struct Request<'a> {
    pub(crate) method: &'a Method,
    pub(crate) path: &'a str,
    /// The `Cookie` header, empty when the request has none.
    pub(crate) cookies: &'a str,
    pub(crate) body: &'a Bytes,
}

impl Request<'_> {
    /// The value of the cookie `name` the request sends, if any.
    pub(crate) fn cookie(&self, name: &str) -> Option<&str> {
        self.cookies
            .split(';')
            .filter_map(|pair| pair.trim().split_once('='))
            .find_map(|(key, value)| (key == name).then_some(value))
    }
}

/// A reply's status, media type, extra headers and body.
pub(crate) struct Reply {
    pub(crate) status: StatusCode,
    pub(crate) content_type: &'static str,
    /// Headers beyond `Content-Type`, such as `Location` and `Set-Cookie`.
    pub(crate) headers: Vec<(&'static str, String)>,
    pub(crate) body: Bytes,
}

impl Reply {
    pub(crate) fn json(status: StatusCode, value: &impl Serialize) -> Self {
        Self {
            status,
            content_type: "application/json",
            headers: Vec::new(),
            body: serde_json::to_vec(value)
                .expect("invariant: the Proving ground's own values serialize")
                .into(),
        }
    }

    pub(crate) fn html(page: String) -> Self {
        Self::html_with_status(StatusCode::OK, page)
    }

    pub(crate) fn html_with_status(status: StatusCode, page: String) -> Self {
        Self {
            status,
            content_type: "text/html; charset=utf-8",
            headers: Vec::new(),
            body: page.into(),
        }
    }

    /// A `303 See Other` to `location`, a path on the same host.
    pub(crate) fn see_other(location: &str) -> Self {
        Self {
            status: StatusCode::SEE_OTHER,
            content_type: "text/plain; charset=utf-8",
            headers: vec![("location", location.to_owned())],
            body: Bytes::new(),
        }
    }

    /// A JSON error naming its code.
    pub(crate) fn error(status: StatusCode, code: &str) -> Self {
        Self::json(status, &serde_json::json!({ "error": code }))
    }

    /// The `404` of a path a family does not have.
    pub(crate) fn not_found() -> Self {
        Self::error(StatusCode::NOT_FOUND, "not_found")
    }

    /// Adds a `Set-Cookie` header for a cookie on the whole host.
    #[must_use]
    pub(crate) fn set_cookie(mut self, name: &str, value: &str) -> Self {
        self.headers.push((
            "set-cookie",
            format!("{name}={value}; Path=/; HttpOnly; SameSite=Lax"),
        ));
        self
    }
}
