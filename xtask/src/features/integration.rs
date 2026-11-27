//! The `integration:` Features: one per Integration action or trigger declared
//! in `integrations/<product>/<tool>/integration.toml`, named
//! `integration:<tool>/<name>` (ADRs 0070, 0072 and 0095). Entries declared
//! under `integrations/collie/` ship with both Products; entries declared only
//! under `integrations/foxhound/` ship with Foxhound alone, and an entry
//! repeated by Foxhound deduplicates into Collie's `Both` Feature. A folder
//! without `integration.toml`, or a workspace without `integrations/`,
//! generates nothing.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::io::ErrorKind;
use std::path::Path;

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;

use super::entry::{Carriers, Entry, unreadable};
use super::files::{self, Kind, Status};

const INTEGRATIONS: &str = "integrations";
const MANIFEST: &str = "integration.toml";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct IntegrationManifest {
    #[serde(default)]
    actions: Vec<String>,
    #[serde(default)]
    triggers: Vec<String>,
}

pub(super) fn entries(root: &Path) -> Result<Vec<Entry>> {
    let mut by_name = BTreeMap::new();
    for (product, carriers) in [("collie", Carriers::Both), ("foxhound", Carriers::Foxhound)] {
        for feature in read_product(root, product)? {
            by_name.entry(feature).or_insert(carriers);
        }
    }
    Ok(by_name
        .into_iter()
        .map(|(name, carriers)| Entry {
            kind: Kind::Integration,
            name,
            area: None,
            milestone: None,
            status: Status::Active,
            carriers,
        })
        .collect())
}

fn read_product(root: &Path, product: &str) -> Result<Vec<String>> {
    let folder = root.join(INTEGRATIONS).join(product);
    let listing = match std::fs::read_dir(&folder) {
        Ok(listing) => listing,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error).with_context(|| format!("listing {}", folder.display())),
    };
    let mut tools = Vec::new();
    for item in listing {
        let item = item.with_context(|| format!("listing {}", folder.display()))?;
        if !item
            .file_type()
            .with_context(|| format!("reading {}", item.path().display()))?
            .is_dir()
        {
            continue;
        }
        let manifest_path = item.path().join(MANIFEST);
        if !manifest_path
            .try_exists()
            .with_context(|| format!("reading {}", manifest_path.display()))?
        {
            continue;
        }
        let tool = item.file_name().into_string().map_err(|name| {
            anyhow::anyhow!("{} is not named in UTF-8", Path::new(&name).display())
        })?;
        validate_segment(&tool, &manifest_path.display().to_string())?;
        tools.push(tool);
    }
    tools.sort();
    let mut features = Vec::new();
    for tool in tools {
        let relative = format!("{INTEGRATIONS}/{product}/{tool}/{MANIFEST}");
        let manifest: IntegrationManifest =
            files::read_toml(root, &relative).map_err(|error| unreadable(error, &relative))?;
        let mut seen = BTreeSet::new();
        for name in manifest.actions.into_iter().chain(manifest.triggers) {
            validate_segment(&name, &relative)?;
            if !seen.insert(name.clone()) {
                bail!("{relative} lists `{name}` more than once");
            }
            features.push(format!("{tool}/{name}"));
        }
    }
    Ok(features)
}

fn validate_segment(segment: &str, source: &str) -> Result<()> {
    if segment.is_empty() || segment.contains(|c: char| matches!(c, ':' | '/') || c.is_whitespace())
    {
        bail!("{source} has invalid Integration Feature name `{segment}`");
    }
    Ok(())
}
