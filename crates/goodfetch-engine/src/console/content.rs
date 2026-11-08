//! The Console's content origin (ADR 0031; spec #592): the second origin
//! that shows a Run's files, scraped HTML, screenshots, DOM snapshots and
//! page bodies, only to an address `content_origin` signed for that one
//! file, and under headers that keep anything it shows from running: a
//! policy of `sandbox` and `script-src 'none'`, framed only by the
//! Console's own origins, no referrer, no sniffing, nothing stored. It
//! sets no cookie and reads none: the address is its only credential.

use std::time::SystemTime;

use http_body_util::Full;
use hyper::body::Bytes;
use hyper::header::{self, HeaderValue};
use hyper::{Method, Response, StatusCode};

use crate::content_origin::{ContentFile, ContentKey, Unsigned};
use crate::endpoint::{self, Authorities, Hosts, Origins};

/// The content origin as the Console's site holds it.
pub(super) struct Content {
    /// What its addresses are signed under.
    pub(super) key: ContentKey,
    /// The origin its addresses name, with no trailing `/`.
    pub(super) origin: String,
    /// The names it is reached at: a request's `Host` must be one, and an
    /// `Origin`, when a request carries one, one of them or the Console's.
    pub(super) authorities: Authorities,
    /// Whether it shares the Console's listener, told apart by `Host`.
    pub(super) shared: bool,
    /// Its policy, which lets only the Console's origins frame it.
    pub(super) policy: HeaderValue,
}

/// What the content origin answers a request, before the file is read.
pub(super) enum Asked {
    /// The file the request's address is signed for, and whether only its
    /// head was asked for.
    File { file: ContentFile, head: bool },
    /// A refusal by the endpoint table's rule `reason`, answered `status`.
    Refused {
        reason: &'static str,
        status: StatusCode,
    },
    /// An answer with no file, such as a path that names none.
    Answered(Response<Full<Bytes>>),
}

impl Content {
    /// The content origin's policy, which lets the Console's origins
    /// `console` frame it: no script, no plugin, no form, no navigation of
    /// another frame, an opaque origin, inline styles and `data:` images
    /// and fonts only.
    pub(super) fn policy(console: &Authorities) -> String {
        format!(
            "sandbox; default-src 'none'; script-src 'none'; style-src 'unsafe-inline'; img-src data:; font-src data:; base-uri 'none'; form-action 'none'; frame-ancestors {}",
            console.origins().join(" ")
        )
    }

    /// Whether a request naming `host` on the Console's shared listener is
    /// the content origin's.
    pub(super) fn takes(&self, host: Option<&str>) -> bool {
        self.shared && host.is_some_and(|host| self.authorities.admits_host(host))
    }

    /// What a request of `method` for `target` with `host` and `origin`
    /// asks for at `now`, whose Console is reached at `console`.
    pub(super) fn asked(
        &self,
        method: &Method,
        target: &str,
        host: Option<&str>,
        origin: Option<&str>,
        console: &Authorities,
        now: SystemTime,
    ) -> Asked {
        if let Some(refusal) = endpoint::refusal(
            host,
            origin,
            Hosts::Named(&self.authorities),
            Origins::Allowed(&[&self.authorities, console]),
        ) {
            return Asked::Refused {
                reason: refusal.reason(),
                status: StatusCode::FORBIDDEN,
            };
        }
        let file = match self.key.check(target, now) {
            Ok(file) => file,
            Err(Unsigned::NoFile) => return Asked::Answered(self.plain(StatusCode::NOT_FOUND)),
            Err(refused) => {
                return Asked::Refused {
                    reason: refused.reason().unwrap_or("token_wrong"),
                    status: StatusCode::UNAUTHORIZED,
                };
            }
        };
        if method != Method::GET && method != Method::HEAD {
            let mut response = self.plain(StatusCode::METHOD_NOT_ALLOWED);
            response
                .headers_mut()
                .insert(header::ALLOW, HeaderValue::from_static("GET, HEAD"));
            return Asked::Answered(response);
        }
        Asked::File {
            file,
            head: method == Method::HEAD,
        }
    }

    /// The answer of `file`'s bytes, `bytes`, or of its head alone.
    pub(super) fn file(
        &self,
        file: &ContentFile,
        bytes: Vec<u8>,
        head: bool,
    ) -> Response<Full<Bytes>> {
        let length = bytes.len();
        let body = if head {
            Bytes::new()
        } else {
            Bytes::from(bytes)
        };
        let mut response = self.plain(StatusCode::OK);
        *response.body_mut() = Full::new(body);
        let headers = response.headers_mut();
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static(file.shown.media_type()),
        );
        headers.insert(header::CONTENT_LENGTH, HeaderValue::from(length));
        response
    }

    /// An answer with no body, under the content origin's headers.
    pub(super) fn plain(&self, status: StatusCode) -> Response<Full<Bytes>> {
        let mut response = Response::new(Full::new(Bytes::new()));
        *response.status_mut() = status;
        let headers = response.headers_mut();
        headers.insert(header::CONTENT_SECURITY_POLICY, self.policy.clone());
        headers.insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("private, no-store"),
        );
        headers.insert(
            header::REFERRER_POLICY,
            HeaderValue::from_static("no-referrer"),
        );
        headers.insert(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        );
        headers.insert(
            "cross-origin-opener-policy",
            HeaderValue::from_static("same-origin"),
        );
        headers.insert(
            "cross-origin-resource-policy",
            HeaderValue::from_static("same-site"),
        );
        response
    }
}
