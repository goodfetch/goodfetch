//! The published Operation catalog's writer for the Compatibility corpus
//! (ADR 0066; issue 1662): a compact representative `PublishedCatalog` carrying
//! both Products' marks, Connection framing types and `$defs`, built from the
//! protocol's types, read back through [`PublishedCatalog::read`], and written
//! into the folder given as its only argument as `catalog.json`.
#![expect(
    clippy::disallowed_methods,
    reason = "a synchronous corpus writer that writes files (ADR 0066)"
)]

use std::collections::BTreeMap;
use std::path::PathBuf;

use goodfetch_protocol::catalog::PROTOCOL_VERSION;
use goodfetch_protocol::published_catalog::{
    PUBLISHED_CATALOG_VERSION, PublishedCatalog, SCHEMA_DRAFT,
};
use serde_json::json;

fn main() {
    let output = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .expect("invariant: xtask compat passes the output folder"),
    );
    let document = PublishedCatalog {
        format_version: PUBLISHED_CATALOG_VERSION,
        protocol_version: PROTOCOL_VERSION,
        schema: SCHEMA_DRAFT.to_owned(),
        catalog: BTreeMap::from([
            (
                "operations".to_owned(),
                json!([
                    {
                        "name": "page.goto",
                        "product": "collie",
                        "params": { "$ref": "#/$defs/GotoParams" },
                        "result": { "$ref": "#/$defs/Navigated" }
                    },
                    {
                        "name": "crawl.start",
                        "product": "foxhound",
                        "params": { "$ref": "#/$defs/CrawlStartParams" },
                        "result": { "$ref": "#/$defs/RunStarted" }
                    }
                ]),
            ),
            (
                "error_codes".to_owned(),
                json!([
                    {
                        "name": "navigation_failed",
                        "product": "collie",
                        "group": "unavailable"
                    }
                ]),
            ),
        ]),
        connection: BTreeMap::from([(
            "hello".to_owned(),
            json!({
                "params": { "$ref": "#/$defs/HelloParams" },
                "result": { "$ref": "#/$defs/HelloResult" }
            }),
        )]),
        defs: BTreeMap::from([
            (
                "GotoParams".to_owned(),
                json!({ "type": "object", "properties": { "url": { "type": "string" } } }),
            ),
            (
                "Navigated".to_owned(),
                json!({ "type": "object", "properties": { "status": { "type": "integer" } } }),
            ),
            (
                "CrawlStartParams".to_owned(),
                json!({ "type": "object", "properties": { "seeds": { "type": "array" } } }),
            ),
            (
                "RunStarted".to_owned(),
                json!({ "type": "object", "properties": { "run_id": { "type": "string" } } }),
            ),
            (
                "HelloParams".to_owned(),
                json!({ "type": "object", "properties": { "protocol_version": { "type": "string" } } }),
            ),
            (
                "HelloResult".to_owned(),
                json!({ "type": "object", "properties": { "protocol_version": { "type": "string" } } }),
            ),
        ]),
    };

    let mut text =
        serde_json::to_string_pretty(&document).expect("invariant: a published catalog serializes");
    text.push('\n');
    let read = PublishedCatalog::read(text.as_bytes())
        .expect("invariant: the corpus published catalog is valid");
    assert_eq!(read, document, "catalog.json reads back as written");
    std::fs::write(output.join("catalog.json"), text)
        .expect("invariant: the output folder is writable");
}
