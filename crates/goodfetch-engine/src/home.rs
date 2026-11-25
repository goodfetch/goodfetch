//! The Goodfetch home (ADRs 0031, 0059 and 0098): the folder of one user's
//! Goodfetch data, shared by both Products, and in it the per-major folder
//! that holds the run folders and the Run index. `main` resolves it, from
//! `GOODFETCH_HOME` when it is set and the OS's usual data folder otherwise,
//! and passes it to the engine as a value. During `0.y` previews each `0.y`
//! minor counts as a major (ADR 0030). The names of its files and folders,
//! and the per-major folder's rule, are `goodfetch_protocol::home`'s, which
//! every other reader of the home shares.

use std::path::{Path, PathBuf};

use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::home::{
    ACCOUNTS_STORE, AUDIT_LOG, AUDIT_LOG_HEAD, ENGINE_KEY_LOCK, KNOWN_CERTIFICATES, LOGS,
    POLITENESS_FILE, RUN_INDEX, RUNS, SECRET_FILES, USAGE_LEDGER, per_major,
};

/// One user's Goodfetch data folder, and the copy of the Browser cache
/// `GOODFETCH_BROWSERS_PATH` names in place of its own, if any.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Home {
    root: PathBuf,
    browsers_path: Option<PathBuf>,
}

impl Home {
    /// The home at `root`: the folder `GOODFETCH_HOME` names, or the
    /// `goodfetch` folder of the OS's usual data folder.
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            browsers_path: None,
        }
    }

    /// The same home, whose launches take their browsers from the Browser
    /// cache at `browsers_path`, from `GOODFETCH_BROWSERS_PATH`, when it is
    /// set (ADR 0059).
    #[must_use]
    pub fn with_browsers_path(mut self, browsers_path: Option<PathBuf>) -> Self {
        self.browsers_path = browsers_path;
        self
    }

    /// Its folder.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The Browser cache `GOODFETCH_BROWSERS_PATH` names, when it is set.
    pub fn browsers_path(&self) -> Option<&Path> {
        self.browsers_path.as_deref()
    }

    /// The per-major folder of the Product version `version`: `1` for any
    /// `1.x.y`, `0.4` for any `0.4.y`.
    pub fn per_major(&self, version: &ProductVersion) -> PathBuf {
        per_major(&self.root, version)
    }

    /// The per-major data folder's run folders, the default
    /// `run_state.location`.
    pub fn runs(&self, version: &ProductVersion) -> PathBuf {
        self.per_major(version).join(RUNS)
    }

    /// The per-major Run index's file.
    pub fn run_index(&self, version: &ProductVersion) -> PathBuf {
        self.per_major(version).join(RUN_INDEX)
    }

    /// The per-major Accounts store's file, beside the Operator policy,
    /// which only the Daemon of its major opens (ADR 0149).
    pub fn accounts_store(&self, version: &ProductVersion) -> PathBuf {
        self.per_major(version).join(ACCOUNTS_STORE)
    }

    /// The per-user Usage ledger's file, outside the per-major folders, which
    /// every engine of every major and of both Products opens (ADR 0041).
    pub fn usage_ledger(&self) -> PathBuf {
        self.root.join(USAGE_LEDGER)
    }

    /// The per-user Politeness file, outside the per-major folders, which
    /// every major and both Products share (ADRs 0053 and 0098).
    pub fn politeness_file(&self) -> PathBuf {
        self.root.join(POLITENESS_FILE)
    }

    /// The per-user Audit log's store, outside the per-major folders, to
    /// which every engine of every major and of both Products appends
    /// (ADR 0016).
    pub fn audit_log(&self) -> PathBuf {
        self.root.join(AUDIT_LOG)
    }

    /// The Audit log's head file, beside its store, which names the last
    /// entry so that a truncated tail shows.
    pub fn audit_log_head(&self) -> PathBuf {
        self.root.join(AUDIT_LOG_HEAD)
    }

    /// The per-user known-certificates store, beside the Usage ledger and
    /// outside the per-major folders, which every engine of every major and
    /// of both Products reads (ADR 0042).
    pub fn known_certificates(&self) -> PathBuf {
        self.root.join(KNOWN_CERTIFICATES)
    }

    /// The log folder, outside the per-major folders, which holds one set
    /// of files per Daemon and one per other process that writes its own
    /// (ADR 0122).
    pub fn logs(&self) -> PathBuf {
        self.root.join(LOGS)
    }

    /// The folder of the encrypted local files `secretfile:` reads, and of
    /// the identity they are encrypted to, outside the per-major folders,
    /// which every major and both Products share (spec #564).
    pub fn secret_files(&self) -> PathBuf {
        self.root.join(SECRET_FILES)
    }

    /// The file whose lock an engine holds while it makes its own
    /// key-encryption key in the OS keychain, so two engines of the user,
    /// of any major and either Product, make one between them.
    pub fn engine_key_lock(&self) -> PathBuf {
        self.root.join(ENGINE_KEY_LOCK)
    }
}

/// Where the run folders are: the per-major data folder's, unless
/// `run_state.location` names another folder.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum RunStateLocation {
    /// The per-major data folder's `runs`.
    #[default]
    PerMajor,
    /// The folder `run_state.location` names.
    Folder(PathBuf),
}

impl RunStateLocation {
    /// The folder that holds the run folders of `home`'s major of
    /// `version`.
    pub fn folder(&self, home: &Home, version: &ProductVersion) -> PathBuf {
        match self {
            Self::PerMajor => home.runs(version),
            Self::Folder(folder) => folder.clone(),
        }
    }
}

#[cfg(test)]
mod tests;
