//! A call on a page the Product serves itself, as a fetch tier serves a
//! page it opened from a document (ADR 0093): the page crate's guard finds
//! the page the call names and hands the call to the Product, in process,
//! with no browser. The call decodes its params once, the guard's search
//! for its page included (ADR 0140's E1, issue 2617). A call on a page a
//! call encloses, as a crawl handler encloses its pages' calls, ends at
//! that call's end (ADR 0080).
#![expect(
    clippy::disallowed_methods,
    reason = "tests run on the test Clock and Entropy (ADR 0059)"
)]

use std::collections::BTreeMap;
use std::future::{pending, ready};
use std::time::Duration;

use goodfetch_counted_work::Row;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::deadline::{Deadline, Enclosing};
use goodfetch_engine::entropy::Entropy;
use goodfetch_engine::home::{Home, RunStateLocation};
use goodfetch_engine::metrics::Metrics;
use goodfetch_engine::run_state::RunStateOptions;
use goodfetch_engine::tasks::BlockingPolicy;
use goodfetch_engine::transport::{self, Network};
use goodfetch_engine::{
    Answered, CallContext, Channel, Composition, Dependencies, Engine, GuardAnswer, Identity,
    ReadParams,
};
use goodfetch_page::Launcher;
use goodfetch_page::page_value::{HandlerPages, Observer, PageValue};
use goodfetch_protocol::catalog::{Product, Setting};
use goodfetch_protocol::connection::{EnclosingCall, ProductVersion};
use goodfetch_protocol::jsonrpc::RequestId;
use goodfetch_protocol::page::{PageId, SessionId};
use goodfetch_protocol::run::RunId;
use goodfetch_protocol::settings::{TIMEOUT_FIRST_BYTE, TIMEOUT_HOOK};
use goodfetch_test_support::TestEnv;
use rstest::rstest;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};

use crate::handler_page::NoBlocks;

/// The page the Product serves.
const SERVED: &str = "served-page";

/// Foxhound's `timeout.handler` with its 60 s default, which
/// `goodfetch-page` may not name (ADR 0099).
const TIMEOUT_HANDLER: Setting = Setting {
    name: "timeout.handler",
    default: || json!(60_000),
    ..TIMEOUT_HOOK
};

/// A Product that answers every call on [`SERVED`] with the call's name.
struct Served;

impl HandlerPages for Served {
    fn value(&self, _run: &RunId, _session: &SessionId) -> PageValue {
        PageValue::empty()
    }

    fn observer(&self, _run: &RunId, _session: &SessionId) -> Option<Observer> {
        None
    }

    fn call(
        &self,
        page_id: &PageId,
        operation: &'static str,
        _params: ReadParams<'_>,
        context: &CallContext,
    ) -> Option<GuardAnswer> {
        (page_id.0 == SERVED)
            .then(|| Box::pin(ready(Answered::encode(context, &operation))) as GuardAnswer)
    }
}

/// A Product whose call on [`SERVED`] never answers, and whose pages a
/// crawl handler's call encloses, its timeout, such as `timeout.handler`,
/// ending 10 s after the page's call arrives.
struct Enclosed(&'static Setting);

impl HandlerPages for Enclosed {
    fn value(&self, _run: &RunId, _session: &SessionId) -> PageValue {
        PageValue::empty()
    }

    fn observer(&self, _run: &RunId, _session: &SessionId) -> Option<Observer> {
        None
    }

    fn encloses_calls(&self) -> bool {
        true
    }

    fn enclosing(&self, page_id: &PageId) -> Option<Enclosing> {
        let ends_at = tokio::time::Instant::now() + Duration::from_secs(10);
        (page_id.0 == SERVED).then(|| {
            Enclosing::new(
                &Deadline::declared(self.0),
                EnclosingCall {
                    operation: "crawl_request.handle".to_owned(),
                    call_id: RequestId::Number(3),
                },
                move || ends_at,
            )
        })
    }

    fn call(
        &self,
        page_id: &PageId,
        _operation: &'static str,
        _params: ReadParams<'_>,
        _context: &CallContext,
    ) -> Option<GuardAnswer> {
        (page_id.0 == SERVED).then(|| Box::pin(pending()) as GuardAnswer)
    }
}

fn dependencies(env: &TestEnv) -> Dependencies {
    Dependencies {
        environment: BTreeMap::new(),
        keychain: goodfetch_engine::secrets::Keychain::absent(),
        home: Home::new(env.home().to_owned()),
        clock: Clock::paused(env.start_time()),
        entropy: Entropy::seeded(env.seed()),
        blocking: BlockingPolicy::Inline,
        run_state_location: RunStateLocation::PerMajor,
        run_state: RunStateOptions::default(),
        politeness: goodfetch_engine::politeness::PolitenessOptions::default(),
    }
}

/// Says `hello` to an engine of the engine's operations and the page
/// crate's, its pages [`Served`]'s, then calls `method` with `params`: the
/// call's reply and the decodes of params it counted.
fn called(method: &str, params: &Value) -> (Value, u64) {
    called_on(Served, method, params)
}

/// [`called`], its pages `product`'s.
fn called_on(product: impl HandlerPages + 'static, method: &str, params: &Value) -> (Value, u64) {
    called_in(None, product, method, params)
}

/// [`called_on`], its `hello` declaring `form` when one is given.
fn called_in(
    form: Option<&str>,
    product: impl HandlerPages + 'static,
    method: &str,
    params: &Value,
) -> (Value, u64) {
    let mut hello = json!({
        "protocol_version": "0.1",
        "surface": "rust",
        "needs": "collie",
        "product_version": "1.4.0",
    });
    if let Some(form) = form {
        hello["form"] = json!(form);
    }
    let env = TestEnv::new();
    let metrics = Metrics::none();
    let work = metrics.counted_work().clone();
    let composition = Composition::default()
        .with(goodfetch_engine::contribution())
        .with(goodfetch_page::contribution_with(
            (Vec::new(), Launcher::Executable),
            product,
            std::sync::Arc::new(NoBlocks),
        ));
    let identity = Identity {
        product: Product::Collie,
        product_version: ProductVersion::parse("1.4.0").expect("invariant: a version"),
        channel: Channel::CratesIo,
    };
    let engine = Engine::new(identity, &composition)
        .expect("invariant: the composition composes")
        .with_metrics(metrics);
    goodfetch_test_support::runtime().block_on(async {
        let (network, dialer) = transport::in_memory();
        let client = async {
            let (reader, mut writer) = dialer
                .connect()
                .await
                .expect("invariant: the engine listens")
                .split();
            let mut lines = BufReader::new(reader).lines();
            let mut reply = async |id: i64, method: &str, params: &Value| {
                let mut line = serde_json::to_vec(
                    &json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }),
                )
                .expect("invariant: JSON");
                line.push(b'\n');
                writer.write_all(&line).await.expect("invariant: in memory");
                loop {
                    let line = lines
                        .next_line()
                        .await
                        .expect("invariant: in memory")
                        .expect("invariant: the engine answers");
                    let message: Value =
                        serde_json::from_str(&line).expect("invariant: the engine writes JSON");
                    if message["id"] == json!(id) {
                        return message;
                    }
                }
            };
            let hello = reply(0, "hello", &hello).await;
            assert_eq!(hello["result"]["product"], "collie");
            let before = work.read(Row::MessageDecodes);
            let answer = reply(1, method, params).await;
            let decodes = work.read(Row::MessageDecodes) - before;
            drop(reply);
            writer.shutdown().await.expect("invariant: in memory");
            (answer, decodes)
        };
        let (served, answer) = tokio::join!(
            engine.run_attached(Network::InMemory(network), dependencies(&env)),
            client
        );
        served.expect("invariant: the engine got its Connection");
        answer
    })
}

/// A call on a page the Product serves reaches the Product, whether the
/// call names its page itself or through its locator, and decodes its
/// params once: the guard finds the page in the pass that reads them into
/// the handler's type.
/// Covers: operation:page.goto, operation:locator.click
#[rstest]
#[case::by_its_page_id(
    "page.goto",
    json!({ "page_id": SERVED, "url": "https://a.test/" }),
)]
#[case::by_its_locator(
    "locator.click",
    json!({ "locator": { "selector": "#go", "page_id": SERVED } }),
)]
fn a_call_on_a_served_page_decodes_its_params_once(#[case] method: &str, #[case] params: Value) {
    let (reply, decodes) = called(method, &params);

    assert_eq!((&reply["result"], decodes), (&json!(method), 1), "{reply}");
}

/// A call on a page a crawl handler's call encloses, which would wait for
/// ever, ends at that call's end and names the handler call as `ended_by`
/// (ADR 0080).
/// Covers: story:11-41, story:11-43
#[rstest]
#[case::by_its_page_id(
    "page.goto",
    json!({ "page_id": SERVED, "url": "https://a.test/" }),
)]
#[case::by_its_locator(
    "locator.click",
    json!({ "locator": { "selector": "#go", "page_id": SERVED } }),
)]
fn a_call_on_an_enclosed_page_ends_at_the_enclosing_calls_end(
    #[case] method: &str,
    #[case] params: Value,
) {
    let (reply, _) = called_on(Enclosed(&TIMEOUT_HANDLER), method, &params);

    assert_eq!(
        &reply["error"]["data"],
        &json!({
            "code": "deadline_exceeded",
            "retryable": true,
            "limit": { "key": "timeout.handler", "value": 60_000, "layer": "default" },
            "elapsed": 10_000,
            "waited_for": "answer",
            "ended_by": { "operation": "crawl_request.handle", "call_id": 3 },
        }),
        "{reply}"
    );
}

/// The same failed call heads its message with its operation in the form
/// the Client's `hello` declares, and names it canonically in `operation`
/// (ADRs 0079 and 0099): a locator action and the page and locator waits,
/// each on a page that is gone. A `hello` that declares no form takes
/// its surface's, here Rust's.
#[rstest]
#[case::action_typescript(Some("typescript"), "locator.click", "locator.click")]
#[case::action_python(Some("python"), "locator.click", "locator.click")]
#[case::action_rust(Some("rust"), "locator.click", "locator.click")]
#[case::page_wait_typescript(Some("typescript"), "page.wait_for_url", "page.waitForURL")]
#[case::page_wait_python(Some("python"), "page.wait_for_url", "page.wait_for_url")]
#[case::page_wait_rust(Some("rust"), "page.wait_for_url", "page.wait_for_url")]
#[case::locator_wait_typescript(Some("typescript"), "locator.wait_for", "locator.waitFor")]
#[case::locator_wait_python(Some("python"), "locator.wait_for", "locator.wait_for")]
#[case::locator_wait_rust(Some("rust"), "locator.wait_for", "locator.wait_for")]
#[case::page_wait_undeclared(None, "page.wait_for_url", "page.wait_for_url")]
fn a_failed_call_names_its_operation_in_the_declared_form(
    #[case] form: Option<&str>,
    #[case] method: &str,
    #[case] spelled: &str,
) {
    let params = match method {
        "page.wait_for_url" => {
            json!({ "page_id": "gone", "url": { "kind": "glob", "pattern": "**/next" } })
        }
        _ => json!({ "locator": { "selector": "#go", "page_id": "gone" } }),
    };

    let (reply, _) = called_in(form, Served, method, &params);

    assert_eq!(
        (
            &reply["error"]["message"],
            &reply["error"]["data"]["operation"]
        ),
        (
            &json!(format!(
                "{spelled}: the page `gone` is closed, or its Run ended"
            )),
            &json!(method),
        ),
        "{reply}"
    );
}

/// A failure names a setting's key as the Client's `hello` declares its
/// form spells it, and its `limit` keeps it canonical (ADR 0099): the
/// enclosing call's timeout that ended a page's call.
#[rstest]
#[case::typescript("typescript", "`timeout.firstByte` (")]
#[case::python("python", "`timeout.first_byte` (")]
#[case::rust("rust", "`timeout::FIRST_BYTE` (")]
fn a_failure_names_its_setting_in_the_declared_form(#[case] form: &str, #[case] spelled: &str) {
    let params = json!({ "page_id": SERVED, "url": "https://a.test/" });

    let (reply, _) = called_in(
        Some(form),
        Enclosed(&TIMEOUT_FIRST_BYTE),
        "page.goto",
        &params,
    );

    let message = reply["error"]["message"].as_str().unwrap_or_default();
    assert!(message.contains(spelled), "{reply}");
    assert_eq!(
        reply["error"]["data"]["limit"]["key"],
        json!("timeout.first_byte"),
        "{reply}"
    );
}

/// A failure names a call's parameter as the Client's `hello` declares its
/// form spells it, and its details keep it canonical (ADR 0099):
/// `page.request`'s `max_redirects` past its count.
#[rstest]
#[case::typescript("typescript", "`maxRedirects` is 1000")]
#[case::python("python", "`max_redirects` is 1000")]
#[case::rust("rust", "`max_redirects` is 1000")]
fn a_failure_names_its_parameter_in_the_declared_form(#[case] form: &str, #[case] spelled: &str) {
    let params = json!({ "page_id": "gone", "url": "https://a.test/", "max_redirects": 1000 });

    let (reply, _) = called_in(Some(form), Served, "page.request", &params);

    let message = reply["error"]["message"].as_str().unwrap_or_default();
    assert!(message.contains(spelled), "{reply}");
    assert_eq!(
        reply["error"]["data"]["key"],
        json!("max_redirects"),
        "{reply}"
    );
}
