//! The process Plugin target (#171, Seam 3) against two toy Plugins of the
//! `sample_store` kind, each a real process speaking the plugin protocol
//! over its stdio: `goodfetch-toy-plugin`, which conforms, and the same
//! executable with `--forget`, which keeps no value and so is broken in the
//! `replace` capability it declares. With `--leave-out`, it declares one
//! capability fewer, which the suite then never reaches.

use std::collections::BTreeMap;
use std::process::{Command, Stdio};

use goodfetch_conformance::{
    PluginSuite, PluginSuiteError, PluginTarget, ProvingGround, run_plugin_suite,
};
use goodfetch_protocol::journey::FeatureId;
use goodfetch_protocol::result_document::{
    CapabilityResult, CaseOutcome, ConformanceDocument, LoadedDocument, Outcome, ResultDocument,
    TargetIdentity, read,
};
use goodfetch_test_support::{ProvingGround as LocalCopy, TestEnv};
use serde_json::{Value, json};

use crate::schema_violations;

/// This commit's toy Plugin.
const TOY_PLUGIN: &str = env!("CARGO_BIN_EXE_goodfetch-toy-plugin");

/// The toy Plugin started with `arguments`, in the test's folder, with
/// nothing in its environment the harness does not add.
fn toy(env: &TestEnv, arguments: &[&str]) -> PluginTarget {
    PluginTarget {
        executable: TOY_PLUGIN.into(),
        arguments: arguments
            .iter()
            .map(|&argument| argument.to_owned())
            .collect(),
        environment: BTreeMap::new(),
        folder: env.folder().to_path_buf(),
    }
}

/// The `sample_store` suite run against the toy Plugin started with
/// `arguments`, and the local copy of the Proving ground.
fn run(arguments: &[&str]) -> ConformanceDocument {
    let env = TestEnv::new();
    let copy = LocalCopy::start(&env, env.seed()).unwrap();
    let proving_ground = ProvingGround::local(&copy.address().to_string());
    run_plugin_suite(
        &PluginSuite::sample_store(),
        &toy(&env, arguments),
        &proving_ground,
    )
    .unwrap()
}

/// Each capability's name and outcome.
fn outcomes(document: &ConformanceDocument) -> Vec<(&str, Outcome)> {
    document
        .capabilities()
        .iter()
        .map(|capability| (capability.name(), capability.outcome()))
        .collect()
}

/// The outcome of each case, in the suite's order.
fn case_outcomes(document: &ConformanceDocument) -> Vec<(&str, CaseOutcome)> {
    document
        .capabilities()
        .iter()
        .flat_map(CapabilityResult::cases)
        .map(|case| (case.facts().id.as_str(), case.outcome()))
        .collect()
}

/// Covers: story:2-51
#[test]
fn the_conforming_toy_plugin_is_proved_in_every_capability_in_a_valid_conformance_document() {
    let document = run(&[]);
    crate::keep(&document, "sample-store-toy-plugin");

    let header = document.header();
    let written = ResultDocument::Conformance(document.clone());
    let covers: Vec<Vec<&str>> = document
        .capabilities()
        .iter()
        .flat_map(CapabilityResult::cases)
        .map(|case| case.facts().covers.iter().map(FeatureId::as_str).collect())
        .collect();
    assert_eq!(
        (
            outcomes(&document),
            header.suite.name.as_str(),
            header.target.name.as_str(),
            &header.target.identity,
            header.target.product,
            serde_json::to_value(&written).unwrap()["kind"].clone(),
            covers,
            schema_violations(&written),
        ),
        (
            vec![
                ("put", Outcome::Proved),
                ("replace", Outcome::Proved),
                ("newer_params", Outcome::Proved),
            ],
            "sample-store",
            "goodfetch-toy-plugin",
            &TargetIdentity::Version(env!("CARGO_PKG_VERSION").to_owned()),
            None,
            json!("conformance"),
            vec![vec!["story:2-51"]; 3],
            Vec::<String>::new(),
        )
    );
    assert_eq!(
        read(&serde_json::to_vec(&written).unwrap()).unwrap(),
        LoadedDocument::Conformance(document)
    );
}

/// Covers: story:2-51
#[test]
fn a_toy_plugin_broken_in_a_declared_capability_fails_it_with_its_case_and_evidence() {
    let document = run(&["--forget"]);

    let failed = &document.capabilities()[1].cases()[0];
    let entry = &failed.facts().entries[0];
    let answers: Vec<&Value> = entry
        .evidence
        .messages
        .iter()
        .filter(|kept| kept["from"] == "plugin")
        .map(|kept| &kept["message"]["result"])
        .collect();
    assert_eq!(
        (
            outcomes(&document),
            case_outcomes(&document),
            entry
                .checks
                .iter()
                .map(|check| (check.name.as_str(), check.passed))
                .collect::<Vec<_>>(),
            answers,
        ),
        (
            vec![
                ("put", Outcome::Proved),
                ("replace", Outcome::Failed),
                ("newer_params", Outcome::Proved),
            ],
            vec![
                (
                    "a-value-under-a-new-key-replaces-nothing",
                    CaseOutcome::Passed
                ),
                ("a-value-under-a-kept-key-replaces-it", CaseOutcome::Failed),
                (
                    "params-from-a-newer-engine-are-read-without-their-unknown-field",
                    CaseOutcome::Passed
                ),
            ],
            vec![
                ("the first value under the key replaces nothing", true),
                ("the second value under the key replaces the first", false),
            ],
            vec![&json!({ "replaced": false }), &json!({ "replaced": false })],
        )
    );
    assert_eq!(
        schema_violations(&ResultDocument::Conformance(document)),
        Vec::<String>::new()
    );
}

/// Covers: story:2-51
#[test]
fn a_capability_the_toy_plugin_does_not_declare_is_never_reached_and_stays_unproved() {
    let document = run(&["--leave-out", "newer_params"]);

    let unreached = &document.capabilities()[2].cases()[0];
    assert_eq!(
        (
            outcomes(&document),
            unreached.outcome(),
            unreached.facts().entries.len(),
        ),
        (
            vec![
                ("put", Outcome::Proved),
                ("replace", Outcome::Proved),
                ("newer_params", Outcome::Unproved),
            ],
            CaseOutcome::NotReached,
            0,
        )
    );
}

/// The harness gives the Plugin the plugin protocol's cookie, without which
/// the toy Plugin, started by hand, refuses to run (ADR 0033).
///
/// Covers: story:2-51
#[test]
fn the_toy_plugin_started_without_the_cookie_refuses_to_run() {
    let started = Command::new(TOY_PLUGIN)
        .env_clear()
        .stdin(Stdio::null())
        .output()
        .unwrap();

    assert_eq!(
        (started.status.success(), started.stdout.is_empty()),
        (false, true)
    );
}

/// Covers: story:2-51
#[test]
fn a_plugin_that_declares_no_implementation_of_the_suites_kind_is_refused() {
    let env = TestEnv::new();
    let copy = LocalCopy::start(&env, env.seed()).unwrap();
    let proving_ground = ProvingGround::local(&copy.address().to_string());
    let suite = PluginSuite::read(
        r#"
        product = "foxhound"
        kind = "sample_page_filter"

        [suite]
        name = "sample-page-filter"
        version = "1"

        [[capability]]
        name = "put"

        [[capability.case]]
        id = "a-value-under-a-new-key-replaces-nothing"
        covers = ["story:2-51"]
        "#,
    )
    .unwrap();

    let refused = run_plugin_suite(&suite, &toy(&env, &[]), &proving_ground).unwrap_err();

    assert!(
        matches!(
            &refused,
            PluginSuiteError::KindUndeclared { kind, declared }
                if kind == "sample_page_filter" && declared == &["sample_store".to_owned()]
        ),
        "{refused:?}"
    );
}
