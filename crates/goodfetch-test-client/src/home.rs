//! The test home's layout: where, in a test's Goodfetch home, the commit's
//! executables keep their files. The names and the per-major rule are
//! `goodfetch-protocol`'s `home` (ADRs 0030, 0031 and 0059); the version is
//! the workspace's, which every member and so every executable a test runs
//! shares. A test passes its home in and gets paths out, so none spells a
//! per-major folder such as `0.0` or a file's name itself.

use std::path::{Path, PathBuf};

use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::home::{
    self, DaemonFile, OPERATOR_POLICY_FILE, RUN_FILE, RUN_INDEX, RUNS, USER_CONFIGURATION_FILE,
};

/// The Product version of the commit's executables: the workspace's.
pub fn product_version() -> ProductVersion {
    ProductVersion::parse(env!("CARGO_PKG_VERSION"))
        .expect("invariant: the workspace's version is a Product version")
}

/// The paths of one test home's per-major folder for the commit's
/// executables.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HomeLayout {
    per_major: PathBuf,
}

impl HomeLayout {
    /// The layout of the Goodfetch home at `home`, as the commit's
    /// executables lay it out.
    pub fn of(home: &Path) -> Self {
        Self {
            per_major: home::per_major(home, &product_version()),
        }
    }

    /// The per-major folder.
    pub fn per_major(&self) -> &Path {
        &self.per_major
    }

    /// The folder that holds every run folder.
    pub fn runs(&self) -> PathBuf {
        self.per_major.join(RUNS)
    }

    /// The run folder of the Run `run_id`.
    pub fn run_folder(&self, run_id: &str) -> PathBuf {
        self.runs().join(run_id)
    }

    /// The run file of the Run `run_id`.
    pub fn run_file(&self, run_id: &str) -> PathBuf {
        self.run_folder(run_id).join(RUN_FILE)
    }

    /// The run trace's file `file` of the Run `run_id`, `file` being one of
    /// `goodfetch_protocol::home`'s `RUN_TRACE_*` names.
    pub fn run_trace(&self, run_id: &str, file: &str) -> PathBuf {
        self.run_folder(run_id).join(file)
    }

    /// The Run index.
    pub fn run_index(&self) -> PathBuf {
        self.per_major.join(RUN_INDEX)
    }

    /// The Daemon's file `file`.
    pub fn daemon_file(&self, file: DaemonFile) -> PathBuf {
        self.per_major.join(file.name())
    }

    /// The Operator policy's file.
    pub fn operator_policy(&self) -> PathBuf {
        self.per_major.join(OPERATOR_POLICY_FILE)
    }

    /// User configuration's file.
    pub fn user_configuration(&self) -> PathBuf {
        self.per_major.join(USER_CONFIGURATION_FILE)
    }
}

#[cfg(test)]
mod tests;
