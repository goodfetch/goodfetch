//! The Markdown converter's fixes, each on a fixture that fails without
//! it: ADR 0007's two, code in `<pre>` without `<code>` and a table whose
//! rows start with a row header, and a heading with a line break.

use serde_json::json;

use super::read;

/// Covers: story:6-1
#[test]
fn code_in_a_pre_without_a_code_element_is_a_code_block() {
    let read = read(
        include_str!("../fixtures/pre-without-code.html"),
        json!({ "markdown": true }),
    );

    assert_eq!(
        read.markdown.as_deref(),
        Some(
            "## Building the lab\n\nThe lab builds with Cargo in release mode, which takes a few minutes on a laptop the first time and a few seconds after that, since only the crates that changed are compiled again.\n\n```\n$ cargo build --release\n   Compiling lab v0.1.0\n```\n\nWhen the build is done, the executable sits in the target folder, and every later command in these notes runs it from there without installing anything.\n"
        )
    );
}

/// Covers: story:6-1
#[test]
fn a_line_break_in_a_heading_keeps_the_heading_whole() {
    let read = read(
        include_str!("../fixtures/heading-with-break.html"),
        json!({ "markdown": true }),
    );

    assert_eq!(
        read.markdown.as_deref(),
        Some(
            "## Winter flying on the north beach\n\nCold air is denser than summer air, so the same wind lifts a kite harder in winter, and a smaller sail is often enough on the north beach from December to March.\n"
        )
    );
}

/// Covers: story:6-1
#[test]
fn a_tables_row_headers_are_kept_as_each_rows_first_cell() {
    let read = read(
        include_str!("../fixtures/row-headers.html"),
        json!({ "markdown": true }),
    );

    assert_eq!(
        read.markdown.as_deref(),
        Some(
            "## Desk Lamp\n\nA small desk lamp with a weighted base and an arm that holds its place, made for long evenings of reading and writing without tiring the eyes.\n\n| Power | 8 W |\n| --- | --- |\n| Colour | Black |\n\nThe lamp ships with a USB cable and works from any charger, so it needs no socket of its own on a crowded desk.\n"
        )
    );
}
