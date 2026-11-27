//! The Secret provider's kind suite (`conformance/secret/cases.toml`, spec
//! #596, ticket 996): run by the harness (#171) against
//! `goodfetch-scripted-plugin` scripted as a toy Secret provider over the
//! plugin protocol, and in process through `PluginSuite::checks_in_process`.

use std::collections::BTreeMap;
use std::fs;

use goodfetch_conformance::{PluginSuite, PluginTarget, ProvingGround, run_plugin_suite};
use goodfetch_protocol::plugin::PluginManifest;
use goodfetch_protocol::result_document::{CaseCheck, ConformanceDocument, Outcome};
use goodfetch_test_support::{ProvingGround as LocalCopy, TestEnv, Tool, tools};
use serde_json::{Value, json};

/// Runs the Secret provider kind suite against `goodfetch-scripted-plugin`
/// declaring `secret`'s `resolve` capability and answering `secret.resolve`
/// with `answer`.
#[expect(
    clippy::disallowed_methods,
    reason = "the test writes the scripted Plugin's script before the harness starts it"
)]
fn run_against_scripted(answer: &Value) -> ConformanceDocument {
    let env = TestEnv::new();
    let script = env.folder().join("script.json");
    let hello = json!({
        "protocol_version": "0.1",
        "name": "scripted-vault",
        "version": "1.0.0",
        "kinds": [{ "kind": "secret", "capabilities": ["resolve"] }],
        "network_level": "none",
    });
    fs::write(
        &script,
        json!({ "hello": hello, "answers": { "secret.resolve": answer } }).to_string(),
    )
    .unwrap();
    let manifest = PluginManifest::read(
        json!({
            "manifest_version": "0.1",
            "name": "scripted-vault",
            "version": "1.0.0",
            "kinds": ["secret"],
            "protocol": { "min": "0.1", "max": "0.1" },
            "entrypoint": "plugin",
            "network_level": "none",
            "scheme": "scripted-vault",
        })
        .to_string()
        .as_bytes(),
    )
    .unwrap();
    let target = PluginTarget {
        manifest,
        executable: tools()
            .unwrap()
            .path(Tool::ScriptedPlugin)
            .unwrap()
            .to_owned(),
        arguments: vec![script.display().to_string()],
        environment: BTreeMap::new(),
        folder: env.folder().to_path_buf(),
    };
    let copy = LocalCopy::start(&env, env.seed()).unwrap();
    let proving_ground = ProvingGround::local(&copy.address().to_string());
    run_plugin_suite(&PluginSuite::secret(), &target, &proving_ground).unwrap()
}

fn outcomes(document: &ConformanceDocument) -> Vec<(&str, Outcome)> {
    document
        .capabilities()
        .iter()
        .map(|capability| (capability.name(), capability.outcome()))
        .collect()
}

/// A toy Secret provider answering `{ value }` passes the `#171` kind suite
/// (`conformance/secret/cases.toml`); one returning an extra field or an empty
/// value fails it.
///
/// Covers: story:7-90
#[test]
fn the_secret_suite_proves_a_conforming_toy_secret_provider_and_fails_a_broken_one() {
    let conforming = run_against_scripted(&json!({ "result": { "value": "vault-secret-value" } }));
    let extra_field = run_against_scripted(
        &json!({ "result": { "value": "vault-secret-value", "extra": true } }),
    );
    let empty_value = run_against_scripted(&json!({ "result": { "value": "" } }));

    assert_eq!(
        (
            outcomes(&conforming),
            outcomes(&extra_field),
            outcomes(&empty_value),
        ),
        (
            vec![("resolve", Outcome::Proved)],
            vec![("resolve", Outcome::Failed)],
            vec![("resolve", Outcome::Failed)],
        ),
    );
}

/// An in-process Secret provider passes the same `conformance/secret/cases.toml`
/// cases as a process Plugin (ADR 0129).
///
/// Covers: story:7-90
#[test]
fn the_secret_suite_checks_in_process_pass_for_a_conforming_implementation() {
    let results: Vec<(String, Vec<CaseCheck>)> =
        PluginSuite::secret().checks_in_process(|method, params| {
            assert_eq!(method, "secret.resolve");
            assert_eq!(params["reference"], json!("plugin:conformance:item"));
            json!({ "result": { "value": "in-process-vault-value" } })
        });
    let failed: Vec<_> = results
        .into_iter()
        .filter(|(_, checks)| checks.iter().any(|check| !check.passed))
        .collect();
    assert_eq!(failed, Vec::<(String, Vec<CaseCheck>)>::new());
}
