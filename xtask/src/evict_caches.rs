//! `xtask evict-caches`: the cache priority of ADR 0061 within GitHub's 10 GB
//! (TESTING.md, Sharding, caching and reports). Once the repository's
//! caches pass 9 GB, it deletes the least recently used ones that are not
//! the pull-request tier's Linux caches until they fit, so the priority
//! holds before GitHub's own eviction, which ignores it.
//!
//! The pull-request tier's caches are known by their keys: each starts with
//! `pull-request-`, or with `v0-rust-pull-request` for `Swatinem/rust-cache`
//! under a `shared-key` starting with `pull-request`, and a Linux one names
//! `Linux` as a part of its key, as rust-cache and `runner.os` write it.

use std::process::ExitCode;

use anyhow::{Context as _, Result};
use jiff::Timestamp;
use serde::Deserialize;

use crate::environment::Environment;
use crate::github::{self, List};
use crate::output;

/// The total past which caches are deleted: a gigabyte under GitHub's 10 GB
/// (ADR 0061).
const LIMIT_BYTES: u64 = 9_000_000_000;

#[derive(clap::Args)]
pub(crate) struct Arguments {
    /// The repository whose caches are kept, and GitHub's API.
    #[command(flatten)]
    github: github::Arguments,
}

#[derive(Deserialize)]
struct Cache {
    id: u64,
    key: String,
    last_accessed_at: Timestamp,
    size_in_bytes: u64,
}

impl Cache {
    fn is_pull_request_linux(&self) -> bool {
        let pull_request =
            self.key.starts_with("pull-request-") || self.key.starts_with("v0-rust-pull-request");
        pull_request && self.key.split('-').any(|part| part == "Linux")
    }
}

pub(crate) fn run(arguments: &Arguments, environment: &Environment) -> Result<ExitCode> {
    let api = arguments.github.api(environment.token());
    let caches_path = format!("/repos/{}/actions/caches", arguments.github.repository);
    let mut caches: Vec<Cache> = api
        .get_all(&caches_path, List::In("actions_caches"))
        .context("listing the repository's caches")?;
    let mut total: u64 = caches.iter().map(|cache| cache.size_in_bytes).sum();
    caches.retain(|cache| !cache.is_pull_request_linux());
    caches.sort_by_key(|cache| (cache.last_accessed_at, cache.id));
    for cache in &caches {
        if total <= LIMIT_BYTES {
            break;
        }
        api.delete(&format!("{caches_path}/{}", cache.id))?;
        total = total.saturating_sub(cache.size_in_bytes);
        output::line(&format!(
            "deleted cache {} ({} bytes): {}",
            cache.id, cache.size_in_bytes, cache.key
        ));
    }
    output::line(&format!(
        "caches: {total} bytes, {} the limit of {LIMIT_BYTES}",
        if total <= LIMIT_BYTES {
            "within"
        } else {
            "still past"
        }
    ));
    Ok(ExitCode::SUCCESS)
}
