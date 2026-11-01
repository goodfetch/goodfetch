//! The Journey catalog's files (ADR 0063): `journeys/J-NNN.toml`, one
//! Journey each, and `journeys/catalog.toml`, which holds the next free
//! number. A Journey names its Product, Collie when Collie carries every
//! step and Foxhound otherwise, and its surfaces are that Product's (ADR
//! 0094).
//!
//! Both types refuse unknown fields. `xtask codegen` writes the Journey's
//! JSON Schema to `journeys/journey.schema.json`, which editors and the
//! TypeScript and Python helpers validate against, and the bundle's `files`
//! maps both files to their schemas. `xtask journeys check` parses every
//! file with these types and holds the catalog to ADR 0063's rules; the
//! result document's reader compares a `journey` document with its Journey.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fmt;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};

use crate::catalog::Product;
use crate::releases::Sha256;

/// One Journey: a goal a person reaches through its Product's public
/// surfaces, written as steps from a starting state to an outcome checked
/// from outside the engine (ADR 0063).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Journey {
    /// `J-` and three to nine digits, equal to the file's name.
    pub id: JourneyId,
    /// A short name in the glossary's terms.
    pub title: String,
    /// What the person achieves and what MUST be true at the end, in RFC
    /// 2119 words, readable as a specification without the code. It never
    /// changes under the Journey's ID.
    pub goal: String,
    /// The number of the ADR 0037 area whose spec owns the Journey.
    pub area: u8,
    /// The Product whose executable and SDK packages the Journey drives:
    /// Collie when Collie carries every step, Foxhound otherwise (ADR 0094).
    pub product: Product,
    /// Whether the Journey is still run.
    pub status: Status,
    /// The Journeys that hold a retired Journey's goal now, possibly none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub replaced_by: Vec<JourneyId>,
    /// Why a retired Journey was retired.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// The Feature IDs the Journey exercises, each `<kind>:<id>`, such as
    /// `row:CO-18` for a row of ADR 0037's matrix or `story:3-18` for a user
    /// story.
    pub covers: Vec<FeatureId>,
    /// The Proving ground data the Journey starts from; absent when its goal
    /// touches no page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proving_ground: Option<ProvingGround>,
    /// Each surface the goal is reachable on, by its name in ADR 0063, with
    /// the milestone by which that surface must prove it.
    pub surfaces: BTreeMap<String, Milestone>,
    /// The steps, in order, in declarative words.
    pub steps: Vec<Step>,
    /// The checks of the outcome.
    pub expect: Vec<Check>,
}

/// Whether a Journey is still run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// Run on every surface it lists.
    Active,
    /// Kept under its ID, never run; its file names `replaced_by` and a
    /// `reason`.
    Retired,
}

/// The Proving ground data a Journey starts from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProvingGround {
    /// The seed every variant starts its Proving ground with, never
    /// overridden by `GOODFETCH_TEST_SEED`.
    pub seed: u64,
    /// Each Proving ground task the Journey names, pinned by its digest.
    #[serde(default)]
    pub tasks: Vec<PinnedTask>,
}

/// A Proving ground task a Journey names.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PinnedTask {
    /// The task's ID in the task list of the Journey's seed.
    pub id: TaskId,
    /// The digest the task list gives the task, which ignores the port.
    pub digest: Sha256,
}

/// One step of a Journey.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Step {
    /// What the person does, in declarative words that survive a change of
    /// implementation.
    pub text: String,
    /// The one surface the step runs on in every variant, when the goal
    /// needs that surface.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on: Option<String>,
    /// Why the step needs a person; a variant records it as `manual`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manual: Option<String>,
}

/// One check of a Journey's outcome.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Check {
    /// What must be true.
    pub text: String,
    /// A Proving ground task the oracle must answer `verified` for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<TaskId>,
    /// The surfaces of the variants the check belongs to, when it belongs
    /// to some only, such as an SDK package's install checks; every variant
    /// when empty. A `foxhound-cli` variant takes the `cli` variant's.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub surfaces: Vec<String>,
}

impl Check {
    /// Whether the check belongs to the variant on `surface`.
    pub fn belongs_to(&self, surface: &str) -> bool {
        self.surfaces.is_empty()
            || self
                .surfaces
                .iter()
                .any(|listed| listed == surface || (surface == "foxhound-cli" && listed == "cli"))
    }
}

/// `journeys/catalog.toml`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct JourneyCatalog {
    /// The next free Journey number: a new Journey takes it and increments
    /// it in the same pull request, and every ID is below it.
    pub next: u64,
}

/// A milestone of ADR 0058, `M1` to `M4`.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
pub enum Milestone {
    /// The first milestone.
    M1,
    /// The second milestone.
    M2,
    /// The third milestone.
    M3,
    /// The fourth milestone.
    M4,
}

/// A surface of ADR 0063, on which a Journey's variant runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Surface {
    /// The command line of the Product's executable.
    Cli,
    /// The Console.
    Console,
    /// The Rust SDK.
    SdkRust,
    /// The TypeScript SDK.
    SdkTypescript,
    /// The Python SDK.
    SdkPython,
    /// The MCP server over stdio.
    McpStdio,
    /// The MCP server over HTTP.
    McpHttp,
    /// The agent skills.
    Skills,
    /// The HTTP listener.
    HttpListener,
    /// A framework adapter, by its name.
    Adapter(String),
    /// An Integration, by its name.
    Integration(String),
}

impl Surface {
    /// The surface a name of ADR 0063 stands for, such as `cli` or
    /// `adapter-langchain`; none for any other name.
    pub fn from_name(name: &str) -> Option<Self> {
        let named = |prefix: &str| {
            name.strip_prefix(prefix)
                .filter(|rest| lowercase_words(rest))
                .map(str::to_owned)
        };
        Some(match name {
            "cli" => Self::Cli,
            "console" => Self::Console,
            "sdk-rust" => Self::SdkRust,
            "sdk-typescript" => Self::SdkTypescript,
            "sdk-python" => Self::SdkPython,
            "mcp-stdio" => Self::McpStdio,
            "mcp-http" => Self::McpHttp,
            "skills" => Self::Skills,
            "http-listener" => Self::HttpListener,
            _ => {
                if let Some(adapter) = named("adapter-") {
                    Self::Adapter(adapter)
                } else {
                    Self::Integration(named("integration-")?)
                }
            }
        })
    }

    /// The first milestone that ships the surface (ADR 0058); no variant on
    /// it is due earlier.
    pub fn first_milestone(&self) -> Milestone {
        match self {
            Self::Cli | Self::Console => Milestone::M1,
            Self::SdkRust
            | Self::SdkTypescript
            | Self::SdkPython
            | Self::McpStdio
            | Self::McpHttp
            | Self::Skills => Milestone::M2,
            Self::HttpListener | Self::Adapter(_) | Self::Integration(_) => Milestone::M4,
        }
    }
}

/// Lowercase words of letters and digits joined by hyphens.
fn lowercase_words(text: &str) -> bool {
    text.split('-').all(|word| {
        !word.is_empty()
            && word
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
    })
}

/// Why a value of a Journey file was refused.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
#[error("`{value}` is not {what}")]
#[non_exhaustive]
pub struct Malformed {
    what: &'static str,
    value: String,
}

fn malformed(what: &'static str, value: &str) -> Malformed {
    Malformed {
        what,
        value: value.to_owned(),
    }
}

/// The most digits a Journey number has, so it fits a `u64` and the
/// schema's pattern states the same bound.
const ID_DIGITS: std::ops::RangeInclusive<usize> = 3..=9;

/// A Journey's ID: `J-` and three to nine digits, assigned from
/// `journeys/catalog.toml` and never reused.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct JourneyId(String);

impl JourneyId {
    /// The ID as written.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Its number, leading zeros aside: `J-001` and `J-0001` are both 1.
    pub fn number(&self) -> u64 {
        self.0[2..]
            .parse()
            .expect("invariant: a JourneyId holds three to nine digits")
    }
}

impl TryFrom<String> for JourneyId {
    type Error = Malformed;

    fn try_from(text: String) -> Result<Self, Malformed> {
        let valid = text.strip_prefix("J-").is_some_and(|digits| {
            ID_DIGITS.contains(&digits.len()) && digits.bytes().all(|byte| byte.is_ascii_digit())
        });
        if valid {
            Ok(Self(text))
        } else {
            Err(malformed(
                "a Journey ID, `J-` and three to nine digits",
                &text,
            ))
        }
    }
}

impl From<JourneyId> for String {
    fn from(id: JourneyId) -> Self {
        id.0
    }
}

impl fmt::Display for JourneyId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl JsonSchema for JourneyId {
    fn schema_name() -> Cow<'static, str> {
        "JourneyId".into()
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "A Journey's ID: `J-` and three to nine digits (ADR 0063).",
            "type": "string",
            "pattern": "^J-[0-9]{3,9}$",
        })
    }
}

/// A Proving ground task's ID, `<family>/<ordinal>`, scoped by the seed
/// (ADR 0063).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct TaskId(String);

impl TaskId {
    /// The ID as written.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for TaskId {
    type Error = Malformed;

    fn try_from(text: String) -> Result<Self, Malformed> {
        let valid = text.split_once('/').is_some_and(|(family, ordinal)| {
            lowercase_words(family)
                && !ordinal.starts_with('0')
                && !ordinal.is_empty()
                && ordinal.bytes().all(|byte| byte.is_ascii_digit())
        });
        if valid {
            Ok(Self(text))
        } else {
            Err(malformed(
                "a Proving ground task ID, `<family>/<ordinal>`",
                &text,
            ))
        }
    }
}

impl From<TaskId> for String {
    fn from(id: TaskId) -> Self {
        id.0
    }
}

impl fmt::Display for TaskId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl JsonSchema for TaskId {
    fn schema_name() -> Cow<'static, str> {
        "TaskId".into()
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "A Proving ground task's ID, `<family>/<ordinal>` (ADR 0063).",
            "type": "string",
            "pattern": "^[a-z0-9]+(-[a-z0-9]+)*/[1-9][0-9]*$",
        })
    }
}

/// A Feature's ID, `<kind>:<id>`, such as `row:CO-18` or `story:3-18`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct FeatureId(String);

impl FeatureId {
    /// The ID as written.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for FeatureId {
    type Error = Malformed;

    fn try_from(text: String) -> Result<Self, Malformed> {
        let valid = text.split_once(':').is_some_and(|(kind, id)| {
            lowercase_words(kind)
                && !id.is_empty()
                && !id.contains(|character: char| character == ':' || character.is_whitespace())
        });
        if valid {
            Ok(Self(text))
        } else {
            Err(malformed("a Feature ID, `<kind>:<id>`", &text))
        }
    }
}

impl From<FeatureId> for String {
    fn from(id: FeatureId) -> Self {
        id.0
    }
}

impl JsonSchema for FeatureId {
    fn schema_name() -> Cow<'static, str> {
        "FeatureId".into()
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "A Feature's ID, `<kind>:<id>`, such as `row:CO-18` or `story:3-18`.",
            "type": "string",
            "pattern": r"^[a-z0-9]+(-[a-z0-9]+)*:[^:\s]+$",
        })
    }
}
