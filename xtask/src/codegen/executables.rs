//! What both Products' executables give `xtask codegen` (ADRs 0071, 0072,
//! 0087 and 0099): their command lines, which the `command:` Features and
//! the table of Foxhound's names read, and the namespaces of the commands
//! Collie's executable generates from Foxhound's catalog, which holds
//! Collie's, as the Rust constant in Collie's crate a start reads instead of
//! walking the catalog. `xtask` builds no engine crate (ADR 0072's amendment of
//! 2026-11-06): `xtask codegen` runs Collie's `command-lines` example, which
//! gives what each executable's hidden `command-line` command prints, and
//! commits it, and `--check` reads the committed files, so it starts no
//! process for them. Each executable's tests check the committed files
//! against what it prints. A workspace without Collie's executable crate,
//! such as a test's fixture, keeps the files it holds, none when it holds
//! none.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs and process bans target async crates (STYLE.md)"
)]

use std::ffi::OsStr;
use std::path::Path;

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::features::CommandLines;

/// Where Collie's crate's generated files live.
pub(super) const COLLIE_FOLDER: &str = "crates/goodfetch-collie/generated";

/// The file of the namespaces, which Collie's executable reads.
const NAMESPACES: &str = "crates/goodfetch-collie/generated/namespaces.rs";

/// The file of both executables' command lines: `{"collie": <tree>,
/// "foxhound": <tree>}`, each tree a command's `name` and the `commands`
/// under it, hidden ones included.
const COMMAND_LINES: &str = "crates/goodfetch-protocol/generated/command_lines.json";

/// Collie's executable crate, whose `executable` module both Products'
/// executables start from (ADR 0087).
const COLLIE: &str = "crates/goodfetch-collie";

/// The file the example writes into the folder it is given.
const WRITTEN: &str = "command-lines.json";

/// What one executable's `command-line` prints.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Printed {
    command_line: Value,
    namespaces: String,
}

/// What the example writes: each Product's [`Printed`].
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Both {
    collie: Printed,
    foxhound: Printed,
}

/// The two files, as the executables printed them or as the workspace holds
/// them; none of one the workspace does not hold.
pub(super) struct Files {
    command_lines: Option<Vec<u8>>,
    namespaces: Option<Vec<u8>>,
}

impl Files {
    /// The files as `root` holds them, which `--check` reads.
    pub(super) fn committed(root: &Path) -> Self {
        Self {
            command_lines: std::fs::read(root.join(COMMAND_LINES)).ok(),
            namespaces: std::fs::read(root.join(NAMESPACES)).ok(),
        }
    }

    /// The files as the executables at `root` print them, or those `root`
    /// holds when it has no Collie's executable crate.
    pub(super) fn printed(cargo: &OsStr, root: &Path) -> Result<Self> {
        if !root.join(COLLIE).is_dir() {
            return Ok(Self::committed(root));
        }
        let Both { collie, foxhound } = run_example(cargo, root)?;
        if collie.namespaces != foxhound.namespaces {
            bail!("Collie's and Foxhound's command lines give different namespaces");
        }
        Ok(Self {
            command_lines: Some(super::json_text(&json!({
                "collie": collie.command_line,
                "foxhound": foxhound.command_line,
            }))?),
            namespaces: Some(collie.namespaces.into_bytes()),
        })
    }

    /// Both executables' command lines, none when the workspace has none.
    pub(super) fn command_lines(&self) -> Result<Option<CommandLines>> {
        self.command_lines
            .as_deref()
            .map(|text| {
                serde_json::from_slice(text).with_context(|| format!("reading {COMMAND_LINES}"))
            })
            .transpose()
    }

    /// The namespaces file's text, none when the workspace has none.
    pub(super) fn namespaces(&self) -> Option<&[u8]> {
        self.namespaces.as_deref()
    }

    /// Each file by its path relative to the root.
    pub(super) fn into_files(self) -> impl Iterator<Item = (String, Vec<u8>)> {
        [
            (COMMAND_LINES, self.command_lines),
            (NAMESPACES, self.namespaces),
        ]
        .into_iter()
        .filter_map(|(path, text)| Some((path.to_owned(), text?)))
    }
}

fn run_example(cargo: &OsStr, root: &Path) -> Result<Both> {
    let folder = tempfile::tempdir().context("creating a folder for the command lines")?;
    let output = crate::child_cargo::command(cargo)
        .args([
            "run",
            "--quiet",
            "--package",
            "goodfetch-collie",
            "--features",
            "executable",
            "--example",
            "command-lines",
            "--",
        ])
        .arg(folder.path())
        .current_dir(root)
        .output()
        .context("running goodfetch-collie's command-lines example")?;
    if !output.status.success() {
        bail!(
            "goodfetch-collie's command-lines example failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let path = folder.path().join(WRITTEN);
    let text = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_slice(&text).context("reading goodfetch-collie's command lines")
}
