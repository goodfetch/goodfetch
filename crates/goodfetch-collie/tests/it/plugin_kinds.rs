//! A kind's driver over a process Plugin (ADRs 0033, 0127 and 0129): the
//! Extractor's, which calls `goodfetch-scripted-plugin` declared as an
//! Extractor, started confined through Collie's executable, reads its
//! answer as the engine reads any decoded input and accounts for every
//! call it makes.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#![expect(
    clippy::disallowed_types,
    reason = "the test's Run records its calls behind a lock held for one push, never across an `.await`, outside any code loom checks (ADR 0060)"
)]

use std::collections::BTreeMap;
use std::pin::Pin;
use std::sync::Mutex;

use goodfetch_engine::CallError;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::deadline::Deadline;
use goodfetch_engine::plugin_host::instance::{PluginInstance, Scope};
use goodfetch_engine::plugin_kinds::extractor::{ExtractorDriver, ProcessExtractor};
use goodfetch_engine::plugin_kinds::{Accounts, Failure, PluginCall};
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_protocol::plugin::extractor::ExtractContentResult;
use goodfetch_protocol::settings::{SettingsLayer, TIMEOUT_PLUGIN};
use goodfetch_protocol::usage::LIMIT_REACHED;
use goodfetch_test_support::{capturing, real_time_runtime};
use rstest::rstest;
use serde_json::{Value, json};
use tracing::Level;
use url::Url;

use crate::plugin_host::{Plugin, hello};

/// What an [`Accounts`] answers, once awaited.
type Accounted<'call> = Pin<Box<dyn Future<Output = Result<(), CallError>> + Send + 'call>>;

/// The Run a test's calls are accounted to: it records each call as the
/// Run trace would write it, and refuses each call's start when it refuses.
#[derive(Default)]
struct Recorded {
    /// Whether a Limit refuses each call before it goes.
    refuses: bool,
    /// Each call, as it started or once it ended.
    calls: Mutex<Vec<(&'static str, Value)>>,
}

impl Recorded {
    fn record(&self, when: &'static str, call: &PluginCall<'_>) {
        self.calls
            .lock()
            .unwrap()
            .push((when, serde_json::to_value(call).unwrap()));
    }
}

impl Accounts for Recorded {
    fn call_starts<'call>(&'call self, call: &'call PluginCall<'call>) -> Accounted<'call> {
        self.record("starts", call);
        let refused = self.refuses.then(|| {
            CallError::new(
                &LIMIT_REACHED,
                "The Run Limit on plugin_calls has nothing left.".to_owned(),
                &json!({}),
            )
        });
        Box::pin(async move { refused.map_or(Ok(()), Err) })
    }

    fn call_ended<'call>(&'call self, call: &'call PluginCall<'call>) -> Accounted<'call> {
        self.record("ended", call);
        Box::pin(async { Ok(()) })
    }
}

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
    extracted_for(&Recorded::default(), answer, millis)
}

/// What [`extracted`] answers, with its call accounted to `run`.
fn extracted_for(
    run: &Recorded,
    answer: &Value,
    millis: u64,
) -> (Result<ExtractContentResult, Failure>, String) {
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
        let extracted = ExtractorDriver::new(&extractor, run, &clock, &deadline)
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

/// Every call the driver makes is accounted to its Run however it ends:
/// charged before it goes, then recorded once it ended by implementation
/// and method, with the class the engine gives `extract.content`, the time
/// it took and the Error code it failed with.
///
/// Covers: story:7-38, story:7-39
#[rstest]
#[case::answered(json!({ "result": content() }), PLUGIN_MILLIS, None, 0)]
#[case::known_code(
    json!({ "error": { "code": -32003, "message": "refused", "data": { "code": "egress_refused", "retryable": false } } }),
    PLUGIN_MILLIS,
    Some("egress_refused"),
    0
)]
#[case::malformed(json!({ "result": CANARY }), PLUGIN_MILLIS, Some("extract_handler_failed"), 0)]
#[case::unanswered(json!({ "silent": true }), 2_000, Some("deadline_exceeded"), 2_000)]
fn every_call_is_accounted_with_the_engines_class(
    #[case] answer: Value,
    #[case] millis: u64,
    #[case] failed: Option<&str>,
    #[case] took_at_least: u64,
) {
    let run = Recorded::default();

    let (extracted, _) = extracted_for(&run, &answer, millis);

    let mut calls = run.calls.into_inner().unwrap();
    let took = calls
        .get_mut(1)
        .and_then(|(_, ended)| ended.as_object_mut())
        .and_then(|ended| ended.insert("time".to_owned(), json!(0)))
        .and_then(|time| time.as_u64());
    let call = |failed: Option<&str>| {
        json!({
            "plugin": "scripted",
            "kind": "extract",
            "method": "extract.content",
            "class": "read",
            "time": 0,
            "failed": failed,
        })
    };
    assert_eq!(
        (
            calls,
            extracted.err().map(|failure| failure.error().code()),
            took.is_some_and(|took| took >= took_at_least),
        ),
        (
            vec![("starts", call(None)), ("ended", call(failed))],
            failed,
            true
        )
    );
}

/// A call its Run's Usage refuses fails with `limit_reached` before it
/// goes: nothing records it as made.
///
/// Covers: story:7-38, error:limit_reached
#[test]
fn a_call_the_runs_usage_refuses_is_never_made() {
    let run = Recorded {
        refuses: true,
        ..Recorded::default()
    };

    let (extracted, _) = extracted_for(&run, &json!({ "result": content() }), PLUGIN_MILLIS);

    let Err(Failure::Refused(error)) = extracted else {
        panic!("{extracted:?} was not refused");
    };
    assert_eq!(
        (
            error.code(),
            run.calls
                .into_inner()
                .unwrap()
                .into_iter()
                .map(|(when, _)| when)
                .collect::<Vec<_>>()
        ),
        ("limit_reached", vec!["starts"])
    );
}
