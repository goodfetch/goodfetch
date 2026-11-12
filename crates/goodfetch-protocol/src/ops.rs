//! The Daemon's ops contracts (ADR 0050): where its ops listener listens,
//! where its metrics are exported over OTLP, and the runtime's metrics,
//! whose names under `goodfetch.*` (rendered `goodfetch_*` for Prometheus)
//! are a versioned contract, each Stable or Experimental, each label taking
//! a bounded set of values. Every Daemon of either Product serves them, so
//! everything here is Collie's.

use crate::catalog::{
    Authority, Entries, Instrument, Metric, MetricLabel, ProtocolVersion, Setting, Stability,
    ValueKind,
};
use crate::connection::{PanicEnded, Surface};
use crate::run::{Completeness, Lifecycle, RunKind, Verdict};
use crate::wire_name;

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

fn unset() -> serde_json::Value {
    serde_json::Value::Null
}

/// `ops.address` (ADR 0050): where the foreground Daemon's ops listener
/// listens, if anywhere.
pub const ADDRESS: Setting = Setting {
    name: "ops.address",
    description: "Where `daemon run`, the foreground Daemon, serves its ops listener, as `host:port`: `/livez` and `/readyz` answer a status and no body to anyone, and `/metrics`, Prometheus' text, only with the ops token, the `ops.token` file of the major's folder, as a bearer token. Unset by default: no ops listener and no Prometheus export. Any address, so a cluster's probes and scrapers reach it; a request whose `Host` is a name other than `localhost`, or that carries an `Origin`, is refused. Set in user configuration or an Operator policy, or for `daemon run` through `GOODFETCH_OPS_ADDRESS`.",
    kind: ValueKind::Text,
    default: unset,
    clearable: true,
    authority: Authority::Machine,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    arrived_in: ARRIVED_IN,
};

/// `otlp.metrics_endpoint` (ADR 0050): where the foreground Daemon exports
/// its metrics over OTLP, if anywhere.
pub const OTLP_METRICS_ENDPOINT: Setting = Setting {
    name: "otlp.metrics_endpoint",
    description: "The OTLP/HTTP endpoint `daemon run`, the foreground Daemon, exports its metrics to, in protobuf, every 60 seconds and once more as it exits: a full URL such as `http://collector:4318/v1/metrics`. Unset by default: nothing is exported (ADR 0043).",
    kind: ValueKind::Text,
    default: unset,
    clearable: true,
    authority: Authority::Machine,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    arrived_in: ARRIVED_IN,
};

/// The `run_kind` label: every [`RunKind`].
pub const RUN_KIND_LABEL: MetricLabel = MetricLabel {
    name: "run_kind",
    values: || wire_name::all(&RunKind::ALL),
};

/// The `lifecycle` label of `goodfetch.run.ended`: every [`Lifecycle`] a
/// Run ends in, all but `running`.
pub const LIFECYCLE_LABEL: MetricLabel = MetricLabel {
    name: "lifecycle",
    values: || {
        Lifecycle::ALL
            .into_iter()
            .filter(|lifecycle| *lifecycle != Lifecycle::Running)
            .map(Lifecycle::as_str)
            .collect()
    },
};

/// The `verdict` label of `goodfetch.run.ended`: every [`Verdict`].
pub const VERDICT_LABEL: MetricLabel = MetricLabel {
    name: "verdict",
    values: || wire_name::all(&Verdict::ALL),
};

/// The `completeness` label of `goodfetch.run.ended`: every
/// [`Completeness`].
pub const COMPLETENESS_LABEL: MetricLabel = MetricLabel {
    name: "completeness",
    values: || wire_name::all(&Completeness::ALL),
};

/// The `browser` label of `goodfetch.browser.running`: the pinned Chrome for
/// Testing, or the distribution's Chromium.
pub const BROWSER_LABEL: MetricLabel = MetricLabel {
    name: "browser",
    values: || vec!["chrome_for_testing", "chromium"],
};

/// The `reason` label of `goodfetch.browser.recycled`: why a pool recycled
/// a browser.
pub const RECYCLE_REASON_LABEL: MetricLabel = MetricLabel {
    name: "reason",
    values: || vec!["pages", "memory", "idle", "shed"],
};

/// The `surface` label of `goodfetch.client.connected`: every [`Surface`].
pub const SURFACE_LABEL: MetricLabel = MetricLabel {
    name: "surface",
    values: || wire_name::all(&Surface::ALL),
};

/// The `ended` label of `goodfetch.engine.panics`: every [`PanicEnded`].
pub const PANIC_ENDED_LABEL: MetricLabel = MetricLabel {
    name: "ended",
    values: || wire_name::all(&PanicEnded::ALL),
};

/// `goodfetch.engine.panics`: the panics the engine survived or stopped
/// on, by what each ended (ADR 0148).
pub const ENGINE_PANICS: Metric = Metric {
    name: "goodfetch.engine.panics",
    description: "The panics in the engine, each a bug in Goodfetch with a crash report on the machine, by what it ended: a call, an attempt, a Page, a Session, a browser, a Run, a Connection or the engine (ADR 0148).",
    instrument: Instrument::Counter,
    unit: "{panic}",
    labels: &[PANIC_ENDED_LABEL],
    stability: Stability::Experimental,
    arrived_in: ARRIVED_IN,
};

/// `goodfetch.run.active`: the live Runs.
pub const RUN_ACTIVE: Metric = Metric {
    name: "goodfetch.run.active",
    description: "The Daemon's live Runs, whose Lifecycle is `running`, by kind.",
    instrument: Instrument::UpDownCounter,
    unit: "{run}",
    labels: &[RUN_KIND_LABEL],
    stability: Stability::Stable,
    arrived_in: ARRIVED_IN,
};

/// `goodfetch.run.ended`: the Runs that ended, by their three axes.
pub const RUN_ENDED: Metric = Metric {
    name: "goodfetch.run.ended",
    description: "The Runs that ended on the Daemon, by kind, Lifecycle, Verdict and Completeness (ADR 0029).",
    instrument: Instrument::Counter,
    unit: "{run}",
    labels: &[
        RUN_KIND_LABEL,
        LIFECYCLE_LABEL,
        VERDICT_LABEL,
        COMPLETENESS_LABEL,
    ],
    stability: Stability::Stable,
    arrived_in: ARRIVED_IN,
};

/// `goodfetch.browser.running`: the browsers the Runs launched.
pub const BROWSER_RUNNING: Metric = Metric {
    name: "goodfetch.browser.running",
    description: "The browsers the Daemon's Runs launched that run now, by kind: the pinned Chrome for Testing, or the distribution's Chromium where no build is made.",
    instrument: Instrument::UpDownCounter,
    unit: "{browser}",
    labels: &[BROWSER_LABEL],
    stability: Stability::Stable,
    arrived_in: ARRIVED_IN,
};

/// `goodfetch.browser.recycled`: the browsers the Runs' pools recycled.
pub const BROWSER_RECYCLED: Metric = Metric {
    name: "goodfetch.browser.recycled",
    description: "The browsers the Daemon's Runs' pools recycled, by why: `pages`, after `governor.browser.recycle_pages`; `memory`, past `governor.browser.memory_cap`; `idle`, past `governor.browser.recycle_idle` with no open page; `shed`, by the Resource governor at critical (ADR 0040).",
    instrument: Instrument::Counter,
    unit: "{browser}",
    labels: &[RECYCLE_REASON_LABEL],
    stability: Stability::Experimental,
    arrived_in: ARRIVED_IN,
};

/// `goodfetch.browser.memory`: what the Runs' browsers use.
pub const BROWSER_MEMORY: Metric = Metric {
    name: "goodfetch.browser.memory",
    description: "The memory the browsers the Resource governor counts used at its last sample, each process with its descendants, PSS on Linux, summed over every browser; 0 with none (ADR 0040).",
    instrument: Instrument::Gauge,
    unit: "By",
    labels: &[],
    stability: Stability::Experimental,
    arrived_in: ARRIVED_IN,
};

/// `goodfetch.client.connected`: the connected Clients.
pub const CLIENT_CONNECTED: Metric = Metric {
    name: "goodfetch.client.connected",
    description: "The Clients connected to the Daemon, by the surface each named in its `hello`.",
    instrument: Instrument::UpDownCounter,
    unit: "{client}",
    labels: &[SURFACE_LABEL],
    stability: Stability::Stable,
    arrived_in: ARRIVED_IN,
};

/// `goodfetch.daemon.draining`: whether the Daemon drains.
pub const DAEMON_DRAINING: Metric = Metric {
    name: "goodfetch.daemon.draining",
    description: "1 while the Daemon drains, taking no Run (ADR 0049), else 0.",
    instrument: Instrument::Gauge,
    unit: "{daemon}",
    labels: &[],
    stability: Stability::Stable,
    arrived_in: ARRIVED_IN,
};

/// `goodfetch.counted_work`'s `shared_path` label: the Shared paths
/// (ADR 0140) with a counted row so far, each by name, which `xtask codegen`
/// writes from `goodfetch-counted-work`'s rows.
pub const COUNTED_WORK_PATH_LABEL: MetricLabel = MetricLabel {
    name: "shared_path",
    values: || SHARED_PATHS.to_vec(),
};

/// `goodfetch.counted_work`'s `work` label: what those rows count, which
/// `xtask codegen` writes from the same rows.
pub const COUNTED_WORK_WORK_LABEL: MetricLabel = MetricLabel {
    name: "work",
    values: || WORKS.to_vec(),
};

include!("../generated/counted_work.rs");

/// `goodfetch.counted_work`: the Counted work of each Shared path (ADR 0140).
pub const COUNTED_WORK: Metric = Metric {
    name: "goodfetch.counted_work",
    description: "The Counted work the Daemon did, by Shared path and kind of work: what `xtask counts` holds to the Quality register's limits, per unit, on its benchmarks.",
    instrument: Instrument::Counter,
    unit: "{work}",
    labels: &[COUNTED_WORK_PATH_LABEL, COUNTED_WORK_WORK_LABEL],
    stability: Stability::Experimental,
    arrived_in: ARRIVED_IN,
};

/// `goodfetch.run_state.wal.size`: what the open run files' write-ahead
/// logs hold.
pub const RUN_STATE_WAL_SIZE: Metric = Metric {
    name: "goodfetch.run_state.wal.size",
    description: "The bytes the write-ahead logs of the Daemon's open run files hold, summed over them, each as of its writer's last commit. A run file's log stays under twice its checkpoint threshold (32 MiB) unless a reader holds an old snapshot of it (ADR 0138).",
    instrument: Instrument::UpDownCounter,
    unit: "By",
    labels: &[],
    stability: Stability::Experimental,
    arrived_in: ARRIVED_IN,
};

/// `goodfetch.run_state.blocked_checkpoints`: the checkpoints a reader's
/// old snapshot blocked.
pub const RUN_STATE_BLOCKED_CHECKPOINTS: Metric = Metric {
    name: "goodfetch.run_state.blocked_checkpoints",
    description: "The checkpoints of a run file's write-ahead log that could not copy the whole log into the file, or start the log again, because a reader held an old snapshot of it (ADR 0138).",
    instrument: Instrument::Counter,
    unit: "{checkpoint}",
    labels: &[],
    stability: Stability::Experimental,
    arrived_in: ARRIVED_IN,
};

/// Every entry this module defines.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[],
    events: &[],
    handler_requests: &[],
    error_codes: &[],
    plugin_kinds: &[],
    metrics: &[
        RUN_ACTIVE,
        RUN_ENDED,
        BROWSER_RUNNING,
        BROWSER_RECYCLED,
        BROWSER_MEMORY,
        CLIENT_CONNECTED,
        DAEMON_DRAINING,
        COUNTED_WORK,
        RUN_STATE_WAL_SIZE,
        RUN_STATE_BLOCKED_CHECKPOINTS,
        ENGINE_PANICS,
    ],
    settings: &[ADDRESS, OTLP_METRICS_ENDPOINT],
    hook_stages: &[],
    shared_type_values: &[],
};

#[cfg(test)]
mod tests;
