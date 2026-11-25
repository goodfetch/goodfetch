//! The Recipe format (issue 1663): the manifest every engine, CLI and host
//! reads, `goodfetch-recipe.toml` at a package's root, which describes
//! exactly one Recipe: its name, the Product it needs (ADR 0096), its entry,
//! the runtime a program entry runs on and its settings with the ones its
//! users may set (ADR 0028). A Recipe's entry is a program, a step list or a
//! `CrawlDeclaration`, so a Recipe made of data holds no source and names no
//! runtime. The manifest is a versioned format of the registry (ADR 0030),
//! whose schema the bundle carries for editors and validators.
//!
//! [`read`] is the one rule that turns a manifest's document into a
//! [`RecipeManifest`], each refusal naming its field and the fix; a reader
//! checks [`version_of`] under the registry's rule first, so that a newer
//! manifest is refused for its version rather than for a field this reader
//! does not know.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::catalog::{Entries, ErrorCodeDefinition, ErrorGroup, Product, ProtocolVersion};
use crate::settings::Exposure;

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// The manifest format's current version, the registry's `recipe-manifest`
/// (ADR 0030).
pub const RECIPE_MANIFEST_VERSION: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// A Recipe's manifest file, at its package's root.
pub const RECIPE_MANIFEST_FILE: &str = "goodfetch-recipe.toml";

/// The file at a package's root whose rules, in gitignore's syntax, leave
/// files out of the package.
pub const RECIPE_IGNORE_FILE: &str = ".goodfetchignore";

/// What a package always leaves out, whatever its ignore file says, in
/// gitignore's syntax: version control, secrets files, dependency folders
/// and build output.
pub const ALWAYS_IGNORED: &[&str] = &[
    ".git/",
    ".env",
    ".env.*",
    "node_modules/",
    ".venv/",
    "venv/",
    "__pycache__/",
    "target/",
    "dist/",
    "build/",
];

/// A Recipe's name: lowercase ASCII letters, digits, `-` and `_`, opening
/// with a letter, at most 64 characters, so that a name is never read as a
/// path.
pub const RECIPE_NAME_PATTERN: &str = "^[a-z][a-z0-9_-]{0,63}$";

/// A Recipe's manifest (issue 1663; ADRs 0028, 0030 and 0096).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RecipeManifest {
    /// The version of this format the manifest was written in.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub version: ProtocolVersion,
    /// The Recipe's name.
    #[schemars(pattern(RECIPE_NAME_PATTERN), extend("arrived_in" = "0.1"))]
    pub name: String,
    /// The Product the Recipe needs, which `init` writes (ADR 0096).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub product: Product,
    /// What the Recipe runs.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub entry: RecipeEntry,
    /// The runtime a program entry runs on, which a step list or a
    /// `CrawlDeclaration` names none of.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub runtime: Option<Runtime>,
    /// The settings it declares, each locked for its users unless exposed.
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub settings: Map<String, Value>,
    /// The settings its users may set, by key, each with its bounds.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub expose: BTreeMap<String, Exposure>,
}

/// What a Recipe runs: exactly one of a program, a step list or a
/// `CrawlDeclaration`, each a file of the package named by its path from
/// the package's root, with `/` between its parts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum RecipeEntry {
    /// A program: its main source file, run on the manifest's runtime.
    #[schemars(extend("arrived_in" = "0.1"))]
    Program(String),
    /// A step list (issue 1685), replayed with no program.
    #[schemars(extend("arrived_in" = "0.1"))]
    Steps(String),
    /// A `CrawlDeclaration` (issue 1514), a crawl as data, which only
    /// Foxhound runs.
    #[schemars(extend("arrived_in" = "0.1"))]
    Crawl(String),
}

impl RecipeEntry {
    /// Its key in the manifest's `entry` table.
    pub fn key(&self) -> &'static str {
        match self {
            Self::Program(_) => "program",
            Self::Steps(_) => "steps",
            Self::Crawl(_) => "crawl",
        }
    }

    /// The path of its file from the package's root.
    pub fn path(&self) -> &str {
        match self {
            Self::Program(path) | Self::Steps(path) | Self::Crawl(path) => path,
        }
    }
}

/// The runtime a program entry runs on.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Runtime {
    /// Its language.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub language: RuntimeLanguage,
    /// Its version, one of those this Product version lists for the
    /// language ([`RuntimeLanguage::versions`]).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub version: String,
}

/// A language a Recipe's program may be written in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeLanguage {
    /// Node.js, for a TypeScript or JavaScript program.
    Node,
    /// CPython.
    Python,
    /// Rust's toolchain.
    Rust,
}

impl RuntimeLanguage {
    /// Every value, in declaration order.
    pub const ALL: [Self; 3] = [Self::Node, Self::Python, Self::Rust];

    /// Its name in a manifest.
    pub fn as_str(self) -> &'static str {
        crate::wire_name::of(&self)
    }

    /// The versions this Product version runs it at, oldest first: the Node
    /// lines and CPython minors the SDKs are tested on (`ci/runtimes.toml`)
    /// and Rust from the workspace's MSRV to its toolchain, which the
    /// starters' Dockerfiles build on.
    pub fn versions(self) -> &'static [&'static str] {
        match self {
            Self::Node => &["24", "26"],
            Self::Python => &["3.11", "3.12", "3.13", "3.14"],
            Self::Rust => &["1.97", "1.98", "1.99"],
        }
    }
}

/// A manifest that is no valid Recipe manifest (issue 1663).
pub const RECIPE_INVALID: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "recipe_invalid",
    description: "A Recipe manifest, `goodfetch-recipe.toml`, or its package that is not valid: a manifest that does not parse, lacks a required field or holds an unknown one, names no entry or two, names a program without a runtime or a runtime it does not list, names a `CrawlDeclaration` for Collie, or an entry the package does not hold. The details name the field and the fix.",
    group: ErrorGroup::Program,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// What `recipe_invalid` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RecipeInvalidDetails {
    /// The field, dotted from the manifest's root such as `entry.program`,
    /// or the package's path of a file it refuses.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub field: String,
    /// What is wrong with it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub reason: String,
    /// What to write instead.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub fix: String,
}

impl RecipeInvalidDetails {
    fn new(field: impl Into<String>, reason: impl Into<String>, fix: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            reason: reason.into(),
            fix: fix.into(),
        }
    }
}

/// The manifest's fields, in the order the schema lists them.
const FIELDS: [&str; 7] = [
    "version", "name", "product", "entry", "runtime", "settings", "expose",
];

/// The version a manifest's document, its TOML as JSON, was written in,
/// which a reader checks under the format registry's rule for
/// `recipe-manifest` before [`read`], so that a newer manifest is refused
/// for its version rather than for a field this reader does not know.
///
/// # Errors
///
/// `recipe_invalid`'s details for a document that is no table or names no
/// `major.minor` version.
pub fn version_of(document: &Value) -> Result<ProtocolVersion, RecipeInvalidDetails> {
    let Value::Object(top) = document else {
        return Err(not_a_table());
    };
    version(top.get("version"))
}

/// Reads a manifest's document, its TOML as JSON, whose version the reader
/// has accepted ([`version_of`]): every field, then the rules between them.
///
/// # Errors
///
/// `recipe_invalid`'s details for any fault, naming its field.
pub fn read(document: Value) -> Result<RecipeManifest, RecipeInvalidDetails> {
    let Value::Object(mut top) = document else {
        return Err(not_a_table());
    };
    let version = version(top.get("version"))?;
    top.remove("version");
    if let Some(unknown) = top.keys().find(|key| !FIELDS.contains(&key.as_str())) {
        return Err(RecipeInvalidDetails::new(
            unknown.clone(),
            format!("`{unknown}` is no field of a Recipe manifest"),
            format!("remove it; the fields are {}", listed(&FIELDS)),
        ));
    }
    let name: String = field(&mut top, "name")?.ok_or_else(|| missing("name"))?;
    if !is_recipe_name(&name) {
        return Err(RecipeInvalidDetails::new(
            "name",
            format!("`{name}` is no Recipe name"),
            "give lowercase ASCII letters, digits, `-` and `_`, opening with a letter, at most 64 characters",
        ));
    }
    let product: Product = field(&mut top, "product")?.ok_or_else(|| missing("product"))?;
    let entry = entry(top.remove("entry"))?;
    let runtime: Option<Runtime> = field(&mut top, "runtime")?;
    let manifest = RecipeManifest {
        version,
        name,
        product,
        entry,
        runtime,
        settings: field(&mut top, "settings")?.unwrap_or_default(),
        expose: field(&mut top, "expose")?.unwrap_or_default(),
    };
    check(&manifest)?;
    Ok(manifest)
}

/// The rules between fields: a program needs a runtime this Product version
/// lists, data names none, and a `CrawlDeclaration` needs Foxhound.
fn check(manifest: &RecipeManifest) -> Result<(), RecipeInvalidDetails> {
    let entry = &manifest.entry;
    let field = format!("entry.{}", entry.key());
    if let Err(reason) = package_path(entry.path()) {
        return Err(RecipeInvalidDetails::new(
            field,
            reason,
            "name the file by its path from the package's root, with `/` between its parts",
        ));
    }
    if matches!(entry, RecipeEntry::Crawl(_)) && manifest.product == Product::Collie {
        return Err(RecipeInvalidDetails::new(
            field,
            "a `CrawlDeclaration` is a crawl, which only Foxhound runs",
            "set `product = \"foxhound\"`, or give a program or a step list for Collie",
        ));
    }
    match (entry, &manifest.runtime) {
        (RecipeEntry::Program(_), None) => Err(RecipeInvalidDetails::new(
            "runtime",
            "a program entry names no runtime to run on",
            format!(
                "add `[runtime]` with a `language`, one of {}, and its `version`",
                listed(&RuntimeLanguage::ALL.map(RuntimeLanguage::as_str))
            ),
        )),
        (RecipeEntry::Program(_), Some(runtime))
            if !runtime
                .language
                .versions()
                .contains(&runtime.version.as_str()) =>
        {
            Err(RecipeInvalidDetails::new(
                "runtime.version",
                format!(
                    "`{}` is no {} version this Product version runs",
                    runtime.version,
                    runtime.language.as_str()
                ),
                format!("give one of {}", listed(runtime.language.versions())),
            ))
        }
        (RecipeEntry::Steps(_) | RecipeEntry::Crawl(_), Some(_)) => Err(RecipeInvalidDetails::new(
            "runtime",
            format!(
                "a `{}` entry is data, run with no program, so it names no runtime",
                entry.key()
            ),
            "remove `[runtime]`",
        )),
        _ => Ok(()),
    }
}

/// Whether `name` is a Recipe name ([`RECIPE_NAME_PATTERN`]).
pub fn is_recipe_name(name: &str) -> bool {
    let mut characters = name.chars();
    name.len() <= 64
        && characters
            .next()
            .is_some_and(|first| first.is_ascii_lowercase())
        && characters.all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || matches!(character, '-' | '_')
        })
}

/// Checks that `path` names a file inside a package: relative, its parts
/// joined by `/`, none empty, `.` or `..`.
///
/// # Errors
///
/// What is wrong with it.
pub fn package_path(path: &str) -> Result<(), String> {
    if path.is_empty() {
        return Err("it names no file".to_owned());
    }
    if path.starts_with('/') || path.contains('\\') || path.contains(':') {
        return Err(format!(
            "`{path}` is no relative path with `/` between its parts"
        ));
    }
    if path
        .split('/')
        .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(format!(
            "`{path}` has an empty, `.` or `..` part, so it may lead out of the package"
        ));
    }
    Ok(())
}

fn not_a_table() -> RecipeInvalidDetails {
    RecipeInvalidDetails::new(
        RECIPE_MANIFEST_FILE,
        "it is not a table",
        "write the manifest's fields at the file's top level",
    )
}

fn version(found: Option<&Value>) -> Result<ProtocolVersion, RecipeInvalidDetails> {
    let fix = format!("write `version = \"{RECIPE_MANIFEST_VERSION}\"`");
    let Some(found) = found else {
        return Err(RecipeInvalidDetails::new(
            "version",
            "the manifest names no `version`",
            fix,
        ));
    };
    found
        .as_str()
        .and_then(|text| text.parse().ok())
        .ok_or_else(|| {
            RecipeInvalidDetails::new(
                "version",
                format!("{found} is no `major.minor` version"),
                fix,
            )
        })
}

fn entry(found: Option<Value>) -> Result<RecipeEntry, RecipeInvalidDetails> {
    const KEYS: [&str; 3] = ["program", "steps", "crawl"];
    let fix = || {
        format!(
            "give `[entry]` exactly one of {}: a program, a step list or a `CrawlDeclaration`",
            listed(&KEYS)
        )
    };
    let Some(found) = found else {
        return Err(missing("entry"));
    };
    let Value::Object(table) = found else {
        return Err(RecipeInvalidDetails::new(
            "entry",
            "`entry` is not a table",
            fix(),
        ));
    };
    if let Some(unknown) = table.keys().find(|key| !KEYS.contains(&key.as_str())) {
        return Err(RecipeInvalidDetails::new(
            format!("entry.{unknown}"),
            format!("`{unknown}` is no kind of entry"),
            fix(),
        ));
    }
    if table.len() != 1 {
        let reason = if table.is_empty() {
            "`entry` names no entry".to_owned()
        } else {
            format!(
                "`entry` names {}; a Recipe has one entry",
                listed(&table.keys().map(String::as_str).collect::<Vec<_>>())
            )
        };
        return Err(RecipeInvalidDetails::new("entry", reason, fix()));
    }
    let (key, path) = table
        .into_iter()
        .next()
        .expect("invariant: the table holds exactly one entry, checked above");
    let Value::String(path) = path else {
        return Err(RecipeInvalidDetails::new(
            format!("entry.{key}"),
            format!("`{key}` is not a path"),
            "give the file's path from the package's root as a string",
        ));
    };
    Ok(match key.as_str() {
        "program" => RecipeEntry::Program(path),
        "steps" => RecipeEntry::Steps(path),
        _ => RecipeEntry::Crawl(path),
    })
}

/// The field `name` taken out of `top` as a `T`, or none when it is absent.
fn field<T: DeserializeOwned>(
    top: &mut Map<String, Value>,
    name: &str,
) -> Result<Option<T>, RecipeInvalidDetails> {
    top.remove(name)
        .map(|value| {
            serde_json::from_value(value).map_err(|error| {
                RecipeInvalidDetails::new(
                    name,
                    format!("`{name}` does not read: {error}"),
                    format!("write `{name}` as the manifest's schema describes it"),
                )
            })
        })
        .transpose()
}

fn missing(name: &str) -> RecipeInvalidDetails {
    RecipeInvalidDetails::new(
        name,
        format!("the manifest names no `{name}`, which every Recipe needs"),
        format!("add `{name}`"),
    )
}

fn listed(items: &[&str]) -> String {
    items
        .iter()
        .map(|item| format!("`{item}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Every entry this module defines.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[],
    events: &[],
    handler_requests: &[],
    error_codes: &[RECIPE_INVALID],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[],
    hook_stages: &[],
    shared_type_values: &[],
};

#[cfg(test)]
mod tests;
