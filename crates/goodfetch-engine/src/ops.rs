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
//! does, is refused before anything else; each refusal is logged and
//! recorded in the Audit log with its reason, by the endpoint rule.

use std::convert::Infallible;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use goodfetch_protocol::audit::{Listener, RefusalReason};
use goodfetch_protocol::daemon::{DaemonHealth, Unready};
use http_body_util::Full;
use hyper::body::{Bytes, Incoming};
use hyper::header::{self, HeaderMap, HeaderValue};
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::{TokioIo, TokioTimer};
use tokio::sync::Semaphore;

use crate::endpoint::{Refusals, names_an_address, same_token};
use crate::engine_context::EngineContext;
use crate::runs::{Runs, critical_among};
use crate::tasks::TaskOwner;
use crate::transport::OpsListener;

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
        let (folder, essential, governor) = (
            registry.folder.clone(),
            registry.essential.clone(),
            registry.governor.clone(),
        );
        // The filesystems of the essential writes are read afresh, as a
        // start reads them, so readiness and the start's refusal agree
        // (ADRs 0050 and 0122).
        let (writable, installed, storage_critical) = self
            .blocking
            .blocking(move || {
                (
                    writable(&folder),
                    browsers.is_none_or(|browsers| browsers.installed()),
                    critical_among(&governor, essential).is_some(),
                )
            })
            .await;
        let unready: Vec<Unready> = [
            (registry.draining, Unready::Draining),
            (!writable, Unready::RunStateNotWritable),
            (!installed, Unready::BrowsersMissing),
            (registry.governor_last_resort, Unready::GovernorLastResort),
            (storage_critical, Unready::StorageCritical),
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

/// How long a connection has to send a request head, the next one's on a
/// kept-alive connection included, before the listener closes it.
const HEADER_READ_TIMEOUT: Duration = Duration::from_secs(5);

/// How many ops connections may be open at once: a kubelet and a scraper
/// need a few, and past it a new one is closed as it is accepted, so no
/// peer that reaches `ops.address` holds the Daemon's descriptors.
const OPEN_AT_MOST: usize = 64;

/// How long the listener waits before accepting again after a failure that
/// is not one connection's own, such as running out of descriptors
/// (`EMFILE`, `ENFILE`), which would fail again at once; as axum's server
/// does.
const ACCEPT_PAUSE: Duration = Duration::from_secs(1);

/// Serves the ops listener of `options` until `owner` stops, each
/// connection a task of its own: at most [`OPEN_AT_MOST`] at once, each
/// closed when it sends no request head within [`HEADER_READ_TIMEOUT`].
///
/// # Cancel safety
///
/// Dropping the future stops accepting; the connections are `owner`'s,
/// which its stop ends.
pub(crate) async fn serve(
    options: OpsOptions,
    health: Health,
    owner: TaskOwner,
    engine: EngineContext,
) {
    let clock = engine.clock();
    let OpsOptions {
        listener,
        token,
        scrape,
    } = options;
    let serving = Arc::new(Serving {
        token,
        scrape,
        refusals: engine.refusals(),
        health,
    });
    let open = Arc::new(Semaphore::new(OPEN_AT_MOST));
    loop {
        let accepted = tokio::select! {
            () = owner.token().cancelled() => return,
            accepted = listener.accept() => accepted,
        };
        match accepted {
            Ok(stream) => {
                let Ok(permit) = Arc::clone(&open).try_acquire_owned() else {
                    tracing::debug!("an ops connection past the ones open at once is closed");
                    drop(stream);
                    continue;
                };
                let stopping = owner.token().clone();
                let serving = Arc::clone(&serving);
                owner.spawn(async move {
                    let _permit = permit;
                    let connection = hyper::server::conn::http1::Builder::new()
                        .timer(TokioTimer::new())
                        .header_read_timeout(HEADER_READ_TIMEOUT)
                        .serve_connection(
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
                if !connections_own(&error) {
                    tokio::select! {
                        () = owner.token().cancelled() => return,
                        () = clock.sleep(ACCEPT_PAUSE) => {}
                    }
                }
            }
        }
    }
}

/// Whether a failure to accept is the connection's own, such as a peer that
/// reset it before it was accepted, after which the next accept may
/// succeed at once.
fn connections_own(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::ConnectionRefused
    )
}

/// What the ops listener answers with.
struct Serving {
    token: String,
    scrape: Arc<dyn Scrape>,
    refusals: Refusals,
    health: Health,
}

impl Serving {
    /// The answer to `request`.
    async fn answer(&self, request: &Request<Incoming>) -> Response<Full<Bytes>> {
        let headers = request.headers();
        let host = headers
            .get(header::HOST)
            .and_then(|host| host.to_str().ok());
        if !names_an_address(host) {
            return self
                .refused(StatusCode::FORBIDDEN, RefusalReason::HostForeign)
                .await;
        }
        if headers.contains_key(header::ORIGIN) {
            return self
                .refused(StatusCode::FORBIDDEN, RefusalReason::OriginForeign)
                .await;
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
                None => {
                    self.refused(StatusCode::UNAUTHORIZED, RefusalReason::TokenMissing)
                        .await
                }
                Some(given) if !same_token(given, &self.token) => {
                    self.refused(StatusCode::UNAUTHORIZED, RefusalReason::TokenWrong)
                        .await
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

/// The bearer token of the request's `Authorization`, if any.
fn bearer(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = value.split_once(' ')?;
    scheme.eq_ignore_ascii_case("bearer").then(|| token.trim())
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

impl Serving {
    /// The refusal `status` for `reason`, logged and recorded.
    async fn refused(&self, status: StatusCode, reason: RefusalReason) -> Response<Full<Bytes>> {
        self.refusals.refused(Listener::OpsListener, reason).await;
        let mut answer = empty(status);
        if status == StatusCode::UNAUTHORIZED {
            answer
                .headers_mut()
                .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
        }
        answer
    }
}
