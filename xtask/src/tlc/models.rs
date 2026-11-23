//! The TLA+ models: each folder of `test-support/models/` holding a
//! `model.toml`, which names the model's TLA+ module and the modules whose
//! behaviour it describes, the ones it traces (ADR 0060). The folder also
//! holds the model's TLC configurations, `small.cfg` and `large.cfg`. A
//! model that validates traces names its trace spec, a module of its folder
//! built on the template of [`TEMPLATE_FOLDER`] and checked with its
//! `trace.cfg`, and the committed traces of real runs it validates.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::path::Path;

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;

/// The models' folder, from the repository root.
pub(crate) const MODELS_FOLDER: &str = "test-support/models";

/// The trace-validation spec template's folder, from the repository root,
/// which TLC reads as a library.
pub(crate) const TEMPLATE_FOLDER: &str = "test-support/models/trace-validation";

/// Each model's manifest, in its folder.
const MANIFEST: &str = "model.toml";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    spec: String,
    traces: Vec<String>,
    trace_spec: Option<String>,
    #[serde(default)]
    trace_files: Vec<String>,
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
    /// Its trace-validation spec, a file of its folder, when it validates
    /// traces.
    pub(crate) trace_spec: Option<String>,
    /// The committed traces, from the repository root, that its trace spec
    /// validates.
    pub(crate) trace_files: Vec<String>,
}

/// Whether `spec` is a .tla file of the model's folder.
fn is_module_of_folder(spec: &str) -> bool {
    Path::new(spec)
        .extension()
        .is_some_and(|extension| extension == "tla")
        && !spec.contains(['/', '\\'])
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
        if let Some(spec) = std::iter::once(&manifest.spec)
            .chain(&manifest.trace_spec)
            .find(|spec| !is_module_of_folder(spec))
        {
            bail!(
                "{folder}/{MANIFEST}: the spec `{spec}` is not a .tla file of the model's folder"
            );
        }
        if manifest.trace_spec.is_none() && !manifest.trace_files.is_empty() {
            bail!("{folder}/{MANIFEST}: `trace_files` needs a `trace_spec` to validate them");
        }
        if let Some(missing) = manifest
            .trace_files
            .iter()
            .find(|file| !root.join(file).is_file())
        {
            bail!("{folder}/{MANIFEST}: the trace file `{missing}` does not exist");
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
            trace_spec: manifest.trace_spec,
            trace_files: manifest.trace_files,
        });
    }
    models.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(models)
}
