//! The OTLP/HTTP exports' HTTP client (ADRs 0017, 0050 and 0132), which
//! both the spans' export and the metrics' share: the engine's own HTTP
//! client, reaching the collector as the Operator-configured endpoint it
//! is (`goodfetch_engine::otlp`), on the export's own thread (the spans'
//! batch processor, the metrics' periodic reader), which waits on each
//! export, each bounded by [`EXPORT_TIMEOUT`] over a runtime of its own.

use std::time::Duration;

use bytes::Bytes;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::otlp::Collector;
use goodfetch_engine::settings::Resolved;
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};

/// How long one export to the collector may take.
pub(super) const EXPORT_TIMEOUT: Duration = Duration::from_secs(10);

/// The client.
#[derive(Debug)]
pub(super) struct ToCollector {
    collector: Collector,
    clock: Clock,
}

impl ToCollector {
    /// A client to the collector the machine's `settings` configure, whose
    /// every export ends within [`EXPORT_TIMEOUT`], and which hands the
    /// exporter every status the collector answers. Its `tls.trusted_cas`
    /// files are read inline: the export's own thread waits on each export.
    pub(super) fn new(settings: &Resolved, clock: Clock) -> Self {
        Self {
            collector: Collector::of(
                settings,
                clock,
                TaskOwner::root(BlockingPolicy::Inline).blocking_threads(),
            ),
            clock,
        }
    }
}

#[async_trait::async_trait]
impl opentelemetry_http::HttpClient for ToCollector {
    async fn send_bytes(
        &self,
        request: http::Request<Bytes>,
    ) -> Result<http::Response<Bytes>, opentelemetry_http::HttpError> {
        // The export's own thread runs no runtime: each export gets one of
        // its own, as `main`'s other blocking steps do.
        #[expect(
            clippy::disallowed_methods,
            reason = "main builds the runtime (ADR 0060)"
        )]
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        runtime.block_on(async {
            tokio::select! {
                answered = self.collector.send(request) => answered.map_err(Into::into),
                () = self.clock.sleep(EXPORT_TIMEOUT) => Err(format!(
                    "the collector did not answer within {} seconds",
                    EXPORT_TIMEOUT.as_secs()
                )
                .into()),
            }
        })
    }
}
