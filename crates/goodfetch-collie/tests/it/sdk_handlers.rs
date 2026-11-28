//! SDK handlers end-to-end and conformance tests (spec #596, ticket #1029;
//! ADRs 0033, 0104, 0109 and 0132).
//!
//! Covers: story:7-72, story:7-73, story:7-114, story:7-76
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "integration tests set up homes, read Run traces and manage Daemon lifecycles on their own thread"
)]

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fmt::Write as _;
use std::fs;
use std::sync::mpsc;
use std::time::Duration;

use goodfetch_collie::{Collie, ConnectOptions, EngineChoice};
use goodfetch_collie_client::settings::{Settings, content, timeout};
use goodfetch_collie_client::{ClientConnection, HandlerRequests};
use goodfetch_protocol::error_code::ErrorCode;
use goodfetch_protocol::home::RUN_TRACE_PROVENANCE;
use goodfetch_protocol::key_value_store::VALUE_INVALID;
use goodfetch_protocol::page::{Asked, ExtractorName, FormatsRequest, MarkdownFormat};
use goodfetch_protocol::plugin::extractor::{EXTRACT_CONTENT, ExtractHandlerFailedDetails};
use goodfetch_protocol::run::{PluginConfinement, PluginForm, RunReport};
use goodfetch_test_client::HomeLayout;
use goodfetch_test_support::{
    BROWSERS_PATH_VARIABLE, ProvingGround, TestEnv, browser_cache, cli, real_time_runtime,
};
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::trace::{SdkTracerProvider, SpanData, SpanExporter};
use serde_json::{Value, json};
use tracing::Instrument as _;
use tracing_subscriber::layer::SubscriberExt as _;

use crate::COLLIE;
use crate::daemon::{StopsTheDaemon, short_idle};

const SEED: u64 = 1029;

fn env_options(env: &TestEnv) -> ConnectOptions {
    let map: BTreeMap<OsString, OsString> = env
        .env()
        .iter()
        .map(|(k, v)| (OsString::from(k), OsString::from(v)))
        .collect();
    ConnectOptions::default().environment(map)
}

fn extractor_name(name: &str) -> ExtractorName {
    ExtractorName::new(name).unwrap()
}

/// Two handlers of a single-key kind fail `connect()` with `setting_invalid`
/// when nothing else sets the key, and succeed when `content.extractor` is
/// set; a handler named as a built-in or an installed Plugin fails `connect()`
/// with `setting_invalid`; an unknown `content.extractor` fails when its scope
/// opens (`connect()` or `run.start`).
///
/// Covers: story:7-72, error:setting_invalid
#[test]
fn connect_and_run_start_refuse_ambiguous_clashing_and_unknown_extractors() {
    let env = TestEnv::new();
    // Install a dummy Extractor Plugin in the store to verify name clash.
    let bundle = env
        .home()
        .join("plugins")
        .join("installed_ext")
        .join("1.0.0");
    fs::create_dir_all(&bundle).unwrap();
    fs::write(
        bundle.join("goodfetch-plugin.json"),
        json!({
            "manifest_version": "0.1",
            "name": "installed_ext",
            "version": "1.0.0",
            "kinds": ["extract"],
            "protocol": { "min": "0.1", "max": "0.1" },
            "entrypoint": "plugin",
            "network_level": "none",
        })
        .to_string(),
    )
    .unwrap();

    real_time_runtime().block_on(async {
        // 1. Two handlers without choosing setting -> setting_invalid on connect().
        let ambiguous = Collie::connect(env_options(&env).extractor("alpha").extractor("beta"))
            .await
            .unwrap_err();
        assert_eq!(ambiguous.code(), ErrorCode::SETTING_INVALID, "{ambiguous}");

        // 2. Built-in name clash -> setting_invalid on connect().
        let builtin = Collie::connect(env_options(&env).extractor("dom_smoothie"))
            .await
            .unwrap_err();
        assert_eq!(builtin.code(), ErrorCode::SETTING_INVALID, "{builtin}");

        // 3. Installed Plugin name clash -> setting_invalid on connect().
        let installed = Collie::connect(env_options(&env).extractor("installed_ext"))
            .await
            .unwrap_err();
        assert_eq!(installed.code(), ErrorCode::SETTING_INVALID, "{installed}");

        // 4. Unknown handler in connect() settings -> setting_invalid.
        let unknown_connect = Collie::connect(
            env_options(&env)
                .settings(Settings::new().set(content::EXTRACTOR, extractor_name("nonexistent"))),
        )
        .await
        .unwrap_err();
        assert_eq!(
            unknown_connect.code(),
            ErrorCode::SETTING_INVALID,
            "{unknown_connect}"
        );

        // 5. Two handlers WITH content.extractor chosen -> succeeds!
        let chosen = Collie::connect(
            env_options(&env)
                .extractor("alpha")
                .extractor("beta")
                .settings(Settings::new().set(content::EXTRACTOR, extractor_name("alpha"))),
        )
        .await
        .unwrap();

        // 6. Unknown handler in run.start settings -> setting_invalid.
        let unknown_run = chosen
            .start()
            .settings(Settings::new().set(content::EXTRACTOR, extractor_name("still_missing")))
            .await
            .unwrap_err();
        assert_eq!(
            unknown_run.code(),
            ErrorCode::SETTING_INVALID,
            "{unknown_run}"
        );

        chosen.close().await.unwrap();
    });
}

/// On a Daemon shared by two Clients, Client A's registered SDK handler is
/// visible only to Client A's Runs: Client B's Run naming that handler is
/// refused with `setting_invalid` and never routes to Client A.
///
/// Covers: story:7-72, story:7-73, error:setting_invalid
#[test]
fn daemon_routes_sdk_handlers_only_to_the_client_that_registered_them() {
    let env = TestEnv::new();
    short_idle(&env, 60_000);
    let _stop = StopsTheDaemon::of(&env, COLLIE);
    let started = cli(&env, COLLIE).args(["daemon", "start"]).run().unwrap();
    assert_eq!(started.exit, Some(0), "{started:?}");

    real_time_runtime().block_on(async {
        let client_a = Collie::connect(
            env_options(&env)
                .engine(EngineChoice::Daemon)
                .extractor("client_a_only"),
        )
        .await
        .unwrap();
        let client_b = Collie::connect(env_options(&env).engine(EngineChoice::Daemon))
            .await
            .unwrap();

        // Client A's Run can use client_a_only (auto-selected via `registered`).
        let run_a = client_a.start().await.unwrap();
        run_a.finish().await.unwrap();

        // Client B's Run naming client_a_only is refused with setting_invalid.
        let refused = client_b
            .start()
            .settings(Settings::new().set(content::EXTRACTOR, extractor_name("client_a_only")))
            .await
            .unwrap_err();
        assert_eq!(refused.code(), ErrorCode::SETTING_INVALID, "{refused}");

        client_b.close().await.unwrap();
        client_a.close().await.unwrap();
    });
}

#[derive(Debug)]
struct Exported(mpsc::Sender<SpanData>);

impl SpanExporter for Exported {
    fn export(
        &self,
        batch: Vec<SpanData>,
    ) -> impl std::future::Future<Output = OTelSdkResult> + Send {
        for span in batch {
            let _ = self.0.send(span);
        }
        std::future::ready(Ok(()))
    }
}

fn proving_ground_toml(proving_ground: &ProvingGround) -> String {
    let mut toml = String::from("egress.private_ranges = [");
    for (i, host) in proving_ground.description().resolve.keys().enumerate() {
        if i > 0 {
            toml.push_str(", ");
        }
        let _ = write!(toml, "\"{host}\"");
    }
    toml.push_str("]\n[settings.network.resolve]\n");
    for (host, address) in &proving_ground.description().resolve {
        let _ = writeln!(toml, "\"{host}\" = [\"{address}\"]");
    }
    toml
}

async fn serve_four_extractor_calls(mut requests: HandlerRequests) -> (bool, Option<String>, bool) {
    // Call 1: succeed + check trace context span nesting.
    let call1 = requests.next().await.unwrap();
    let saw_name = call1
        .params()
        .get("handler")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let span = call1.span();
    let mut nested_span_ok = false;
    span.in_scope(|| {
        let child = tracing::info_span!("sdk_handler_child");
        let _entered = child.enter();
        nested_span_ok = true;
    });
    call1
        .answer(json!({
            "title": "SDK Title",
            "text": "SDK body text",
            "markdown": "# SDK Title\n\nSDK body text",
            "metadata": { "source_kind": "sdk" },
        }))
        .unwrap();

    // Call 2: return a GoodfetchError -> keeps its code (value_invalid).
    let call2 = requests.next().await.unwrap();
    let gf_err = goodfetch_collie_client::Error::of_sdk(
        &VALUE_INVALID,
        "bad html from handler",
        "fix html",
        &json!({ "value": "html" }),
    );
    call2
        .fail(gf_err, ErrorCode::EXTRACT_HANDLER_FAILED)
        .unwrap();

    // Call 3: return a non-Goodfetch BoxError -> becomes extract_handler_failed.
    let call3 = requests.next().await.unwrap();
    call3
        .fail("rust extractor boom", ErrorCode::EXTRACT_HANDLER_FAILED)
        .unwrap();

    // Call 4: wait for engine $/cancel past timeout.plugin -> cancelled() resolves.
    let call4 = requests.next().await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), call4.cancelled())
        .await
        .expect("HandlerCall::cancelled() must resolve on engine $/cancel");
    let cancel_ok = call4.is_cancelled();
    // Late answer after $/cancel is discarded cleanly.
    call4
        .answer(json!({
            "title": "Late",
            "text": "Late",
            "markdown": "Late",
            "metadata": {},
        }))
        .unwrap();

    (cancel_ok, saw_name, nested_span_ok)
}

fn assert_unconfined_report_and_provenance(env: &TestEnv, run_id: &str, report: &RunReport) {
    let recorded = report
        .plugins
        .as_deref()
        .unwrap_or_default()
        .iter()
        .find(|p| p.name == "my_ext")
        .expect("RunReport::plugins must list the SDK handler");
    assert_eq!(recorded.form, PluginForm::SdkHandler);
    assert_eq!(recorded.confinement, PluginConfinement::Unconfined);

    let provenance_path = HomeLayout::of(env.home()).run_trace(run_id, RUN_TRACE_PROVENANCE);
    let provenance_lines: Vec<Value> = fs::read_to_string(&provenance_path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let entry = provenance_lines
        .iter()
        .find(|line| line.get("action") == Some(&json!("extract_content")))
        .expect("trace/provenance.jsonl must record extract_content");
    assert_eq!(entry.get("extractor"), Some(&json!("my_ext")));
    assert_eq!(entry.get("form"), Some(&json!("sdk_handler")));
    assert_eq!(entry.get("confined"), Some(&json!(false)));
    assert_eq!(entry.get("confinement"), Some(&json!("unconfined")));
}

/// A Rust SDK handler registered via `ConnectOptions::extractor` serves
/// `page.formats` on a real linked Collie engine:
/// - routes `extract.content` with `handler` naming it;
/// - writes `trace/provenance.jsonl` with `form: "sdk_handler"`, `confined: false`;
/// - records `PluginForm::SdkHandler` and `PluginConfinement::Unconfined` in `RunReport::plugins`;
/// - keeps a `GoodfetchError`'s code, wraps other errors in `extract_handler_failed`;
/// - resolves `HandlerCall::cancelled()` when `timeout.plugin` passes and the engine sends `$/cancel`;
/// - makes the incoming `_meta` Trace context current via `HandlerCall::span()` so child spans nest under the engine's.
///
/// Covers: story:7-72, story:7-73, story:7-114, story:7-76, error:extract_handler_failed, error:deadline_exceeded
#[test]
fn sdk_extractor_serves_page_formats_records_unconfined_provenance_handles_errors_cancel_and_trace()
{
    let (sender, received) = mpsc::channel();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(Exported(sender))
        .build();
    tracing::subscriber::set_global_default(
        tracing_subscriber::registry()
            .with(tracing_opentelemetry::layer().with_tracer(provider.tracer("sdk-handlers-test"))),
    )
    .unwrap();

    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    env.user_configuration(&proving_ground_toml(&proving_ground));
    let cache = browser_cache(&env).unwrap();
    let product_url = proving_ground.url("extraction.test", "/markdown/mixed/product");

    let mut env_map: BTreeMap<OsString, OsString> = env
        .env()
        .iter()
        .map(|(k, v)| (OsString::from(k), OsString::from(v)))
        .collect();
    env_map.insert(
        OsString::from(BROWSERS_PATH_VARIABLE),
        cache.as_os_str().to_owned(),
    );

    let (cancel_resolved, saw_handler_name, trace_nested) = real_time_runtime().block_on(async {
        let collie = Collie::connect(
            ConnectOptions::default()
                .environment(env_map)
                .extractor("my_ext")
                .settings(Settings::new().set(timeout::PLUGIN, Duration::from_millis(250))),
        )
        .await
        .unwrap();

        let client: &ClientConnection = collie.as_ref();
        let requests = client.handler_requests(EXTRACT_CONTENT).unwrap();
        let handler_task = tokio::spawn(serve_four_extractor_calls(requests));

        let run = collie.start().await.unwrap();
        let run_id = run.id().to_string();
        let page = run.new_page().await.unwrap();
        page.goto(&product_url).await.unwrap();

        // 1. Happy path -> served by SDK handler "my_ext" via `registered`,
        //    inside a program span so the handler's child span inherits its trace.
        let formats = async {
            page.formats(FormatsRequest {
                markdown: Some(MarkdownFormat::Asked(Asked)),
                text: Some(Asked),
                ..Default::default()
            })
            .await
        }
        .instrument(tracing::info_span!("program_formats"))
        .await
        .unwrap();
        assert_eq!(
            formats.markdown.as_deref(),
            Some("# SDK Title\n\nSDK body text")
        );
        assert_eq!(formats.text.as_deref(), Some("SDK body text"));

        // 2. GoodfetchError keeps its code.
        let text_only = || FormatsRequest {
            text: Some(Asked),
            ..Default::default()
        };
        let err2 = page.formats(text_only()).await.unwrap_err();
        assert_eq!(err2.code(), ErrorCode::VALUE_INVALID, "{err2}");

        // 3. Non-Goodfetch BoxError becomes extract_handler_failed with handler & BoxError details.
        let err3 = page.formats(text_only()).await.unwrap_err();
        assert_eq!(err3.code(), ErrorCode::EXTRACT_HANDLER_FAILED, "{err3}");
        let details3: ExtractHandlerFailedDetails =
            err3.details().expect("extract_handler_failed details");
        assert_eq!(details3.handler.as_deref(), Some("my_ext"));
        assert_eq!(details3.exception_type.as_deref(), Some("BoxError"));
        assert_eq!(
            details3.exception_message.as_deref(),
            Some("rust extractor boom")
        );

        // 4. Past timeout.plugin -> deadline_exceeded and engine sends $/cancel.
        let err4 = page.formats(text_only()).await.unwrap_err();
        assert_eq!(err4.code(), ErrorCode::DEADLINE_EXCEEDED, "{err4}");

        let report = run.finish().await.unwrap();
        collie.close().await.unwrap();
        assert_unconfined_report_and_provenance(&env, &run_id, &report);

        handler_task.await.unwrap()
    });

    provider.shutdown().unwrap();
    let spans: Vec<SpanData> = received.try_iter().collect();
    assert!(cancel_resolved, "HandlerCall::cancelled() must resolve");
    assert_eq!(saw_handler_name.as_deref(), Some("my_ext"));
    assert!(trace_nested);
    let program_span = spans
        .iter()
        .find(|s| s.name == "program_formats")
        .expect("expected program_formats span in exported spans");
    let child_span = spans
        .iter()
        .find(|s| s.name == "sdk_handler_child")
        .expect("expected sdk_handler_child span in exported spans");
    assert_eq!(
        child_span.span_context.trace_id(),
        program_span.span_context.trace_id(),
        "SDK handler child span must inherit the calling program span's trace_id"
    );
}
