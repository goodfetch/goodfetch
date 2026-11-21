use std::collections::BTreeMap;

use serde_json::json;

use super::generate;

/// Covers: story:11-182
#[test]
fn the_list_names_each_deprecated_key_with_its_replacement_in_key_order() {
    let catalog = json!({
        "settings": [
            {
                "name": "timeout.wait",
                "products": ["collie", "foxhound"],
                "deprecated": {
                    "since": "0.2",
                    "replacement": "timeout.navigation",
                    "earliest_removal": "1.0",
                    "note": "One timeout | both."
                }
            },
            { "name": "timeout.navigation", "products": ["collie"], "deprecated": null },
            {
                "name": "sample.echo_repeats",
                "products": ["collie"],
                "deprecated": {
                    "since": "0.1",
                    "replacement": null,
                    "earliest_removal": "1.0",
                    "note": "A sample."
                }
            }
        ]
    });

    let python = [
        (
            "goodfetch_collie_client.generated.settings".to_owned(),
            BTreeMap::from([(
                "sample.echo_repeats".to_owned(),
                vec![
                    "ConnectionSampleSettings.echo_repeats".to_owned(),
                    "RunSampleSettings.echo_repeats".to_owned(),
                ],
            )]),
        ),
        (
            "goodfetch_foxhound.generated.settings".to_owned(),
            BTreeMap::from([(
                "sample.echo_repeats".to_owned(),
                vec!["RunSampleSettings.echo_repeats".to_owned()],
            )]),
        ),
    ];

    let list = generate(&catalog, &python).unwrap();

    let rows: Vec<&str> = list
        .lines()
        .filter(|line| line.starts_with("| `"))
        .collect();
    assert_eq!(
        rows,
        [
            "| `sample.echo_repeats` | collie | `goodfetch_collie_client.generated.settings`: `ConnectionSampleSettings.echo_repeats`, `RunSampleSettings.echo_repeats`; `goodfetch_foxhound.generated.settings`: `RunSampleSettings.echo_repeats` | 0.1 | 1.0 | none | A sample. |",
            "| `timeout.wait` | collie, foxhound | none | 0.2 | 1.0 | `timeout.navigation` | One timeout \\| both. |",
        ],
        "{list}"
    );
}

#[test]
fn the_list_says_so_when_no_key_is_deprecated() {
    let catalog = json!({ "settings": [{ "name": "timeout.navigation", "products": ["collie"] }] });

    let list = generate(&catalog, &[]).unwrap();

    assert!(
        list.ends_with("\nNo settings key is deprecated.\n"),
        "{list}"
    );
}
