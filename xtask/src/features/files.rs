//! The files of `features/` as Rust types (ADR 0072): the generated
//! inventory, and the stories, rows, areas and exceptions files people write.
//! Each refuses an unknown field, and `xtask codegen` writes its JSON Schema
//! beside it, so an editor and the Vitest and Playwright configurations
//! validate what they read.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::fmt;
use std::path::Path;

use anyhow::{Context as _, Result};
use goodfetch_protocol::catalog::Product;
use schemars::JsonSchema;
use schemars::generate::SchemaSettings;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The generated inventory.
pub(crate) const INVENTORY: &str = "features/inventory.json";
/// Each area's stories, `<area number>.toml`.
pub(crate) const STORIES: &str = "features/stories";
/// Each matrix row's milestone.
pub(crate) const ROWS: &str = "features/rows.toml";
/// The owning area of each generated Feature.
pub(crate) const AREAS: &str = "features/areas.toml";
/// The Features no pinned test can prove.
pub(crate) const EXCEPTIONS: &str = "features/exceptions.toml";
/// ADR 0037's matrix, whose rows are the `row:` Features.
pub(crate) const MATRIX: &str = "docs/adr/0037-v1-scope-matrix.md";

/// A Feature's kind, the part of its ID before the first `:`. Each later
/// generator adds its kind here (ADR 0072's table).
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Kind {
    /// An ADR 0037 matrix row that is not `excluded`.
    Row,
    /// A spec's user story, `story:<area>-<n>`.
    Story,
    /// An operation of the Operation catalog.
    Operation,
    /// An Error code.
    Error,
    /// A catalog event.
    Event,
    /// A setting of the settings schema.
    Setting,
    /// An exit code of the exit-code table.
    Exit,
    /// A versioned format of the format registry.
    Format,
    /// A CLI command path of a Product's executable, from clap's `Command`.
    Command,
    /// A Block cause.
    BlockCause,
}

impl Kind {
    /// Whether people write the Features of this kind, rather than a
    /// generator reading them from a contract's definition.
    pub(crate) fn is_declared(self) -> bool {
        matches!(self, Self::Row | Self::Story)
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Row => "row",
            Self::Story => "story",
            Self::Operation => "operation",
            Self::Error => "error",
            Self::Event => "event",
            Self::Setting => "setting",
            Self::Exit => "exit",
            Self::Format => "format",
            Self::Command => "command",
            Self::BlockCause => "block-cause",
        }
    }
}

impl fmt::Display for Kind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Whether a Feature is still promised.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Status {
    /// Promised.
    Active,
    /// A story removed from its spec, kept so its number is never reused.
    Retired,
}

/// `features/inventory.json`: every Feature, sorted by ID.
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct Inventory {
    /// Every Feature.
    pub(crate) features: Vec<Feature>,
}

/// One behaviour Goodfetch promises.
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct Feature {
    /// `<kind>:<id>`, as tests declare it.
    pub(crate) id: String,
    /// Its kind.
    pub(crate) kind: Kind,
    /// Its owning area's number: a story's file, a row's first spec area, a
    /// generated Feature's entry of `features/areas.toml`; none when no entry
    /// or several match it.
    pub(crate) area: Option<u32>,
    /// The milestone of the ticket that finishes it, for a declared Feature;
    /// a generated one is due in the pull request that adds it.
    pub(crate) milestone: Option<String>,
    /// Whether it is still promised.
    pub(crate) status: Status,
    /// The Products that carry it (ADR 0094): Foxhound alone for a name of
    /// ADR 0087's generated table of Foxhound's names, both otherwise.
    pub(crate) products: Vec<Product>,
}

/// `features/stories/<area number>.toml`: one area's user stories.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoryFile {
    /// Its stories, each under its spec's own number.
    #[serde(default)]
    pub(crate) story: Vec<Story>,
}

/// One user story of a spec.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct Story {
    /// The spec's own number for it, never reused.
    pub(crate) id: u32,
    /// The story's sentence.
    #[expect(
        dead_code,
        reason = "part of the file's schema; the check reads the flags, never the sentence"
    )]
    pub(crate) title: String,
    /// The GitHub milestone of the ticket that finishes it.
    pub(crate) milestone: String,
    /// Whether it needs a Journey (ADR 0075).
    pub(crate) journey: bool,
    /// The first of ADR 0075's three tests it fails; required exactly when
    /// `journey` is false.
    pub(crate) not_journey: Option<NotJourney>,
    /// Whether it is still promised.
    pub(crate) status: Status,
    /// Why it was retired; required exactly when it is.
    pub(crate) reason: Option<String>,
    /// The one Product that carries it, for a story of Foxhound's part;
    /// both Products, since Foxhound contains Collie, when absent (ADR
    /// 0094).
    #[serde(default)]
    pub(crate) product: Option<OnlyProduct>,
}

/// The one Product that carries a story or a matrix row: only Foxhound can
/// be, since Foxhound contains Collie (ADR 0084).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum OnlyProduct {
    Foxhound,
}

/// ADR 0075's three tests, by the name of the one a story fails.
#[derive(Clone, Copy, Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub(crate) enum NotJourney {
    /// Its actor is no person using a shipped surface.
    Actor,
    /// It states only part of a goal.
    Part,
    /// No Journey variant can reach its outcome.
    Reach,
}

/// `features/rows.toml`: each matrix row's milestone.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct RowsFile {
    /// The rows given a milestone so far.
    #[serde(default)]
    pub(crate) row: Vec<Row>,
}

/// One matrix row's milestone.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct Row {
    /// The row's ID in the matrix, such as `RT-10` or `durability-resume`.
    pub(crate) id: String,
    /// The GitHub milestone of the ticket that finishes it.
    pub(crate) milestone: String,
    /// The one Product that carries it, for a row of Foxhound's alone; both
    /// Products when absent (ADR 0094).
    #[serde(default)]
    pub(crate) product: Option<OnlyProduct>,
}

/// `features/areas.toml`: the owning area of each generated Feature.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct AreasFile {
    /// Each area's share of the generated Features.
    #[serde(default)]
    pub(crate) area: Vec<Area>,
}

/// The generated Features of one kind whose IDs start with `prefix`, owned
/// by one area.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct Area {
    /// The area's number in ADR 0037's table.
    pub(crate) number: u32,
    /// A generated kind.
    pub(crate) kind: Kind,
    /// The start of the IDs it owns, after `<kind>:`.
    pub(crate) prefix: String,
}

impl AreasFile {
    /// The areas whose entries match a generated Feature.
    pub(crate) fn owners(&self, kind: Kind, name: &str) -> Vec<u32> {
        self.area
            .iter()
            .filter(|area| area.kind == kind && name.starts_with(&area.prefix))
            .map(|area| area.number)
            .collect()
    }
}

/// `features/exceptions.toml`: the Features no pinned test can prove.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExceptionsFile {
    /// Each excepted Feature.
    #[serde(default)]
    pub(crate) exception: Vec<Exception>,
}

/// One excepted Feature. The check reads its Feature; the report lists it
/// by its evidence, with its reason and link.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct Exception {
    /// Its ID.
    pub(crate) feature: String,
    /// What proves it instead.
    pub(crate) evidence: Evidence,
    /// Why no pinned test can.
    pub(crate) reason: String,
    /// Where it is checked.
    pub(crate) link: String,
}

/// The evidence of an exception (ADR 0072).
#[derive(Clone, Copy, Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Evidence {
    /// A person: an owner-test step or an ADR's manual test.
    Manual,
    /// A Live check or an Evaluation.
    Live,
    /// Repository configuration, checked by a named CI job.
    Check,
    /// Nothing in this repository reaches it.
    None,
}

impl Evidence {
    pub(crate) const ALL: [Self; 4] = [Self::Manual, Self::Live, Self::Check, Self::None];

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Live => "live",
            Self::Check => "check",
            Self::None => "none",
        }
    }
}

/// Each file's JSON Schema, by the path `xtask codegen` writes it to.
pub(crate) fn schemas() -> Vec<(&'static str, Value)> {
    fn schema<T: JsonSchema>() -> Value {
        SchemaSettings::draft2020_12()
            .into_generator()
            .into_root_schema_for::<T>()
            .to_value()
    }
    vec![
        ("features/areas.schema.json", schema::<AreasFile>()),
        (
            "features/exceptions.schema.json",
            schema::<ExceptionsFile>(),
        ),
        ("features/inventory.schema.json", schema::<Inventory>()),
        ("features/rows.schema.json", schema::<RowsFile>()),
        ("features/stories.schema.json", schema::<StoryFile>()),
    ]
}

/// Why a file could not be read as its type.
pub(crate) enum Unreadable {
    /// It is missing or not text.
    Io(anyhow::Error),
    /// It breaks its schema; the message says where.
    Schema(String),
}

/// Reads a TOML file of the workspace as `T`.
pub(crate) fn read_toml<T: DeserializeOwned>(root: &Path, relative: &str) -> Result<T, Unreadable> {
    let path = root.join(relative);
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("reading {}", path.display()))
        .map_err(Unreadable::Io)?;
    toml::from_str(&text).map_err(|error| Unreadable::Schema(error.message().to_owned()))
}

/// Reads a TOML file that may be absent, as `T`'s default then.
pub(crate) fn read_optional_toml<T: DeserializeOwned + Default>(
    root: &Path,
    relative: &str,
) -> Result<T, Unreadable> {
    if root.join(relative).is_file() {
        read_toml(root, relative)
    } else {
        Ok(T::default())
    }
}

/// Reads the inventory.
pub(crate) fn read_inventory(root: &Path) -> Result<Inventory, Unreadable> {
    let path = root.join(INVENTORY);
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("reading {}", path.display()))
        .map_err(Unreadable::Io)?;
    serde_json::from_str(&text).map_err(|error| Unreadable::Schema(error.to_string()))
}

/// Every stories file, as its area number and its path relative to the root,
/// sorted by area.
pub(crate) fn story_files(root: &Path) -> Result<Vec<(Option<u32>, String)>> {
    let folder = root.join(STORIES);
    let Ok(entries) = std::fs::read_dir(&folder) else {
        return Ok(Vec::new());
    };
    let mut files = Vec::new();
    for entry in entries {
        let entry = entry.with_context(|| format!("reading {}", folder.display()))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(stem) = name.strip_suffix(".toml") else {
            continue;
        };
        files.push((stem.parse().ok(), format!("{STORIES}/{name}")));
    }
    files.sort();
    Ok(files)
}
