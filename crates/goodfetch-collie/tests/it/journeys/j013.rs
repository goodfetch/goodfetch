//! J-013, "Run a one-off SDK script on an Attached engine, then two scripts
//! that share the Daemon", on the `sdk-rust` surface (ADRs 0063, 0075 and
//! 0094): a one-off Rust SDK program with no Daemon running runs on its
//! linked in-process engine (Rust links the engine instead of spawning a
//! stdio child process) without starting a Daemon; then once the Daemon is
//! started on the CLI, two SDK connections run at once and share that one
//! Daemon without starting a separate engine. TypeScript's
//! `tests/journeys/J-013.test.ts` and Python's `tests/journeys/test_j013.py`
//! are its twins.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::Path;

use goodfetch_collie::{
    Collie, ConnectOptions, EngineInfo, EngineKind, FieldLocator, FieldTransform, PageField,
    ValueField,
};
use goodfetch_protocol::result_document::ResultDocument;
use goodfetch_test_support::{JourneyRun, browser_cache_in_home, real_time_runtime};
use serde::Deserialize;
use serde_json::{Number, Value, json};

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// The Proving ground's task whose product page each script extracts.
const TASK: &str = "catalog/2";

/// Stops the Daemon when the test ends, whether it passed or not.
struct StopsTheDaemon<F: FnMut()>(F);

impl<F: FnMut()> Drop for StopsTheDaemon<F> {
    fn drop(&mut self) {
        (self.0)();
    }
}

/// The product page's fields as the script reads them.
#[derive(Debug, Deserialize)]
struct Product {
    id: Option<String>,
    name: Option<String>,
    price: Option<Number>,
}

impl Product {
    fn record(&self) -> Value {
        json!({ "id": self.id, "name": self.name, "price": self.price })
    }
}

/// The product's ID, name and price, as task `catalog/2` names them.
fn product_fields() -> BTreeMap<String, PageField> {
    let css = |selector: &str| ValueField {
        locator: FieldLocator::Css {
            selector: selector.to_owned(),
        },
        fallbacks: Vec::new(),
        attribute: None,
        matched: None,
        transform: FieldTransform::Text,
    };
    BTreeMap::from([
        ("id".to_owned(), PageField::Value(css("span.id"))),
        ("name".to_owned(), PageField::Value(css("h1.name"))),
        (
            "price".to_owned(),
            PageField::Value(ValueField {
                attribute: Some("data-price".to_owned()),
                transform: FieldTransform::Number {
                    locale: "en".to_owned(),
                },
                ..css("span.price")
            }),
        ),
    ])
}

/// What the executable answered `args` with `--json`, which it must answer
/// successfully.
fn answer<C>(run: &JourneyRun<C>, args: &[&str]) -> Value {
    let output = run.command().args(args).arg("--json").output().unwrap();
    assert!(output.status.success(), "{args:?}: {output:?}");
    serde_json::from_slice(&output.stdout).unwrap_or_else(|_| panic!("{args:?}: {output:?}"))
}

/// Runs one Rust SDK script extracting `product_url` and returns the reached
/// engine info and extracted product record.
async fn extract_product(
    env: BTreeMap<OsString, OsString>,
    product_url: &str,
) -> (EngineInfo, Value) {
    let collie = Collie::connect(ConnectOptions::default().environment(env))
        .await
        .unwrap();
    let engine = collie.engine().clone();
    let run = collie.start().await.unwrap();
    let page = run.new_page().await.unwrap();
    page.goto(product_url).await.unwrap();
    let read = page
        .read(product_fields())
        .typed::<Product>()
        .await
        .unwrap();
    run.finish().await.unwrap();
    collie.close().await.unwrap();
    (engine, read.values.record())
}

/// Covers: row:RT-19, story:3-1, story:3-2
#[test]
fn sdk_rust() {
    let mut run = JourneyRun::start(
        "J-013",
        "sdk-rust",
        Path::new(COLLIE),
        ResultDocument::journey,
    )
    .expect("J-013's sdk-rust variant starts");
    run.surface_package("goodfetch-collie", env!("CARGO_PKG_VERSION"));
    let proving_ground = run.proving_ground().expect("J-013 has a Proving ground");
    let product = proving_ground.start_url(TASK).unwrap();
    run.env()
        .user_configuration(&proving_ground.description().reaching());
    browser_cache_in_home(run.env()).expect("Playwright's Chrome for Testing is installed");

    let mut stop = run.command();
    stop.args(["daemon", "stop"]);
    let _stops = StopsTheDaemon(move || {
        let _ = stop.output();
    });

    let mut one_off: Option<(EngineInfo, Value)> = None;
    let mut status_after_one_off = Value::Null;
    let mut daemon_pid = Value::Null;
    let mut shared: Option<((EngineInfo, Value), (EngineInfo, Value))> = None;
    let mut status_after_shared = Value::Null;

    run.step(
        "With no Daemon running and nothing configured, run a one-off script that extracts the catalog's product page",
        |run| {
            let env = run.env().env_os();
            let outcome = real_time_runtime().block_on(extract_product(env, &product));
            status_after_one_off = answer(run, &["daemon", "status"]);
            one_off = Some(outcome);
        },
    );

    run.step("Start the Daemon", |run| {
        let started = answer(run, &["daemon", "start"]);
        assert!(started["pid"].is_u64(), "{started}");
        daemon_pid = started["pid"].clone();
    });

    run.step(
        "Run two scripts at once, each extracting the catalog's product page",
        |run| {
            let first_env = run.env().env_os();
            let second_env = run.env().env_os();
            let pair = real_time_runtime().block_on(async {
                tokio::join!(
                    extract_product(first_env, &product),
                    extract_product(second_env, &product),
                )
            });
            status_after_shared = answer(run, &["daemon", "status"]);
            shared = Some(pair);
        },
    );

    run.check(
        "The one-off script ran on an Attached engine it started over stdio, which stopped when the script ended, and no Daemon was started",
        |_| {
            let (engine, _) = one_off.as_ref().expect("one-off script ran");
            assert_eq!(engine.kind, EngineKind::Linked);
            assert_eq!(engine.url, None);
            assert_eq!(status_after_one_off["pid"], Value::Null);
        },
    );

    run.expect_task(
        "The oracle verifies the one-off script's result",
        &json!({
            "status": "success",
            "records": one_off.as_ref().map(|(_, record)| vec![record.clone()]).unwrap_or_default(),
        }),
    );

    run.check(
        "Both scripts ran on the one running Daemon, and neither started an Attached engine",
        |_| {
            let ((first_engine, _), (second_engine, _)) =
                shared.as_ref().expect("shared scripts ran");
            assert_eq!(first_engine.kind, EngineKind::Daemon);
            assert_eq!(second_engine.kind, EngineKind::Daemon);
            assert_eq!(first_engine.engine_path, None);
            assert_eq!(second_engine.engine_path, None);
            assert!(first_engine.url.is_some(), "{first_engine:?}");
            assert_eq!(second_engine.url, first_engine.url);
            assert_eq!(status_after_shared["pid"], daemon_pid);
        },
    );

    run.expect_task(
        "The oracle verifies the first script's result",
        &json!({
            "status": "success",
            "records": shared.as_ref().map(|((_, record), _)| vec![record.clone()]).unwrap_or_default(),
        }),
    );

    run.expect_task(
        "The oracle verifies the second script's result",
        &json!({
            "status": "success",
            "records": shared.as_ref().map(|(_, (_, record))| vec![record.clone()]).unwrap_or_default(),
        }),
    );

    run.finish()
        .expect("J-013's document is written")
        .assert_proved();
}
