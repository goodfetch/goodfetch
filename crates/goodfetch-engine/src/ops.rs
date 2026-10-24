//! The Daemon's health and its ops listener (ADR 0050). A Daemon is live
//! while its runtime and its Runs' registry, which writes their run state,
//! respond; it is ready while it is live, its run state is writable, the
//! browsers its configuration uses are installed, it does not drain, its
//! Resource governor is not at its last resort (ADR 0040) and the
//! governor's last storage sample did not find its run state's filesystem
//! at `critical` (ADR 0122). In
//! a Daemon group, readiness ignores the Lead Daemon's reachability.
//! `daemon.health` answers both with the reasons, and the ops listener's
//! `/livez` and `/readyz` with a status alone: 200, or 503, and no body,
//! to anyone. Its `/metrics` answers Prometheus' text of every metric only
//! to a request whose bearer token is the ops token, a token no other
//! listener takes, and never to one carrying it in its query string. A
//! request naming a `Host` other than an IP address or `localhost`, as a
//! DNS-rebinding page's does, or carrying an `Origin`, as a browser's page
//! does, is refused before anything else; each refusal is logged under the
//! listeners' refusal target with its reason.

use std::convert::Infallible;
use std::net::IpAddr;
use std::path::Path;
use std::sync::Arc;

use goodfetch_protocol::daemon::{DaemonHealth, Unready};
use http_body_util::Full;
use hyper::body::{Bytes, Incoming};
use hyper::header::{self, HeaderMap, HeaderValue};
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;

use crate::rpc::REFUSAL_TARGET;
use crate::runs::Runs;
use crate::tasks::TaskOwner;
use crate::transport::OpsListener;

/// The ops listener's name in the endpoint table (ADR 0067).
const LISTENER: &str = "ops_listener";

/// What `/metrics` answers with: Prometheus' text format.
const PROMETHEUS_TEXT: &str = "text/plain; version=0.0.4; charset=utf-8";

/// Whether the browsers a Daemon's configuration uses are installed, such as
/// the pinned build in the Browser cache. Blocking: it runs on the tasks
/// module's blocking policy.
pub trait ConfiguredBrowsers: Send + Sync {
    /// Whether every one of them is installed.
    fn installed(&self) -> bool;
}

/// What `/metrics` serves: every metric of the Daemon's meter, in
/// Prometheus' text format. Blocking, as [`ConfiguredBrowsers`].
pub trait Scrape: Send + Sync {
    /// The text of every metric now.
    fn text(&self) -> String;
}

/// What a Daemon checks its health with, and its ops listener, as its
/// starter set them up; none of either by default.
#[derive(Default)]
pub struct Ops {
    /// Its ops listener, when its starter opened one, as `daemon run` does
    /// at `ops.address`.
    pub listener: Option<OpsOptions>,
    /// Whether the browsers its configuration uses are installed, which its
    /// readiness checks; none when it uses no browser its starter knows of.
    pub browsers: Option<Arc<dyn ConfiguredBrowsers>>,
}

/// A Daemon's ops listener, as its starter set it up.
pub struct OpsOptions {
    /// The listener, bound at `ops.address`.
    pub listener: OpsListener,
    /// The ops token, which `/metrics` takes as a bearer token.
    pub token: String,
    /// What `/metrics` serves.
    pub scrape: Arc<dyn Scrape>,
}

/// What a Daemon's health is checked on.
#[derive(Clone)]
pub(crate) struct Health {
    pub(crate) runs: Runs,
    pub(crate) browsers: Option<Arc<dyn ConfiguredBrowsers>>,
    /// Where the blocking checks run.
    pub(crate) blocking: Arc<TaskOwner>,
}

impl Health {
    /// The Daemon's health now.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses only the answer.
    pub(crate) async fn check(&self) -> DaemonHealth {
        let Some(registry) = self.runs.health().await else {
            return DaemonHealth {
                live: false,
                ready: false,
                unready: Vec::new(),
            };
        };
        let browsers = self.browsers.clone();
        let (writable, installed) = self
            .blocking
            .blocking(move || {
                (
                    writable(&registry.folder),
                    browsers.is_none_or(|browsers| browsers.installed()),
                )
            })
            .await;
        let unready: Vec<Unready> = [
            (registry.draining, Unready::Draining),
            (!writable, Unready::RunStateNotWritable),
            (!installed, Unready::BrowsersMissing),
            (registry.governor_last_resort, Unready::GovernorLastResort),
            (registry.storage_critical, Unready::StorageCritical),
        ]
        .into_iter()
        .filter_map(|(holds, reason)| holds.then_some(reason))
        .collect();
        DaemonHealth {
            live: true,
            ready: unready.is_empty(),
            unready,
        }
    }
}

/// Whether a file can be written in `folder`, the folder of the run
/// folders, which it makes if it is missing, as a Run's start does.
fn writable(folder: &Path) -> bool {
    #[expect(
        clippy::disallowed_methods,
        reason = "it runs on the tasks module's blocking policy"
    )]
    let made = std::fs::create_dir_all(folder);
    made.is_ok() && tempfile::NamedTempFile::new_in(folder).is_ok()
}

/// Serves the ops listener of `options` until `owner` stops, each
/// connection a task of its own.
///
/// # Cancel safety
///
/// Dropping the future stops accepting; the connections are `owner`'s,
/// which its stop ends.
pub(crate) async fn serve(options: OpsOptions, health: Health, owner: TaskOwner) {
    let OpsOptions {
        listener,
        token,
        scrape,
    } = options;
    let serving = Arc::new(Serving {
        token,
        scrape,
        health,
    });
    loop {
        let accepted = tokio::select! {
            () = owner.token().cancelled() => return,
            accepted = listener.accept() => accepted,
        };
        match accepted {
            Ok(stream) => {
                let stopping = owner.token().clone();
                let serving = Arc::clone(&serving);
                owner.spawn(async move {
                    let connection = hyper::server::conn::http1::Builder::new().serve_connection(
                        TokioIo::new(stream),
                        hyper::service::service_fn(move |request: Request<Incoming>| {
                            let serving = Arc::clone(&serving);
                            async move { Ok::<_, Infallible>(serving.answer(&request).await) }
                        }),
                    );
                    tokio::pin!(connection);
                    tokio::select! {
                        ended = connection.as_mut() => {
                            if let Err(error) = ended {
                                tracing::debug!(%error, "an ops connection ended with an error");
                            }
                        }
                        () = stopping.cancelled() => connection.as_mut().graceful_shutdown(),
                    }
                });
            }
            Err(error) => {
                tracing::warn!(%error, "the ops listener cannot accept a connection");
            }
        }
    }
}

/// What the ops listener answers with.
struct Serving {
    token: String,
    scrape: Arc<dyn Scrape>,
    health: Health,
}

impl Serving {
    /// The answer to `request`.
    async fn answer(&self, request: &Request<Incoming>) -> Response<Full<Bytes>> {
        let headers = request.headers();
        if !names_an_address(headers) {
            return refused(StatusCode::FORBIDDEN, "host_foreign");
        }
        if headers.contains_key(header::ORIGIN) {
            return refused(StatusCode::FORBIDDEN, "origin_foreign");
        }
        if request.method() != Method::GET && request.method() != Method::HEAD {
            let mut answer = empty(StatusCode::METHOD_NOT_ALLOWED);
            answer
                .headers_mut()
                .insert(header::ALLOW, HeaderValue::from_static("GET, HEAD"));
            return answer;
        }
        match request.uri().path() {
            "/livez" => {
                let health = self.health.check().await;
                empty(probe_status(health.live))
            }
            "/readyz" => {
                let health = self.health.check().await;
                empty(probe_status(health.ready))
            }
            "/metrics" => match bearer(headers) {
                None => refused(StatusCode::UNAUTHORIZED, "token_missing"),
                Some(given) if !same_token(given, &self.token) => {
                    refused(StatusCode::UNAUTHORIZED, "token_wrong")
                }
                Some(_) => {
                    let scrape = Arc::clone(&self.scrape);
                    let text = self.health.blocking.blocking(move || scrape.text()).await;
                    let mut answer = Response::new(Full::new(Bytes::from(text)));
                    let headers = answer.headers_mut();
                    headers.insert(
                        header::CONTENT_TYPE,
                        HeaderValue::from_static(PROMETHEUS_TEXT),
                    );
                    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
                    answer
                }
            },
            _ => empty(StatusCode::NOT_FOUND),
        }
    }
}

/// A probe's status: 200 when its answer is yes, else 503.
fn probe_status(yes: bool) -> StatusCode {
    if yes {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    }
}

/// Whether the request's `Host` is an IP address or `localhost`, with or
/// without a port: a name a DNS-rebinding page could point here is not.
fn names_an_address(headers: &HeaderMap) -> bool {
    let Some(host) = headers
        .get(header::HOST)
        .and_then(|host| host.to_str().ok())
    else {
        return false;
    };
    let name = match host.strip_prefix('[') {
        Some(bracketed) => bracketed.split(']').next().unwrap_or_default(),
        None => host.rsplit_once(':').map_or(host, |(name, _)| name),
    };
    name.eq_ignore_ascii_case("localhost") || name.parse::<IpAddr>().is_ok()
}

/// The bearer token of the request's `Authorization`, if any.
fn bearer(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = value.split_once(' ')?;
    scheme.eq_ignore_ascii_case("bearer").then(|| token.trim())
}

/// Whether two tokens are equal, in time that depends only on their
/// lengths, so a scraper cannot guess one byte at a time.
fn same_token(given: &str, expected: &str) -> bool {
    let (given, expected) = (given.as_bytes(), expected.as_bytes());
    given.len() == expected.len()
        && given
            .iter()
            .zip(expected)
            .fold(0_u8, |differs, (left, right)| differs | (left ^ right))
            == 0
}

/// An answer with `status` and no body.
fn empty(status: StatusCode) -> Response<Full<Bytes>> {
    let mut answer = Response::new(Full::new(Bytes::new()));
    *answer.status_mut() = status;
    answer
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    answer
}

/// The refusal `status` for `reason`, logged.
fn refused(status: StatusCode, reason: &'static str) -> Response<Full<Bytes>> {
    tracing::warn!(target: REFUSAL_TARGET, listener = LISTENER, reason, "the ops listener refused a request");
    let mut answer = empty(status);
    if status == StatusCode::UNAUTHORIZED {
        answer
            .headers_mut()
            .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
    }
    answer
}
