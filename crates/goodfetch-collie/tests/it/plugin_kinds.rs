//! A kind's driver over a process Plugin (ADRs 0033, 0127 and 0129): the
//! Extractor's, which calls `goodfetch-scripted-plugin` declared as an
//! Extractor, started confined through Collie's executable, and reads its
//! answer as the engine reads any decoded input.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::collections::BTreeMap;

use goodfetch_engine::clock::Clock;
use goodfetch_engine::deadline::Deadline;
use goodfetch_engine::plugin_host::instance::{PluginInstance, Scope};
use goodfetch_engine::plugin_kinds::Failure;
use goodfetch_engine::plugin_kinds::extractor::{ExtractorDriver, ProcessExtractor};
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_protocol::plugin::extractor::ExtractContentResult;
use goodfetch_protocol::settings::{SettingsLayer, TIMEOUT_PLUGIN};
use goodfetch_test_support::{capturing, real_time_runtime};
use rstest::rstest;
use serde_json::{Value, json};
use tracing::Level;
use url::Url;

use crate::plugin_host::{Plugin, hello};

/// Text the scripted Extractor answers that may reach the log, never an
/// error.
const CANARY: &str = "canary text of the Extractor";

/// The deadline the tests' calls wait under, long enough for a confined
/// start on a loaded host.
const PLUGIN_MILLIS: u64 = 30_000;

/// What the Extractor driver makes of a scripted Extractor that answers
/// `extract.content` with `answer`, under a deadline of `millis`, and what
/// the driver logged at `warn`: on real time, as the Plugin is a real
/// process.
fn extracted(answer: &Value, millis: u64) -> (Result<ExtractContentResult, Failure>, String) {
    let (subscriber, log) = capturing(&[("goodfetch_engine::plugin_kinds", Level::WARN)]);
    let _logging = tracing::subscriber::set_default(subscriber);
    let plugin = Plugin::of_kind(
        "extract",
        &json!({
            "hello": hello(&json!([{ "kind": "extract", "capabilities": [] }])),
            "answers": { "extract.content": answer },
        }),
    );
    let extracted = real_time_runtime().block_on(async {
        let root = TaskOwner::root(BlockingPolicy::Threads);
        let extractor = ProcessExtractor::new(
            "scripted".to_owned(),
            PluginInstance::start(&root, plugin.instance(), Scope::Task(None)),
        );
        let clock = Clock::system();
        let deadline = Deadline::given(&TIMEOUT_PLUGIN, millis, SettingsLayer::Default);
        let extracted = ExtractorDriver::new(&extractor, &clock, &deadline)
            .content(&page(), "<main><h1>Kettle</h1></main>".to_owned())
            .await;
        root.stop().await;
        extracted
    });
    (extracted, String::from_utf8(log.written()).unwrap())
}

/// The page the tests extract.
fn page() -> Url {
    Url::parse("https://shop.test/kettle").unwrap()
}

/// A result of the Extractor's shape.
fn content() -> Value {
    json!({
        "title": "Kettle",
        "text": "Kettle",
        "markdown": "# Kettle",
        "metadata": { "author": "Ada" },
    })
}

/// A result the Extractor answers is the call's.
///
/// Covers: story:7-87
#[test]
fn an_extractors_result_is_the_calls() {
    let (extracted, _) = extracted(&json!({ "result": content() }), PLUGIN_MILLIS);

    assert_eq!(
        extracted.unwrap(),
        ExtractContentResult {
            title: Some("Kettle".to_owned()),
            text: "Kettle".to_owned(),
            markdown: "# Kettle".to_owned(),
            metadata: BTreeMap::from([("author".to_owned(), "Ada".to_owned())]),
        }
    );
}

/// An Error code of the catalog the Extractor answers is kept, with its
/// message and details.
///
/// Covers: story:7-113
#[test]
fn an_extractors_known_error_code_is_kept() {
    let answer = json!({ "error": {
        "code": -32003,
        "message": "cdn.test is not an allowed domain",
        "data": { "code": "egress_refused", "retryable": false, "host": "cdn.test" },
    } });

    let (extracted, log) = extracted(&answer, PLUGIN_MILLIS);

    let Err(Failure::Answered(error)) = extracted else {
        panic!("{extracted:?} kept no answered code");
    };
    assert_eq!(
        (
            error.code(),
            error.message(),
            error.details().cloned().map(Value::Object),
            log
        ),
        (
            "egress_refused",
            "cdn.test is not an allowed domain",
            Some(json!({ "host": "cdn.test" })),
            String::new()
        )
    );
}

/// What the Extractor answers that the driver cannot take fails the call
/// with `extract_handler_failed`, which names the Extractor and the page
/// and holds none of its text; that text is logged at `warn`.
///
/// Covers: story:7-113, error:extract_handler_failed
#[rstest]
#[case::unknown_code(
    json!({ "error": { "code": -32000, "message": CANARY, "data": { "code": "kettle_on_fire", "retryable": false } } }),
    CANARY
)]
#[case::error_without_code(json!({ "error": { "code": -32000, "message": CANARY } }), CANARY)]
#[case::crawl_handlers_code(
    json!({ "error": { "code": -32001, "message": CANARY, "data": { "code": "handler_failed", "retryable": true } } }),
    CANARY
)]
#[case::no_message(json!({ "jsonrpc": "1.0" }), "cannot be read")]
#[case::not_an_object(json!({ "result": CANARY }), CANARY)]
#[case::field_it_does_not_know(
    json!({ "result": { "title": null, "text": "", "markdown": "", "metadata": {}, "canary": CANARY } }),
    "canary"
)]
#[case::missing_field(
    json!({ "result": { "title": CANARY, "text": "", "metadata": {} } }),
    "markdown"
)]
#[case::metadata_key_not_snake_case(
    json!({ "result": { "title": null, "text": "", "markdown": "", "metadata": { "Canary-Key": CANARY } } }),
    "Canary-Key"
)]
fn an_answer_the_driver_cannot_take_is_the_extractors_failure(
    #[case] answer: Value,
    #[case] logged: &str,
) {
    let (extracted, log) = extracted(&answer, PLUGIN_MILLIS);

    let Err(Failure::Malformed(error)) = extracted else {
        panic!("{extracted:?} is no malformed answer");
    };
    assert_eq!(
        (
            error.code(),
            error.message().contains(CANARY),
            error.details().cloned().map(Value::Object),
            log.contains(" WARN ") && log.contains(logged),
        ),
        (
            "extract_handler_failed",
            false,
            Some(json!({ "extractor": "scripted", "url": page().as_str() })),
            true
        ),
        "{log}"
    );
}

/// An Extractor that leaves its call unanswered past `timeout.plugin`
/// fails it with `deadline_exceeded`, which names the setting.
///
/// Covers: setting:timeout.plugin, error:deadline_exceeded
#[test]
fn an_unanswered_call_ends_at_timeout_plugin() {
    let (extracted, _) = extracted(&json!({ "silent": true }), 2_000);

    let Err(Failure::DeadlineExceeded(error)) = extracted else {
        panic!("{extracted:?} did not end at its deadline");
    };
    assert_eq!(
        (
            error.code(),
            error.message().contains("timeout.plugin"),
            error.details().map(|details| details["waited_for"].clone()),
        ),
        ("deadline_exceeded", true, Some(json!("plugin_answer")))
    );
}
