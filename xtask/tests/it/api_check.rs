//! The API breakage check of the contracts (ADRs 0030, 0050, 0069, 0094 and
//! 0095): each Product's catalog in its projection, its settings and metric
//! names among it, and the bundle's `--json` output types, compared with the
//! commit `compat/api.toml` names as the previous release. A change is a
//! break or an addition by the side the data flows: what a user sends may
//! widen, what Goodfetch answers may grow. An Experimental entry and an
//! announced break pass.
//! Covers: story:1-30

use serde_json::{Value, json};

use crate::workspace::{Workspace, finding};

const COLLIE: &str = "crates/goodfetch-protocol/generated/collie.json";
const FOXHOUND: &str = "crates/goodfetch-protocol/generated/foxhound.json";
const BUNDLE: &str = "crates/goodfetch-protocol/generated/bundle.json";
const API: &str = "compat/api.toml";

fn edit_json(workspace: &Workspace, path: &str, change: impl FnOnce(&mut Value)) {
    let mut value: Value = serde_json::from_str(&workspace.read_text(path)).unwrap();
    change(&mut value);
    workspace.write(
        path,
        serde_json::to_string_pretty(&value).unwrap().as_bytes(),
    );
}

fn entry<'a>(projection: &'a mut Value, section: &str, name: &str) -> &'a mut Value {
    projection[section]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|entry| entry["name"] == name)
        .unwrap()
}

fn remove(projection: &mut Value, section: &str, name: &str) {
    projection[section]
        .as_array_mut()
        .unwrap()
        .retain(|entry| entry["name"] != name);
}

/// A stable setting of Collie's, in both projections, as a setting is.
fn setting() -> Value {
    json!({
        "arrived_in": "0.1",
        "authority": "every",
        "clearable": true,
        "default": 10,
        "deprecated": null,
        "description": "How many sample pages a crawl visits at most.",
        "fixed_for_run": false,
        "name": "sample.max_pages",
        "product": "collie",
        "products": ["collie", "foxhound"],
        "scopes": ["run"],
        "value": {"minimum": 1, "type": "integer"},
        "weakens": false
    })
}

/// A `--json` output type of a hand-written command.
fn output() -> Value {
    json!({
        "properties": {
            "state": {"enum": ["running", "stopped"], "type": "string"},
            "pid": {"type": ["integer", "null"]}
        },
        "required": ["state", "pid"],
        "type": "object"
    })
}

/// The codegen fixture with `before` applied, committed as the previous
/// release that `compat/api.toml` names, with `announced` as its announced
/// breaks.
fn released_with(before: impl FnOnce(&Workspace), announced: &str) -> Workspace {
    let workspace = Workspace::from_fixture("codegen");
    before(&workspace);
    let previous = workspace.commit_all();
    workspace.write(
        API,
        format!("previous = \"{previous}\"\n{announced}").as_bytes(),
    );
    workspace
}

fn released() -> Workspace {
    released_with(
        |workspace| {
            for path in [COLLIE, FOXHOUND] {
                edit_json(workspace, path, |projection| {
                    projection["settings"]
                        .as_array_mut()
                        .unwrap()
                        .push(setting());
                });
            }
            edit_json(workspace, BUNDLE, |bundle| {
                bundle["outputs"] = json!({"daemon.status": output()});
            });
        },
        "",
    )
}

#[test]
fn an_unchanged_api_passes() {
    let workspace = released();

    let outcome = workspace.xtask(&["api-check"]);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stderr);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

#[test]
fn a_removed_stable_entry_is_a_break_of_its_products() {
    let workspace = released();
    for path in [COLLIE, FOXHOUND] {
        edit_json(&workspace, path, |projection| {
            remove(projection, "error_codes", "sample_refused");
        });
    }

    let outcome = workspace.xtask(&["api-check"]);

    assert_eq!(
        outcome.findings(),
        vec![
            finding("unannounced-break", "collie error_codes sample_refused"),
            finding("unannounced-break", "foxhound error_codes sample_refused"),
        ],
        "{}",
        outcome.stderr
    );
    assert_eq!(outcome.code, 1);
}

#[test]
fn an_added_entry_is_an_addition() {
    let workspace = released();
    for path in [COLLIE, FOXHOUND] {
        edit_json(&workspace, path, |projection| {
            let mut added = setting();
            added["name"] = json!("sample.min_pages");
            projection["settings"].as_array_mut().unwrap().push(added);
        });
    }

    let outcome = workspace.xtask(&["api-check"]);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stderr);
    assert!(
        outcome
            .notes()
            .contains(&finding("addition", "collie settings sample.min_pages")),
        "{}",
        outcome.stdout
    );
    assert_eq!(outcome.code, 0);
}

#[test]
fn an_entry_leaving_collies_catalog_for_foxhounds_is_a_break_for_collie_alone() {
    let workspace = released();
    edit_json(&workspace, COLLIE, |projection| {
        remove(projection, "metrics", "goodfetch.sample.echoes");
    });
    edit_json(&workspace, FOXHOUND, |projection| {
        entry(projection, "metrics", "goodfetch.sample.echoes")["product"] = json!("foxhound");
    });

    let outcome = workspace.xtask(&["api-check"]);

    assert_eq!(
        outcome.findings(),
        vec![finding(
            "unannounced-break",
            "collie metrics goodfetch.sample.echoes"
        )],
        "{}",
        outcome.stderr
    );
}

#[test]
fn an_entry_joining_collies_catalog_from_foxhounds_is_an_addition() {
    let workspace = released();
    let mut moved = Value::Null;
    edit_json(&workspace, FOXHOUND, |projection| {
        let crawl = entry(projection, "operations", "sample.crawl");
        crawl["product"] = json!("collie");
        moved = crawl.clone();
    });
    edit_json(&workspace, COLLIE, |projection| {
        projection["operations"].as_array_mut().unwrap().push(moved);
        let definitions: Value = serde_json::from_str(&workspace.read_text(FOXHOUND)).unwrap();
        for name in ["SampleCrawlParams", "SampleCrawlResult"] {
            projection["$defs"][name] = definitions["$defs"][name].clone();
        }
    });

    let outcome = workspace.xtask(&["api-check"]);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stderr);
    assert!(
        outcome
            .notes()
            .contains(&finding("addition", "collie operations sample.crawl")),
        "{}",
        outcome.stdout
    );
}

#[test]
fn a_change_to_an_experimental_entry_passes() {
    let workspace = released();
    for path in [COLLIE, FOXHOUND] {
        edit_json(&workspace, path, |projection| {
            remove(projection, "operations", "sample.echo");
            remove(projection, "events", "sample.echoed");
        });
    }

    let outcome = workspace.xtask(&["api-check"]);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stderr);
    assert_eq!(outcome.code, 0);
}

#[test]
fn a_stable_entry_made_experimental_is_a_break() {
    let workspace = released();
    edit_json(&workspace, FOXHOUND, |projection| {
        entry(projection, "operations", "sample.crawl")["stability"] = json!("experimental");
    });

    let outcome = workspace.xtask(&["api-check"]);

    assert_eq!(
        outcome.findings(),
        vec![finding(
            "unannounced-break",
            "foxhound operations sample.crawl"
        )],
        "{}",
        outcome.stderr
    );
}

#[test]
fn an_announced_break_passes() {
    let workspace = released_with(
        |_| {},
        "\n[[announced]]\nentry = \"foxhound error_codes sample_crawl_refused\"\nreason = \"Refused by sample.crawl's own checks now.\"\n",
    );
    edit_json(&workspace, FOXHOUND, |projection| {
        remove(projection, "error_codes", "sample_crawl_refused");
    });

    let outcome = workspace.xtask(&["api-check"]);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stderr);
    assert!(
        outcome.notes().contains(&finding(
            "announced-break",
            "foxhound error_codes sample_crawl_refused"
        )),
        "{}",
        outcome.stdout
    );
    assert_eq!(outcome.code, 0);
}

#[test]
fn an_announcement_naming_no_break_fails() {
    let workspace = released_with(
        |_| {},
        "\n[[announced]]\nentry = \"foxhound error_codes sample_crawl_refused\"\nreason = \"Refused by sample.crawl's own checks now.\"\n",
    );

    let outcome = workspace.xtask(&["api-check"]);

    assert_eq!(
        outcome.findings(),
        vec![finding(
            "announced-without-break",
            "foxhound error_codes sample_crawl_refused"
        )],
        "{}",
        outcome.stderr
    );
}

/// Each case changes the stable `sample.crawl`'s types in Foxhound's
/// projection and says whether that breaks its callers.
#[rstest::rstest]
#[case::a_new_required_parameter(
    "SampleCrawlParams",
    |schema: &mut Value| {
        schema["properties"]["depth"] = json!({"type": "integer"});
        schema["required"].as_array_mut().unwrap().push(json!("depth"));
    },
    true
)]
#[case::a_new_optional_parameter(
    "SampleCrawlParams",
    |schema: &mut Value| { schema["properties"]["depth"] = json!({"type": "integer"}); },
    false
)]
#[case::a_removed_parameter(
    "SampleCrawlParams",
    |schema: &mut Value| {
        schema["properties"].as_object_mut().unwrap().remove("pages");
        schema["required"] = json!(["echo"]);
    },
    true
)]
#[case::a_parameter_made_optional(
    "SampleCrawlParams",
    |schema: &mut Value| { schema["required"] = json!(["echo"]); },
    false
)]
#[case::a_parameter_of_another_type(
    "SampleCrawlParams",
    |schema: &mut Value| { schema["properties"]["pages"]["type"] = json!("string"); },
    true
)]
#[case::a_parameter_bound_narrowed(
    "SampleCrawlParams",
    |schema: &mut Value| { schema["properties"]["pages"]["minimum"] = json!(1); },
    true
)]
#[case::a_parameter_accepting_null_too(
    "SampleCrawlParams",
    |schema: &mut Value| { schema["properties"]["pages"]["type"] = json!(["integer", "null"]); },
    false
)]
#[case::a_parameter_of_a_shared_type_required(
    "SampleEchoParams",
    |schema: &mut Value| {
        schema["properties"]["times"] = json!({"type": "integer"});
        schema["required"].as_array_mut().unwrap().push(json!("times"));
    },
    true
)]
#[case::a_new_result_field(
    "SampleCrawlResult",
    |schema: &mut Value| { schema["properties"]["skipped"] = json!({"type": "integer"}); },
    false
)]
#[case::a_removed_result_field(
    "SampleCrawlResult",
    |schema: &mut Value| {
        schema["properties"].as_object_mut().unwrap().remove("visited");
        schema["required"] = json!([]);
    },
    true
)]
#[case::a_result_field_made_optional(
    "SampleCrawlResult",
    |schema: &mut Value| { schema["required"] = json!([]); },
    true
)]
#[case::a_result_field_that_may_be_null(
    "SampleCrawlResult",
    |schema: &mut Value| { schema["properties"]["visited"]["type"] = json!(["integer", "null"]); },
    true
)]
#[case::a_reworded_description(
    "SampleCrawlResult",
    |schema: &mut Value| { schema["properties"]["visited"]["description"] = json!("Pages visited."); },
    false
)]
fn an_operations_types_change_by_the_side_its_data_flows(
    #[case] definition: &str,
    #[case] change: fn(&mut Value),
    #[case] breaks: bool,
) {
    let workspace = released();
    edit_json(&workspace, FOXHOUND, |projection| {
        change(&mut projection["$defs"][definition]);
    });

    let outcome = workspace.xtask(&["api-check"]);

    let expected = if breaks {
        vec![finding(
            "unannounced-break",
            "foxhound operations sample.crawl",
        )]
    } else {
        vec![]
    };
    assert_eq!(outcome.findings(), expected, "{}", outcome.stdout);
}

#[rstest::rstest]
#[case::a_removed_setting(|setting: &mut Value| { *setting = Value::Null; }, true)]
#[case::a_narrower_value(|setting: &mut Value| { setting["value"]["minimum"] = json!(2); }, true)]
#[case::a_wider_value(|setting: &mut Value| { setting["value"]["minimum"] = json!(0); }, false)]
#[case::a_scope_removed(|setting: &mut Value| { setting["scopes"] = json!([]); }, true)]
#[case::a_scope_added(|setting: &mut Value| { setting["scopes"] = json!(["run", "connection"]); }, false)]
#[case::a_tuning_default(|setting: &mut Value| { setting["default"] = json!(20); }, false)]
fn a_settings_change_by_what_a_user_may_write(
    #[case] change: fn(&mut Value),
    #[case] breaks: bool,
) {
    let workspace = released();
    edit_json(&workspace, COLLIE, |projection| {
        let settings = projection["settings"].as_array_mut().unwrap();
        change(&mut settings[0]);
        settings.retain(|setting| !setting.is_null());
    });

    let outcome = workspace.xtask(&["api-check"]);

    let expected = if breaks {
        vec![finding(
            "unannounced-break",
            "collie settings sample.max_pages",
        )]
    } else {
        vec![]
    };
    assert_eq!(outcome.findings(), expected, "{}", outcome.stdout);
}

#[test]
fn a_changed_default_of_a_setting_that_weakens_a_guarantee_is_a_break() {
    let workspace = released_with(
        |workspace| {
            for path in [COLLIE, FOXHOUND] {
                edit_json(workspace, path, |projection| {
                    let mut weakening = setting();
                    weakening["weakens"] = json!(true);
                    projection["settings"]
                        .as_array_mut()
                        .unwrap()
                        .push(weakening);
                });
            }
        },
        "",
    );
    edit_json(&workspace, COLLIE, |projection| {
        entry(projection, "settings", "sample.max_pages")["default"] = json!(20);
    });

    let outcome = workspace.xtask(&["api-check"]);

    assert_eq!(
        outcome.findings(),
        vec![finding(
            "unannounced-break",
            "collie settings sample.max_pages"
        )],
        "{}",
        outcome.stdout
    );
}

#[rstest::rstest]
#[case::a_label_removed(json!(["tier"]), json!([]), true)]
#[case::a_label_added(json!([]), json!(["tier"]), false)]
fn a_metrics_labels(#[case] before: Value, #[case] after: Value, #[case] breaks: bool) {
    let workspace = released_with(
        |workspace| {
            edit_json(workspace, COLLIE, |projection| {
                entry(projection, "metrics", "goodfetch.sample.echoes")["labels"] = before;
            });
        },
        "",
    );
    edit_json(&workspace, COLLIE, |projection| {
        entry(projection, "metrics", "goodfetch.sample.echoes")["labels"] = after;
    });

    let outcome = workspace.xtask(&["api-check"]);

    let expected = if breaks {
        vec![finding(
            "unannounced-break",
            "collie metrics goodfetch.sample.echoes",
        )]
    } else {
        vec![]
    };
    assert_eq!(outcome.findings(), expected, "{}", outcome.stdout);
}

#[rstest::rstest]
#[case::a_removed_output(|outputs: &mut Value| { outputs.as_object_mut().unwrap().remove("daemon.status"); }, true)]
#[case::a_removed_field(
    |outputs: &mut Value| {
        let status = &mut outputs["daemon.status"];
        status["properties"].as_object_mut().unwrap().remove("pid");
        status["required"] = json!(["state"]);
    },
    true
)]
#[case::a_new_field(
    |outputs: &mut Value| { outputs["daemon.status"]["properties"]["address"] = json!({"type": "string"}); },
    false
)]
#[case::a_new_value(
    |outputs: &mut Value| { outputs["daemon.status"]["properties"]["state"]["enum"] = json!(["running", "stopped", "starting"]); },
    false
)]
#[case::a_new_command(|outputs: &mut Value| { outputs["daemon.stop"] = json!({"type": "object"}); }, false)]
fn a_json_outputs_change_by_what_a_reader_may_meet(
    #[case] change: fn(&mut Value),
    #[case] breaks: bool,
) {
    let workspace = released();
    edit_json(&workspace, BUNDLE, |bundle| change(&mut bundle["outputs"]));

    let outcome = workspace.xtask(&["api-check"]);

    let expected = if breaks {
        vec![finding("unannounced-break", "bundle outputs daemon.status")]
    } else {
        vec![]
    };
    assert_eq!(outcome.findings(), expected, "{}", outcome.stdout);
}

#[test]
fn an_input_value_removed_is_a_break_and_one_added_is_not() {
    let workspace = released_with(
        |workspace| {
            edit_json(workspace, FOXHOUND, |projection| {
                projection["$defs"]["SampleCrawlParams"]["properties"]["mode"] =
                    json!({"enum": ["fast", "slow"], "type": "string"});
            });
        },
        "",
    );
    edit_json(&workspace, FOXHOUND, |projection| {
        projection["$defs"]["SampleCrawlParams"]["properties"]["mode"]["enum"] =
            json!(["fast", "thorough"]);
    });

    let outcome = workspace.xtask(&["api-check"]);

    assert_eq!(
        outcome.findings(),
        vec![finding(
            "unannounced-break",
            "foxhound operations sample.crawl"
        )],
        "{}",
        outcome.stdout
    );
}
