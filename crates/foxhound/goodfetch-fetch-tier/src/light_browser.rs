//! The light-browser tier's out-of-process bridge (spec #579; ADRs 0020
//! and 0093): Lightpanda, which runs a page's scripts without layout, is
//! never bundled, so its AGPL licence stays outside the distribution. Its
//! build is a Pinned artifact of Foxhound's (`pins.json`: a single
//! executable per platform, by its release's URL and digest), downloaded on
//! first use into the Browser cache with the digest check and held there
//! while the engine runs it.
//!
//! Each run on the tier starts one Lightpanda process, `lightpanda fetch`,
//! which loads the request's URL, runs its scripts until the network is
//! idle (at most [`WAIT_MS`]), and writes the document they left as JSON
//! on its standard output, which the engine reads through a pipe; the
//! process then exits, and dropping the run kills it. Every request it
//! sends goes through the run's loopback [`bridge`], which connects only
//! where the Run's HTTP tier may (ADR 0017). Its telemetry is off.
//!
//! A platform with no pinned build (Windows, musl) cannot take the tier.

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;

use goodfetch_engine::CallError;
use goodfetch_engine::browser_cache::{
    BrowserCache, BuildPlatform, CacheLocation, HeldArtifact, PinnedArtifact, Platform,
};
use goodfetch_engine::children::Children;
use goodfetch_engine::home::Home;
use goodfetch_engine::pinned_download::Fetching;
use goodfetch_engine::response_limits::Limits;
use goodfetch_engine::runs::Run;
use goodfetch_protocol::browser_cache::{ArtifactId, ArtifactKind, PinnedBy};
use goodfetch_protocol::page::{NAVIGATION_FAILED, NavigationFailedDetails};
use serde::Deserialize;
use serde_json::Value;
use tokio::io::AsyncReadExt as _;
use url::Url;

pub(crate) mod bridge;

/// The pinned builds, by platform.
const PINS: &str = include_str!("light_browser/pins.json");

/// The Browser cache's name for Lightpanda's builds.
pub const LIGHTPANDA: &str = "lightpanda";

/// Where Lightpanda's release assets download from, by build.
const RELEASES: &str = "https://github.com/lightpanda-io/browser/releases/download";

/// How long Lightpanda waits for a page's network to go idle.
pub(crate) const WAIT_MS: u64 = 10_000;

/// How long its scripts may run before it stops them.
const TERMINATE_MS: u64 = 30_000;

/// The Browser cache's name for a platform, as `pins.json` spells it.
fn platform_named(name: &str) -> Option<BuildPlatform> {
    Some(match name {
        "linux-x64" => BuildPlatform::LinuxX64,
        "linux-arm64" => BuildPlatform::LinuxArm64,
        "mac-arm64" => BuildPlatform::MacArm64,
        "mac-x64" => BuildPlatform::MacX64,
        _ => return None,
    })
}

/// Every Lightpanda build Foxhound's version pins, one per platform.
///
/// # Panics
///
/// When `pins.json` is not what it should be, which a test catches.
pub fn pins() -> Vec<PinnedArtifact> {
    let pins: Value = serde_json::from_str(PINS).expect("invariant: pins.json is JSON");
    let lightpanda = &pins["lightpanda"];
    let build = lightpanda["build"]
        .as_str()
        .expect("invariant: pins.json names its Lightpanda build");
    lightpanda["platforms"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(platform, pinned)| {
            let text = |key: &str| {
                pinned[key]
                    .as_str()
                    .unwrap_or_else(|| panic!("invariant: pins.json gives {platform} its {key}"))
            };
            let digest = |key: &str| {
                text(key)
                    .parse()
                    .expect("invariant: pins.json's digests read")
            };
            PinnedArtifact {
                id: ArtifactId {
                    kind: ArtifactKind::Browser,
                    name: LIGHTPANDA.to_owned(),
                    build: build.to_owned(),
                },
                platform: Some(
                    platform_named(platform)
                        .unwrap_or_else(|| panic!("invariant: pins.json names no `{platform}`")),
                ),
                url: format!("{RELEASES}/{build}/{}", text("asset")),
                archive_digest: Some(digest("archive_digest")),
                digest: digest("digest"),
                // Lightpanda is no Chromium the relay presents.
                application_settings: std::collections::BTreeMap::new(),
            }
        })
        .collect()
}

/// The Lightpanda build pinned for this platform, if it has one.
pub(crate) fn pinned_here() -> Option<PinnedArtifact> {
    let platform = Platform::current().build_platform()?;
    pins()
        .into_iter()
        .find(|pin| pin.platform == Some(platform))
}

/// A Lightpanda build in the Browser cache, held while it may run.
pub struct Prepared {
    /// Its executable.
    pub executable: PathBuf,
    /// Whether it was downloaded now.
    pub downloaded: bool,
    _held: HeldArtifact,
}

/// The build `pin` from the Browser cache `cache`, downloaded into it
/// first when it holds none, its digest checked either way.
///
/// # Errors
///
/// The Browser cache's Error codes: `browser_not_installed`,
/// `artifact_unavailable_offline`, `artifact_download_failed`.
///
/// # Cancel safety
///
/// Dropping the future leaves no partial build in place.
pub async fn prepare(
    cache: &BrowserCache,
    pin: &PinnedArtifact,
    pinned_by: &PinnedBy,
    fetching: &Fetching,
    now: jiff::Timestamp,
) -> Result<Prepared, CallError> {
    let prepared = cache
        .prepare(pin, pinned_by, fetching, now, &mut |progress| {
            tracing::debug!(received = progress.received_bytes, total = ?progress.total_bytes, "downloading Lightpanda");
        })
        .await
        .map_err(|error| error.to_call_error())?;
    let name = pin.url.rsplit('/').next().unwrap_or(LIGHTPANDA);
    Ok(Prepared {
        executable: prepared.artifact.files().join(name),
        downloaded: prepared.downloaded,
        _held: prepared.artifact,
    })
}

/// The engine's Lightpanda build, prepared once for every Run that takes
/// the tier, then held while the engine lives.
#[derive(Default)]
pub(crate) struct LightBrowser {
    prepared: tokio::sync::OnceCell<Arc<Prepared>>,
}

impl LightBrowser {
    /// The build pinned for this platform, prepared for `run` in the
    /// Browser cache of `home` by the Product version `pinned_by`.
    ///
    /// # Errors
    ///
    /// [`prepare`]'s, and `browser_not_installed` on a platform with no
    /// pinned build.
    ///
    /// # Cancel safety
    ///
    /// As [`prepare`]'s: dropping the future of the call that prepares the
    /// build leaves none prepared, and the next call prepares it.
    ///
    /// # Panics
    ///
    /// When the clock reads a time outside jiff's range, years -9999 to
    /// 9999.
    pub(crate) async fn prepared(
        &self,
        run: &Run,
        home: &Home,
        pinned_by: &PinnedBy,
    ) -> Result<Arc<Prepared>, CallError> {
        let prepared = self
            .prepared
            .get_or_try_init(|| async {
                let pin = pinned_here().ok_or_else(|| {
                    CallError::new(
                        &NAVIGATION_FAILED,
                        "Lightpanda builds nothing for this platform.",
                        &serde_json::json!({}),
                    )
                })?;
                let cache = BrowserCache::new(&CacheLocation::new(
                    home.root(),
                    home.browsers_path().map(std::path::Path::to_path_buf),
                ));
                let fetching = run.settings().map_or_else(
                    || Fetching::new(run.clock()),
                    |settings| Fetching::of(settings.resolved(), run.clock()),
                );
                let now = run.clock().timestamp();
                prepare(&cache, &pin, pinned_by, &fetching, now)
                    .await
                    .map(Arc::new)
            })
            .await?;
        Ok(Arc::clone(prepared))
    }
}

/// The document Lightpanda left once its scripts ran, as its `fetch`
/// command's JSON reports it.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Rendered {
    /// The URL it ended on, its redirects followed.
    pub(crate) url: Url,
    /// The status its document was answered with.
    pub(crate) status: u16,
    /// That answer's headers, their names lower-cased, in order.
    pub(crate) headers: Vec<(String, String)>,
    /// The document as its scripts left it, serialized.
    pub(crate) html: String,
}

#[derive(Deserialize)]
struct Report {
    url: String,
    http_status: Option<u16>,
    #[serde(default)]
    headers: Vec<Header>,
    content: Option<String>,
    error: Option<String>,
}

#[derive(Deserialize)]
struct Header {
    name: String,
    value: String,
}

/// What Lightpanda's `fetch --json --dump html` wrote, `output`.
///
/// # Errors
///
/// What went wrong: its own error's name, or a report that does not read.
pub(crate) fn rendered(output: &[u8]) -> Result<Rendered, String> {
    let report: Report = serde_json::from_slice(output)
        .map_err(|error| format!("its report does not read: {error}"))?;
    if let Some(error) = report.error {
        return Err(error);
    }
    Ok(Rendered {
        url: Url::parse(&report.url).map_err(|error| format!("it names no URL: {error}"))?,
        status: report.http_status.ok_or("it reached no document")?,
        headers: report
            .headers
            .into_iter()
            .map(|header| (header.name.to_ascii_lowercase(), header.value))
            .collect(),
        html: report.content.ok_or("it dumped no document")?,
    })
}

/// The arguments of a run of `fetch` on `url` through the bridge at
/// `proxy`.
pub(crate) fn arguments(url: &str, proxy: &str) -> Vec<String> {
    [
        "fetch",
        "--json",
        "--dump",
        "html",
        "--log-level",
        "fatal",
        "--wait-until",
        "networkidle",
        "--wait-ms",
        &WAIT_MS.to_string(),
        "--terminate-ms",
        &TERMINATE_MS.to_string(),
        "--http-version",
        "1.1",
        "--http-proxy",
        proxy,
        url,
    ]
    .map(str::to_owned)
    .to_vec()
}

/// Loads `url` in Lightpanda's `executable` for `run`, its requests
/// through a bridge where `check` lets them go, its report read up to
/// `limits`' `network.max_body_size` (ADR 0067).
///
/// # Errors
///
/// `navigation_failed` when Lightpanda cannot start, fails, or reaches no
/// document; `response_limit_exceeded` when its report passes
/// `network.max_body_size`.
///
/// # Cancel safety
///
/// Dropping the future kills the process and closes the bridge.
pub(crate) async fn render(
    run: &Run,
    executable: &std::path::Path,
    url: &str,
    (check, limits): (crate::transports::AddressCheck, &Limits),
) -> Result<Rendered, CallError> {
    let failed = |reason: String| {
        CallError::new(
            &NAVIGATION_FAILED,
            format!("Lightpanda did not load `{url}`: {reason}."),
            &NavigationFailedDetails {
                url: Some(url.to_owned()),
                error: Some("net::ERR_FAILED".to_owned()),
            },
        )
    };
    let bridge = bridge::Bridge::open(run, check)
        .await
        .map_err(|error| failed(error.to_string()))?;
    let mut command = tokio::process::Command::new(executable);
    command
        .args(arguments(url, &bridge.proxy_url()))
        .env_clear()
        .env("LIGHTPANDA_DISABLE_TELEMETRY", "true")
        .current_dir(run.folder())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut group = Children::new().map_err(|error| failed(error.to_string()))?;
    let mut child = group
        .spawn(command)
        .map_err(|error| failed(format!("it does not start: {error}")))?;
    let mut output = Vec::new();
    if let Some(stdout) = child.stdout.take() {
        stdout
            .take(limits.max_body_size.saturating_add(1))
            .read_to_end(&mut output)
            .await
            .map_err(|error| failed(error.to_string()))?;
    }
    let read = u64::try_from(output.len()).unwrap_or(u64::MAX);
    limits
        .judge_decoded(url, read, None)
        .map_err(|exceeded| exceeded.error())?;
    let status = child
        .wait()
        .await
        .map_err(|error| failed(error.to_string()))?;
    drop(bridge);
    match rendered(&output) {
        Ok(rendered) => Ok(rendered),
        Err(reason) if status.success() => Err(failed(reason)),
        Err(reason) => Err(failed(format!("{reason} ({status})"))),
    }
}

#[cfg(test)]
mod tests;
