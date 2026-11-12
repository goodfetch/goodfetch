//! What the notice inventory knows of one third-party dependency or input:
//! its ecosystem or input set, the license its authors state, where its
//! source is and how Goodfetch uses it.

use std::fmt;

use serde::{Deserialize, Serialize};

/// A package ecosystem, or a set of declared inputs (ADRs 0018, 0035, 0043
/// and 0047).
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Ecosystem {
    Cargo,
    Npm,
    Python,
    /// The Goodfetch Chromium's third-party inputs, the variations seed
    /// included (ADR 0043).
    Chromium,
    /// The Clean-up rules' inputs (ADR 0047).
    CleanUpRules,
    /// The sources Collie's block-detection rules are seeded from (ADR
    /// 0035).
    BlockDetectionRules,
}

impl fmt::Display for Ecosystem {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Cargo => "cargo",
            Self::Npm => "npm",
            Self::Python => "python",
            Self::Chromium => "chromium",
            Self::CleanUpRules => "clean-up-rules",
            Self::BlockDetectionRules => "block-detection-rules",
        })
    }
}

/// How Goodfetch uses a dependency or an input, which decides the licenses
/// it may carry (ADRs 0018 and 0047).
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Usage {
    /// Shipped as its authors published it; a package from its registry is.
    Unmodified,
    /// Shipped with changes of Goodfetch's.
    Modified,
    /// A Filter list whose compiled form the executable embeds, an
    /// adaptation (ADR 0047).
    CompiledFilterList,
    /// Never embedded: the engine downloads it, pinned by digest (ADR 0047).
    DownloadOnly,
}

pub(crate) struct Entry {
    pub(crate) ecosystem: Ecosystem,
    pub(crate) name: String,
    pub(crate) version: String,
    /// The license expression its authors state, if they state one.
    pub(crate) license: Option<String>,
    /// Where its source is: its registry page, repository or folder, if
    /// its declaration states it.
    pub(crate) source: Option<String>,
    pub(crate) usage: Usage,
}
