//! The Browser cache entry's writer for the Compatibility corpus (ADR 0066):
//! lays out a build as a prepared `GOODFETCH_BROWSERS_PATH` copy holds it,
//! has the engine prepare it for a launch at ADR 0059's test `Clock` start,
//! which rewrites its entry, and copies that entry into the folder given as
//! the only argument as `entry.json`. `xtask compat write` runs it.
#![expect(
    clippy::disallowed_methods,
    reason = "a synchronous corpus writer that builds its runtime and writes files (ADR 0066)"
)]

use std::path::PathBuf;

use goodfetch_engine::browser_cache::{
    BrowserCache, CacheLocation, PinnedArtifact, content_digest,
};
use goodfetch_engine::clock::Clock;
use goodfetch_engine::pinned_download::{Digest, Fetching};
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_protocol::browser_cache::{ArtifactId, ArtifactKind, PinnedBy};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::settings::SettingsLayer;

/// ADR 0059's test `Clock` start.
const START: &str = "2026-01-05T09:00:00Z";

fn main() {
    let output = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .expect("invariant: xtask compat passes the output folder"),
    );
    let scratch = tempfile::tempdir().expect("invariant: the OS gives a temporary folder");
    let cache = BrowserCache::new(
        &CacheLocation::new(scratch.path(), None),
        TaskOwner::root(BlockingPolicy::Inline).blocking_threads(),
    );
    let id = ArtifactId {
        kind: ArtifactKind::Browser,
        name: "chrome-for-testing".to_owned(),
        build: "141.0.7390.54-linux64".to_owned(),
    };
    let folder = cache.root().join("browser").join(&id.name).join(&id.build);
    std::fs::create_dir_all(folder.join("files"))
        .expect("invariant: the scratch folder is writable");
    std::fs::write(folder.join("files").join("chrome"), "#!chrome\n")
        .expect("invariant: the scratch folder is writable");
    std::fs::write(folder.join("lock"), "").expect("invariant: the scratch folder is writable");
    let digest =
        content_digest(&folder.join("files")).expect("invariant: the files were just written");
    let installed = serde_json::json!({
        "format_version": "0.1",
        "artifact": { "kind": "browser", "name": id.name, "build": id.build },
        "digest": digest.to_string(),
        "installed_at": "2025-12-01T00:00:00Z",
        "used_at": "2025-12-01T00:00:00Z",
    });
    std::fs::write(folder.join("entry.json"), installed.to_string())
        .expect("invariant: the scratch folder is writable");
    let artifact = PinnedArtifact {
        id,
        platform: None,
        url: "https://builds.goodfetch.test/chrome.zip".to_owned(),
        archive_digest: Some(Digest::of(b"never downloaded")),
        application_settings: std::collections::BTreeMap::new(),
        digest,
    };
    let pinned_by = PinnedBy {
        product: Product::Collie,
        product_version: ProductVersion::parse("0.0.0").expect("invariant: a Product version"),
    };
    let offline = Fetching {
        offline: Some(SettingsLayer::Run),
        ..Fetching::new(Clock::system())
    };
    let runtime = goodfetch_runtime::current_thread()
        .build()
        .expect("invariant: the OS gives the runtime its thread");
    let prepared = runtime
        .block_on(cache.prepare(
            &artifact,
            &pinned_by,
            &offline,
            START.parse().expect("invariant: an RFC 3339 time"),
            &mut |_| {},
        ))
        .expect("invariant: the laid-out build has its pinned digest");
    drop(prepared);
    std::fs::copy(folder.join("entry.json"), output.join("entry.json"))
        .expect("invariant: the output folder is writable");
}
