//! The page's tables (spec #580, ticket #786): each table's caption,
//! header rows and body rows, a row's header kept as its first cell
//! (ADR 0007's table fix).

use goodfetch_protocol::page::Table;
use serde_json::json;

use super::read;

fn tables(html: &str) -> Vec<Table> {
    read(html, json!({ "tables": true })).tables.unwrap()
}

fn rows(rows: &[&[&str]]) -> Vec<Vec<String>> {
    rows.iter()
        .map(|row| row.iter().map(|&cell| cell.to_owned()).collect())
        .collect()
}

/// Covers: story:6-5
#[test]
fn a_tables_caption_header_rows_and_body_rows_keep_each_rows_header_first() {
    let html = r#"<table>
        <caption> Line strength <i>by material</i> </caption>
        <thead><tr><th></th><th colspan="2">Breaking strain</th></tr><tr><th>Material</th><th>kg</th><th>lb</th></tr></thead>
        <tbody>
          <tr><th scope="row">Polyester</th><td>45</td><td>100</td></tr>
          <tr><th scope="row">Dyneema</th><td> 90 </td><td>200</td></tr>
        </tbody>
        <tfoot><tr><td>Measured</td><td colspan="2">dry, at 20 &deg;C</td></tr></tfoot>
    </table>"#;

    assert_eq!(
        tables(html),
        [Table {
            caption: Some("Line strength by material".to_owned()),
            header_rows: rows(&[&["", "Breaking strain"], &["Material", "kg", "lb"]]),
            body_rows: rows(&[
                &["Polyester", "45", "100"],
                &["Dyneema", "90", "200"],
                &["Measured", "dry, at 20 \u{b0}C"],
            ]),
        }]
    );
}

/// Covers: story:6-5
#[test]
fn without_a_thead_the_rows_of_th_it_begins_with_are_its_header_rows() {
    let html = "<table><tr><th>Knot</th><th>Use</th></tr><tr><th>Bowline</th><td>A loop<br>that holds</td></tr><tr><td>Hitch</td><td>To a post</td></tr></table>";

    assert_eq!(
        tables(html),
        [Table {
            caption: None,
            header_rows: rows(&[&["Knot", "Use"]]),
            body_rows: rows(&[&["Bowline", "A loop that holds"], &["Hitch", "To a post"]]),
        }]
    );
}

/// Covers: story:6-5
#[test]
fn a_table_inside_another_is_listed_on_its_own_after_it_and_a_tfoot_comes_last() {
    let html = "<table><tfoot><tr><td>Total</td></tr></tfoot><tr><td>Outer <table><tr><td>Inner</td></tr></table></td></tr></table><p>No table</p>";

    assert_eq!(
        tables(html),
        [
            Table {
                caption: None,
                header_rows: Vec::new(),
                body_rows: rows(&[&["Outer"], &["Total"]]),
            },
            Table {
                caption: None,
                header_rows: Vec::new(),
                body_rows: rows(&[&["Inner"]]),
            },
        ]
    );
}

/// Covers: story:6-5
#[test]
fn a_page_with_no_table_answers_none() {
    assert_eq!(tables("<p>Only text</p>"), []);
}
