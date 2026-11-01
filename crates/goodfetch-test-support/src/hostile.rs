//! The hostile-input case tables (ADR 0067): one table per family, committed
//! as data among test support's shared fixtures, each case with its input,
//! the typed error every layer must give, and what serves it end to end,
//! a Proving ground page of its `headers` family among them, so every layer
//! refuses the same inputs.

use std::collections::HashSet;
use std::fmt;

use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// One family's table: every case, in the committed file's order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HostileTable<I> {
    /// The cases.
    pub cases: Vec<HostileCase<I>>,
}

/// One hostile-input case: the input a layer meets, the typed error it must
/// give, and what serves the input end to end.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HostileCase<I> {
    /// The case's name, unique in its table, in snake case.
    pub name: String,
    /// The hostile input, of the family's own shape.
    pub input: I,
    /// The typed error every layer gives. A case without one fails the
    /// table's parse.
    pub expected: ExpectedError,
    /// What serves the input end to end.
    pub served_by: ServedBy,
}

/// The typed error a case expects, by its Error code.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "code", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExpectedError {
    /// `header_value_invalid`: a header value Goodfetch would send holds a
    /// CR, an LF or a NUL.
    HeaderValueInvalid,
    /// `secret_domain_not_allowed`: a request carrying a Secret placeholder's
    /// value was redirected outside the binding's domains (ADR 0092).
    SecretDomainNotAllowed,
    /// `path_unsafe`: a name or path from outside input would leave the
    /// folder Goodfetch writes it in, or is no safe file name on some OS.
    PathUnsafe,
    /// `schema_ref_not_allowed`: a JSON Schema references a file, a URL or a
    /// private address.
    SchemaRefNotAllowed,
    /// `response_limit_exceeded`, naming the setting whose limit was passed.
    ResponseLimitExceeded {
        /// The setting.
        setting: LimitSetting,
    },
    /// `message_too_large`: a JSON-RPC message longer than the engine reads.
    MessageTooLarge,
    /// `message_too_deep`: a JSON-RPC message nested deeper than the engine
    /// reads.
    MessageTooDeep,
    /// `navigation_failed`, with the browser's network error where the
    /// browser path gives one (ADR 0085).
    NavigationFailed {
        /// The browser's network error, such as
        /// `net::ERR_INVALID_HTTP_RESPONSE`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        network_error: Option<String>,
    },
}

/// A setting whose limit a `response_limit_exceeded` names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum LimitSetting {
    /// The decoded bytes of a body the engine reads.
    #[serde(rename = "network.max_body_size")]
    MaxBodySize,
    /// The ratio of decoded to received bytes, once a decoded body passes
    /// 1 MiB.
    #[serde(rename = "network.max_decompression_ratio")]
    MaxDecompressionRatio,
    /// The nesting of JSON the engine parses from outside.
    #[serde(rename = "network.max_json_depth")]
    MaxJsonDepth,
}

/// What serves a case's input end to end.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum ServedBy {
    /// The Proving ground's task whose page serves it, by its ID, such as
    /// `headers/5`.
    Page(String),
    /// A page the owner builds, described, such as a decompression bomb's:
    /// until it exists the case runs in-process only.
    WaitingOnOwner(String),
    /// A page the Proving ground does not have yet, described, which a later
    /// ticket adds: until then the case runs in-process only.
    MissingPage(String),
    /// No page: the program, a Recipe, a setting or a peer hands the value to
    /// Goodfetch, so a layer's end-to-end runner passes it in itself.
    ProgramInput,
}

/// The input of a `header_values` case: a CR, an LF or a NUL in a header
/// value, from where it reaches Goodfetch.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HeaderValue {
    /// Where the value comes from.
    pub source: HeaderSource,
    /// The header's name.
    pub name: String,
    /// The header's value.
    pub value: String,
}

/// Where a header value comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HeaderSource {
    /// A Recipe's request header.
    Recipe,
    /// A Sink's configured header.
    Sink,
    /// A Notification webhook's configured header.
    Webhook,
    /// A header of a `page.request` call, which reaches the browser through
    /// CDP.
    PageRequest,
    /// A route's header override (`continue`, `fallback`, `fetch`), which
    /// reaches the browser through CDP.
    Route,
    /// A page's response head.
    Response,
}

/// The input of a `secret_redirects` case: a request carrying a Secret
/// placeholder's value, bound to `bound_to`, redirected to `location`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SecretRedirect {
    /// Where the request carries the value.
    pub placement: SecretPlacement,
    /// The binding's registrable domains.
    pub bound_to: Vec<String>,
    /// The URL the request is sent to, on a bound domain.
    pub url: String,
    /// The redirect's status.
    #[schemars(range(min = 300, max = 399))]
    pub status: u16,
    /// The redirect's `Location`, outside the binding.
    pub location: String,
}

/// Where a request carries a Secret placeholder's value (ADR 0092).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SecretPlacement {
    /// A request header.
    Header,
    /// A string of the JSON body `page.request` sends.
    JsonBody,
}

/// The input of a `paths` case: a name or path from outside input that
/// Goodfetch would write under one of its folders.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PathName {
    /// What Goodfetch would name with it.
    pub targets: Vec<PathTarget>,
    /// The name or path.
    pub name: String,
    /// The OSes whose file systems it attacks, which its case runs on.
    pub os: Schedule,
    /// What makes it hostile.
    pub hazard: PathHazard,
    /// Where the entry links to, for a link in an archive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link_to: Option<String>,
}

/// What Goodfetch names from outside input.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PathTarget {
    /// An Export's file name.
    ExportName,
    /// A Key-value store key, which names a file.
    KeyValueStoreKey,
    /// A screenshot's path.
    ScreenshotPath,
    /// A Run trace's path.
    RunTracePath,
    /// A Plan's file name.
    PlanName,
    /// An entry of a Plugin bundle being unpacked.
    PluginBundleEntry,
}

/// The OSes a case runs on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Schedule {
    /// Linux, macOS and Windows.
    Every,
    /// Linux and macOS.
    Unix,
    /// Linux.
    Linux,
    /// macOS.
    Macos,
    /// Windows.
    Windows,
}

/// An OS a case can run on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Os {
    /// Linux.
    Linux,
    /// macOS.
    Macos,
    /// Windows.
    Windows,
}

/// What makes a name or path hostile.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PathHazard {
    /// `..` components that climb out of the folder.
    ParentTraversal,
    /// A path from the root.
    Absolute,
    /// A Windows drive, absolute or drive-relative.
    Drive,
    /// A Windows verbatim path, `\\?\`.
    Verbatim,
    /// A Windows device path, `\\.\`.
    DeviceNamespace,
    /// A Windows UNC path.
    Unc,
    /// A Windows reserved device name, with or without an extension.
    ReservedName,
    /// A trailing dot or space, which Windows removes.
    TrailingDotOrSpace,
    /// An NTFS alternate data stream.
    AlternateDataStream,
    /// A character Windows forbids in a name: `<`, `>`, `:`, `"`, `|`, `?`
    /// or `*`.
    ForbiddenCharacter,
    /// A control character or a NUL.
    ControlCharacter,
    /// A decomposed name that APFS opens as the composed name it equals.
    Normalisation,
    /// A name longer than the 255 bytes a file system's name holds.
    Overlong,
    /// An empty name, `.` or `..`.
    DotName,
    /// A link in an archive that points out of it.
    ArchiveLink,
}

/// The input of a `schema_refs` case: a JSON Schema whose reference reaches
/// outside the schema.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SchemaRef {
    /// Where Goodfetch reads the schema.
    pub uses: Vec<SchemaUse>,
    /// The referencing keyword.
    pub keyword: RefKeyword,
    /// The reference.
    pub reference: String,
}

/// Where Goodfetch reads a JSON Schema from outside.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SchemaUse {
    /// An extraction schema.
    ExtractionSchema,
    /// The schema of a model's structured answer.
    StructuredOutput,
    /// A site tool's input schema, which the page declares.
    SiteToolInput,
    /// A Dataset's schema.
    DatasetSchema,
}

/// A JSON Schema keyword that references another schema.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum RefKeyword {
    /// `$ref`.
    #[serde(rename = "$ref")]
    Ref,
    /// `$dynamicRef`.
    #[serde(rename = "$dynamicRef")]
    DynamicRef,
}

/// The input of a `limits` case: input past one of the engine's limits.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ResourceLimit {
    /// A response body that decodes to far more than it sends.
    DecompressionBomb {
        /// Its `Content-Encoding` codings, in the order they were applied.
        content_encodings: Vec<String>,
    },
    /// A response body that decodes to more than the engine reads, within
    /// the decompression ratio when it is coded.
    OversizedBody {
        /// Its length once decoded.
        bytes: u64,
        /// Its `Content-Encoding` codings, in the order they were applied;
        /// none when it is sent uncoded.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        content_encodings: Vec<String>,
    },
    /// A response body that never ends.
    EndlessBody {
        /// How the body is framed.
        framing: BodyFraming,
    },
    /// JSON nested deeper than the engine reads.
    DeepNesting {
        /// Where the JSON arrives.
        channel: JsonChannel,
        /// Its nesting of arrays and objects.
        depth: u32,
    },
    /// A message longer than the engine reads.
    OversizedMessage {
        /// Where the message arrives.
        channel: MessageChannel,
        /// Its length.
        bytes: u64,
    },
}

/// How an endless body is framed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BodyFraming {
    /// Chunks that never end with the last chunk.
    Chunked,
    /// No length: the body ends only when the connection closes.
    CloseDelimited,
}

/// Where JSON the engine parses arrives.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum JsonChannel {
    /// A response body the engine parses as JSON, a discovered API's
    /// included.
    ResponseJson,
    /// A JSON-RPC message from a Client or a handler.
    JsonRpcMessage,
}

/// Where an oversized message arrives.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MessageChannel {
    /// A JSON-RPC message from a Client or a handler.
    JsonRpcMessage,
    /// A page's response head.
    ResponseHead,
}

/// Why a table cannot be read.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum HostileTableError {
    /// The text is not a table of the family's format, such as a case with
    /// no expected error.
    #[error("the {family} hostile-input table is malformed: {source}")]
    Malformed {
        /// The family's file name.
        family: &'static str,
        /// What the parser met.
        source: toml::de::Error,
    },
}

impl<I: DeserializeOwned> HostileTable<I> {
    /// The table of `family` written in `text`.
    ///
    /// # Errors
    ///
    /// [`HostileTableError::Malformed`] when the text is not TOML of the
    /// family's format, a case with no `expected` error among them.
    pub fn parse(family: &'static str, text: &str) -> Result<Self, HostileTableError> {
        toml::from_str(text).map_err(|source| HostileTableError::Malformed { family, source })
    }
}

impl<I> HostileTable<I> {
    /// The cases' names that appear more than once, in the table's order.
    pub fn duplicate_names(&self) -> Vec<&str> {
        let mut seen = HashSet::new();
        self.cases
            .iter()
            .map(|case| case.name.as_str())
            .filter(|name| !seen.insert(*name))
            .collect()
    }
}

impl HostileTable<PathName> {
    /// The cases that run on `os`.
    pub fn cases_on(&self, os: Os) -> impl Iterator<Item = &HostileCase<PathName>> {
        self.cases
            .iter()
            .filter(move |case| case.input.os.includes(os))
    }

    /// The cases that run on the OS the test runs on.
    pub fn cases_here(&self) -> impl Iterator<Item = &HostileCase<PathName>> {
        self.cases_on(Os::current())
    }
}

impl Schedule {
    /// Whether the schedule runs on `os`.
    pub fn includes(self, os: Os) -> bool {
        match self {
            Self::Every => true,
            Self::Unix => os != Os::Windows,
            Self::Linux => os == Os::Linux,
            Self::Macos => os == Os::Macos,
            Self::Windows => os == Os::Windows,
        }
    }
}

impl Os {
    /// The OS the test runs on.
    pub fn current() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::Macos
        } else {
            Self::Linux
        }
    }
}

impl SchemaRef {
    /// The schema a runner hands to the layer: an object whose one property
    /// is the reference.
    pub fn schema(&self) -> serde_json::Value {
        let keyword = match self.keyword {
            RefKeyword::Ref => "$ref",
            RefKeyword::DynamicRef => "$dynamicRef",
        };
        serde_json::json!({
            "type": "object",
            "properties": { "value": { keyword: self.reference } }
        })
    }
}

/// Every family's committed table, `test-support/fixtures/hostile/`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostileTables {
    /// CR, LF or NUL in header values.
    pub header_values: HostileTable<HeaderValue>,
    /// Requests carrying a domain-bound Secret, redirected across origins.
    pub secret_redirects: HostileTable<SecretRedirect>,
    /// Path traversal, with each OS's cases.
    pub paths: HostileTable<PathName>,
    /// External references in JSON Schemas.
    pub schema_refs: HostileTable<SchemaRef>,
    /// Decompression bombs, endless bodies, deep nesting and oversized
    /// messages.
    pub limits: HostileTable<ResourceLimit>,
}

/// A way the committed tables fall short.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TableGap {
    /// Two cases of one family share a name.
    DuplicateName {
        /// The family.
        family: &'static str,
        /// The name.
        case: String,
    },
    /// A case names a page the Proving ground has no task for.
    UnknownPage {
        /// The family.
        family: &'static str,
        /// The case.
        case: String,
        /// The task ID it names.
        page: String,
    },
}

/// Every gap of the tables, family by family.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableGaps {
    /// The gaps.
    pub gaps: Vec<TableGap>,
}

impl HostileTables {
    /// The committed tables, which every layer's runner applies.
    ///
    /// # Errors
    ///
    /// [`HostileTableError::Malformed`] naming the first family whose file
    /// is, which the Quick suite catches.
    pub fn committed() -> Result<Self, HostileTableError> {
        Ok(Self {
            header_values: HostileTable::parse(HEADER_VALUES, COMMITTED_HEADER_VALUES)?,
            secret_redirects: HostileTable::parse(SECRET_REDIRECTS, COMMITTED_SECRET_REDIRECTS)?,
            paths: HostileTable::parse(PATHS, COMMITTED_PATHS)?,
            schema_refs: HostileTable::parse(SCHEMA_REFS, COMMITTED_SCHEMA_REFS)?,
            limits: HostileTable::parse(LIMITS, COMMITTED_LIMITS)?,
        })
    }

    /// Each family's name and what serves each of its cases, by case name.
    fn served(&self) -> [(&'static str, Vec<(&str, &ServedBy)>); 5] {
        fn of<I>(table: &HostileTable<I>) -> Vec<(&str, &ServedBy)> {
            table
                .cases
                .iter()
                .map(|case| (case.name.as_str(), &case.served_by))
                .collect()
        }
        [
            (HEADER_VALUES, of(&self.header_values)),
            (SECRET_REDIRECTS, of(&self.secret_redirects)),
            (PATHS, of(&self.paths)),
            (SCHEMA_REFS, of(&self.schema_refs)),
            (LIMITS, of(&self.limits)),
        ]
    }

    /// Every case that names a Proving ground page: its family, its name
    /// and the task ID, family by family in the tables' order.
    pub fn pages(&self) -> Vec<(&'static str, &str, &str)> {
        self.served()
            .into_iter()
            .flat_map(|(family, cases)| {
                cases
                    .into_iter()
                    .filter_map(move |(case, served_by)| match served_by {
                        ServedBy::Page(task) => Some((family, case, task.as_str())),
                        ServedBy::WaitingOnOwner(_)
                        | ServedBy::MissingPage(_)
                        | ServedBy::ProgramInput => None,
                    })
            })
            .collect()
    }

    /// Checks that each family's names are unique and that every page a
    /// case names is one of `tasks`, the Proving ground's task IDs.
    ///
    /// # Errors
    ///
    /// [`TableGaps`] listing every gap.
    pub fn check(&self, tasks: &[&str]) -> Result<(), TableGaps> {
        let mut gaps = Vec::new();
        for (family, cases) in self.served() {
            let mut seen = HashSet::new();
            for (case, served_by) in cases {
                if !seen.insert(case) {
                    gaps.push(TableGap::DuplicateName {
                        family,
                        case: case.to_owned(),
                    });
                }
                if let ServedBy::Page(page) = served_by
                    && !tasks.contains(&page.as_str())
                {
                    gaps.push(TableGap::UnknownPage {
                        family,
                        case: case.to_owned(),
                        page: page.clone(),
                    });
                }
            }
        }
        if gaps.is_empty() {
            Ok(())
        } else {
            Err(TableGaps { gaps })
        }
    }
}

/// The families' file names, without `.toml`.
const HEADER_VALUES: &str = "header_values";
const SECRET_REDIRECTS: &str = "secret_redirects";
const PATHS: &str = "paths";
const SCHEMA_REFS: &str = "schema_refs";
const LIMITS: &str = "limits";

/// The committed tables' text.
const COMMITTED_HEADER_VALUES: &str =
    include_str!("../../../test-support/fixtures/hostile/header_values.toml");
const COMMITTED_SECRET_REDIRECTS: &str =
    include_str!("../../../test-support/fixtures/hostile/secret_redirects.toml");
const COMMITTED_PATHS: &str = include_str!("../../../test-support/fixtures/hostile/paths.toml");
const COMMITTED_SCHEMA_REFS: &str =
    include_str!("../../../test-support/fixtures/hostile/schema_refs.toml");
const COMMITTED_LIMITS: &str = include_str!("../../../test-support/fixtures/hostile/limits.toml");

impl fmt::Display for TableGap {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateName { family, case } => {
                write!(formatter, "two {family} cases are named {case}")
            }
            Self::UnknownPage { family, case, page } => write!(
                formatter,
                "{family} case {case} names {page}, which the Proving ground has no task for"
            ),
        }
    }
}

impl fmt::Display for TableGaps {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("the hostile-input tables fall short:")?;
        for gap in &self.gaps {
            write!(formatter, "\n  {gap}")?;
        }
        Ok(())
    }
}

impl std::error::Error for TableGaps {}

#[cfg(test)]
mod tests;
