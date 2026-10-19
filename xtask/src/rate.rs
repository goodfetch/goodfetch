//! What a minute of a GitHub-hosted runner counts in Linux-equivalent
//! minutes (ADR 0108), the one rule both minute ceilings count by: the
//! Release runs' 600 and test health's 1,000.

/// A hosted runner's rate in Linux-equivalent minutes: its price per minute
/// over Linux 2-core x64's, rounded up, kept at or above GitHub's earlier
/// multipliers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Rate {
    Linux,
    Windows,
    Macos,
}

impl Rate {
    pub(crate) fn multiplier(self) -> u32 {
        match self {
            Self::Linux => 1,
            Self::Windows => 2,
            Self::Macos => 11,
        }
    }

    /// The rate of a job from its runner labels; nothing for a self-hosted
    /// runner, which uses no included minutes. A hosted runner of no known
    /// image counts as macOS, the highest rate, so a ceiling never counts
    /// less than it should.
    pub(crate) fn of_labels(labels: &[String]) -> Option<Self> {
        let lowered: Vec<String> = labels
            .iter()
            .map(|label| label.to_ascii_lowercase())
            .collect();
        let has = |prefix: &str| lowered.iter().any(|label| label.starts_with(prefix));
        if lowered
            .iter()
            .any(|label| label == "self-hosted" || label == "owner-machine")
        {
            None
        } else if has("ubuntu") {
            Some(Self::Linux)
        } else if has("windows") {
            Some(Self::Windows)
        } else {
            Some(Self::Macos)
        }
    }
}
