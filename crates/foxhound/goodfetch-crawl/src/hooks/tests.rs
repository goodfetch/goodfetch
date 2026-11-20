use std::collections::BTreeMap;

use goodfetch_engine::settings::{Layers, Resolved, Values, resolve};
use goodfetch_fetch_tier::LabelSettings;
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::foxhound::crawl::LabelDeclaration;
use goodfetch_protocol::foxhound::hook::{CrawlHookDeclaration, CrawlHookStage};
use goodfetch_protocol::settings::{RunOrigin, SettingsLayer};
use serde_json::{Value, json};

use super::{Hooks, check};

fn resolved(run: Value) -> Resolved {
    let Value::Object(run) = run else {
        panic!("settings are a table");
    };
    let layers = Layers {
        run: Values::read(SettingsLayer::Run, &run, Product::Foxhound).unwrap(),
        ..Layers::default()
    };
    resolve(&layers, Product::Foxhound, Product::Foxhound).unwrap()
}

fn hook(name: &str, stage: CrawlHookStage, labels: Option<&[&str]>) -> CrawlHookDeclaration {
    CrawlHookDeclaration {
        name: name.to_owned(),
        stage,
        labels: labels.map(|labels| labels.iter().map(|&label| label.to_owned()).collect()),
        datasets: None,
    }
}

fn labels(declared: &[(&str, Value)]) -> LabelSettings {
    LabelSettings::of(
        &declared
            .iter()
            .map(|(name, settings)| {
                (
                    (*name).to_owned(),
                    LabelDeclaration {
                        payload: None,
                        settings: settings.as_object().cloned().unwrap_or_default(),
                    },
                )
            })
            .collect::<BTreeMap<_, _>>(),
    )
    .unwrap()
}

/// A Record goes through the Record stage's Hooks in the order declared,
/// scoped by Labels and Datasets alike, with no priority.
#[test]
fn a_record_goes_through_the_hooks_its_label_and_dataset_reach_in_declared_order() {
    let mut products_only = hook("price", CrawlHookStage::BeforeRecordCommit, None);
    products_only.datasets = Some(vec!["products".to_owned()]);
    let hooks = Hooks::of(
        vec![
            hook("stamp", CrawlHookStage::BeforeRecordCommit, None),
            products_only,
            hook(
                "detail_only",
                CrawlHookStage::BeforeRecordCommit,
                Some(&["detail"]),
            ),
            hook("sign", CrawlHookStage::BeforeRecordCommit, None),
        ],
        &resolved(json!({})),
        &LabelSettings::default(),
    );

    let names = |label, dataset| {
        hooks
            .on_record(label, dataset)
            .map(|hook| hook.name.as_str())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        (names("detail", "products"), names("list", "reviews")),
        (
            vec!["stamp", "price", "detail_only", "sign"],
            vec!["stamp", "sign"]
        )
    );
}

/// A Hook's call waits under its Label's own `timeout.hook`, set at the
/// run layer from the crawl's settings, else under the Run's.
///
/// Covers: setting:timeout.hook
#[test]
fn a_hook_waits_under_its_labels_timeout_else_the_runs() {
    let hooks = Hooks::of(
        Vec::new(),
        &resolved(json!({ "timeout": { "hook": 2_000 } })),
        &labels(&[("detail", json!({ "timeout": { "hook": 50 } }))]),
    );

    let limit = |label| {
        let limit = hooks.deadline(label).limit();
        (limit.value.clone(), limit.layer, limit.origin)
    };
    assert_eq!(
        (limit("detail"), limit("list")),
        (
            (json!(50), SettingsLayer::Run, Some(RunOrigin::Settings)),
            (json!(2_000), SettingsLayer::Run, None),
        )
    );
}

/// A start declaring a Hook at a stage this engine does not run Hooks in
/// code at yet is refused with `invalid_params`; the Record stage passes.
#[test]
fn a_hook_at_a_stage_not_run_yet_is_refused() {
    let refused = check(&[
        hook("stamp", CrawlHookStage::BeforeRecordCommit, None),
        hook("sign", CrawlHookStage::BeforeRequest, None),
    ])
    .unwrap_err();

    assert_eq!(
        (
            refused.code(),
            check(&[hook("stamp", CrawlHookStage::BeforeRecordCommit, None)]).is_ok(),
        ),
        ("invalid_params", true)
    );
}
