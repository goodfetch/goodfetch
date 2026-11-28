//! Dataset Exports through the Rust SDK (spec #611's #1488; ADRs 0103, 0116
//! and 0119): `run.export(&dataset, format)` returns an `Export` once the
//! file is whole; `save_as` writes it through a hidden temporary file,
//! creating missing folders, replacing existing files, and refusing unwritable
//! paths with `path_not_writable` without leaving a partial file; `stream()`
//! yields the exact same bytes; CSV has no BOM by default and carries one
//! under `export.csv.bom`; all text formats preserve UTF-8.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "tests read and inspect exported files outside any async worker"
)]

use futures_util::TryStreamExt as _;
use goodfetch_foxhound::settings::{Settings, export as export_settings};
use goodfetch_foxhound::{Crawl, CrawlDeclaration, ExportFormat, Foxhound, RunId, Seed, blocking};
use goodfetch_protocol::result_document::OracleOutcome;
use goodfetch_protocol::run::Lifecycle;
use goodfetch_test_support::{ProvingGround, TestEnv, ToolStatus, real_time_runtime};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use super::catalog::{SEED, TASK, judged};
use super::crawl::options;

/// Every Export format `run.export` supports.
const ALL_FORMATS: [ExportFormat; 12] = [
    ExportFormat::Json,
    ExportFormat::Jsonl,
    ExportFormat::Csv,
    ExportFormat::Xlsx,
    ExportFormat::Xml,
    ExportFormat::Parquet,
    ExportFormat::HtmlTable,
    ExportFormat::Rss,
    ExportFormat::Atom,
    ExportFormat::Avro,
    ExportFormat::ArrowIpc,
    ExportFormat::Markdown,
];

/// UTF-8 Byte Order Mark.
const UTF8_BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];

/// Lowercase hexadecimal SHA-256 of `bytes`.
fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut hex, "{byte:02x}").unwrap();
    }
    hex
}

/// The catalog crawled as data, filling `products`.
fn catalog_declaration() -> CrawlDeclaration {
    let css = |selector: &str| json!({ "kind": "css", "selector": selector });
    serde_json::from_value(json!({
        "labels": [
            {
                "name": "listing",
                "follow": [
                    { "links": "a[rel=next]", "label": "listing" },
                    { "links": "li.product a", "label": "product" },
                ],
            },
            {
                "name": "product",
                "extract": [{
                    "dataset": "products",
                    "fields": {
                        "id": { "locator": css(".id") },
                        "name": { "locator": css("h1.name") },
                        "price": { "locator": css(".price"), "attribute": "data-price" },
                    },
                }],
            },
        ],
        "datasets": [{ "name": "products", "key": ["id"] }],
        "settings": { "politeness": { "domain": { "delay": 0 } } },
    }))
    .unwrap()
}

/// Exporting the crawled catalog Dataset in every format returns a descriptor
/// whose `size`, `records`, `last_number` and `sha256` match the saved file
/// and the streamed bytes; the JSON export passes the Proving ground oracle.
///
/// Covers: story:6-276, operation:run.export, operation:export.save_as, operation:export.stream
#[test]
fn export_in_every_format_matches_saved_file_stream_and_oracle() {
    let env = TestEnv::new();
    let site = ProvingGround::start(&env, SEED).unwrap();
    let start = site.start_url(TASK).unwrap();
    let out_dir = env.folder().join("exports");

    let (lifecycle, json_records) = real_time_runtime().block_on(async {
        let foxhound = Foxhound::connect(options(&env, &site)).await.unwrap();
        let mut handle_crawl = Crawl::new();
        let products = handle_crawl.dataset::<Value>("products").unwrap();
        let run = foxhound
            .start_crawl(catalog_declaration(), [Seed::named(&start, "listing")])
            .run_id(&RunId::parse("export-all").unwrap())
            .await
            .unwrap();
        let report = run.finish().await.unwrap().into_inner();

        let mut json_records = Vec::new();
        for format in ALL_FORMATS {
            let export = run.export(&products, format).await.unwrap();
            assert_eq!(export.format(), format);
            assert!(export.records() > 0, "{format:?} exported zero records");
            assert_eq!(export.last_number(), export.records());

            let path = out_dir.join(format!("{format:?}/nested/products.out"));
            let saved = export.save_as(&path).await.unwrap();
            let file_bytes = std::fs::read(&path).unwrap();
            let streamed: Vec<u8> = export
                .stream()
                .try_collect::<Vec<Vec<u8>>>()
                .await
                .unwrap()
                .into_iter()
                .flatten()
                .collect();

            let file_sha = sha256_hex(&file_bytes);
            assert_eq!(
                (
                    export.size(),
                    export.sha256(),
                    saved.size,
                    saved.sha256.as_str(),
                    streamed == file_bytes,
                ),
                (
                    file_bytes.len() as u64,
                    file_sha.as_str(),
                    export.size(),
                    export.sha256(),
                    true,
                ),
                "format {format:?} mismatch"
            );
            if format == ExportFormat::Json {
                json_records = serde_json::from_slice(&file_bytes).unwrap();
            }
        }
        foxhound.close().await.unwrap();
        (report.report.lifecycle, json_records)
    });

    let answer = judged(&site, ToolStatus::Success, &json_records);
    assert_eq!(
        (lifecycle, answer.outcome),
        (Lifecycle::Finished, OracleOutcome::Verified),
        "{answer:?}"
    );
}

/// CSV has no BOM by default and carries a UTF-8 BOM under `export.csv.bom`;
/// all text Exports are UTF-8 and preserve non-ASCII text; `save_as` creates
/// missing folders, replaces existing files whole, and refuses unwritable
/// paths with `path_not_writable` leaving no partial file.
///
/// Covers: story:6-277, operation:run.export, operation:export.save_as, operation:export.stream
#[test]
fn csv_bom_utf8_and_save_as_file_guarantees_hold() {
    let env = TestEnv::new();
    let site = ProvingGround::start(&env, SEED).unwrap();
    let start = site.start_url("extraction/7").unwrap();
    let out_dir = env.folder().join("utf8-exports");

    real_time_runtime().block_on(async {
        let foxhound = Foxhound::connect(options(&env, &site)).await.unwrap();
        let mut crawl = Crawl::new();
        let page = crawl.label::<()>("page").unwrap();
        let notes = crawl.dataset::<Value>("notes").unwrap();
        let notes_handle = notes.clone();
        crawl
            .handle(&page, move |ctx| {
                let notes = notes_handle.clone();
                async move {
                    ctx.push(&notes, &json!({ "title": "Café — 日本語", "ok": true }))
                        .await?;
                    Ok(())
                }
            })
            .unwrap();
        let run = foxhound
            .start_crawl(&crawl, [Seed::new(&start, &page, ())])
            .run_id(&RunId::parse("utf8-bom").unwrap())
            .await
            .unwrap();
        crawl.serve(&run).await.unwrap();

        // Default CSV: no BOM, valid UTF-8 with non-ASCII preserved.
        let csv_default = run.export(&notes, ExportFormat::Csv).await.unwrap();
        let csv_path = out_dir.join("deep/sub/notes.csv");
        csv_default.save_as(&csv_path).await.unwrap();
        let csv_bytes = std::fs::read(&csv_path).unwrap();
        assert!(!csv_bytes.starts_with(&UTF8_BOM));
        assert!(
            std::str::from_utf8(&csv_bytes)
                .unwrap()
                .contains("Café — 日本語")
        );

        // With `export.csv.bom = true`: starts with UTF-8 BOM and replaces existing file.
        let csv_bom = run
            .export(&notes, ExportFormat::Csv)
            .settings(Settings::new().set(export_settings::csv::BOM, true))
            .await
            .unwrap();
        csv_bom.save_as(&csv_path).await.unwrap();
        let bom_bytes = std::fs::read(&csv_path).unwrap();
        assert!(bom_bytes.starts_with(&UTF8_BOM));
        assert!(
            std::str::from_utf8(&bom_bytes[3..])
                .unwrap()
                .contains("Café — 日本語")
        );

        // Other text Exports are UTF-8 without BOM and round-trip non-ASCII text.
        for format in [
            ExportFormat::Json,
            ExportFormat::Jsonl,
            ExportFormat::Xml,
            ExportFormat::HtmlTable,
            ExportFormat::Markdown,
        ] {
            let export = run.export(&notes, format).await.unwrap();
            let bytes: Vec<u8> = export
                .stream()
                .try_collect::<Vec<Vec<u8>>>()
                .await
                .unwrap()
                .into_iter()
                .flatten()
                .collect();
            assert!(
                !bytes.starts_with(&UTF8_BOM),
                "{format:?} had unexpected BOM"
            );
            let text = std::str::from_utf8(&bytes).unwrap();
            assert!(
                text.contains("Café — 日本語"),
                "{format:?} lost UTF-8: {text}"
            );
        }

        // Unwritable path (a directory) fails with `path_not_writable`, not retryable,
        // and leaves no partial file behind.
        let dir_target = out_dir.join("is-a-dir");
        std::fs::create_dir_all(&dir_target).unwrap();
        let refused = csv_default.save_as(&dir_target).await.unwrap_err();
        assert_eq!(refused.code().as_str(), "path_not_writable", "{refused}");
        assert!(!refused.retryable());
        let partials: Vec<String> = std::fs::read_dir(&out_dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains(".partial"))
            .collect();
        assert!(partials.is_empty(), "left partial files: {partials:?}");

        foxhound.close().await.unwrap();
    });
}

/// The blocking SDK's `export`, `save_as` and `stream` match their async twins.
///
/// Covers: story:6-276, operation:run.export, operation:export.save_as, operation:export.stream
#[test]
fn blocking_export_save_as_and_stream_match() {
    let env = TestEnv::new();
    let site = ProvingGround::start(&env, SEED).unwrap();
    let start = site.start_url("extraction/7").unwrap();
    let out = env.folder().join("blocking/out.jsonl");

    let foxhound = blocking::Foxhound::connect(options(&env, &site)).unwrap();
    let mut crawl = blocking::Crawl::new();
    let page = crawl.label::<()>("page").unwrap();
    let notes = crawl.dataset::<Value>("notes").unwrap();
    let notes_handle = notes.clone();
    crawl
        .handle(&page, move |ctx| {
            ctx.push(&notes_handle, &json!({ "title": "Blocking Café" }))?;
            Ok(())
        })
        .unwrap();
    let run = foxhound
        .start_crawl(&crawl, [Seed::new(&start, &page, ())])
        .run_id(&RunId::parse("blocking-export").unwrap())
        .send()
        .unwrap();
    crawl.serve(&run).unwrap();

    let export = run.export(&notes, ExportFormat::Jsonl).send().unwrap();
    let saved = export.save_as(&out).send().unwrap();
    let file_bytes = std::fs::read(&out).unwrap();
    let streamed: Vec<u8> = export.stream().flat_map(Result::unwrap).collect();
    foxhound.close().unwrap();

    assert_eq!(
        (
            export.records(),
            export.size(),
            saved.sha256.as_str(),
            streamed == file_bytes,
            std::str::from_utf8(&file_bytes)
                .unwrap()
                .contains("Blocking Café"),
        ),
        (1, file_bytes.len() as u64, export.sha256(), true, true)
    );
}
