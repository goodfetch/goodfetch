//! One request a page family answers and the reply it gives: the parts of
//! HTTP the families read and write, apart from hyper's types.

use std::net::IpAddr;

use hyper::body::Bytes;
use hyper::{Method, StatusCode};
use serde::Serialize;

use crate::client_hello::ClientHello;

/// The parts of a request a page family reads.
pub(crate) struct Request<'a> {
    pub(crate) method: &'a Method,
    pub(crate) path: &'a str,
    /// The query, without its `?`; empty when the target has none.
    pub(crate) query: &'a str,
    /// The `Cookie` header, empty when the request has none.
    pub(crate) cookies: &'a str,
    /// The `Authorization` header, empty when the request has none.
    pub(crate) authorization: &'a str,
    pub(crate) body: &'a Bytes,
    /// The address the request came from, as the site sees it.
    pub(crate) peer: IpAddr,
    /// The `ClientHello` of the request's connection; `None` over plain
    /// text.
    pub(crate) tls: Option<&'a ClientHello>,
}

impl Request<'_> {
    /// The first value of the query parameter `name`, percent-decoded, if
    /// the query has it.
    pub(crate) fn parameter(&self, name: &str) -> Option<String> {
        form_value(self.query, name)
    }

    /// The value of the cookie `name` the request sends, if any.
    pub(crate) fn cookie(&self, name: &str) -> Option<&str> {
        cookie(self.cookies, name)
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

    /// A `204 No Content`.
    pub(crate) fn no_content() -> Self {
        Self {
            status: StatusCode::NO_CONTENT,
            content_type: "text/plain; charset=utf-8",
            headers: Vec::new(),
            body: Bytes::new(),
        }
    }

    /// A `302 Found` to `location`, a path on the same host.
    pub(crate) fn found(location: &str) -> Self {
        Self {
            status: StatusCode::FOUND,
            ..Self::see_other(location)
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

/// The value of the cookie `name` in a `Cookie` header's `cookies`, if
/// it holds one.
pub(crate) fn cookie<'a>(cookies: &'a str, name: &str) -> Option<&'a str> {
    cookies
        .split(';')
        .filter_map(|pair| pair.trim().split_once('='))
        .find_map(|(key, value)| (key == name).then_some(value))
}

/// The first value of `name` in a form-encoded text (a query or a form body),
/// percent-decoded, `+` read as a space; `None` when it is absent or does not
/// decode to UTF-8.
pub(crate) fn form_value(form: &str, name: &str) -> Option<String> {
    form.split('&')
        .filter_map(|pair| pair.split_once('=').or(Some((pair, ""))))
        .find(|(key, _)| decode(key).as_deref() == Some(name))
        .and_then(|(_, value)| decode(value))
}

/// A form-encoded component, percent-decoded, `+` read as a space; `None`
/// when it does not decode to UTF-8.
pub(crate) fn decode(component: &str) -> Option<String> {
    let mut bytes = Vec::with_capacity(component.len());
    let mut rest = component.bytes();
    while let Some(byte) = rest.next() {
        bytes.push(match byte {
            b'+' => b' ',
            b'%' => {
                let high = char::from(rest.next()?).to_digit(16)?;
                let low = char::from(rest.next()?).to_digit(16)?;
                u8::try_from(high * 16 + low).ok()?
            }
            other => other,
        });
    }
    String::from_utf8(bytes).ok()
}

/// A text encoded as one component of a query, every byte but an unreserved
/// one (RFC 3986) percent-encoded.
pub(crate) fn encode(text: &str) -> String {
    text.bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
                char::from(byte).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_form_value_is_percent_decoded() {
        assert_eq!(
            form_value("a=1&next=%2Freports%3Fpage%3D2&b", "next").as_deref(),
            Some("/reports?page=2")
        );
        assert_eq!(
            form_value("name=Ada+L%C3%B6", "name").as_deref(),
            Some("Ada Lö")
        );
        assert_eq!(form_value("b&a=1", "b").as_deref(), Some(""));
        assert_eq!(form_value("a=1", "b"), None);
        assert_eq!(form_value("a=%zz", "a"), None);
    }

    #[test]
    fn an_encoded_component_decodes_to_itself() {
        assert_eq!(encode("/reports?page=2"), "%2Freports%3Fpage%3D2");
        assert_eq!(decode(&encode("Ünï ✓&=")).as_deref(), Some("Ünï ✓&="));
    }
}
