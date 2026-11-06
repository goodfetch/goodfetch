//! How the details window words the Daemon's values: a Run's progress,
//! Usage and report, the settings in force and the Resource governor's
//! readings, each as labelled fields made safe to show by the Text
//! function.

use goodfetch_protocol::foxhound::progress::RequestProgress;
use goodfetch_protocol::governor::{
    AllowanceSource, GovernorLevel, KindConcurrency, PressureSource, SystemInfo,
};
use goodfetch_protocol::run::{
    Lifecycle, RunIndexRow, RunProgress, RunReport, RunUsage, StopReason,
};
use goodfetch_protocol::settings::{
    ExplainedSetting, RunOrigin, SettingsExplanation, SettingsLayer,
};
use serde_json::{Map, Value};

use super::super::product_name;
use super::{Field, SettingLine, SettingsPanel, VALUE_MOST};
use crate::client::CallFailure;
use crate::text;

/// The Daemon settings view of the settings in force, once read.
pub(super) fn settings_panel(
    explained: Option<&Result<Box<SettingsExplanation>, CallFailure>>,
) -> SettingsPanel {
    match explained {
        None => SettingsPanel {
            note: Some("Reading the settings…".to_owned()),
            sources: Vec::new(),
            settings: Vec::new(),
        },
        Some(Err(failure)) => SettingsPanel {
            note: Some(text::shown(
                &format!(
                    "The Daemon did not explain its settings: {} ({})",
                    failure.message, failure.code
                ),
                VALUE_MOST,
            )),
            sources: Vec::new(),
            settings: Vec::new(),
        },
        Some(Ok(explanation)) => {
            let mut sources = Vec::new();
            if let Some(version) = &explanation.policy_version {
                sources.push(Field::new("Operator policy", version));
            }
            for file in &explanation.files {
                let found = if file.found { "" } else { " (not found)" };
                sources.push(Field::new(
                    &format!("{} file", file_layer(file.layer)),
                    &format!("{}{found}", file.path),
                ));
            }
            SettingsPanel {
                note: None,
                sources,
                settings: explanation.settings.iter().map(setting_line).collect(),
            }
        }
    }
}

/// One setting's line of the Daemon settings view.
pub(super) fn setting_line(setting: &ExplainedSetting) -> SettingLine {
    let mut value = match &setting.value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    };
    if let Some(cap) = &setting.cap {
        value = format!("{value}, capped at {cap}");
    }
    if !setting.applied {
        let products: Vec<&str> = setting
            .products
            .iter()
            .map(|product| product_name(*product))
            .collect();
        let honour = if products.len() == 1 {
            "honours"
        } else {
            "honour"
        };
        value = format!(
            "{value}, not applied: only {} {honour} it",
            products.join(" and ")
        );
    }
    let set_by = match (setting.layer, setting.origin) {
        (SettingsLayer::Run, Some(origin)) => format!("the Run ({})", run_origin(origin)),
        (layer, _) => layer_name(layer).to_owned(),
    };
    SettingLine {
        key: text::shown(&setting.key, VALUE_MOST),
        value: text::shown(&value, VALUE_MOST),
        set_by: text::shown(&set_by, VALUE_MOST),
        locked_by: setting
            .locked_by
            .map_or_else(String::new, |layer| layer_name(layer).to_owned()),
    }
}

/// What the window calls the layer that set or locked a setting.
pub(super) fn layer_name(layer: SettingsLayer) -> &'static str {
    match layer {
        SettingsLayer::Default => "the default",
        SettingsLayer::OperatorPolicy => "the Operator policy",
        SettingsLayer::UserConfiguration => "user configuration",
        SettingsLayer::ProjectConfiguration => "project configuration",
        SettingsLayer::Recipe => "the Recipe",
        SettingsLayer::Run => "the Run",
    }
}

/// What the window calls the layer a configuration file is of.
pub(super) fn file_layer(layer: SettingsLayer) -> &'static str {
    match layer {
        SettingsLayer::Default => "Default",
        SettingsLayer::OperatorPolicy => "Operator policy",
        SettingsLayer::UserConfiguration => "User configuration",
        SettingsLayer::ProjectConfiguration => "Project configuration",
        SettingsLayer::Recipe => "Recipe",
        SettingsLayer::Run => "Run",
    }
}

/// Where a run-layer value came from.
pub(super) fn run_origin(origin: RunOrigin) -> &'static str {
    match origin {
        RunOrigin::Argument => "a call's argument",
        RunOrigin::Settings => "a scope's settings",
        RunOrigin::Flag => "a CLI flag",
        RunOrigin::Form => "an Integration's form",
    }
}

/// The Resource governor's reading, as its view shows it.
pub(super) fn governor_fields(reading: &SystemInfo) -> Vec<Field> {
    let level = match reading.pressure_source {
        Some(source) => format!(
            "{}, held by {}",
            governor_level(reading.level),
            pressure_source(source)
        ),
        None => governor_level(reading.level).to_owned(),
    };
    let mut fields = vec![Field::new("Level", &level)];
    if reading.last_resort {
        fields.push(Field::new(
            "Last resort",
            "memory stayed critical at the floor",
        ));
    }
    let memory = &reading.memory;
    fields.push(Field::new(
        "Memory",
        &format!(
            "{} used of {}, {}",
            bytes(memory.used),
            bytes(memory.allowance),
            allowance_source(memory.allowance_source)
        ),
    ));
    fields.push(Field::new("Browsers' memory", &bytes(memory.browsers)));
    fields.push(Field::new("Host memory", governor_level(memory.host)));
    fields.push(Field::new(
        "CPU",
        &format!(
            "{}% busy, {} threads",
            reading.cpu.busy, reading.cpu.parallelism
        ),
    ));
    fields.push(Field::new(
        "Tokio workers",
        &format!("{}% busy", reading.cpu.tokio_busy),
    ));
    let concurrency = &reading.concurrency;
    for (label, kind) in [
        ("HTTP fetches", &concurrency.http),
        ("Browser pages", &concurrency.browser_page),
        ("Model calls", &concurrency.model_call),
    ] {
        fields.push(Field::new(label, &kind_concurrency(kind)));
    }
    fields
}

pub(super) fn kind_concurrency(kind: &KindConcurrency) -> String {
    format!(
        "{} in use of {} (from {} to {})",
        kind.in_use, kind.target, kind.floor, kind.ceiling
    )
}

pub(super) fn governor_level(level: GovernorLevel) -> &'static str {
    match level {
        GovernorLevel::Normal => "normal",
        GovernorLevel::Pressure => "pressure",
        GovernorLevel::Critical => "critical",
    }
}

pub(super) fn pressure_source(source: PressureSource) -> &'static str {
    match source {
        PressureSource::Memory => "memory",
        PressureSource::HostMemory => "the host's memory",
        PressureSource::Cpu => "CPU",
        PressureSource::Tokio => "the Tokio workers",
    }
}

pub(super) fn allowance_source(source: AllowanceSource) -> &'static str {
    match source {
        AllowanceSource::Cgroup => "its cgroup's limit",
        AllowanceSource::JobObject => "its Job object's limit",
        AllowanceSource::Setting => "governor.memory_allowance",
        AllowanceSource::Physical => "half of physical memory",
    }
}

/// A count of bytes in GiB, or in MiB below one.
#[expect(clippy::cast_precision_loss, reason = "a reading shown to one decimal")]
pub(super) fn bytes(count: u64) -> String {
    const MIB: f64 = 1024.0 * 1024.0;
    let count = count as f64;
    if count >= 1024.0 * MIB {
        format!("{:.1} GiB", count / (1024.0 * MIB))
    } else {
        format!("{:.1} MiB", count / MIB)
    }
}

/// The chosen Run's progress, from its last progress read, or else its
/// row.
pub(super) fn progress_fields(
    row: Option<&RunIndexRow>,
    progress: Option<&RunProgress>,
    crawls: bool,
) -> Vec<Field> {
    let mut fields = Vec::new();
    let Some(progress) = progress else {
        if let Some(row) = row {
            fields.push(Field::new("Lifecycle", lifecycle(row.lifecycle)));
            if let Some(message) = &row.progress_message {
                fields.push(Field::new("Message", message));
            }
        }
        return fields;
    };
    fields.push(Field::new("Lifecycle", lifecycle(progress.lifecycle)));
    if let Some(message) = &progress.message {
        fields.push(Field::new("Message", message.as_str()));
    }
    if let Some(reason) = &progress.stop_reason {
        fields.push(Field::new("Stop reason", &stop_reason(reason)));
    }
    if !progress.pauses.is_empty() {
        fields.push(Field::new(
            "Human pauses",
            &progress.pauses.len().to_string(),
        ));
    }
    if crawls && let Some(requests) = part::<RequestProgress>(&progress.kind_parts) {
        fields.push(Field::new(
            "Requests",
            &format!(
                "{} succeeded, {} failed, {} skipped, {} blocked, {} pending, {} in flight",
                requests.succeeded,
                requests.failed,
                requests.skipped,
                requests.blocked,
                requests.pending,
                requests.fetching + requests.handling
            ),
        ));
    }
    fields
}

/// A Run's Usage, with the Limits that apply to it.
pub(super) fn usage_fields(usage: &RunUsage) -> Vec<Field> {
    let mut fields = vec![
        Field::new("Requests", &usage.requests.to_string()),
        Field::new("Model calls", &usage.model.calls.to_string()),
        Field::new("Model tokens", &usage.model.tokens.to_string()),
        Field::new("Browser time", &format!("{} s", usage.browser_time / 1000)),
        Field::new("Proxy bytes", &usage.proxy_bytes.to_string()),
    ];
    for limit in &usage.limits {
        fields.push(Field::new(
            "Limit",
            &format!(
                "{}: {} left of {}",
                limit.usage_kind, limit.left, limit.amount
            ),
        ));
    }
    if usage.uncapped {
        fields.push(Field::new(
            "Uncapped",
            "it spent in a currency no Limit caps",
        ));
    }
    fields
}

/// A Run's report: its axes with their reasons, its times, its error and
/// its warnings.
pub(super) fn report_fields(report: &RunReport) -> Vec<Field> {
    let mut fields = vec![Field::new("Lifecycle", lifecycle(report.lifecycle))];
    if let Some(reason) = &report.stop_reason {
        fields.push(Field::new("Stop reason", &stop_reason(reason)));
    }
    fields.push(Field::new("Verdict", report.verdict.as_str()));
    for reason in &report.verdict_reasons {
        fields.push(Field::new(
            "Why",
            &format!("{}: {}", reason.code, reason.message),
        ));
    }
    fields.push(Field::new("Completeness", report.completeness.as_str()));
    for reason in &report.completeness_reasons {
        fields.push(Field::new(
            "Why",
            &format!("{}: {}", reason.code, reason.message),
        ));
    }
    fields.push(Field::new("Started", report.started_at.as_str()));
    fields.push(Field::new("Ended", report.ended_at.as_str()));
    if let Some(error) = &report.error {
        fields.push(Field::new(
            "Error",
            &format!("{}: {}", error.code, error.message),
        ));
    }
    for warning in &report.warnings {
        fields.push(Field::new(
            "Warning",
            &format!("{}: {}", warning.code, warning.message),
        ));
    }
    fields
}

/// What the window says of a report read that failed with `code`.
pub(super) fn unreported(code: &str) -> String {
    if code == goodfetch_protocol::run::RUN_INTERRUPTED.name {
        format!("None: no engine works the Run, and a start with its Run ID resumes it ({code})")
    } else {
        format!("None: the Daemon answered {code}")
    }
}

/// A crawl's part `T` among a progress's or a report's kind parts, under
/// its `requests` key.
pub(super) fn part<T: serde::de::DeserializeOwned>(parts: &Map<String, Value>) -> Option<T> {
    parts
        .get("requests")
        .and_then(|requests| serde_json::from_value(requests.clone()).ok())
}

/// A stop reason by its kind, as the wire names it.
pub(super) fn stop_reason(reason: &StopReason) -> String {
    serde_json::to_value(reason)
        .ok()
        .and_then(|value| value.get("kind").and_then(Value::as_str).map(str::to_owned))
        .unwrap_or_default()
}

/// What the window says of a Lifecycle.
pub(super) fn lifecycle(lifecycle: Lifecycle) -> &'static str {
    match lifecycle {
        Lifecycle::Running => "Running",
        Lifecycle::Stopped => "Stopped",
        Lifecycle::Finished => "Finished",
        Lifecycle::Failed => "Failed",
    }
}
