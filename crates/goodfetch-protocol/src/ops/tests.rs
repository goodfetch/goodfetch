use rstest::rstest;

use super::{CLIENT_CONNECTED, RUN_ACTIVE, RUN_ENDED};
use crate::catalog::Metric;
use crate::connection::Surface;
use crate::entries::MODULES;
use crate::governor::{PRESSURE_SOURCE, PressureSource, TARGET, WorkKind};
use crate::run::{Completeness, Lifecycle, RunKind, Verdict};
use crate::storage::{AVAILABLE, INODES_AVAILABLE, LEVEL, SampledFolder};
use crate::wire_name::tests::{every_value_of, sorted};

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
                    && !(label.values)().is_empty()
                    && (label.values)().iter().all(|value| is_word(value));
                (!bounded).then(|| format!("{}'s label {}", metric.name, label.name))
            });
            name.into_iter().chain(labels)
        })
        .collect();
    assert_eq!(faults, Vec::<String>::new());
}

fn values_of(metric: &Metric, label: &str) -> Vec<&'static str> {
    (metric
        .labels
        .iter()
        .find(|declared| declared.name == label)
        .unwrap()
        .values)()
}

/// A runtime metric's label that labels a protocol enum takes every value
/// of it, so a value the engine records is always a declared one; `lifecycle`
/// all but `running`, which no ended Run is in.
#[rstest]
#[case::lifecycle(
    sorted(&[values_of(&RUN_ENDED, "lifecycle"), vec!["running"]].concat()),
    every_value_of::<Lifecycle>()
)]
#[case::storage_inodes_holds(
    sorted(&values_of(&INODES_AVAILABLE, "holds")),
    every_value_of::<SampledFolder>()
)]
#[case::storage_level_holds(sorted(&values_of(&LEVEL, "holds")), every_value_of::<SampledFolder>())]
#[case::surface(sorted(&values_of(&CLIENT_CONNECTED, "surface")), every_value_of::<Surface>())]
#[case::active_run_kind(sorted(&values_of(&RUN_ACTIVE, "run_kind")), every_value_of::<RunKind>())]
#[case::ended_run_kind(sorted(&values_of(&RUN_ENDED, "run_kind")), every_value_of::<RunKind>())]
#[case::verdict(sorted(&values_of(&RUN_ENDED, "verdict")), every_value_of::<Verdict>())]
#[case::completeness(
    sorted(&values_of(&RUN_ENDED, "completeness")),
    every_value_of::<Completeness>()
)]
#[case::storage_holds(sorted(&values_of(&AVAILABLE, "holds")), every_value_of::<SampledFolder>())]
#[case::governor_kind(sorted(&values_of(&TARGET, "kind")), every_value_of::<WorkKind>())]
#[case::governor_source(
    sorted(&values_of(&PRESSURE_SOURCE, "source")),
    every_value_of::<PressureSource>()
)]
fn a_runtime_metrics_enum_label_takes_every_value_of_its_enum(
    #[case] declared: Vec<String>,
    #[case] every_value: Vec<String>,
) {
    assert_eq!(declared, every_value);
}
