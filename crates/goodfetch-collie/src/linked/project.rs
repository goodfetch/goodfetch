//! The project file a connection reads (ADR 0083): the `goodfetch.toml`
//! found upward from the working directory, the one the program names, or
//! none, read once at `connect()` and refused there before any engine is
//! reached.

use std::io;
use std::path::PathBuf;

use goodfetch_collie_client::Error;
use goodfetch_engine::settings::{SettingsError, find_project, nested, project_settings};
use goodfetch_protocol::checkpoint::{FORMAT_VERSION_UNSUPPORTED, FormatVersionUnsupportedDetails};
use goodfetch_protocol::formats::Compatibility;
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
    /// found, or none is to be read. A named file is read, carried and
    /// reported by its absolute path, so that the engine, or a later read
    /// of the Run it starts, finds the same file. Blocking, as the
    /// machine's settings' read the opening makes is.
    ///
    /// # Errors
    ///
    /// `setting_invalid` at `project_configuration` when the working
    /// directory cannot be read, for a named file that is not there, and
    /// for a file that does not read as a project file, and
    /// `format_version_unsupported` for one of a format version this
    /// engine cannot read.
    pub(super) fn read(self) -> Result<Option<Project>, Error> {
        let file = match self {
            Self::Found => {
                let folder =
                    std::env::current_dir().map_err(|error| no_working_directory(&error))?;
                let Some(found) = find_project(&folder) else {
                    return Ok(None);
                };
                found
            }
            Self::Named(file) => {
                std::path::absolute(&file).map_err(|error| no_working_directory(&error))?
            }
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
    /// Its absolute path.
    pub(super) file: PathBuf,
    sent: ProjectSettings,
}

impl Project {
    /// Its path and settings, as the connection carries them.
    pub(super) fn carried(&self) -> ProjectSettings {
        self.sent.clone()
    }

    /// The `timeout.connect` it sets, if it does, as a table or as the
    /// key written whole, as the engine reads it.
    pub(super) fn connect_timeout(&self) -> Option<&serde_json::Value> {
        nested(&self.sent.settings, TIMEOUT_CONNECT.name)
    }
}

/// The fix of a project file's refusal: what its program can do instead.
const PROJECT_FIX: &str =
    "name another project file with `project`, or read none with `no_project()`";

/// The connect's `setting_invalid` when the working directory, which the
/// project file is found or named from, cannot be read for `error`.
fn no_working_directory(error: &io::Error) -> Error {
    let reason = format!("the working directory cannot be read ({error})");
    let fix = "Name the project file by its absolute path with `project`, or read none with `no_project()`.".to_owned();
    Error::setting_invalid(
        format!("No project file can be found: {reason}."),
        &SettingInvalidDetails {
            key: "project".to_owned(),
            layer: Some(SettingsLayer::ProjectConfiguration),
            reason,
            limit: None,
            fix: Some(fix),
        },
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
            fix: Some(format!("Create it, or {PROJECT_FIX}.")),
        },
    )
}

/// The connect's error for the project file at `path`, which
/// `project_settings` refused, with the fix the engine's own refusal of it
/// gives.
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
                fix: Some(format!("Fix the project file at {path}, or {PROJECT_FIX}.")),
            },
        ),
        SettingsError::Format {
            format,
            found,
            supported,
            compatibility,
            ..
        } => {
            let fix = match compatibility {
                Compatibility::OlderMajor => "Run `upgrade-files`, then try again.".to_owned(),
                Compatibility::Compatible
                | Compatibility::NewerMinor
                | Compatibility::NewerMajor => {
                    format!("Upgrade Goodfetch to a version that reads {format} {found}.")
                }
            };
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
