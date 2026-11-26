//! Installing a Plugin from its OCI artifact (ADR 0033, issue 889), pulled
//! from a stand-in registry on loopback over HTTPS.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use goodfetch_engine::pinned_download::{Digest, Fetching};
use goodfetch_engine::plugin_install::{self, InstallError, PullFailure};
use goodfetch_engine::plugin_oci::{
    INDEX_MEDIA_TYPE, MANIFEST_MEDIA_TYPE, Platform, Reference, TAR_GZIP_MEDIA_TYPE, TAR_MEDIA_TYPE,
};
use goodfetch_engine::plugin_store::{Installed, PluginStore};
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_protocol::plugin::PluginKindName;
use goodfetch_protocol::settings::SettingsLayer;
use goodfetch_test_support::TestEnv;
use hyper::{HeaderMap, StatusCode};
use rstest::rstest;
use serde_json::json;

use crate::downloads::{Received, StandIn};

/// The repository the stand-in registry serves the Plugin `kettle` from.
pub(crate) const REPOSITORY: &str = "goodfetch-plugin-kettle";

/// A tar archive of `entries`: each a path, its bytes, its mode and its
/// kind.
fn tar(entries: &[(&str, &[u8], u32, tar::EntryType)]) -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());
    for (path, bytes, mode, kind) in entries {
        let mut header = tar::Header::new_gnu();
        header.set_size(u64::try_from(bytes.len()).unwrap());
        header.set_mode(*mode);
        header.set_entry_type(*kind);
        builder.append_data(&mut header, path, *bytes).unwrap();
    }
    builder.into_inner().unwrap()
}

/// The bundle of the Extractor `kettle`: its manifest, its executable and
/// an install script, each a script that would write `marker` if anything
/// ran it.
pub(crate) fn bundle(marker: &Path) -> Vec<u8> {
    let manifest = json!({
        "manifest_version": "0.1",
        "name": "kettle",
        "version": "1.0.0",
        "kinds": ["extract"],
        "protocol": { "min": "0.1", "max": "0.1" },
        "entrypoint": "plugin",
        "network_level": "none",
    })
    .to_string();
    let script = format!("#!/bin/sh\ntouch '{}'\n", marker.display());
    tar(&[
        (
            "goodfetch-plugin.json",
            manifest.as_bytes(),
            0o644,
            tar::EntryType::Regular,
        ),
        ("plugin", script.as_bytes(), 0o755, tar::EntryType::Regular),
        (
            "install.sh",
            script.as_bytes(),
            0o755,
            tar::EntryType::Regular,
        ),
    ])
}

/// `bytes` compressed with gzip.
pub(crate) fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(bytes).unwrap();
    encoder.finish().unwrap()
}

/// A Plugin's OCI artifact: its image index, the host platform's manifest
/// and that manifest's one layer.
#[derive(Clone)]
pub(crate) struct Artifact {
    pub(crate) index: Vec<u8>,
    pub(crate) manifest: Vec<u8>,
    pub(crate) layer: Vec<u8>,
}

impl Artifact {
    /// The artifact whose one layer is `layer`, of `media_type`.
    pub(crate) fn of(layer: Vec<u8>, media_type: &str) -> Self {
        let manifest = serde_json::to_vec(&json!({
            "schemaVersion": 2,
            "mediaType": MANIFEST_MEDIA_TYPE,
            "config": {
                "mediaType": "application/vnd.oci.empty.v1+json",
                "digest": Digest::of(b"{}").to_string(),
                "size": 2,
            },
            "layers": [{
                "mediaType": media_type,
                "digest": Digest::of(&layer).to_string(),
                "size": layer.len(),
            }],
        }))
        .unwrap();
        let host = Platform::host();
        let index = serde_json::to_vec(&json!({
            "schemaVersion": 2,
            "mediaType": INDEX_MEDIA_TYPE,
            "manifests": [{
                "mediaType": MANIFEST_MEDIA_TYPE,
                "digest": Digest::of(&manifest).to_string(),
                "size": manifest.len(),
                "platform": { "os": host.os, "architecture": host.architecture },
            }],
        }))
        .unwrap();
        Self {
            index,
            manifest,
            layer,
        }
    }

    /// The path of the blob or document `bytes` in the repository.
    fn path(kind: &str, bytes: &[u8]) -> String {
        format!("/v2/{REPOSITORY}/{kind}/{}", Digest::of(bytes))
    }

    /// Its reference in the repository of the stand-in registry `stand_in`,
    /// pinned by its index's digest.
    pub(crate) fn reference(&self, stand_in: &StandIn) -> Reference {
        format!(
            "{}/{REPOSITORY}@{}",
            stand_in.authority(),
            Digest::of(&self.index)
        )
        .parse()
        .unwrap()
    }

    /// Answers as a registry that holds it does, each document only to a
    /// request that accepts its media type, with `served` in place of what
    /// each path pins.
    pub(crate) fn answer(
        &self,
        served: Self,
    ) -> impl Fn(&str, &HeaderMap) -> (StatusCode, Vec<u8>) + use<> {
        let documents = [
            (
                Self::path("manifests", &self.index),
                INDEX_MEDIA_TYPE,
                served.index,
            ),
            (
                Self::path("manifests", &self.manifest),
                MANIFEST_MEDIA_TYPE,
                served.manifest,
            ),
        ];
        let blob = (Self::path("blobs", &self.layer), served.layer);
        move |path, headers| {
            let accepts = |media_type: &str| {
                headers
                    .get(hyper::header::ACCEPT)
                    .is_some_and(|accept| accept == media_type)
            };
            if let Some((_, _, bytes)) = documents
                .iter()
                .find(|(at, media_type, _)| at == path && accepts(media_type))
            {
                (StatusCode::OK, bytes.clone())
            } else if path == blob.0 {
                (StatusCode::OK, blob.1.clone())
            } else {
                (StatusCode::NOT_FOUND, Vec::new())
            }
        }
    }

    /// Installs it into the store of `env` from a stand-in registry that
    /// answers with `answer`, through what `fetching` makes of the
    /// stand-in's fetching; answers the outcome and the requests the
    /// registry received.
    fn install(
        &self,
        env: &TestEnv,
        answer: impl Fn(&str, &HeaderMap) -> (StatusCode, Vec<u8>) + Send + Sync + 'static,
        fetching: impl FnOnce(Fetching) -> Fetching,
    ) -> (Result<Installed, InstallError>, Vec<Received>) {
        let store = PluginStore::new(store(env));
        let blocking = TaskOwner::root(BlockingPolicy::Inline).blocking_threads();
        goodfetch_test_support::runtime().block_on(async {
            let mut stand_in = StandIn::serve_with(answer).await;
            let reference = self.reference(&stand_in);
            let fetching = fetching(stand_in.fetching());
            let installed = plugin_install::install(&reference, &fetching, &blocking, &store).await;
            (installed, stand_in.requests())
        })
    }
}

/// What is left in the home of `env` beside the store, and in the store.
#[expect(
    clippy::disallowed_methods,
    reason = "the test reads the folders the engine wrote, outside any async worker"
)]
fn left_behind(env: &TestEnv) -> Vec<String> {
    let names = |folder: &Path| -> Vec<String> {
        std::fs::read_dir(folder).map_or_else(
            |_| Vec::new(),
            |entries| {
                entries
                    .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                    .collect()
            },
        )
    };
    let mut left = names(&env.home().join("data"));
    left.retain(|name| name != "plugins");
    left.extend(names(&store(env)));
    left
}

/// The Plugin store's folder in the home of `env`, alone in its parent.
pub(crate) fn store(env: &TestEnv) -> PathBuf {
    env.home().join("data").join("plugins")
}

/// A Plugin installs from its registry for the host's platform: the index
/// and the manifest asked for by their media types, the bundle's layer by
/// its digest, and its files land in the store, where it resolves, with
/// nothing of it run and no staging folder left.
///
/// Covers: story:7-5
#[test]
fn a_plugin_installs_from_its_registry_for_the_host_platform() {
    let env = TestEnv::new();
    let marker = env.home().join("ran");
    let artifact = Artifact::of(gzip(&bundle(&marker)), TAR_GZIP_MEDIA_TYPE);

    let (installed, requests) =
        artifact.install(&env, artifact.answer(artifact.clone()), |fetching| fetching);

    let bundle = store(&env).join("kettle").join("1.0.0");
    assert_eq!(installed.unwrap().bundle, bundle);
    let resolved = PluginStore::new(store(&env))
        .resolve("kettle", &PluginKindName::EXTRACT)
        .unwrap();
    assert_eq!(resolved.bundle, bundle);
    assert!(bundle.join("install.sh").is_file() && !marker.exists());
    assert_eq!(left_behind(&env), ["kettle"]);
    let asked: Vec<_> = requests
        .into_iter()
        .map(|received| {
            let accept = received.headers.iter().any(|name| name == "accept");
            (received.path, accept)
        })
        .collect();
    assert_eq!(
        asked,
        [
            (Artifact::path("manifests", &artifact.index), true),
            (Artifact::path("manifests", &artifact.manifest), true),
            (Artifact::path("blobs", &artifact.layer), false),
        ]
    );
}

/// A registry that refuses the pull installs nothing, and its refusal is
/// the named failure `artifact_download_failed` carries.
///
/// Covers: story:7-81
#[rstest]
#[case::absent(StatusCode::NOT_FOUND, PullFailure::Absent)]
#[case::unauthorized(StatusCode::UNAUTHORIZED, PullFailure::Unauthorized(401))]
#[case::rate_limited(StatusCode::TOO_MANY_REQUESTS, PullFailure::RateLimited)]
#[case::server_error(StatusCode::BAD_GATEWAY, PullFailure::ServerError(502))]
fn a_pull_the_registry_refuses_installs_nothing(
    #[case] status: StatusCode,
    #[case] expected: PullFailure,
) {
    let env = TestEnv::new();
    let artifact = Artifact::of(tar(&[]), TAR_MEDIA_TYPE);

    let (installed, _) = artifact.install(
        &env,
        move |_, _| (status, b"refused".to_vec()),
        |fetching| fetching,
    );

    assert!(
        matches!(&installed, Err(InstallError::Pull(failure)) if *failure == expected),
        "{installed:?}"
    );
    assert_eq!(left_behind(&env), Vec::<String>::new());
}

/// Bytes the registry serves that are not the ones their digest pins, the
/// index's or the bundle's, install nothing.
///
/// Covers: story:7-81
#[rstest]
#[case::index(|served: &mut Artifact| served.index.push(b' '))]
#[case::layer(|served: &mut Artifact| served.layer.push(0))]
fn bytes_that_are_not_the_pinned_ones_install_nothing(#[case] tamper: fn(&mut Artifact)) {
    let env = TestEnv::new();
    let artifact = Artifact::of(gzip(&bundle(&env.home().join("ran"))), TAR_GZIP_MEDIA_TYPE);
    let mut served = artifact.clone();
    tamper(&mut served);

    let (installed, _) = artifact.install(&env, artifact.answer(served), |fetching| fetching);

    assert!(
        matches!(
            installed,
            Err(InstallError::Pull(PullFailure::DigestMismatch))
        ),
        "{installed:?}"
    );
    assert_eq!(left_behind(&env), Vec::<String>::new());
}

/// A bundle the unpack refuses, here one holding a FIFO, installs
/// nothing and leaves no staging folder.
///
/// Covers: story:7-5
#[test]
fn a_bundle_the_unpack_refuses_installs_nothing() {
    let env = TestEnv::new();
    let artifact = Artifact::of(
        tar(&[("pipe", b"", 0o644, tar::EntryType::Fifo)]),
        TAR_MEDIA_TYPE,
    );

    let (installed, _) =
        artifact.install(&env, artifact.answer(artifact.clone()), |fetching| fetching);

    assert!(
        matches!(installed, Err(InstallError::Bundle(_))),
        "{installed:?}"
    );
    assert_eq!(left_behind(&env), Vec::<String>::new());
}

/// `egress.offline` refuses the pull before anything connects.
///
/// Covers: setting:egress.offline
#[test]
fn offline_refuses_the_pull_before_it_connects() {
    let env = TestEnv::new();
    let artifact = Artifact::of(tar(&[]), TAR_MEDIA_TYPE);

    let (installed, requests) =
        artifact.install(&env, artifact.answer(artifact.clone()), |fetching| {
            Fetching {
                offline: Some(SettingsLayer::UserConfiguration),
                ..fetching
            }
        });

    assert!(
        matches!(
            installed,
            Err(InstallError::Offline(SettingsLayer::UserConfiguration))
        ),
        "{installed:?}"
    );
    assert_eq!(requests, []);
}
