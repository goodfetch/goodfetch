//! `xtask codegen` (ADRs 0038, 0087, 0094 and 0095): builds `goodfetch-protocol`'s
//! `emit` example, which writes the JSON Schema bundle and catalog file
//! through schemars, and writes from them every generated file: each
//! Product's projection, the names only Foxhound carries, the cargo-deny
//! `wrappers` list of Foxhound's crates, and each SDK package's `generated/`
//! layer.
//!
//! `--check` regenerates in memory what `xtask codegen` writes itself and
//! fails on any difference. It runs neither outside generator: the output of
//! `json-schema-to-typescript` and `datamodel-code-generator` is checked
//! against the digests recorded when it was written, of its input (a type
//! changed and not regenerated) and of itself (a hand edit).
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs and process bans target async crates (STYLE.md)"
)]

mod deny;
mod digest;
mod emitter;
mod layer;
mod names;
mod naming;
mod projection;
mod python;
mod typescript;

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context as _, Result, bail};
use serde_json::Value;

use crate::check::Finding;
use layer::{Layer, Product};

/// Where `goodfetch-protocol`'s committed outputs live.
const PROTOCOL_GENERATED: &str = "crates/goodfetch-protocol/generated";

/// The digests of each outside generator's input and output, written with them.
const DIGESTS: &str = "crates/goodfetch-protocol/generated/digests.json";

/// The cargo-deny configuration that holds the `wrappers` list.
const DENY: &str = "deny.toml";

#[derive(Clone, Copy)]
enum Language {
    TypeScript,
    Python,
}

/// One SDK package's generated layer (ADR 0095): each language's Collie
/// client package holds Collie's entries, Foxhound's package its own.
struct Package {
    language: Language,
    product: Product,
    folder: &'static str,
}

const PACKAGES: [Package; 4] = [
    Package {
        language: Language::TypeScript,
        product: Product::Collie,
        folder: "sdks/typescript/collie-client/src/generated",
    },
    Package {
        language: Language::TypeScript,
        product: Product::Foxhound,
        folder: "sdks/typescript/foxhound/src/generated",
    },
    Package {
        language: Language::Python,
        product: Product::Collie,
        folder: "sdks/python/collie-client/src/goodfetch_collie_client/generated",
    },
    Package {
        language: Language::Python,
        product: Product::Foxhound,
        folder: "sdks/python/foxhound/src/goodfetch_foxhound/generated",
    },
];

/// The outside generators, as paths to their executables.
pub(crate) struct Generators {
    pub(crate) json2ts: PathBuf,
    pub(crate) datamodel_codegen: PathBuf,
}

#[derive(Clone, Copy)]
enum Generator {
    Json2ts,
    DatamodelCodegen,
}

/// A file an outside generator writes from a JSON Schema input.
struct External {
    output: String,
    generator: Generator,
    /// The input's file name, which `datamodel-code-generator` writes into
    /// its header, so it never varies.
    input_name: &'static str,
    input: Vec<u8>,
    /// Lines put before the generator's output: the imports of types
    /// another package defines.
    prefix: String,
}

/// What one language's generator gives for one package's layer.
struct Generated {
    /// The files `xtask codegen` writes itself, by name in the layer's folder.
    files: Vec<(&'static str, String)>,
    /// The types file, which the language's outside generator writes.
    types: TypesFile,
}

struct TypesFile {
    /// Its name in the layer's folder.
    output: &'static str,
    /// The JSON Schema the generator reads.
    input: Value,
    /// Lines put before the generator's output.
    prefix: String,
}

/// Everything `xtask codegen` writes, by path relative to the root.
struct Plan {
    files: BTreeMap<String, Vec<u8>>,
    externals: Vec<External>,
}

impl Plan {
    fn owned_folders() -> impl Iterator<Item = &'static str> {
        std::iter::once(PROTOCOL_GENERATED).chain(PACKAGES.iter().map(|package| package.folder))
    }

    /// Every file `xtask codegen` writes inside the folders it owns.
    fn written(&self) -> BTreeSet<&str> {
        self.files
            .keys()
            .map(String::as_str)
            .chain(
                self.externals
                    .iter()
                    .map(|external| external.output.as_str()),
            )
            .chain(std::iter::once(DIGESTS))
            .collect()
    }
}

fn plan(cargo: &OsStr, root: &Path) -> Result<Plan> {
    let emitted = emitter::emit(cargo, root)?;
    let bundle: Value =
        serde_json::from_slice(&emitted.bundle).context("reading the emitted bundle")?;
    let catalog: Value =
        serde_json::from_slice(&emitted.catalog).context("reading the emitted catalog")?;

    let mut files = BTreeMap::new();
    files.insert(format!("{PROTOCOL_GENERATED}/bundle.json"), emitted.bundle);
    files.insert(
        format!("{PROTOCOL_GENERATED}/catalog.json"),
        emitted.catalog,
    );

    let collie = projection::project(&catalog, &bundle, Product::Collie)?;
    let foxhound = projection::project(&catalog, &bundle, Product::Foxhound)?;
    files.insert(
        format!("{PROTOCOL_GENERATED}/collie.json"),
        json_text(&collie)?,
    );
    files.insert(
        format!("{PROTOCOL_GENERATED}/foxhound.json"),
        json_text(&foxhound)?,
    );
    files.insert(
        format!("{PROTOCOL_GENERATED}/foxhound_names.rs"),
        names::render(&catalog).into_bytes(),
    );

    let deny_path = root.join(DENY);
    let deny_text = std::fs::read_to_string(&deny_path)
        .with_context(|| format!("reading {}", deny_path.display()))?;
    let crates = deny::foxhound_crates(root)?;
    files.insert(
        DENY.to_owned(),
        deny::render(&deny_text, &crates).into_bytes(),
    );

    let mut externals = Vec::new();
    for package in &PACKAGES {
        let layer = Layer::new(&collie, &foxhound, package.product)?;
        let generated = match package.language {
            Language::TypeScript => typescript::generate(&layer),
            Language::Python => python::generate(&layer),
        };
        for (name, text) in generated.files {
            files.insert(format!("{}/{name}", package.folder), text.into_bytes());
        }
        externals.push(External {
            output: format!("{}/{}", package.folder, generated.types.output),
            generator: match package.language {
                Language::TypeScript => Generator::Json2ts,
                Language::Python => Generator::DatamodelCodegen,
            },
            input_name: package.product.input_name(),
            input: json_text(&generated.types.input)?,
            prefix: generated.types.prefix,
        });
    }
    Ok(Plan { files, externals })
}

/// JSON as every generated file writes it: pretty, with a final newline.
fn json_text(value: &Value) -> Result<Vec<u8>> {
    let mut text = serde_json::to_string_pretty(value)?;
    text.push('\n');
    Ok(text.into_bytes())
}

/// Writes every generated file, removing any other file from the folders
/// `xtask codegen` owns.
pub(crate) fn write(cargo: &OsStr, generators: &Generators, root: &Path) -> Result<()> {
    let plan = plan(cargo, root)?;
    let written = plan.written();
    for folder in Plan::owned_folders() {
        for file in files_under(root, folder)? {
            if !written.contains(file.as_str()) {
                std::fs::remove_file(root.join(&file))
                    .with_context(|| format!("removing {file}"))?;
            }
        }
    }
    for (path, contents) in &plan.files {
        write_file(&root.join(path), contents)?;
    }
    let scratch = tempfile::tempdir().context("creating a scratch folder")?;
    let mut digests = digest::Digests::default();
    for external in &plan.externals {
        let output = run_generator(external, generators, root, scratch.path())?;
        write_file(&root.join(&external.output), &output)?;
        digests.record(&external.output, &external.input, &output);
    }
    write_file(&root.join(DIGESTS), &json_text(&digests.to_json()?)?)?;
    Ok(())
}

/// What `--check` finds: files that differ from what the types generate.
pub(crate) fn check(cargo: &OsStr, root: &Path) -> Result<Vec<Finding>> {
    let plan = plan(cargo, root)?;
    let mut findings = Vec::new();
    for (path, expected) in &plan.files {
        if std::fs::read(root.join(path)).ok().as_ref() != Some(expected) {
            findings.push(Finding::new(
                "drift",
                path.as_str(),
                "differs from what `xtask codegen` writes; run it and commit the result",
            ));
        }
    }
    let digests = digest::Digests::read(&root.join(DIGESTS))?;
    for external in &plan.externals {
        let actual = std::fs::read(root.join(&external.output)).ok();
        match digests.verify(&external.output, &external.input, actual.as_deref()) {
            digest::Verdict::Current => {}
            digest::Verdict::Stale => findings.push(Finding::new(
                "stale",
                external.output.as_str(),
                "was generated from other types; run `xtask codegen` and commit the result",
            )),
            digest::Verdict::Edited => findings.push(Finding::new(
                "edited",
                external.output.as_str(),
                "differs from what its generator wrote; generated code is never edited by hand",
            )),
        }
    }
    let written = plan.written();
    for folder in Plan::owned_folders() {
        for file in files_under(root, folder)? {
            if !written.contains(file.as_str()) {
                findings.push(Finding::new(
                    "unexpected",
                    file,
                    "is no file `xtask codegen` writes; generated folders hold only its output",
                ));
            }
        }
    }
    Ok(findings)
}

/// Caches tools leave beside the files they read, which are never output.
const IGNORED: [&str; 2] = ["__pycache__", ".DS_Store"];

/// Every file under `folder`, as paths relative to the root with `/`.
fn files_under(root: &Path, folder: &str) -> Result<Vec<String>> {
    let mut files = Vec::new();
    let mut pending = vec![root.join(folder)];
    while let Some(current) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&current) else {
            continue;
        };
        for entry in entries {
            let entry = entry.with_context(|| format!("reading {}", current.display()))?;
            if IGNORED.contains(&entry.file_name().to_string_lossy().as_ref()) {
                continue;
            }
            let path = entry.path();
            if entry.file_type()?.is_dir() {
                pending.push(path);
            } else {
                let relative = path.strip_prefix(root).unwrap_or(&path);
                files.push(relative.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    files.sort();
    Ok(files)
}

fn write_file(path: &Path, contents: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    std::fs::write(path, contents).with_context(|| format!("writing {}", path.display()))
}

fn run_generator(
    external: &External,
    generators: &Generators,
    root: &Path,
    scratch: &Path,
) -> Result<Vec<u8>> {
    let input = scratch.join(external.input_name);
    let output = scratch.join("output");
    std::fs::write(&input, &external.input)
        .with_context(|| format!("writing {}", input.display()))?;
    let mut command = match external.generator {
        Generator::Json2ts => {
            let mut command = Command::new(std::path::absolute(root.join(&generators.json2ts))?);
            command
                .arg("--input")
                .arg(&input)
                .arg("--output")
                .arg(&output)
                .args([
                    "--unreachableDefinitions",
                    "--additionalProperties",
                    "false",
                ]);
            command
        }
        Generator::DatamodelCodegen => {
            let mut command = Command::new(std::path::absolute(
                root.join(&generators.datamodel_codegen),
            )?);
            command
                .arg("--input")
                .arg(&input)
                .arg("--output")
                .arg(&output)
                .args(python::DATAMODEL_CODEGEN_OPTIONS);
            command
        }
    };
    // The input's name, never its folder, reaches the generated header.
    command.current_dir(scratch);
    let result = command.output().with_context(|| {
        format!(
            "running the generator of {}; install the tools with `pnpm install` and `uv sync`",
            external.output
        )
    })?;
    if !result.status.success() {
        bail!(
            "the generator of {} failed: {}",
            external.output,
            String::from_utf8_lossy(&result.stderr)
        );
    }
    let generated = std::fs::read(&output)
        .with_context(|| format!("reading what the generator of {} wrote", external.output))?;
    let mut contents = external.prefix.clone().into_bytes();
    contents.extend_from_slice(&generated);
    Ok(contents)
}
