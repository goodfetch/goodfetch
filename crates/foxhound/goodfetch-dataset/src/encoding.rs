//! How an Export's bytes are written from its Records (ADRs 0103 and 0119):
//! each Record's value as its JSON text was committed, so nothing is
//! normalized or repaired and ADR 0073's Unicode record round-trips byte
//! for byte; UTF-8 with no BOM, characters written as themselves, since
//! the committed text already escapes only what JSON requires. `json` is
//! one array of the values, `[` then each value after a `,` but the first,
//! then `]`, or under `export.json.indent` each value on lines of its own;
//! `jsonl` one value per line, each ended by a line feed. `csv`, `xlsx`,
//! `html_table`, `markdown`, `parquet`, `avro` and `arrow_ipc` are tables of
//! the [`Columns`] seen in every Record before the first row is written
//! (pitfall D07); `xml`, `rss` and `atom` write one element per Record; each
//! is shaped by the [`ExportOptions`] the `export.*` settings give. A Record
//! read past a Wall is written with its marker as a `#wall` field (ADR
//! 0052), and the [`Compressor`] compresses the whole file. Pure: it writes
//! into a buffer and does no I/O.

mod avro;
mod columnar;
mod columns;
mod compression;
mod csv;
mod feed;
mod html;
mod markdown;
mod written;
mod xlsx;
mod xml;

use std::borrow::Cow;

use goodfetch_engine::settings::catalog_default;
use goodfetch_protocol::catalog::Setting;
use goodfetch_protocol::foxhound::export::{
    EXPORT_CSV_BOM, EXPORT_CSV_DELIMITER, EXPORT_CSV_NESTED, EXPORT_FEED_LINK,
    EXPORT_FEED_PUBLISHED, EXPORT_FEED_SUMMARY, EXPORT_FEED_TITLE, EXPORT_JSON_INDENT,
    EXPORT_PARQUET_COMPRESSION, EXPORT_XML_RECORD, EXPORT_XML_ROOT, ExportFormat, JSON_INDENT_MAX,
};
use serde_json::Value;

use self::avro::Avro;
use self::columnar::Columnar;
pub(crate) use self::columnar::ParquetCodec;
pub(crate) use self::columns::{Columns, Nested};
pub(crate) use self::compression::Compressor;
use self::csv::Csv;
pub(crate) use self::feed::FeedFields;
use self::feed::{Feed, Flavor};
use self::html::HtmlTable;
use self::markdown::Markdown;
use self::xlsx::Xlsx;
use self::xml::Xml;

/// The longest name a sheet may have.
const SHEET_NAME_MAX: usize = 31;

/// The characters a sheet's name cannot hold.
const NOT_IN_SHEET_NAMES: [char; 7] = [':', '\\', '/', '?', '*', '[', ']'];

/// The field a Record's Wall marker is written in.
pub(crate) const WALL_FIELD: &str = "#wall";

/// One committed Record as an Export writes it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Exported<'a> {
    /// Its number in its Dataset.
    pub number: u64,
    /// Its value's JSON text, as committed.
    pub value: &'a str,
    /// The JSON text of the Wall marker its Provenance holds; none where
    /// its page met no Wall.
    pub wall: Option<&'a str>,
    /// When it was committed, RFC 3339.
    pub committed_at: &'a str,
}

impl Exported<'_> {
    /// Its value's JSON text with its Wall marker as a `#wall` field, which
    /// replaces any of its own; as committed where it has none.
    #[must_use]
    pub(crate) fn marked(&self) -> Cow<'_, str> {
        let Some(wall) = self.wall else {
            return Cow::Borrowed(self.value);
        };
        let mut value = parsed(self.value);
        let marker: Value =
            serde_json::from_str(wall).expect("invariant: a Provenance's Wall marker is JSON");
        if let Value::Object(object) = &mut value {
            object.insert(WALL_FIELD.to_owned(), marker);
        }
        Cow::Owned(value.to_string())
    }
}

/// What the `export.*` settings make of an Export's format.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportOptions {
    /// Whether a CSV Export starts with a byte order mark,
    /// `export.csv.bom`.
    pub csv_bom: bool,
    /// The character between a CSV Export's fields,
    /// `export.csv.delimiter`.
    pub csv_delimiter: char,
    /// How a CSV Export writes an object, `export.csv.nested`.
    pub csv_nested: Nested,
    /// The name of an XLSX Export's sheet, `export.xlsx.sheet`.
    pub xlsx_sheet: String,
    /// The Dataset's name: an HTML table's and a feed's title, and an Avro
    /// schema's record.
    pub title: String,
    /// The spaces a JSON Export indents each level by, `export.json.indent`.
    pub json_indent: usize,
    /// The XML root element's name, `export.xml.root`.
    pub xml_root: String,
    /// The XML element of each Record, `export.xml.record`.
    pub xml_record: String,
    /// A Parquet file's codec, `export.parquet.compression`.
    pub parquet_codec: ParquetCodec,
    /// The fields a feed's items are taken from, `export.feed.*`.
    pub feed: FeedFields,
    /// A feed's ID, a URI each entry's ID without a link adds its Record's
    /// number to.
    pub feed_id: String,
}

impl ExportOptions {
    /// The built-in options of an Export of the Dataset `dataset`: each
    /// `export.*` setting's catalog default.
    #[must_use]
    pub fn of(dataset: &str) -> Self {
        let text = |setting: &Setting| catalog_default::<&str>(setting).to_owned();
        Self {
            csv_bom: catalog_default(&EXPORT_CSV_BOM),
            csv_delimiter: csv_delimiter(catalog_default(&EXPORT_CSV_DELIMITER))
                .expect("invariant: the catalog's default delimiter is one character"),
            csv_nested: Nested::of(catalog_default(&EXPORT_CSV_NESTED))
                .expect("invariant: the catalog's default is one of its choices"),
            xlsx_sheet: sheet_named_after(dataset),
            title: dataset.to_owned(),
            json_indent: json_indent(catalog_default(&EXPORT_JSON_INDENT))
                .expect("invariant: the catalog's default indent is within its maximum"),
            xml_root: text(&EXPORT_XML_ROOT),
            xml_record: text(&EXPORT_XML_RECORD),
            parquet_codec: ParquetCodec::of(catalog_default(&EXPORT_PARQUET_COMPRESSION))
                .expect("invariant: the catalog's default is one of its choices"),
            feed: FeedFields {
                title: text(&EXPORT_FEED_TITLE),
                link: text(&EXPORT_FEED_LINK),
                published: text(&EXPORT_FEED_PUBLISHED),
                summary: text(&EXPORT_FEED_SUMMARY),
            },
            feed_id: format!("urn:goodfetch:dataset:{}", urn_text(dataset)),
        }
    }

    /// The built-in options of an Export of the Run `run_id`'s Dataset
    /// `dataset`, whose feed's ID names both.
    #[must_use]
    pub fn of_run(run_id: &str, dataset: &str) -> Self {
        Self {
            feed_id: format!(
                "urn:goodfetch:run:{}:dataset:{}",
                urn_text(run_id),
                urn_text(dataset)
            ),
            ..Self::of(dataset)
        }
    }

    /// How a table Export in `format` writes an object: as
    /// `export.csv.nested` says in CSV, as its JSON text elsewhere.
    #[must_use]
    pub fn nested(&self, format: ExportFormat) -> Nested {
        match format {
            ExportFormat::Csv => self.csv_nested,
            _ => Nested::Json,
        }
    }
}

/// The delimiter `text` names, or why it names none: one character, but
/// not a double quote, a carriage return or a line feed.
///
/// # Errors
///
/// What is wrong with `text`.
pub(crate) fn csv_delimiter(text: &str) -> Result<char, &'static str> {
    let mut characters = text.chars();
    match (characters.next(), characters.next()) {
        (Some('"' | '\r' | '\n'), None) => {
            Err("is a double quote, a carriage return or a line feed, which a CSV field holds")
        }
        (Some(character), None) => Ok(character),
        _ => Err("is not one character"),
    }
}

/// `text` as an XML element's name, or why it cannot be one.
///
/// # Errors
///
/// What is wrong with `text`.
pub(crate) fn xml_name(text: &str) -> Result<String, &'static str> {
    if xml::is_name(text) {
        Ok(text.to_owned())
    } else {
        Err("is no XML name: a letter or `_`, then letters, digits, `-`, `.` and `_`")
    }
}

/// `count` as `export.json.indent`, or why it cannot be one.
///
/// # Errors
///
/// What is wrong with `count`.
pub(crate) fn json_indent(count: u64) -> Result<usize, String> {
    if count <= JSON_INDENT_MAX {
        Ok(usize::try_from(count).expect("invariant: at most `JSON_INDENT_MAX`"))
    } else {
        Err(format!("is more than {JSON_INDENT_MAX}"))
    }
}

/// `text` as a field a feed reads, or why it cannot be one.
///
/// # Errors
///
/// What is wrong with `text`.
pub(crate) fn feed_field(text: &str) -> Result<String, &'static str> {
    if text.is_empty() {
        Err("is empty")
    } else {
        Ok(text.to_owned())
    }
}

/// `text` with each byte outside RFC 3986's unreserved characters
/// percent-encoded, as a URN's text.
fn urn_text(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(byte));
        } else {
            out.push('%');
            out.push_str(&data_encoding::HEXUPPER.encode(&[byte]));
        }
    }
    out
}

/// `text` as a sheet's name, or why it cannot be one.
///
/// # Errors
///
/// What is wrong with `text`.
pub(crate) fn sheet_name(text: &str) -> Result<String, String> {
    if text.is_empty() {
        return Err("is empty".to_owned());
    }
    if text.chars().count() > SHEET_NAME_MAX {
        return Err(format!("is longer than {SHEET_NAME_MAX} characters"));
    }
    if text.contains(NOT_IN_SHEET_NAMES) {
        let (last, others) = NOT_IN_SHEET_NAMES
            .split_last()
            .expect("invariant: a sheet's name refuses some characters");
        let others: Vec<String> = others
            .iter()
            .map(|character| format!("`{character}`"))
            .collect();
        return Err(format!("holds one of {} and `{last}`", others.join(", ")));
    }
    if text.starts_with('\'') || text.ends_with('\'') {
        return Err("starts or ends with `'`".to_owned());
    }
    Ok(text.to_owned())
}

/// The sheet's name after the Dataset `dataset`: each character a sheet's
/// name cannot hold written `_`, cut to 31 characters.
fn sheet_named_after(dataset: &str) -> String {
    let name: String = dataset
        .chars()
        .map(|character| {
            if NOT_IN_SHEET_NAMES.contains(&character) {
                '_'
            } else {
                character
            }
        })
        .take(SHEET_NAME_MAX)
        .collect();
    let name = name.trim_matches('\'');
    if name.is_empty() {
        "Sheet1".to_owned()
    } else {
        name.to_owned()
    }
}

/// How one Export's values are written.
#[derive(Debug)]
enum Writer {
    Json(usize),
    Jsonl,
    Csv(Csv),
    Xlsx(Box<Xlsx>),
    HtmlTable(HtmlTable),
    Markdown,
    Xml(Xml),
    Columnar(Box<Columnar>),
    Avro(Avro),
    Feed(Box<Feed>),
}

/// The writer of one Export's bytes, value by value.
#[derive(Debug)]
pub(crate) struct ExportEncoder {
    writer: Writer,
    columns: Columns,
    values: u64,
}

impl ExportEncoder {
    /// The writer of an Export in `format` with `options`, which holds no
    /// value yet; a table's `columns` are those of every value it will
    /// write, each as [`Exported::marked`] gives it, which the other
    /// formats ignore.
    #[must_use]
    pub(crate) fn new(format: ExportFormat, options: &ExportOptions, columns: Columns) -> Self {
        let feed = |flavor| {
            Writer::Feed(Box::new(Feed {
                flavor,
                fields: options.feed.clone(),
                title: options.title.clone(),
                id: options.feed_id.clone(),
                updated: None,
            }))
        };
        let writer = match format {
            ExportFormat::Json => Writer::Json(options.json_indent),
            ExportFormat::Jsonl => Writer::Jsonl,
            ExportFormat::Csv => Writer::Csv(Csv {
                bom: options.csv_bom,
                delimiter: options.csv_delimiter,
            }),
            ExportFormat::Xlsx => Writer::Xlsx(Box::new(Xlsx::new(options.xlsx_sheet.clone()))),
            ExportFormat::HtmlTable => Writer::HtmlTable(HtmlTable {
                title: options.title.clone(),
            }),
            ExportFormat::Markdown => Writer::Markdown,
            ExportFormat::Xml => Writer::Xml(Xml {
                root: options.xml_root.clone(),
                record: options.xml_record.clone(),
            }),
            ExportFormat::Parquet => Writer::Columnar(Box::new(Columnar::new(
                columns.names(),
                columns.kinds(),
                Some(options.parquet_codec),
            ))),
            ExportFormat::ArrowIpc => Writer::Columnar(Box::new(Columnar::new(
                columns.names(),
                columns.kinds(),
                None,
            ))),
            ExportFormat::Avro => {
                Writer::Avro(Avro::new(&options.title, columns.names(), columns.kinds()))
            }
            ExportFormat::Rss => feed(Flavor::Rss),
            ExportFormat::Atom => feed(Flavor::Atom),
        };
        Self {
            writer,
            columns,
            values: 0,
        }
    }

    /// Writes what comes before the first value into `out`.
    pub(crate) fn begin(&mut self, out: &mut Vec<u8>) {
        let names = self.columns.names();
        match &mut self.writer {
            Writer::Json(_) => out.push(b'['),
            Writer::Jsonl => {}
            Writer::Csv(csv) => csv.begin(names, out),
            Writer::Xlsx(xlsx) => xlsx.begin(names, out),
            Writer::HtmlTable(html) => html.begin(names, out),
            Writer::Markdown => Markdown::begin(names, out),
            Writer::Xml(xml) => xml.begin(out),
            Writer::Columnar(columnar) => columnar.begin(out),
            Writer::Avro(avro) => avro.begin(out),
            Writer::Feed(feed) => feed.begin(out),
        }
    }

    /// Writes the next Record into `out`.
    pub(crate) fn value(&mut self, record: &Exported<'_>, out: &mut Vec<u8>) {
        let marked = record.marked();
        let json = marked.as_ref();
        match &mut self.writer {
            Writer::Json(0) => {
                if self.values > 0 {
                    out.push(b',');
                }
                out.extend_from_slice(json.as_bytes());
            }
            Writer::Json(indent) => indented(json, *indent, self.values == 0, out),
            Writer::Jsonl => {
                out.extend_from_slice(json.as_bytes());
                out.push(b'\n');
            }
            Writer::Csv(csv) => csv.row(&self.columns.row(json), out),
            Writer::Xlsx(xlsx) => xlsx.row(&self.columns.row(json), out),
            Writer::HtmlTable(_) => HtmlTable::row(&self.columns.row(json), out),
            Writer::Markdown => Markdown::row(&self.columns.row(json), out),
            Writer::Columnar(columnar) => columnar.row(self.columns.row(json), out),
            Writer::Avro(avro) => avro.row(&self.columns.row(json), out),
            Writer::Xml(xml) => xml.value(&parsed(json), out),
            Writer::Feed(feed) => {
                feed.value(record.number, &parsed(json), record.committed_at, out);
            }
        }
        self.values += 1;
    }

    /// Writes what comes after the last value into `out`.
    pub(crate) fn end(&mut self, out: &mut Vec<u8>) {
        match &mut self.writer {
            Writer::Json(indent) => {
                if *indent > 0 && self.values > 0 {
                    out.push(b'\n');
                }
                out.push(b']');
            }
            Writer::Jsonl | Writer::Csv(_) | Writer::Markdown => {}
            Writer::Xlsx(xlsx) => xlsx.end(out),
            Writer::HtmlTable(_) => HtmlTable::end(out),
            Writer::Xml(xml) => xml.end(out),
            Writer::Columnar(columnar) => columnar.end(out),
            Writer::Avro(avro) => avro.end(out),
            Writer::Feed(feed) => feed.end(out),
        }
    }

    /// How many values it wrote.
    #[must_use]
    pub(crate) fn values(&self) -> u64 {
        self.values
    }
}

/// Writes `json`, one value of a JSON array indented by `indent` spaces a
/// level, on lines of its own after the array's `[` or the previous
/// value's `,`.
fn indented(json: &str, indent: usize, first: bool, out: &mut Vec<u8>) {
    let spaces = " ".repeat(indent);
    let mut text = Vec::new();
    let mut serializer = serde_json::Serializer::with_formatter(
        &mut text,
        serde_json::ser::PrettyFormatter::with_indent(spaces.as_bytes()),
    );
    serde::Serialize::serialize(&parsed(json), &mut serializer)
        .expect("invariant: a JSON value serializes into memory");
    if !first {
        out.push(b',');
    }
    for line in text.split(|byte| *byte == b'\n') {
        out.push(b'\n');
        out.extend_from_slice(spaces.as_bytes());
        out.extend_from_slice(line);
    }
}

/// The value of a Record's JSON text, which the Dataset checked was JSON
/// when it was pushed.
fn parsed(json: &str) -> Value {
    serde_json::from_str(json).expect("invariant: a committed Record's text is JSON")
}

#[cfg(test)]
mod tests;
