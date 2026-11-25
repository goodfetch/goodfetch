//! The Extractor, a Plugin kind (ADRs 0007, 0033 and 0129): an
//! implementation answers `extract.content` with a page's main content,
//! `{ title, text, markdown, metadata }`, in place of the main-content step
//! `content.extractor` names; and the Error codes of an Extractor that
//! answered what the engine cannot take, or that no one answers.
//!
//! The engine sends the params and ignores none of the result's fields: a
//! result that holds a field it does not know is a shape it refuses, and
//! fails the call with `extract_handler_failed` (ADRs 0077 and 0127).

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::catalog::{
    Entries, ErrorCodeDefinition, ErrorGroup, PluginKind, PluginMethod, ProtocolVersion, Stability,
    schema_of,
};
use crate::page::CONTENT_EXTRACTOR;

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// The Extractor's method, its wire name with its kind's.
pub const EXTRACT_CONTENT: &str = "extract.content";

/// What `extract.content` reads a page's main content from: the document
/// the page holds, after the Clean-up rules, and the URL it came from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ExtractContentParams {
    /// The page's URL, which its relative links resolve against.
    pub url: String,
    /// The page's document, serialized as HTML.
    pub html: String,
}

/// A page's main content, as an Extractor read it. Everything in it is
/// page content (ADR 0014): it reaches a model only marked as such.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExtractContentResult {
    /// The main content's title, when the Extractor found one.
    pub title: Option<String>,
    /// The main content as plain text.
    pub text: String,
    /// The main content as Markdown.
    pub markdown: String,
    /// What the Extractor read about the page, such as its author or its
    /// date, each a text under its `snake_case` name.
    pub metadata: BTreeMap<String, String>,
}

/// What `extract_handler_failed` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ExtractHandlerFailedDetails {
    /// The Extractor, as `content.extractor` names it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub extractor: String,
    /// The URL of the page it was extracting.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub url: String,
}

/// An Extractor whose answer the engine cannot take (ADRs 0104 and 0127).
pub const EXTRACT_HANDLER_FAILED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "extract_handler_failed",
    description: "The Extractor `content.extractor` names answered `extract.content` with what the engine cannot take: an answer it cannot read, a result whose shape is not `{ title, text, markdown, metadata }`, or an Error code the catalog does not know. Its text is logged, never returned. The details name the Extractor and the page's URL. Fix the Extractor, or set `content.extractor` to a built-in one.",
    group: ErrorGroup::Program,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// An Extractor that no one answers for (ADR 0104).
pub const EXTRACTOR_UNAVAILABLE: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "extractor_unavailable",
    description: "The Extractor `content.extractor` names is a handler the program registered, which left its call unanswered: the call was dropped, or the stream that reads its calls closed. Retry once the program reads its Extractor's calls again.",
    group: ErrorGroup::Unavailable,
    retryable: true,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// The Extractor kind.
pub const EXTRACTOR: PluginKind = PluginKind {
    name: "extract",
    description: "Reads a page's main content in place of the main-content step, on the page Collie is on.",
    methods: &[PluginMethod {
        name: "content",
        description: "Reads the main content of a page's document.",
        params: schema_of::<ExtractContentParams>,
        result: schema_of::<ExtractContentResult>,
    }],
    chosen_by: Some(&CONTENT_EXTRACTOR),
    stability: Stability::Experimental,
    arrived_in: ARRIVED_IN,
};

/// The Extractor kind and its Error codes.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[],
    events: &[],
    handler_requests: &[],
    error_codes: &[EXTRACT_HANDLER_FAILED, EXTRACTOR_UNAVAILABLE],
    plugin_kinds: &[EXTRACTOR],
    metrics: &[],
    settings: &[],
    hook_stages: &[],
    shared_type_values: &[],
};
