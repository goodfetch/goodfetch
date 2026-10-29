//! `upgrade-files` (ADRs 0030 and 0066): gives the executable's major its
//! copy of an older major's files in Goodfetch home, from a major the
//! release's `compat/upgrade-paths.toml` lists, never changing the older
//! major's files.

use std::process::ExitCode;

use goodfetch_cli::output::{self, Human};
use goodfetch_engine::CallError;
use goodfetch_engine::Identity;
use goodfetch_engine::home::Home;
use goodfetch_engine::upgrade::{UpgradeError, UpgradePaths, upgrade_files};
use goodfetch_protocol::checkpoint::{FILE_WRITE_FAILED, FileWriteFailedDetails, WriteSide};
use goodfetch_protocol::cli;
use goodfetch_protocol::versions::{FilesUpgraded, UPGRADE_SOURCE_UNSUPPORTED};

/// Upgrades from `from`, or from the newest older major Goodfetch home
/// holds. A `--from` that names no major is a usage error, `2`, as clap's
/// own refusal of a flag's value is; a major it does not upgrade from is
/// `upgrade_source_unsupported`, and a file it cannot copy
/// `file_write_failed`, each printed as every command's failure is.
pub(super) fn upgrade(
    identity: &Identity,
    home: &Home,
    from: Option<&str>,
    json: bool,
    human: &Human,
) -> ExitCode {
    let upgraded = match upgrade_files(
        home,
        &identity.product_version,
        from,
        &UpgradePaths::committed(),
    ) {
        Ok(upgraded) => upgraded,
        Err(error @ UpgradeError::NotAMajor { .. }) => {
            human.failure(&format!("invalid value for '--from <FROM>': {error}"), &[]);
            return ExitCode::from(cli::USAGE.code);
        }
        Err(UpgradeError::Io { path, message }) => {
            let failure = CallError::new(
                &FILE_WRITE_FAILED,
                format!(
                    "`{}` could not be copied: {message}. What was copied before it stays, and the older major's files are unchanged.",
                    path.display()
                ),
                &FileWriteFailedDetails {
                    reason: "other".to_owned(),
                    os_code: None,
                    path: Some(path.display().to_string()),
                    side: WriteSide::Engine,
                },
            );
            return output::fail(failure.to_error_object(), None, json, human);
        }
        Err(error) => {
            let failure = CallError::new(
                &UPGRADE_SOURCE_UNSUPPORTED,
                error.to_string(),
                &serde_json::Map::new(),
            );
            return output::fail(failure.to_error_object(), None, json, human);
        }
    };
    let shown = |paths: &[std::path::PathBuf]| -> Vec<String> {
        paths
            .iter()
            .map(|path| path.display().to_string())
            .collect()
    };
    if json {
        output::json(&FilesUpgraded {
            upgraded: shown(&upgraded.copied),
            unchanged: shown(&upgraded.unchanged),
            refused: shown(&upgraded.kept),
        });
        return ExitCode::SUCCESS;
    }
    match &upgraded.from {
        None => output::line(&format!(
            "Goodfetch home holds no older major's files: major {} has nothing to upgrade.",
            upgraded.to
        )),
        Some(from) => output::line(&format!(
            "Gave major {} its copy of major {from}'s files: {} copied, {} already there, {} kept as major {} had them. Major {from}'s files are unchanged.",
            upgraded.to,
            upgraded.copied.len(),
            upgraded.unchanged.len(),
            upgraded.kept.len(),
            upgraded.to
        )),
    }
    ExitCode::SUCCESS
}
