//! The spans a program's subscriber exports while it plays on the linked
//! engine (ADR 0132). The linked engine's threads record their request
//! spans through the process's global subscriber, so the test that reads
//! them sets one, and runs in a process of its own, as `just test` runs
//! every test.

use std::sync::mpsc;

use goodfetch_collie::Collie;
use goodfetch_test_support::{TestEnv, real_time_runtime};
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::trace::{SdkTracerProvider, SpanData, SpanExporter};
use tracing_subscriber::layer::SubscriberExt as _;

use super::options;

/// The spans the OpenTelemetry SDK exports, each sent on as it ends.
#[derive(Debug)]
struct Exported(mpsc::Sender<SpanData>);

impl SpanExporter for Exported {
    fn export(
        &self,
        batch: Vec<SpanData>,
    ) -> impl std::future::Future<Output = OTelSdkResult> + Send {
        for span in batch {
            // The test reads every span once the provider is shut down,
            // so the receiver outlives each export.
            self.0.send(span).unwrap();
        }
        std::future::ready(Ok(()))
    }
}

/// Every span exported while `play` runs on a connection to the linked
/// engine of a new home, the program's and the engine's, read once the
/// connection is closed. It sets the process's global subscriber, so a
/// test calls it once.
pub(super) fn linked_spans(play: impl AsyncFnOnce(&Collie)) -> Vec<SpanData> {
    let (sender, received) = mpsc::channel();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(Exported(sender))
        .build();
    tracing::subscriber::set_global_default(
        tracing_subscriber::registry()
            .with(tracing_opentelemetry::layer().with_tracer(provider.tracer("program"))),
    )
    .unwrap();
    let env = TestEnv::new();

    real_time_runtime().block_on(async {
        let collie = Collie::connect(options(&env)).await.unwrap();
        play(&collie).await;
        collie.close().await.unwrap();
    });
    provider.shutdown().unwrap();
    received.try_iter().collect()
}

/// The one exported span named `name`.
pub(super) fn the_span<'a>(spans: &'a [SpanData], name: &str) -> &'a SpanData {
    let found: Vec<&SpanData> = spans.iter().filter(|span| span.name == name).collect();
    assert_eq!(found.len(), 1, "{name}: {found:#?}");
    found[0]
}
