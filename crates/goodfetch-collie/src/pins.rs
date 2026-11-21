//! The Pinned artifacts Collie's version pins (ADRs 0024 and 0098): the
//! builds its launches take from the Browser cache, which `browser install`
//! fetches and whose entries the clean-up keeps. Foxhound's version pins the
//! same, since it launches what Collie launches.
//!
//! Chrome for Testing's build is the one Playwright's pinned release runs.
//! Its digests are `pins.json`, which `examples/pin-chrome-for-testing.rs`
//! writes: a platform with no archive digest yet is never downloaded, only
//! launched from a cache that holds it, and a platform the file does not
//! list has no pinned build. The file also holds the ALPS payload the build
//! sends, which the same tool captures when it pins the build (ADR 0042):
//! the egress relay sends a site the same, and a build without one is no
//! pin this reads, rather than one whose relay sends another.

use goodfetch_engine::browser_cache::{BuildPlatform, CHROME_FOR_TESTING, PinnedArtifact};
use goodfetch_protocol::browser_cache::{ArtifactId, ArtifactKind};
use std::collections::BTreeMap;

use serde_json::Value;

/// The digests of the pinned builds.
const PINS: &str = include_str!("pins.json");

/// Chrome for Testing's archives, by build and platform.
const STORAGE: &str = "https://storage.googleapis.com/chrome-for-testing-public";

/// Every artifact this version pins.
///
/// # Panics
///
/// When `pins.json` is not what its writer writes, which a test catches,
/// a build without its captured ALPS payload among them.
///
/// # Examples
///
/// ```
/// let pins = goodfetch_collie::pinned_artifacts();
/// assert!(pins.iter().all(|pin| pin.url.starts_with("https://")));
/// ```
pub fn pinned_artifacts() -> Vec<PinnedArtifact> {
    let pins: Value = serde_json::from_str(PINS).expect("invariant: pins.json is JSON");
    let chrome = &pins["chrome_for_testing"];
    let build = chrome["build"]
        .as_str()
        .expect("invariant: pins.json names its Chrome for Testing build");
    let application_settings: BTreeMap<String, Vec<u8>> = chrome["application_settings"]
        .as_object()
        .filter(|captured| captured.contains_key("h2"))
        .expect("invariant: pins.json holds the ALPS payload captured from its build (`pin-chrome-for-testing alps`)")
        .iter()
        .map(|(protocol, payload)| {
            let payload = payload
                .as_str()
                .and_then(hex)
                .expect("invariant: pins.json's ALPS payloads are hex");
            (protocol.clone(), payload)
        })
        .collect();
    chrome["platforms"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(platform, digests)| {
            let digest = |key: &str| {
                digests[key]
                    .as_str()
                    .map(|text| text.parse().expect("invariant: pins.json's digests read"))
            };
            PinnedArtifact {
                id: ArtifactId {
                    kind: ArtifactKind::Browser,
                    name: CHROME_FOR_TESTING.to_owned(),
                    build: build.to_owned(),
                },
                platform: Some(build_platform(platform)),
                url: format!("{STORAGE}/{build}/{platform}/chrome-{platform}.zip"),
                archive_digest: digest("archive_digest"),
                digest: digest("digest").expect("invariant: each pinned platform has its digest"),
                application_settings: application_settings.clone(),
            }
        })
        .collect()
}

/// The ids of every artifact this version pins, which an executable or
/// linked program records in the list of installed executables so the
/// clean-up keeps their entries (ADR 0098).
pub(crate) fn pinned_artifact_ids() -> Vec<ArtifactId> {
    pinned_artifacts()
        .into_iter()
        .map(|artifact| artifact.id)
        .collect()
}

/// The bytes `text` spells in hex, two digits each.
fn hex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(text.get(at..at + 2)?, 16).ok())
        .collect()
}

/// A Chrome for Testing platform's name as the Browser cache's.
fn build_platform(platform: &str) -> BuildPlatform {
    BuildPlatform::of_chrome_for_testing_name(platform)
        .unwrap_or_else(|| panic!("invariant: pins.json names no platform `{platform}`"))
}
