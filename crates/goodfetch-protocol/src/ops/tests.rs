use rstest::rstest;
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use super::{CLIENT_CONNECTED, RUN_ACTIVE, RUN_ENDED};
use crate::catalog::Metric;
use crate::connection::Surface;
use crate::entries::MODULES;
use crate::run::{Completeness, Lifecycle, RunKind, Verdict};
use crate::storage::{AVAILABLE, INODES_AVAILABLE, LEVEL, SampledFolder};

/// What a label must never name: a value from an unbounded set, which
/// would blow up cardinality and leak targets (ADR 0050).
const UNBOUNDED: [&str; 8] = [
    "url",
    "domain",
    "host",
    "origin",
    "path",
    "run_id",
    "session_id",
    "page_id",
];

fn is_word(text: &str) -> bool {
    !text.is_empty()
        && text
            .chars()
            .all(|character| character.is_ascii_lowercase() || character == '_')
}

/// Every metric of the catalog is named under `goodfetch.`, and each of its
/// labels declares its values, each a word, and names nothing unbounded.
///
/// Covers: story:3-103
#[test]
fn every_metric_is_named_under_goodfetch_and_its_labels_take_bounded_values() {
    let metrics: Vec<&Metric> = MODULES.iter().flat_map(|entries| entries.metrics).collect();

    let faults: Vec<String> = metrics
        .iter()
        .flat_map(|metric| {
            let name = (!metric.name.starts_with("goodfetch."))
                .then(|| format!("{} is not under goodfetch.", metric.name));
            let labels = metric.labels.iter().filter_map(move |label| {
                let bounded = is_word(label.name)
                    && !UNBOUNDED.contains(&label.name)
                    && !label.values.is_empty()
                    && label.values.iter().all(|value| is_word(value));
                (!bounded).then(|| format!("{}'s label {}", metric.name, label.name))
            });
            name.into_iter().chain(labels)
        })
        .collect();
    assert_eq!(faults, Vec::<String>::new());
}

fn values_of(metric: &Metric, label: &str) -> &'static [&'static str] {
    metric
        .labels
        .iter()
        .find(|declared| declared.name == label)
        .unwrap()
        .values
}

fn every_value_is_a<T: DeserializeOwned>(values: &[&str]) -> bool {
    values
        .iter()
        .all(|value| serde_json::from_value::<T>(json!(value)).is_ok())
}

/// The runtime metrics' label values are the protocol's own values of what
/// they label.
#[rstest]
#[case::active_run_kind(every_value_is_a::<RunKind>(values_of(&RUN_ACTIVE, "run_kind")))]
#[case::ended_run_kind(every_value_is_a::<RunKind>(values_of(&RUN_ENDED, "run_kind")))]
#[case::lifecycle(every_value_is_a::<Lifecycle>(values_of(&RUN_ENDED, "lifecycle")))]
#[case::verdict(every_value_is_a::<Verdict>(values_of(&RUN_ENDED, "verdict")))]
#[case::completeness(every_value_is_a::<Completeness>(values_of(&RUN_ENDED, "completeness")))]
#[case::surface(every_value_is_a::<Surface>(values_of(&CLIENT_CONNECTED, "surface")))]
#[case::storage_available_holds(every_value_is_a::<SampledFolder>(values_of(&AVAILABLE, "holds")))]
#[case::storage_inodes_holds(every_value_is_a::<SampledFolder>(values_of(&INODES_AVAILABLE, "holds")))]
#[case::storage_level_holds(every_value_is_a::<SampledFolder>(values_of(&LEVEL, "holds")))]
fn a_runtime_metrics_label_values_are_the_protocols_own(#[case] own: bool) {
    assert!(own);
}

/// Every value `T` takes on the wire, as its JSON Schema lists them: the
/// variants its serde attributes name, read apart from any hand-kept list.
fn every_value_of<T: JsonSchema>() -> Vec<String> {
    let schema = schemars::schema_for!(T);
    let schema = schema.as_value();
    let listed = schema["enum"].as_array().into_iter().flatten();
    let one_of = schema["oneOf"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|variant| &variant["const"]);
    let mut values: Vec<String> = listed
        .chain(one_of)
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect();
    values.sort();
    values
}

fn sorted(values: &[&str]) -> Vec<String> {
    let mut values: Vec<String> = values.iter().map(|value| (*value).to_owned()).collect();
    values.sort();
    values
}

/// A runtime metric's label that labels a protocol enum takes every value
/// of it, so a value the engine records is always a declared one.
#[rstest]
#[case::surface(sorted(values_of(&CLIENT_CONNECTED, "surface")), every_value_of::<Surface>())]
#[case::active_run_kind(sorted(values_of(&RUN_ACTIVE, "run_kind")), every_value_of::<RunKind>())]
#[case::ended_run_kind(sorted(values_of(&RUN_ENDED, "run_kind")), every_value_of::<RunKind>())]
#[case::verdict(sorted(values_of(&RUN_ENDED, "verdict")), every_value_of::<Verdict>())]
#[case::completeness(
    sorted(values_of(&RUN_ENDED, "completeness")),
    every_value_of::<Completeness>()
)]
fn a_runtime_metrics_enum_label_takes_every_value_of_its_enum(
    #[case] declared: Vec<String>,
    #[case] every_value: Vec<String>,
) {
    assert_eq!(declared, every_value);
}
