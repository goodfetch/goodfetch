//! The cells of the Release suite's private-period matrix (ADR 0108): where
//! each runs, the Rust target it builds, the runtimes it tests (ADR 0069) and
//! what a minute of its hosted runner counts in Linux-equivalent minutes.

use std::fmt;

use anyhow::{Result, bail};

use crate::runtimes::Set;

pub(crate) use crate::rate::Rate;

/// Where a cell runs while the repository is private (ADR 0108).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Runner {
    Hosted(Rate),
    /// The owner's machine, on `main` only; it uses no included minutes.
    OwnerMachine,
}

/// The Linux jobs of the Release workflow: one per architecture, whose
/// matrix is that architecture's started cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LinuxJob {
    X64,
    Arm64,
}

impl LinuxJob {
    pub(crate) const ALL: [Self; 2] = [Self::X64, Self::Arm64];

    /// The job's output that holds its matrix.
    pub(crate) fn output(self) -> &'static str {
        match self {
            Self::X64 => "linux-x64",
            Self::Arm64 => "linux-arm64",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Cell {
    LinuxX64Gnu,
    LinuxX64Musl,
    LinuxArm64Gnu,
    LinuxArm64Musl,
    WindowsX64,
    MacosArm64,
    MacosX64,
}

impl Cell {
    /// Every cell of the private-period matrix. Native macOS Intel and
    /// Windows arm64 first run once the repository is public (ADR 0108).
    pub(crate) const ALL: [Self; 7] = [
        Self::LinuxX64Gnu,
        Self::LinuxX64Musl,
        Self::LinuxArm64Gnu,
        Self::LinuxArm64Musl,
        Self::WindowsX64,
        Self::MacosArm64,
        Self::MacosX64,
    ];

    /// The name `just release-ci` and the workflow's `cells` input take.
    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::LinuxX64Gnu => "linux-x64-gnu",
            Self::LinuxX64Musl => "linux-x64-musl",
            Self::LinuxArm64Gnu => "linux-arm64-gnu",
            Self::LinuxArm64Musl => "linux-arm64-musl",
            Self::WindowsX64 => "windows-x64",
            Self::MacosArm64 => "macos-arm64",
            Self::MacosX64 => "macos-x64",
        }
    }

    /// The name of the cell's job, the cell's name everywhere it is shown:
    /// the translated cell says so (ADR 0108).
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::MacosX64 => "macos-x64 (Rosetta 2)",
            other => other.key(),
        }
    }

    pub(crate) fn parse(key: &str) -> Result<Self> {
        match Self::ALL.into_iter().find(|cell| cell.key() == key.trim()) {
            Some(cell) => Ok(cell),
            None => bail!(
                "`{}` is not a cell; the cells are {}",
                key.trim(),
                Self::ALL.map(Self::key).join(", ")
            ),
        }
    }

    pub(crate) fn runner(self) -> Runner {
        match self {
            Self::LinuxX64Gnu | Self::LinuxX64Musl | Self::LinuxArm64Gnu | Self::LinuxArm64Musl => {
                Runner::Hosted(Rate::Linux)
            }
            Self::WindowsX64 => Runner::Hosted(Rate::Windows),
            Self::MacosArm64 | Self::MacosX64 => Runner::OwnerMachine,
        }
    }

    pub(crate) fn target(self) -> &'static str {
        match self {
            Self::LinuxX64Gnu => "x86_64-unknown-linux-gnu",
            Self::LinuxX64Musl => "x86_64-unknown-linux-musl",
            Self::LinuxArm64Gnu => "aarch64-unknown-linux-gnu",
            Self::LinuxArm64Musl => "aarch64-unknown-linux-musl",
            Self::WindowsX64 => "x86_64-pc-windows-msvc",
            Self::MacosArm64 => "aarch64-apple-darwin",
            Self::MacosX64 => "x86_64-apple-darwin",
        }
    }

    /// The set of `ci/runtimes.toml` the cell tests (ADR 0069): every
    /// runtime on Linux x64 (glibc), the ends of each line with the
    /// free-threaded builds on the other Linux cells, and the ends without
    /// them on Windows and macOS.
    pub(crate) fn runtimes(self) -> Set {
        match self {
            Self::LinuxX64Gnu => Set::ReleaseLinuxX64Gnu,
            Self::LinuxX64Musl | Self::LinuxArm64Gnu | Self::LinuxArm64Musl => Set::ReleaseLinux,
            Self::WindowsX64 | Self::MacosArm64 | Self::MacosX64 => Set::ReleaseOther,
        }
    }

    pub(crate) fn linux_job(self) -> Option<LinuxJob> {
        match self {
            Self::LinuxX64Gnu | Self::LinuxX64Musl => Some(LinuxJob::X64),
            Self::LinuxArm64Gnu | Self::LinuxArm64Musl => Some(LinuxJob::Arm64),
            Self::WindowsX64 | Self::MacosArm64 | Self::MacosX64 => None,
        }
    }

    /// Whether a job of a run belongs to this cell: its name is the cell's,
    /// or starts with it and a space (a cell split into several jobs).
    pub(crate) fn owns_job(self, name: &str) -> bool {
        name == self.label()
            || name
                .strip_prefix(self.label())
                .is_some_and(|rest| rest.starts_with(' '))
    }

    /// The name of the JUnit artifact the cell's job uploads (TESTING.md,
    /// Sharding, caching and reports), which the results store reads as the
    /// cell's configuration.
    pub(crate) fn junit_artifact(self) -> String {
        match self {
            Self::MacosX64 => "junit-release-dispatch-macos-x64-rosetta-1".to_owned(),
            other => format!("junit-release-dispatch-{}-1", other.key()),
        }
    }

    /// The cell's entry in its Linux job's matrix.
    pub(crate) fn matrix_entry(self) -> serde_json::Value {
        serde_json::json!({
            "cell": self.key(),
            "target": self.target(),
            "runtimes": self.runtimes().key(),
        })
    }
}

impl fmt::Display for Cell {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.label())
    }
}

/// The cells a dispatch asks for: `all`, or cell names separated by commas.
pub(crate) fn parse_selection(text: &str) -> Result<Vec<Cell>> {
    if text.trim() == "all" || text.trim().is_empty() {
        return Ok(Cell::ALL.to_vec());
    }
    let mut cells = text
        .split(',')
        .map(Cell::parse)
        .collect::<Result<Vec<_>>>()?;
    cells.sort_unstable();
    cells.dedup();
    Ok(cells)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hosted_cells_count_270_minutes_at_one_45_minute_job_each() {
        let minutes: u32 = Cell::ALL
            .into_iter()
            .filter_map(|cell| match cell.runner() {
                Runner::Hosted(rate) => Some(45 * rate.multiplier()),
                Runner::OwnerMachine => None,
            })
            .sum();

        assert_eq!(minutes, 270);
    }

    /// The workflow uploads each cell's JUnit files under the artifact name
    /// the results store reads the cell's configuration from.
    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "a synchronous unit test reads the workflow file"
    )]
    fn each_cells_junit_artifact_is_the_one_its_job_uploads() {
        let workflow = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../.github/workflows/release-suite.yml"),
        )
        .unwrap();

        for cell in Cell::ALL {
            let uploaded = match cell.linux_job() {
                Some(_) => "junit-release-dispatch-${{ matrix.cell }}-1".to_owned(),
                None => cell.junit_artifact(),
            };
            assert!(
                workflow.contains(&format!("name: {uploaded}")),
                "{cell} uploads no `{uploaded}`"
            );
        }
        assert_eq!(
            Cell::LinuxArm64Musl.junit_artifact(),
            "junit-release-dispatch-linux-arm64-musl-1"
        );
    }

    #[test]
    fn a_split_cells_jobs_belong_to_it_and_a_longer_names_do_not() {
        assert!(Cell::MacosX64.owns_job("macos-x64 (Rosetta 2) shard 1/2"));
        assert!(!Cell::LinuxX64Gnu.owns_job("linux-x64-gnux"));
    }
}
