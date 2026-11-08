//! The Audit log's OpenTelemetry events (ADRs 0016 and 0043): once the
//! machine's settings, read when the process starts, turn `audit.otlp` on
//! and `otlp.logs_endpoint` names a collector, each entry the process's
//! engine appends is also sent there as the event `goodfetch.audit`, its
//! exported JSONL line as the body and its sequence, kind and hash as
//! attributes, through a batch processor on its own thread over the
//! engine's own HTTP client ([`super::otlp`]); never otherwise, so nothing
//! leaves the machine unless configured.

use std::time::SystemTime;

use goodfetch_engine::audit::AuditEvents;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::settings::Resolved;
use goodfetch_protocol::audit::{AUDIT_OTLP, ExportedAuditEntry};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::log::OTLP_LOGS_ENDPOINT;
use opentelemetry::logs::{AnyValue, LogRecord as _, Logger as _, LoggerProvider as _, Severity};
use opentelemetry_otlp::{WithExportConfig as _, WithHttpConfig as _};
use opentelemetry_sdk::logs::SdkLoggerProvider;

use super::otlp::{EXPORT_TIMEOUT, ToCollector};

/// The event each entry is sent as.
const EVENT_NAME: &str = "goodfetch.audit";

/// The events' export, which sends the events still waiting when it is
/// dropped.
pub(super) struct Export {
    provider: SdkLoggerProvider,
    events: AuditEvents,
}

impl Export {
    /// What the engine sends each new entry to.
    pub(super) fn events(&self) -> AuditEvents {
        self.events.clone()
    }
}

impl Drop for Export {
    fn drop(&mut self) {
        let _ = self.provider.shutdown();
    }
}

/// The events' export to the collector `settings` configure, when they
/// turn `audit.otlp` on and name `otlp.logs_endpoint`; none otherwise, or
/// when it cannot start, which it logs.
pub(super) fn export(
    settings: Option<&Resolved>,
    clock: Clock,
    product: Product,
) -> Option<Export> {
    let settings = settings?;
    if !settings.get::<bool>(&AUDIT_OTLP) {
        return None;
    }
    let endpoint = settings.get::<Option<&str>>(&OTLP_LOGS_ENDPOINT)?;
    let exporter = opentelemetry_otlp::LogExporter::builder()
        .with_http()
        .with_http_client(ToCollector::new(settings, clock))
        .with_endpoint(endpoint)
        .with_timeout(EXPORT_TIMEOUT)
        .build()
        .inspect_err(|error| {
            tracing::error!(%error, "the Audit log's OTLP export cannot start, and is off");
        })
        .ok()?;
    let resource = opentelemetry_sdk::Resource::builder_empty()
        .with_service_name(format!("goodfetch-{}", product.as_str()))
        .build();
    let provider = SdkLoggerProvider::builder()
        .with_batch_exporter(exporter)
        .with_resource(resource)
        .build();
    let logger = provider.logger("goodfetch");
    let events = AuditEvents::new(move |exported: &ExportedAuditEntry| {
        let mut record = logger.create_log_record();
        record.set_event_name(EVENT_NAME);
        record.set_severity_number(Severity::Info);
        if let Ok(time) = exported.entry.time.as_str().parse::<jiff::Timestamp>() {
            record.set_timestamp(SystemTime::from(time));
        }
        if let Ok(line) = serde_json::to_string(exported) {
            record.set_body(AnyValue::from(line));
        }
        record.add_attribute(
            "goodfetch.audit.sequence",
            i64::try_from(exported.entry.sequence).unwrap_or(i64::MAX),
        );
        record.add_attribute("goodfetch.audit.kind", exported.entry.kind.clone());
        record.add_attribute("goodfetch.audit.hash", exported.hash.clone());
        logger.emit(record);
    });
    Some(Export { provider, events })
}
