//! Settings-named `hooks` layering, locking, duplicate detection and product
//! checks (ADRs 0033, 0088 and 0104; issue 954).

use std::collections::BTreeMap;

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::plugin::HookStageName;
use goodfetch_protocol::plugin::hook::{HOOKS, HookEntry};
use goodfetch_protocol::recipe::{RECIPE_MANIFEST_VERSION, RecipeEntry, RecipeManifest};
use goodfetch_protocol::settings::{Exposure, SettingsLayer};
use serde_json::{Map, Value, json};

use crate::settings::{Layers, Policy, Recipe, SettingsError, Values, resolve};

fn hooks_map(entries: &Value) -> Map<String, Value> {
    json!({ "hooks": entries }).as_object().unwrap().clone()
}

fn entry(name: &str, stage: &str, plugin: &str) -> Value {
    json!({ "name": name, "stage": stage, "plugin": plugin })
}

fn resolved_names(layers: &Layers) -> Vec<String> {
    resolve(layers, Product::Collie, Product::Collie)
        .unwrap()
        .effective(&HOOKS)
        .unwrap()
        .value
        .as_array()
        .unwrap()
        .iter()
        .map(|raw| HookEntry::read(raw).unwrap().name)
        .collect()
}

/// Entries from Operator policy, user configuration, project configuration,
/// Recipe and run layers append in that exact order, and a lower layer cannot
/// remove an earlier layer's Hook.
///
/// Covers: story:7-63, story:7-69, story:7-110
#[test]
fn hooks_append_in_layer_order_and_lower_layer_never_removes() {
    let recipe_manifest = RecipeManifest {
        version: RECIPE_MANIFEST_VERSION,
        name: "sample-recipe".to_owned(),
        product: Product::Collie,
        entry: RecipeEntry::Steps("sample.steps.json".to_owned()),
        runtime: None,
        settings: hooks_map(&json!([entry("h-recipe", "after_navigation", "p-recipe")])),
        expose: BTreeMap::from([("hooks".to_owned(), Exposure::default())]),
    };
    let layers = Layers {
        operator_policy: Policy {
            settings: Values::read(
                SettingsLayer::OperatorPolicy,
                &hooks_map(&json!([entry("h-policy", "before_navigation", "p-policy")])),
                Product::Collie,
            )
            .unwrap(),
            ..Policy::default()
        },
        user_configuration: Values::read(
            SettingsLayer::UserConfiguration,
            &hooks_map(&json!([entry("h-user", "before_navigation", "p-user")])),
            Product::Collie,
        )
        .unwrap(),
        project_configuration: Values::read(
            SettingsLayer::ProjectConfiguration,
            &hooks_map(&json!([entry("h-project", "page_ready", "p-project")])),
            Product::Collie,
        )
        .unwrap(),
        recipe: Some(Recipe::read(&recipe_manifest, Product::Collie).unwrap()),
        run: Values::read(
            SettingsLayer::Run,
            &hooks_map(&json!([entry("h-run", "page_ready", "p-run")])),
            Product::Collie,
        )
        .unwrap(),
        ..Layers::default()
    };

    assert_eq!(
        resolved_names(&layers),
        vec!["h-policy", "h-user", "h-project", "h-recipe", "h-run"]
    );

    let empty_run = Layers {
        run: Values::read(SettingsLayer::Run, &hooks_map(&json!([])), Product::Collie).unwrap(),
        ..layers
    };
    assert_eq!(
        resolved_names(&empty_run),
        vec!["h-policy", "h-user", "h-project", "h-recipe"]
    );
}

/// Locking `hooks` in Operator policy forbids adding more in lower layers, and
/// a repeated `name` across layers fails with `hook_duplicate`.
///
/// Covers: story:7-63, story:7-110
#[test]
fn hooks_respect_operator_policy_lock_and_refuse_duplicates() {
    let locked = Layers {
        operator_policy: Policy {
            lock: Values::read(
                SettingsLayer::OperatorPolicy,
                &hooks_map(&json!([entry("h-policy", "before_navigation", "p-policy")])),
                Product::Collie,
            )
            .unwrap(),
            ..Policy::default()
        },
        run: Values::read(
            SettingsLayer::Run,
            &hooks_map(&json!([entry("h-run", "page_ready", "p-run")])),
            Product::Collie,
        )
        .unwrap(),
        ..Layers::default()
    };
    assert!(matches!(
        resolve(&locked, Product::Collie, Product::Collie),
        Err(SettingsError::Locked { ref key, .. }) if key == "hooks"
    ));

    let duplicate = Layers {
        operator_policy: Policy {
            settings: Values::read(
                SettingsLayer::OperatorPolicy,
                &hooks_map(&json!([entry("shared", "before_navigation", "p-policy")])),
                Product::Collie,
            )
            .unwrap(),
            ..Policy::default()
        },
        run: Values::read(
            SettingsLayer::Run,
            &hooks_map(&json!([entry("shared", "page_ready", "p-run")])),
            Product::Collie,
        )
        .unwrap(),
        ..Layers::default()
    };
    assert_eq!(
        resolve(&duplicate, Product::Collie, Product::Collie),
        Err(SettingsError::HookDuplicate {
            hook: "shared".to_owned(),
            stage: HookStageName::PAGE_READY.as_str().to_owned(),
        })
    );
}

/// Foxhound's `labels` and `datasets` fields on `hooks` entries are refused on
/// Collie with `not_in_product` at explicit layers, kept aside in shared
/// configuration files, and accepted on Foxhound (ADRs 0088 and 0095).
///
/// Covers: story:7-63, story:7-110
#[test]
fn foxhound_hook_scope_fields_are_refused_on_collie_with_not_in_product() {
    let with_labels = hooks_map(&json!([{
        "name": "scoped",
        "stage": "before_navigation",
        "plugin": "p",
        "labels": ["product"],
    }]));
    let with_datasets = hooks_map(&json!([{
        "name": "scoped-ds",
        "stage": "page_ready",
        "plugin": "p",
        "datasets": ["items"],
    }]));

    assert_eq!(
        Values::read(SettingsLayer::Run, &with_labels, Product::Collie),
        Err(SettingsError::NotInProduct {
            key: "labels".to_owned(),
            needs: Product::Foxhound,
        })
    );
    assert_eq!(
        Values::read(SettingsLayer::Run, &with_datasets, Product::Collie),
        Err(SettingsError::NotInProduct {
            key: "datasets".to_owned(),
            needs: Product::Foxhound,
        })
    );
    assert!(Values::read(SettingsLayer::Run, &with_labels, Product::Foxhound).is_ok());
    assert!(
        Values::read(
            SettingsLayer::UserConfiguration,
            &with_labels,
            Product::Collie
        )
        .is_ok()
    );
}
