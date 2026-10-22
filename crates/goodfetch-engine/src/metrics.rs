//! The runtime's metrics (ADR 0050), instrumented once with OpenTelemetry's
//! API on the meter the executable gives the engine, whose SDK exports them
//! through Prometheus, OTLP or both; with none given, a no-op meter, so an
//! SDK crate's linked engine records nothing. Each metric's name, unit and
//! description are `goodfetch-protocol`'s, and each label takes one of the
//! values the protocol declares for it, never a URL or a Run ID. A crate
//! that measures what only it runs, such as Foxhound's crawl, makes its own
//! instruments on the same meter, which each Run hands out.

use std::sync::Arc;

use goodfetch_protocol::connection::Surface;
use goodfetch_protocol::ops::{
    BROWSER_RUNNING, CLIENT_CONNECTED, DAEMON_DRAINING, RUN_ACTIVE, RUN_ENDED,
};
use goodfetch_protocol::run::{RunKind, RunReport};
use opentelemetry::KeyValue;
use opentelemetry::metrics::{
    Counter, Gauge, Meter, MeterProvider as _, NoopMeterProvider, UpDownCounter,
};

/// The name of the engine's meter, its instrumentation scope.
pub const METER: &str = "goodfetch";

/// The runtime's instruments, shared by every clone.
#[derive(Clone, Debug)]
pub struct Metrics(Arc<Instruments>);

#[derive(Debug)]
struct Instruments {
    meter: Meter,
    browser_running: UpDownCounter<i64>,
    run_active: UpDownCounter<i64>,
    run_ended: Counter<u64>,
    client_connected: UpDownCounter<i64>,
    daemon_draining: Gauge<u64>,
}

impl Default for Metrics {
    fn default() -> Self {
        Self::none()
    }
}

impl Metrics {
    /// The instruments of `meter`.
    pub fn new(meter: &Meter) -> Self {
        Self(Arc::new(Instruments {
            meter: meter.clone(),
            browser_running: meter
                .i64_up_down_counter(BROWSER_RUNNING.name)
                .with_description(BROWSER_RUNNING.description)
                .with_unit(BROWSER_RUNNING.unit)
                .build(),
            run_active: meter
                .i64_up_down_counter(RUN_ACTIVE.name)
                .with_description(RUN_ACTIVE.description)
                .with_unit(RUN_ACTIVE.unit)
                .build(),
            run_ended: meter
                .u64_counter(RUN_ENDED.name)
                .with_description(RUN_ENDED.description)
                .with_unit(RUN_ENDED.unit)
                .build(),
            client_connected: meter
                .i64_up_down_counter(CLIENT_CONNECTED.name)
                .with_description(CLIENT_CONNECTED.description)
                .with_unit(CLIENT_CONNECTED.unit)
                .build(),
            daemon_draining: meter
                .u64_gauge(DAEMON_DRAINING.name)
                .with_description(DAEMON_DRAINING.description)
                .with_unit(DAEMON_DRAINING.unit)
                .build(),
        }))
    }

    /// Instruments that record nothing.
    pub fn none() -> Self {
        Self::new(&NoopMeterProvider::new().meter(METER))
    }

    /// The meter they are made on, on which a crate makes the instruments
    /// of what only it runs.
    pub fn meter(&self) -> &Meter {
        &self.0.meter
    }

    /// A browser of `browser`, a value of `goodfetch.browser.running`'s
    /// `browser` label, runs until the answer drops.
    #[must_use]
    pub fn browser_running(&self, browser: &'static str) -> Counted {
        Counted::new(
            self.0.browser_running.clone(),
            vec![KeyValue::new("browser", browser)],
        )
    }

    /// A Run of `kind` became live, started or resumed.
    pub(crate) fn run_live(&self, kind: RunKind) {
        self.0
            .run_active
            .add(1, &[KeyValue::new("run_kind", kind.as_str())]);
    }

    /// A live Run of `kind` ended as `report` says.
    pub(crate) fn run_ended(&self, kind: RunKind, report: &RunReport) {
        self.0
            .run_active
            .add(-1, &[KeyValue::new("run_kind", kind.as_str())]);
        self.0.run_ended.add(
            1,
            &[
                KeyValue::new("run_kind", kind.as_str()),
                KeyValue::new("lifecycle", report.lifecycle.as_str()),
                KeyValue::new("verdict", report.verdict.as_str()),
                KeyValue::new("completeness", report.completeness.as_str()),
            ],
        );
    }

    /// A Client of `surface` connected, when `by` is 1, or left, when -1.
    pub(crate) fn clients(&self, surface: Surface, by: i64) {
        self.0
            .client_connected
            .add(by, &[KeyValue::new("surface", surface.as_str())]);
    }

    /// Whether the Daemon drains.
    pub(crate) fn draining(&self, draining: bool) {
        self.0.daemon_draining.record(u64::from(draining), &[]);
    }
}

/// One of something an up-down counter counts, from its making until it
/// drops.
#[derive(Debug)]
pub struct Counted {
    counter: UpDownCounter<i64>,
    attributes: Vec<KeyValue>,
}

impl Counted {
    /// Counts one more on `counter` with `attributes`, until the answer
    /// drops.
    pub fn new(counter: UpDownCounter<i64>, attributes: Vec<KeyValue>) -> Self {
        counter.add(1, &attributes);
        Self {
            counter,
            attributes,
        }
    }
}

impl Drop for Counted {
    fn drop(&mut self) {
        self.counter.add(-1, &self.attributes);
    }
}
