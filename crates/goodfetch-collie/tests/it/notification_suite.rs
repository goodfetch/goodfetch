//! The Notification channel's kind suite (`conformance/notification/cases.toml`,
//! spec #596, ticket 1023): run by the harness (#171) against
//! `goodfetch-scripted-plugin` scripted as a toy Notification channel over the
//! plugin protocol, and in process through `PluginSuite::checks_in_process`,
//! plus manifest network-level checks (all three levels allowed).

use std::collections::BTreeMap;
use std::fs;

use goodfetch_conformance::{PluginSuite, PluginTarget, ProvingGround, run_plugin_suite};
use goodfetch_protocol::plugin::{NetworkLevel, PluginManifest};
use goodfetch_protocol::result_document::{CaseCheck, ConformanceDocument, Outcome};
use goodfetch_test_support::{ProvingGround as LocalCopy, TestEnv, Tool, tools};
use serde_json::{Value, json};

/// Runs the Notification channel kind suite against `goodfetch-scripted-plugin`
/// declaring `notification`'s `send` capability and answering
/// `notification.send` with `answer`.
#[expect(
    clippy::disallowed_methods,
    reason = "the test writes the scripted Plugin's script before the harness starts it"
)]
fn run_against_scripted(answer: &Value) -> ConformanceDocument {
    let env = TestEnv::new();
    let script = env.folder().join("script.json");
    let hello = json!({
        "protocol_version": "0.1",
        "name": "scripted-pager",
        "version": "1.0.0",
        "kinds": [{ "kind": "notification", "capabilities": ["send"] }],
        "network_level": "none",
    });
    fs::write(
        &script,
        json!({ "hello": hello, "answers": { "notification.send": answer } }).to_string(),
    )
    .unwrap();
    let manifest = PluginManifest::read(
        json!({
            "manifest_version": "0.1",
            "name": "scripted-pager",
            "version": "1.0.0",
            "kinds": ["notification"],
            "protocol": { "min": "0.1", "max": "0.1" },
            "entrypoint": "plugin",
            "network_level": "none",
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
    run_plugin_suite(&PluginSuite::notification(), &target, &proving_ground).unwrap()
}

fn outcomes(document: &ConformanceDocument) -> Vec<(&str, Outcome)> {
    document
        .capabilities()
        .iter()
        .map(|capability| (capability.name(), capability.outcome()))
        .collect()
}

/// A toy Notification channel answering `{ delivered: true }` passes the
/// `#171` kind suite (`conformance/notification/cases.toml`); one returning
/// an extra field or `delivered: false` fails it.
///
/// Covers: story:7-96
#[test]
fn the_notification_suite_proves_a_conforming_toy_channel_and_fails_a_broken_one() {
    let conforming = run_against_scripted(&json!({ "result": { "delivered": true } }));
    let extra_field =
        run_against_scripted(&json!({ "result": { "delivered": true, "extra": true } }));
    let not_delivered = run_against_scripted(&json!({ "result": { "delivered": false } }));

    assert_eq!(
        (
            outcomes(&conforming),
            outcomes(&extra_field),
            outcomes(&not_delivered),
        ),
        (
            vec![("send", Outcome::Proved)],
            vec![("send", Outcome::Failed)],
            vec![("send", Outcome::Failed)],
        ),
    );
}

/// An in-process Notification channel passes the same
/// `conformance/notification/cases.toml` cases as a process Plugin (ADR 0129).
///
/// Covers: story:7-96
#[test]
fn the_notification_suite_checks_in_process_pass_for_a_conforming_implementation() {
    let results: Vec<(String, Vec<CaseCheck>)> =
        PluginSuite::notification().checks_in_process(|method, params| {
            assert_eq!(method, "notification.send");
            assert_eq!(params["event"], json!("run_failed"));
            assert!(params["event_id"].is_string());
            assert!(params["text"].is_string());
            json!({ "result": { "delivered": true } })
        });
    let failed: Vec<_> = results
        .into_iter()
        .filter(|(_, checks)| checks.iter().any(|check| !check.passed))
        .collect();
    assert_eq!(failed, Vec::<(String, Vec<CaseCheck>)>::new());
}

/// A Notification channel Plugin manifest accepts all three network levels:
/// `none`, `proxy_tcp` and `direct` (ADR 0033; issue 1023).
///
/// Covers: story:7-96
#[test]
fn notification_channel_manifests_accept_all_three_network_levels() {
    for level in NetworkLevel::ALL {
        let manifest = PluginManifest::read(
            json!({
                "manifest_version": "0.1",
                "name": "pager",
                "version": "1.0.0",
                "kinds": ["notification"],
                "protocol": { "min": "0.1", "max": "0.1" },
                "entrypoint": "plugin",
                "network_level": level.as_str(),
            })
            .to_string()
            .as_bytes(),
        )
        .unwrap();
        assert_eq!(manifest.network_level, level);
    }
}
