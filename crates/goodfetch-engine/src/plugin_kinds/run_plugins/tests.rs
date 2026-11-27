//! A Run's process Plugins through [`RunPlugins`], on the paused `Clock`
//! and a Plugin store in the test's home.
#![expect(
    clippy::disallowed_methods,
    reason = "tests run on the test Clock and Entropy (ADR 0059) and write the bundle the store reads"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::sync::Arc;
use std::time::Duration;

use goodfetch_protocol::audit::AuditEntry;
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::plugin::approvals::PLUGIN_SECRET_VALUE_RELEASED;
use goodfetch_protocol::run::RunId;
use goodfetch_protocol::settings::SettingsLayer;
use goodfetch_test_support::TestEnv;
use rstest::rstest;
use serde_json::{Map, Value, json};

use super::{RunPlugins, StartError};
use crate::audit::AuditLog;
use crate::clock::Clock;
use crate::entropy::Entropy;
use crate::home::Home;
use crate::mask::Redaction;
use crate::sandbox_probe::{HostFacts, SandboxProbe, UserNamespace};
use crate::secrets::{Carried, Keychain, Secrets};
use crate::settings::{Layers, Policy, Values, resolve};
use crate::tasks::{BlockingPolicy, TaskOwner};

/// A visibly fake image index digest.
const DIGEST: &str = "sha256:0000000000000000000000000000000000000000000000000000000000000000";

/// A visibly fake value of the variable `TOKEN`.
const TOKEN: &str = "test-token";

/// A host that refuses the user namespace, on a kernel whose Landlock ABI
/// 4 cannot confine a Plugin without one.
fn refusing_host() -> HostFacts {
    HostFacts {
        user_namespace: UserNamespace::Refused(1),
        landlock_abi: 4,
        ..HostFacts::NO_NAMESPACE_NEEDED
    }
}

/// A host that confines a Plugin.
fn confining_host() -> HostFacts {
    HostFacts::NO_NAMESPACE_NEEDED
}

/// Installs the Extractor `scripted` in `home`'s Plugin store, whose
/// manifest values `EXTRACT_TOKEN` from its secret field `token`.
fn install(home: &Home) {
    let bundle = home.plugins().join("scripted").join("1.0.0");
    std::fs::create_dir_all(&bundle).unwrap();
    let manifest = json!({
        "manifest_version": "0.1",
        "name": "scripted",
        "version": "1.0.0",
        "kinds": ["extract"],
        "protocol": { "min": "0.1", "max": "0.1" },
        "entrypoint": "plugin",
        "network_level": "none",
        "settings_schema": {
            "type": "object",
            "properties": { "token": { "type": "string", "secret": true } },
        },
        "env": { "EXTRACT_TOKEN": "token" },
        "permissions": { "grants": ["secret_value"] },
    });
    std::fs::write(bundle.join("goodfetch-plugin.json"), manifest.to_string()).unwrap();
}

/// The settings `value` gives, an object.
fn object(value: Value) -> Map<String, Value> {
    let Value::Object(map) = value else {
        panic!("settings are an object");
    };
    map
}

/// Starts `scripted`, installed in `home`, under an Operator policy whose
/// `plugins` entry for it is `policy` (none when null), with the run's
/// `token` `env:TOKEN`, on `host`, in an engine whose environment values
/// `TOKEN`: what the start gives, and the Audit log's entries after it.
fn start(
    env: &TestEnv,
    home: &Home,
    policy: &Value,
    host: fn() -> HostFacts,
) -> (Result<(), StartError>, Vec<AuditEntry>) {
    let owner = Arc::new(TaskOwner::root(BlockingPolicy::Inline));
    let policy = if policy.is_null() {
        Map::new()
    } else {
        object(json!({ "plugins": { "scripted": policy } }))
    };
    let run =
        object(json!({ "plugins": { "scripted": { "settings": { "token": "env:TOKEN" } } } }));
    let layers = Layers {
        operator_policy: Policy {
            settings: Values::read(SettingsLayer::OperatorPolicy, &policy, Product::Collie)
                .unwrap(),
            ..Policy::default()
        },
        run: Values::read(SettingsLayer::Run, &run, Product::Collie).unwrap(),
        ..Layers::default()
    };
    let resolved = resolve(&layers, Product::Collie, Product::Collie).unwrap();
    let probe = SandboxProbe::of(host, BTreeMap::new());
    let audit = Arc::new(AuditLog::of(home, None, Arc::clone(&owner)));

    let started = goodfetch_test_support::runtime().block_on(async {
        let secrets = Secrets::new(
            Carried {
                environment: BTreeMap::from([(OsString::from("TOKEN"), OsString::from(TOKEN))]),
                keychain: Keychain::absent(),
                home: home.clone(),
                entropy: Entropy::seeded(env.seed()),
            },
            Arc::clone(&audit),
            Arc::clone(&owner),
            Clock::paused(env.start_time()),
        )
        .for_run_under(&resolved, Duration::from_secs(30), Redaction::On);
        let plugins = RunPlugins::new(
            home,
            None,
            Product::Collie,
            RunId::parse("confined").unwrap(),
            (Arc::clone(&owner), Clock::paused(env.start_time())),
        );
        plugins
            .extractor("scripted", Some(&resolved), &secrets, &probe)
            .await
            .map(drop)
    });
    let entries = audit
        .export()
        .unwrap()
        .into_iter()
        .map(|exported| exported.entry)
        .collect();
    (started, entries)
}

/// The code and the details of the refusal `started` holds.
fn refusal(started: Result<(), StartError>) -> (&'static str, Map<String, Value>) {
    match started {
        Ok(()) => ("started", Map::new()),
        Err(StartError::Refused(refusal)) => (
            refusal.code(),
            refusal.details().cloned().unwrap_or_default(),
        ),
        Err(StartError::Secret(_)) => ("a secret did not resolve", Map::new()),
    }
}

/// On a host that cannot confine a Plugin, the start of an allowed Plugin
/// is refused with `confinement_unavailable` before any of its settings is
/// read: the reference of its secret field is never resolved.
///
/// Covers: story:7-27, error:confinement_unavailable
#[test]
fn a_host_that_cannot_confine_refuses_the_start_before_resolving_a_secret() {
    let env = TestEnv::new();
    let home = Home::new(env.home().to_path_buf());
    install(&home);

    let policy = json!({ "allow": true, "approved": { "secret_values": ["env:TOKEN"] } });
    let (started, entries) = start(&env, &home, &policy, refusing_host);

    assert_eq!(refusal(started).0, "confinement_unavailable");
    assert!(entries.is_empty(), "{entries:?}");
}

/// A store Plugin, which is not first-party, loads only when the Operator
/// policy allows it at the digest it was added at: left out, denied or
/// pinned to another digest, its start is refused with
/// `plugin_not_allowed` naming the reason and the entry that lifts it,
/// before the host is probed.
///
/// Covers: story:7-16, story:7-17, error:plugin_not_allowed
#[rstest]
#[case::left_out(Value::Null, "not_allowed", "allow", None)]
#[case::denied(json!({ "allow": false }), "denied", "allow", None)]
#[case::pinned_elsewhere(
    json!({ "allow": true, "digest": DIGEST }),
    "digest",
    "digest",
    Some(json!(DIGEST))
)]
fn a_plugin_the_policy_does_not_allow_is_refused(
    #[case] policy: Value,
    #[case] reason: &str,
    #[case] entry: &str,
    #[case] pinned: Option<Value>,
) {
    let env = TestEnv::new();
    let home = Home::new(env.home().to_path_buf());
    install(&home);
    let found = "sha256:1111111111111111111111111111111111111111111111111111111111111111";
    std::fs::write(home.plugins().join("scripted").join("1.0.0.digest"), found).unwrap();

    let (started, _) = start(&env, &home, &policy, refusing_host);

    let (code, details) = refusal(started);
    assert_eq!(code, "plugin_not_allowed", "{details:?}");
    assert_eq!(details["reason"], json!(reason));
    assert!(
        details["fix"]
            .as_str()
            .unwrap()
            .contains(&format!("plugins.scripted.{entry}")),
        "{details:?}"
    );
    assert_eq!(details.get("pinned").cloned(), pinned);
    let found = pinned.map(|_| json!(found));
    assert_eq!(details.get("found").cloned(), found);
}

/// A Plugin the policy allows but whose Secret value it has not approved is
/// refused with `plugin_permission_denied`, naming the permission, and the
/// Secret's value is never resolved.
///
/// Covers: story:7-18, error:plugin_permission_denied
#[test]
fn a_secret_value_not_approved_is_refused() {
    let env = TestEnv::new();
    let home = Home::new(env.home().to_path_buf());
    install(&home);

    let (started, entries) = start(&env, &home, &json!({ "allow": true }), confining_host);

    let (code, details) = refusal(started);
    assert_eq!(code, "plugin_permission_denied", "{details:?}");
    assert_eq!(
        details["permission"],
        json!({ "permission": "secret_value", "reference": "env:TOKEN" })
    );
    assert!(
        details["fix"]
            .as_str()
            .unwrap()
            .contains("plugins.scripted.approved")
    );
    assert!(entries.is_empty(), "{entries:?}");
}

/// A Secret's value the policy approved is released into the Plugin's
/// environment, and the release is in the Audit log, naming the Plugin and
/// the reference and never the value.
///
/// Covers: story:7-18, story:7-20
#[test]
fn an_approved_secret_value_released_is_audited() {
    let env = TestEnv::new();
    let home = Home::new(env.home().to_path_buf());
    install(&home);

    let policy = json!({ "allow": true, "approved": { "secret_values": ["env:TOKEN"] } });
    let (_, entries) = start(&env, &home, &policy, confining_host);

    let released: Vec<_> = entries
        .iter()
        .filter(|entry| entry.kind == PLUGIN_SECRET_VALUE_RELEASED.name)
        .collect();
    assert_eq!(released.len(), 1, "{entries:?}");
    assert_eq!(
        released[0].details,
        json!({ "plugin": "scripted", "reference": "env:TOKEN" })
    );
    assert!(!format!("{entries:?}").contains(TOKEN));
}
