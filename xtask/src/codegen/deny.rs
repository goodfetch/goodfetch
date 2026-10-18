//! The `wrappers` list of ADR 0087's workspace-wide direction rule, in a
//! marked region of `deny.toml`: cargo-deny bans each of Foxhound's crates,
//! the folders under `crates/foxhound/` that hold a manifest, unless its
//! direct parent is one of Foxhound's crates or `xtask`.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::fmt::Write as _;
use std::path::Path;

use anyhow::{Context as _, Result};

const BEGIN: &str = "# xtask codegen: begin, the crates under crates/foxhound/ (ADR 0087). Never edit by hand; run `xtask codegen`.";
const END: &str = "# xtask codegen: end";

/// The only crate outside `crates/foxhound/` that may depend on one inside.
const BUILD_TOOL: &str = "xtask";

/// Foxhound's crates, by folder name, sorted.
pub(super) fn foxhound_crates(root: &Path) -> Result<Vec<String>> {
    let mut crates = Vec::new();
    let mut pending = vec![root.join("crates/foxhound")];
    while let Some(folder) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&folder) else {
            continue;
        };
        for entry in entries {
            let entry = entry.with_context(|| format!("reading {}", folder.display()))?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let path = entry.path();
            if path.join("Cargo.toml").is_file() {
                crates.push(entry.file_name().to_string_lossy().into_owned());
            } else {
                pending.push(path);
            }
        }
    }
    crates.sort();
    Ok(crates)
}

/// `deny.toml` with its region holding `crates`, added at the end when the
/// file has none.
pub(super) fn render(current: &str, crates: &[String]) -> String {
    let region = region(crates);
    if let Some(start) = current.find(BEGIN)
        && let Some(end) = current[start..].find(END)
    {
        let end = start + end + END.len();
        let end = current[end..].strip_prefix('\n').map_or(end, |_| end + 1);
        return format!("{}{region}{}", &current[..start], &current[end..]);
    }
    let separator = if current.ends_with('\n') || current.is_empty() {
        "\n"
    } else {
        "\n\n"
    };
    format!("{current}{separator}{region}")
}

fn region(crates: &[String]) -> String {
    let wrappers = crates
        .iter()
        .map(String::as_str)
        .chain(std::iter::once(BUILD_TOOL))
        .map(|name| format!("{name:?}"))
        .collect::<Vec<_>>()
        .join(", ");
    let mut text = format!("{BEGIN}\n");
    for name in crates {
        let _ = write!(
            text,
            "[[bans.deny]]\ncrate = {name:?}\nwrappers = [{wrappers}]\nreason = \"only Foxhound's crates and xtask depend on a crate under crates/foxhound/ (ADR 0087)\"\n\n",
        );
    }
    let _ = writeln!(text, "{END}");
    text
}
