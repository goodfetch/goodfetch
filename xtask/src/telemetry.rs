//! ADR 0043's "no telemetry, with no setting that could turn it on": no key
//! of the settings catalog, which both Products' settings schema is
//! generated from, names telemetry, and no lockfile of the Rust, Python or
//! pnpm workspace holds an analytics or error-reporting client, so neither
//! the engine nor an SDK can send one.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context as _, Result};
use serde::Deserialize;

use crate::check::Finding;

/// The pnpm workspace's lockfile, the one in YAML.
const PNPM_LOCKFILE: &str = "pnpm-lock.yaml";

/// The settings catalog `xtask codegen` writes.
const CATALOG: &str = "crates/goodfetch-protocol/generated/catalog.json";

/// Words no setting key may hold: each names a way to report use.
const TELEMETRY_WORDS: [&str; 6] = [
    "telemetry",
    "analytics",
    "phone_home",
    "usage_report",
    "usage_statistics",
    "crash_report",
];

/// Analytics and error-reporting clients, by lockfile: an exact package name,
/// or a scope ending in `/` whose every package is one.
const CLIENTS: [(&str, &[&str]); 3] = [
    (
        "Cargo.lock",
        &[
            "sentry",
            "sentry-core",
            "posthog-rs",
            "segment",
            "mixpanel",
            "amplitude",
            "rudderanalytics",
            "bugsnag",
            "rollbar",
            "honeybadger",
        ],
    ),
    (
        "uv.lock",
        &[
            "sentry-sdk",
            "posthog",
            "segment-analytics-python",
            "analytics-python",
            "mixpanel",
            "amplitude-analytics",
            "rudder-sdk-python",
            "bugsnag",
            "rollbar",
            "honeybadger",
        ],
    ),
    (
        PNPM_LOCKFILE,
        &[
            "@sentry/",
            "posthog-js",
            "posthog-node",
            "@segment/",
            "analytics-node",
            "mixpanel",
            "mixpanel-browser",
            "@amplitude/",
            "@vercel/analytics",
            "@vercel/speed-insights",
            "@datadog/",
            "@bugsnag/",
            "rollbar",
            "logrocket",
            "@rudderstack/",
            "react-ga",
            "react-ga4",
        ],
    ),
];

#[derive(Deserialize)]
struct Catalog {
    settings: Vec<Setting>,
}

#[derive(Deserialize)]
struct Setting {
    name: String,
}

/// `Cargo.lock` and `uv.lock`: a `[[package]]` table each.
#[derive(Deserialize)]
struct TomlLockfile {
    #[serde(rename = "package", default)]
    packages: Vec<Package>,
}

#[derive(Deserialize)]
struct Package {
    name: String,
}

/// One document of `pnpm-lock.yaml`, its packages keyed `<name>@<version>`.
#[derive(Deserialize)]
struct PnpmLockfile {
    #[serde(default)]
    packages: BTreeMap<String, serde_json::Value>,
}

pub(crate) fn check(root: &Path) -> Result<Vec<Finding>> {
    let mut findings: Vec<Finding> = settings(root)?
        .into_iter()
        .filter(|name| TELEMETRY_WORDS.iter().any(|word| name.contains(word)))
        .map(|name| {
            Finding::new(
                "telemetry-setting",
                name,
                "a setting that could turn telemetry on, which ADR 0043 refuses",
            )
        })
        .collect();
    for (lockfile, clients) in CLIENTS {
        for package in packages(root, lockfile)? {
            if clients.iter().any(|client| is_client(client, &package)) {
                findings.push(Finding::new(
                    "analytics-client",
                    lockfile,
                    format!("`{package}` reports use, which ADR 0043 refuses"),
                ));
            }
        }
    }
    Ok(findings)
}

fn is_client(client: &str, package: &str) -> bool {
    if client.ends_with('/') {
        package.starts_with(client)
    } else {
        package == client
    }
}

fn read(root: &Path, file: &str) -> Result<String> {
    let path = root.join(file);
    std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))
}

fn settings(root: &Path) -> Result<Vec<String>> {
    let catalog: Catalog = serde_json::from_str(&read(root, CATALOG)?)
        .with_context(|| format!("parsing {CATALOG}"))?;
    Ok(catalog
        .settings
        .into_iter()
        .map(|setting| setting.name)
        .collect())
}

/// Every package name `lockfile` holds.
fn packages(root: &Path, lockfile: &str) -> Result<Vec<String>> {
    let text = read(root, lockfile)?;
    let parsing = || format!("parsing {lockfile}");
    if lockfile == PNPM_LOCKFILE {
        let documents: Vec<PnpmLockfile> =
            serde_saphyr::from_multiple(&text).with_context(parsing)?;
        return Ok(documents
            .into_iter()
            .flat_map(|document| document.packages.into_keys())
            .map(|key| name_of(&key).to_owned())
            .collect());
    }
    let parsed: TomlLockfile = toml::from_str(&text).with_context(parsing)?;
    Ok(parsed
        .packages
        .into_iter()
        .map(|package| package.name)
        .collect())
}

/// The name in a pnpm package key `<name>@<version>`, whose name may begin
/// with its scope's `@`.
fn name_of(key: &str) -> &str {
    match key.get(1..).and_then(|rest| rest.find('@')) {
        Some(at) => &key[..=at],
        None => key,
    }
}
