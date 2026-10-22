//! The TLA+ models: each folder of `test-support/models/` holding a
//! `model.toml`, which names the model's TLA+ module and the modules whose
//! behaviour it describes, the ones it traces (ADR 0060). The folder also
//! holds the model's TLC configurations, `small.cfg` and `large.cfg`.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::path::Path;

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;

/// The models' folder, from the repository root.
pub(crate) const MODELS_FOLDER: &str = "test-support/models";

/// Each model's manifest, in its folder.
const MANIFEST: &str = "model.toml";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    spec: String,
    traces: Vec<String>,
}

pub(crate) struct Model {
    /// Its folder's name.
    pub(crate) name: String,
    /// Its folder, from the repository root.
    pub(crate) folder: String,
    /// The TLA+ module TLC checks, a file of its folder.
    pub(crate) spec: String,
    /// The files and folders, from the repository root, of the modules it
    /// traces.
    pub(crate) traces: Vec<String>,
}

/// Every model, by name; none when the models' folder does not exist.
pub(crate) fn read(root: &Path) -> Result<Vec<Model>> {
    let folder = root.join(MODELS_FOLDER);
    if !folder.is_dir() {
        return Ok(Vec::new());
    }
    let mut models = Vec::new();
    let entries =
        std::fs::read_dir(&folder).with_context(|| format!("reading {}", folder.display()))?;
    for entry in entries {
        let entry = entry.with_context(|| format!("reading {}", folder.display()))?;
        let manifest_path = entry.path().join(MANIFEST);
        if !manifest_path.is_file() {
            continue;
        }
        let name = entry
            .file_name()
            .into_string()
            .map_err(|name| anyhow::anyhow!("{} is not UTF-8", name.display()))?;
        let text = std::fs::read_to_string(&manifest_path)
            .with_context(|| format!("reading {}", manifest_path.display()))?;
        let manifest: Manifest = toml::from_str(&text)
            .with_context(|| format!("parsing {}", manifest_path.display()))?;
        let folder = format!("{MODELS_FOLDER}/{name}");
        if Path::new(&manifest.spec)
            .extension()
            .is_none_or(|extension| extension != "tla")
            || manifest.spec.contains(['/', '\\'])
        {
            bail!(
                "{folder}/{MANIFEST}: the spec `{}` is not a .tla file of the model's folder",
                manifest.spec
            );
        }
        let traces: Vec<String> = manifest
            .traces
            .into_iter()
            .map(|trace| trace.trim_end_matches('/').to_owned())
            .collect();
        // A renamed or removed module would silently stop selecting the
        // model, and the gate would pass without it.
        if let Some(missing) = traces.iter().find(|trace| !root.join(trace).exists()) {
            bail!(
                "{folder}/{MANIFEST}: the traced path `{missing}` does not exist; trace the module where it now lives"
            );
        }
        models.push(Model {
            name,
            folder,
            spec: manifest.spec,
            traces,
        });
    }
    models.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(models)
}
