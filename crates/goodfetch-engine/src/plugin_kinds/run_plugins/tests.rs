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
use std::sync::Arc;
use std::time::Duration;

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::run::RunId;
use goodfetch_protocol::settings::SettingsLayer;
use goodfetch_test_support::TestEnv;
use serde_json::{Value, json};

use super::{RunPlugins, StartError};
use crate::audit::AuditLog;
use crate::clock::Clock;
use crate::entropy::Entropy;
use crate::home::Home;
use crate::mask::Redaction;
use crate::sandbox_probe::{HostFacts, SandboxProbe, UserNamespace};
use crate::secrets::{Carried, Keychain, Secrets};
use crate::settings::{Layers, Values, resolve};
use crate::tasks::{BlockingPolicy, TaskOwner};

/// A host that refuses the user namespace, on a kernel whose Landlock ABI
/// 4 cannot confine a Plugin without one.
fn refusing_host() -> HostFacts {
    HostFacts {
        user_namespace: UserNamespace::Refused(1),
        landlock_abi: 4,
        ..HostFacts::NO_NAMESPACE_NEEDED
    }
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
        "settings_schema": {
            "type": "object",
            "properties": { "token": { "type": "string", "secret": true } },
        },
        "env": { "EXTRACT_TOKEN": "token" },
        "permissions": { "grants": ["secret_value"] },
    });
    std::fs::write(bundle.join("goodfetch-plugin.json"), manifest.to_string()).unwrap();
}

/// On a host that cannot confine a Plugin, its start is refused with
/// `confinement_unavailable` before any of its settings is read: the
/// reference of its secret field, which no variable values, is never
/// resolved.
///
/// Covers: story:7-27, error:confinement_unavailable
#[test]
fn a_host_that_cannot_confine_refuses_the_start_before_resolving_a_secret() {
    let env = TestEnv::new();
    let home = Home::new(env.home().to_path_buf());
    install(&home);
    let owner = Arc::new(TaskOwner::root(BlockingPolicy::Inline));
    let Value::Object(run) =
        json!({ "plugins": { "scripted": { "settings": { "token": "env:ABSENT" } } } })
    else {
        panic!("settings are an object");
    };
    let layers = Layers {
        run: Values::read(SettingsLayer::Run, &run, Product::Collie).unwrap(),
        ..Layers::default()
    };
    let resolved = resolve(&layers, Product::Collie, Product::Collie).unwrap();
    let probe = SandboxProbe::of(refusing_host, BTreeMap::new());

    let started = goodfetch_test_support::runtime().block_on(async {
        let secrets = Secrets::new(
            Carried {
                environment: BTreeMap::new(),
                keychain: Keychain::absent(),
                home: home.clone(),
                entropy: Entropy::seeded(env.seed()),
            },
            Arc::new(AuditLog::of(&home, None, Arc::clone(&owner))),
            Arc::clone(&owner),
            Clock::paused(env.start_time()),
        )
        .for_run_under(&resolved, Duration::from_secs(30), Redaction::On);
        let plugins = RunPlugins::new(
            &home,
            None,
            Product::Collie,
            RunId::parse("confined").unwrap(),
            (Arc::clone(&owner), Clock::paused(env.start_time())),
        );
        plugins
            .extractor("scripted", Some(&resolved), &secrets, &probe)
            .await
    });

    let code = match started {
        Ok(_) => "started",
        Err(StartError::Refused(refusal)) => refusal.code(),
        Err(StartError::Secret(_)) => "a secret resolved",
    };
    assert_eq!(code, "confinement_unavailable");
}
