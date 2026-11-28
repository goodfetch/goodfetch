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
/// Covers: story:7-18, story:7-45, error:plugin_permission_denied
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
/// Covers: story:7-18, story:7-20, story:7-45
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

/// Installs a `notification` Plugin named `name` at `network_level: "direct"`,
/// with an `endpoint` setting and a secret field `aws_secret_access_key`
/// mapped to `AWS_SECRET_ACCESS_KEY`.
fn install_direct_plugin(home: &Home, name: &str) {
    let plugin_dir = home.plugins().join(name);
    let bundle = plugin_dir.join("1.0.0");
    std::fs::create_dir_all(&bundle).unwrap();
    let manifest = json!({
        "manifest_version": "0.1",
        "name": name,
        "version": "1.0.0",
        "kinds": ["notification"],
        "protocol": { "min": "0.1", "max": "0.1" },
        "entrypoint": "plugin",
        "network_level": "direct",
        "settings_schema": {
            "type": "object",
            "properties": {
                "endpoint": { "type": "string" },
                "aws_secret_access_key": { "type": "string", "secret": true },
            },
        },
        "env": { "AWS_SECRET_ACCESS_KEY": "aws_secret_access_key" },
        "permissions": { "grants": ["secret_value"] },
    });
    std::fs::write(bundle.join("goodfetch-plugin.json"), manifest.to_string()).unwrap();
    std::fs::write(plugin_dir.join("1.0.0.digest"), DIGEST).unwrap();
}

/// Persists a signed Plugin index in `home` registering `name` at [`DIGEST`]
/// with `origin`.
fn write_index_with_origin(
    home: &Home,
    name: &str,
    origin: goodfetch_protocol::plugin::approvals::Origin,
    now: jiff::Timestamp,
) {
    use goodfetch_protocol::plugin::PluginKindName;
    use goodfetch_protocol::plugin::index::{IndexEntry, PluginIndex, SignerIdentity};

    let (signer, trust) = crate::signed_set::SignedSetSigner::for_root(
        "direct-test-key",
        "https://token.actions.githubusercontent.com",
        "https://github.com/goodfetch/goodfetch/.github/workflows/plugin-index.yml@refs/heads/main",
        [0x42; 32],
    );
    let expires_at = now
        .checked_add(jiff::Span::new().hours(24))
        .expect("invariant: valid test expiry");
    let index = PluginIndex {
        version: 1,
        signed_at: now.to_string(),
        expires_at: expires_at.to_string(),
        plugins: vec![IndexEntry {
            name: name.to_owned(),
            reference: format!("ghcr.io/goodfetch/goodfetch-plugin-{name}"),
            digest: DIGEST.to_owned(),
            kinds: vec![PluginKindName::NOTIFICATION],
            signer: SignerIdentity {
                issuer: "https://token.actions.githubusercontent.com".to_owned(),
                workflow:
                    "https://github.com/goodfetch/goodfetch/.github/workflows/release.yml@refs/tags/v1.0.0"
                        .to_owned(),
                repository: "github.com/goodfetch/goodfetch".to_owned(),
            },
            origin,
            conformance: Vec::new(),
        }],
        revocations: Vec::new(),
    };
    let bundle = signer.sign(index).unwrap();
    crate::plugin_index::PluginIndexStore::new(home.plugin_index())
        .apply(bundle, &trust, now)
        .unwrap();
}

/// Starts a `direct` notification Plugin `name` through [`RunPlugins`] with
/// real [`AuditLog`], [`PluginRunRecorder`], and [`Places`].
fn start_direct(
    env: &TestEnv,
    home: &Home,
    name: &str,
    operator_policy: Value,
    user_config: Value,
    run_settings: Value,
    env_vars: BTreeMap<OsString, OsString>,
) -> (
    Result<(), StartError>,
    Vec<AuditEntry>,
    Option<Vec<goodfetch_protocol::run::PluginRun>>,
) {
    use goodfetch_confinement::deny_list::Places;
    use goodfetch_protocol::run::PluginRun;

    struct RecordingPlugins(tokio::sync::Mutex<Vec<PluginRun>>);

    impl super::PluginRunRecorder for RecordingPlugins {
        fn record(&self, plugin: PluginRun) -> futures_util::future::BoxFuture<'_, ()> {
            Box::pin(async move {
                self.0.lock().await.push(plugin);
            })
        }
    }

    let owner = Arc::new(TaskOwner::root(BlockingPolicy::Inline));
    let layers = Layers {
        operator_policy: Policy {
            settings: Values::read(
                SettingsLayer::OperatorPolicy,
                &object(operator_policy),
                Product::Collie,
            )
            .unwrap(),
            ..Policy::default()
        },
        user_configuration: Values::read(
            SettingsLayer::UserConfiguration,
            &object(user_config),
            Product::Collie,
        )
        .unwrap(),
        run: Values::read(SettingsLayer::Run, &object(run_settings), Product::Collie).unwrap(),
        ..Layers::default()
    };
    let resolved = resolve(&layers, Product::Collie, Product::Collie).unwrap();
    let probe = SandboxProbe::of(confining_host, BTreeMap::new());
    let audit = Arc::new(AuditLog::of(home, None, Arc::clone(&owner)));
    let places = Arc::new(Places {
        home: env.folder().to_path_buf(),
        goodfetch_home: home.root().to_path_buf(),
        app_data: None,
        local_app_data: None,
    });
    let run_id = RunId::parse("direct-run").unwrap();
    let recorder = Arc::new(RecordingPlugins(tokio::sync::Mutex::new(Vec::new())));

    let (started, report_plugins) = goodfetch_test_support::runtime().block_on(async {
        let secrets = Secrets::new(
            Carried {
                environment: env_vars,
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
            Some(places),
            Product::Collie,
            run_id,
            (Arc::clone(&owner), Clock::paused(env.start_time())),
        )
        .with_audit((*audit).clone())
        .with_run_recorder(Arc::clone(&recorder) as Arc<dyn super::PluginRunRecorder>);
        let started = plugins
            .notification_channel(name, "alerts", Some(&resolved), &secrets, &probe)
            .await
            .map(drop);
        let plugins_list = recorder.0.lock().await.clone();
        let report_plugins = (!plugins_list.is_empty()).then_some(plugins_list);
        (started, report_plugins)
    });
    let entries = audit
        .export()
        .unwrap()
        .into_iter()
        .map(|exported| exported.entry)
        .collect();
    (started, entries, report_plugins)
}

/// A third-party Plugin declaring `network_level = "direct"` is refused with
/// `plugin_permission_denied` naming `Permission::Direct` unless Operator
/// policy explicitly approves `direct`.
///
/// Covers: story:7-56, error:plugin_permission_denied
#[test]
fn a_third_party_direct_plugin_is_refused_unless_operator_policy_approves_direct() {
    let env = TestEnv::new();
    let home = Home::new(env.home().to_path_buf());
    install_direct_plugin(&home, "pager");

    let (started, entries, report_plugins) = start_direct(
        &env,
        &home,
        "pager",
        json!({ "plugins": { "pager": { "allow": true } } }),
        json!({}),
        json!({}),
        BTreeMap::new(),
    );

    let (code, details) = refusal(started);
    assert_eq!(code, "plugin_permission_denied", "{details:?}");
    assert_eq!(details["permission"], json!({ "permission": "direct" }));
    assert!(entries.is_empty(), "{entries:?}");
    assert_eq!(report_plugins, None);
}

/// Every configured destination on a `direct` Plugin is checked under ADR 0017
/// before starting (and before resolving any Secrets): cloud metadata,
/// hostnames resolving to metadata, private addresses under
/// `egress.no_private_endpoints`, and unconfigured Run-layer private hosts for
/// a third-party Plugin all stop the start with `egress_refused`.
///
/// Covers: story:7-111, error:egress_refused
#[rstest]
#[case::cloud_metadata_literal(
    json!({ "plugins": { "pager": { "allow": true, "approved": { "direct": true } } } }),
    json!({ "plugins": { "pager": { "settings": { "endpoint": "http://169.254.169.254/latest/meta-data", "aws_secret_access_key": "env:AWS_KEY" } } } }),
    json!({}),
)]
#[case::cloud_metadata_postgres(
    json!({ "plugins": { "pager": { "allow": true, "approved": { "direct": true } } } }),
    json!({ "plugins": { "pager": { "settings": { "endpoint": "postgres://user:pass@169.254.169.254:5432/db", "aws_secret_access_key": "env:AWS_KEY" } } } }),
    json!({}),
)]
#[case::resolved_to_cloud_metadata(
    json!({ "plugins": { "pager": { "allow": true, "approved": { "direct": true } } } }),
    json!({
        "network": { "resolve": { "meta.example": ["169.254.169.254"] } },
        "plugins": { "pager": { "settings": { "endpoint": "https://meta.example/hook", "aws_secret_access_key": "env:AWS_KEY" } } },
    }),
    json!({}),
)]
#[case::private_endpoint_under_no_private_endpoints(
    json!({
        "egress": { "no_private_endpoints": true },
        "plugins": { "pager": { "allow": true, "approved": { "direct": true } } },
    }),
    json!({ "plugins": { "pager": { "settings": { "endpoint": "postgres://user:pass@127.0.0.1:5432/db", "aws_secret_access_key": "env:AWS_KEY" } } } }),
    json!({}),
)]
#[case::third_party_run_layer_loopback(
    json!({ "plugins": { "pager": { "allow": true, "approved": { "direct": true } } } }),
    json!({}),
    json!({ "plugins": { "pager": { "settings": { "endpoint": "http://127.0.0.1:8080/hook", "aws_secret_access_key": "env:AWS_KEY" } } } }),
)]
fn a_direct_plugin_with_a_disallowed_destination_stops_the_start_before_secrets(
    #[case] operator_policy: Value,
    #[case] user_config: Value,
    #[case] run_settings: Value,
) {
    let env = TestEnv::new();
    let home = Home::new(env.home().to_path_buf());
    install_direct_plugin(&home, "pager");

    let (started, entries, report_plugins) = start_direct(
        &env,
        &home,
        "pager",
        operator_policy,
        user_config,
        run_settings,
        BTreeMap::from([(OsString::from("AWS_KEY"), OsString::from("fake-aws-secret"))]),
    );

    let (code, details) = refusal(started);
    assert_eq!(code, "egress_refused", "{details:?}");
    assert!(entries.is_empty(), "{entries:?}");
    assert_eq!(report_plugins, None);
}

/// A first-party Plugin declaring `direct` runs by default without Operator
/// policy naming it, receives cloud credentials from the engine as Secrets
/// (audited and masked), and is marked at `direct` in both the Audit log and
/// the Run report.
///
/// Covers: story:7-56, story:7-111
#[test]
fn a_first_party_direct_plugin_runs_by_default_receives_cloud_credentials_as_secrets_and_is_marked()
{
    use goodfetch_protocol::plugin::approvals::{Origin, PLUGIN_DIRECT_STARTED};
    use goodfetch_protocol::run::PluginRun;

    let env = TestEnv::new();
    let home = Home::new(env.home().to_path_buf());
    install_direct_plugin(&home, "cloud-notify");
    write_index_with_origin(
        &home,
        "cloud-notify",
        Origin::FirstParty,
        jiff::Timestamp::try_from(env.start_time()).unwrap(),
    );

    let cloud_secret = "fake-cloud-credential-value";
    let (started, entries, report_plugins) = start_direct(
        &env,
        &home,
        "cloud-notify",
        json!({}),
        json!({
            "network": { "resolve": { "notify.example": ["93.184.216.34"] } },
            "plugins": {
                "cloud-notify": {
                    "settings": {
                        "endpoint": "https://notify.example/send",
                        "aws_secret_access_key": "env:AWS_SECRET_ACCESS_KEY",
                    }
                }
            }
        }),
        json!({}),
        BTreeMap::from([(
            OsString::from("AWS_SECRET_ACCESS_KEY"),
            OsString::from(cloud_secret),
        )]),
    );

    assert!(started.is_ok(), "{started:?}");
    let released: Vec<_> = entries
        .iter()
        .filter(|entry| entry.kind == PLUGIN_SECRET_VALUE_RELEASED.name)
        .collect();
    assert_eq!(released.len(), 1, "{entries:?}");
    assert_eq!(
        released[0].details,
        json!({ "plugin": "cloud-notify", "reference": "env:AWS_SECRET_ACCESS_KEY" })
    );
    assert!(!format!("{entries:?}").contains(cloud_secret));

    let direct_entries: Vec<_> = entries
        .iter()
        .filter(|entry| entry.kind == PLUGIN_DIRECT_STARTED.name)
        .collect();
    assert_eq!(direct_entries.len(), 1, "{entries:?}");
    assert_eq!(
        direct_entries[0].details,
        json!({
            "plugin": "cloud-notify",
            "origin": "first_party",
            "network_level": "direct",
            "digest": DIGEST,
        })
    );
    assert_eq!(
        report_plugins,
        Some(vec![PluginRun {
            name: "cloud-notify".to_owned(),
            form: goodfetch_protocol::run::PluginForm::Process,
            origin: Some(Origin::FirstParty),
            digest: Some(DIGEST.to_owned()),
            network_level: "direct".to_owned(),
            confinement: goodfetch_protocol::run::PluginConfinement::Full,
            confinement_missing: Vec::new(),
        }])
    );
}

/// An Operator-approved third-party Plugin at `direct` starts and is marked at
/// `direct` in both the Audit log and the Run report.
///
/// Covers: story:7-56
#[test]
fn an_approved_third_party_direct_plugin_is_marked_in_audit_and_run_report() {
    use goodfetch_protocol::plugin::approvals::PLUGIN_DIRECT_STARTED;
    use goodfetch_protocol::run::PluginRun;

    let env = TestEnv::new();
    let home = Home::new(env.home().to_path_buf());
    install_direct_plugin(&home, "pager");

    let (started, entries, report_plugins) = start_direct(
        &env,
        &home,
        "pager",
        json!({
            "plugins": {
                "pager": {
                    "allow": true,
                    "approved": { "direct": true }
                }
            }
        }),
        json!({
            "network": { "resolve": { "pager.example": ["93.184.216.34"] } },
            "plugins": {
                "pager": {
                    "settings": { "endpoint": "https://pager.example/events" }
                }
            }
        }),
        json!({}),
        BTreeMap::new(),
    );

    assert!(started.is_ok(), "{started:?}");
    let direct_entries: Vec<_> = entries
        .iter()
        .filter(|entry| entry.kind == PLUGIN_DIRECT_STARTED.name)
        .collect();
    assert_eq!(direct_entries.len(), 1, "{entries:?}");
    assert_eq!(
        direct_entries[0].details,
        json!({
            "plugin": "pager",
            "origin": "third_party",
            "network_level": "direct",
            "digest": DIGEST,
        })
    );
    assert_eq!(
        report_plugins,
        Some(vec![PluginRun {
            name: "pager".to_owned(),
            form: goodfetch_protocol::run::PluginForm::Process,
            origin: Some(goodfetch_protocol::plugin::approvals::Origin::ThirdParty),
            digest: Some(DIGEST.to_owned()),
            network_level: "direct".to_owned(),
            confinement: goodfetch_protocol::run::PluginConfinement::Full,
            confinement_missing: Vec::new(),
        }])
    );
}

fn unconfining_host() -> goodfetch_confinement::host::HostFacts {
    goodfetch_confinement::host::HostFacts {
        user_namespace: goodfetch_confinement::host::UserNamespace::Refused(1),
        seccomp_filtered: true,
        apparmor_restricts: false,
        landlock_abi: 4,
        wsl1: false,
    }
}

fn start_on_unconfining_host(
    env: &TestEnv,
    home: &Home,
    audit: &Arc<AuditLog>,
    owner: &Arc<TaskOwner>,
    plugin_name: &str,
    operator_policy: Value,
) -> (
    Result<(), StartError>,
    Vec<goodfetch_protocol::run::PluginRun>,
) {
    use goodfetch_confinement::deny_list::Places;
    use goodfetch_protocol::run::PluginRun;

    struct RecordingPlugins(tokio::sync::Mutex<Vec<PluginRun>>);

    impl super::PluginRunRecorder for RecordingPlugins {
        fn record(&self, plugin: PluginRun) -> futures_util::future::BoxFuture<'_, ()> {
            Box::pin(async move {
                self.0.lock().await.push(plugin);
            })
        }
    }

    let layers = Layers {
        operator_policy: Policy {
            settings: Values::read(
                SettingsLayer::OperatorPolicy,
                &object(operator_policy),
                Product::Collie,
            )
            .unwrap(),
            ..Policy::default()
        },
        user_configuration: Values::read(
            SettingsLayer::UserConfiguration,
            &object(json!({
                "network": { "resolve": { "notify.example": ["93.184.216.34"] } },
                "plugins": {
                    plugin_name: {
                        "settings": { "endpoint": "https://notify.example/send" }
                    }
                }
            })),
            Product::Collie,
        )
        .unwrap(),
        ..Layers::default()
    };
    let resolved = resolve(&layers, Product::Collie, Product::Collie).unwrap();
    let probe = SandboxProbe::of(unconfining_host, BTreeMap::new());
    let places = Arc::new(Places {
        home: env.folder().to_path_buf(),
        goodfetch_home: home.root().to_path_buf(),
        app_data: None,
        local_app_data: None,
    });
    let recorder = Arc::new(RecordingPlugins(tokio::sync::Mutex::new(Vec::new())));
    let run_id = RunId::parse(&format!("reduced-{plugin_name}")).unwrap();
    goodfetch_test_support::runtime().block_on(async {
        let secrets = Secrets::new(
            Carried {
                environment: BTreeMap::new(),
                keychain: Keychain::absent(),
                home: home.clone(),
                entropy: Entropy::seeded(env.seed()),
            },
            Arc::clone(audit),
            Arc::clone(owner),
            Clock::paused(env.start_time()),
        )
        .for_run_under(&resolved, Duration::from_secs(30), Redaction::On);
        let plugins = RunPlugins::new(
            home,
            Some(places),
            Product::Collie,
            run_id,
            (Arc::clone(owner), Clock::paused(env.start_time())),
        )
        .with_audit((**audit).clone())
        .with_run_recorder(Arc::clone(&recorder) as Arc<dyn super::PluginRunRecorder>);
        let started = plugins
            .notification_channel(plugin_name, "alerts", Some(&resolved), &secrets, &probe)
            .await
            .map(drop);
        let entries = recorder.0.lock().await.clone();
        (started, entries)
    })
}

/// On a host that cannot confine a process Plugin, third-party Plugins are
/// always refused with `confinement_unavailable` (even with
/// `reduced_confinement: true`), first-party Plugins are refused by default,
/// and first-party Plugins with Operator policy `reduced_confinement: true`
/// start, audit `plugin_reduced_confinement`, and record `confinement: reduced`
/// with the missing confinement parts in `run_plugins`.
///
/// Covers: story:7-25, story:7-21, error:confinement_unavailable
#[test]
fn reduced_confinement_allows_only_first_party_and_reports_what_was_not_confined() {
    use goodfetch_protocol::plugin::approvals::Origin;

    let env = TestEnv::new();
    let home = Home::new(env.home().to_path_buf());
    install_direct_plugin(&home, "first-party-reduced");
    install_direct_plugin(&home, "third-party-reduced");
    write_index_with_origin(
        &home,
        "first-party-reduced",
        Origin::FirstParty,
        jiff::Timestamp::try_from(env.start_time()).unwrap(),
    );

    // Non-policy layer setting `reduced_confinement` is refused on resolve as `setting_not_applicable`.
    let bad_layers = Layers {
        user_configuration: Values::read(
            SettingsLayer::UserConfiguration,
            &object(json!({
                "plugins": { "first-party-reduced": { "reduced_confinement": true } }
            })),
            Product::Collie,
        )
        .unwrap(),
        ..Layers::default()
    };
    assert!(
        matches!(
            resolve(&bad_layers, Product::Collie, Product::Collie),
            Err(crate::settings::SettingsError::NotApplicable { ref key, .. })
                if key == "plugins.first-party-reduced.reduced_confinement"
        ),
        "user configuration must not set reduced_confinement"
    );

    let owner = Arc::new(TaskOwner::root(BlockingPolicy::Inline));
    let audit = Arc::new(AuditLog::of(&home, None, Arc::clone(&owner)));

    // 1. Third-party Plugin is refused even if Operator policy sets `reduced_confinement: true`.
    let (third_started, third_entries) = start_on_unconfining_host(
        &env,
        &home,
        &audit,
        &owner,
        "third-party-reduced",
        json!({
            "plugins": {
                "third-party-reduced": {
                    "allow": true,
                    "approved": { "direct": true },
                    "reduced_confinement": true,
                }
            }
        }),
    );
    let (code, _) = refusal(third_started);
    assert_eq!(code, "confinement_unavailable");
    assert_eq!(third_entries, Vec::new());

    // 2. First-party Plugin is refused by default without `reduced_confinement: true`.
    let (default_started, default_entries) = start_on_unconfining_host(
        &env,
        &home,
        &audit,
        &owner,
        "first-party-reduced",
        json!({}),
    );
    let (code, _) = refusal(default_started);
    assert_eq!(code, "confinement_unavailable");
    assert_eq!(default_entries, Vec::new());

    let (allowed_started, allowed_entries) = start_on_unconfining_host(
        &env,
        &home,
        &audit,
        &owner,
        "first-party-reduced",
        json!({ "plugins": { "first-party-reduced": { "reduced_confinement": true } } }),
    );
    assert!(allowed_started.is_ok(), "{allowed_started:?}");
    assert_reduced_confinement_recorded(&audit, &allowed_entries);
}

fn assert_reduced_confinement_recorded(
    audit: &AuditLog,
    allowed_entries: &[goodfetch_protocol::run::PluginRun],
) {
    use goodfetch_protocol::plugin::approvals::{Origin, PLUGIN_REDUCED_CONFINEMENT};
    use goodfetch_protocol::run::{PluginConfinement, PluginForm, PluginRun};

    assert_eq!(
        allowed_entries,
        &[PluginRun {
            name: "first-party-reduced".to_owned(),
            form: PluginForm::Process,
            origin: Some(Origin::FirstParty),
            digest: Some(DIGEST.to_owned()),
            network_level: "direct".to_owned(),
            confinement: PluginConfinement::Reduced,
            confinement_missing: vec!["signal_scope".to_owned(), "user_namespace".to_owned()],
        }]
    );
    let audit_entries: Vec<_> = audit
        .export()
        .unwrap()
        .into_iter()
        .map(|exported| exported.entry)
        .filter(|entry| entry.kind == PLUGIN_REDUCED_CONFINEMENT.name)
        .collect();
    assert_eq!(audit_entries.len(), 1, "{audit_entries:?}");
    assert_eq!(
        audit_entries[0].details,
        json!({
            "plugin": "first-party-reduced",
            "origin": "first_party",
            "network_level": "direct",
            "digest": DIGEST,
            "confinement_missing": ["signal_scope", "user_namespace"],
        })
    );
}
