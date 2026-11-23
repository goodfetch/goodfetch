//! J-002, "Crawl a site over HTTP to Markdown and export its Dataset as
//! JSON, CSV and Parquet", on the `cli` surface (ADRs 0063 and 0099;
//! tickets #1512 and #1513): a person runs Foxhound's shipped executable,
//! starts a code-free crawl of the Proving ground's catalog with `crawl
//! start`, reading each page's Markdown and, from a `--fields` file, each
//! product's `id`, `name` and `price`, follows its Records with `run
//! records --follow`, and exports its Dataset with `run export --out` into
//! files of their own, all through the Daemon of the variant's home.
#![expect(
    clippy::disallowed_methods,
    reason = "the person's steps write the settings file and read the Exports' files"
)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use goodfetch_protocol::result_document::ResultDocument;
use goodfetch_test_support::{JourneyRun, cli as command_line};
use serde_json::{Value, json};

use crate::crawl_commands::{settings_file, short_idle};
use crate::export_formats::parquet_rows;

const FOXHOUND: &str = env!("CARGO_BIN_EXE_goodfetch-foxhound");

/// The crawl's Run.
const RUN: &str = "catalog";

/// The fields each product's Record holds, read from its product page.
const PRODUCT_FIELDS: [&str; 3] = ["id", "name", "price"];

/// The named fields the crawl reads from each page beside its Markdown,
/// as `--fields` takes them: each product's, by CSS selector, the price
/// from its machine-readable attribute.
pub(crate) fn fields_read() -> Value {
    json!({
        "id": { "locator": { "kind": "css", "selector": "article.product .id" } },
        "name": { "locator": { "kind": "css", "selector": "article.product .name" } },
        "price": {
            "locator": { "kind": "css", "selector": "article.product .price" },
            "attribute": "data-price",
        },
    })
}

/// The products among `records`, a Dataset's Records as `run.records`
/// reads them: each product page's `id`, `name` and `price`, as the
/// oracle takes them; a listing page's Record has no `id`.
pub(crate) fn products(records: &[Value]) -> Vec<Value> {
    records
        .iter()
        .filter(|record| record["value"]["id"].is_string())
        .map(|record| {
            PRODUCT_FIELDS
                .iter()
                .map(|field| ((*field).to_owned(), record["value"][*field].clone()))
                .collect::<serde_json::Map<String, Value>>()
                .into()
        })
        .collect()
}

/// The file `run export --out` writes the Export of `format` to, in the
/// person's folder.
fn export_file(folder: &Path, format: &str) -> PathBuf {
    folder.join(format!("catalog.{format}"))
}

/// Covers: story:11-1130, story:11-1132, row:SA-01, row:CO-18, row:CO-25, row:CO-27, row:CO-29, story:1-1, story:1-2, story:1-3
#[test]
#[expect(
    clippy::too_many_lines,
    reason = "a Journey variant reads as its file: each step, then each check, in order"
)]
fn cli() {
    let mut run = JourneyRun::start("J-002", "cli", Path::new(FOXHOUND), ResultDocument::journey)
        .expect("J-002's cli variant starts");
    let mut version = String::new();
    let mut followed: Vec<Value> = Vec::new();
    let mut committed: Vec<Value> = Vec::new();
    let mut outcomes: Vec<Value> = Vec::new();
    let mut exports: Vec<(String, Value)> = Vec::new();
    short_idle(run.env());

    run.step(
        "Install Foxhound into a fresh project from the commit's packages alone, with install scripts ignored: the surface's SDK package, or Foxhound's executable on the other surfaces",
        |run| {
            let output = run.command().arg("--version").output().unwrap();
            assert!(output.status.success(), "{output:?}");
            version = String::from_utf8(output.stdout).unwrap();
        },
    );
    run.step(
        "Crawl the catalog over HTTP with no browser, from its first listing page, turning each page into Markdown",
        |run| {
            let proving_ground = run.proving_ground().expect("J-002 has a Proving ground");
            let settings = settings_file(run.env(), proving_ground);
            let start = proving_ground.start_url("catalog/1").unwrap();
            let fields = run.env().folder().join("fields.json");
            std::fs::write(&fields, fields_read().to_string()).unwrap();
            let started = command_line(run.env(), run.engine())
                .args(["crawl", "start", &start, "--run-id", RUN])
                .args(["--format", "markdown", "--links", "scope"])
                .args(["--fields", fields.to_str().unwrap()])
                .args(["--settings", settings.to_str().unwrap(), "--json"])
                .json("RunId")
                .unwrap();
            assert_eq!(started.documents[0], RUN, "{}", started.output);
        },
    );
    run.step("Read the crawl's Records as they arrive", |run| {
        let read = command_line(run.env(), run.engine())
            .args([
                "run",
                "records",
                RUN,
                "--dataset",
                "pages",
                "--follow",
                "--json",
            ])
            .json_lines("RecordRead")
            .unwrap();
        followed = read
            .documents
            .iter()
            .map(|read| read["record"].clone())
            .collect();
        let ended = command_line(run.env(), run.engine())
            .args(["run", "outcomes", RUN, "--follow", "--json"])
            .json_lines("CrawlRequestOutcome")
            .unwrap();
        outcomes = ended.documents;
    });
    run.step(
        "Export the crawl's Dataset as JSON, as CSV and as Parquet",
        |run| {
            for format in ["json", "csv", "parquet"] {
                let out = export_file(run.env().folder(), format);
                let export = command_line(run.env(), run.engine())
                    .args(["run", "export", RUN, "pages", "--format", format])
                    .args(["--out", out.to_str().unwrap(), "--json"])
                    .json("Export")
                    .unwrap();
                exports.push((format.to_owned(), export.documents[0].clone()));
            }
            let read = command_line(run.env(), run.engine())
                .args(["run", "records", RUN, "--dataset", "pages", "--json"])
                .json_array("RecordRead")
                .unwrap();
            committed = read.documents[0]
                .as_array()
                .unwrap()
                .iter()
                .map(|read| read["record"].clone())
                .collect();
        },
    );

    run.check(
        "The installed package runs, and reports the commit's Product version, the one Foxhound's executable and every SDK package report",
        |_| {
            assert_eq!(
                version.trim(),
                format!("goodfetch-foxhound {}", env!("CARGO_PKG_VERSION"))
            );
        },
    );
    run.expect_task(
        "The oracle verifies the crawl's Records",
        &json!({ "status": "success", "records": products(&committed) }),
    );
    run.check(
        "The crawl fetched every page over HTTP and launched no browser",
        |_| {
            let text = |value: &Value| value.as_str().unwrap_or_default().to_owned();
            let tiers: BTreeSet<String> = committed
                .iter()
                .map(|record| text(&record["provenance"]["tier"]["tier"]))
                .collect();
            let kinds: BTreeSet<String> = outcomes
                .iter()
                .map(|outcome| text(&outcome["kind"]))
                .collect();
            assert_eq!(
                (tiers, kinds, committed.len() == outcomes.len()),
                (
                    BTreeSet::from(["http".to_owned()]),
                    BTreeSet::from(["succeeded".to_owned()]),
                    true,
                ),
                "{outcomes:?}"
            );
        },
    );
    run.check("Each Record holds its page's Markdown", |_| {
        assert!(
            !committed.is_empty()
                && committed.iter().all(|record| record["value"]["markdown"]
                    .as_str()
                    .is_some_and(|markdown| !markdown.is_empty())),
            "{committed:?}"
        );
    });
    run.check(
        "The Records read as they arrived are the Dataset's, in order from its first, each once",
        |_| {
            let numbers: Vec<Value> = followed
                .iter()
                .map(|record| record["number"].clone())
                .collect();
            let expected: Vec<Value> = (1..=committed.len()).map(|number| json!(number)).collect();
            assert_eq!((numbers, &followed), (expected, &committed));
        },
    );
    run.check(
        "The JSON, CSV and Parquet Exports each hold every Record of the Dataset, with every field",
        |run| {
            let values: Vec<Value> = committed
                .iter()
                .map(|record| record["value"].clone())
                .collect();
            let fields: BTreeSet<String> = values
                .iter()
                .flat_map(|value| value.as_object().unwrap().keys().cloned())
                .collect();
            let folder = run.env().folder();
            let mut held = Vec::new();
            for (format, export) in &exports {
                let bytes = std::fs::read(export_file(folder, format)).unwrap();
                assert_eq!(json!(bytes.len()), export["size"], "{format}");
                let (count, columns): (usize, BTreeSet<String>) = match format.as_str() {
                    "json" => {
                        let exported: Vec<Value> = serde_json::from_slice(&bytes).unwrap();
                        assert_eq!(exported, values);
                        (exported.len(), fields.clone())
                    }
                    "csv" => {
                        let mut reader = csv::Reader::from_reader(bytes.as_slice());
                        let columns = reader
                            .headers()
                            .unwrap()
                            .iter()
                            .map(str::to_owned)
                            .collect();
                        (reader.records().count(), columns)
                    }
                    _ => {
                        let (columns, rows) = parquet_rows(bytes);
                        (
                            rows.len(),
                            columns.into_iter().map(|(name, _)| name).collect(),
                        )
                    }
                };
                held.push((format.clone(), export["records"].clone(), count, columns));
            }
            let expected: Vec<_> = ["json", "csv", "parquet"]
                .iter()
                .map(|format| {
                    (
                        (*format).to_owned(),
                        json!(values.len()),
                        values.len(),
                        fields.clone(),
                    )
                })
                .collect();
            assert_eq!(held, expected);
        },
    );
    // The Daemon the commands started stops before the home goes.
    let _ = command_line(run.env(), FOXHOUND)
        .args(["daemon", "stop"])
        .run();
    run.finish()
        .expect("J-002's document is written")
        .assert_proved();
}
