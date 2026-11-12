//! `xtask codegen` (ADRs 0038, 0087, 0094 and 0095): emits the JSON Schema
//! bundle and catalog file from `goodfetch-protocol`'s types through
//! schemars (`emit`), and writes from them every generated file: each
//! Product's projection, the names only Foxhound carries, the namespaces of
//! the commands Collie's executable generates, the cargo-deny
//! `wrappers` list of Foxhound's crates and Collie's ban of them, cargo-deny's
//! copy of the license allowlist, each SDK package's `generated/` layer, and
//! the Console's operation map from Collie's projection (spec #592) and
//! Foxhound's Console's from Foxhound's (spec #593),
//! each registered format's refusal cases (ADR 0066), the Journey files'
//! schema on its own, `journeys/journey.schema.json`, for
//! editors and the TypeScript and Python helpers (ADR 0063), and the result
//! document's types those helpers write it with, in their `test-support/`
//! packages. It also commits
//! the JSON Schemas of `goodfetch-test-support`'s file types, which
//! `goodfetch-test-env --print-schemas` prints (ADR 0062), the script
//! format's among the engine scripts (ADR 0069), and `goodfetch-cdp`'s CDP
//! bindings, from the pinned Chromium's protocol file (spec #577), and the
//! facts tests read across crates and languages (`facts`): the Secret
//! canary kinds, the pinned browser builds and the `tracing` targets, each
//! written from its one source in Rust for test support, `xtask` and the
//! TypeScript and Python helpers.
//! It writes the resolved-address classification's table into
//! `goodfetch-engine`, from IANA's special-purpose registries committed
//! beside it with their date (ADR 0067), and the label values of
//! `goodfetch.counted_work`, from `goodfetch-counted-work`'s rows (ADR 0140).
//!
//! `--check` regenerates in memory what `xtask codegen` writes itself and
//! fails on any difference. It runs neither outside generator: the output of
//! `json-schema-to-typescript` and `datamodel-code-generator` is checked
//! against what was recorded when it was written: the digest of its input
//! and options, and its tools' versions as the lockfiles pin them (a type,
//! an option or a version changed and not regenerated), and the digest of
//! itself (a hand edit). Both generators' files open with `xtask codegen`'s
//! own header; the Python models have Google docstrings in ruff's format.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs and process bans target async crates (STYLE.md)"
)]

mod cdp;
mod console;
mod counted_work;
mod deny;
mod digest;
mod emit;
mod emitter;
mod executables;
mod facts;
mod helpers;
mod layer;
mod names;
mod naming;
mod projection;
mod python;
mod refused;
mod special_purpose;
mod test_support;
mod typescript;

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use anyhow::{Context as _, Result, bail};
use serde_json::{Map, Value};

use crate::check::Finding;
use crate::compat;
use crate::environment::Environment;
use crate::features;
use emit::JOURNEY_FILES;
use goodfetch_protocol::catalog::Product;
use layer::Layer;

/// Where `goodfetch-protocol`'s committed outputs live.
const PROTOCOL_GENERATED: &str = "crates/goodfetch-protocol/generated";

/// The digests of each outside generator's input and output, written with them.
const DIGESTS: &str = "crates/goodfetch-protocol/generated/digests.json";

/// The schema of every Journey file, written on its own from the bundle's
/// `files` (ADR 0063).
const JOURNEY_SCHEMA: &str = "journeys/journey.schema.json";

/// The cargo-deny configuration that holds the `wrappers` list and the
/// licenses.
const DENY: &str = "deny.toml";

#[derive(Clone, Copy)]
enum Language {
    TypeScript,
    Python,
}

impl Language {
    /// The outside generator that writes its layer's types.
    fn generator(self) -> Generator {
        match self {
            Self::TypeScript => Generator::Json2ts,
            Self::Python => Generator::DatamodelCodegen,
        }
    }
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

impl Generator {
    /// The tools whose versions shape this generator's output, each with the
    /// lockfile that pins it: `datamodel-code-generator` formats its output
    /// with ruff.
    fn tools(self) -> &'static [(&'static str, Lockfile)] {
        match self {
            Self::Json2ts => &[("json-schema-to-typescript", Lockfile::Pnpm)],
            Self::DatamodelCodegen => &[
                ("datamodel-code-generator", Lockfile::Uv),
                ("ruff", Lockfile::Uv),
            ],
        }
    }
}

#[derive(Clone, Copy)]
enum Lockfile {
    /// `pnpm-lock.yaml`: the root importer's dependencies.
    Pnpm,
    /// `uv.lock`: its packages.
    Uv,
}

/// The locked version of each tool of `generator`, read from the lockfiles at
/// `root`, so `--check` notices a bump without running the generator.
fn tool_versions(root: &Path, generator: Generator) -> Result<digest::Tools> {
    let mut versions = digest::Tools::new();
    for (tool, lockfile) in generator.tools() {
        let version = match lockfile {
            Lockfile::Pnpm => {
                let path = root.join("pnpm-lock.yaml");
                let text = std::fs::read_to_string(&path)
                    .with_context(|| format!("reading {}", path.display()))?;
                // pnpm writes its configuration's lock as a first document.
                let documents: Vec<Value> = serde_saphyr::from_str_multiple(&text)
                    .with_context(|| format!("parsing {}", path.display()))?;
                documents.iter().find_map(|lock| {
                    let importer = &lock["importers"]["."];
                    ["dependencies", "devDependencies"]
                        .iter()
                        .find_map(|section| importer[*section][*tool]["version"].as_str())
                        .map(str::to_owned)
                })
            }
            Lockfile::Uv => {
                let path = root.join("uv.lock");
                let text = std::fs::read_to_string(&path)
                    .with_context(|| format!("reading {}", path.display()))?;
                let lock: Value =
                    toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
                lock["package"].as_array().and_then(|packages| {
                    packages
                        .iter()
                        .find(|package| package["name"].as_str() == Some(tool))
                        .and_then(|package| package["version"].as_str())
                        .map(str::to_owned)
                })
            }
        };
        let version = version.with_context(|| format!("no lockfile pins {tool}"))?;
        versions.insert((*tool).to_owned(), version);
    }
    Ok(versions)
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
    /// What the generator writes first, its banner or file header.
    header: String,
}

impl External {
    /// Everything the generator's output follows from, besides its version:
    /// its input, its options and the header it writes, so a changed option
    /// that was not regenerated is stale too.
    fn recipe(&self) -> Vec<u8> {
        let options: &[&str] = match self.generator {
            Generator::Json2ts => &JSON2TS_OPTIONS,
            Generator::DatamodelCodegen => &python::DATAMODEL_CODEGEN_OPTIONS,
        };
        let rewrites: &[&str] = match self.generator {
            Generator::Json2ts => &[],
            Generator::DatamodelCodegen => &[python::model_defaults::REWRITE],
        };
        let mut recipe = self.input.clone();
        for part in options
            .iter()
            .chain(rewrites)
            .copied()
            .chain([self.header.as_str(), &self.prefix])
        {
            recipe.push(b'\0');
            recipe.extend_from_slice(part.as_bytes());
        }
        recipe
    }
}

/// How `xtask codegen` runs `json-schema-to-typescript`, before its banner.
/// A bounded array stays an array (`--maxItems -1`) rather than a union of
/// every tuple length: the engine checks the bound, and a program's
/// `string[]` is then a value of the type.
const JSON2TS_OPTIONS: [&str; 5] = [
    "--unreachableDefinitions",
    "--additionalProperties",
    "false",
    "--maxItems",
    "-1",
];

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
    /// What the generator writes first: `json-schema-to-typescript`'s banner,
    /// `datamodel-code-generator`'s file header.
    header: String,
}

/// Everything `xtask codegen` writes, by path relative to the root.
struct Plan {
    files: BTreeMap<String, Vec<u8>>,
    externals: Vec<External>,
}

impl Plan {
    fn owned_folders() -> impl Iterator<Item = &'static str> {
        [
            PROTOCOL_GENERATED,
            special_purpose::OUTPUT_FOLDER,
            executables::COLLIE_FOLDER,
            cdp::FOLDER,
            test_support::FOLDER,
            facts::TEST_SUPPORT_FOLDER,
            facts::XTASK_FOLDER,
            helpers::TYPESCRIPT_FOLDER,
            helpers::PYTHON_FOLDER,
            console::FOLDER,
            console::FOXHOUND_FOLDER,
        ]
        .into_iter()
        .chain(PACKAGES.iter().map(|package| package.folder))
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

/// Everything `xtask codegen` writes at `root`, with the executables' files
/// `executables` gives.
fn plan(cargo: &OsStr, root: &Path, executables: executables::Files) -> Result<Plan> {
    let emitted = emitter::emit(cargo, root)?;
    let bundle: Value =
        serde_json::from_slice(&emitted.bundle).context("reading the emitted bundle")?;
    let catalog: Value =
        serde_json::from_slice(&emitted.catalog).context("reading the emitted catalog")?;

    let mut files = compat::refusals::files(&emitted.bundle)?;
    files.insert(format!("{PROTOCOL_GENERATED}/bundle.json"), emitted.bundle);
    files.insert(
        format!("{PROTOCOL_GENERATED}/catalog.json"),
        emitted.catalog,
    );

    let collie = projection::project(&catalog, &bundle, Product::Collie)?;
    let foxhound = projection::project(&catalog, &bundle, Product::Foxhound)?;
    refused::check(&[&collie, &foxhound])?;
    files.insert(
        format!("{PROTOCOL_GENERATED}/collie.json"),
        json_text(&collie)?,
    );
    files.insert(
        format!("{PROTOCOL_GENERATED}/foxhound.json"),
        json_text(&foxhound)?,
    );
    let command_lines = executables.command_lines()?;
    let foxhound_names = names::table(&catalog, command_lines.as_ref());
    files.insert(
        format!("{PROTOCOL_GENERATED}/foxhound_names.rs"),
        names::render(&foxhound_names).into_bytes(),
    );
    files.extend(special_purpose::generate(root)?);
    files.extend(counted_work::generate(root)?);
    files.extend(executables.into_files());
    let sources = features::Sources {
        root,
        catalog: &catalog,
        bundle: &bundle,
        foxhound_names: &foxhound_names,
        command_lines: command_lines.as_ref(),
    };
    files.insert(
        features::INVENTORY.to_owned(),
        features::inventory_text(&sources)?,
    );
    for (path, schema) in features::schemas() {
        files.insert(path.to_owned(), json_text(&schema)?);
    }
    files.insert(
        JOURNEY_SCHEMA.to_owned(),
        json_text(&standalone(&bundle, JOURNEY_FILES)?)?,
    );

    let deny_path = root.join(DENY);
    let deny_text = std::fs::read_to_string(&deny_path)
        .with_context(|| format!("reading {}", deny_path.display()))?;
    let crates = deny::foxhound_crates(root)?;
    let licenses = deny::crate_licenses(root)?;
    files.insert(
        DENY.to_owned(),
        deny::render_workspace(&deny_text, &crates, &licenses).into_bytes(),
    );
    let collie_path = root.join(deny::COLLIE);
    let collie_text = std::fs::read_to_string(&collie_path)
        .with_context(|| format!("reading {}", collie_path.display()))?;
    files.insert(
        deny::COLLIE.to_owned(),
        deny::render_collie(&collie_text, &crates).into_bytes(),
    );

    for (path, schema) in test_support::schemas(cargo, root)? {
        files.insert(path, json_text(&schema)?);
    }

    files.insert(
        test_support::CANARY_RULE.to_owned(),
        test_support::canary_rule(cargo, root)?,
    );
    files.extend(facts::files(cargo, root)?);
    files.extend(cdp::generate(root)?);
    for (path, text) in console::all_files(root, &collie, &foxhound, &foxhound_names)? {
        files.insert(path, text.into_bytes());
    }

    let version = product_version(root)?;
    let mut externals = Vec::new();
    for package in &PACKAGES {
        let layer = Layer::new(&bundle, &collie, &foxhound, package.product, &version)?;
        let generated = match package.language {
            Language::TypeScript => typescript::generate(&layer),
            Language::Python => python::generate(&layer),
        };
        for (name, text) in generated.files {
            files.insert(format!("{}/{name}", package.folder), text.into_bytes());
        }
        externals.push(External {
            output: format!("{}/{}", package.folder, generated.types.output),
            generator: package.language.generator(),
            input_name: layer::input_name(package.product),
            input: json_text(&generated.types.input)?,
            prefix: generated.types.prefix,
            header: generated.types.header,
        });
    }
    helpers::add(&bundle, &mut files, &mut externals)?;
    Ok(Plan { files, externals })
}

/// The schema the bundle's `files` gives `key`, on its own: the bundle's
/// draft, with the types it reaches under its own `$defs`.
fn standalone(bundle: &Value, key: &str) -> Result<Value> {
    let root = bundle
        .get("files")
        .and_then(|files| files.get(key))
        .and_then(Value::as_object)
        .with_context(|| format!("the bundle's `files` maps no `{key}`"))?;
    let definitions = bundle
        .get("$defs")
        .and_then(Value::as_object)
        .context("the bundle has no `$defs`")?;
    let reached = projection::reached(&Value::Object(root.clone()), definitions)?;
    let mut schema = Map::new();
    if let Some(draft) = bundle.get("$schema") {
        schema.insert("$schema".to_owned(), draft.clone());
    }
    schema.extend(root.clone());
    schema.insert(
        "$defs".to_owned(),
        Value::Object(
            definitions
                .iter()
                .filter(|(name, _)| reached.contains(name.as_str()))
                .map(|(name, definition)| (name.clone(), definition.clone()))
                .collect(),
        ),
    );
    Ok(Value::Object(schema))
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
    let plan = plan(cargo, root, executables::Files::printed(cargo, root)?)?;
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
        let tools = tool_versions(root, external.generator)?;
        let output = run_generator(external, generators, root, scratch.path())?;
        write_file(&root.join(&external.output), &output)?;
        digests.record(&external.output, &external.recipe(), &tools, &output);
    }
    write_file(&root.join(DIGESTS), &json_text(&digests.to_json()?)?)?;
    Ok(())
}

/// What `--check` finds: files that differ from what the types generate.
pub(crate) fn check(cargo: &OsStr, root: &Path) -> Result<Vec<Finding>> {
    let plan = plan(cargo, root, executables::Files::committed(root))?;
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
        let tools = tool_versions(root, external.generator)?;
        match digests.verify(&external.output, &external.recipe(), &tools, actual.as_deref()) {
            digest::Verdict::Current => {}
            digest::Verdict::Stale => findings.push(Finding::new(
                "stale",
                external.output.as_str(),
                "was generated from other types or by another version of its generator; run `xtask codegen` and commit the result",
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
                .args(JSON2TS_OPTIONS)
                .arg("--bannerComment")
                .arg(&external.header);
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
                .args(python::DATAMODEL_CODEGEN_OPTIONS)
                .arg("--custom-file-header")
                .arg(&external.header);
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
    let mut generated = std::fs::read(&output)
        .with_context(|| format!("reading what the generator of {} wrote", external.output))?;
    if let Generator::DatamodelCodegen = external.generator {
        let text = String::from_utf8(generated)
            .with_context(|| format!("reading {}'s generated text", external.output))?;
        generated = match python::model_defaults::rewrite(&text) {
            Some(rewritten) => {
                ruff_format(&generators.datamodel_codegen, root, scratch, &rewritten)
                    .with_context(|| format!("formatting {} again", external.output))?
            }
            None => text.into_bytes(),
        };
    }
    let mut contents = external.prefix.clone().into_bytes();
    contents.extend_from_slice(&generated);
    Ok(contents)
}

/// `text`, Python, formatted by the ruff beside `datamodel_codegen`, run in
/// `scratch` as `datamodel-code-generator` runs it, so its settings are the
/// same.
fn ruff_format(
    datamodel_codegen: &Path,
    root: &Path,
    scratch: &Path,
    text: &str,
) -> Result<Vec<u8>> {
    use std::io::Write as _;
    let ruff = std::path::absolute(root.join(datamodel_codegen.with_file_name("ruff")))?;
    let mut child = Command::new(&ruff)
        .args(["format", "--stdin-filename", "models.py", "-"])
        .current_dir(scratch)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .with_context(|| {
            format!(
                "running {}; install the tools with `uv sync`",
                ruff.display()
            )
        })?;
    child
        .stdin
        .take()
        .context("ruff's standard input")?
        .write_all(text.as_bytes())?;
    let result = child.wait_with_output()?;
    if !result.status.success() {
        bail!(
            "ruff format failed: {}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    Ok(result.stdout)
}

/// `xtask codegen`'s arguments.
#[derive(clap::Args)]
pub(crate) struct Arguments {
    /// Fails on any committed file that differs from what the types
    /// generate, without running the TypeScript and Python generators.
    #[arg(long)]
    check: bool,
    /// `json-schema-to-typescript`'s executable.
    #[arg(long, default_value = "node_modules/.bin/json2ts")]
    json2ts: PathBuf,
    /// `datamodel-code-generator`'s executable.
    #[arg(long, default_value = ".venv/bin/datamodel-codegen")]
    datamodel_codegen: PathBuf,
}

pub(crate) fn run(arguments: Arguments, environment: &Environment) -> Result<ExitCode> {
    let (cargo, root) = (environment.cargo.as_os_str(), environment.root.as_path());
    if arguments.check {
        return Ok(crate::check::report(&check(cargo, root)?));
    }
    let generators = Generators {
        json2ts: arguments.json2ts,
        datamodel_codegen: arguments.datamodel_codegen,
    };
    let before = executables::Files::committed(root);
    write(cargo, &generators, root)?;
    // The executables print the command lines they were built with, so once
    // the namespaces changed, they print them again built with the new ones.
    if executables::Files::committed(root).namespaces() != before.namespaces() {
        write(cargo, &generators, root)?;
    }
    Ok(ExitCode::SUCCESS)
}

/// The Product version the workspace carries: the root manifest's
/// `[workspace.package] version`, which every crate takes (ADR 0030).
fn product_version(root: &Path) -> Result<String> {
    let path = root.join("Cargo.toml");
    let manifest: toml::Table = toml::from_str(
        &std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?,
    )
    .with_context(|| format!("reading {}", path.display()))?;
    manifest
        .get("workspace")
        .and_then(|workspace| workspace.get("package"))
        .and_then(|package| package.get("version"))
        .and_then(toml::Value::as_str)
        .map(str::to_owned)
        .with_context(|| format!("{} has no `[workspace.package] version`", path.display()))
}
