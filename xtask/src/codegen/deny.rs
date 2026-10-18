//! The lists `xtask codegen` writes into marked regions of cargo-deny's
//! configurations:
//! - in `deny.toml`, ADR 0087's workspace-wide direction rule: cargo-deny
//!   bans each of Foxhound's crates (the folders under `crates/foxhound/`
//!   that hold a manifest) unless its direct parent is one of Foxhound's
//!   crates or `xtask`;
//! - in `ci/deny/collie.toml`, the ban of each of Foxhound's crates from what
//!   Collie's executable ships (ADR 0087);
//! - in `deny.toml`, the licenses of ADR 0018's allowlist that a crate may
//!   carry, from `license-allowlist.toml`, the one list every license check
//!   reads: all but those allowed only for compiled Filter lists, which no
//!   crate's code may carry (ADR 0047).
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::fmt::Write as _;
use std::path::Path;

use anyhow::{Context as _, Result};
use serde::Deserialize;

/// Where each region begins; each ends at the first `END` after it.
const WRAPPERS_BEGIN: &str = "# xtask codegen: begin, the crates under crates/foxhound/ (ADR 0087). Never edit by hand; run `xtask codegen`.";
const COLLIE_BEGIN: &str = "# xtask codegen: begin, Foxhound's crates, which Collie's executable never ships (ADR 0087). Never edit by hand; run `xtask codegen`.";
const LICENSES_BEGIN: &str = "# xtask codegen: begin, ADR 0018's license allowlist, from license-allowlist.toml. Never edit by hand; run `xtask codegen`.";
const END: &str = "# xtask codegen: end";

/// The only crate outside `crates/foxhound/` that may depend on one inside.
const BUILD_TOOL: &str = "xtask";

/// ADR 0018's license allowlist.
pub(super) const ALLOWLIST: &str = "license-allowlist.toml";

/// Collie's cargo-deny configuration, rooted at its executable crate.
pub(super) const COLLIE: &str = "ci/deny/collie.toml";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Allowlist {
    license: Vec<License>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct License {
    /// An SPDX license expression.
    id: String,
    /// The one use the license is allowed for, when it is not allowed for all.
    only: Option<Use>,
}

#[derive(Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
enum Use {
    /// Files used as their authors published them (MPL-2.0, ADR 0018); a
    /// crate from a registry is.
    UnmodifiedFiles,
    /// The compiled form of Filter lists embedded in the executable (CC
    /// BY-SA, ADRs 0018 and 0047); never code.
    CompiledFilterLists,
}

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

/// The licenses a crate may carry, in the allowlist's order.
pub(super) fn crate_licenses(root: &Path) -> Result<Vec<String>> {
    let path = root.join(ALLOWLIST);
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let allowlist: Allowlist =
        toml::from_str(&text).with_context(|| format!("reading {ALLOWLIST}"))?;
    Ok(allowlist
        .license
        .into_iter()
        .filter(|license| license.only != Some(Use::CompiledFilterLists))
        .map(|license| license.id)
        .collect())
}

/// `deny.toml` with its regions holding Foxhound's `crates` and the
/// `licenses` a crate may carry.
pub(super) fn render_workspace(current: &str, crates: &[String], licenses: &[String]) -> String {
    let current = replace(current, LICENSES_BEGIN, &licenses_region(licenses));
    replace(&current, WRAPPERS_BEGIN, &wrappers_region(crates))
}

/// Collie's configuration with its region banning Foxhound's `crates`.
pub(super) fn render_collie(current: &str, crates: &[String]) -> String {
    let mut text = format!("{COLLIE_BEGIN}\n");
    for name in crates {
        let _ = write!(
            text,
            "[[bans.deny]]\ncrate = {name:?}\nreason = \"one of Foxhound's crates, under crates/foxhound/ (ADR 0087)\"\n\n",
        );
    }
    let _ = writeln!(text, "{END}");
    replace(current, COLLIE_BEGIN, &text)
}

/// `current` with the region that starts with `begin` replaced by `region`,
/// added at the end when the file has none.
fn replace(current: &str, begin: &str, region: &str) -> String {
    if let Some(start) = current.find(begin)
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

fn wrappers_region(crates: &[String]) -> String {
    let wrappers = crates
        .iter()
        .map(String::as_str)
        .chain(std::iter::once(BUILD_TOOL))
        .map(|name| format!("{name:?}"))
        .collect::<Vec<_>>()
        .join(", ");
    let mut text = format!("{WRAPPERS_BEGIN}\n");
    for name in crates {
        let _ = write!(
            text,
            "[[bans.deny]]\ncrate = {name:?}\nwrappers = [{wrappers}]\nreason = \"only Foxhound's crates and xtask depend on a crate under crates/foxhound/ (ADR 0087)\"\n\n",
        );
    }
    let _ = writeln!(text, "{END}");
    text
}

fn licenses_region(licenses: &[String]) -> String {
    let mut text = format!("{LICENSES_BEGIN}\n[licenses]\nallow = [\n");
    for license in licenses {
        let _ = writeln!(text, "  {license:?},");
    }
    let _ = writeln!(text, "]\n\n{END}");
    text
}
