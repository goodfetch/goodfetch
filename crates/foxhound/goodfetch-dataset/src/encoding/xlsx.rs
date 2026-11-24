//! XLSX (ECMA-376 `SpreadsheetML`): one ZIP of a workbook of one sheet,
//! named by `export.xlsx.sheet` or after the Dataset, its header row then
//! one row per Record. A string, and an object's or array's JSON text, is
//! an inline string, a number a number and `true` or `false` a boolean;
//! `null` and a missing field leave no cell. A character XML 1.0 cannot
//! hold is written `_xHHHH_`, as `SpreadsheetML` escapes it, and an `_`
//! that would start such an escape as `_x005F_`. The ZIP is written as a
//! stream, its entries deflated, each dated 1980-01-01 so the same Records
//! give the same bytes, and its sheet's bytes handed on a row at a time.
//! A sheet that would pass what Excel opens, in rows, columns or one
//! cell's length, is refused before any byte is written.

use std::io::Write;

use goodfetch_protocol::foxhound::export::SheetCapacity;
use zip::write::{SimpleFileOptions, StreamWriter};
use zip::{CompressionMethod, DateTime, ZipWriter};

use super::columns::{Cell, Columns};
use super::written::Written;

/// An XLSX Export's one sheet would hold more than Excel opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error(
    "the sheet would hold {figure} {}, past the {} Excel opens",
    unit(self.capacity),
    self.capacity.maximum()
)]
pub(crate) struct SheetCapacityExceeded {
    /// The capacity it passes.
    pub(crate) capacity: SheetCapacity,
    /// The Dataset's figure for it: the sheet's rows, the header row
    /// included, its columns, or its longest cell's length in UTF-16 code
    /// units.
    pub(crate) figure: u64,
}

/// What `capacity` counts, for the refusal's message.
fn unit(capacity: SheetCapacity) -> &'static str {
    match capacity {
        SheetCapacity::Rows => "rows",
        SheetCapacity::Columns => "columns",
        SheetCapacity::CellLength => "characters in one cell",
    }
}

/// Whether a sheet of `columns`, a header row then a row per Record they
/// saw, fits what Excel opens: its rows, then its columns, then its longest
/// cell.
pub(crate) fn fits(columns: &Columns) -> Result<(), SheetCapacityExceeded> {
    let figures = [
        (SheetCapacity::Rows, columns.records().saturating_add(1)),
        (
            SheetCapacity::Columns,
            u64::try_from(columns.names().len()).unwrap_or(u64::MAX),
        ),
        (SheetCapacity::CellLength, columns.longest()),
    ];
    match figures
        .into_iter()
        .find(|&(capacity, figure)| figure > capacity.maximum())
    {
        Some((capacity, figure)) => Err(SheetCapacityExceeded { capacity, figure }),
        None => Ok(()),
    }
}

const CONTENT_TYPES: &str = concat!(
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
    "\n",
    r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">"#,
    r#"<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>"#,
    r#"<Default Extension="xml" ContentType="application/xml"/>"#,
    r#"<Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>"#,
    r#"<Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>"#,
    r#"<Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/>"#,
    "</Types>"
);

const ROOT_RELATIONSHIPS: &str = concat!(
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
    "\n",
    r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
    r#"<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>"#,
    "</Relationships>"
);

const WORKBOOK_RELATIONSHIPS: &str = concat!(
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
    "\n",
    r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
    r#"<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>"#,
    r#"<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>"#,
    "</Relationships>"
);

const STYLES: &str = concat!(
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
    "\n",
    r#"<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">"#,
    r#"<fonts count="1"><font><sz val="11"/><name val="Calibri"/></font></fonts>"#,
    r#"<fills count="2"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill></fills>"#,
    r#"<borders count="1"><border><left/><right/><top/><bottom/><diagonal/></border></borders>"#,
    r#"<cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs>"#,
    r#"<cellXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/></cellXfs>"#,
    r#"<cellStyles count="1"><cellStyle name="Normal" xfId="0" builtinId="0"/></cellStyles>"#,
    "</styleSheet>"
);

const SHEET_START: &str = concat!(
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
    "\n",
    r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>"#
);

const SHEET_END: &str = "</sheetData></worksheet>";

/// The writer of one XLSX Export.
pub(super) struct Xlsx {
    sheet: String,
    written: Written,
    zip: Option<ZipWriter<StreamWriter<Written>>>,
    rows: u64,
}

impl std::fmt::Debug for Xlsx {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Xlsx")
            .field("sheet", &self.sheet)
            .field("rows", &self.rows)
            .finish_non_exhaustive()
    }
}

fn options() -> SimpleFileOptions {
    SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .last_modified_time(DateTime::default())
        .unix_permissions(0o644)
}

impl Xlsx {
    pub(super) fn new(sheet: String) -> Self {
        Self {
            sheet,
            written: Written::default(),
            zip: None,
            rows: 0,
        }
    }

    fn zip(&mut self) -> &mut ZipWriter<StreamWriter<Written>> {
        self.zip
            .as_mut()
            .expect("invariant: an XLSX Export's rows come after its start")
    }

    pub(super) fn begin(&mut self, names: &[String], out: &mut Vec<u8>) {
        let mut zip = ZipWriter::new_stream(self.written.clone());
        let mut workbook = String::from(concat!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
            "\n",
            r#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name=""#
        ));
        escape(&self.sheet, &mut workbook);
        workbook.push_str(r#"" sheetId="1" r:id="rId1"/></sheets></workbook>"#);
        for (name, text) in [
            ("[Content_Types].xml", CONTENT_TYPES),
            ("_rels/.rels", ROOT_RELATIONSHIPS),
            ("xl/workbook.xml", workbook.as_str()),
            ("xl/_rels/workbook.xml.rels", WORKBOOK_RELATIONSHIPS),
            ("xl/styles.xml", STYLES),
        ] {
            zip.start_file(name, options())
                .expect("invariant: a ZIP entry starts in memory");
            zip.write_all(text.as_bytes())
                .expect("invariant: writing into memory never fails");
        }
        zip.start_file("xl/worksheets/sheet1.xml", options().large_file(true))
            .expect("invariant: a ZIP entry starts in memory");
        zip.write_all(SHEET_START.as_bytes())
            .expect("invariant: writing into memory never fails");
        self.zip = Some(zip);
        let header: Vec<Cell> = names.iter().cloned().map(Cell::Text).collect();
        self.row(&header, out);
    }

    pub(super) fn row(&mut self, cells: &[Cell], out: &mut Vec<u8>) {
        self.rows += 1;
        let number = self.rows;
        let mut text = format!(r#"<row r="{number}">"#);
        let row = number.to_string();
        for (column, cell) in cells.iter().enumerate() {
            let open = |text: &mut String, kind: &str| {
                text.push_str(r#"<c r=""#);
                text.push_str(&letters(column));
                text.push_str(&row);
                text.push('"');
                text.push_str(kind);
                text.push('>');
            };
            match cell {
                Cell::Empty => {}
                Cell::Number(value) => {
                    open(&mut text, "");
                    text.push_str("<v>");
                    text.push_str(value);
                    text.push_str("</v></c>");
                }
                Cell::Boolean(boolean) => {
                    open(&mut text, r#" t="b""#);
                    text.push_str(if *boolean {
                        "<v>1</v></c>"
                    } else {
                        "<v>0</v></c>"
                    });
                }
                Cell::Text(string) | Cell::Json(string) => {
                    open(&mut text, r#" t="inlineStr""#);
                    text.push_str(r#"<is><t xml:space="preserve">"#);
                    escape(string, &mut text);
                    text.push_str("</t></is></c>");
                }
            }
        }
        text.push_str("</row>");
        self.zip()
            .write_all(text.as_bytes())
            .expect("invariant: writing into memory never fails");
        self.written.take(out);
    }

    pub(super) fn end(&mut self, out: &mut Vec<u8>) {
        let mut zip = self
            .zip
            .take()
            .expect("invariant: an XLSX Export ends after its start");
        zip.write_all(SHEET_END.as_bytes())
            .expect("invariant: writing into memory never fails");
        zip.finish()
            .expect("invariant: a ZIP written in memory finishes");
        self.written.take(out);
    }
}

/// The letters of the column at `index` from 0: `A` to `Z`, then `AA`.
fn letters(index: usize) -> String {
    let mut letters = Vec::new();
    let mut rest = index + 1;
    while rest > 0 {
        let digit = u8::try_from((rest - 1) % 26).expect("invariant: a digit below 26");
        letters.push(char::from(b'A' + digit));
        rest = (rest - 1) / 26;
    }
    letters.iter().rev().collect()
}

/// Writes `text` as `SpreadsheetML` text: XML's five escapes, `_xHHHH_` for
/// a character XML 1.0 cannot hold, and `_x005F_` for an `_` that would
/// start such an escape.
fn escape(text: &str, out: &mut String) {
    for (at, character) in text.char_indices() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '_' if starts_escape(&text[at..]) => out.push_str("_x005F_"),
            '\t' | '\n' | '\r' => out.push(character),
            '\u{0}'..='\u{1f}' | '\u{fffe}' | '\u{ffff}' => {
                let code = format!("_x{:04X}_", u32::from(character));
                out.push_str(&code);
            }
            other => out.push(other),
        }
    }
}

/// Whether `text` starts with `_x`, four hexadecimal digits and `_`.
fn starts_escape(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() >= 7
        && bytes[1] == b'x'
        && bytes[2..6].iter().all(u8::is_ascii_hexdigit)
        && bytes[6] == b'_'
}

#[cfg(test)]
mod tests {
    use goodfetch_protocol::foxhound::export::SheetCapacity;
    use rstest::rstest;

    use super::{SheetCapacityExceeded, escape, fits, letters};
    use crate::encoding::{Columns, Nested};

    /// A sheet holds 1,048,576 rows, its header row included: 1,048,575
    /// Records fit and one more is refused, naming the rows it would hold.
    /// The whole Export's refusal is `write_export`'s test in `tests/it`, as
    /// a run file of a million Records outlasts the Quick suite.
    #[rstest]
    #[case::at_the_capacity(1_048_575, Ok(()))]
    #[case::one_past_it(
        1_048_576,
        Err(SheetCapacityExceeded { capacity: SheetCapacity::Rows, figure: 1_048_577 })
    )]
    fn a_sheet_holds_a_header_and_1_048_575_records(
        #[case] records: u32,
        #[case] expected: Result<(), SheetCapacityExceeded>,
    ) {
        let mut columns = Columns::new(Nested::Json);
        for _ in 0..records {
            columns.see("{}");
        }

        let fitted = fits(&columns);

        assert_eq!(fitted, expected);
    }

    #[test]
    fn a_column_is_named_by_its_letters() {
        assert_eq!(
            [0, 25, 26, 51, 701, 702, 16_383].map(letters),
            ["A", "Z", "AA", "AZ", "ZZ", "AAA", "XFD"]
        );
    }

    #[test]
    fn text_is_escaped_as_spreadsheetml_reads_it() {
        let mut out = String::new();
        escape("a<b>&\"'\u{1}_x0041_ _x", &mut out);
        assert_eq!(out, "a&lt;b&gt;&amp;&quot;&apos;_x0001__x005F_x0041_ _x");
    }
}
