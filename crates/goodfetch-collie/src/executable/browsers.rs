//! `browsers install` and `browsers clean` (ADRs 0024 and 0098), commands of
//! both Products' executables: the first fetches every build the Product
//! version pins into the Browser cache, for CI and images; the second
//! deletes what nothing installed pins, nothing used for
//! `browser_cache.max_unused_age` and nothing holds, or under `--dry-run`
//! lists it. Their `--json` output is `goodfetch-protocol`'s type. Both
//! read the machine's settings (ADR 0098): `network.offline`,
//! `network.resolve` and `browser_cache.max_unused_age`, from the Operator
//! policy and user configuration of Goodfetch home.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{OsStr, OsString};
use std::net::IpAddr;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use goodfetch_engine::browser_cache::{
    BROWSERS_PATH_VARIABLE, BrowserCache, CacheError, CacheLocation, CleanUp, Fetching,
    MaxUnusedAge, Pins, Platform, Resolve,
};
use goodfetch_engine::home::Home;
use goodfetch_engine::settings::{Layers, Resolved, SettingsError, SettingsFiles, resolve};
use goodfetch_protocol::browser_cache::{
    ArtifactId, BrowsersCleanOutput, BrowsersInstallOutput, InstallOutcome, InstalledArtifact,
    MAX_UNUSED_AGE, NotCleaned, PinnedBy,
};
use goodfetch_protocol::settings::{NETWORK_OFFLINE, NETWORK_RESOLVE};

use super::output;

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
/// `home`'s major, as the engine resolves them before any Run.
fn machine_settings(home: &Home, pinned_by: &PinnedBy) -> Result<Resolved, SettingsError> {
    let product = pinned_by.product;
    let machine = SettingsFiles::of(home, &pinned_by.product_version).load(product)?;
    let layers = Layers {
        operator_policy: machine.operator_policy,
        user_configuration: machine.user_configuration,
        ..Layers::default()
    };
    resolve(&layers, product, product)
}

/// What downloads may do under `settings`: `network.offline` and the layer
/// that set it, and `network.resolve`'s mapping.
fn fetching(settings: &Resolved) -> Fetching {
    let offline = settings
        .get(NETWORK_OFFLINE.name)
        .filter(|offline| offline.value == serde_json::Value::Bool(true))
        .map(|offline| offline.layer);
    let mut mapping = Resolve::new();
    if let Some(hosts) = settings.value(NETWORK_RESOLVE.name).as_object() {
        for (host, addresses) in hosts {
            let addresses: Vec<IpAddr> = addresses
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|address| address.as_str()?.parse().ok())
                .collect();
            mapping = mapping.with(host.clone(), addresses);
        }
    }
    Fetching {
        offline,
        resolve: mapping,
        ..Fetching::default()
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

fn refused(error: &SettingsError) -> ExitCode {
    tracing::error!(code = error.code(), "{error}");
    ExitCode::FAILURE
}

/// The time the command runs at.
fn now() -> jiff::Timestamp {
    #[expect(
        clippy::disallowed_methods,
        reason = "main reads the system's wall time once, as the system Clock will (ADR 0059)"
    )]
    let now = std::time::SystemTime::now();
    jiff::Timestamp::try_from(now).expect("invariant: the system clock reads a representable time")
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

fn failed(error: &CacheError) -> ExitCode {
    tracing::error!(code = error.code(), "{error}");
    ExitCode::FAILURE
}

fn no_home() -> ExitCode {
    tracing::error!(
        "no Goodfetch home: set {HOME_VARIABLE}, or {BROWSERS_PATH_VARIABLE} for the Browser cache alone"
    );
    ExitCode::FAILURE
}

/// `browsers install`.
pub(super) fn install(
    pinned_by: PinnedBy,
    environment: &BTreeMap<OsString, OsString>,
    json: bool,
) -> ExitCode {
    let Some((home, location)) = location(environment) else {
        return no_home();
    };
    let settings = match machine_settings(&home, &pinned_by) {
        Ok(settings) => settings,
        Err(error) => return refused(&error),
    };
    let cache = BrowserCache::new(&location);
    let pins = Pins {
        pinned_by,
        artifacts: crate::pinned_artifacts(),
    };
    let now = now();
    let fetching = fetching(&settings);
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
                .clean_up_daily(kept(&pins), now, max_unused_age)
                .await?;
        }
        Ok::<_, CacheError>(outcomes)
    });
    let outcomes = match installed {
        Ok(outcomes) => outcomes,
        Err(error) => return failed(&error),
    };
    let report = BrowsersInstallOutput {
        cache: cache.root().display().to_string(),
        artifacts: outcomes
            .into_iter()
            .map(|(artifact, outcome)| InstalledArtifact { artifact, outcome })
            .collect(),
    };
    if json {
        output::json(&report);
    } else if report.artifacts.is_empty() {
        output::line(&format!(
            "{} {} pins no build for this platform; the Browser cache is {}",
            pins.pinned_by.product.name(),
            pins.pinned_by.product_version,
            report.cache,
        ));
    } else {
        for installed in &report.artifacts {
            let done = match installed.outcome {
                InstallOutcome::Installed => "installed",
                InstallOutcome::AlreadyInstalled => "already installed",
            };
            output::line(&format!("{done}: {}", describe(&installed.artifact)));
        }
    }
    ExitCode::SUCCESS
}

/// `browsers clean`.
pub(super) fn clean(
    pinned_by: PinnedBy,
    environment: &BTreeMap<OsString, OsString>,
    dry_run: bool,
    json: bool,
) -> ExitCode {
    let Some((home, location)) = location(environment) else {
        return no_home();
    };
    let settings = match machine_settings(&home, &pinned_by) {
        Ok(settings) => settings,
        Err(error) => return refused(&error),
    };
    let cache = BrowserCache::new(&location);
    let pins = Pins {
        pinned_by,
        artifacts: crate::pinned_artifacts(),
    };
    let request = CleanUp {
        kept: kept(&pins),
        now: now(),
        // `browsers clean` uses the default while the setting is `null`.
        max_unused_age: max_unused_age(&settings).counting(),
        dry_run,
    };
    let report = match runtime().block_on(cache.clean_up(&request)) {
        Ok(report) => report,
        Err(error) => return failed(&error),
    };
    let report = BrowsersCleanOutput {
        cache: cache.root().display().to_string(),
        dry_run,
        deleted: report.deleted,
        not_cleaned: report.not_cleaned,
    };
    if json {
        output::json(&report);
    } else if report.not_cleaned == Some(NotCleaned::CacheNotWritable) {
        output::line(&format!(
            "the Browser cache at {} cannot be written: nothing was cleaned",
            report.cache
        ));
    } else {
        let done = if dry_run { "would delete" } else { "deleted" };
        for artifact in &report.deleted {
            output::line(&format!("{done}: {}", describe(artifact)));
        }
    }
    ExitCode::SUCCESS
}

/// What nothing may delete: the builds this executable pins, since it runs
/// and so counts as installed. The list of installed executables (#220)
/// adds every other executable that counts.
fn kept(pins: &Pins) -> BTreeSet<ArtifactId> {
    pins.artifacts
        .iter()
        .map(|artifact| artifact.id.clone())
        .collect()
}

fn describe(artifact: &ArtifactId) -> String {
    format!(
        "{} {} {}",
        artifact.kind.as_str(),
        artifact.name,
        artifact.build
    )
}
