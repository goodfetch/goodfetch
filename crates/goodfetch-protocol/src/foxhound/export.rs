//! A Dataset's Export (ADRs 0026, 0103, 0116 and 0119): `run.export`
//! writes one file of the Records a Dataset committed when the call is
//! made, in number order, into the Run's folder, and answers an `Export`
//! only once that file is whole, so an `Export` a program holds is always
//! whole. A failed write raises `file_write_failed`, side `engine`, and
//! leaves no unfinished file; at the `critical` storage level it fails
//! before writing. `export.save_as` puts the file at the program's path, as
//! `download.save_as` puts a Download's, and `export.stream` sends its
//! bytes. Each carries the run epoch of the Run's state it was written
//! from, in its answers, its ID and its kept file's name, so a destination
//! that compares and sets refuses a fenced copy's Export (ADR 0003).
//! Every text Export is UTF-8: JSON and JSONL carry no BOM, write
//! characters as themselves and write each Record's value as it was
//! committed. CSV, XLSX, HTML table, Markdown, Parquet, Avro and Arrow IPC
//! carry one column per field seen in any Record, in first-seen order, so a
//! later Record's field is never dropped (pitfall D07); XML, RSS and Atom
//! write one element per Record; each is shaped by the `export.*`
//! settings, and plain, gzip or zstd (CO-27). A Record read past a Wall
//! carries its marker in every format, and an Export can take only those
//! Records or only the others (ADR 0052).
//! Foxhound's own; the two Error codes of a failed write are Collie's (ADR
//! 0095).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::catalog::{
    Authority, Entries, Operation, OperationClass, OperationKind, ProtocolVersion, Reach, Setting,
    Stability, ValueKind, schema_of,
};
use crate::run::RunId;
use crate::settings::ProjectSettings;

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// What separates an Export's Run ID from its run epoch and its file's
/// SHA-256 in its ID; those two are digits and hexadecimal alone, so the
/// last one splits the ID.
const EXPORT_MARK: &str = ".export-";

/// An Export of a Run, named by the engine when its file is whole:
/// `<run_id>.export-<run_epoch>-<sha256>`, the run epoch of the Run's
/// state it was written from (ADR 0003) and the SHA-256 of its bytes in
/// lowercase hexadecimal, so two Exports of the same bytes under one run
/// epoch are one file.
#[derive(
    Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct ExportId(pub String);

impl ExportId {
    /// The ID of the Run `run_id`'s Export written under the run epoch
    /// `run_epoch`, whose bytes' SHA-256 is `sha256`.
    pub fn of(run_id: &RunId, run_epoch: u64, sha256: &str) -> Self {
        Self(format!("{run_id}{EXPORT_MARK}{run_epoch}-{sha256}"))
    }

    /// Its Run, its run epoch and its file's SHA-256, or `None` for a text
    /// no engine names an Export with.
    pub fn split(&self) -> Option<(RunId, u64, &str)> {
        let (run, named) = self.0.rsplit_once(EXPORT_MARK)?;
        let (epoch, sha256) = named.split_once('-')?;
        let run_epoch: u64 = epoch.parse().ok()?;
        let digest = sha256.len() == 64
            && sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
        if !digest || run_epoch.to_string() != epoch {
            return None;
        }
        Some((RunId::parse(run).ok()?, run_epoch, sha256))
    }
}

/// An Export's format, named, never inferred from a file name (ADR 0103).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    /// One JSON array of the Records' values, indented as
    /// `export.json.indent` says.
    Json,
    /// One Record's value per line, each line ended by a line feed.
    Jsonl,
    /// RFC 4180 CSV: a header row of the fields seen, then one row per
    /// Record, each line ended by CR LF.
    Csv,
    /// One XLSX workbook of one sheet: a header row of the fields seen,
    /// then one row per Record.
    Xlsx,
    /// One HTML document holding one table: a header row of the fields
    /// seen, then one row per Record.
    HtmlTable,
    /// One GitHub Flavored Markdown table: a header row of the fields seen,
    /// then one row per Record.
    Markdown,
    /// One XML document: a root element named by `export.xml.root` holding
    /// one element per Record named by `export.xml.record`, each field an
    /// element of its name (`field` with a `name` attribute where its name
    /// is no XML name), an object's fields as elements, an array's values
    /// as `item` elements, and `null` an empty element with
    /// `xsi:nil="true"`.
    Xml,
    /// One Apache Parquet file of one column per field seen, typed: a
    /// column whose values are all `true` or `false` is boolean, all whole
    /// numbers a 64-bit integer, all numbers a 64-bit float, and any other
    /// a string holding each cell's text, every column nullable.
    Parquet,
    /// One Apache Avro object container file of one record per Record,
    /// with one field per field seen, typed as Parquet's columns are, each
    /// field's name made a valid Avro name with the field's own in its
    /// `doc`.
    Avro,
    /// One Apache Arrow IPC file of one column per field seen, typed as
    /// Parquet's columns are.
    ArrowIpc,
    /// One RSS 2.0 feed named after the Dataset, of one item per Record,
    /// its title, link, date and summary taken from the fields the
    /// `export.feed.*` settings name.
    Rss,
    /// One Atom feed named after the Dataset, of one entry per Record, its
    /// title, link, date and summary taken from the fields the
    /// `export.feed.*` settings name.
    Atom,
}

impl ExportFormat {
    /// Whether it is a table of one column per field seen, which needs
    /// every field before its first row.
    pub fn is_table(self) -> bool {
        matches!(
            self,
            Self::Csv
                | Self::Xlsx
                | Self::HtmlTable
                | Self::Markdown
                | Self::Parquet
                | Self::Avro
                | Self::ArrowIpc
        )
    }
}

/// How an Export's bytes are compressed: the whole file, once written in
/// its format.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExportCompression {
    /// Not compressed.
    #[default]
    None,
    /// One gzip member (RFC 1952).
    Gzip,
    /// One Zstandard frame (RFC 8878).
    Zstd,
}

/// What `run.export` takes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunExportParams {
    /// The crawl's Run.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: RunId,
    /// The Dataset's name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub dataset: String,
    /// The format.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub format: ExportFormat,
    /// The compression; absent, `none`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub compression: Option<ExportCompression>,
    /// `true` writes only the Records whose page was read past a Wall,
    /// `false` only the others; absent, every Record (ADR 0052).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub wall: Option<bool>,
    /// The Export's settings, nested by key: the run layer (ADR 0083),
    /// whose `export.*` keys shape its format.
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub settings: Map<String, Value>,
    /// The project configuration the Client found, layer 3 (ADR 0083).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub project: Option<ProjectSettings>,
}

/// What `run.export` answers once the Export's file is whole (ADR 0103).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Export {
    /// The Export, which `export.save_as` and `export.stream` take.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub export_id: ExportId,
    /// Its format.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub format: ExportFormat,
    /// Its compression.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub compression: ExportCompression,
    /// How many Records it holds.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub records: u64,
    /// The number of the last Record it holds, 0 when it holds none.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub last_number: u64,
    /// Its size in bytes.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub size: u64,
    /// The SHA-256 of its bytes, in lowercase hexadecimal.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub sha256: String,
    /// The run epoch of the Run's state it was written from, which a
    /// destination that compares and sets holds it against (ADR 0003).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_epoch: u64,
}

/// `run.export`.
pub const RUN_EXPORT: Operation = Operation {
    name: "run.export",
    description: "Writes an Export of one Dataset of a crawl's Run: the Records it committed when the call is made, in number order, in the format named, as its `format` value says: a table (`csv`, `xlsx`, `html_table`, `markdown`, and typed, `parquet`, `avro` and `arrow_ipc`) has one column per field seen in any of its Records, in first-seen order, and one row per Record. Every text Export is UTF-8 with no BOM unless `export.csv.bom` puts one on CSV, characters written as themselves (a character XML 1.0 cannot hold as U+FFFD in XML and feeds) and each value as it was committed: a table cell holds a string as its text, a number, `true` or `false` as JSON writes it, nothing for `null` or a field its Record lacks, and an object or array as its JSON text, or, in CSV under `export.csv.nested: flatten`, an object's fields in dotted columns. A Record whose page was read past a Wall carries its Wall marker, `{kind, vendor?}` as its Provenance holds it, as a `#wall` field in every format but the feeds, where it is a category of the scheme `urn:goodfetch:wall` naming its kind; `wall` set to `true` writes only those Records, `false` only the others. `compression` `gzip` or `zstd` compresses the whole file. Its `settings` and `project` are the run layer and the project configuration its `export.*` settings resolve under; a value the format cannot take fails with `setting_invalid`. Answers the Export, with its Record count, its last Record's number, its size, its SHA-256 and the run epoch of the Run's state it was written from, which its ID and its kept file's name carry too, so a destination that compares and sets refuses the Export of a copy an older run epoch fenced (ADR 0003), only once its file is whole in the Run's folder, where `export.save_as` and `export.stream` read it. Records committed after the call starts are not in it. A write the engine's storage refuses fails with `file_write_failed`, side `engine`, leaving no unfinished file, as does a call while that storage is at the `critical` level, before any byte is written. A Dataset the crawl does not declare fails with `dataset_unknown`.",
    class: OperationClass::IdempotentWrite,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<RunExportParams>,
    result: schema_of::<Export>,
    events: &[],
    cancellable: true,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// What `export.save_as` takes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExportSaveAsParams {
    /// The Export.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub export_id: ExportId,
    /// The program's path, absolute, which an Attached engine writes
    /// itself; any other engine leaves it to the SDK and sends the file's
    /// bytes instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub path: Option<String>,
}

/// What `export.save_as` answers once the file is whole.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ExportSaved {
    /// The file's size in bytes.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub size: u64,
    /// The SHA-256 of its bytes, in lowercase hexadecimal.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub sha256: String,
    /// Whether the engine wrote the program's path itself; when not, the
    /// `$/chunk` notifications before this answer carried every byte, and
    /// the SDK writes the path.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub written: bool,
    /// The run epoch the Export was written under (ADR 0003).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_epoch: u64,
}

/// `export.save_as`.
pub const EXPORT_SAVE_AS: Operation = Operation {
    name: "export.save_as",
    description: "Saves an Export's file, which the engine keeps in its Run's folder, whole or not at all. An Attached engine given the program's path writes it through a hidden temporary file beside it, synced and renamed over it, creating missing folders and replacing a file there; any other engine writes nothing on its own machine and sends the file in `$/chunk` notifications of at most 64 KiB, one per credit granted with `$/credit`, for the SDK to write. Answers the file's size, its SHA-256 and the run epoch the Export was written under. A path that cannot be written raises `path_not_writable`, and full or failing storage `file_write_failed`, leaving the path as it was.",
    class: OperationClass::IdempotentWrite,
    kind: OperationKind::RoundTrip,
    reach: Reach::SdkOnly,
    params: schema_of::<ExportSaveAsParams>,
    result: schema_of::<ExportSaved>,
    events: &[],
    cancellable: true,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// What `export.stream` takes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExportStreamParams {
    /// The Export.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub export_id: ExportId,
}

/// What `export.stream` answers once it sent every byte.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ExportStreamed {
    /// The file's size in bytes.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub size: u64,
    /// The SHA-256 of its bytes, in lowercase hexadecimal.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub sha256: String,
    /// The run epoch the Export was written under (ADR 0003).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_epoch: u64,
}

/// `export.stream`.
pub const EXPORT_STREAM: Operation = Operation {
    name: "export.stream",
    description: "Sends an Export's bytes, which the engine keeps in its Run's folder, in order, in `$/chunk` notifications of at most 64 KiB, one per credit granted with `$/credit`, and answers its size, its SHA-256 and the run epoch it was written under once every byte is sent. Nothing is written on either machine: a program that writes the bytes itself meets its own language's file errors.",
    class: OperationClass::Read,
    kind: OperationKind::Sequence,
    reach: Reach::SdkOnly,
    params: schema_of::<ExportStreamParams>,
    result: schema_of::<ExportStreamed>,
    events: &[],
    cancellable: true,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `export.csv.bom` (ADR 0119).
pub const EXPORT_CSV_BOM: Setting = Setting {
    name: "export.csv.bom",
    description: "Whether a CSV Export starts with a UTF-8 byte order mark, which some spreadsheet programs need to read it as UTF-8 and which Python's `csv` module, Node's csv-parse and Unix tools read as part of the first column's name: off by default. Set for the connection or the Run.",
    kind: ValueKind::Boolean,
    default: no,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `export.csv.delimiter` (spec #582, shape).
pub const EXPORT_CSV_DELIMITER: Setting = Setting {
    name: "export.csv.delimiter",
    description: "The one character between a CSV Export's fields: `,` by default, or such as `;` or a tab. A double quote, a carriage return, a line feed, or a text of more or less than one character fails the Export with `setting_invalid`. Set for the connection or the Run.",
    kind: ValueKind::Text,
    default: comma,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// The values of `export.csv.nested`.
pub const CSV_NESTED_VALUES: &[&str] = &["json", "flatten"];

/// `export.csv.nested` (spec #582, shape).
pub const EXPORT_CSV_NESTED: Setting = Setting {
    name: "export.csv.nested",
    description: "How a CSV Export writes a field whose value is an object: `json`, the default, its JSON text in the field's column; `flatten`, each of its fields in a column of its own named by its dotted path, such as `author.name`, at any depth. An array, and an empty object, is its JSON text either way. Set for the connection or the Run.",
    kind: ValueKind::Choice(CSV_NESTED_VALUES),
    default: json_text,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `export.xlsx.sheet` (spec #582, shape).
pub const EXPORT_XLSX_SHEET: Setting = Setting {
    name: "export.xlsx.sheet",
    description: "The name of an XLSX Export's one sheet: none by default, which names it after the Dataset, each character a sheet name cannot hold (`:`, `\\`, `/`, `?`, `*`, `[` and `]`) written as `_` and cut to 31 characters. A name that is empty, longer than 31 characters, holds one of those characters, or starts or ends with `'` fails the Export with `setting_invalid`. Set for the connection or the Run.",
    kind: ValueKind::Text,
    default: none,
    clearable: true,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// The longest indentation `export.json.indent` takes.
pub const JSON_INDENT_MAX: u64 = 8;

/// `export.json.indent` (spec #582, shape).
pub const EXPORT_JSON_INDENT: Setting = Setting {
    name: "export.json.indent",
    description: "How many spaces a JSON Export indents each level by: 0, the default, writes the array and each Record's value on one line, as committed; 1 to 8 write the array one value per line and each value's members on lines of their own, indented by that many spaces per level. JSONL is one line per Record either way. A count above 8 fails the Export with `setting_invalid`. Set for the connection or the Run.",
    kind: ValueKind::Count,
    default: zero,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `export.xml.root` (spec #582, shape).
pub const EXPORT_XML_ROOT: Setting = Setting {
    name: "export.xml.root",
    description: "The name of an XML Export's root element, which holds one element per Record: `records` by default. A text that is no XML name without a colon (a letter or `_`, then letters, digits, `-`, `.` and `_`) fails the Export with `setting_invalid`. Set for the connection or the Run.",
    kind: ValueKind::Text,
    default: records,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `export.xml.record` (spec #582, shape).
pub const EXPORT_XML_RECORD: Setting = Setting {
    name: "export.xml.record",
    description: "The name of the element an XML Export writes each Record in: `record` by default. A text that is no XML name without a colon (a letter or `_`, then letters, digits, `-`, `.` and `_`) fails the Export with `setting_invalid`. Set for the connection or the Run.",
    kind: ValueKind::Text,
    default: record,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// The values of `export.parquet.compression`.
pub const PARQUET_COMPRESSION_VALUES: &[&str] = &["zstd", "snappy", "none"];

/// `export.parquet.compression` (spec #582, shape).
pub const EXPORT_PARQUET_COMPRESSION: Setting = Setting {
    name: "export.parquet.compression",
    description: "The codec a Parquet Export compresses each column's pages with: `zstd`, the default, `snappy`, or `none`. It is inside the file, which every Parquet reader decodes, apart from the Export's own `compression`. Set for the connection or the Run.",
    kind: ValueKind::Choice(PARQUET_COMPRESSION_VALUES),
    default: zstd,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `export.feed.title` (spec #582, shape).
pub const EXPORT_FEED_TITLE: Setting = Setting {
    name: "export.feed.title",
    description: "The field of each Record that an RSS item's or an Atom entry's title is taken from: `title` by default. A Record without it gets an empty title, or in RSS none where it has a summary. An empty text fails the Export with `setting_invalid`. Set for the connection or the Run.",
    kind: ValueKind::Text,
    default: title,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `export.feed.link` (spec #582, shape).
pub const EXPORT_FEED_LINK: Setting = Setting {
    name: "export.feed.link",
    description: "The field of each Record that an RSS item's or an Atom entry's link is taken from: `url` by default; it is also the entry's ID in Atom and the item's `guid` in RSS. A Record without it gets no link, and in Atom an ID from the Run, the Dataset and its number. An empty text fails the Export with `setting_invalid`. Set for the connection or the Run.",
    kind: ValueKind::Text,
    default: url,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `export.feed.published` (spec #582, shape).
pub const EXPORT_FEED_PUBLISHED: Setting = Setting {
    name: "export.feed.published",
    description: "The field of each Record that an RSS item's `pubDate` and an Atom entry's `published` and `updated` are taken from: `published` by default, read as an RFC 3339 or RFC 2822 date and time. A Record without it, or whose value is no such date, gets no date in RSS, and in Atom the time it was committed as `updated`. An empty text fails the Export with `setting_invalid`. Set for the connection or the Run.",
    kind: ValueKind::Text,
    default: published,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `export.feed.summary` (spec #582, shape).
pub const EXPORT_FEED_SUMMARY: Setting = Setting {
    name: "export.feed.summary",
    description: "The field of each Record that an RSS item's `description` and an Atom entry's `summary` are taken from, as text: `summary` by default. A Record without it gets none. An empty text fails the Export with `setting_invalid`. Set for the connection or the Run.",
    kind: ValueKind::Text,
    default: summary,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

fn zero() -> Value {
    Value::from(0)
}

fn records() -> Value {
    Value::from("records")
}

fn record() -> Value {
    Value::from("record")
}

fn zstd() -> Value {
    Value::from("zstd")
}

fn title() -> Value {
    Value::from("title")
}

fn url() -> Value {
    Value::from("url")
}

fn published() -> Value {
    Value::from("published")
}

fn summary() -> Value {
    Value::from("summary")
}

fn no() -> Value {
    Value::Bool(false)
}

fn comma() -> Value {
    Value::from(",")
}

fn json_text() -> Value {
    Value::from("json")
}

fn none() -> Value {
    Value::Null
}

/// Every entry this module defines.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[RUN_EXPORT, EXPORT_SAVE_AS, EXPORT_STREAM],
    events: &[],
    handler_requests: &[],
    error_codes: &[],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[
        EXPORT_CSV_BOM,
        EXPORT_CSV_DELIMITER,
        EXPORT_CSV_NESTED,
        EXPORT_XLSX_SHEET,
        EXPORT_JSON_INDENT,
        EXPORT_XML_ROOT,
        EXPORT_XML_RECORD,
        EXPORT_PARQUET_COMPRESSION,
        EXPORT_FEED_TITLE,
        EXPORT_FEED_LINK,
        EXPORT_FEED_PUBLISHED,
        EXPORT_FEED_SUMMARY,
    ],
    hook_stages: &[],
    shared_type_values: &[],
};

#[cfg(test)]
mod tests;
