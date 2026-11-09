//! `browser install` and `browser clean` (ADRs 0024 and 0098), commands of
//! both Products' executables: the first fetches every build the Product
//! version pins into the Browser cache, for CI and images; the second
//! deletes what nothing installed pins, nothing used for
//! `browser_cache.max_unused_age` and nothing holds, or under `--dry-run`
//! lists it. Their `--json` output is `goodfetch-protocol`'s type. Both
//! read the machine's settings (ADR 0098): `egress.offline`,
//! `network.resolve` and `browser_cache.max_unused_age`, from the Operator
//! policy and user configuration of the Goodfetch home `main` found, and
//! fail with their Error code when they cannot be read.

use std::collections::BTreeSet;
use std::path::Path;
use std::process::ExitCode;

use goodfetch_engine::Identity;
use goodfetch_engine::browser_cache::{
    BrowserCache, CacheError, CleanUp, MaxUnusedAge, Pins, Platform, kept_artifacts,
};
use goodfetch_engine::clock::Clock;
use goodfetch_engine::home::Home;
use goodfetch_engine::installed::{InstalledList, Seen, kept_inputs};
use goodfetch_engine::pinned_download::Fetching;
use goodfetch_engine::settings::Resolved;
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_protocol::browser_cache::{
    ArtifactId, BrowserCleaned, BrowserInstalled, InstallOutcome, InstalledBuild, MAX_UNUSED_AGE,
    NotCleaned, PinnedBy, RemovedBuild,
};

use super::daemon::Machine;
use super::output::{self, Human};

/// `browser_cache.max_unused_age` under `settings`.
fn max_unused_age(settings: &Resolved) -> MaxUnusedAge {
    MaxUnusedAge::new(settings.get(&MAX_UNUSED_AGE))
}

/// The time the command runs at, on the engine's Clock.
fn now(clock: Clock) -> jiff::Timestamp {
    clock.timestamp()
}

fn runtime() -> tokio::runtime::Runtime {
    #[expect(
        clippy::disallowed_methods,
        reason = "main builds the runtime (ADR 0060)"
    )]
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("invariant: the OS gives the runtime its thread")
}

/// Prints the Browser cache's failure `error` as every command's failure
/// prints, and answers its exit code.
fn failed(error: &CacheError, json: bool, human: &Human) -> ExitCode {
    output::fail(error.to_call_error().to_error_object(), None, json, human)
}

/// What a Browser cache command's builds are pinned by: the executable
/// that runs as `identity`.
fn pinned_by(identity: &Identity) -> PinnedBy {
    PinnedBy {
        product: identity.product,
        product_version: identity.product_version.clone(),
    }
}

/// `browser install`, into the Browser cache of `machine`'s Goodfetch
/// home.
pub(super) fn install(machine: &Machine<'_>, json: bool, human: &Human) -> ExitCode {
    let (identity, home, clock) = (machine.identity, machine.home, machine.clock);
    let settings = match machine.settings() {
        Ok(settings) => settings,
        Err(failure) => return failure.print(json, human),
    };
    let pinned_by = pinned_by(identity);
    let owner = TaskOwner::root(BlockingPolicy::Threads);
    let cache = BrowserCache::new(&machine.cache_location(), owner.blocking_threads());
    let pins = Pins {
        pinned_by,
        artifacts: crate::pinned_artifacts(),
    };
    let now = now(clock);
    let fetching = Fetching::of(settings, clock);
    let max_unused_age = max_unused_age(settings);
    let installed = runtime().block_on(async {
        let outcomes = cache
            .install(
                &pins,
                Platform::current().build_platform(),
                &fetching,
                now,
                &mut |progress| {
                    tracing::debug!(
                        artifact = %progress.artifact.name,
                        received = progress.received_bytes,
                        total = progress.total_bytes,
                        "downloading",
                    );
                },
            )
            .await?;
        if outcomes
            .iter()
            .any(|(_, outcome)| *outcome == InstallOutcome::Installed)
        {
            cache
                .clean_up_daily(
                    kept(&pins, home, cache.root(), now, max_unused_age),
                    now,
                    max_unused_age,
                )
                .await?;
        }
        Ok::<_, CacheError>(outcomes)
    });
    let outcomes = match installed {
        Ok(outcomes) => outcomes,
        Err(error) => return failed(&error, json, human),
    };
    if json {
        output::json(&BrowserInstalled {
            builds: outcomes
                .iter()
                .map(|(artifact, outcome)| InstalledBuild {
                    name: artifact.name.clone(),
                    version: artifact.build.clone(),
                    outcome: *outcome,
                })
                .collect(),
        });
    } else if outcomes.is_empty() {
        output::line(&format!(
            "{} {} pins no build for this platform; the Browser cache is {}",
            pins.pinned_by.product.name(),
            pins.pinned_by.product_version,
            cache.root().display(),
        ));
    } else {
        for (artifact, outcome) in &outcomes {
            let done = match outcome {
                InstallOutcome::Installed => "installed",
                InstallOutcome::AlreadyInstalled => "already installed",
            };
            output::line(&format!("{done}: {}", describe(artifact)));
        }
    }
    ExitCode::SUCCESS
}

/// `browser clean`, in the Browser cache of `machine`'s Goodfetch home.
pub(super) fn clean(
    machine: &Machine<'_>,
    (dry_run, json): (bool, bool),
    human: &Human,
) -> ExitCode {
    let (identity, home, clock) = (machine.identity, machine.home, machine.clock);
    let settings = match machine.settings() {
        Ok(settings) => settings,
        Err(failure) => return failure.print(json, human),
    };
    let pinned_by = pinned_by(identity);
    let owner = TaskOwner::root(BlockingPolicy::Threads);
    let cache = BrowserCache::new(&machine.cache_location(), owner.blocking_threads());
    let pins = Pins {
        pinned_by,
        artifacts: crate::pinned_artifacts(),
    };
    let now = now(clock);
    let max_unused_age = max_unused_age(settings);
    let request = CleanUp {
        kept: kept(&pins, home, cache.root(), now, max_unused_age),
        now,
        // `browser clean` uses the default while the setting is `null`.
        max_unused_age: max_unused_age.counting(),
        dry_run,
    };
    let report = match runtime().block_on(cache.clean_up(&request)) {
        Ok(report) => report,
        Err(error) => return failed(&error, json, human),
    };
    let cleaned = BrowserCleaned {
        dry_run,
        removed: report
            .deleted
            .iter()
            .map(|artifact| RemovedBuild {
                name: artifact.name.clone(),
                version: artifact.build.clone(),
                bytes: report.bytes_of(artifact).unwrap_or(0),
            })
            .collect(),
        bytes_freed: report.bytes.values().sum(),
        not_cleaned: report.not_cleaned,
    };
    if json {
        output::json(&cleaned);
    } else if cleaned.not_cleaned == Some(NotCleaned::CacheNotWritable) {
        output::line(&format!(
            "the Browser cache at {} cannot be written: nothing was cleaned",
            cache.root().display()
        ));
    } else {
        let done = if dry_run { "would delete" } else { "deleted" };
        for (artifact, removed) in report.deleted.iter().zip(&cleaned.removed) {
            output::line(&format!(
                "{done}: {} ({} bytes)",
                describe(artifact),
                removed.bytes
            ));
        }
        let freed = if dry_run { "would free" } else { "freed" };
        output::line(&format!("{freed} {} bytes", cleaned.bytes_freed));
    }
    ExitCode::SUCCESS
}

/// What nothing may delete (ADR 0098): the builds this executable pins,
/// since it runs and so counts as installed, and those of every entry of
/// the list of installed executables that counts, linked programs and
/// versioned copies included.
fn kept(
    pins: &Pins,
    home: &Home,
    cache: &Path,
    now: jiff::Timestamp,
    max_unused_age: MaxUnusedAge,
) -> BTreeSet<ArtifactId> {
    let entries = InstalledList::of(home).read().unwrap_or_else(|error| {
        tracing::warn!(%error, "the list of installed executables cannot be read; only this executable's builds are kept");
        Vec::new()
    });
    let seen: Vec<Seen<'_>> = entries
        .iter()
        .map(|entry| Seen::look(entry, cache))
        .collect();
    let mut kept = kept_artifacts(&kept_inputs(&seen), &[], now, max_unused_age);
    kept.extend(pins.artifacts.iter().map(|artifact| artifact.id.clone()));
    kept
}

fn describe(artifact: &ArtifactId) -> String {
    format!(
        "{} {} {}",
        artifact.kind.as_str(),
        artifact.name,
        artifact.build
    )
}
