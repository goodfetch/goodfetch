//! The Export part of the File saving conformance cases for the formats
//! after the tables, on Foxhound's executable over stdio (spec #582's seam:
//! the protocol; CO-27, ADRs 0026, 0052 and 0103): every one of the twelve
//! formats, plain, gzip and zstd, opens in a reference reader and gives the
//! Records back, the Unicode record round-tripping in the text formats; a
//! Record read past a Wall carries its marker in every format, and
//! `run.records` and `run.export` filter by it; the `export.*` settings of
//! JSON, XML, Parquet and the feeds shape them, a value they cannot take
//! failing with `setting_invalid`.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use arrow_array::RecordBatch;
use arrow_array::cast::AsArray as _;
use arrow_array::types::{Float64Type, Int64Type};
use arrow_schema::DataType;
use goodfetch_test_support::TestEnv;
use quick_xml::Reader;
use quick_xml::escape::resolve_predefined_entity;
use quick_xml::events::Event;
use serde_json::{Map, Value, json};

use crate::crawl::Attached;
use crate::exports::{
    book, crawl, csv_rows, export_under, finish, html_rows, markdown_rows, saved, table_of,
    third_book, unicode_value, xlsx_rows,
};

/// The twelve formats.
const FORMATS: [&str; 12] = [
    "json",
    "jsonl",
    "csv",
    "xlsx",
    "xml",
    "parquet",
    "html_table",
    "rss",
    "atom",
    "avro",
    "arrow_ipc",
    "markdown",
];

/// `run.export`'s reply for `books` of `run_id` in `format`, compressed
/// as `compression` says, under `settings`, with `wall`'s filter.
fn exported(
    engine: &mut Attached,
    run_id: &str,
    format: &str,
    compression: &str,
    wall: Option<bool>,
) -> Value {
    let mut params = json!({
        "run_id": run_id, "dataset": "books", "format": format, "compression": compression,
    });
    if let Some(wall) = wall {
        params["wall"] = json!(wall);
    }
    engine.call("export", "run.export", &params, &mut |_| None)
}

/// `bytes` decompressed as `compression` says.
fn plain(bytes: &[u8], compression: &str) -> Vec<u8> {
    match compression {
        "gzip" => {
            let mut plain = Vec::new();
            std::io::Read::read_to_end(&mut flate2::read::GzDecoder::new(bytes), &mut plain)
                .unwrap();
            plain
        }
        "zstd" => zstd::decode_all(bytes).unwrap(),
        _ => bytes.to_vec(),
    }
}

/// A Record as a typed format reads it back: `null` and absent fields
/// left out, an object or array as its JSON text.
fn typed(record: &Value) -> Value {
    Value::Object(
        record
            .as_object()
            .unwrap()
            .iter()
            .filter(|(_, value)| !value.is_null())
            .map(|(name, value)| {
                let value = match value {
                    Value::Object(_) | Value::Array(_) => Value::String(value.to_string()),
                    other => other.clone(),
                };
                (name.clone(), value)
            })
            .collect::<Map<_, _>>(),
    )
}

/// A Record as XML reads it back: every scalar its text, `null` kept.
fn texts(value: &Value) -> Value {
    match value {
        Value::Object(object) => Value::Object(
            object
                .iter()
                .map(|(name, value)| (name.clone(), texts(value)))
                .collect(),
        ),
        Value::Array(values) => Value::Array(values.iter().map(texts).collect()),
        Value::Null => Value::Null,
        Value::String(text) => Value::String(text.clone()),
        other => Value::String(other.to_string()),
    }
}

/// The columns' names and types, and the rows, of Arrow record batches,
/// each row its non-null cells by column.
fn batch_rows(batches: &[RecordBatch]) -> (Vec<(String, DataType)>, Vec<Value>) {
    let mut columns = Vec::new();
    let mut rows = Vec::new();
    for batch in batches {
        columns = batch
            .schema()
            .fields()
            .iter()
            .map(|field| (field.name().clone(), field.data_type().clone()))
            .collect();
        for row in 0..batch.num_rows() {
            let mut cells = Map::new();
            for (index, (name, data_type)) in columns.iter().enumerate() {
                let column = batch.column(index);
                if column.is_null(row) {
                    continue;
                }
                let cell = match data_type {
                    DataType::Boolean => json!(column.as_boolean().value(row)),
                    DataType::Int64 => json!(column.as_primitive::<Int64Type>().value(row)),
                    DataType::Float64 => json!(column.as_primitive::<Float64Type>().value(row)),
                    DataType::Utf8 => json!(column.as_string::<i32>().value(row)),
                    other => panic!("unexpected column type {other}"),
                };
                cells.insert(name.clone(), cell);
            }
            rows.push(Value::Object(cells));
        }
    }
    (columns, rows)
}

pub(crate) fn parquet_rows(bytes: Vec<u8>) -> (Vec<(String, DataType)>, Vec<Value>) {
    let batches: Vec<RecordBatch> =
        parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder::try_new(bytes::Bytes::from(
            bytes,
        ))
        .unwrap()
        .build()
        .unwrap()
        .map(Result::unwrap)
        .collect();
    batch_rows(&batches)
}

fn arrow_rows(bytes: Vec<u8>) -> (Vec<(String, DataType)>, Vec<Value>) {
    let batches: Vec<RecordBatch> =
        arrow_ipc::reader::FileReader::try_new(std::io::Cursor::new(bytes), None)
            .unwrap()
            .map(Result::unwrap)
            .collect();
    batch_rows(&batches)
}

/// The fields' names (each its `doc` where it has one) and the records of
/// an Avro file, each record its non-null fields.
fn avro_rows(bytes: &[u8]) -> (Vec<String>, Vec<Value>) {
    use apache_avro::types::Value as Avro;

    let reader = apache_avro::Reader::new(bytes).unwrap();
    let apache_avro::Schema::Record(schema) = reader.writer_schema().clone() else {
        panic!("an Avro Export's schema is a record");
    };
    let names: Vec<String> = schema
        .fields
        .iter()
        .map(|field| field.doc.clone().unwrap_or_else(|| field.name.clone()))
        .collect();
    let rows = reader
        .map(|record| {
            let Avro::Record(fields) = record.unwrap() else {
                panic!("an Avro Export holds records");
            };
            let mut cells = Map::new();
            for (name, (_, value)) in names.iter().zip(fields) {
                let value = match value {
                    Avro::Union(_, inner) => *inner,
                    other => other,
                };
                let cell = match value {
                    Avro::Null => continue,
                    Avro::Boolean(boolean) => json!(boolean),
                    Avro::Long(long) => json!(long),
                    Avro::Double(double) => json!(double),
                    Avro::String(text) => json!(text),
                    other => panic!("unexpected Avro value {other:?}"),
                };
                cells.insert(name.clone(), cell);
            }
            Value::Object(cells)
        })
        .collect();
    (names, rows)
}

/// The text of an XML element's content events, resolving references.
fn push_text(event: &Event<'_>, text: &mut String) {
    match event {
        Event::Text(part) => text.push_str(&part.decode().unwrap()),
        Event::GeneralRef(reference) => match reference.resolve_char_ref().unwrap() {
            Some(character) => text.push(character),
            None => text.push_str(resolve_predefined_entity(&reference.decode().unwrap()).unwrap()),
        },
        _ => {}
    }
}

/// One XML element read back: its name, its attributes, its text and its
/// children.
#[derive(Debug, Default)]
struct Element {
    name: String,
    attributes: Map<String, Value>,
    text: String,
    children: Vec<Element>,
}

/// The document element of the XML `text`.
fn document(text: &str) -> Element {
    let mut reader = Reader::from_str(text);
    let mut stack: Vec<Element> = vec![Element::default()];
    loop {
        let event = reader.read_event().unwrap();
        match &event {
            Event::Start(start) | Event::Empty(start) => {
                let mut element = Element {
                    name: String::from_utf8(start.name().as_ref().to_vec()).unwrap(),
                    ..Element::default()
                };
                for attribute in start.attributes().map(Result::unwrap) {
                    let key = String::from_utf8(attribute.key.as_ref().to_vec()).unwrap();
                    let value = crate::exports::attribute(start, attribute.key.as_ref()).unwrap();
                    element.attributes.insert(key, json!(value));
                }
                if matches!(event, Event::Empty(_)) {
                    stack.last_mut().unwrap().children.push(element);
                } else {
                    stack.push(element);
                }
            }
            Event::End(_) => {
                let element = stack.pop().unwrap();
                stack.last_mut().unwrap().children.push(element);
            }
            Event::Eof => break,
            other => push_text(other, &mut stack.last_mut().unwrap().text),
        }
    }
    stack.pop().unwrap().children.pop().unwrap()
}

/// An XML Export's element read back as JSON: `xsi:nil` as `null`, `item`
/// children as an array, other children as an object's fields (a `field`
/// by its `name`), and text as a string.
fn xml_value(element: &Element) -> Value {
    if element.attributes.get("xsi:nil") == Some(&json!("true")) {
        return Value::Null;
    }
    if element.children.is_empty() {
        return Value::String(element.text.clone());
    }
    if element.children.iter().all(|child| child.name == "item") {
        return Value::Array(element.children.iter().map(xml_value).collect());
    }
    Value::Object(
        element
            .children
            .iter()
            .map(|child| {
                let name = match child.attributes.get("name") {
                    Some(Value::String(name)) if child.name == "field" => name.clone(),
                    _ => child.name.clone(),
                };
                (name, xml_value(child))
            })
            .collect(),
    )
}

/// The root's name, each Record element's name, and the Records of an XML
/// Export.
fn xml_rows(bytes: &[u8]) -> (String, Vec<String>, Vec<Value>) {
    let root = document(std::str::from_utf8(bytes).unwrap());
    (
        root.name.clone(),
        root.children
            .iter()
            .map(|child| child.name.clone())
            .collect(),
        root.children.iter().map(xml_value).collect(),
    )
}

/// A feed's items or entries, each element's text or `href` by its name,
/// and a category by its scheme.
fn feed_items(bytes: &[u8]) -> (String, Vec<Value>) {
    let root = document(std::str::from_utf8(bytes).unwrap());
    let (title_holder, items) = if root.name == "rss" {
        let channel = &root.children[0];
        (channel, "item")
    } else {
        (&root, "entry")
    };
    let title = title_holder
        .children
        .iter()
        .find(|child| child.name == "title")
        .map(|child| child.text.clone())
        .unwrap();
    let read = title_holder
        .children
        .iter()
        .filter(|child| child.name == items)
        .map(|item| {
            Value::Object(
                item.children
                    .iter()
                    .map(|child| {
                        let value = child
                            .attributes
                            .get("href")
                            .or_else(|| child.attributes.get("term"))
                            .cloned()
                            .unwrap_or_else(|| json!(child.text));
                        (child.name.clone(), value)
                    })
                    .collect(),
            )
        })
        .collect();
    (title, read)
}

/// The Records of the format tests: the first book, the Unicode record,
/// and the third book with every kind of value.
fn three() -> [Value; 3] {
    [book(1), unicode_value(), third_book()]
}

/// What each format's reference reader gives back of `bytes`, which
/// `three()` was exported as, beside what it should give.
fn read_back(format: &str, bytes: Vec<u8>) -> (Value, Value) {
    let records = three();
    let columns = [
        "page",
        "title",
        "astral",
        "bidi",
        "bom",
        "nfc",
        "nfd",
        "tab_and_newline",
        "zero_width_joiner",
        "zwj_emoji",
        "author",
        "in_stock",
        "note",
        "price",
        "tags",
    ];
    let table = table_of(&columns, &records);
    let typed_rows: Vec<Value> = records.iter().map(typed).collect();
    let text = || String::from_utf8(bytes.clone()).unwrap();
    match format {
        "json" => (serde_json::from_slice(&bytes).unwrap(), json!(records)),
        "jsonl" => (
            json!(
                text()
                    .lines()
                    .map(|line| serde_json::from_str::<Value>(line).unwrap())
                    .collect::<Vec<_>>()
            ),
            json!(records),
        ),
        "csv" => (json!(csv_rows(&bytes, b',')), json!(table)),
        "xlsx" => (json!(xlsx_rows(bytes).1), json!(table)),
        "html_table" => (json!(html_rows(&text()).1), json!(table)),
        "markdown" => (
            json!(markdown_rows(&text())),
            json!(
                table
                    .iter()
                    .map(|row| row.iter().map(|cell| cell.replace('\n', "<br>")).collect())
                    .collect::<Vec<Vec<String>>>()
            ),
        ),
        "xml" => {
            let (root, names, read) = xml_rows(&bytes);
            (
                json!([root, names, read]),
                json!([
                    "records",
                    ["record", "record", "record"],
                    records.iter().map(texts).collect::<Vec<_>>()
                ]),
            )
        }
        "parquet" | "arrow_ipc" => {
            let (types, rows) = if format == "parquet" {
                parquet_rows(bytes)
            } else {
                arrow_rows(bytes)
            };
            let types: Vec<Value> = types
                .iter()
                .map(|(name, data_type)| json!([name, data_type.to_string()]))
                .collect();
            (
                json!([types, rows]),
                json!([typed_columns(&columns), typed_rows]),
            )
        }
        "avro" => {
            let (names, rows) = avro_rows(&bytes);
            (json!([names, rows]), json!([columns, typed_rows]))
        }
        _ => {
            let (title, items) = feed_items(&bytes);
            let titles: Vec<Value> = items
                .iter()
                .map(|item| item.get("title").cloned().unwrap_or(Value::Null))
                .collect();
            (
                json!([title, titles]),
                json!(["books", ["Café 1", "", "Café 3"]]),
            )
        }
    }
}

/// The columns of `three()` with the types the typed formats give them.
fn typed_columns(columns: &[&str]) -> Vec<Value> {
    columns
        .iter()
        .map(|&name| {
            let data_type = match name {
                "page" => "Int64",
                "in_stock" => "Boolean",
                "price" => "Float64",
                _ => "Utf8",
            };
            json!([name, data_type])
        })
        .collect()
}

/// CO-27's acceptance test: each of the twelve formats, plain, gzip and
/// zstd, opens in a reference reader (serde_json, a CSV reader, the
/// workbook's ZIP and XML, an HTML parser, a Markdown parser, quick-xml for
/// XML and the feeds, arrow-rs for Parquet and Arrow IPC, apache-avro) and
/// gives the Records back, the Unicode record's every field among them in
/// the text formats; a compressed Export is the plain one's bytes, as gzip
/// and zstd give them back.
///
/// Covers: story:6-276, story:6-277, operation:run.export, operation:export.save_as, format:export
#[test]
fn every_format_plain_gzip_and_zstd_opens_in_a_reference_reader_and_gives_the_records_back() {
    let env = TestEnv::new();
    let mut engine = Attached::start(&env);
    crawl(&mut engine, "formats", 3);
    let [first, second, third] = three();
    finish(&mut engine, "formats", |page| match page {
        1 => first.clone(),
        2 => second.clone(),
        _ => third.clone(),
    });
    let mut read = Vec::new();
    let mut compressed = Vec::new();
    for format in FORMATS {
        let reply = exported(&mut engine, "formats", format, "none", None);
        let bytes = saved(&mut engine, &env, &reply, &format!("books.{format}"));
        for compression in ["gzip", "zstd"] {
            let packed = exported(&mut engine, "formats", format, compression, None);
            let packed_bytes = saved(
                &mut engine,
                &env,
                &packed,
                &format!("books.{format}.{compression}"),
            );
            compressed.push((
                format,
                compression,
                packed["result"]["compression"].clone(),
                plain(&packed_bytes, compression) == bytes,
            ));
        }
        read.push((
            format,
            reply["result"]["records"].clone(),
            read_back(format, bytes),
        ));
    }
    assert!(engine.finish());

    for (format, records, (got, expected)) in read {
        assert_eq!((format, records, got), (format, json!(3), expected));
    }
    for (format, compression, answered, same) in compressed {
        assert_eq!((format, answered, same), (format, json!(compression), true));
    }
}

/// Marks the Record numbered `number` of `books` in `run_id`'s run file as
/// read past a paywall of the vendor `v`, as the Wall allowance's commit
/// writes its Provenance (ADR 0052): the engine sets no Wall marker in M1,
/// so the test writes the Provenance a Record past a Wall carries.
fn mark_wall(env: &TestEnv, run_id: &str, number: i64) {
    let connection = rusqlite::Connection::open(
        env.home()
            .join("0.0")
            .join("runs")
            .join(run_id)
            .join("run.sqlite"),
    )
    .unwrap();
    let changed = connection
        .execute(
            "UPDATE records SET provenance = json_set(provenance, '$.wall', json(?1))
             WHERE dataset = 'books' AND number = ?2",
            (r#"{"kind":"paywall","vendor":"v"}"#, number),
        )
        .unwrap();
    assert_eq!(changed, 1);
}

/// The numbers of the Records `run.records` reads of `books` of `run_id`
/// with `wall`'s filter, and the Wall marker of each.
fn read_walls(engine: &mut Attached, run_id: &str, wall: Option<bool>) -> Vec<(u64, Value)> {
    let mut params = json!({ "run_id": run_id, "dataset": "books" });
    if let Some(wall) = wall {
        params["wall"] = json!(wall);
    }
    engine.send(&json!({
        "jsonrpc": "2.0", "id": "records", "method": "run.records", "params": params,
    }));
    engine.send(&json!({
        "jsonrpc": "2.0", "method": "$/credit", "params": { "id": "records", "credits": 1000 },
    }));
    let mut read = Vec::new();
    loop {
        let message = engine.next().expect("the engine closed");
        if message["method"] == "$/record" {
            let record = &message["params"]["record"];
            read.push((
                record["number"].as_u64().unwrap(),
                record["provenance"]["wall"].clone(),
            ));
        } else if message["id"] == "records" {
            return read;
        }
    }
}

/// Where each format carries the Wall marker of the one Record in `bytes`.
fn wall_in(format: &str, bytes: Vec<u8>) -> Value {
    let marker = json!({ "kind": "paywall", "vendor": "v" });
    let text = || String::from_utf8(bytes.clone()).unwrap();
    let column = |header: &[String], row: &[String]| {
        let at = header.iter().position(|name| name == "#wall").unwrap();
        serde_json::from_str::<Value>(&row[at]).unwrap()
    };
    let got = match format {
        "json" => serde_json::from_slice::<Value>(&bytes).unwrap()[0]["#wall"].clone(),
        "jsonl" => serde_json::from_str::<Value>(text().trim_end()).unwrap()["#wall"].clone(),
        "csv" => {
            let rows = csv_rows(&bytes, b',');
            column(&rows[0], &rows[1])
        }
        "xlsx" => {
            let rows = xlsx_rows(bytes).1;
            column(&rows[0], &rows[1])
        }
        "html_table" => {
            let rows = html_rows(&text()).1;
            column(&rows[0], &rows[1])
        }
        "markdown" => {
            let rows = markdown_rows(&text());
            column(&rows[0], &rows[1])
        }
        "xml" => xml_rows(&bytes).2[0]["#wall"].clone(),
        "parquet" | "arrow_ipc" => {
            let rows = if format == "parquet" {
                parquet_rows(bytes).1
            } else {
                arrow_rows(bytes).1
            };
            serde_json::from_str(rows[0]["#wall"].as_str().unwrap()).unwrap()
        }
        "avro" => serde_json::from_str(avro_rows(&bytes).1[0]["#wall"].as_str().unwrap()).unwrap(),
        _ => {
            let (_, items) = feed_items(&bytes);
            return json!(items[0]["category"]);
        }
    };
    if format == "xml" {
        assert_eq!(got, marker);
        return json!("paywall");
    }
    assert_eq!(got, marker, "{format}");
    json!("paywall")
}

/// ADR 0052's acceptance test: a Record read past a Wall carries its Wall
/// marker in every Export format, as a `#wall` field or column, and as a
/// category in the feeds; `run.records` and `run.export` with `wall: true`
/// take only the Records read past a Wall, and with `false` only the
/// others.
///
/// Covers: story:6-274, operation:run.records, operation:run.export
#[test]
fn a_record_read_past_a_wall_carries_its_marker_in_every_format_and_filters() {
    let env = TestEnv::new();
    let mut engine = Attached::start(&env);
    crawl(&mut engine, "walls", 3);
    finish(&mut engine, "walls", book);
    mark_wall(&env, "walls", 2);
    let all = read_walls(&mut engine, "walls", None);
    let walled = read_walls(&mut engine, "walls", Some(true));
    let open = read_walls(&mut engine, "walls", Some(false));
    let mut carried = Vec::new();
    for format in FORMATS {
        let reply = exported(&mut engine, "walls", format, "none", Some(true));
        let bytes = saved(&mut engine, &env, &reply, &format!("walled.{format}"));
        carried.push((
            format,
            reply["result"]["records"].clone(),
            wall_in(format, bytes),
        ));
    }
    let others = exported(&mut engine, "walls", "jsonl", "none", Some(false));
    let others_bytes = saved(&mut engine, &env, &others, "others.jsonl");
    assert!(engine.finish());

    let marker = json!({ "kind": "paywall", "vendor": "v" });
    assert_eq!(
        (all, walled, open, String::from_utf8(others_bytes).unwrap()),
        (
            vec![(1, Value::Null), (2, marker.clone()), (3, Value::Null)],
            vec![(2, marker)],
            vec![(1, Value::Null), (3, Value::Null)],
            format!("{}\n{}\n", book(1), book(3)),
        )
    );
    for (format, records, wall) in carried {
        assert_eq!(
            (format, records, wall),
            (format, json!(1), json!("paywall"))
        );
    }
}

/// `export.json.indent` indents a JSON Export, `export.xml.root` and
/// `export.xml.record` name an XML Export's elements,
/// `export.parquet.compression` sets the Parquet codec, and the
/// `export.feed.*` settings name the fields a feed reads; an indent above
/// 8, a root that is no XML name and an empty feed field fail with
/// `setting_invalid` naming the key.
///
/// Covers: story:6-279, operation:run.export, setting:export.json.indent, setting:export.xml.root, setting:export.xml.record, setting:export.parquet.compression, setting:export.feed.title, setting:export.feed.link, setting:export.feed.published, setting:export.feed.summary, error:setting_invalid
#[test]
fn the_export_settings_shape_json_xml_parquet_and_the_feeds() {
    let env = TestEnv::new();
    let mut engine = Attached::start(&env);
    crawl(&mut engine, "shapes", 2);
    finish(&mut engine, "shapes", |page| {
        json!({
            "name": format!("Livre {page}"),
            "href": format!("https://shop.test/{page}"),
            "date": "2026-01-02T08:00:00Z",
            "text": "Résumé",
        })
    });
    let json_reply = export_under(
        &mut engine,
        "shapes",
        "json",
        &json!({ "export": { "json": { "indent": 2 } } }),
    );
    let xml_reply = export_under(
        &mut engine,
        "shapes",
        "xml",
        &json!({ "export": { "xml": { "root": "livres", "record": "livre" } } }),
    );
    let mut codecs = Vec::new();
    for codec in ["zstd", "snappy", "none"] {
        let reply = export_under(
            &mut engine,
            "shapes",
            "parquet",
            &json!({ "export": { "parquet": { "compression": codec } } }),
        );
        let bytes = saved(&mut engine, &env, &reply, &format!("books.{codec}.parquet"));
        let metadata = parquet::file::reader::FileReader::metadata(
            &parquet::file::serialized_reader::SerializedFileReader::new(bytes::Bytes::from(bytes))
                .unwrap(),
        )
        .row_group(0)
        .column(0)
        .compression()
        .to_string();
        codecs.push(metadata);
    }
    let fields = json!({ "export": { "feed": {
        "title": "name", "link": "href", "published": "date", "summary": "text",
    } } });
    let atom_reply = export_under(&mut engine, "shapes", "atom", &fields);
    let refusals = [
        ("json", json!({ "export": { "json": { "indent": 9 } } })),
        ("xml", json!({ "export": { "xml": { "root": "1livres" } } })),
        ("rss", json!({ "export": { "feed": { "title": "" } } })),
    ]
    .map(|(format, settings)| {
        let reply = export_under(&mut engine, "shapes", format, &settings);
        (
            reply["error"]["data"]["code"].clone(),
            reply["error"]["data"]["key"].clone(),
        )
    });
    let json_bytes = saved(&mut engine, &env, &json_reply, "books.json");
    let xml_bytes = saved(&mut engine, &env, &xml_reply, "books.xml");
    let atom_bytes = saved(&mut engine, &env, &atom_reply, "books.atom");
    assert!(engine.finish());

    let values: Value = serde_json::from_slice(&json_bytes).unwrap();
    let (root, names, _) = xml_rows(&xml_bytes);
    let (_, entries) = feed_items(&atom_bytes);
    assert_eq!(
        (
            String::from_utf8(json_bytes).unwrap(),
            (root, names),
            codecs,
            entries[0].clone(),
            refusals,
        ),
        (
            serde_json::to_string_pretty(&values).unwrap(),
            (
                "livres".to_owned(),
                vec!["livre".to_owned(), "livre".to_owned()]
            ),
            vec![
                "ZSTD(ZstdLevel(1))".to_owned(),
                "SNAPPY".to_owned(),
                "UNCOMPRESSED".to_owned()
            ],
            json!({
                "id": "https://shop.test/1",
                "title": "Livre 1",
                "link": "https://shop.test/1",
                "published": "2026-01-02T08:00:00Z",
                "updated": "2026-01-02T08:00:00Z",
                "summary": "Résumé",
            }),
            [
                (json!("setting_invalid"), json!("export.json.indent")),
                (json!("setting_invalid"), json!("export.xml.root")),
                (json!("setting_invalid"), json!("export.feed.title")),
            ],
        )
    );
}
