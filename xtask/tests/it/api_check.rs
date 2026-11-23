//! The API breakage check of the contracts (ADRs 0030, 0050, 0069, 0094 and
//! 0095): each Product's catalog in its projection, its settings and metric
//! names among it, and the bundle's `--json` output types, compared with the
//! commit `compat/api.toml` names as the previous release. A change is a
//! break or an addition by the side the data flows: what a user sends may
//! widen, what Goodfetch answers may grow. An Experimental entry and an
//! announced break pass.
//! Covers: story:1-30

use serde_json::{Value, json};

use crate::workspace::{Finding, Workspace, finding};

mod connection;
mod defaults;

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

/// The stable entries `released` adds to both projections beside the
/// fixture's own, one per section the fixture leaves empty.
fn add_stable_entries(projection: &mut Value) {
    projection["settings"]
        .as_array_mut()
        .unwrap()
        .push(setting());
    projection["handler_requests"] = json!([{
        "name": "sample.ask",
        "params": {"$ref": "#/$defs/SampleAsk"},
        "result": {"$ref": "#/$defs/SampleAnswer"},
        "stability": "stable"
    }]);
    projection["exit_codes"] = json!([{"code": 3, "name": "refused"}]);
    projection["shared_type_values"] = json!([
        {"name": "consent_required", "type": "block_cause"},
        {"name": "consent_required", "type": "completeness_reason_code"}
    ]);
    entry(projection, "metrics", "goodfetch.sample.echoes")["labels"] =
        json!([{"name": "tier", "values": ["first", "second"]}]);
    entry(projection, "plugin_kinds", "sample_store")["stability"] = json!("stable");
    let definitions = &mut projection["$defs"];
    definitions["SampleAsk"] = json!({
        "additionalProperties": false,
        "properties": {"question": {"oneOf": [{"const": "why"}, {"const": "how"}]}},
        "required": ["question"],
        "type": "object"
    });
    definitions["SampleAnswer"] = json!({
        "additionalProperties": false,
        "properties": {"tone": {"oneOf": [{"const": "plain"}, {"const": "warm"}]}},
        "required": ["tone"],
        "type": "object"
    });
    definitions["SampleRefusal"] = json!({
        "properties": {"limit": {"type": "integer"}},
        "required": ["limit"],
        "type": "object"
    });
    if let Some(refused) = projection["error_codes"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|code| code["name"] == "sample_crawl_refused")
    {
        refused["details"] = json!({"$ref": "#/$defs/SampleRefusal"});
    }
}

fn released() -> Workspace {
    released_with(
        |workspace| {
            for path in [COLLIE, FOXHOUND] {
                edit_json(workspace, path, add_stable_entries);
            }
            edit_json(workspace, BUNDLE, |bundle| {
                bundle["outputs"] = json!({"daemon.status": output()});
                bundle["command_failed"] = json!({"$ref": "#/$defs/CommandFailed"});
                bundle["$defs"]["CommandFailed"] = json!({
                    "properties": {"code": {"type": "string"}},
                    "required": ["code"],
                    "type": "object"
                });
            });
        },
        "",
    )
}

/// The findings of a change to `subject`: one unannounced break, or none.
fn expected(breaks: bool, subject: &str) -> Vec<Finding> {
    if breaks {
        vec![finding("unannounced-break", subject)]
    } else {
        vec![]
    }
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

#[test]
fn an_announcement_of_a_rust_crates_break_is_left_to_semver_checks() {
    let workspace = released_with(
        |_| {},
        "\n[[announced]]\nentry = \"rust goodfetch-foxhound function_missing\"\nreason = \"`crawl` goes.\"\n",
    );

    let outcome = workspace.xtask(&["api-check"]);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stderr);
    assert_eq!(outcome.code, 0);
}

#[test]
fn an_announcement_of_an_sdk_packages_break_is_left_to_sdk_api() {
    let workspace = released_with(
        |_| {},
        "\n[[announced]]\nentry = \"typescript @goodfetch/collie/testing connectOver\"\nreason = \"`connectOver` goes.\"\n\n[[announced]]\nentry = \"python goodfetch_collie.testing connect_over\"\nreason = \"`connect_over` goes.\"\n",
    );

    let outcome = workspace.xtask(&["api-check"]);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stderr);
    assert_eq!(outcome.code, 0);
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

    assert_eq!(
        outcome.findings(),
        expected(breaks, "foxhound operations sample.crawl"),
        "{}",
        outcome.stdout
    );
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

    assert_eq!(
        outcome.findings(),
        expected(breaks, "collie settings sample.max_pages"),
        "{}",
        outcome.stdout
    );
}

/// Each case changes a stable entry of Foxhound's projection, other than an
/// operation's, and says whether that breaks the code written against it.
#[rstest::rstest]
#[case::a_metric_label_removed(
    "metrics goodfetch.sample.echoes",
    |projection: &mut Value| { entry(projection, "metrics", "goodfetch.sample.echoes")["labels"] = json!([]); },
    true
)]
#[case::a_metric_label_added(
    "metrics goodfetch.sample.echoes",
    |projection: &mut Value| {
        entry(projection, "metrics", "goodfetch.sample.echoes")["labels"]
            .as_array_mut().unwrap().push(json!({"name": "kind", "values": ["text"]}));
    },
    false
)]
#[case::a_metric_label_value_removed(
    "metrics goodfetch.sample.echoes",
    |projection: &mut Value| { entry(projection, "metrics", "goodfetch.sample.echoes")["labels"][0]["values"] = json!(["first"]); },
    true
)]
#[case::a_metric_label_value_added(
    "metrics goodfetch.sample.echoes",
    |projection: &mut Value| {
        entry(projection, "metrics", "goodfetch.sample.echoes")["labels"][0]["values"] = json!(["first", "second", "third"]);
    },
    false
)]
#[case::an_error_code_no_longer_retryable(
    "error_codes sample_crawl_refused",
    |projection: &mut Value| { entry(projection, "error_codes", "sample_crawl_refused")["retryable"] = json!(false); },
    true
)]
#[case::an_error_codes_details_field_removed(
    "error_codes sample_crawl_refused",
    |projection: &mut Value| { projection["$defs"]["SampleRefusal"] = json!({"type": "object"}); },
    true
)]
#[case::an_error_code_with_new_details(
    "error_codes sample_refused",
    |projection: &mut Value| { entry(projection, "error_codes", "sample_refused")["details"] = json!({"$ref": "#/$defs/SampleRefusal"}); },
    false
)]
#[case::an_exit_code_renumbered(
    "exit_codes refused",
    |projection: &mut Value| { entry(projection, "exit_codes", "refused")["code"] = json!(4); },
    true
)]
#[case::an_events_payload_field_removed(
    "events sample.crawled",
    |projection: &mut Value| { projection["$defs"]["SampleCrawled"] = json!({"type": "object"}); },
    true
)]
#[case::an_events_payload_field_added(
    "events sample.crawled",
    |projection: &mut Value| { projection["$defs"]["SampleCrawled"]["properties"]["skipped"] = json!({"type": "integer"}); },
    false
)]
#[case::a_handler_request_asking_a_new_question(
    "handler_requests sample.ask",
    |projection: &mut Value| { projection["$defs"]["SampleAsk"]["properties"]["question"]["oneOf"].as_array_mut().unwrap().push(json!({"const": "when"})); },
    false
)]
#[case::a_handler_request_asking_one_question_less(
    "handler_requests sample.ask",
    |projection: &mut Value| { projection["$defs"]["SampleAsk"]["properties"]["question"]["oneOf"] = json!([{"const": "why"}]); },
    false
)]
#[case::a_handler_request_asking_with_other_fields(
    "handler_requests sample.ask",
    |projection: &mut Value| { projection["$defs"]["SampleAsk"].as_object_mut().unwrap().remove("additionalProperties"); },
    false
)]
#[case::a_handler_answer_refusing_a_tone(
    "handler_requests sample.ask",
    |projection: &mut Value| { projection["$defs"]["SampleAnswer"]["properties"]["tone"]["oneOf"] = json!([{"const": "plain"}]); },
    true
)]
#[case::a_handler_answer_taking_another_tone(
    "handler_requests sample.ask",
    |projection: &mut Value| { projection["$defs"]["SampleAnswer"]["properties"]["tone"]["oneOf"].as_array_mut().unwrap().push(json!({"const": "terse"})); },
    false
)]
#[case::a_handler_answer_taking_other_fields(
    "handler_requests sample.ask",
    |projection: &mut Value| { projection["$defs"]["SampleAnswer"].as_object_mut().unwrap().remove("additionalProperties"); },
    false
)]
#[case::a_handler_answer_requiring_a_new_field(
    "handler_requests sample.ask",
    |projection: &mut Value| {
        let answer = &mut projection["$defs"]["SampleAnswer"];
        answer["properties"]["mood"] = json!({"type": "string"});
        answer["required"] = json!(["tone", "mood"]);
    },
    true
)]
#[case::a_plugin_method_added(
    "plugin_kinds sample_store",
    |projection: &mut Value| {
        let method = json!({"name": "take", "params": {"$ref": "#/$defs/SampleStorePutParams"}, "result": {"$ref": "#/$defs/SampleStorePutResult"}});
        entry(projection, "plugin_kinds", "sample_store")["methods"].as_array_mut().unwrap().push(method);
    },
    true
)]
#[case::a_plugin_method_given_a_new_parameter(
    "plugin_kinds sample_store",
    |projection: &mut Value| { projection["$defs"]["SampleStorePutParams"]["properties"]["ttl"] = json!({"type": "integer"}); },
    false
)]
#[case::a_shared_type_value_removed(
    "shared_type_values completeness_reason_code.consent_required",
    |projection: &mut Value| { projection["shared_type_values"].as_array_mut().unwrap().pop(); },
    true
)]
#[case::a_shared_type_value_moved(
    "shared_type_values completeness_reason_code.consent_required",
    |projection: &mut Value| { projection["shared_type_values"][1]["type"] = json!("audit_kind"); },
    true
)]
fn a_stable_entry_changes_by_the_side_its_data_flows(
    #[case] subject: &str,
    #[case] change: fn(&mut Value),
    #[case] breaks: bool,
) {
    let workspace = released();
    edit_json(&workspace, FOXHOUND, change);

    let outcome = workspace.xtask(&["api-check"]);

    assert_eq!(
        outcome.findings(),
        expected(breaks, &format!("foxhound {subject}")),
        "{}",
        outcome.stdout
    );
}

#[test]
fn a_shared_type_value_added_is_an_addition() {
    let workspace = released();
    edit_json(&workspace, FOXHOUND, |projection| {
        projection["shared_type_values"]
            .as_array_mut()
            .unwrap()
            .push(json!({"name": "payment_required", "type": "block_cause"}));
    });

    let outcome = workspace.xtask(&["api-check"]);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stdout);
    assert!(
        outcome.notes().contains(&finding(
            "addition",
            "foxhound shared_type_values block_cause.payment_required"
        )),
        "{}",
        outcome.stdout
    );
}

#[test]
fn two_entries_under_one_name_fail_rather_than_hide_one() {
    let workspace = released();
    edit_json(&workspace, FOXHOUND, |projection| {
        let refused = entry(projection, "exit_codes", "refused").clone();
        projection["exit_codes"]
            .as_array_mut()
            .unwrap()
            .push(refused);
    });

    let outcome = workspace.xtask(&["api-check"]);

    assert_eq!(
        outcome.findings(),
        vec![finding("duplicate-entry", "foxhound exit_codes refused")],
        "{}",
        outcome.stdout
    );
}

#[rstest::rstest]
#[case::a_removed_output(
    "outputs daemon.status",
    |bundle: &mut Value| { bundle["outputs"].as_object_mut().unwrap().remove("daemon.status"); },
    true
)]
#[case::a_removed_field(
    "outputs daemon.status",
    |bundle: &mut Value| {
        let status = &mut bundle["outputs"]["daemon.status"];
        status["properties"].as_object_mut().unwrap().remove("pid");
        status["required"] = json!(["state"]);
    },
    true
)]
#[case::a_new_field(
    "outputs daemon.status",
    |bundle: &mut Value| { bundle["outputs"]["daemon.status"]["properties"]["address"] = json!({"type": "string"}); },
    false
)]
#[case::a_new_value(
    "outputs daemon.status",
    |bundle: &mut Value| { bundle["outputs"]["daemon.status"]["properties"]["state"]["enum"] = json!(["running", "stopped", "starting"]); },
    false
)]
#[case::a_new_command(
    "outputs daemon.stop",
    |bundle: &mut Value| { bundle["outputs"]["daemon.stop"] = json!({"type": "object"}); },
    false
)]
#[case::a_failure_without_its_code(
    "command_failed",
    |bundle: &mut Value| { bundle["$defs"]["CommandFailed"] = json!({"type": "object"}); },
    true
)]
#[case::a_failure_with_a_new_field(
    "command_failed",
    |bundle: &mut Value| { bundle["$defs"]["CommandFailed"]["properties"]["hint"] = json!({"type": "string"}); },
    false
)]
fn a_json_outputs_change_by_what_a_reader_may_meet(
    #[case] subject: &str,
    #[case] change: fn(&mut Value),
    #[case] breaks: bool,
) {
    let workspace = released();
    edit_json(&workspace, BUNDLE, change);

    let outcome = workspace.xtask(&["api-check"]);

    assert_eq!(
        outcome.findings(),
        expected(breaks, &format!("bundle {subject}")),
        "{}",
        outcome.stdout
    );
}

#[test]
fn an_announcement_without_a_reason_fails() {
    let workspace = released_with(
        |_| {},
        "\n[[announced]]\nentry = \"foxhound error_codes sample_crawl_refused\"\nreason = \" \"\n",
    );
    edit_json(&workspace, FOXHOUND, |projection| {
        remove(projection, "error_codes", "sample_crawl_refused");
    });

    let outcome = workspace.xtask(&["api-check"]);

    assert_eq!(
        outcome.findings(),
        vec![finding(
            "announcement-without-reason",
            "foxhound error_codes sample_crawl_refused"
        )],
        "{}",
        outcome.stdout
    );
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
