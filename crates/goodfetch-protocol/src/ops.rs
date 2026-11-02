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
    arrived_in: ARRIVED_IN,
};

/// The values of the `run_kind` label: [`crate::run::RunKind`]'s.
const RUN_KIND: MetricLabel = MetricLabel {
    name: "run_kind",
    values: &["collie", "crawl"],
};

/// `goodfetch.run.active`: the live Runs.
pub const RUN_ACTIVE: Metric = Metric {
    name: "goodfetch.run.active",
    description: "The Daemon's live Runs, whose Lifecycle is `running`, by kind.",
    instrument: Instrument::UpDownCounter,
    unit: "{run}",
    labels: &[RUN_KIND],
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
        RUN_KIND,
        MetricLabel {
            name: "lifecycle",
            values: &["stopped", "finished", "failed"],
        },
        MetricLabel {
            name: "verdict",
            values: &["verified", "rejected", "needs_review"],
        },
        MetricLabel {
            name: "completeness",
            values: &["complete", "incomplete", "unverified"],
        },
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
    labels: &[MetricLabel {
        name: "browser",
        values: &["chrome_for_testing", "chromium"],
    }],
    stability: Stability::Stable,
    arrived_in: ARRIVED_IN,
};

/// `goodfetch.browser.recycled`: the browsers the Runs' pools recycled.
pub const BROWSER_RECYCLED: Metric = Metric {
    name: "goodfetch.browser.recycled",
    description: "The browsers the Daemon's Runs' pools recycled, by why: `pages`, after `governor.browser.recycle_pages`; `memory`, past `governor.browser.memory_cap`; `idle`, past `governor.browser.recycle_idle` with no open page; `shed`, by the Resource governor at critical (ADR 0040).",
    instrument: Instrument::Counter,
    unit: "{browser}",
    labels: &[MetricLabel {
        name: "reason",
        values: &["pages", "memory", "idle", "shed"],
    }],
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
    description: "The Clients connected to the Daemon, by the surface each named in its `hello`, Console tabs included.",
    instrument: Instrument::UpDownCounter,
    unit: "{client}",
    labels: &[MetricLabel {
        name: "surface",
        values: &[
            "sdk_typescript",
            "sdk_python",
            "rust",
            "cli",
            "mcp",
            "console",
            "cloud",
            "browser_host",
            "worker",
        ],
    }],
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

/// The values of `goodfetch.counted_work`'s `shared_path` label: the Shared
/// paths (ADR 0140) with a counted row so far, each by name.
const COUNTED_WORK_PATH: MetricLabel = MetricLabel {
    name: "shared_path",
    values: &[
        "action",
        "cdp_command",
        "frontier_admission",
        "frontier_hop",
        "frontier_payload",
        "frontier_sitemap",
        "hook_stage",
        "json_rpc_message",
        "page_analysis",
        "record",
        "run_state",
        "sink_delivery",
        "usage_charge",
    ],
};

/// The values of `goodfetch.counted_work`'s `work` label: what those rows
/// count.
const COUNTED_WORK_WORK: MetricLabel = MetricLabel {
    name: "work",
    values: &[
        "deep_copies",
        "encodes",
        "parses",
        "selector_compilations",
        "readability_runs",
        "file_opens",
        "locale_form_builds",
        "plugin_serializations",
        "plugin_answer_parses",
        "builds",
        "calls",
        "messages_no_hook",
        "opens",
        "transactions",
        "writer_reads",
        "declaration_reads",
        "validator_builds",
        "validations",
        "serializations",
        "constant_lookups",
        "invariant_reads",
        "compilations",
        "sends",
        "log_sends_off",
        "script_sends",
        "check_commands",
        "click_commands",
        "fill_commands",
        "press_commands",
        "configuration_reads",
        "prunes",
        "effect_syncs",
        "statements",
    ],
};

/// `goodfetch.counted_work`: the Counted work of each Shared path (ADR 0140).
pub const COUNTED_WORK: Metric = Metric {
    name: "goodfetch.counted_work",
    description: "The Counted work the Daemon did, by Shared path and kind of work: what `xtask counts` holds to the Quality register's limits, per unit, on its benchmarks.",
    instrument: Instrument::Counter,
    unit: "{work}",
    labels: &[COUNTED_WORK_PATH, COUNTED_WORK_WORK],
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
    ],
    settings: &[ADDRESS, OTLP_METRICS_ENDPOINT],
    hook_stages: &[],
    shared_type_values: &[],
};

#[cfg(test)]
mod tests;
