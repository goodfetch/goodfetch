//! A crawl as data (spec #611's #1514; ADR 0103): the `CrawlDeclaration`
//! a crawl with no handler in the program starts from, its Labels each
//! following links and extracting named fields into the Datasets it
//! declares. It holds no handler and no Hook, which none of its types can
//! express. Foxhound's own.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::foxhound::crawl::{ExtractionFields, Seed};
use crate::foxhound::dataset::DatasetDeclaration;

/// A crawl declared as data: the engine handles each of its requests
/// itself, by the request's Label's `follow` and `extract`. A start naming
/// it declares no Label and no Dataset of its own, else it fails with
/// `invalid_params`, and never with `extraction` too, else it fails with
/// `setting_invalid`; `extraction` is its one-Label shorthand.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CrawlDeclaration {
    /// The requests the crawl starts from, beside the start's own `seeds`,
    /// each request seed naming a Label it declares; none by default.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub seeds: Vec<Seed>,
    /// Its Labels, in order, each with what its pages follow and extract.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub labels: Vec<DeclaredLabel>,
    /// Its Datasets, as `crawl.start`'s `datasets`, which its Labels'
    /// `extract` fills.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub datasets: Vec<DatasetDeclaration>,
    /// Its Run's settings, nested by key, beneath the start's own
    /// `settings`, which win key by key.
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub settings: serde_json::Map<String, Value>,
}

/// One Label of a [`CrawlDeclaration`]: as `crawl.start`'s `labels` declare
/// one, with what each of its pages follows and extracts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeclaredLabel {
    /// Its name, unique among the declaration's Labels.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub name: String,
    /// The JSON Schema its requests' Payloads are checked against; none
    /// takes any Payload.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub payload_schema: Option<Value>,
    /// Its settings, nested by key, as a Label declared at the start takes
    /// them.
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub settings: serde_json::Map<String, Value>,
    /// The links each of its pages enqueues, rule by rule, each as
    /// `enqueueLinks` admits them; none by default.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub follow: Vec<FollowRule>,
    /// The Records each of its pages pushes, one per entry; none by
    /// default.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub extract: Vec<DeclaredExtract>,
}

/// The links a page of a declared Label enqueues, as `enqueueLinks` takes
/// them: each `links` matches, under `label`, narrowed by `include` and
/// `exclude`, at most `limit` of them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FollowRule {
    /// The CSS selector of the elements whose `href` it follows.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub links: String,
    /// The Label of the requests it enqueues, one the declaration declares.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub label: String,
    /// URL patterns of which a link must match one when there are any.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub include: Vec<String>,
    /// URL patterns a link must match none of.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub exclude: Vec<String>,
    /// At most this many links of a page are added; none for no limit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub limit: Option<u64>,
}

/// One Record a page of a declared Label pushes into `dataset`: its named
/// `fields`, read as `extraction`'s are and written in the order declared;
/// no field name repeats across one Label's extracts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeclaredExtract {
    /// The Dataset it pushes into, one the declaration declares.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub dataset: String,
    /// Its named fields, by name, each a value field, a list or rows.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub fields: ExtractionFields,
}
