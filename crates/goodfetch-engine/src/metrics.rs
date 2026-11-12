//! The runtime's metrics (ADR 0050), instrumented once with OpenTelemetry's
//! API on the meter the executable gives the engine, whose SDK exports them
//! through Prometheus, OTLP or both; with none given, a no-op meter, so an
//! SDK crate's linked engine records nothing. Each metric's name, unit and
//! description are `goodfetch-protocol`'s, and each label takes one of the
//! values the protocol declares for it, never a URL or a Run ID. A crate
//! that measures what only it runs, such as Foxhound's crawl, makes its own
//! instruments on the same meter, which each Run hands out.

use std::sync::Arc;

use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_protocol::connection::{PanicEnded, Surface};
use goodfetch_protocol::governor::{
    KIND_LABEL, PRESSURE_SOURCE, PressureSource, SATURATION, SOURCE_LABEL, SystemInfo, TARGET,
    WorkKind,
};
use goodfetch_protocol::ops::{
    BROWSER_LABEL, BROWSER_MEMORY, BROWSER_RECYCLED, BROWSER_RUNNING, CLIENT_CONNECTED,
    COMPLETENESS_LABEL, COUNTED_WORK, COUNTED_WORK_PATH_LABEL, COUNTED_WORK_WORK_LABEL,
    DAEMON_DRAINING, ENGINE_PANICS, LIFECYCLE_LABEL, PANIC_ENDED_LABEL, RECYCLE_REASON_LABEL,
    RUN_ACTIVE, RUN_ENDED, RUN_KIND_LABEL, SURFACE_LABEL, VERDICT_LABEL,
};
use goodfetch_protocol::run::{RunKind, RunReport};
use goodfetch_protocol::storage::{
    AVAILABLE, HOLDS_LABEL, INODES_AVAILABLE, LEVEL, SampledFolder, StorageLevel,
};
use opentelemetry::KeyValue;
use opentelemetry::metrics::{
    AsyncInstrument, Counter, Gauge, Meter, MeterProvider as _, NoopMeterProvider, UpDownCounter,
};
use tokio::sync::watch;

/// The name of the engine's meter, its instrumentation scope.
pub const METER: &str = "goodfetch";

/// The runtime's instruments, shared by every clone.
#[derive(Clone, Debug)]
pub struct Metrics(Arc<Instruments>);

#[derive(Debug)]
struct Instruments {
    meter: Meter,
    browser_running: UpDownCounter<i64>,
    browser_recycled: Counter<u64>,
    browser_memory: Gauge<u64>,
    run_active: UpDownCounter<i64>,
    run_ended: Counter<u64>,
    client_connected: UpDownCounter<i64>,
    daemon_draining: Gauge<u64>,
    engine_panics: Counter<u64>,
    governor_saturation: Gauge<f64>,
    governor_target: Gauge<u64>,
    governor_pressure_source: Gauge<u64>,
    /// The storage series of the last sample, which the storage gauges
    /// read whole at each collection.
    storage: watch::Sender<Arc<[FolderStorage]>>,
    /// The counts `goodfetch.counted_work` reads whole at each collection,
    /// which each primitive adds to (ADR 0140).
    counted_work: CountedWork,
}

/// What a storage sample found of the filesystem holding one folder
/// (ADR 0122).
#[derive(Clone, Copy, Debug)]
pub(crate) struct FolderStorage {
    /// What the folder holds.
    pub(crate) holds: SampledFolder,
    /// Its filesystem's level.
    pub(crate) level: StorageLevel,
    /// The space left on it, in bytes.
    pub(crate) free: u64,
    /// Its inodes left, where it reports them.
    pub(crate) free_inodes: Option<u64>,
}

/// A storage gauge of `meter` named as `metric` says, each collection
/// observing what `value` reads of every folder of the last sample.
fn storage_gauge(
    meter: &Meter,
    metric: &goodfetch_protocol::catalog::Metric,
    storage: watch::Receiver<Arc<[FolderStorage]>>,
    value: fn(&FolderStorage) -> Option<u64>,
) {
    meter
        .u64_observable_gauge(metric.name)
        .with_description(metric.description)
        .with_unit(metric.unit)
        .with_callback(move |observer: &dyn AsyncInstrument<u64>| {
            for folder in storage.borrow().iter() {
                if let Some(value) = value(folder) {
                    observer.observe(
                        value,
                        &[KeyValue::new(HOLDS_LABEL.name, folder.holds.as_str())],
                    );
                }
            }
        })
        .build();
}

impl Default for Metrics {
    fn default() -> Self {
        Self::none()
    }
}

impl Metrics {
    /// The instruments of `meter`.
    pub fn new(meter: &Meter) -> Self {
        let (storage, read) = watch::channel(Arc::from([]));
        storage_gauge(meter, &AVAILABLE, read.clone(), |folder| Some(folder.free));
        storage_gauge(meter, &INODES_AVAILABLE, read.clone(), |folder| {
            folder.free_inodes
        });
        storage_gauge(meter, &LEVEL, read, |folder| Some(folder.level.gauge()));
        let counted_work = CountedWork::default();
        let counted = counted_work.clone();
        meter
            .u64_observable_counter(COUNTED_WORK.name)
            .with_description(COUNTED_WORK.description)
            .with_unit(COUNTED_WORK.unit)
            .with_callback(move |observer: &dyn AsyncInstrument<u64>| {
                for row in Row::ALL {
                    observer.observe(
                        counted.read(row),
                        &[
                            KeyValue::new(COUNTED_WORK_PATH_LABEL.name, row.path()),
                            KeyValue::new(COUNTED_WORK_WORK_LABEL.name, row.work()),
                        ],
                    );
                }
            })
            .build();
        Self(Arc::new(Instruments {
            meter: meter.clone(),
            browser_running: meter
                .i64_up_down_counter(BROWSER_RUNNING.name)
                .with_description(BROWSER_RUNNING.description)
                .with_unit(BROWSER_RUNNING.unit)
                .build(),
            browser_recycled: meter
                .u64_counter(BROWSER_RECYCLED.name)
                .with_description(BROWSER_RECYCLED.description)
                .with_unit(BROWSER_RECYCLED.unit)
                .build(),
            browser_memory: meter
                .u64_gauge(BROWSER_MEMORY.name)
                .with_description(BROWSER_MEMORY.description)
                .with_unit(BROWSER_MEMORY.unit)
                .build(),
            run_active: meter
                .i64_up_down_counter(RUN_ACTIVE.name)
                .with_description(RUN_ACTIVE.description)
                .with_unit(RUN_ACTIVE.unit)
                .build(),
            engine_panics: meter
                .u64_counter(ENGINE_PANICS.name)
                .with_description(ENGINE_PANICS.description)
                .with_unit(ENGINE_PANICS.unit)
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
            governor_saturation: meter
                .f64_gauge(SATURATION.name)
                .with_description(SATURATION.description)
                .with_unit(SATURATION.unit)
                .build(),
            governor_target: meter
                .u64_gauge(TARGET.name)
                .with_description(TARGET.description)
                .with_unit(TARGET.unit)
                .build(),
            governor_pressure_source: meter
                .u64_gauge(PRESSURE_SOURCE.name)
                .with_description(PRESSURE_SOURCE.description)
                .with_unit(PRESSURE_SOURCE.unit)
                .build(),
            storage,
            counted_work,
        }))
    }

    /// Instruments that record nothing.
    pub fn none() -> Self {
        Self::new(&NoopMeterProvider::new().meter(METER))
    }

    /// The Counted work of everything that records here (ADR 0140), which
    /// each primitive counts on.
    pub fn counted_work(&self) -> &CountedWork {
        &self.0.counted_work
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
            vec![KeyValue::new(BROWSER_LABEL.name, browser)],
        )
    }

    /// A browser of a Run's pool was recycled for `reason`, a value of
    /// `goodfetch.browser.recycled`'s `reason` label.
    pub fn browser_recycled(&self, reason: &'static str) {
        self.0
            .browser_recycled
            .add(1, &[KeyValue::new(RECYCLE_REASON_LABEL.name, reason)]);
    }

    /// A level of `metric`, an up-down counter with no label, that a crate
    /// sets as what it counts changes, such as a queue's depth.
    pub fn level(&self, metric: &goodfetch_protocol::catalog::Metric) -> Level {
        Level {
            counter: self
                .0
                .meter
                .i64_up_down_counter(metric.name)
                .with_description(metric.description)
                .with_unit(metric.unit)
                .build(),
            counted: 0,
        }
    }

    /// A Run of `kind` became live, started or resumed.
    pub(crate) fn run_live(&self, kind: RunKind) {
        self.0
            .run_active
            .add(1, &[KeyValue::new(RUN_KIND_LABEL.name, kind.as_str())]);
    }

    /// A live Run of `kind` ended as `report` says, or with no report when
    /// its end could not be recorded (ADR 0122): either way it is no
    /// longer live, and only a report counts an end by its three axes.
    pub(crate) fn run_ended(&self, kind: RunKind, report: Option<&RunReport>) {
        self.0
            .run_active
            .add(-1, &[KeyValue::new(RUN_KIND_LABEL.name, kind.as_str())]);
        let Some(report) = report else {
            return;
        };
        self.0.run_ended.add(
            1,
            &[
                KeyValue::new(RUN_KIND_LABEL.name, kind.as_str()),
                KeyValue::new(LIFECYCLE_LABEL.name, report.lifecycle.as_str()),
                KeyValue::new(VERDICT_LABEL.name, report.verdict.as_str()),
                KeyValue::new(COMPLETENESS_LABEL.name, report.completeness.as_str()),
            ],
        );
    }

    /// A panic in the engine ended `ended` (ADR 0148).
    pub(crate) fn panicked(&self, ended: PanicEnded) {
        self.0
            .engine_panics
            .add(1, &[KeyValue::new(PANIC_ENDED_LABEL.name, ended.as_str())]);
    }

    /// A Client of `surface` connected, when `by` is 1, or left, when -1.
    pub(crate) fn clients(&self, surface: Surface, by: i64) {
        self.0
            .client_connected
            .add(by, &[KeyValue::new(SURFACE_LABEL.name, surface.as_str())]);
    }

    /// Whether the Daemon drains.
    pub(crate) fn draining(&self, draining: bool) {
        self.0.daemon_draining.record(u64::from(draining), &[]);
    }

    /// The Resource governor's sample `info`: each kind's target, what
    /// holds it above normal, its saturation, the highest share of a
    /// kind's target in use, and what its browsers use.
    pub(crate) fn governor(&self, info: &SystemInfo) {
        self.0.browser_memory.record(info.memory.browsers, &[]);
        let mut saturation: f64 = 0.0;
        for kind in WorkKind::ALL {
            let concurrency = info.concurrency.of(kind);
            self.0.governor_target.record(
                u64::from(concurrency.target),
                &[KeyValue::new(KIND_LABEL.name, kind.as_str())],
            );
            if concurrency.target > 0 {
                saturation =
                    saturation.max(f64::from(concurrency.in_use) / f64::from(concurrency.target));
            }
        }
        self.0.governor_saturation.record(saturation.min(1.0), &[]);
        for source in PressureSource::ALL {
            self.0.governor_pressure_source.record(
                u64::from(info.pressure_source == Some(source)),
                &[KeyValue::new(SOURCE_LABEL.name, source.as_str())],
            );
        }
    }

    /// What the Resource governor's last storage sample found of the
    /// filesystems holding `folders` (ADR 0122), in place of the previous
    /// sample's: a collection reads one sample's series whole, never some
    /// folders of it without the others.
    pub(crate) fn storage(&self, folders: impl IntoIterator<Item = FolderStorage>) {
        self.0.storage.send_replace(folders.into_iter().collect());
    }
}

/// How many of something one owner holds now, on an up-down counter that
/// other owners add to as well: none from its making, and none again once
/// it drops.
#[derive(Debug)]
pub struct Level {
    counter: UpDownCounter<i64>,
    counted: i64,
}

impl Level {
    /// Counts `now` in place of what it counted.
    pub fn set(&mut self, now: usize) {
        let now = i64::try_from(now).unwrap_or(i64::MAX);
        self.counter.add(now - self.counted, &[]);
        self.counted = now;
    }
}

impl Drop for Level {
    fn drop(&mut self) {
        self.counter.add(-self.counted, &[]);
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
