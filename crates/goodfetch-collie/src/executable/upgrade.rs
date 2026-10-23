//! `upgrade-files` (ADRs 0030 and 0066): gives the executable's major its
//! copy of an older major's files in Goodfetch home, from a major the
//! release's `compat/upgrade-paths.toml` lists, never changing the older
//! major's files.

use std::process::ExitCode;

use goodfetch_cli::exit_codes;
use goodfetch_cli::output;
use goodfetch_engine::Identity;
use goodfetch_engine::home::Home;
use goodfetch_engine::upgrade::{UpgradeError, UpgradePaths, upgrade_files};
use goodfetch_protocol::versions::{UPGRADE_SOURCE_UNSUPPORTED, UpgradeFilesOutput};

/// Upgrades from `from`, or from the newest older major Goodfetch home
/// holds.
pub(super) fn upgrade(
    identity: &Identity,
    home: &Home,
    from: Option<&str>,
    json: bool,
) -> ExitCode {
    let upgraded = match upgrade_files(
        home,
        &identity.product_version,
        from,
        &UpgradePaths::committed(),
    ) {
        Ok(upgraded) => upgraded,
        Err(error @ (UpgradeError::Io { .. } | UpgradeError::NotAMajor { .. })) => {
            tracing::error!("{error}");
            return ExitCode::FAILURE;
        }
        Err(error) => {
            tracing::error!(code = UPGRADE_SOURCE_UNSUPPORTED.name, "{error}");
            return ExitCode::from(exit_codes::of_error(
                UPGRADE_SOURCE_UNSUPPORTED.name,
                UPGRADE_SOURCE_UNSUPPORTED.group,
            ));
        }
    };
    let shown = |paths: &[std::path::PathBuf]| -> Vec<String> {
        paths
            .iter()
            .map(|path| path.display().to_string())
            .collect()
    };
    if json {
        output::json(&UpgradeFilesOutput {
            from: upgraded.from.clone(),
            to: upgraded.to.clone(),
            copied: shown(&upgraded.copied),
            unchanged: shown(&upgraded.unchanged),
            kept: shown(&upgraded.kept),
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
