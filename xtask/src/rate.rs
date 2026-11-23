//! What a minute of a GitHub-hosted runner counts in Linux-equivalent
//! minutes (ADR 0108), the one rule both minute ceilings count by: the
//! Release runs' 600 and test health's 1,000, and whether that minute
//! bills at all: the Release runs' ceiling counts only minutes that bill.

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

/// The labels of GitHub's standard hosted runners ([GitHub-hosted
/// runners](https://docs.github.com/en/actions/reference/runners/github-hosted-runners),
/// checked 2026-11-23). Any other hosted label is a larger runner's, so a
/// ceiling never counts less than it should.
const STANDARD_LABELS: [&str; 20] = [
    "ubuntu-slim",
    "ubuntu-latest",
    "ubuntu-26.04",
    "ubuntu-24.04",
    "ubuntu-22.04",
    "ubuntu-26.04-arm",
    "ubuntu-24.04-arm",
    "ubuntu-22.04-arm",
    "windows-latest",
    "windows-2025",
    "windows-2025-vs2026",
    "windows-2022",
    "windows-11-arm",
    "windows-11-vs2026-arm",
    "macos-latest",
    "macos-26",
    "macos-15",
    "macos-26-intel",
    "macos-15-intel",
    "xcode-27",
];

/// Whether a hosted runner is one of GitHub's standard runners or a larger
/// runner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Size {
    Standard,
    Larger,
}

/// A repository's visibility, which decides whether its standard runners
/// bill.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Visibility {
    Private,
    Public,
}

/// A GitHub-hosted runner, as a ceiling of included minutes counts it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Hosted {
    pub(crate) rate: Rate,
    pub(crate) size: Size,
}

impl Hosted {
    /// The hosted runner of a job from its runner labels; nothing for a
    /// self-hosted runner. It is standard only when every label is a
    /// standard runner's.
    pub(crate) fn of_labels(labels: &[String]) -> Option<Self> {
        let rate = Rate::of_labels(labels)?;
        let standard = !labels.is_empty()
            && labels
                .iter()
                .all(|label| STANDARD_LABELS.contains(&label.to_ascii_lowercase().as_str()));
        Some(Self {
            rate,
            size: if standard {
                Size::Standard
            } else {
                Size::Larger
            },
        })
    }

    /// Whether a minute on this runner bills in a repository of
    /// `visibility`: a standard runner of a public repository is free and
    /// draws on no included minutes, and a larger runner bills in any
    /// repository ([GitHub Actions
    /// billing](https://docs.github.com/en/billing/concepts/product-billing/github-actions),
    /// checked 2026-11-22; ADR 0108's note of 2026-11-23).
    pub(crate) fn bills_in(self, visibility: Visibility) -> bool {
        match (self.size, visibility) {
            (Size::Standard, Visibility::Public) => false,
            (Size::Standard, Visibility::Private) | (Size::Larger, _) => true,
        }
    }

    /// What `minutes` on this runner count in Linux-equivalent minutes in a
    /// repository of `visibility`: nothing when they do not bill.
    pub(crate) fn counted(self, minutes: u32, visibility: Visibility) -> u32 {
        if self.bills_in(visibility) {
            minutes * self.rate.multiplier()
        } else {
            0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(labels: &[&str]) -> Vec<String> {
        labels.iter().map(|&label| label.to_owned()).collect()
    }

    #[rstest::rstest]
    #[case::ubuntu(&["ubuntu-24.04"], Some((Rate::Linux, Size::Standard)))]
    #[case::ubuntu_arm(&["ubuntu-24.04-arm"], Some((Rate::Linux, Size::Standard)))]
    #[case::windows(&["windows-2025"], Some((Rate::Windows, Size::Standard)))]
    #[case::macos_intel(&["macos-15-intel"], Some((Rate::Macos, Size::Standard)))]
    #[case::named_larger_runner(&["ubuntu-24.04-16core"], Some((Rate::Linux, Size::Larger)))]
    #[case::macos_larger_runner(&["macos-15-xlarge"], Some((Rate::Macos, Size::Larger)))]
    #[case::self_hosted(&["self-hosted", "owner-machine"], None)]
    fn a_jobs_labels_give_its_runners_rate_and_size(
        #[case] job: &[&str],
        #[case] hosted: Option<(Rate, Size)>,
    ) {
        assert_eq!(
            Hosted::of_labels(&labels(job)).map(|runner| (runner.rate, runner.size)),
            hosted
        );
    }
}
