//! A Plugin's language runtime fetched by digest at `plugin add`, pinned by
//! the installed Plugins that declare it, and refused without downloading
//! under `egress.offline` when missing at start (ADRs 0033, 0043, 0098 and
//! 0115; issue 895).
#![expect(
    clippy::disallowed_methods,
    reason = "the tests build tar archives, set up store bundles and run on the test Clock and Entropy (ADR 0059)"
)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use goodfetch_engine::audit::AuditLog;
use goodfetch_engine::browser_cache::{
    BrowserCache, CacheLocation, CleanUp, MaxUnusedAge, PinnedRuntime, kept_artifacts,
};
use goodfetch_engine::clock::Clock;
use goodfetch_engine::entropy::Entropy;
use goodfetch_engine::home::Home;
use goodfetch_engine::mask::Redaction;
use goodfetch_engine::pinned_download::Digest;
use goodfetch_engine::plugin_add;
use goodfetch_engine::plugin_kinds::run_plugins::{RunPlugins, StartError};
use goodfetch_engine::plugin_oci::TAR_GZIP_MEDIA_TYPE;
use goodfetch_engine::plugin_runtime::{
    RuntimeCatalog, RuntimeEntry, RuntimePlatform, artifact_id,
};
use goodfetch_engine::plugin_store::PluginStore;
use goodfetch_engine::sandbox_probe::SandboxProbe;
use goodfetch_engine::secrets::{Carried, Keychain, Secrets};
use goodfetch_engine::settings::{Layers, Policy, Values, resolve};
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::plugin::Runtime;
use goodfetch_protocol::plugin::lockfile::LOCKFILE_NAME;
use goodfetch_protocol::run::RunId;
use goodfetch_protocol::settings::SettingsLayer;
use goodfetch_test_support::TestEnv;
use hyper::StatusCode;
use serde_json::{Map, Value, json};

use crate::downloads::StandIn;
use crate::plugin_install::{Artifact, gzip, store};

/// The runtime path served by the loopback stand-in HTTPS server.
const RUNTIME_PATH: &str = "/runtimes/node-22.11.0.tar.gz";

/// Builds a tar archive from `(path, bytes, mode)` entries.
fn tar(entries: &[(&str, &[u8], u32)]) -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());
    for (path, bytes, mode) in entries {
        let mut header = tar::Header::new_gnu();
        header.set_size(u64::try_from(bytes.len()).unwrap());
        header.set_mode(*mode);
        header.set_entry_type(tar::EntryType::Regular);
        builder.append_data(&mut header, path, *bytes).unwrap();
    }
    builder.into_inner().unwrap()
}

/// A Plugin bundle declaring `runtime: { kind: "node", version: "22.11.0" }`.
fn node_plugin_artifact() -> Artifact {
    let manifest = json!({
        "manifest_version": "0.1",
        "name": "kettle",
        "version": "1.0.0",
        "kinds": ["extract"],
        "protocol": { "min": "0.1", "max": "0.1" },
        "entrypoint": "index.js",
        "runtime": { "kind": "node", "version": "22.11.0" },
        "network_level": "none",
    })
    .to_string();
    let archive = gzip(&tar(&[
        ("goodfetch-plugin.json", manifest.as_bytes(), 0o644),
        ("index.js", b"// plugin entrypoint\n", 0o644),
    ]));
    Artifact::of(archive, TAR_GZIP_MEDIA_TYPE)
}

/// A gzipped tar archive holding a toy `bin/node` runtime executable.
fn runtime_archive() -> Vec<u8> {
    gzip(&tar(&[("bin/node", b"#!/bin/sh\nexit 0\n", 0o755)]))
}

/// Builds a [`RuntimeCatalog`] pinning `node 22.11.0` on the host platform
/// to `stand_in`'s [`RUNTIME_PATH`] with `archive_digest`.
fn catalog_for(stand_in: &StandIn, archive_digest: Digest) -> RuntimeCatalog {
    let runtime = Runtime::Node {
        version: "22.11.0".to_owned(),
    };
    let platform = RuntimePlatform::host();
    let id = artifact_id(&runtime, platform).unwrap();
    RuntimeCatalog::empty().with_entry(
        &runtime,
        platform,
        RuntimeEntry {
            pin: PinnedRuntime {
                id,
                url: format!("https://{}{RUNTIME_PATH}", stand_in.authority()),
                archive_digest,
            },
            executable: "bin/node".to_owned(),
        },
    )
}

/// `plugin add` of a Plugin declaring a runtime downloads it by digest from
/// the local HTTPS server the pinned host resolves to, checks its digest,
/// stores it in the runtime cache, and keeps it pinned across cache
/// clean-up.
///
/// Covers: story:7-4
#[test]
fn plugin_add_downloads_verifies_and_caches_declared_runtime() {
    let env = TestEnv::new();
    let artifact = node_plugin_artifact();
    let runtime_bytes = runtime_archive();
    let runtime_digest = Digest::of(&runtime_bytes);
    let lockfile = env.home().join("project").join(LOCKFILE_NAME);
    let plugin_store = PluginStore::new(store(&env));
    let blocking = TaskOwner::root(BlockingPolicy::Inline).blocking_threads();
    let cache = BrowserCache::new(&CacheLocation::new(env.home(), None), blocking.clone());
    let now = Clock::system().timestamp();

    let (added, requests, clean_report) = goodfetch_test_support::runtime().block_on(async {
        let oci_answer = artifact.answer(artifact.clone());
        let served_runtime = runtime_bytes.clone();
        let mut stand_in = StandIn::serve_with(move |path, headers| {
            if path == RUNTIME_PATH {
                (StatusCode::OK, served_runtime.clone())
            } else {
                oci_answer(path, headers)
            }
        })
        .await;
        let reference = artifact.reference(&stand_in);
        let fetching = stand_in.fetching();
        let catalog = catalog_for(&stand_in, runtime_digest);
        let added = plugin_add::add_with_runtime(
            &reference,
            &fetching,
            &blocking,
            &plugin_store,
            &lockfile,
            &cache,
            &catalog,
            now,
        )
        .await;
        let requests = stand_in.requests();

        // Clean-up after max_unused_age keeps the runtime pinned by the installed Plugin.
        let declared = plugin_store
            .declared_runtimes(RuntimePlatform::host().as_str())
            .unwrap();
        let later = jiff::Timestamp::from_second(now.as_second() + 40 * 24 * 3600).unwrap();
        let kept = kept_artifacts(&[], &declared, later, MaxUnusedAge::default());
        let clean_report = cache
            .clean_up(&CleanUp {
                kept,
                now: later,
                max_unused_age: MaxUnusedAge::DEFAULT,
                dry_run: false,
            })
            .await
            .unwrap();
        (added, requests, clean_report)
    });

    assert_eq!(added.unwrap().name, "kettle");
    assert!(
        requests.iter().any(|req| req.path == RUNTIME_PATH),
        "{requests:?}"
    );
    let runtime_id = artifact_id(
        &Runtime::Node {
            version: "22.11.0".to_owned(),
        },
        RuntimePlatform::host(),
    )
    .unwrap();
    let cached_node = env
        .home()
        .join("browsers")
        .join("plugin_runtime")
        .join(&runtime_id.name)
        .join(&runtime_id.build)
        .join("files")
        .join("bin")
        .join("node");
    assert!(cached_node.is_file(), "{}", cached_node.display());
    assert!(
        clean_report.deleted.is_empty(),
        "pinned runtime must not be cleaned up: {:?}",
        clean_report.deleted
    );
}

/// A runtime whose downloaded bytes do not match the pinned digest is
/// refused with `artifact_download_failed` (`digest_mismatch`), leaving
/// neither the Plugin store nor the lockfile modified.
///
/// Covers: story:7-4
#[test]
fn plugin_add_refuses_runtime_with_wrong_digest() {
    let env = TestEnv::new();
    let artifact = node_plugin_artifact();
    let expected_digest = Digest::of(&runtime_archive());
    let lockfile = env.home().join("project").join(LOCKFILE_NAME);
    let plugin_store = PluginStore::new(store(&env));
    let blocking = TaskOwner::root(BlockingPolicy::Inline).blocking_threads();
    let cache = BrowserCache::new(&CacheLocation::new(env.home(), None), blocking.clone());
    let now = Clock::system().timestamp();

    let error = goodfetch_test_support::runtime().block_on(async {
        let oci_answer = artifact.answer(artifact.clone());
        let stand_in = StandIn::serve_with(move |path, headers| {
            if path == RUNTIME_PATH {
                (StatusCode::OK, b"tampered runtime archive".to_vec())
            } else {
                oci_answer(path, headers)
            }
        })
        .await;
        let reference = artifact.reference(&stand_in);
        let fetching = stand_in.fetching();
        let catalog = catalog_for(&stand_in, expected_digest);
        plugin_add::add_with_runtime(
            &reference,
            &fetching,
            &blocking,
            &plugin_store,
            &lockfile,
            &cache,
            &catalog,
            now,
        )
        .await
        .unwrap_err()
    });

    assert_eq!(error.code(), "artifact_download_failed");
    let details = error.details().unwrap();
    assert_eq!(details["artifact"]["kind"], json!("plugin_runtime"));
    assert_eq!(details["artifact"]["name"], json!("node"));
    assert_eq!(details["failure"], json!("digest_mismatch"));
    assert_eq!(
        details["pinned_by"],
        json!([{ "kind": "plugin", "name": "kettle", "version": "1.0.0" }])
    );
    assert!(!lockfile.exists());
    assert!(plugin_store.installed_all().unwrap().is_empty());
}

/// A failed runtime download answers `artifact_download_failed` with its
/// `failure` detail (`http_status`), naming the artifact and the Plugin
/// pinning it.
///
/// Covers: story:7-4
#[test]
fn plugin_add_reports_failed_runtime_download_with_failure_detail() {
    let env = TestEnv::new();
    let artifact = node_plugin_artifact();
    let expected_digest = Digest::of(&runtime_archive());
    let lockfile = env.home().join("project").join(LOCKFILE_NAME);
    let plugin_store = PluginStore::new(store(&env));
    let blocking = TaskOwner::root(BlockingPolicy::Inline).blocking_threads();
    let cache = BrowserCache::new(&CacheLocation::new(env.home(), None), blocking.clone());
    let now = Clock::system().timestamp();

    let error = goodfetch_test_support::runtime().block_on(async {
        let oci_answer = artifact.answer(artifact.clone());
        let stand_in = StandIn::serve_with(move |path, headers| {
            if path == RUNTIME_PATH {
                (StatusCode::SERVICE_UNAVAILABLE, Vec::new())
            } else {
                oci_answer(path, headers)
            }
        })
        .await;
        let reference = artifact.reference(&stand_in);
        let fetching = stand_in.fetching();
        let catalog = catalog_for(&stand_in, expected_digest);
        plugin_add::add_with_runtime(
            &reference,
            &fetching,
            &blocking,
            &plugin_store,
            &lockfile,
            &cache,
            &catalog,
            now,
        )
        .await
        .unwrap_err()
    });

    assert_eq!(error.code(), "artifact_download_failed");
    let details = error.details().unwrap();
    assert_eq!(details["artifact"]["kind"], json!("plugin_runtime"));
    assert_eq!(details["failure"], json!("http_status"));
    assert_eq!(details["status"], json!(503));
    assert_eq!(
        details["pinned_by"],
        json!([{ "kind": "plugin", "name": "kettle", "version": "1.0.0" }])
    );
}

/// Under `egress.offline`, starting a Plugin whose declared language
/// runtime is missing from the cache fails with
/// `artifact_unavailable_offline`, naming the artifact and every installed
/// Plugin that declares it, and attempts no download.
///
/// Covers: story:7-4
#[test]
fn offline_start_with_missing_runtime_names_artifact_and_pinners_without_downloading() {
    let env = TestEnv::new();
    let home = Home::new(env.home().to_path_buf());
    for (name, version) in [("kettle", "1.0.0"), ("teapot", "2.0.0")] {
        let bundle = home.plugins().join(name).join(version);
        std::fs::create_dir_all(&bundle).unwrap();
        let manifest = json!({
            "manifest_version": "0.1",
            "name": name,
            "version": version,
            "kinds": ["extract"],
            "protocol": { "min": "0.1", "max": "0.1" },
            "entrypoint": "index.js",
            "runtime": { "kind": "node", "version": "22.11.0" },
            "network_level": "none",
        });
        std::fs::write(bundle.join("goodfetch-plugin.json"), manifest.to_string()).unwrap();
    }

    let owner = Arc::new(TaskOwner::root(BlockingPolicy::Inline));
    let Value::Object(policy_settings) = json!({ "plugins": { "kettle": { "allow": true } } })
    else {
        unreachable!();
    };
    let Value::Object(user_settings) = json!({ "egress": { "offline": true } }) else {
        unreachable!();
    };
    let layers = Layers {
        operator_policy: Policy {
            settings: Values::read(
                SettingsLayer::OperatorPolicy,
                &policy_settings,
                Product::Collie,
            )
            .unwrap(),
            ..Policy::default()
        },
        user_configuration: Values::read(
            SettingsLayer::UserConfiguration,
            &user_settings,
            Product::Collie,
        )
        .unwrap(),
        ..Layers::default()
    };
    let resolved = resolve(&layers, Product::Collie, Product::Collie).unwrap();
    let probe = SandboxProbe::for_this_host(BTreeMap::new());
    let audit = Arc::new(AuditLog::of(&home, None, Arc::clone(&owner)));

    let started = goodfetch_test_support::runtime().block_on(async {
        let mut stand_in = StandIn::serve(BTreeMap::new()).await;
        let secrets = Secrets::new(
            Carried {
                environment: BTreeMap::new(),
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
            &home,
            None,
            Product::Collie,
            RunId::parse("offline-runtime").unwrap(),
            (Arc::clone(&owner), Clock::paused(env.start_time())),
        );
        let outcome = plugins
            .extractor("kettle", Some(&resolved), &secrets, &probe)
            .await
            .map(drop);
        let requests = stand_in.requests();
        (outcome, requests)
    });

    let (Err(StartError::Refused(error)), requests) = started else {
        panic!("expected StartError::Refused");
    };
    assert_eq!(error.code(), "artifact_unavailable_offline");
    assert_eq!(requests, []);
    let details: Map<String, Value> = error.details().cloned().unwrap_or_default();
    let expected_build = format!("22.11.0-{}", RuntimePlatform::host().as_str());
    assert_eq!(
        details["artifact"],
        json!({
            "kind": "plugin_runtime",
            "name": "node",
            "build": expected_build,
        })
    );
    assert_eq!(
        details["pinned_by"],
        json!([
            { "kind": "plugin", "name": "kettle", "version": "1.0.0" },
            { "kind": "plugin", "name": "teapot", "version": "2.0.0" },
        ])
    );
    assert_eq!(details["limit"]["key"], json!("egress.offline"));
    assert_eq!(details["limit"]["layer"], json!("user_configuration"));
}
