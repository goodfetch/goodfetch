//! The OTLP/HTTP exports' HTTP client (ADRs 0050 and 0132), which both the
//! spans' export and the metrics' share: ureq, blocking, on the export's
//! own thread (the spans' batch processor, the metrics' periodic reader),
//! which waits on each export, each bounded by [`EXPORT_TIMEOUT`].

use std::time::Duration;

use bytes::Bytes;

/// How long one export to the collector may take.
pub(super) const EXPORT_TIMEOUT: Duration = Duration::from_secs(10);

/// The client.
#[derive(Debug)]
pub(super) struct Ureq(ureq::Agent);

impl Ureq {
    /// A client whose every export ends within [`EXPORT_TIMEOUT`], and which
    /// hands the exporter every status the collector answers.
    pub(super) fn new() -> Self {
        Self(
            ureq::Agent::config_builder()
                .http_status_as_error(false)
                .timeout_global(Some(EXPORT_TIMEOUT))
                .build()
                .new_agent(),
        )
    }
}

#[async_trait::async_trait]
impl opentelemetry_http::HttpClient for Ureq {
    async fn send_bytes(
        &self,
        request: http::Request<Bytes>,
    ) -> Result<http::Response<Bytes>, opentelemetry_http::HttpError> {
        let response = self.0.run(request.map(|body| body.to_vec()))?;
        let (parts, mut body) = response.into_parts();
        let body = body.read_to_vec()?;
        Ok(http::Response::from_parts(parts, Bytes::from(body)))
    }
}
