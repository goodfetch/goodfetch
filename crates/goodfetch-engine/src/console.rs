//! The Console's HTTP surface (ADRs 0031, 0043 and 0057): the files of a
//! Product's Console build, a value only the executable crates pass in
//! their composition, so an SDK crate's linked engine serves no Console,
//! answered from its listener as they are stored, brotli-compressed, with
//! `Content-Encoding: br`, and every answer under the production headers:
//! the strict Content Security Policy (own scripts and styles only, no
//! inline code or `eval`, Trusted Types, nothing third-party), `frame-
//! ancestors 'none'`, `Referrer-Policy: no-referrer`, COOP, CORP and a
//! `Cache-Control` per kind of file.

use std::convert::Infallible;

use http_body_util::Full;
use hyper::body::{Bytes, Incoming};
use hyper::header::{self, HeaderValue};
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;

use crate::tasks::TaskOwner;
use crate::transport::ConsoleListener;

/// The production Content Security Policy (ADR 0031): the Console's own
/// scripts, styles, images, fonts and connections only; no inline code and
/// no `eval`, since no source allows them; every string sent to a script
/// sink refused (`require-trusted-types-for 'script'`) and no Trusted Types
/// policy allowed; no frame may embed it; violations reported to its own
/// address.
pub const CONTENT_SECURITY_POLICY: &str = "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self'; font-src 'self'; connect-src 'self'; manifest-src 'self'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'; require-trusted-types-for 'script'; trusted-types 'none'; report-uri /csp-report";

/// Where browsers send the policy's violation reports.
const REPORTS: &str = "/csp-report";

/// A file whose name holds its content's hash: it never changes.
const IMMUTABLE: &str = "public, max-age=31536000, immutable";
/// A file at a fixed name, such as the page itself: checked on every use.
const REVALIDATED: &str = "no-cache";
/// Any other answer.
const UNSTORED: &str = "no-store";

/// One file of a Console build: its path from the build's root, such as
/// `/assets/index-4f2a.js`, and its bytes, brotli-compressed.
#[derive(Clone, Copy, Debug)]
pub struct ConsoleFile {
    path: &'static str,
    brotli: &'static [u8],
}

impl ConsoleFile {
    /// The file at `path`, such as `/index.html`, holding `brotli`.
    pub const fn new(path: &'static str, brotli: &'static [u8]) -> Self {
        Self { path, brotli }
    }
}

/// A Product's Console build, as its executable embeds it: brotli files
/// only, `xtask console build`'s output (ADR 0057).
#[derive(Debug)]
pub struct ConsoleBuild {
    files: &'static [ConsoleFile],
}

impl ConsoleBuild {
    /// The build made of `files`.
    pub const fn new(files: &'static [ConsoleFile]) -> Self {
        Self { files }
    }

    /// The file a request's path names: `/` names the page itself.
    fn file(&self, path: &str) -> Option<&ConsoleFile> {
        let path = if path == "/" { "/index.html" } else { path };
        self.files.iter().find(|file| file.path == path)
    }
}

/// Serves `build` on `listener` until `owner` stops, each connection a task
/// of its own.
///
/// # Cancel safety
///
/// Dropping the future stops accepting; the connections are `owner`'s,
/// which its stop ends.
pub(crate) async fn serve(
    build: &'static ConsoleBuild,
    listener: ConsoleListener,
    owner: TaskOwner,
) {
    loop {
        let accepted = tokio::select! {
            () = owner.token().cancelled() => return,
            accepted = listener.accept() => accepted,
        };
        match accepted {
            Ok(stream) => {
                let stopping = owner.token().clone();
                owner.spawn(async move {
                    let connection = hyper::server::conn::http1::Builder::new().serve_connection(
                        TokioIo::new(stream),
                        hyper::service::service_fn(move |request: Request<Incoming>| async move {
                            Ok::<_, Infallible>(answer(
                                build,
                                request.method(),
                                request.uri().path(),
                            ))
                        }),
                    );
                    tokio::pin!(connection);
                    tokio::select! {
                        ended = connection.as_mut() => {
                            if let Err(error) = ended {
                                tracing::debug!(%error, "a Console connection ended with an error");
                            }
                        }
                        () = stopping.cancelled() => connection.as_mut().graceful_shutdown(),
                    }
                });
            }
            Err(error) => {
                tracing::warn!(%error, "the Console's listener cannot accept a connection");
            }
        }
    }
}

/// What the Console answers `method` on `path`: a file of `build` as it is
/// stored, the reports endpoint's acknowledgement, or an error, each under
/// the production headers.
fn answer(build: &ConsoleBuild, method: &Method, path: &str) -> Response<Full<Bytes>> {
    if path == REPORTS {
        return if method == Method::POST {
            tracing::debug!("the Console received a Content Security Policy report");
            plain(StatusCode::NO_CONTENT)
        } else {
            refused_method(&["POST"])
        };
    }
    let Some(file) = build.file(path) else {
        return plain(StatusCode::NOT_FOUND);
    };
    if method != Method::GET && method != Method::HEAD {
        return refused_method(&["GET", "HEAD"]);
    }
    let body = if method == Method::HEAD {
        Bytes::new()
    } else {
        Bytes::from_static(file.brotli)
    };
    let mut response = Response::new(Full::new(body));
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(content_type(file.path)),
    );
    headers.insert(header::CONTENT_ENCODING, HeaderValue::from_static("br"));
    let cache = if file.path.starts_with("/assets/") {
        IMMUTABLE
    } else {
        REVALIDATED
    };
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    secured(response)
}

/// An answer with no body.
fn plain(status: StatusCode) -> Response<Full<Bytes>> {
    let mut response = Response::new(Full::new(Bytes::new()));
    *response.status_mut() = status;
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static(UNSTORED));
    secured(response)
}

/// A method the path does not take, naming those it does.
fn refused_method(allowed: &[&'static str]) -> Response<Full<Bytes>> {
    let mut response = plain(StatusCode::METHOD_NOT_ALLOWED);
    if let Ok(allow) = HeaderValue::from_str(&allowed.join(", ")) {
        response.headers_mut().insert(header::ALLOW, allow);
    }
    response
}

/// `response` with the headers every answer of the Console carries.
fn secured(mut response: Response<Full<Bytes>>) -> Response<Full<Bytes>> {
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(CONTENT_SECURITY_POLICY),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(
        "cross-origin-opener-policy",
        HeaderValue::from_static("same-origin"),
    );
    headers.insert(
        "cross-origin-resource-policy",
        HeaderValue::from_static("same-origin"),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response
}

/// The media type of a build's file, by its extension.
fn content_type(path: &str) -> &'static str {
    match path.rsplit_once('.').map(|(_, extension)| extension) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("woff2") => "font/woff2",
        Some("txt") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}
