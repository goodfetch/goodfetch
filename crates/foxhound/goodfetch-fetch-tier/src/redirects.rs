//! Fetch's redirect rules (ADR 0118; the Fetch standard's "HTTP-redirect
//! fetch"), as a pure function of a hop and its answer's head: which
//! answers are followed, and the hop each one leads to.
//!
//! - **Followed:** 301, 302, 303, 307 and 308 with a `Location` that
//!   parses, against the hop's URL, to an HTTP(S) URL. Any other 3xx, or
//!   one of those whose `Location` is missing, does not parse or names
//!   another scheme, is [`Next::NotOk`].
//! - **Method and body:** a 301 or 302 turns a `POST` into a `GET`, and a
//!   303 turns any method but `GET` or `HEAD` into a `GET`, each without
//!   its body and the request-body headers; a 307 or 308 keeps both.
//! - **Headers:** `Authorization` is removed once a hop goes to another
//!   origin than the hop before it, and stays removed.

use url::Url;

use crate::driver::Hop;

/// What follows a hop's answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Next {
    /// No redirect: the answer is the hop's document.
    Answer,
    /// A redirect, to this hop.
    Follow(Hop),
    /// A 3xx that is not followed, which fails the fetch with
    /// `response_not_ok`, and why.
    NotOk(String),
}

/// What follows `hop`'s answer of `status` whose `Location` is `location`.
pub(crate) fn next(hop: &Hop, status: u16, location: Option<&str>) -> Next {
    if !(300..400).contains(&status) {
        return Next::Answer;
    }
    if !REDIRECTS.contains(&status) {
        return Next::NotOk(format!(
            "a {status} is not one of Fetch's redirect statuses"
        ));
    }
    let Some(location) = location else {
        return Next::NotOk(format!("the {status} names no `Location`"));
    };
    let url = match hop.url.join(location.trim()) {
        Ok(url) if matches!(url.scheme(), "http" | "https") => url,
        Ok(url) => {
            return Next::NotOk(format!(
                "the {status}'s `Location` names a `{}:` URL, not an HTTP(S) one",
                url.scheme()
            ));
        }
        Err(error) => {
            return Next::NotOk(format!(
                "the {status}'s `Location` `{location}` does not parse: {error}"
            ));
        }
    };
    let to_get = match status {
        301 | 302 => hop.method == http::Method::POST,
        303 => ![http::Method::GET, http::Method::HEAD].contains(&hop.method),
        _ => false,
    };
    Next::Follow(hop_to(hop, url, to_get))
}

/// Fetch's redirect statuses.
const REDIRECTS: [u16; 5] = [301, 302, 303, 307, 308];

/// The headers that describe a request's body, which a hop turned into a
/// `GET` drops with the body (Fetch's request-body-header names).
const REQUEST_BODY_HEADERS: [&str; 4] = [
    "content-encoding",
    "content-language",
    "content-location",
    "content-type",
];

/// The hop from `hop` to `url` that turns into a `GET` with no body when
/// `to_get`, as a redirect or a refresh stub leads (ADR 0118):
/// `Authorization` is removed when `url` is of another origin.
pub(crate) fn hop_to(hop: &Hop, url: Url, to_get: bool) -> Hop {
    let cross_origin = hop.url.origin() != url.origin();
    let headers = hop
        .headers
        .iter()
        .filter(|(name, _)| {
            let dropped_with_body = to_get
                && (REQUEST_BODY_HEADERS
                    .iter()
                    .any(|header| name.eq_ignore_ascii_case(header))
                    || name.eq_ignore_ascii_case("content-length"));
            let dropped_cross_origin = cross_origin && name.eq_ignore_ascii_case("authorization");
            !(dropped_with_body || dropped_cross_origin)
        })
        .cloned()
        .collect();
    let (method, body) = if to_get {
        (http::Method::GET, None)
    } else {
        (hop.method.clone(), hop.body.clone())
    };
    Hop {
        url,
        method,
        headers,
        body,
        read: hop.read,
    }
}

#[cfg(test)]
mod tests;
