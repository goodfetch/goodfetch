//! How an Export's bytes are written from its Records (ADRs 0103 and 0119):
//! each Record's value as its JSON text was committed, so nothing is
//! normalized or repaired and ADR 0073's Unicode record round-trips byte
//! for byte; UTF-8 with no BOM, characters written as themselves, since
//! the committed text already escapes only what JSON requires. `json` is
//! one array of the values, `[` then each value after a `,` but the first,
//! then `]`; `jsonl` one value per line, each ended by a line feed. `csv`,
//! `xlsx`, `html_table` and `markdown` are tables of the [`Columns`] seen in
//! every Record before the first row is written (pitfall D07), shaped by
//! the [`ExportOptions`] the `export.*` settings give. Pure: it writes into
//! a buffer and does no I/O.

mod columns;
mod csv;
mod html;
mod markdown;
mod xlsx;

use goodfetch_protocol::foxhound::export::ExportFormat;

pub use self::columns::{Columns, Nested};
use self::csv::Csv;
use self::html::HtmlTable;
use self::markdown::Markdown;
use self::xlsx::Xlsx;

/// The longest name a sheet may have.
const SHEET_NAME_MAX: usize = 31;

/// The characters a sheet's name cannot hold.
const NOT_IN_SHEET_NAMES: [char; 7] = [':', '\\', '/', '?', '*', '[', ']'];

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
    /// The Dataset's name, an HTML table's title.
    pub title: String,
}

impl ExportOptions {
    /// The built-in options of an Export of the Dataset `dataset`.
    #[must_use]
    pub fn of(dataset: &str) -> Self {
        Self {
            csv_bom: false,
            csv_delimiter: ',',
            csv_nested: Nested::Json,
            xlsx_sheet: sheet_named_after(dataset),
            title: dataset.to_owned(),
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
pub fn csv_delimiter(text: &str) -> Result<char, &'static str> {
    let mut characters = text.chars();
    match (characters.next(), characters.next()) {
        (Some('"' | '\r' | '\n'), None) => {
            Err("is a double quote, a carriage return or a line feed, which a CSV field holds")
        }
        (Some(character), None) => Ok(character),
        _ => Err("is not one character"),
    }
}

/// `text` as a sheet's name, or why it cannot be one.
///
/// # Errors
///
/// What is wrong with `text`.
pub fn sheet_name(text: &str) -> Result<String, &'static str> {
    if text.is_empty() {
        return Err("is empty");
    }
    if text.chars().count() > SHEET_NAME_MAX {
        return Err("is longer than 31 characters");
    }
    if text.contains(NOT_IN_SHEET_NAMES) {
        return Err("holds one of `:`, `\\`, `/`, `?`, `*`, `[` and `]`");
    }
    if text.starts_with('\'') || text.ends_with('\'') {
        return Err("starts or ends with `'`");
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
    Json,
    Jsonl,
    Csv(Csv),
    Xlsx(Box<Xlsx>),
    HtmlTable(HtmlTable),
    Markdown,
}

/// The writer of one Export's bytes, value by value.
#[derive(Debug)]
pub struct ExportEncoder {
    writer: Writer,
    columns: Columns,
    values: u64,
}

impl ExportEncoder {
    /// The writer of an Export in `format` with `options`, which holds no
    /// value yet; a table's `columns` are those of every value it will
    /// write, which the other formats ignore.
    #[must_use]
    pub fn new(format: ExportFormat, options: &ExportOptions, columns: Columns) -> Self {
        let writer = match format {
            ExportFormat::Json => Writer::Json,
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
        };
        Self {
            writer,
            columns,
            values: 0,
        }
    }

    /// Writes what comes before the first value into `out`.
    pub fn begin(&mut self, out: &mut Vec<u8>) {
        let names = self.columns.names();
        match &mut self.writer {
            Writer::Json => out.push(b'['),
            Writer::Jsonl => {}
            Writer::Csv(csv) => csv.begin(names, out),
            Writer::Xlsx(xlsx) => xlsx.begin(names, out),
            Writer::HtmlTable(html) => html.begin(names, out),
            Writer::Markdown => Markdown::begin(names, out),
        }
    }

    /// Writes the next value, a Record's JSON text as committed, into
    /// `out`.
    pub fn value(&mut self, json: &str, out: &mut Vec<u8>) {
        match &mut self.writer {
            Writer::Json => {
                if self.values > 0 {
                    out.push(b',');
                }
                out.extend_from_slice(json.as_bytes());
            }
            Writer::Jsonl => {
                out.extend_from_slice(json.as_bytes());
                out.push(b'\n');
            }
            Writer::Csv(csv) => csv.row(&self.columns.row(json), out),
            Writer::Xlsx(xlsx) => xlsx.row(&self.columns.row(json), out),
            Writer::HtmlTable(_) => HtmlTable::row(&self.columns.row(json), out),
            Writer::Markdown => Markdown::row(&self.columns.row(json), out),
        }
        self.values += 1;
    }

    /// Writes what comes after the last value into `out`.
    pub fn end(&mut self, out: &mut Vec<u8>) {
        match &mut self.writer {
            Writer::Json => out.push(b']'),
            Writer::Jsonl | Writer::Csv(_) | Writer::Markdown => {}
            Writer::Xlsx(xlsx) => xlsx.end(out),
            Writer::HtmlTable(_) => HtmlTable::end(out),
        }
    }

    /// How many values it wrote.
    #[must_use]
    pub fn values(&self) -> u64 {
        self.values
    }
}

#[cfg(test)]
mod tests;
