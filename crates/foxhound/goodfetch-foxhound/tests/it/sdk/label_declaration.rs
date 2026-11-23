//! A Rust crawl's Label declared whole (ADRs 0093 and 0099):
//! `crawl.label(LabelDeclaration::<P>::new(name).payload(schema).settings(settings))`
//! sends its Payload schema and its settings with the crawl's start, so
//! the engine checks each Payload of the Label against the schema and
//! applies the settings to the Label's requests; a key a Label does not
//! set fails the declaration at once with `setting_not_applicable`.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_foxhound::settings::{self, Settings, politeness};
use goodfetch_foxhound::{
    Crawl, CrawlContext, Error, ErrorCode, Foxhound, LabelDeclaration, Schema, Seed, error_code,
};
use goodfetch_test_support::{ProvingGround, TestEnv, real_time_runtime};
use serde_json::{Value, json};

use super::crawl::options;

/// The Proving ground's seed the crawl reads.
const SITE: u64 = 1602;

/// A Label's Payload schema: a depth, a whole number, and nothing else.
fn visit_schema() -> Value {
    json!({
        "type": "object",
        "properties": { "depth": { "type": "integer" } },
        "required": ["depth"],
        "additionalProperties": false,
    })
}

/// The seed's handler enqueues the empty page under its own Label with a
/// Payload the Label's schema refuses: the enqueue fails with
/// `payload_invalid`, and the Run adds no request for it.
///
/// Covers: story:11-1014, error:payload_invalid
#[test]
fn an_enqueue_its_labels_schema_refuses_fails_with_payload_invalid() {
    let env = TestEnv::new();
    let site = ProvingGround::start(&env, SITE).unwrap();
    let (book, empty) = (
        site.url("shop.test", "/book"),
        site.url("shop.test", "/empty"),
    );
    real_time_runtime().block_on(async {
        let foxhound = Foxhound::connect(options(&env, &site)).await.unwrap();
        let visits = Schema::json(visit_schema()).unwrap();
        let mut crawl = Crawl::new();
        let page = crawl
            .label(LabelDeclaration::new("page").payload(&visits))
            .unwrap();
        let (refused_tx, mut refused_rx) = tokio::sync::mpsc::unbounded_channel();
        let enqueued_label = page.clone();
        crawl
            .handle(&page, move |ctx: CrawlContext<Value>| async move {
                let refused = ctx
                    .enqueue(&empty, &enqueued_label, json!({ "depth": "one" }))
                    .await
                    .map_err(|error| error.code());
                refused_tx.send(refused).unwrap();
                Ok(())
            })
            .unwrap();

        let run = foxhound
            .start_crawl(&crawl, [Seed::new(&book, &page, json!({ "depth": 0 }))])
            .await
            .unwrap();
        let report = crawl.serve(&run).await.unwrap();
        foxhound.close().await.unwrap();

        let requests = report.requests.unwrap();
        assert_eq!(
            (
                refused_rx.recv().await.unwrap().map(|_| ()),
                requests.succeeded,
                requests.failed,
            ),
            (Err(error_code::PAYLOAD_INVALID), 1, 0),
            "{requests:?}"
        );
    });
}

/// A Label whose settings take only JSON responses skips its seed's HTML
/// page before its handler: the settings reach the engine with the
/// Label's declaration.
///
/// Covers: story:11-1014
#[test]
fn a_labels_settings_apply_to_its_requests() {
    let env = TestEnv::new();
    let site = ProvingGround::start(&env, SITE).unwrap();
    let book = site.url("shop.test", "/book");
    real_time_runtime().block_on(async {
        let foxhound = Foxhound::connect(options(&env, &site)).await.unwrap();
        let mut crawl = Crawl::new();
        let json_only = crawl
            .label(
                LabelDeclaration::<Value>::new("json").settings(Settings::new().set(
                    settings::crawl::CONTENT_TYPES,
                    vec!["application/json".to_owned()],
                )),
            )
            .unwrap();
        crawl
            .handle(&json_only, |_ctx: CrawlContext<Value>| async {
                panic!("the `json` Label's HTML page is skipped before its handler")
            })
            .unwrap();

        let run = foxhound
            .start_crawl(&crawl, [Seed::new(&book, &json_only, json!({}))])
            .await
            .unwrap();
        let report = crawl.serve(&run).await.unwrap();
        foxhound.close().await.unwrap();

        let requests = report.requests.unwrap();
        assert_eq!(
            (requests.succeeded, requests.skipped, requests.failed),
            (0, 1, 0),
            "{requests:?}"
        );
    });
}

/// A seed whose Payload the Label's schema refuses is refused with
/// `payload_invalid` by the crawl's start.
///
/// Covers: story:11-1014, error:payload_invalid
#[test]
fn a_seed_its_labels_schema_refuses_fails_the_start() {
    let env = TestEnv::new();
    let site = ProvingGround::start(&env, SITE).unwrap();
    let book = site.url("shop.test", "/book");
    real_time_runtime().block_on(async {
        let foxhound = Foxhound::connect(options(&env, &site)).await.unwrap();
        let mut crawl = Crawl::new();
        let visits = Schema::json(visit_schema()).unwrap();
        let page = crawl
            .label(LabelDeclaration::new("page").payload(&visits))
            .unwrap();
        crawl
            .handle(&page, |_ctx: CrawlContext<Value>| async { Ok(()) })
            .unwrap();

        let refused = foxhound
            .start_crawl(&crawl, [Seed::new(&book, &page, json!({ "depth": -0.5 }))])
            .await
            .unwrap_err();
        foxhound.close().await.unwrap();

        assert_eq!(refused.code(), error_code::PAYLOAD_INVALID, "{refused}");
    });
}

/// A key a Label's settings do not set, a Run-wide one, fails the
/// declaration at once with `setting_not_applicable`, with no connection,
/// and declares no Label: the name stays free.
///
/// Covers: story:11-1014, error:setting_not_applicable
#[test]
fn a_run_wide_key_in_a_labels_settings_fails_its_declaration_at_once() {
    let _env = TestEnv::new();
    let mut crawl = Crawl::new();
    let refused = crawl
        .label(
            LabelDeclaration::<()>::new("page")
                .settings(Settings::new().set(politeness::domain::CONCURRENCY, 2_u64)),
        )
        .unwrap_err();
    let declared = crawl.label::<()>("page");

    assert_eq!(
        (
            refused.code(),
            declared
                .map(|label| label.name().to_owned())
                .map_err(|error: Error| error.code())
        ),
        (ErrorCode::SETTING_NOT_APPLICABLE, Ok("page".to_owned())),
        "{refused}"
    );
}
