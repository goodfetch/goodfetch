//! `browser install` and `browser clean` (ADRs 0024 and 0098), commands of
//! both Products' executables: the first fetches every build the Product
//! version pins into the Browser cache, for CI and images; the second
//! deletes what nothing installed pins, nothing used for
//! `browser_cache.max_unused_age` and nothing holds, or under `--dry-run`
//! lists it. Their `--json` output is `goodfetch-protocol`'s type. Both
//! read the machine's settings (ADR 0098): `egress.offline`,
//! `network.resolve` and `browser_cache.max_unused_age`, from the Operator
//! policy and user configuration of Goodfetch home.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use goodfetch_engine::Identity;
use goodfetch_engine::browser_cache::{
    BROWSERS_PATH_VARIABLE, BrowserCache, CacheError, CacheLocation, CleanUp, MaxUnusedAge, Pins,
    Platform, kept_artifacts,
};
use goodfetch_engine::clock::Clock;
use goodfetch_engine::happy_eyeballs::Delays;
use goodfetch_engine::home::Home;
use goodfetch_engine::http_client::Resolve;
use goodfetch_engine::installed::{InstalledList, Seen, kept_inputs};
use goodfetch_engine::pinned_download::Fetching;
use goodfetch_engine::run::local_settings_error;
use goodfetch_engine::settings::{Layers, Resolved, SettingsError, SettingsFiles, resolve};
use goodfetch_protocol::browser_cache::{
    ArtifactId, BrowserCleaned, BrowserInstalled, InstallOutcome, InstalledBuild, MAX_UNUSED_AGE,
    NotCleaned, PinnedBy, RemovedBuild,
};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::egress::EGRESS_OFFLINE;
use goodfetch_protocol::settings::NETWORK_RESOLVE;

use goodfetch_cli::output::{self, Human};

/// The variable that roots Goodfetch home (ADR 0059).
const HOME_VARIABLE: &str = "GOODFETCH_HOME";

/// The Goodfetch home and the Browser cache this executable uses:
/// `GOODFETCH_BROWSERS_PATH`, else `browsers` in `GOODFETCH_HOME`, else in
/// the OS's data folder.
fn location(environment: &BTreeMap<OsString, OsString>) -> Option<(Home, CacheLocation)> {
    let variable = |name: &str| {
        environment
            .get(OsStr::new(name))
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    };
    let home =
        variable(HOME_VARIABLE).or_else(|| dirs::data_dir().map(|data| data.join("goodfetch")))?;
    let location = CacheLocation::new(&home, variable(BROWSERS_PATH_VARIABLE));
    Some((Home::new(home), location))
}

/// The machine's settings: the Operator policy and user configuration of
/// the major of `version` in `home`, as the engine resolves them before
/// any Run.
pub(super) fn machine_settings(
    home: &Home,
    product: Product,
    version: &ProductVersion,
) -> Result<Resolved, SettingsError> {
    let machine = SettingsFiles::of(home, version).load(product)?;
    let layers = Layers {
        operator_policy: machine.operator_policy,
        user_configuration: machine.user_configuration,
        ..Layers::default()
    };
    resolve(&layers, product, product)
}

/// What downloads may do under `settings`: `egress.offline` and the layer
/// that set it, `network.resolve`'s mapping and the Happy Eyeballs delays.
fn fetching(settings: &Resolved, clock: Clock) -> Fetching {
    let offline = settings
        .get(EGRESS_OFFLINE.name)
        .filter(|offline| offline.value == serde_json::Value::Bool(true))
        .map(|offline| offline.layer);
    let mapping = settings
        .hosts(NETWORK_RESOLVE.name)
        .into_iter()
        .fold(Resolve::new(), |mapping, (host, addresses)| {
            mapping.with(host, addresses)
        });
    Fetching {
        offline,
        resolve: mapping,
        delays: Delays::of(settings),
        ..Fetching::new(clock)
    }
}

/// `browser_cache.max_unused_age` under `settings`.
fn max_unused_age(settings: &Resolved) -> MaxUnusedAge {
    MaxUnusedAge::new(
        settings
            .value(MAX_UNUSED_AGE.name)
            .as_u64()
            .map(Duration::from_millis),
    )
}

/// Prints the refusal of the machine's settings, `error`, on the
/// executable that runs as `identity`, as every command's failure prints,
/// and answers its exit code.
fn refused(error: &SettingsError, identity: &Identity, json: bool, human: &Human) -> ExitCode {
    output::fail(
        local_settings_error(error, identity).to_error_object(),
        None,
        json,
        human,
    )
}

/// The time the command runs at, on the engine's Clock.
fn now(clock: Clock) -> jiff::Timestamp {
    jiff::Timestamp::try_from(clock.wall())
        .expect("invariant: the system clock reads a representable time")
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

/// The Goodfetch home and the Browser cache the executable that runs as
/// `identity` uses, or the failure it printed when the OS knows no home
/// for it: `setting_invalid` for `GOODFETCH_HOME`.
fn located(
    identity: &Identity,
    environment: &BTreeMap<OsString, OsString>,
    json: bool,
    human: &Human,
) -> Result<(Home, CacheLocation), ExitCode> {
    location(environment).ok_or_else(|| {
        let error = SettingsError::Invalid {
            key: HOME_VARIABLE.to_owned(),
            layer: None,
            reason: format!(
                "the OS knows no data folder for Goodfetch home: set {HOME_VARIABLE}, or {BROWSERS_PATH_VARIABLE} for the Browser cache alone"
            ),
        };
        refused(&error, identity, json, human)
    })
}

/// What a Browser cache command's builds are pinned by: the executable
/// that runs as `identity`.
fn pinned_by(identity: &Identity) -> PinnedBy {
    PinnedBy {
        product: identity.product,
        product_version: identity.product_version.clone(),
    }
}

/// `browser install`.
pub(super) fn install(
    identity: &Identity,
    environment: &BTreeMap<OsString, OsString>,
    clock: Clock,
    json: bool,
    human: &Human,
) -> ExitCode {
    let (home, location) = match located(identity, environment, json, human) {
        Ok(located) => located,
        Err(exit) => return exit,
    };
    let pinned_by = pinned_by(identity);
    let settings = match machine_settings(&home, pinned_by.product, &pinned_by.product_version) {
        Ok(settings) => settings,
        Err(error) => return refused(&error, identity, json, human),
    };
    let cache = BrowserCache::new(&location);
    let pins = Pins {
        pinned_by,
        artifacts: crate::pinned_artifacts(),
    };
    let now = now(clock);
    let fetching = fetching(&settings, clock);
    let max_unused_age = max_unused_age(&settings);
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
                    kept(&pins, &home, cache.root(), now, max_unused_age),
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

/// `browser clean`.
pub(super) fn clean(
    identity: &Identity,
    environment: &BTreeMap<OsString, OsString>,
    clock: Clock,
    (dry_run, json): (bool, bool),
    human: &Human,
) -> ExitCode {
    let (home, location) = match located(identity, environment, json, human) {
        Ok(located) => located,
        Err(exit) => return exit,
    };
    let pinned_by = pinned_by(identity);
    let settings = match machine_settings(&home, pinned_by.product, &pinned_by.product_version) {
        Ok(settings) => settings,
        Err(error) => return refused(&error, identity, json, human),
    };
    let cache = BrowserCache::new(&location);
    let pins = Pins {
        pinned_by,
        artifacts: crate::pinned_artifacts(),
    };
    let now = now(clock);
    let max_unused_age = max_unused_age(&settings);
    let request = CleanUp {
        kept: kept(&pins, &home, cache.root(), now, max_unused_age),
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
