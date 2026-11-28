//! On-demand Sink inspection operations (`sink.probe` and `sink.list`, ADRs
//! 0026 and 0099) and shared settings-opening helpers used by `run/deliver.rs`.

use std::collections::BTreeMap;

use goodfetch_engine::notification_endpoint::NotificationEndpoint;
use goodfetch_engine::run::{run_error, run_layer, settings_error};
use goodfetch_engine::runs::{RunError, Runs};
use goodfetch_engine::settings::{Opening, Resolved};
use goodfetch_engine::{CallContext, CallError};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::foxhound::sink::{
    SINKS, SinkList, SinkListEntry, SinkListParams, SinkProbe, SinkProbeOutcome, SinkProbeParams,
    SinkSettings,
};
use goodfetch_protocol::run::{Presence, RunKind, RunListParams};
use goodfetch_protocol::settings::{ProjectSettings, RunOrigin, SettingsLayer};

use crate::contract::RunCopy;
use crate::report;
use crate::throughput::DEFAULT_WRITE_TIMEOUT;

use super::{Destination, deliverable, entries, layer, probe_failed};

/// Resolves `settings` and `project` over the connection's settings as a
/// Foxhound crawl Run would.
pub(super) async fn open_settings(
    settings: &serde_json::Map<String, serde_json::Value>,
    project: Option<ProjectSettings>,
    context: &CallContext,
) -> Result<Resolved, CallError> {
    let (opened, _) = context
        .engine()
        .settings()
        .open(Opening {
            run: Some(Product::Foxhound),
            project,
            recipe: None,
            settings: run_layer(settings, context)?,
            origin: Some(RunOrigin::Settings),
            registered: context.registered_settings().clone(),
            acting_for_model: context.surface().acts_for_model(),
            exposed: context.exposed(),
        })
        .await
        .map_err(|error| settings_error(&error, context))?;
    Ok(opened.resolved().clone())
}

/// Probes one configured Sink outside any Run.
async fn probe_configured(
    sink: &str,
    entry: &SinkSettings,
    layer: SettingsLayer,
    resolved: &Resolved,
    context: &CallContext,
) -> SinkProbeOutcome {
    let (_, secrets) = context.runs().outside_a_run();
    let owner = context.engine().owner();
    let clock = context.engine().clock();
    let endpoint = NotificationEndpoint::of(resolved, clock, owner.blocking_threads());
    let destination = Destination {
        name: sink,
        settings: entry,
        layer,
        write_timeout: DEFAULT_WRITE_TIMEOUT,
        run_id: None,
        endpoint: &endpoint,
    };
    let run = RunCopy {
        run_id: String::new(),
        run_epoch: 0,
    };
    match destination.open(&secrets, owner, clock, run).await {
        Ok(mut driver) => match owner.blocking(move || driver.probe()).await {
            Ok(class) => SinkProbeOutcome::Proved { class },
            Err(failure) => SinkProbeOutcome::Failed {
                cause: probe_failed(&failure),
            },
        },
        Err(cause) => SinkProbeOutcome::Failed { cause },
    }
}

/// `sink.probe`: probes the Sink the params name, among the `sinks` their
/// settings resolve to as a crawl's Run would.
pub(super) async fn probe(
    mut params: SinkProbeParams,
    context: CallContext,
) -> Result<SinkProbe, CallError> {
    let settings = open_settings(&params.settings, params.project.take(), &context).await?;
    let layer = layer(&settings);
    let sink = params.sink;
    let Some((_, entry)) = entries(&settings).find(|(name, _)| *name == sink) else {
        let named: Vec<String> = entries(&settings)
            .map(|(name, _)| format!("`{name}`"))
            .collect();
        let cause = if named.is_empty() {
            format!("the settings name no Sink: `{}` is empty", SINKS.name)
        } else {
            format!(
                "the settings name no Sink `{sink}`, only {}",
                named.join(", ")
            )
        };
        return Ok(SinkProbe {
            sink,
            outcome: SinkProbeOutcome::Failed { cause },
        });
    };
    deliverable(&sink, &entry, layer)?;
    let outcome = probe_configured(&sink, &entry, layer, &settings, &context).await;
    Ok(SinkProbe { sink, outcome })
}

/// `sink.list`: lists each Sink with its Guarantee class, dead-letter count
/// and deliveries outside the destination's deduplication window (ADRs 0026
/// and 0099).
pub(super) async fn list(
    mut params: SinkListParams,
    context: CallContext,
) -> Result<SinkList, CallError> {
    if let Some(run_id) = params.run_id {
        let failed = |error: RunError| run_error(error, &context);
        let mut reader = context.runs().reader(run_id).await.map_err(failed)?;
        let reports = reader.read(report::reports).await.map_err(failed)?;
        let sinks = reports
            .into_iter()
            .map(|entry| SinkListEntry {
                name: entry.name,
                guarantee_class: Some(entry.guarantee_class),
                dead_letters: entry.dead_letters,
                outside_dedup_window: entry.outside_dedup_window,
                status: Some(entry.status),
                probe_cause: None,
            })
            .collect();
        return Ok(SinkList { sinks });
    }
    let settings = open_settings(&params.settings, params.project.take(), &context).await?;
    let layer = layer(&settings);
    let totals = aggregate_sink_counts(context.runs()).await;
    let mut sinks = Vec::new();
    for (name, entry) in entries(&settings) {
        let (dead_letters, outside_dedup_window) = totals.get(&name).copied().unwrap_or((0, 0));
        let (guarantee_class, probe_cause) = match deliverable(&name, &entry, layer) {
            Ok(()) => match probe_configured(&name, &entry, layer, &settings, &context).await {
                SinkProbeOutcome::Proved { class } => (Some(class), None),
                SinkProbeOutcome::Failed { cause } => (None, Some(cause)),
            },
            Err(error) => (None, Some(error.to_string())),
        };
        sinks.push(SinkListEntry {
            name,
            guarantee_class,
            dead_letters,
            outside_dedup_window,
            status: None,
            probe_cause,
        });
    }
    sinks.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(SinkList { sinks })
}

/// Aggregates `(dead_letters, outside_dedup_window)` per Sink across every
/// known Crawl Run.
async fn aggregate_sink_counts(runs: &Runs) -> BTreeMap<String, (u64, u64)> {
    let mut totals: BTreeMap<String, (u64, u64)> = BTreeMap::new();
    let Ok(rows) = runs.list(RunListParams::default()).await else {
        return totals;
    };
    for row in rows {
        if row.presence != Presence::Present || row.kind != RunKind::Crawl {
            continue;
        }
        let Ok(mut reader) = runs.reader(row.run_id).await else {
            continue;
        };
        let Ok(reports) = reader.read(report::reports).await else {
            continue;
        };
        for entry in reports {
            let counts = totals.entry(entry.name).or_insert((0, 0));
            counts.0 = counts.0.saturating_add(entry.dead_letters);
            counts.1 = counts.1.saturating_add(entry.outside_dedup_window);
        }
    }
    totals
}
