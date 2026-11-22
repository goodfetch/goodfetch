//! The project file a connection reads (ADR 0083): the `goodfetch.toml`
//! found upward from the working directory, the one the program names, or
//! none, read once at `connect()` and refused there before any engine is
//! reached.

use std::path::PathBuf;

use goodfetch_collie_client::Error;
use goodfetch_engine::settings::{SettingsError, find_project, project_settings};
use goodfetch_protocol::checkpoint::{FORMAT_VERSION_UNSUPPORTED, FormatVersionUnsupportedDetails};
use goodfetch_protocol::settings::{
    ProjectSettings, SettingInvalidDetails, SettingsLayer, TIMEOUT_CONNECT,
};

/// Which project file a connection reads (ADR 0083).
#[derive(Clone, Debug, Default)]
pub(super) enum ProjectChoice {
    /// `goodfetch.toml` in the working directory or the nearest folder
    /// above it, if there is one.
    #[default]
    Found,
    /// This file, which must be there.
    Named(PathBuf),
    /// None.
    Skipped,
}

impl ProjectChoice {
    /// The project file chosen, read, `None` when none is: none is
    /// found, or none is to be read. Blocking, as the machine's settings'
    /// read the opening makes is.
    ///
    /// # Errors
    ///
    /// `setting_invalid` at `project_configuration` for a named file that
    /// is not there, and for a file that does not read as a project file,
    /// `format_version_unsupported` for one of a format version this
    /// engine cannot read.
    pub(super) fn read(self) -> Result<Option<Project>, Error> {
        let file = match self {
            Self::Found => {
                let Ok(folder) = std::env::current_dir() else {
                    return Ok(None);
                };
                let Some(found) = find_project(&folder) else {
                    return Ok(None);
                };
                found
            }
            Self::Named(file) => file,
            Self::Skipped => return Ok(None),
        };
        let path = file.display().to_string();
        match project_settings(&file) {
            Ok(Some(settings)) => Ok(Some(Project {
                file,
                sent: ProjectSettings { path, settings },
            })),
            Ok(None) => Err(project_missing(&path)),
            Err(error) => Err(project_refused(error, &path)),
        }
    }
}

/// The project file a connection read (ADR 0083): where it is, and its
/// settings as `run.start` and `config.explain` carry them.
#[derive(Debug)]
pub(crate) struct Project {
    pub(super) file: PathBuf,
    pub(super) sent: ProjectSettings,
}

impl Project {
    /// The `timeout.connect` it sets, if it does.
    pub(super) fn connect_timeout(&self) -> Option<&serde_json::Value> {
        let (table, key) = TIMEOUT_CONNECT
            .name
            .split_once('.')
            .expect("invariant: `timeout.connect` is a key of the `timeout` table");
        self.sent.settings.get(table)?.get(key)
    }
}

/// What a project file's refusal tells its program to do, for the file
/// at `path`.
fn project_fix(path: &str) -> String {
    format!(
        "Fix the project file at {path}, name another with `project`, or read none with `no_project()`."
    )
}

/// The connect's `setting_invalid` for the project file named at `path`,
/// which is not there.
fn project_missing(path: &str) -> Error {
    let reason = "the file is not there";
    Error::setting_invalid(
        format!("The project file {path} cannot be read: {reason}."),
        &SettingInvalidDetails {
            key: path.to_owned(),
            layer: Some(SettingsLayer::ProjectConfiguration),
            reason: reason.to_owned(),
            limit: None,
            fix: Some(
                "Name a project file that is there with `project`, or read none with `no_project()`."
                    .to_owned(),
            ),
        },
    )
}

/// The connect's error for the project file at `path`, which
/// `project_settings` refused, as the TypeScript and Python SDKs raise it.
fn project_refused(error: SettingsError, path: &str) -> Error {
    let message = error.to_string();
    match error {
        SettingsError::Invalid { reason, layer, .. } => Error::setting_invalid(
            format!("The project file {path} cannot be read: {reason}."),
            &SettingInvalidDetails {
                key: path.to_owned(),
                layer,
                reason,
                limit: None,
                fix: Some(project_fix(path)),
            },
        ),
        SettingsError::Format {
            format,
            found,
            supported,
            ..
        } => {
            let fix = format!("Upgrade Goodfetch to a version that reads {format} {found}.");
            Error::of_sdk(
                &FORMAT_VERSION_UNSUPPORTED,
                &format!("The project file {path} cannot be read: {message}"),
                &fix,
                &FormatVersionUnsupportedDetails {
                    format: format.to_owned(),
                    found,
                    supported,
                    fix: fix.clone(),
                },
            )
        }
        other => unreachable!(
            "invariant: project_settings refuses a file only as invalid or of another format, not {other:?}"
        ),
    }
}
