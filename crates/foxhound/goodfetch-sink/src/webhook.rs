//! Webhook Sink signed under Standard Webhooks (ADRs 0008, 0017 and 0026;
//! issue 1166): delivers committed Record batches (capped at 256 KB by
//! default, or `webhook.max_batch_bytes`) or thin notifications carrying a
//! record range and a fetch URL (`webhook.payload: thin`) over HTTP POST,
//! signed with `goodfetch_engine::webhook_signing::Signer`.
//!
//! Each delivery's body is the Standard Webhooks envelope
//! `{"run_epoch": <epoch>, "data": <payload>}` so `run_epoch` is signed beside
//! the payload; its `webhook-id` is deterministic per batch (the Record's own
//! delivery ID for singletons, or `<run_id>/<sink>/<dataset>/<from>-<through>`
//! for multi-Record batches, sanitized of `.` so signing text is unambiguous)
//! so replayed deliveries carry the exact same delivery ID for receiver
//! deduplication.

use std::collections::BTreeMap;
use std::time::{Duration, UNIX_EPOCH};

use goodfetch_engine::clock::Clock;
use goodfetch_engine::http_client::WholeFailure;
use goodfetch_engine::notification_endpoint::NotificationEndpoint;
use goodfetch_engine::retry_after;
use goodfetch_engine::webhook_signing::{self, Signer};
use goodfetch_protocol::foxhound::sink::WebhookPayload;
use hyper::body::Bytes;
use hyper::{Method, Request, Uri};
use serde::Serialize;
use serde_json::Value;
use url::Url;

use crate::contract::{
    Batch, Delivered, Failure, FailureKind, GuaranteeClass, Primitives, Probe, RunCopy, Written,
};
use crate::throughput::{DEFAULT_WRITE_TIMEOUT, write_bounded};

/// Default maximum serialized webhook request body size in bytes (256 KB,
/// ADR 0026).
pub const DEFAULT_MAX_BATCH_BYTES: usize = 256 * 1024;

/// Default base URL prefix for `thin` payload fetch URLs when no external
/// fetch base URL is configured.
const DEFAULT_FETCH_BASE: &str = "goodfetch://runs";

/// Maximum bytes of an HTTP error response body retained in failure text.
const PARTY_TEXT_LIMIT: usize = 1024;

/// Configuration for a [`WebhookSink`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebhookConfig {
    /// Whether each POST carries full Records (`Batch`) or a record range and
    /// fetch URL (`Thin`).
    pub payload: WebhookPayload,
    /// Maximum byte size of a single signed POST body (at least 1).
    pub max_batch_bytes: usize,
    /// Base URL for constructing `fetch_url` in `Thin` payloads.
    pub fetch_base_url: String,
}

impl Default for WebhookConfig {
    fn default() -> Self {
        Self {
            payload: WebhookPayload::Batch,
            max_batch_bytes: DEFAULT_MAX_BATCH_BYTES,
            fetch_base_url: DEFAULT_FETCH_BASE.to_owned(),
        }
    }
}

/// Errors returned by [`WebhookSink`] before translation.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum WebhookError {
    /// The webhook destination URL is unusable (missing host or unsupported scheme).
    #[error("a webhook Sink destination must be an http or https URL naming a host")]
    UrlUnusable,
    /// A delivery failure already classified for the driver.
    #[error("{0}")]
    Delivery(Failure),
    /// A single Record's signed payload exceeded `webhook.max_batch_bytes`.
    #[error(
        "a single Record's signed webhook payload exceeded `webhook.max_batch_bytes` ({0} bytes)"
    )]
    RecordTooLarge(usize),
    /// Failed to serialize the delivery payload.
    #[error("failed to serialize webhook payload: {0}")]
    Serialize(String),
    /// No active Tokio runtime available when called synchronously.
    #[error("webhook Sink requires an active Tokio runtime")]
    NoRuntime,
}

#[derive(Serialize)]
struct BatchRecord<'a> {
    delivery_id: &'a str,
    number: u64,
    committed_at: &'a str,
    value: &'a Value,
}

#[derive(Serialize)]
struct BatchPayload<'a> {
    shape: &'static str,
    run_id: &'a str,
    dataset: &'a str,
    from: u64,
    through: u64,
    records: Vec<BatchRecord<'a>>,
}

#[derive(Serialize)]
struct ThinPayload<'a> {
    shape: &'static str,
    run_id: &'a str,
    dataset: &'a str,
    from: u64,
    through: u64,
    count: usize,
    fetch_url: String,
}

/// A built-in Sink delivering signed HTTP POST requests under Standard
/// Webhooks (ADRs 0008, 0017 and 0026).
#[derive(Debug)]
pub struct WebhookSink {
    sink_name: String,
    url: Url,
    uri: Uri,
    signer: Signer,
    endpoint: NotificationEndpoint,
    clock: Clock,
    config: WebhookConfig,
}

impl WebhookSink {
    /// Creates a webhook Sink posting to `url`, signed by `signer`, through
    /// `endpoint`, using `clock` for signature timestamps and write timeouts.
    ///
    /// # Errors
    ///
    /// [`WebhookError::UrlUnusable`] when `url` is not `http`/`https` or has
    /// no host.
    pub fn new(
        sink_name: impl Into<String>,
        url: Url,
        signer: Signer,
        endpoint: NotificationEndpoint,
        clock: Clock,
        mut config: WebhookConfig,
    ) -> Result<Self, WebhookError> {
        if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none_or(str::is_empty) {
            return Err(WebhookError::UrlUnusable);
        }
        let uri = Uri::try_from(url.as_str()).map_err(|_| WebhookError::UrlUnusable)?;
        config.max_batch_bytes = config.max_batch_bytes.max(1);
        Ok(Self {
            sink_name: sink_name.into(),
            url,
            uri,
            signer,
            endpoint,
            clock,
            config,
        })
    }

    /// Configured destination URL.
    #[must_use]
    pub fn url(&self) -> &Url {
        &self.url
    }

    /// Deterministic Standard Webhooks `webhook-id` for `records` of `dataset`
    /// in `run_id`, stable across retries and replays and free of `.` and
    /// control characters so signing text is unambiguous.
    #[must_use]
    pub fn delivery_id(&self, run_id: &str, dataset: &str, records: &[Delivered]) -> String {
        let raw = match (records.first(), records.last()) {
            (Some(only), Some(last)) if only.number == last.number => {
                only.delivery_id.as_str().to_owned()
            }
            (Some(first), Some(last)) => {
                format!(
                    "{run_id}/{}/{dataset}/{}-{}",
                    self.sink_name, first.number, last.number
                )
            }
            _ => format!("{run_id}/{}/{dataset}/empty", self.sink_name),
        };
        sanitize_webhook_id(&raw)
    }

    /// Serializes the Standard Webhooks signed envelope
    /// `{"run_epoch": ..., "data": ...}` for `records`.
    ///
    /// # Errors
    ///
    /// [`WebhookError::Serialize`] when JSON serialization fails.
    pub fn build_body(
        &self,
        run: &RunCopy,
        dataset: &str,
        records: &[Delivered],
    ) -> Result<Vec<u8>, WebhookError> {
        let from = records.first().map_or(0, |r| r.number);
        let through = records.last().map_or(0, |r| r.number);
        match self.config.payload {
            WebhookPayload::Batch => {
                let payload = BatchPayload {
                    shape: "batch",
                    run_id: &run.run_id,
                    dataset,
                    from,
                    through,
                    records: records
                        .iter()
                        .map(|record| BatchRecord {
                            delivery_id: record.delivery_id.as_str(),
                            number: record.number,
                            committed_at: &record.committed_at,
                            value: &record.value,
                        })
                        .collect(),
                };
                webhook_signing::body(run.run_epoch, &payload)
                    .map_err(|error| WebhookError::Serialize(error.to_string()))
            }
            WebhookPayload::Thin => {
                let base = self.config.fetch_base_url.trim_end_matches('/');
                let after = from.saturating_sub(1);
                let fetch_url = format!(
                    "{base}/{}/datasets/{dataset}/records?after={after}&through={through}",
                    run.run_id
                );
                let payload = ThinPayload {
                    shape: "thin",
                    run_id: &run.run_id,
                    dataset,
                    from,
                    through,
                    count: records.len(),
                    fetch_url,
                };
                webhook_signing::body(run.run_epoch, &payload)
                    .map_err(|error| WebhookError::Serialize(error.to_string()))
            }
        }
    }

    /// Builds a signed HTTP POST request for `id` and `body`.
    fn signed_request(&self, id: &str, body: Vec<u8>) -> Request<Bytes> {
        let timestamp = self
            .clock
            .wall()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| {
                i64::try_from(since.as_secs()).unwrap_or(i64::MAX)
            });
        let headers = self
            .signer
            .headers(
                &webhook_signing::Message {
                    id,
                    timestamp,
                    body: &body,
                },
                &[("content-type", "application/json")],
            )
            .expect(
                "invariant: sanitized webhook-id is non-empty ASCII without `.`, CR, LF or NUL",
            );
        headers
            .iter()
            .fold(
                Request::builder()
                    .method(Method::POST)
                    .uri(self.uri.clone()),
                |request, (name, value)| request.header(name, value),
            )
            .body(Bytes::from(body))
            .expect("invariant: URI parsed and Standard Webhooks headers were validated")
    }

    /// Sends one already-sized signed request, bounded by `timeout`.
    async fn post_once(
        &self,
        id: &str,
        body: Vec<u8>,
        timeout: Duration,
    ) -> Result<Written, Failure> {
        let request = self.signed_request(id, body);
        let clock = self.clock;
        write_bounded(clock, timeout, async {
            match self.endpoint.send(request).await {
                Ok(response) => classify_response(&response, clock),
                Err(WholeFailure::TooLong { status }) if status.is_success() => {
                    Ok(Written::Delivered)
                }
                Err(WholeFailure::TooLong { status }) => Err(Failure::from_http_status(
                    status.as_u16(),
                    None,
                    format!("webhook receiver answered HTTP {}", status.as_u16()),
                )),
                Err(failure) => Err(classify_endpoint_failure(&failure)),
            }
        })
        .await
    }

    /// Sends `batch` asynchronously, splitting any sub-batch whose serialized
    /// signed body exceeds `webhook.max_batch_bytes` so no HTTP POST ever
    /// exceeds the cap (ADR 0026).
    ///
    /// # Errors
    ///
    /// Classified [`Failure`] on HTTP error, timeout, network/TLS error, or
    /// when a single Record exceeds `webhook.max_batch_bytes`.
    ///
    /// # Cancel safety
    ///
    /// Dropping the returned future cancels the in-flight HTTP request; any
    /// sub-batch already accepted by the receiver carries its deterministic
    /// `webhook-id` so retries deduplicate cleanly.
    pub async fn send_batch(&self, batch: &Batch, timeout: Duration) -> Result<Written, Failure> {
        if batch.records.is_empty() {
            return Ok(Written::Delivered);
        }
        let mut stack = vec![batch.records.as_slice()];
        while let Some(slice) = stack.pop() {
            let body = self
                .build_body(&batch.run, &batch.dataset, slice)
                .map_err(|error| self.map_failure(error))?;
            if body.len() > self.config.max_batch_bytes {
                if slice.len() == 1 {
                    return Err(Failure::new(
                        FailureKind::RejectedRecords,
                        format!(
                            "Record {} signed webhook body ({} bytes) exceeds webhook.max_batch_bytes ({})",
                            slice[0].number,
                            body.len(),
                            self.config.max_batch_bytes
                        ),
                    ));
                }
                let mid = slice.len() / 2;
                let (left, right) = slice.split_at(mid);
                stack.push(right);
                stack.push(left);
                continue;
            }
            let id = self.delivery_id(&batch.run.run_id, &batch.dataset, slice);
            self.post_once(&id, body, timeout).await?;
        }
        Ok(Written::Delivered)
    }
}

impl Primitives for WebhookSink {
    type Error = WebhookError;

    fn probe(&mut self, _run: &RunCopy) -> Result<Probe, WebhookError> {
        Ok(Probe {
            class: GuaranteeClass::AtLeastOnce,
            delivered: BTreeMap::new(),
        })
    }

    fn write(&mut self, batch: &Batch) -> Result<Written, WebhookError> {
        self.write_with_timeout(batch, DEFAULT_WRITE_TIMEOUT)
    }

    fn write_with_timeout(
        &mut self,
        batch: &Batch,
        timeout: Duration,
    ) -> Result<Written, WebhookError> {
        let handle = tokio::runtime::Handle::try_current().map_err(|_| WebhookError::NoRuntime)?;
        handle
            .block_on(self.send_batch(batch, timeout))
            .map_err(WebhookError::Delivery)
    }

    fn map_failure(&self, error: WebhookError) -> Failure {
        match error {
            WebhookError::Delivery(failure) => failure,
            WebhookError::UrlUnusable => Failure::new(FailureKind::Permanent, error.to_string()),
            WebhookError::RecordTooLarge(_) => {
                Failure::new(FailureKind::RejectedRecords, error.to_string())
            }
            WebhookError::Serialize(_) | WebhookError::NoRuntime => {
                Failure::new(FailureKind::Unrecognized, error.to_string())
            }
        }
    }
}

fn sanitize_webhook_id(raw: &str) -> String {
    let clean: String = raw
        .chars()
        .map(|ch| {
            if ch == '.' || ch.is_control() || !ch.is_ascii() {
                '_'
            } else {
                ch
            }
        })
        .collect();
    if clean.is_empty() {
        "delivery".to_owned()
    } else {
        clean
    }
}

fn classify_response(response: &hyper::Response<Bytes>, clock: Clock) -> Result<Written, Failure> {
    let status = response.status();
    if status.is_success() {
        return Ok(Written::Delivered);
    }
    let retry_after = retry_after::wait_of(response.headers(), clock.wall());
    let body = response.body();
    let preview = body.get(..PARTY_TEXT_LIMIT).unwrap_or(body);
    let detail = format!(
        "webhook receiver answered HTTP {}: {}",
        status.as_u16(),
        String::from_utf8_lossy(preview)
    );
    Err(Failure::from_http_status(
        status.as_u16(),
        retry_after,
        detail,
    ))
}

fn classify_endpoint_failure(failure: &WholeFailure) -> Failure {
    match failure {
        WholeFailure::Refused(_) => Failure::new(FailureKind::Permanent, format!("{failure:?}")),
        WholeFailure::CertificateInvalid(_)
        | WholeFailure::ClientCertificate(_)
        | WholeFailure::Network => Failure::new(FailureKind::Transient, format!("{failure:?}")),
        WholeFailure::TooLong { status } => Failure::from_http_status(
            status.as_u16(),
            None,
            format!("webhook receiver answered HTTP {}", status.as_u16()),
        ),
        _ => Failure::new(FailureKind::Unrecognized, format!("{failure:?}")),
    }
}
