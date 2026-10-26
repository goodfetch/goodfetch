use goodfetch_protocol::foxhound::export::ExportFormat;
use rstest::rstest;

use super::{Columns, ExportEncoder, ExportOptions, Nested, csv_delimiter, sheet_name};

/// The bytes of an Export of `values` in `format` with `options`, a
/// table's columns seen first, as `run.export` sees them.
fn encoded_with(format: ExportFormat, options: &ExportOptions, values: &[&str]) -> String {
    let mut columns = Columns::new(options.nested(format));
    if format.is_table() {
        for value in values {
            columns.see(value);
        }
    }
    let mut encoder = ExportEncoder::new(format, options, columns);
    let mut out = Vec::new();
    encoder.begin(&mut out);
    for value in values {
        encoder.value(value, &mut out);
    }
    encoder.end(&mut out);
    String::from_utf8(out).unwrap()
}

fn encoded(format: ExportFormat, values: &[&str]) -> String {
    encoded_with(format, &ExportOptions::of("books"), values)
}

/// Records whose later ones add fields, one a nested object, one an array.
const GROWING: [&str; 3] = [
    r#"{"page":1,"title":"Café"}"#,
    r#"{"page":2,"price":12.5,"title":"Thé, \"vert\""}"#,
    r#"{"author":{"born":{"year":1950},"name":"Zoé"},"in_stock":true,"note":null,"page":3,"tags":["a","b"]}"#,
];

/// A table carries every field seen, in first-seen order, a missing field
/// and `null` as an empty cell, an object and an array as JSON text.
///
/// Covers: story:6-278
#[rstest]
#[case::csv(
    ExportFormat::Csv,
    "page,title,price,author,in_stock,note,tags\r\n\
     1,Café,,,,,\r\n\
     2,\"Thé, \"\"vert\"\"\",12.5,,,,\r\n\
     3,,,\"{\"\"born\"\":{\"\"year\"\":1950},\"\"name\"\":\"\"Zoé\"\"}\",true,,\"[\"\"a\"\",\"\"b\"\"]\"\r\n"
)]
#[case::markdown(
    ExportFormat::Markdown,
    "| page | title | price | author | in\\_stock | note | tags |\n\
     | --- | --- | --- | --- | --- | --- | --- |\n\
     | 1 | Café |  |  |  |  |  |\n\
     | 2 | Thé, \"vert\" | 12.5 |  |  |  |  |\n\
     | 3 |  |  | {\"born\":{\"year\":1950},\"name\":\"Zoé\"} | true |  | \\[\"a\",\"b\"\\] |\n"
)]
#[case::html_table(
    ExportFormat::HtmlTable,
    "<!DOCTYPE html>\n<html>\n<head>\n<meta charset=\"utf-8\">\n<title>books</title>\n</head>\n<body>\n<table>\n<thead>\n\
     <tr><th>page</th><th>title</th><th>price</th><th>author</th><th>in_stock</th><th>note</th><th>tags</th></tr>\n\
     </thead>\n<tbody>\n\
     <tr><td>1</td><td>Café</td><td></td><td></td><td></td><td></td><td></td></tr>\n\
     <tr><td>2</td><td>Thé, &quot;vert&quot;</td><td>12.5</td><td></td><td></td><td></td><td></td></tr>\n\
     <tr><td>3</td><td></td><td></td><td>{&quot;born&quot;:{&quot;year&quot;:1950},&quot;name&quot;:&quot;Zoé&quot;}</td><td>true</td><td></td><td>[&quot;a&quot;,&quot;b&quot;]</td></tr>\n\
     </tbody>\n</table>\n</body>\n</html>\n"
)]
fn a_table_carries_every_field_seen_in_first_seen_order(
    #[case] format: ExportFormat,
    #[case] expected: &str,
) {
    assert_eq!(encoded(format, &GROWING), expected);
}

/// `export.csv.bom`, `export.csv.delimiter` and `export.csv.nested:
/// flatten` shape a CSV Export: a BOM first, `;` between fields, and an
/// object's fields in dotted columns at any depth.
///
/// Covers: setting:export.csv.bom, setting:export.csv.delimiter, setting:export.csv.nested
#[test]
fn csv_options_put_a_bom_another_delimiter_and_dotted_columns() {
    let options = ExportOptions {
        csv_bom: true,
        csv_delimiter: ';',
        csv_nested: Nested::Flatten,
        ..ExportOptions::of("books")
    };

    assert_eq!(
        encoded_with(ExportFormat::Csv, &options, &GROWING),
        "\u{feff}page;title;price;author.born.year;author.name;in_stock;note;tags\r\n\
         1;Café;;;;;;\r\n\
         2;\"Thé, \"\"vert\"\"\";12.5;;;;;\r\n\
         3;;;1950;Zoé;true;;\"[\"\"a\"\",\"\"b\"\"]\"\r\n"
    );
}

/// A CSV field holding a line break is quoted, and a one-column row whose
/// field is empty is `""`, so a reader never skips it as a blank line.
#[test]
fn csv_quotes_line_breaks_and_a_lone_empty_field() {
    assert_eq!(
        encoded(
            ExportFormat::Csv,
            &[
                r#"{"text":"one\ntwo"}"#,
                r#"{"text":""}"#,
                r#"{"text":null}"#
            ]
        ),
        "text\r\n\"one\ntwo\"\r\n\"\"\r\n\"\"\r\n"
    );
}

/// Markdown escapes what would start inline markup or end a cell, and
/// writes a line break as `<br>`.
#[test]
fn markdown_escapes_markup_and_writes_line_breaks_as_br() {
    assert_eq!(
        encoded(
            ExportFormat::Markdown,
            &[r#"{"text":"a|b *c* `d` <e> [f](g) \\ ~h~ &amp; one\r\ntwo\nthree"}"#]
        ),
        "| text |\n| --- |\n\
         | a\\|b \\*c\\* \\`d\\` \\<e\\> \\[f\\](g) \\\\ \\~h\\~ \\&amp; one<br>two<br>three |\n"
    );
}

/// A table of Records holding no field writes no row in CSV or Markdown.
#[rstest]
#[case::csv(ExportFormat::Csv)]
#[case::markdown(ExportFormat::Markdown)]
fn a_table_of_no_field_writes_nothing(#[case] format: ExportFormat) {
    assert_eq!(encoded(format, &["{}", "{}"]), "");
}

/// `export.csv.delimiter` is one character a CSV field never quotes for.
#[rstest]
#[case::comma(",", Ok(','))]
#[case::tab("\t", Ok('\t'))]
#[case::a_letter_beyond_ascii("§", Ok('§'))]
#[case::empty("", Err("is not one character"))]
#[case::two("ab", Err("is not one character"))]
#[case::a_quote(
    "\"",
    Err("is a double quote, a carriage return or a line feed, which a CSV field holds")
)]
#[case::a_line_feed(
    "\n",
    Err("is a double quote, a carriage return or a line feed, which a CSV field holds")
)]
fn a_delimiter_is_one_character_a_field_does_not_hold(
    #[case] text: &str,
    #[case] expected: Result<char, &str>,
) {
    assert_eq!(csv_delimiter(text), expected);
}

/// `export.xlsx.sheet` names a sheet only as a spreadsheet allows; the
/// built-in names it after the Dataset, made fit.
#[test]
fn a_sheet_name_is_checked_and_the_built_in_one_made_fit() {
    assert_eq!(
        (
            sheet_name("Livres 2026").as_deref(),
            sheet_name(""),
            sheet_name(&"x".repeat(32)),
            sheet_name("a/b"),
            sheet_name("'quoted'"),
            ExportOptions::of("books[2026]/fr").xlsx_sheet,
            ExportOptions::of(&"y".repeat(40)).xlsx_sheet,
        ),
        (
            Ok("Livres 2026"),
            Err("is empty"),
            Err("is longer than 31 characters"),
            Err("holds one of `:`, `\\`, `/`, `?`, `*`, `[` and `]`"),
            Err("starts or ends with `'`"),
            "books_2026__fr".to_owned(),
            "y".repeat(31),
        )
    );
}

#[rstest]
#[case::json_of_none(ExportFormat::Json, &[], "[]")]
#[case::jsonl_of_none(ExportFormat::Jsonl, &[], "")]
#[case::json_of_two(
    ExportFormat::Json,
    &[r#"{"title":"Café"}"#, r#"{"title":"Ünïcode"}"#],
    r#"[{"title":"Café"},{"title":"Ünïcode"}]"#
)]
#[case::jsonl_of_two(
    ExportFormat::Jsonl,
    &[r#"{"title":"Café"}"#, r#"{"title":"Ünïcode"}"#],
    "{\"title\":\"Café\"}\n{\"title\":\"Ünïcode\"}\n"
)]
fn an_export_writes_each_value_as_committed_in_its_format(
    #[case] format: ExportFormat,
    #[case] values: &[&str],
    #[case] expected: &str,
) {
    assert_eq!(encoded(format, values), expected);
}

/// The Export profile's corpus files (ADR 0066), as `xtask compat write`
/// wrote them.
const HEAD_JSON: &str = include_str!("../../../../../compat/export/head/books.json");
const HEAD_JSONL: &str = include_str!("../../../../../compat/export/head/books.jsonl");

/// The corpus's Exports read back as a consumer reads them, each line of
/// the JSONL file one value, and written again, give the same bytes in
/// both formats: the profile holds each value as JSON writes it, in order,
/// with no BOM.
///
/// Covers: format:export
#[test]
fn the_export_profiles_head_set_reads_back_and_writes_the_same_bytes() {
    let values: Vec<String> = HEAD_JSONL
        .lines()
        .map(|line| {
            serde_json::from_str::<serde_json::Value>(line)
                .unwrap()
                .to_string()
        })
        .collect();
    let read: Vec<&str> = values.iter().map(String::as_str).collect();
    let array: Vec<serde_json::Value> = serde_json::from_str(HEAD_JSON).unwrap();

    assert_eq!(
        (
            encoded(ExportFormat::Json, &read),
            encoded(ExportFormat::Jsonl, &read),
            array.len(),
        ),
        (HEAD_JSON.to_owned(), HEAD_JSONL.to_owned(), 3)
    );
}
