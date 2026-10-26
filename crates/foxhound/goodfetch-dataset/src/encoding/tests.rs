use goodfetch_protocol::foxhound::export::{ExportCompression, ExportFormat};
use rstest::rstest;

use super::{
    Columns, Compressor, ExportEncoder, ExportOptions, Exported, FeedFields, Nested, csv_delimiter,
    feed_field, json_indent, sheet_name, xml_name,
};

/// When the test's Records were committed: ADR 0059's test `Clock` start.
const COMMITTED_AT: &str = "2026-01-05T09:00:00Z";

/// The bytes of an Export of `records`, each a value and its Wall marker,
/// in `format` with `options`, a table's columns seen first, as
/// `run.export` sees them.
fn bytes_of(
    format: ExportFormat,
    options: &ExportOptions,
    records: &[(&str, Option<&str>)],
) -> Vec<u8> {
    let records: Vec<Exported<'_>> = records
        .iter()
        .zip(1..)
        .map(|(&(value, wall), number)| Exported {
            number,
            value,
            wall,
            committed_at: COMMITTED_AT,
        })
        .collect();
    let mut columns = Columns::new(options.nested(format));
    if format.is_table() {
        for record in &records {
            columns.see(&record.marked());
        }
    }
    let mut encoder = ExportEncoder::new(format, options, columns);
    let mut out = Vec::new();
    encoder.begin(&mut out);
    for record in &records {
        encoder.value(record, &mut out);
    }
    encoder.end(&mut out);
    out
}

/// The text of an Export of `values` in `format` with `options`.
fn encoded_with(format: ExportFormat, options: &ExportOptions, values: &[&str]) -> String {
    let records: Vec<(&str, Option<&str>)> = values.iter().map(|value| (*value, None)).collect();
    String::from_utf8(bytes_of(format, options, &records)).unwrap()
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

/// An XML Export writes each Record as an element of its fields: an
/// object's as elements inside it, an array's values as `item`s, `null` as
/// `xsi:nil`, and a name XML cannot take as `field` with a `name`.
///
/// Covers: story:6-276
#[test]
fn xml_writes_each_record_as_an_element_of_its_fields() {
    assert_eq!(
        encoded(
            ExportFormat::Xml,
            &[
                r#"{"page":1,"title":"Café"}"#,
                r#"{"author":{"name":"Zoé"},"note":null,"tags":["a","b"],"x y":"<&>\r\u0001"}"#,
            ]
        ),
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <records xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">\n\
         <record><page>1</page><title>Café</title></record>\n\
         <record><author><name>Zoé</name></author><note xsi:nil=\"true\"/>\
         <tags><item>a</item><item>b</item></tags><field name=\"x y\">&lt;&amp;&gt;&#13;\u{fffd}</field></record>\n\
         </records>\n"
    );
}

/// `export.xml.root` and `export.xml.record` name the XML elements, and a
/// name XML cannot take is refused.
///
/// Covers: setting:export.xml.root, setting:export.xml.record
#[test]
fn the_xml_elements_take_the_names_set_and_only_xml_names() {
    let options = ExportOptions {
        xml_root: "livres".to_owned(),
        xml_record: "livre".to_owned(),
        ..ExportOptions::of("books")
    };

    assert_eq!(
        (
            encoded_with(ExportFormat::Xml, &options, &[r#"{"a":1}"#]),
            xml_name("livre_2.x").as_deref(),
            xml_name("2livres").is_err(),
            xml_name("a:b").is_err(),
            xml_name("").is_err(),
        ),
        (
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <livres xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">\n\
             <livre><a>1</a></livre>\n</livres>\n"
                .to_owned(),
            Ok("livre_2.x"),
            true,
            true,
            true,
        )
    );
}

/// `export.json.indent` writes the array as a pretty printer does, each
/// level indented by that many spaces; above 8 it is refused.
///
/// Covers: setting:export.json.indent
#[test]
fn json_indent_writes_the_array_pretty_printed() {
    let values = [r#"{"a":1,"b":[1,2]}"#, r#"{"c":"Ünï"}"#];
    let options = ExportOptions {
        json_indent: 2,
        ..ExportOptions::of("books")
    };
    let parsed: Vec<serde_json::Value> = values
        .iter()
        .map(|value| serde_json::from_str(value).unwrap())
        .collect();

    assert_eq!(
        (
            encoded_with(ExportFormat::Json, &options, &values),
            encoded_with(ExportFormat::Json, &options, &[]),
            json_indent(8),
            json_indent(9),
        ),
        (
            serde_json::to_string_pretty(&parsed).unwrap(),
            "[]".to_owned(),
            Ok(8),
            Err("is more than 8"),
        )
    );
}

/// Records with a feed's fields, the second read past a paywall.
const FEED: [(&str, Option<&str>); 2] = [
    (
        r#"{"published":"2026-03-01T10:00:00+01:00","summary":"S & s","title":"T1","url":"https://shop.test/1?a=1&b=2"}"#,
        None,
    ),
    (
        r#"{"title":"T2"}"#,
        Some(r#"{"kind":"paywall","vendor":"v"}"#),
    ),
];

/// RSS writes one item per Record from the fields `export.feed.*` name, a
/// Wall marker as a category.
///
/// Covers: story:6-276, story:6-274
#[test]
fn rss_writes_one_item_per_record_from_the_feed_fields() {
    assert_eq!(
        String::from_utf8(bytes_of(
            ExportFormat::Rss,
            &ExportOptions::of("books"),
            &FEED
        ))
        .unwrap(),
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<rss version=\"2.0\">\n<channel>\n\
         <title>books</title>\n<link>urn:goodfetch:dataset:books</link>\n<description>books</description>\n\
         <item><title>T1</title><link>https://shop.test/1?a=1&amp;b=2</link><description>S &amp; s</description>\
         <pubDate>Sun, 1 Mar 2026 09:00:00 +0000</pubDate><guid isPermaLink=\"true\">https://shop.test/1?a=1&amp;b=2</guid></item>\n\
         <item><title>T2</title><guid isPermaLink=\"false\">urn:goodfetch:dataset:books:2</guid>\
         <category domain=\"urn:goodfetch:wall\">paywall</category></item>\n\
         </channel>\n</rss>\n"
    );
}

/// Atom writes one entry per Record, `updated` from its date or when it
/// was committed, and the feed's `updated` its latest entry's.
///
/// Covers: story:6-276, story:6-274
#[test]
fn atom_writes_one_entry_per_record_and_the_latest_update() {
    assert_eq!(
        String::from_utf8(bytes_of(
            ExportFormat::Atom,
            &ExportOptions::of("books"),
            &FEED
        ))
        .unwrap(),
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<feed xmlns=\"http://www.w3.org/2005/Atom\">\n\
         <title>books</title>\n<id>urn:goodfetch:dataset:books</id>\n<author><name>books</name></author>\n\
         <entry><id>https://shop.test/1?a=1&amp;b=2</id><title>T1</title><link href=\"https://shop.test/1?a=1&amp;b=2\"/>\
         <published>2026-03-01T09:00:00Z</published><updated>2026-03-01T09:00:00Z</updated><summary>S &amp; s</summary></entry>\n\
         <entry><id>urn:goodfetch:dataset:books:2</id><title>T2</title><updated>2026-01-05T09:00:00Z</updated>\
         <category scheme=\"urn:goodfetch:wall\" term=\"paywall\"/></entry>\n\
         <updated>2026-03-01T09:00:00Z</updated>\n</feed>\n"
    );
}

/// `export.feed.*` name other fields; an empty name is refused.
///
/// Covers: setting:export.feed.title, setting:export.feed.link, setting:export.feed.published, setting:export.feed.summary
#[test]
fn the_feed_fields_are_those_the_settings_name() {
    let options = ExportOptions {
        feed: FeedFields {
            title: "name".to_owned(),
            link: "href".to_owned(),
            published: "date".to_owned(),
            summary: "text".to_owned(),
        },
        ..ExportOptions::of("books")
    };
    let rss = encoded_with(
        ExportFormat::Rss,
        &options,
        &[
            r#"{"date":"Tue, 3 Mar 2026 08:00:00 +0000","href":"https://a.test/","name":"N","text":"X","title":"no"}"#,
        ],
    );

    assert_eq!(
        (rss.lines().nth(6), feed_field("")),
        (
            Some(
                "<item><title>N</title><link>https://a.test/</link><description>X</description>\
                 <pubDate>Tue, 3 Mar 2026 08:00:00 +0000</pubDate><guid isPermaLink=\"true\">https://a.test/</guid></item>"
            ),
            Err("is empty"),
        )
    );
}

/// A Record read past a Wall carries its marker as a `#wall` field: in
/// JSON beside its own fields, in a table as a column of its own; one
/// that met no Wall is written as committed.
///
/// Covers: story:6-274
#[rstest]
#[case::json(
    ExportFormat::Json,
    r##"[{"title":"T1"},{"#wall":{"kind":"paywall","vendor":"v"},"title":"T2"}]"##
)]
#[case::jsonl(
    ExportFormat::Jsonl,
    "{\"title\":\"T1\"}\n{\"#wall\":{\"kind\":\"paywall\",\"vendor\":\"v\"},\"title\":\"T2\"}\n"
)]
#[case::csv(
    ExportFormat::Csv,
    "title,#wall\r\nT1,\r\nT2,\"{\"\"kind\"\":\"\"paywall\"\",\"\"vendor\"\":\"\"v\"\"}\"\r\n"
)]
#[case::xml(
    ExportFormat::Xml,
    "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
     <records xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">\n\
     <record><title>T1</title></record>\n\
     <record><field name=\"#wall\"><kind>paywall</kind><vendor>v</vendor></field><title>T2</title></record>\n\
     </records>\n"
)]
fn a_record_read_past_a_wall_carries_its_marker(
    #[case] format: ExportFormat,
    #[case] expected: &str,
) {
    let records = [
        (r#"{"title":"T1"}"#, None),
        (
            r#"{"title":"T2"}"#,
            Some(r#"{"kind":"paywall","vendor":"v"}"#),
        ),
    ];

    assert_eq!(
        String::from_utf8(bytes_of(format, &ExportOptions::of("books"), &records)).unwrap(),
        expected
    );
}

/// gzip and zstd compress the whole file, written in parts, so that their
/// readers give back its bytes; the same bytes give the same file.
///
/// Covers: story:6-276
#[rstest]
#[case::gzip(ExportCompression::Gzip)]
#[case::zstd(ExportCompression::Zstd)]
fn compression_gives_back_the_bytes_written_in_parts(#[case] compression: ExportCompression) {
    let compressed = || {
        let mut compressor = Compressor::new(compression);
        let mut out = compressor.write(b"one ".to_vec());
        out.append(&mut compressor.write(Vec::new()));
        out.append(&mut compressor.write("ünï ".repeat(1000).into_bytes()));
        out.append(&mut compressor.finish());
        out
    };
    let bytes = compressed();
    let mut plain = Vec::new();
    if compression == ExportCompression::Gzip {
        std::io::Read::read_to_end(&mut flate2::read::GzDecoder::new(&bytes[..]), &mut plain)
            .unwrap();
    } else {
        plain = zstd::decode_all(&bytes[..]).unwrap();
    }

    assert_eq!(
        (plain, bytes == compressed()),
        (
            [b"one ".to_vec(), "ünï ".repeat(1000).into_bytes()].concat(),
            true
        )
    );
}
