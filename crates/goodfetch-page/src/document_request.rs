//! The request a `standard` Session's browser sends for a document (ADR
//! 0021; issue 2315): its `User-Agent` and the headers of a navigation, in
//! the browser's order, for a client that sends a Session's HTTP without
//! its browser and must present the same identity.
//!
//! It is read from the build the Session's browser would launch, never
//! from a launched browser, so a Session that only ever sends HTTP starts
//! none: the pinned build's ID, or, where the browser is an installed one
//! (the distribution's Chromium on Linux arm64 and musl), the version its
//! executable prints for `--version`, read once per Run (issue 2532).
//! Chromium fixes all of it by its major version and its platform:
//!
//! - **The user agent** is Chrome's reduced one, the window's (the
//!   `driver`'s `identity` module gives a headless page the same): its
//!   platform part is one constant for each operating system and its
//!   version is the major alone.
//! - **The brands** of `Sec-CH-UA` are `Chromium` and one made-up brand,
//!   whose name, version and place Chromium derives from the major (its
//!   `GenerateBrandVersionList`).
//! - **The client hints and fetch metadata** go only to a potentially
//!   trustworthy origin, `https:` or loopback; elsewhere the browser also
//!   asks for `gzip` and `deflate` alone.
//!
//! `Accept-Language` is what the build sends under its default locale,
//! `en-US`. Foxhound's test of a `standard` Session's HTTP tier compares
//! these headers with the pinned build's own navigation, so a pin whose
//! browser sends another request fails it.

use goodfetch_engine::browser_cache::{
    BrowserChoice, BrowserSource, BuildPlatform, Os, PinnedArtifact, Platform, browser_source,
    installed_browser,
};
use goodfetch_engine::children::Children;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::runs::Run;
use goodfetch_engine::tasks::BlockingThreads;
use goodfetch_protocol::browser_cache::ArtifactKind;
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::foxhound::report::BROWSER_VERSION_UNREAD;
use goodfetch_protocol::run::RunWarning;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::time::Duration;
use tokio::io::AsyncReadExt as _;
use tokio::process::Command;
use url::{Host, Url};

/// How long an installed browser has to print its version and exit.
const VERSION_READ: Duration = Duration::from_secs(10);

/// The most of a browser's `--version` output read.
const PRINTED_MOST: u64 = 4096;

/// What Chrome accepts for a document.
const ACCEPT: &str = "text/html,application/xhtml+xml,application/xml;q=0.9,image/jxl,image/avif,image/webp,image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.7";

/// What Chrome sends under its default locale.
const ACCEPT_LANGUAGE: &str = "en-US,en;q=0.9";

/// The characters Chromium draws the made-up brand's two separators from.
const BRAND_SEPARATORS: [&str; 11] = [" ", "(", ":", "-", ".", "/", ")", ";", "=", "?", "_"];

/// The versions Chromium draws the made-up brand's from.
const BRAND_VERSIONS: [&str; 3] = ["8", "99", "24"];

/// The document request of one browser build on one platform.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocumentRequest {
    /// The headers a potentially trustworthy origin receives.
    trustworthy: Vec<(String, String)>,
    /// The headers every other origin receives.
    elsewhere: Vec<(String, String)>,
}

impl DocumentRequest {
    /// The document request of the build a Session's browser launches on
    /// this platform, among `pins`: none where the platform has no pinned
    /// build and the browser is an installed one, whose version only a
    /// launch tells.
    fn of_pinned(pins: &[PinnedArtifact]) -> Option<Self> {
        let BrowserSource::Pinned { name, platform, .. } = browser_source(
            Platform::current(),
            &BrowserChoice::Default,
            &BTreeMap::new(),
        ) else {
            return None;
        };
        let pinned = pins.iter().find(|artifact| {
            artifact.id.kind == ArtifactKind::Browser
                && artifact.id.name == name
                && artifact.platform == Some(platform)
        })?;
        Self::of_build(&pinned.id.build, platform)
    }

    /// The document request of the Chrome for Testing build `build`, such
    /// as `141.0.7390.54-linux64`, on `platform`: none for a build ID that
    /// does not start with a major version.
    pub fn of_build(build: &str, platform: BuildPlatform) -> Option<Self> {
        let major: usize = build.split('.').next()?.parse().ok()?;
        let (system, name) = match platform {
            BuildPlatform::LinuxX64 | BuildPlatform::LinuxArm64 => ("X11; Linux x86_64", "Linux"),
            BuildPlatform::MacArm64 | BuildPlatform::MacX64 => {
                ("Macintosh; Intel Mac OS X 10_15_7", "macOS")
            }
            BuildPlatform::WindowsX64 => ("Windows NT 10.0; Win64; x64", "Windows"),
        };
        let user_agent = format!(
            "Mozilla/5.0 ({system}) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/{major}.0.0.0 Safari/537.36"
        );
        let brands = brands(major);
        let platform = format!("\"{name}\"");
        let asked = [
            ("upgrade-insecure-requests", "1"),
            ("user-agent", user_agent.as_str()),
            ("accept", ACCEPT),
        ];
        let hints = [
            ("sec-ch-ua", brands.as_str()),
            ("sec-ch-ua-mobile", "?0"),
            ("sec-ch-ua-platform", platform.as_str()),
        ];
        let metadata = [
            ("sec-fetch-site", "none"),
            ("sec-fetch-mode", "navigate"),
            ("sec-fetch-user", "?1"),
            ("sec-fetch-dest", "document"),
        ];
        let owned = |headers: &[&[(&str, &str)]]| {
            headers
                .concat()
                .into_iter()
                .map(|(name, value)| (name.to_owned(), value.to_owned()))
                .collect()
        };
        Some(Self {
            trustworthy: owned(&[
                &hints,
                &asked,
                &metadata,
                &[
                    ("accept-encoding", "gzip, deflate, br, zstd"),
                    ("accept-language", ACCEPT_LANGUAGE),
                ],
            ]),
            elsewhere: owned(&[
                &asked,
                &[
                    ("accept-encoding", "gzip, deflate"),
                    ("accept-language", ACCEPT_LANGUAGE),
                ],
            ]),
        })
    }

    /// The document request of the browser a Session of `run` would
    /// launch, among `pins`: the pinned build's, or the installed
    /// browser's, read from the version its executable prints. None where
    /// neither tells it, of which `run` is warned.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future kills the version read's processes and may leave
    /// the warning unwritten.
    pub async fn of_browser(pins: &[PinnedArtifact], run: &Run) -> Option<Self> {
        let BrowserSource::Installed { candidates, .. } = browser_source(
            Platform::current(),
            &BrowserChoice::Default,
            &BTreeMap::new(),
        ) else {
            return Self::of_pinned(pins);
        };
        match Self::of_installed(&candidates, &run.owner().blocking_threads(), &run.clock()).await {
            Ok(request) => Some(request),
            Err(unread) => {
                if let Err(error) = run.warn(unread.warning()).await {
                    tracing::warn!(%error, "the warning that the installed browser's version was not read was not kept");
                }
                None
            }
        }
    }

    /// The document request of the browser installed at the first of
    /// `candidates` that exists, on this platform: that of the version its
    /// executable prints for `--version`, which prints it and exits
    /// without starting a browser. The executable has 10 seconds to exit,
    /// and no process it started outlives the read.
    ///
    /// # Errors
    ///
    /// [`Unread`] when no candidate exists, the executable could not be
    /// started, or it printed no version, exited with an error or did not
    /// exit in time.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future kills the executable and every process it
    /// started.
    pub async fn of_installed(
        candidates: &[PathBuf],
        blocking: &BlockingThreads,
        clock: &Clock,
    ) -> Result<Self, Unread> {
        // The launcher's own lookup, so the version read is the executable a
        // launch would start; its error, worded for a launch, is not kept.
        let found = installed_browser(
            blocking,
            &BrowserChoice::Default,
            candidates,
            Product::Foxhound,
        )
        .await
        .map_err(|_| Unread::NotInstalled {
            looked_in: candidates.to_vec(),
        })?;
        let printed = printed_version(&found, clock)
            .await
            .map_err(|error| Unread::Unstarted {
                executable: found.clone(),
                error: error.to_string(),
            })?;
        let platform = user_agent_platform(Platform::current());
        printed
            .as_deref()
            .and_then(version_in)
            .and_then(|version| Self::of_build(version, platform))
            .ok_or(Unread::NoVersion { executable: found })
    }

    /// The headers of the request for the document at `url`, their names
    /// in lowercase, in the browser's order.
    #[must_use]
    pub fn headers(&self, url: &Url) -> &[(String, String)] {
        if potentially_trustworthy(url) {
            &self.trustworthy
        } else {
            &self.elsewhere
        }
    }
}

/// Why an installed browser's document request could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Unread {
    /// No browser is installed at any of the paths.
    NotInstalled {
        /// The paths, in the order they were looked in.
        looked_in: Vec<PathBuf>,
    },
    /// The executable could not be started, such as one the engine's user
    /// may not run.
    Unstarted {
        /// The executable.
        executable: PathBuf,
        /// Why, as the system said it.
        error: String,
    },
    /// The executable printed no version for `--version`, exited with an
    /// error or did not exit in time.
    NoVersion {
        /// The executable.
        executable: PathBuf,
    },
}

impl Unread {
    /// The warning a Run gets for it (`browser_version_unread`), naming the
    /// fix.
    #[must_use]
    pub fn warning(&self) -> RunWarning {
        let message = match self {
            Self::NotInstalled { looked_in } => format!(
                "No browser is installed at {}, so a `standard` Session's own HTTP transport presents no browser identity: install the distribution's Chromium package.",
                looked_in
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", "),
            ),
            Self::Unstarted { executable, error } => format!(
                "The browser at {} could not be started for `--version` ({error}), so a `standard` Session's own HTTP transport presents no browser identity: let the engine's user run it, or reinstall the distribution's Chromium package.",
                executable.display(),
            ),
            Self::NoVersion { executable } => format!(
                "The browser at {} printed no version for `--version`, so a `standard` Session's own HTTP transport presents no browser identity: reinstall the distribution's Chromium package.",
                executable.display(),
            ),
        };
        RunWarning {
            code: BROWSER_VERSION_UNREAD.to_owned(),
            message,
            sessions: Vec::new(),
            folder: None,
        }
    }
}

/// The platform whose user agent and `Sec-CH-UA-Platform` a browser on
/// `platform` presents: Chrome's reduced user agent names one platform for
/// each operating system, whatever its processor.
fn user_agent_platform(platform: Platform) -> BuildPlatform {
    match platform.os {
        Os::LinuxGnu | Os::LinuxMusl => BuildPlatform::LinuxX64,
        Os::Mac => BuildPlatform::MacArm64,
        Os::Windows => BuildPlatform::WindowsX64,
    }
}

/// What the browser at `executable` printed for `--version` before it
/// exited with success within [`VERSION_READ`]; none otherwise. Its process
/// group goes once it exited or its time ran out, every process it started
/// with it.
///
/// # Errors
///
/// When the executable could not be started.
async fn printed_version(executable: &Path, clock: &Clock) -> std::io::Result<Option<String>> {
    let mut command = Command::new(executable);
    command
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut group = Children::new()?;
    let mut child = group.spawn(command)?;
    let mut stdout = child
        .stdout
        .take()
        .expect("invariant: the command's output is piped")
        .take(PRINTED_MOST);
    let deadline = clock.now() + VERSION_READ;
    let exited = tokio::select! {
        status = child.wait() => status.ok().filter(ExitStatus::success),
        () = clock.sleep_until(deadline) => None,
    };
    // A process the browser left behind holds its output open: it goes with
    // the group, which ends the output.
    group.kill_all();
    let mut printed = Vec::new();
    let read = tokio::select! {
        read = stdout.read_to_end(&mut printed) => read.ok(),
        () = clock.sleep_until(deadline) => None,
    };
    let _ = child.wait().await;
    Ok(exited
        .and(read)
        .and_then(|_| String::from_utf8(printed).ok()))
}

/// The version in what a browser printed for `--version`, such as
/// `141.0.7390.54` in `Chromium 141.0.7390.54 built on Debian 12.7`: its
/// first word of numbers joined by dots.
fn version_in(printed: &str) -> Option<&str> {
    printed.split_whitespace().find(|word| {
        word.contains('.')
            && word
                .split('.')
                .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
    })
}

/// The `Sec-CH-UA` brands of Chromium `major`, as its
/// `GenerateBrandVersionList` orders and spells them for a build with no
/// brand of its own: the made-up brand first on an even major.
fn brands(major: usize) -> String {
    let made_up = format!(
        "\"Not{}A{}Brand\";v=\"{}\"",
        BRAND_SEPARATORS[major % BRAND_SEPARATORS.len()],
        BRAND_SEPARATORS[(major + 1) % BRAND_SEPARATORS.len()],
        BRAND_VERSIONS[major % BRAND_VERSIONS.len()],
    );
    let chromium = format!("\"Chromium\";v=\"{major}\"");
    if major.is_multiple_of(2) {
        format!("{made_up}, {chromium}")
    } else {
        format!("{chromium}, {made_up}")
    }
}

/// Whether `url`'s origin is potentially trustworthy (Secure Contexts):
/// `https:`, `localhost`, a name under it, or a loopback address.
fn potentially_trustworthy(url: &Url) -> bool {
    url.scheme() == "https"
        || match url.host() {
            Some(Host::Domain(name)) => {
                let name = name.trim_end_matches('.');
                name.eq_ignore_ascii_case("localhost")
                    || name.to_ascii_lowercase().ends_with(".localhost")
            }
            Some(Host::Ipv4(address)) => address.is_loopback(),
            Some(Host::Ipv6(address)) => address.is_loopback(),
            None => false,
        }
}

#[cfg(test)]
mod tests;
