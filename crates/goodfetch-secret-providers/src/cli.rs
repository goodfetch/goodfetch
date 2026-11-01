//! Running a vendor's CLI as an operator-configured command: the command
//! its `secret_providers.<scheme>.command` names, checked before each use,
//! started through the engine's `children` module (ADR 0024) with the
//! engine's environment map and nothing else, its standard output held
//! only in buffers zeroized when dropped.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::Path;
use std::process::Stdio;

use goodfetch_engine::children::Children;
use goodfetch_engine::secrets::{ConfigProblem, PartyText, ProviderFailure};
use goodfetch_engine::settings::Resolved;
use goodfetch_protocol::catalog::Setting;
use secrecy::SecretString;
use secrecy::zeroize::Zeroizing;

/// A vendor's CLI as its setting configures it.
#[derive(Clone)]
pub(crate) struct Cli {
    /// The key of the setting that names the command,
    /// `secret_providers.<scheme>.command`.
    setting: &'static str,
    /// What the vendor calls its CLI, such as `op`.
    vendor_command: &'static str,
    /// The setting's value, if any.
    command: Option<String>,
    /// The engine's environment map, the CLI's only environment.
    environment: BTreeMap<OsString, OsString>,
}

impl std::fmt::Debug for Cli {
    /// Without the environment, which holds the CLI's sign-in.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Cli")
            .field("setting", &self.setting)
            .field("command", &self.command)
            .finish_non_exhaustive()
    }
}

/// What a CLI that ran answered.
pub(crate) enum Answer {
    /// It exited with success and wrote this on its standard output.
    Value(Zeroizing<Vec<u8>>),
    /// It failed and wrote this on its standard error, which may echo what
    /// it was given; the translation point reads it and keeps it private.
    Failed {
        /// Its standard error, lossily decoded.
        stderr: String,
        /// Its exit code, when it exited with one.
        code: Option<i32>,
    },
}

impl Cli {
    /// The CLI `setting` names in `resolved`, run with `environment`.
    pub(crate) fn configured(
        setting: &'static Setting,
        vendor_command: &'static str,
        resolved: &Resolved,
        environment: BTreeMap<OsString, OsString>,
    ) -> Self {
        let command = resolved.value(setting.name).as_str().map(str::to_owned);
        Self::new(setting, vendor_command, command, environment)
    }

    /// The CLI at `command`, as `setting` would name it.
    pub(crate) fn new(
        setting: &'static Setting,
        vendor_command: &'static str,
        command: Option<String>,
        environment: BTreeMap<OsString, OsString>,
    ) -> Self {
        Self {
            setting: setting.name,
            vendor_command,
            command,
            environment,
        }
    }

    /// The command, when the setting names one as an absolute path.
    pub(crate) fn check(&self) -> Result<&Path, ConfigProblem> {
        let name = self.setting;
        let vendor = self.vendor_command;
        match self.command.as_deref() {
            None => Err(ConfigProblem {
                fix: format!(
                    "Set `{name}` in configuration or the Operator policy to the absolute path of the `{vendor}` CLI."
                ),
            }),
            Some(command) if command.trim().is_empty() || !Path::new(command).is_absolute() => {
                Err(ConfigProblem {
                    fix: format!(
                        "`{name}` is `{command}`, not an absolute path; set it to the absolute path of the `{vendor}` CLI."
                    ),
                })
            }
            Some(command) => Ok(Path::new(command)),
        }
    }

    /// Runs the CLI with `arguments` and `extra` beside the engine's
    /// environment, its standard input closed.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future kills the CLI and its process group.
    pub(crate) async fn run(
        &self,
        arguments: &[&str],
        extra: &[(&str, &str)],
    ) -> Result<Answer, ProviderFailure> {
        let path = self.check().map_err(ProviderFailure::NotConfigured)?;
        let mut command = tokio::process::Command::new(path);
        command
            .args(arguments)
            .env_clear()
            .envs(&self.environment)
            .envs(extra.iter().copied())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut children = Children::new().map_err(|error| self.unstarted(&error))?;
        let child = children
            .spawn(command)
            .map_err(|error| self.unstarted(&error))?;
        let output = child
            .wait_with_output()
            .await
            .map_err(|error| self.unstarted(&error))?;
        let stdout = Zeroizing::new(output.stdout);
        if output.status.success() {
            Ok(Answer::Value(stdout))
        } else {
            Ok(Answer::Failed {
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
                code: output.status.code(),
            })
        }
    }

    /// What a CLI that could not start, or whose output could not be read,
    /// means.
    fn unstarted(&self, error: &std::io::Error) -> ProviderFailure {
        let name = self.setting;
        let command = self.command.as_deref().unwrap_or_default();
        match error.kind() {
            std::io::ErrorKind::NotFound => ProviderFailure::NotConfigured(ConfigProblem {
                fix: format!(
                    "`{name}` names `{command}`, which does not exist; install the `{}` CLI there or set `{name}` to where it is.",
                    self.vendor_command
                ),
            }),
            std::io::ErrorKind::PermissionDenied => ProviderFailure::NotConfigured(ConfigProblem {
                fix: format!(
                    "`{name}` names `{command}`, which the engine may not run; make it executable."
                ),
            }),
            kind => {
                tracing::warn!(
                    provider = self.vendor_command,
                    kind = %kind,
                    "the Secret provider's CLI could not run"
                );
                ProviderFailure::Unrecognized(PartyText::new(error.to_string()))
            }
        }
    }
}

/// `stdout` as a value, when it is UTF-8 text; its bytes are copied once
/// into the value and zeroized with `stdout`.
pub(crate) fn value(
    stdout: &Zeroizing<Vec<u8>>,
    vendor_command: &str,
) -> Result<SecretString, ProviderFailure> {
    match std::str::from_utf8(stdout) {
        Ok(text) => Ok(SecretString::from(text.to_owned())),
        Err(_) => Err(ProviderFailure::Refused {
            fix: format!(
                "The `{vendor_command}` CLI answered with bytes that are not UTF-8 text; store the value as text."
            ),
        }),
    }
}

/// Logs a failure a translation point does not know, without its text,
/// and keeps the text private.
pub(crate) fn unrecognized(
    vendor_command: &'static str,
    stderr: String,
    code: Option<i32>,
) -> ProviderFailure {
    tracing::warn!(
        provider = vendor_command,
        code,
        "an unrecognized failure of the Secret provider's CLI"
    );
    ProviderFailure::Unrecognized(PartyText::new(stderr))
}
