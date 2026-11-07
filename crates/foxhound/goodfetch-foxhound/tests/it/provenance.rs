//! Dataset and Record conformance cases of a Record's Provenance and of
//! Secret masking, on Foxhound's executable over stdio (spec #581's seam:
//! the protocol; ADRs 0045, 0047, 0052, 0092, 0103, 0119 and 0123): every
//! Record `run.records` reads names in its Provenance the crawl request
//! that produced it, the tier its document was fetched on and how that
//! tier was chosen, the encoding the document was read in and where it
//! came from, and the Clean-up rules the Run holds to; it names no Wall,
//! payment, Response cache or Response recording where none was met, and
//! no Value transform where the handler parsed its own values. A Record
//! pushed holding a value the Run resolved is kept as `%name%`, in the
//! Dataset and in its rejection table alike, and the Secret canary is
//! found nowhere in the Run's folder nor in an Export of the Dataset,
//! JSON, a gzip CSV or an XLSX workbook (ADR 0067).
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::collections::BTreeSet;

use goodfetch_test_client::HomeLayout;
use goodfetch_test_support::{Canaries, Output, ProvingGround, SecretKind, TestEnv};
use rusqlite::Connection;
use serde_json::{Value, json};

use crate::crawl::Attached;
use crate::exports::save_as;
use crate::fetch_tiers::{provenance, settings};
use crate::frontier::{Inside, crawl_from};

/// The Proving ground's seed.
const SEED: u64 = 972;

/// The `encodings` family's host.
const ENCODINGS: &str = "encodings.test";

/// The variable the Run's key-encryption key is read from.
const KEY_VARIABLE: &str = "GF_TEST_RUN_KEY";

/// `ctx.push(dataset, record)` from the attempt `handle` gave: its reply.
pub(crate) fn push(inside: &mut Inside<'_>, handle: &Value, dataset: &str, data: &Value) -> Value {
    inside.call(
        "crawl_request.push",
        &json!({
            "run_id": handle["run_id"],
            "crawl_request_id": handle["crawl_request"]["id"],
            "lease_epoch": handle["lease_epoch"],
            "dataset": dataset,
            "data": data,
        }),
    )
}

/// Every Record of `dataset` of the Run `run_id`, as `run.records` reads
/// them, granted credit for all at once.
pub(crate) fn records(engine: &mut Attached, run_id: &str, dataset: &str) -> Vec<Value> {
    engine.send(&json!({
        "jsonrpc": "2.0", "id": "records", "method": "run.records",
        "params": { "run_id": run_id, "dataset": dataset },
    }));
    engine.send(&json!({
        "jsonrpc": "2.0", "method": "$/credit", "params": { "id": "records", "credits": 1000 },
    }));
    let mut read = Vec::new();
    loop {
        let message = engine
            .next()
            .expect("the engine closed before the reader answered");
        if message["method"] == "$/record" {
            read.push(message["params"]["record"].clone());
        } else if message["id"] == "records" {
            assert!(message.get("result").is_some(), "{message}");
            return read;
        }
    }
}

/// Every Record of a crawl's Dataset names its crawl request, its tier
/// and how it was chosen, its document's encoding and where that came
/// from, and the Clean-up rules version the Run holds to, which its Run
/// trace names; with no Wall, payment, replayed response or Value
/// transform met, it names none.
///
/// Covers: story:6-273, story:6-265, operation:run.records, setting:fetch.tier
#[test]
fn every_record_names_its_request_tier_encoding_and_clean_up_rules_in_its_provenance() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = Attached::start(&env);
    let seeds: Vec<Value> = ["shift-jis", "utf-16-bom"]
        .iter()
        .map(|page| {
            json!({
                "url": proving_ground.url(ENCODINGS, &format!("/pages/{page}")),
                "label": "page",
            })
        })
        .collect();
    let settings = settings(
        &proving_ground,
        &json!({
            "fetch": { "tier": "http" },
            "politeness": { "domain": { "delay": 0 } },
            "site_preferences": { "respect_robots_txt": false },
        }),
    );

    let finished = crawl_from(
        &mut engine,
        &json!({
            "run_id": "traced",
            "seeds": seeds,
            "settings": settings,
            "datasets": [{ "name": "pages" }],
        }),
        &mut |inside, handle| {
            let url = handle["crawl_request"]["url"].clone();
            let pushed = push(inside, handle, "pages", &json!({ "url": url }));
            assert!(pushed.get("result").is_some(), "{pushed}");
            json!({ "result": { "records": [] } })
        },
    );
    assert_eq!(finished["result"]["lifecycle"], "finished", "{finished}");
    let read = records(&mut engine, "traced", "pages");
    assert!(engine.finish());

    let clean_up = provenance(&env, "traced", "clean_up_rules");
    let digest = &clean_up.first().expect("the Run names its Clean-up rules")["digest"];
    assert!(
        digest
            .as_str()
            .is_some_and(|digest| digest.starts_with("sha256:")),
        "{clean_up:?}"
    );
    let mut named: Vec<Value> = read
        .iter()
        .map(|record| {
            let url = record["value"]["url"].as_str().unwrap();
            json!({
                "page": url.rsplit('/').next(),
                "names_its_request": record["provenance"]["request_id"] == record["request_id"]
                    && record["request_id"].is_string(),
                "provenance": {
                    "tier": record["provenance"]["tier"],
                    "encoding": record["provenance"]["encoding"],
                    "clean_up_rules": record["provenance"]["clean_up_rules"],
                    "origin": record["provenance"].get("origin"),
                    "wall": record["provenance"].get("wall"),
                    "payment": record["provenance"].get("payment"),
                    "value_transforms": record["provenance"]["value_transforms"],
                },
            })
        })
        .collect();
    named.sort_by_key(|named| named["page"].as_str().unwrap_or_default().to_owned());
    let forced = json!({ "tier": "http", "chosen": "forced", "forced_by": "run" });
    assert_eq!(
        named,
        [
            (
                "shift-jis",
                json!({ "encoding": "Shift_JIS", "source": "header" })
            ),
            (
                "utf-16-bom",
                json!({ "encoding": "UTF-16LE", "source": "bom" })
            ),
        ]
        .map(|(page, encoding)| json!({
            "page": page,
            "names_its_request": true,
            "provenance": {
                "tier": forced,
                "encoding": encoding,
                "clean_up_rules": digest,
                "origin": null,
                "wall": null,
                "payment": null,
                "value_transforms": [],
            },
        }))
        .to_vec()
    );
}

/// The Run file of the Run `run_id`.
fn run_file(env: &TestEnv, run_id: &str) -> Connection {
    Connection::open(HomeLayout::of(env.home()).run_file(run_id)).unwrap()
}

/// The Export of `dataset` of `run_id` in `format`, compressed as
/// `compression` says, saved in the test's folder: its bytes.
fn saved_export(
    env: &TestEnv,
    engine: &mut Attached,
    (run_id, dataset): (&str, &str),
    format: &str,
    compression: &str,
) -> Vec<u8> {
    let exported = engine.call(
        "export",
        "run.export",
        &json!({ "run_id": run_id, "dataset": dataset, "format": format, "compression": compression }),
        &mut |_| None,
    );
    let path = env
        .folder()
        .join(format!("{dataset}.{format}.{compression}"));
    let saved = save_as(engine, &exported["result"]["export_id"], &path);
    assert!(saved.get("result").is_some(), "{exported}\n{saved}");
    #[expect(
        clippy::disallowed_methods,
        reason = "the test reads the Export the engine saved"
    )]
    std::fs::read(&path).unwrap()
}

/// The text of every part of the ZIP `bytes`, such as an XLSX workbook's
/// sheets and shared strings, decompressed, each named by its path.
fn zip_parts(bytes: Vec<u8>) -> Vec<(String, Vec<u8>)> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    (0..archive.len())
        .map(|index| {
            let mut part = archive.by_index(index).unwrap();
            let mut text = Vec::new();
            std::io::Read::read_to_end(&mut part, &mut text).unwrap();
            (part.name().unwrap().into_owned(), text)
        })
        .collect()
}

/// The Exports of `dataset` as JSON, as a CSV compressed with gzip and as
/// an XLSX workbook, read decompressed: the JSON file, the CSV text and
/// each part of the workbook, each named, with the format it came from.
fn decompressed_exports(
    env: &TestEnv,
    engine: &mut Attached,
    dataset: (&str, &str),
) -> Vec<(&'static str, String, Vec<u8>)> {
    let json_export = saved_export(env, engine, dataset, "json", "none");
    let csv_export = saved_export(env, engine, dataset, "csv", "gzip");
    let xlsx_export = saved_export(env, engine, dataset, "xlsx", "none");
    let mut csv_text = Vec::new();
    std::io::Read::read_to_end(
        &mut flate2::read::GzDecoder::new(csv_export.as_slice()),
        &mut csv_text,
    )
    .unwrap();
    let mut exports = vec![
        ("json", format!("{}.json", dataset.1), json_export),
        ("csv", format!("{}.csv", dataset.1), csv_text),
    ];
    exports.extend(
        zip_parts(xlsx_export)
            .into_iter()
            .map(|(name, text)| ("xlsx", name, text)),
    );
    exports
}

/// A Record pushed holding a value the Run resolved, here its
/// key-encryption key, which `run_state.key_reference` names, is kept as
/// `%run_state.key_reference%` in its Dataset, as is one its schema
/// refused in the Dataset's rejection table, and as is a Record of the
/// handler's answer; `run.records` reads it so, and the canary is found
/// nowhere in the Run's folder nor in the Dataset's Exports: JSON, a CSV
/// compressed with gzip, and an XLSX workbook, each part of it read
/// decompressed, each holding the placeholder instead (ticket #972).
///
/// Covers: story:6-275, operation:crawl_request.push, operation:run.records, operation:run.export, operation:export.save_as
#[test]
fn a_record_holding_a_resolved_secret_value_is_kept_with_its_placeholder() {
    let env = TestEnv::new();
    let mut canaries = Canaries::new(env.seed());
    let key = canaries.plant(SecretKind::RunDataKey).value().to_owned();
    env.user_configuration(&format!(
        "run_state.key_reference = \"env:{KEY_VARIABLE}\"\n"
    ));
    let mut engine = Attached::start_with(&env, None, |command| {
        command.env(KEY_VARIABLE, &key);
    });
    let said = format!("the key is {key}");

    let finished = crawl_from(
        &mut engine,
        &json!({
            "run_id": "reflected",
            "seeds": [{ "url": "https://shop.test/", "label": "page" }],
            "datasets": [{
                "name": "pages",
                "schema": { "type": "object", "properties": { "count": { "type": "number" } } },
            }],
        }),
        &mut |inside, handle| {
            let pushed = push(inside, handle, "pages", &json!({ "said": said }));
            assert!(pushed.get("result").is_some(), "{pushed}");
            let refused = push(inside, handle, "pages", &json!({ "count": said }));
            assert_eq!(
                refused["error"]["data"]["code"], "record_invalid",
                "{refused}"
            );
            json!({ "result": { "records": [{ "dataset": "pages", "data": { "answered": said } }] } })
        },
    );
    assert_eq!(finished["result"]["lifecycle"], "finished", "{finished}");
    let read: Vec<Value> = records(&mut engine, "reflected", "pages")
        .into_iter()
        .map(|record| record["value"].clone())
        .collect();
    let exports = decompressed_exports(&env, &mut engine, ("reflected", "pages"));
    assert!(engine.finish());
    let holding: BTreeSet<&str> = exports
        .iter()
        .filter(|(_, _, bytes)| {
            String::from_utf8_lossy(bytes).contains("%run_state.key_reference%")
        })
        .map(|(format, _, _)| *format)
        .collect();
    assert_eq!(holding, BTreeSet::from(["csv", "json", "xlsx"]));
    let exported: Vec<Output<'_>> = exports
        .iter()
        .map(|(_, name, bytes)| Output::Bytes { name, bytes })
        .collect();
    assert_eq!(canaries.leaks(&exported).unwrap(), Vec::new());

    let placeholder = "the key is %run_state.key_reference%";
    let kept: Vec<String> = run_file(&env, "reflected")
        .prepare("SELECT data FROM records ORDER BY number")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let rejected: Vec<String> = run_file(&env, "reflected")
        .prepare("SELECT data FROM rejected_records ORDER BY number")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        (
            read,
            kept,
            rejected,
            canaries
                .leaks(&[Output::Path(&HomeLayout::of(env.home()).runs())])
                .unwrap()
        ),
        (
            vec![
                json!({ "said": placeholder }),
                json!({ "answered": placeholder })
            ],
            vec![
                json!({ "said": placeholder }).to_string(),
                json!({ "answered": placeholder }).to_string(),
            ],
            vec![json!({ "count": placeholder }).to_string()],
            Vec::new(),
        )
    );
}
