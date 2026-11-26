//! What the Response cache may store, and whether what it holds answers a
//! request now (ADR 0046): RFC 9111's rules as one private cache applies
//! them, through `http-cache-semantics`, as pure functions of a request,
//! a stored response and the moment the caller's Clock gives. Nothing here
//! reads a clock, a file or the index.

use std::sync::OnceLock;
use std::time::{Duration, SystemTime};

use bytes::Bytes;
use http::header::{HeaderMap, HeaderName, HeaderValue};
use http_cache_semantics::{AfterResponse, BeforeRequest, CacheOptions, CachePolicy};
use sha2::{Digest as _, Sha256};

use super::canonical;

/// A request as the cache keys and judges it: its method, its URL without
/// its fragment, its headers, names in lower case, and its body, whose
/// canonical hash `replay` keys it by, taken once and only when asked.
#[derive(Clone, Debug)]
pub struct CacheRequest {
    /// Its method.
    pub method: String,
    /// Its URL, without its fragment.
    pub url: String,
    /// Its headers, names in lower case, in the order sent.
    pub headers: Vec<(String, String)>,
    body: Bytes,
    replay_key: OnceLock<String>,
}

impl PartialEq for CacheRequest {
    fn eq(&self, other: &Self) -> bool {
        (&self.method, &self.url, &self.headers, &self.body)
            == (&other.method, &other.url, &other.headers, &other.body)
    }
}

impl Eq for CacheRequest {}

impl CacheRequest {
    /// The request `method` sends to `url`, its fragment dropped, with
    /// `headers`, whose names are put in lower case, and `body`, empty for
    /// none.
    pub fn new(method: &str, url: &url::Url, headers: &[(String, String)], body: Bytes) -> Self {
        let mut url = url.clone();
        url.set_fragment(None);
        Self {
            method: method.to_owned(),
            url: url.into(),
            headers: headers
                .iter()
                .map(|(name, value)| (name.to_ascii_lowercase(), value.clone()))
                .collect(),
            body,
            replay_key: OnceLock::new(),
        }
    }

    /// Its body; empty for none.
    pub fn body(&self) -> &Bytes {
        &self.body
    }

    /// The SHA-256 of its canonical body, as lower-case hex, which `replay`
    /// keys it by: hashed on the first call only.
    pub(crate) fn replay_key(&self) -> &str {
        self.replay_key.get_or_init(|| {
            canonical::body_sha256(first(&self.headers, "content-type"), &self.body)
        })
    }
}

/// A response's head: its status and its headers, names in lower case, in
/// the order received.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResponseHead {
    /// Its status.
    pub status: u16,
    /// Its headers, names in lower case, in the order received.
    pub headers: Vec<(String, String)>,
}

/// What the index holds of one response, which freshness is judged on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Stored {
    /// The request's method.
    pub(crate) method: String,
    /// The request's URL, without its fragment.
    pub(crate) url: String,
    /// The request headers the response's `Vary` names, and `host`.
    pub(crate) request_headers: Vec<(String, String)>,
    /// The response's head.
    pub(crate) head: ResponseHead,
    /// When the response was received.
    pub(crate) fetched_at: SystemTime,
}

/// The variant of a URL a storable response answers: the key of its
/// `Vary`-named request headers, and those headers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Variant {
    /// The SHA-256 of the `Vary`-named request headers, as lower-case hex.
    pub(crate) vary_key: String,
    /// The `Vary`-named request headers, and `host`, as the request sent
    /// them.
    pub(crate) request_headers: Vec<(String, String)>,
}

/// Whether a stored response answers a request now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Freshness {
    /// Fresh: served with no request, `age` old.
    Fresh {
        /// How long it has sat in caches, as its `Age` header says.
        age: Duration,
    },
    /// Stale: the request is sent with these conditions, `If-None-Match`
    /// from its `ETag` and `If-Modified-Since` from its `Last-Modified`,
    /// whichever it has.
    Stale {
        /// The conditional headers to add, names in lower case.
        conditions: Vec<(String, String)>,
    },
    /// It answers another request: another URL, method or variant.
    Other,
}

/// The request headers that make a request the Session partition's
/// (#795), never the public one, and that a Response recording's replay
/// leaves out of a `Vary` match, since a replayed Run sends its own.
pub(crate) const CREDENTIALS: &[&str] = &["cookie", "authorization", "proxy-authorization"];

/// The validators a revalidation sends.
const CONDITIONS: &[&str] = &["if-none-match", "if-modified-since"];

/// The options of a private cache: `private` responses stored, `s-maxage`
/// ignored, the library's 10 % heuristic for a response with only
/// `Last-Modified`.
fn options() -> CacheOptions {
    CacheOptions {
        shared: false,
        ..CacheOptions::default()
    }
}

/// Whether the public partition answers `request`: a GET or a HEAD sent
/// with no cookie or credential. Any other is sent as it is, or answered
/// from the Session partition (#795).
pub(crate) fn public(request: &CacheRequest) -> bool {
    matches!(request.method.as_str(), "GET" | "HEAD") && !credentialed(request)
}

/// Whether `request` carries a cookie or a credential.
pub(crate) fn credentialed(request: &CacheRequest) -> bool {
    request
        .headers
        .iter()
        .any(|(name, _)| CREDENTIALS.contains(&name.as_str()))
}

/// Whether `head` keeps its response out of the public partition: it sets
/// a cookie or is `private`.
fn personal(head: &ResponseHead) -> bool {
    head.headers.iter().any(|(name, _)| name == "set-cookie")
        || directives(&head.headers, "cache-control").any(|directive| directive == "private")
}

/// The variant `head`, the response to `request`, is stored as in the
/// public partition; none when it is not storable there: a method other
/// than GET or HEAD, a request with a cookie or a credential, a response
/// that sets a cookie or is `private`, `no-store`, `Vary: *`, a status RFC
/// 9111 does not let a cache store or a header the `http` crate refuses.
pub(crate) fn storable(request: &CacheRequest, head: &ResponseHead) -> Option<Variant> {
    if !public(request) || personal(head) {
        return None;
    }
    let policy = CachePolicy::new_options(
        &request_parts(&request.method, &request.url, &request.headers)?,
        &response_parts(head)?,
        SystemTime::UNIX_EPOCH,
        options(),
    );
    if !policy.is_storable() {
        return None;
    }
    variant(request, head)
}

/// The variant `head`, the response to `request`, is stored as under
/// `replay` in the public partition, whatever its method, status or
/// freshness; none for a request with a cookie or a credential, a response
/// that sets a cookie or is `private`, or `Vary: *`.
pub(crate) fn replayable(request: &CacheRequest, head: &ResponseHead) -> Option<Variant> {
    if credentialed(request) || personal(head) {
        return None;
    }
    variant(request, head)
}

/// The variant of its URL `request` asks for of a response with `head`:
/// the SHA-256 of the request headers its `Vary` names; none for
/// `Vary: *`, which no request's headers name.
pub(crate) fn variant(request: &CacheRequest, head: &ResponseHead) -> Option<Variant> {
    let mut names: Vec<String> = directives(&head.headers, "vary").collect();
    if names.iter().any(|name| name == "*") {
        return None;
    }
    names.sort();
    names.dedup();
    let mut hash = Sha256::new();
    for name in &names {
        hash.update(name.as_bytes());
        match first(&request.headers, name) {
            Some(value) => {
                hash.update(b"=");
                hash.update(value.as_bytes());
            }
            None => hash.update(b"!"),
        }
        hash.update(b"\n");
    }
    let request_headers = request
        .headers
        .iter()
        .filter(|(name, _)| name == "host" || names.binary_search(name).is_ok())
        .cloned()
        .collect();
    Some(Variant {
        vary_key: data_encoding::HEXLOWER.encode(&hash.finalize()),
        request_headers,
    })
}

/// Whether `stored` answers `request` at `now`.
pub(crate) fn freshness(stored: &Stored, request: &CacheRequest, now: SystemTime) -> Freshness {
    let (Some(policy), Some(presented)) = (
        policy(stored),
        request_parts(&request.method, &request.url, &request.headers),
    ) else {
        return Freshness::Other;
    };
    match policy.before_request(&presented, now) {
        BeforeRequest::Fresh(_) => Freshness::Fresh {
            age: policy.age(now),
        },
        BeforeRequest::Stale {
            matches: true,
            request: revalidation,
        } => Freshness::Stale {
            conditions: CONDITIONS
                .iter()
                .filter_map(|name| {
                    let value = revalidation.headers.get(*name)?.to_str().ok()?;
                    Some(((*name).to_owned(), value.to_owned()))
                })
                .collect(),
        },
        BeforeRequest::Stale { matches: false, .. } => Freshness::Other,
    }
}

/// The stored headers that describe the body, which a 304 never updates
/// (RFC 9111, section 4.3.4).
const KEPT: &[&str] = &[
    "content-length",
    "content-encoding",
    "transfer-encoding",
    "content-range",
];

/// The stored response's headers updated by `head`, a `304 Not Modified`
/// answering the revalidation of `stored` for `request` at `now`; none when
/// `head` is no such answer, or its validators name another response, so
/// the stored body may not be served.
///
/// Each stored header the 304 also sends takes the 304's values in its
/// place, but those that describe the body (RFC 9111, section 4.3.4), and
/// the stored order is kept; a header only the 304 sends is not added.
pub(crate) fn not_modified(
    stored: &Stored,
    request: &CacheRequest,
    head: &ResponseHead,
    now: SystemTime,
) -> Option<Vec<(String, String)>> {
    let policy = policy(stored)?;
    let presented = request_parts(&request.method, &request.url, &request.headers)?;
    match policy.after_response(&presented, &response_parts(head)?, now) {
        AfterResponse::NotModified(..) => {}
        AfterResponse::Modified(..) => return None,
    }
    let mut headers = Vec::with_capacity(stored.head.headers.len());
    let mut replaced: Vec<&str> = Vec::new();
    for (name, value) in &stored.head.headers {
        let updated =
            !KEPT.contains(&name.as_str()) && head.headers.iter().any(|(sent, _)| sent == name);
        if !updated {
            headers.push((name.clone(), value.clone()));
        } else if !replaced.contains(&name.as_str()) {
            replaced.push(name);
            headers.extend(
                head.headers
                    .iter()
                    .filter(|(sent, _)| sent == name)
                    .cloned(),
            );
        }
    }
    Some(headers)
}

/// The headers a hit is served with: the stored ones, its `Age` replaced
/// by `age`, in whole seconds (RFC 9111, section 5.1).
pub(crate) fn served_headers(stored: &[(String, String)], age: Duration) -> Vec<(String, String)> {
    let mut headers: Vec<(String, String)> = stored
        .iter()
        .filter(|(name, _)| name != "age")
        .cloned()
        .collect();
    headers.push(("age".to_owned(), age.as_secs().to_string()));
    headers
}

/// The policy `stored` was stored under, rebuilt from what the index holds;
/// none when a stored header no longer parses.
fn policy(stored: &Stored) -> Option<CachePolicy> {
    Some(CachePolicy::new_options(
        &request_parts(&stored.method, &stored.url, &stored.request_headers)?,
        &response_parts(&stored.head)?,
        stored.fetched_at,
        options(),
    ))
}

/// The comma-separated values of every `name` header in `headers`, trimmed
/// and in lower case, a directive's argument dropped.
fn directives<'a>(
    headers: &'a [(String, String)],
    name: &'a str,
) -> impl Iterator<Item = String> + 'a {
    headers
        .iter()
        .filter(move |(header, _)| header == name)
        .flat_map(|(_, value)| value.split(','))
        .map(|directive| {
            directive
                .split('=')
                .next()
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase()
        })
        .filter(|directive| !directive.is_empty())
}

/// The first value of the header `name` in `headers`.
pub(crate) fn first<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(header, _)| header == name)
        .map(|(_, value)| value.as_str())
}

fn header_map(headers: &[(String, String)]) -> Option<HeaderMap> {
    let mut map = HeaderMap::with_capacity(headers.len());
    for (name, value) in headers {
        map.append(
            HeaderName::from_bytes(name.as_bytes()).ok()?,
            HeaderValue::from_str(value).ok()?,
        );
    }
    Some(map)
}

fn request_parts(
    method: &str,
    url: &str,
    headers: &[(String, String)],
) -> Option<http::request::Parts> {
    let (mut parts, ()) = http::Request::builder()
        .method(method)
        .uri(url)
        .body(())
        .ok()?
        .into_parts();
    parts.headers = header_map(headers)?;
    Some(parts)
}

fn response_parts(head: &ResponseHead) -> Option<http::response::Parts> {
    let (mut parts, ()) = http::Response::builder()
        .status(head.status)
        .body(())
        .ok()?
        .into_parts();
    parts.headers = header_map(&head.headers)?;
    Some(parts)
}

#[cfg(test)]
mod tests;
