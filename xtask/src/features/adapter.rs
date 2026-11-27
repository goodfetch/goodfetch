//! The `adapter:` Features: one per Framework adapter package under
//! `adapters/collie/<name>/` or `adapters/foxhound/<name>/` holding a
//! `package.json` or `pyproject.toml` (ADRs 0070, 0072 and 0095). Collie's
//! adapters ship with both Products; an adapter present only under
//! `adapters/foxhound/` ships with Foxhound alone, and a Foxhound package
//! sharing a Collie adapter's name deduplicates into one `Both` Feature.
//! A folder without a package manifest, or a workspace without `adapters/`,
//! generates nothing.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::collections::BTreeMap;
use std::io::ErrorKind;
use std::path::Path;

use anyhow::{Context as _, Result, bail};

use super::entry::{Carriers, Entry};
use super::files::{Kind, Status};

const ADAPTERS: &str = "adapters";
const MANIFESTS: [&str; 2] = ["package.json", "pyproject.toml"];

pub(super) fn entries(root: &Path) -> Result<Vec<Entry>> {
    let base = root.join(ADAPTERS);
    let mut by_name = BTreeMap::new();
    for (folder, carriers) in [("collie", Carriers::Both), ("foxhound", Carriers::Foxhound)] {
        for name in packages_in(&base.join(folder))? {
            by_name.entry(name).or_insert(carriers);
        }
    }
    Ok(by_name
        .into_iter()
        .map(|(name, carriers)| Entry {
            kind: Kind::Adapter,
            name,
            area: None,
            milestone: None,
            status: Status::Active,
            carriers,
        })
        .collect())
}

/// The adapter folder names directly in `folder` that hold a package manifest.
fn packages_in(folder: &Path) -> Result<Vec<String>> {
    let listing = match std::fs::read_dir(folder) {
        Ok(listing) => listing,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error).with_context(|| format!("listing {}", folder.display())),
    };
    let mut names = Vec::new();
    for item in listing {
        let item = item.with_context(|| format!("listing {}", folder.display()))?;
        if !item
            .file_type()
            .with_context(|| format!("reading {}", item.path().display()))?
            .is_dir()
        {
            continue;
        }
        let mut has_manifest = false;
        for manifest in MANIFESTS {
            let path = item.path().join(manifest);
            if path
                .try_exists()
                .with_context(|| format!("reading {}", path.display()))?
            {
                has_manifest = true;
                break;
            }
        }
        if !has_manifest {
            continue;
        }
        let name = item.file_name().into_string().map_err(|name| {
            anyhow::anyhow!("{} is not named in UTF-8", Path::new(&name).display())
        })?;
        if name.is_empty() || name.contains(|c: char| c == ':' || c.is_whitespace()) {
            bail!(
                "{} is not a valid adapter Feature name",
                item.path().display()
            );
        }
        names.push(name);
    }
    names.sort();
    Ok(names)
}
