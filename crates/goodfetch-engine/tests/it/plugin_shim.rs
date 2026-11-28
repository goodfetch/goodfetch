//! A linked Rust engine locating or downloading its `plugin-shim` executable
//! (ADRs 0033, 0043, 0051, 0090 and 0115; issue 1077):
//! - reads `GOODFETCH_COLLIE_ENGINE_PATH` (Foxhound: `GOODFETCH_FOXHOUND_ENGINE_PATH`)
//!   without touching the network or cache, ignoring the other Product's variable;
//! - otherwise downloads the pinned `EngineExecutable` from a local HTTPS server
//!   into the shared Browser cache, checks its SHA-256 digest, refuses a wrong
//!   digest with `artifact_download_failed`, and reuses the cached copy;
//! - under `egress.offline`, fails a missing shim with
//!   `artifact_unavailable_offline` naming the artifact and what pins it;
//! - on a failed download (HTTP 404), fails with `artifact_download_failed`
//!   naming the artifact and what pins it.
#![expect(
    clippy::disallowed_methods,
    reason = "tests write fixture files and run on the test Clock (ADR 0059)"
)]

use std::collections::BTreeMap;
use std::ffi::OsString;

use goodfetch_engine::browser_cache::{
    BrowserCache, CacheLocation, PinnedArtifact, content_digest,
};
use goodfetch_engine::clock::Clock;
use goodfetch_engine::pinned_download::Digest;
use goodfetch_engine::plugin_shim::{PluginShim, default_pin, host_platform};
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_protocol::browser_cache::{
    ArtifactDownloadFailedDetails, ArtifactId, ArtifactKind, ArtifactUnavailableOfflineDetails,
    DownloadFailure, Pinner,
};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::settings::{
    COLLIE_ENGINE_PATH_VARIABLE, FOXHOUND_ENGINE_PATH_VARIABLE, SettingsLayer,
};
use goodfetch_test_support::TestEnv;
use serde_json::json;

use crate::downloads::StandIn;

const SHIM_PATH: &str = "/releases/v0.1.0/goodfetch-collie";

fn shim_pin(stand_in: &StandIn, bytes: &[u8], expected_digest: Option<Digest>) -> PinnedArtifact {
    let dir = tempfile::tempdir().unwrap();
    let placed = dir.path().join("goodfetch-collie");
    std::fs::write(&placed, bytes).unwrap();
    let tree = content_digest(dir.path()).unwrap();
    let archive_digest = expected_digest.unwrap_or_else(|| Digest::of(bytes));
    PinnedArtifact {
        id: ArtifactId {
            kind: ArtifactKind::EngineExecutable,
            name: Product::Collie.executable_name().to_owned(),
            build: format!("0.1.0-{}", host_platform()),
        },
        platform: None,
        url: format!("https://{}{SHIM_PATH}", stand_in.authority()),
        archive_digest: Some(archive_digest),
        digest: tree,
        application_settings: BTreeMap::new(),
    }
}

/// A linked engine finds `plugin-shim` through its Product's own engine-path
/// environment variable (`GOODFETCH_COLLIE_ENGINE_PATH` for Collie,
/// `GOODFETCH_FOXHOUND_ENGINE_PATH` for Foxhound) without hitting the network
/// even under `egress.offline`, and ignores the other Product's variable.
///
/// Covers: story:7-107
#[test]
fn linked_shim_uses_product_env_var_without_downloading() {
    let env = TestEnv::new();
    let version = ProductVersion::parse("0.1.0").unwrap();
    let blocking = TaskOwner::root(BlockingPolicy::Inline).blocking_threads();
    let cache = BrowserCache::new(&CacheLocation::new(env.home(), None), blocking.clone());
    let now = Clock::system().timestamp();

    goodfetch_test_support::runtime().block_on(async {
        let collie_path = env.home().join("custom-collie");
        let foxhound_path = env.home().join("custom-foxhound");
        let vars = BTreeMap::from([
            (
                OsString::from(COLLIE_ENGINE_PATH_VARIABLE),
                collie_path.clone().into_os_string(),
            ),
            (
                OsString::from(FOXHOUND_ENGINE_PATH_VARIABLE),
                foxhound_path.clone().into_os_string(),
            ),
        ]);

        let collie_shim =
            PluginShim::linked(Product::Collie, version.clone(), None).with_environment(&vars);
        assert!(!collie_shim.needs_cache());
        let resolved = collie_shim
            .resolve(&cache, None, now, &blocking)
            .await
            .unwrap();
        assert_eq!(resolved.launcher, collie_path);
        assert!(resolved.held.is_none());

        let foxhound_shim =
            PluginShim::linked(Product::Foxhound, version, None).with_environment(&vars);
        assert!(!foxhound_shim.needs_cache());
        let resolved_fox = foxhound_shim
            .resolve(&cache, None, now, &blocking)
            .await
            .unwrap();
        assert_eq!(resolved_fox.launcher, foxhound_path);
    });
}

/// Without its engine-path variable, a linked engine downloads the pinned
/// `EngineExecutable` from a local HTTPS server into the shared Browser cache,
/// checks its digest, marks it executable, and reuses the cached copy on later
/// calls without a second download; a server returning bytes with a wrong
/// digest is refused with `artifact_download_failed` (`digest_mismatch`).
///
/// Covers: story:7-107, error:artifact_download_failed
#[test]
fn linked_shim_downloads_verifies_caches_and_refuses_wrong_digest() {
    let env = TestEnv::new();
    let version = ProductVersion::parse("0.1.0").unwrap();
    let blocking = TaskOwner::root(BlockingPolicy::Inline).blocking_threads();
    let cache = BrowserCache::new(&CacheLocation::new(env.home(), None), blocking.clone());
    let now = Clock::system().timestamp();
    let shim_bytes = b"#!/bin/sh\nexit 0\n".to_vec();

    goodfetch_test_support::runtime().block_on(async {
        let mut stand_in = StandIn::serve(BTreeMap::from([(SHIM_PATH, shim_bytes.clone())])).await;
        let pin = shim_pin(&stand_in, &shim_bytes, None);
        let fetching = stand_in.fetching();
        let shim = PluginShim::linked(Product::Collie, version.clone(), Some(pin.clone()));

        let first = shim
            .resolve(&cache, Some(&fetching), now, &blocking)
            .await
            .unwrap();
        assert!(first.launcher.is_file(), "{}", first.launcher.display());
        assert!(first.launcher.starts_with(cache.root()));
        assert!(first.held.is_some());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&first.launcher)
                .unwrap()
                .permissions()
                .mode();
            assert_ne!(mode & 0o111, 0, "downloaded shim must be executable");
        }

        // Second resolution hits the cache without contacting the server again.
        let second = shim
            .resolve(&cache, Some(&fetching), now, &blocking)
            .await
            .unwrap();
        assert_eq!(second.launcher, first.launcher);
        assert_eq!(stand_in.requests().len(), 1);
    });

    // A wrong digest from the server is refused and nothing is cached.
    let bad_env = TestEnv::new();
    let bad_cache = BrowserCache::new(&CacheLocation::new(bad_env.home(), None), blocking.clone());
    goodfetch_test_support::runtime().block_on(async {
        let tampered = b"#!/bin/sh\n# tampered\n".to_vec();
        let stand_in = StandIn::serve(BTreeMap::from([(SHIM_PATH, tampered)])).await;
        let pin = shim_pin(&stand_in, &shim_bytes, Some(Digest::of(&shim_bytes)));
        let fetching = stand_in.fetching();
        let shim = PluginShim::linked(Product::Collie, version.clone(), Some(pin.clone()));

        let error = shim
            .resolve(&bad_cache, Some(&fetching), now, &blocking)
            .await
            .err()
            .expect("wrong digest must be refused");
        assert_eq!(error.code(), "artifact_download_failed");
        let details: ArtifactDownloadFailedDetails =
            serde_json::from_value(serde_json::Value::Object(error.details().unwrap().clone()))
                .unwrap();
        assert_eq!(details.artifact, pin.id);
        assert_eq!(
            details.pinned_by,
            vec![Pinner::Product {
                name: Product::Collie,
                version,
            }]
        );
        assert_eq!(details.failure, DownloadFailure::DigestMismatch);
        let cached_folder = bad_cache
            .root()
            .join("engine_executable")
            .join(&pin.id.name)
            .join(&pin.id.build);
        assert!(
            !cached_folder.exists(),
            "refused digest must leave nothing in cache"
        );
    });
}

/// Under `egress.offline`, a missing `plugin-shim` fails with
/// `artifact_unavailable_offline` naming the artifact and what pins it without
/// contacting the network; an HTTP 404 fails with `artifact_download_failed`
/// naming the artifact and what pins it.
///
/// Covers: story:7-107, error:artifact_unavailable_offline, error:artifact_download_failed
#[test]
fn linked_shim_offline_and_http_failure_name_artifact_and_pinner() {
    let env = TestEnv::new();
    let version = ProductVersion::parse("0.1.0").unwrap();
    let blocking = TaskOwner::root(BlockingPolicy::Inline).blocking_threads();
    let cache = BrowserCache::new(&CacheLocation::new(env.home(), None), blocking.clone());
    let now = Clock::system().timestamp();

    goodfetch_test_support::runtime().block_on(async {
        // Offline refusal on Collie and Foxhound default pins.
        for (product, expected_var) in [
            (Product::Collie, COLLIE_ENGINE_PATH_VARIABLE),
            (Product::Foxhound, FOXHOUND_ENGINE_PATH_VARIABLE),
        ] {
            let mut fetching = goodfetch_engine::pinned_download::Fetching::new(Clock::system());
            fetching.offline = Some(SettingsLayer::Run);
            let shim = PluginShim::linked(product, version.clone(), None);
            let error = shim
                .resolve(&cache, Some(&fetching), now, &blocking)
                .await
                .err()
                .expect("offline missing shim must fail");
            assert_eq!(error.code(), "artifact_unavailable_offline");
            let details: ArtifactUnavailableOfflineDetails =
                serde_json::from_value(serde_json::Value::Object(error.details().unwrap().clone()))
                    .unwrap();
            assert_eq!(details.artifact, default_pin(product, &version).id);
            assert_eq!(
                details.pinned_by,
                vec![Pinner::Product {
                    name: product,
                    version: version.clone(),
                }]
            );
            assert_eq!(details.limit.key, "egress.offline");
            assert_eq!(details.limit.value, json!(true));
            assert!(
                details.fix.contains(expected_var),
                "fix should name {expected_var}: {}",
                details.fix
            );
        }

        // HTTP 404 download failure.
        let mut stand_in = StandIn::serve(BTreeMap::new()).await;
        let pin = shim_pin(&stand_in, b"unused", None);
        let fetching = stand_in.fetching();
        let shim = PluginShim::linked(Product::Collie, version.clone(), Some(pin.clone()));
        let error = shim
            .resolve(&cache, Some(&fetching), now, &blocking)
            .await
            .err()
            .expect("HTTP 404 must fail");
        assert_eq!(error.code(), "artifact_download_failed");
        let details: ArtifactDownloadFailedDetails =
            serde_json::from_value(serde_json::Value::Object(error.details().unwrap().clone()))
                .unwrap();
        assert_eq!(details.artifact, pin.id);
        assert_eq!(
            details.pinned_by,
            vec![Pinner::Product {
                name: Product::Collie,
                version,
            }]
        );
        assert_eq!(details.failure, DownloadFailure::HttpStatus { status: 404 });
        assert_eq!(stand_in.requests().len(), 1);
    });
}
