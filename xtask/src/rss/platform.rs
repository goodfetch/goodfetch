//! The platform a peak was measured on, `<os>-<arch>` as the results store
//! names it: each platform has its own ceilings (ADR 0071).

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Platform {
    LinuxX64,
    LinuxArm64,
    MacosX64,
    MacosArm64,
    WindowsX64,
    WindowsArm64,
}

impl Platform {
    /// The platform `xtask` runs on, which is the one the executables it
    /// measures run on.
    pub(crate) fn current() -> Result<Self> {
        Ok(match (std::env::consts::OS, std::env::consts::ARCH) {
            ("linux", "x86_64") => Self::LinuxX64,
            ("linux", "aarch64") => Self::LinuxArm64,
            ("macos", "x86_64") => Self::MacosX64,
            ("macos", "aarch64") => Self::MacosArm64,
            ("windows", "x86_64") => Self::WindowsX64,
            ("windows", "aarch64") => Self::WindowsArm64,
            (os, arch) => bail!("no ceiling platform for {os} on {arch}"),
        })
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::LinuxX64 => "linux-x64",
            Self::LinuxArm64 => "linux-arm64",
            Self::MacosX64 => "macos-x64",
            Self::MacosArm64 => "macos-arm64",
            Self::WindowsX64 => "windows-x64",
            Self::WindowsArm64 => "windows-arm64",
        }
    }
}
