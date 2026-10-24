//! The foreground Daemon's metrics export (ADR 0050): the OpenTelemetry
//! SDK's meter provider, whose meter the engine records the runtime's
//! metrics on, read by the Prometheus exporter its ops listener's
//! `/metrics` serves when `ops.address` opens one, and by a periodic reader
//! that exports them over OTLP/HTTP in protobuf when `otlp.metrics_endpoint`
//! names a collector, every 60 seconds and once more as the Daemon exits.
//! Each is off until configured, and nothing leaves the machine otherwise
//! (ADR 0043). The Prometheus text carries the metrics alone: no
//! `target_info` and no scope labels, so it reveals nothing of the host.

use std::time::Duration;

use goodfetch_engine::metrics::{METER, Metrics};
use goodfetch_engine::ops::Scrape;
use goodfetch_protocol::catalog::Product;
use opentelemetry::metrics::MeterProvider as _;
use opentelemetry_otlp::{WithExportConfig as _, WithHttpConfig as _};
use opentelemetry_sdk::metrics::{PeriodicReader, SdkMeterProvider};
use prometheus::{Encoder as _, TextEncoder};

use super::otlp::{EXPORT_TIMEOUT, ToCollector};

/// How often the metrics are exported to the collector: OpenTelemetry's
/// default.
const EXPORT_INTERVAL: Duration = Duration::from_secs(60);

/// The metrics' export, which exports them once more when it drops.
pub(super) struct Export {
    provider: Option<SdkMeterProvider>,
    registry: Option<prometheus::Registry>,
}

impl Drop for Export {
    fn drop(&mut self) {
        if let Some(provider) = self.provider.take()
            && let Err(error) = provider.shutdown()
        {
            tracing::warn!(%error, "the metrics' last export failed");
        }
    }
}

impl Export {
    /// The export to Prometheus when `prometheus`, and to the OTLP/HTTP
    /// collector at `otlp` through its client when one is named; none at
    /// all otherwise.
    ///
    /// # Errors
    ///
    /// When an exporter cannot be built, such as for an endpoint that is no
    /// URL.
    pub(super) fn start(
        prometheus: bool,
        otlp: Option<(&str, ToCollector)>,
        product: Product,
    ) -> Result<Self, String> {
        if !prometheus && otlp.is_none() {
            return Ok(Self {
                provider: None,
                registry: None,
            });
        }
        let mut builder = SdkMeterProvider::builder().with_resource(
            opentelemetry_sdk::Resource::builder_empty()
                .with_service_name(format!("goodfetch-{}", product.as_str()))
                .build(),
        );
        let registry = if prometheus {
            let registry = prometheus::Registry::new();
            let exporter = opentelemetry_prometheus::exporter()
                .with_registry(registry.clone())
                .without_target_info()
                .scope_info_enabled(false)
                .build()
                .map_err(|error| error.to_string())?;
            builder = builder.with_reader(exporter);
            Some(registry)
        } else {
            None
        };
        if let Some((endpoint, client)) = otlp {
            let exporter = opentelemetry_otlp::MetricExporter::builder()
                .with_http()
                .with_http_client(client)
                .with_endpoint(endpoint)
                .with_timeout(EXPORT_TIMEOUT)
                .build()
                .map_err(|error| error.to_string())?;
            builder = builder.with_reader(
                PeriodicReader::builder(exporter)
                    .with_interval(EXPORT_INTERVAL)
                    .build(),
            );
        }
        Ok(Self {
            provider: Some(builder.build()),
            registry,
        })
    }

    /// The instruments the engine records on.
    pub(super) fn metrics(&self) -> Metrics {
        self.provider
            .as_ref()
            .map_or_else(Metrics::none, |provider| {
                Metrics::new(&provider.meter(METER))
            })
    }

    /// What the ops listener's `/metrics` serves, when it exports to
    /// Prometheus.
    pub(super) fn scrape(&self) -> Option<PrometheusText> {
        self.registry.clone().map(PrometheusText)
    }
}

/// Prometheus' text of every metric the registry holds.
pub(super) struct PrometheusText(prometheus::Registry);

impl Scrape for PrometheusText {
    fn text(&self) -> String {
        let mut text = Vec::new();
        if let Err(error) = TextEncoder::new().encode(&self.0.gather(), &mut text) {
            tracing::warn!(%error, "the metrics cannot be written as Prometheus' text");
        }
        String::from_utf8(text).unwrap_or_default()
    }
}
