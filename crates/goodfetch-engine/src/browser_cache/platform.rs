//! Which browser a launch uses on a platform (ADR 0024): Chrome for Testing
//! by default, its win64 build under emulation on Windows arm64, the
//! distribution's Chromium where Chrome for Testing has no Linux build (musl
//! hosts and Linux arm64), each with a notice saying so, or an installed
//! Chrome named by its channel or path. A pure rule over a platform value,
//! with the places each channel installs to.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::io::Read;
use std::path::PathBuf;
use std::sync::LazyLock;

use goodfetch_protocol::browser_cache::ChromeChannel;

use super::artifact::BuildPlatform;

/// The name Chrome for Testing's builds are pinned under.
pub const CHROME_FOR_TESTING: &str = "chrome-for-testing";

/// An operating system the engine runs on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Os {
    /// A Linux host with glibc.
    LinuxGnu,
    /// A Linux host with musl, such as Alpine.
    LinuxMusl,
    /// macOS.
    Mac,
    /// Windows.
    Windows,
}

/// A processor architecture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arch {
    /// x86-64.
    X64,
    /// arm64.
    Arm64,
}

/// The platform the engine runs on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Platform {
    /// Its operating system.
    pub os: Os,
    /// Its architecture.
    pub arch: Arch,
}

impl Platform {
    /// The platform this executable runs on. A static musl build runs on
    /// glibc and musl hosts alike (ADR 0024), so it takes the host's C
    /// library, read once per process.
    pub fn current() -> Self {
        let os = if cfg!(target_os = "macos") {
            Os::Mac
        } else if cfg!(target_os = "windows") {
            Os::Windows
        } else if cfg!(target_env = "musl") {
            *MUSL_BUILD_HOST
        } else {
            Os::LinuxGnu
        };
        let arch = if cfg!(target_arch = "aarch64") {
            Arch::Arm64
        } else {
            Arch::X64
        };
        Self { os, arch }
    }

    /// The platform of the builds this one runs: Windows on arm64 runs
    /// x86-64 builds under emulation, and no build is made for musl.
    pub fn build_platform(self) -> Option<BuildPlatform> {
        match (self.os, self.arch) {
            (Os::LinuxGnu, Arch::X64) => Some(BuildPlatform::LinuxX64),
            (Os::LinuxGnu, Arch::Arm64) => Some(BuildPlatform::LinuxArm64),
            (Os::Mac, Arch::Arm64) => Some(BuildPlatform::MacArm64),
            (Os::Mac, Arch::X64) => Some(BuildPlatform::MacX64),
            (Os::Windows, Arch::X64 | Arch::Arm64) => Some(BuildPlatform::WindowsX64),
            (Os::LinuxMusl, Arch::X64 | Arch::Arm64) => None,
        }
    }
}

/// The C library of the Linux host a musl build runs on.
static MUSL_BUILD_HOST: LazyLock<Os> = LazyLock::new(|| linux_os(host_ldd().as_deref()));

/// Where a Linux host keeps `ldd`, which names its C library.
const LDD: &str = "/usr/bin/ldd";

/// How much of `ldd` is read: glibc's and musl's name theirs in their
/// first lines, and glibc's whole script is about 5 KiB.
const LDD_READ_LIMIT: u64 = 64 * 1024;

/// The host's `ldd`, `None` when it is missing or unreadable.
#[expect(
    clippy::disallowed_methods,
    reason = "read once per process, through MUSL_BUILD_HOST: one small script, in microseconds"
)]
fn host_ldd() -> Option<Vec<u8>> {
    let mut text = Vec::new();
    std::fs::File::open(LDD)
        .ok()?
        .take(LDD_READ_LIMIT)
        .read_to_end(&mut text)
        .ok()?;
    Some(text)
}

/// The C library of the Linux host whose `ldd` reads `ldd` (`None` when it
/// is missing or unreadable), for a musl build: glibc where `ldd` is
/// glibc's, as Node's `detect-libc` reads it, else musl, the build's own.
/// glibc's loader does not tell, since Alpine's `gcompat` installs it.
pub(super) fn linux_os(ldd: Option<&[u8]>) -> Os {
    let names =
        |word: &[u8]| ldd.is_some_and(|text| text.windows(word.len()).any(|window| window == word));
    if names(b"GNU C Library") && !names(b"musl") {
        Os::LinuxGnu
    } else {
        Os::LinuxMusl
    }
}

/// The browser a launch is asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BrowserChoice {
    /// The default: Chrome for Testing, from the Browser cache.
    Default,
    /// An installed Chrome, by its channel.
    Channel(ChromeChannel),
    /// A browser at this path.
    Path(PathBuf),
}

/// A message the engine gives when it runs another build than asked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlatformNotice {
    /// Windows on arm64 runs Chrome for Testing's win64 build under
    /// emulation.
    WindowsArm64Emulation,
    /// Chrome for Testing has no build for this Linux, which uses its
    /// distribution's Chromium.
    DistributionChromium,
}

impl fmt::Display for PlatformNotice {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::WindowsArm64Emulation => {
                "Chrome for Testing has no Windows arm64 build: running its win64 build under emulation"
            }
            Self::DistributionChromium => {
                "Chrome for Testing has no build for this Linux (musl, or arm64): using the distribution's Chromium"
            }
        })
    }
}

/// Where a launch's browser comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BrowserSource {
    /// A pinned build of the Browser cache.
    Pinned {
        /// The name it is pinned under.
        name: &'static str,
        /// The platform of the build.
        platform: BuildPlatform,
        /// What the engine says, when it runs another build than asked.
        notice: Option<PlatformNotice>,
    },
    /// An installed browser, at the first of these paths that exists.
    Installed {
        /// Where to look, in order.
        candidates: Vec<PathBuf>,
        /// What the engine says, when it runs another browser than asked.
        notice: Option<PlatformNotice>,
    },
}

/// Where `choice` comes from on `platform`. `environment` gives Windows'
/// install folders (`LOCALAPPDATA`, `PROGRAMFILES`, `PROGRAMFILES(X86)`).
pub fn browser_source(
    platform: Platform,
    choice: &BrowserChoice,
    environment: &BTreeMap<OsString, OsString>,
) -> BrowserSource {
    match choice {
        BrowserChoice::Default => default_source(platform),
        BrowserChoice::Channel(channel) => BrowserSource::Installed {
            candidates: channel_paths(platform.os, *channel, environment),
            notice: None,
        },
        BrowserChoice::Path(path) => BrowserSource::Installed {
            candidates: vec![path.clone()],
            notice: None,
        },
    }
}

fn default_source(platform: Platform) -> BrowserSource {
    let pinned = |platform, notice| BrowserSource::Pinned {
        name: CHROME_FOR_TESTING,
        platform,
        notice,
    };
    match (platform.os, platform.arch) {
        (Os::LinuxGnu, Arch::X64) => pinned(BuildPlatform::LinuxX64, None),
        (Os::Mac, Arch::Arm64) => pinned(BuildPlatform::MacArm64, None),
        (Os::Mac, Arch::X64) => pinned(BuildPlatform::MacX64, None),
        (Os::Windows, Arch::X64) => pinned(BuildPlatform::WindowsX64, None),
        (Os::Windows, Arch::Arm64) => pinned(
            BuildPlatform::WindowsX64,
            Some(PlatformNotice::WindowsArm64Emulation),
        ),
        (Os::LinuxMusl, _) | (Os::LinuxGnu, Arch::Arm64) => BrowserSource::Installed {
            candidates: [
                "/usr/bin/chromium",
                "/usr/bin/chromium-browser",
                "/usr/lib/chromium/chromium",
            ]
            .into_iter()
            .map(PathBuf::from)
            .collect(),
            notice: Some(PlatformNotice::DistributionChromium),
        },
    }
}

fn channel_paths(
    os: Os,
    channel: ChromeChannel,
    environment: &BTreeMap<OsString, OsString>,
) -> Vec<PathBuf> {
    match os {
        Os::LinuxGnu | Os::LinuxMusl => match channel {
            ChromeChannel::Stable => vec!["/opt/google/chrome/chrome".into()],
            ChromeChannel::Beta => vec!["/opt/google/chrome-beta/chrome".into()],
            ChromeChannel::Dev => vec!["/opt/google/chrome-unstable/chrome".into()],
            ChromeChannel::Canary => Vec::new(),
        },
        Os::Mac => {
            let application = match channel {
                ChromeChannel::Stable => "Google Chrome",
                ChromeChannel::Beta => "Google Chrome Beta",
                ChromeChannel::Dev => "Google Chrome Dev",
                ChromeChannel::Canary => "Google Chrome Canary",
            };
            vec![
                PathBuf::from(format!("/Applications/{application}.app/Contents/MacOS"))
                    .join(application),
            ]
        }
        Os::Windows => {
            let (folder, folders): (&str, &[&str]) = match channel {
                ChromeChannel::Stable => (
                    "Chrome",
                    &["LOCALAPPDATA", "PROGRAMFILES", "PROGRAMFILES(X86)"],
                ),
                ChromeChannel::Beta => (
                    "Chrome Beta",
                    &["LOCALAPPDATA", "PROGRAMFILES", "PROGRAMFILES(X86)"],
                ),
                ChromeChannel::Dev => (
                    "Chrome Dev",
                    &["LOCALAPPDATA", "PROGRAMFILES", "PROGRAMFILES(X86)"],
                ),
                ChromeChannel::Canary => ("Chrome SxS", &["LOCALAPPDATA"]),
            };
            folders
                .iter()
                .filter_map(|variable| environment.get(OsStr::new(variable)))
                .map(|root| {
                    PathBuf::from(root)
                        .join("Google")
                        .join(folder)
                        .join("Application")
                        .join("chrome.exe")
                })
                .collect()
        }
    }
}
