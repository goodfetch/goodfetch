//! A Rust program's pages (issue 2803, Rust's part of issue 1236), on the
//! linked engine, which launches the browser of the Browser cache in the
//! test's Goodfetch home and reaches the test's own Proving ground through
//! its user configuration: the `page` and `page.request` client
//! conformance cases in Rust's mode, `linked` (Rust starts no Attached
//! engine), and the Response `goto`, `go_back`, `go_forward`, `request` and
//! `wait_for_request_finished` answer, whose body reads, decodes and fails as
//! ADRs 0085, 0118 and 0119 say.

use std::collections::BTreeMap;

use goodfetch_collie::settings::{Settings, proxy};
use goodfetch_collie::{
    Collie, Error, ErrorCode, FieldLocator, FieldTransform, PageField, Response, UnreadField,
    UnreadReason, ValueField, blocking,
};
use goodfetch_conformance::{ClientCaseRun, RustMode};
use goodfetch_protocol::page::route::UrlMatcher;
use goodfetch_protocol::page::{
    FormatsRequest, MarkdownContent, MarkdownFormat, MarkdownOptions, RedirectLimit,
    TooManyRedirectsDetails,
};
use goodfetch_protocol::run::Lifecycle;
use goodfetch_protocol::settings::SettingInvalidDetails;
use goodfetch_test_support::{
    ProvingGround, TestEnv, ToolStatus, browser_cache_in_home, case_logs, real_time_runtime,
};
use serde::Deserialize;
use serde_json::{Number, Value, json};

use super::options;

/// The Proving ground's own seed for these tests.
const SEED: u64 = 2803;

/// The Proving ground's host of edge cases of the HTTP layer.
const FETCH_EDGE: &str = "fetch-edge.test";

/// A Proving ground of the test's own, which the engine reaches
/// through the test's user configuration, launching the browser of the
/// Browser cache laid out in the test's Goodfetch home.
fn proving_ground(env: &TestEnv) -> ProvingGround {
    browser_cache_in_home(env).unwrap();
    let proving_ground = ProvingGround::start(env, SEED).unwrap();
    env.user_configuration(&proving_ground.description().reaching());
    proving_ground
}

/// The Error code of the engine's failure `result` is, and the failure.
fn failure<T: std::fmt::Debug>(result: Result<T, Error>) -> (String, Error) {
    let Err(failure) = result else {
        panic!("{result:?}");
    };
    (failure.code().to_string(), failure)
}

/// A value field of the element the CSS selector `selector` finds, read
/// as its text.
fn css(selector: &str) -> ValueField {
    ValueField {
        locator: FieldLocator::Css {
            selector: selector.to_owned(),
        },
        fallbacks: Vec::new(),
        attribute: None,
        matched: None,
        transform: FieldTransform::Text,
    }
}

/// The fields of the Proving ground's product page the `page-read-fields`
/// case reads, as the TypeScript SDK's test declares them: its ID, name
/// and price, a number read from an attribute, and a field no element
/// gives.
fn product_fields() -> BTreeMap<String, PageField> {
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
        (
            "missing".to_owned(),
            PageField::Value(css(".no-such-element")),
        ),
    ])
}

/// The product page's fields as a program's type reads them.
#[derive(Debug, Deserialize)]
struct Product {
    id: Option<String>,
    name: Option<String>,
    price: Option<Number>,
    missing: Option<String>,
}

impl Product {
    /// The Record the oracle judges: the fields the page gives.
    fn record(&self) -> Value {
        json!({ "id": self.id, "name": self.name, "price": self.price })
    }
}

/// The `page-form-program` client conformance case, linked: a program
/// opens a page, sends it to the Proving ground's form page, fills a
/// field, clicks a box, presses Enter on the button, reads the page's
/// Markdown once it shows its end state, closes the page and finishes the
/// Run, as the TypeScript SDK's form program does.
/// Covers: story:11-77
#[test]
fn a_program_fills_the_proving_grounds_form_and_reads_its_markdown() {
    let _case = ClientCaseRun::start(
        case_logs().as_deref(),
        "page-form-program",
        RustMode::Linked,
    )
    .unwrap();
    let env = TestEnv::new();
    let proving_ground = proving_ground(&env);
    let form = proving_ground.start_url("actionability/20").unwrap();
    real_time_runtime().block_on(async {
        let collie = Collie::connect(options(&env)).await.unwrap();
        let run = collie.start().await.unwrap();
        let page = run.new_page().await.unwrap();

        let navigated = page.goto(&form).await.unwrap();
        page.locator("#name").fill("Goodfetch").await.unwrap();
        page.locator("#consent").click().await.unwrap();
        page.locator("#submit").press("Enter").await.unwrap();
        page.locator("#done:not(:empty)").wait_for().await.unwrap();
        let formats = page
            .formats(FormatsRequest {
                markdown: Some(MarkdownFormat::Options(MarkdownOptions {
                    content: MarkdownContent::Full,
                })),
                ..FormatsRequest::default()
            })
            .await
            .unwrap();
        page.close().await.unwrap();
        let report = run.finish().await.unwrap();
        collie.close().await.unwrap();

        assert_eq!(navigated.map(|response| response.status()), Some(200));
        let markdown = formats.into_inner().markdown.unwrap_or_default();
        assert!(markdown.contains("done"), "{markdown}");
        assert_eq!(report.lifecycle, Lifecycle::Finished);
    });
}

/// The `goto-404-returns-the-response` client conformance case, linked: a
/// 404 is a value, the document's Response, which is not `ok`.
/// Covers: story:11-84
#[test]
fn goto_answers_the_response_of_a_404() {
    let _case = ClientCaseRun::start(
        case_logs().as_deref(),
        "goto-404-returns-the-response",
        RustMode::Linked,
    )
    .unwrap();
    let env = TestEnv::new();
    let proving_ground = proving_ground(&env);
    let start = proving_ground.start_url("statuses/1").unwrap();
    let missing = url::Url::parse(&start)
        .unwrap()
        .join("/errors/404")
        .unwrap()
        .to_string();
    real_time_runtime().block_on(async {
        let collie = Collie::connect(options(&env)).await.unwrap();
        let run = collie.start().await.unwrap();
        let page = run.new_page().await.unwrap();

        let navigated = page.goto(&missing).await.unwrap();
        run.finish().await.unwrap();
        collie.close().await.unwrap();

        let response = navigated.expect("a 404 is a response");
        assert_eq!(
            (response.status(), response.url(), response.ok()),
            (404, missing.as_str(), false)
        );
    });
}

/// The `goto-skips-a-redirects-retry-after-and-answers-none-for-a-file`
/// client conformance case, linked: a program's own `goto` is outside
/// Politeness, so it follows a 302 whose `Retry-After` asks for an hour, far
/// past `timeout.navigation`, at once (ADRs 0086, 0118); a `goto` to a PDF
/// answers no response and the page keeps its document, which a reload
/// loads again (ADR 0119).
/// Covers: story:11-84, operation:page.goto
#[test]
fn goto_skips_a_redirects_retry_after_and_answers_none_for_a_file() {
    let _case = ClientCaseRun::start(
        case_logs().as_deref(),
        "goto-skips-a-redirects-retry-after-and-answers-none-for-a-file",
        RustMode::Linked,
    )
    .unwrap();
    let env = TestEnv::new();
    let proving_ground = proving_ground(&env);
    let redirect = proving_ground.url(FETCH_EDGE, "/retry-after/302/seconds/long");
    let target = proving_ground.url(FETCH_EDGE, "/target");
    let document = proving_ground.start_url("catalog/2").unwrap();
    let file = url::Url::parse(&proving_ground.start_url("files/2").unwrap())
        .unwrap()
        .join("/files/report.pdf")
        .unwrap()
        .to_string();
    real_time_runtime().block_on(async {
        let collie = Collie::connect(options(&env)).await.unwrap();
        let run = collie.start().await.unwrap();
        let page = run.new_page().await.unwrap();

        let followed = page.goto(&redirect).await.unwrap();
        page.goto(&document).await.unwrap();
        let downloaded = page.goto(&file).await.unwrap();
        let reloaded = page.reload().await.unwrap();
        run.finish().await.unwrap();
        collie.close().await.unwrap();

        let head = |response: Option<Response>| {
            response.map(|response| (response.status(), response.url().to_owned()))
        };
        assert_eq!(head(followed), Some((200, target)));
        assert!(
            downloaded.is_none(),
            "a file the browser downloads answers no response"
        );
        assert_eq!(head(reloaded), Some((200, document)));
    });
}

/// The `second-new-page-in-the-default-session` client conformance case,
/// linked: a Run's second page shares the first's Session, its storage
/// included, and a page of a new Session reads none of it.
/// Covers: story:11-77
#[test]
fn a_runs_second_page_opens_in_its_session_and_a_new_sessions_page_in_another() {
    let _case = ClientCaseRun::start(
        case_logs().as_deref(),
        "second-new-page-in-the-default-session",
        RustMode::Linked,
    )
    .unwrap();
    let env = TestEnv::new();
    let proving_ground = proving_ground(&env);
    let form = proving_ground.start_url("actionability/20").unwrap();
    real_time_runtime().block_on(async {
        let collie = Collie::connect(options(&env)).await.unwrap();
        let run = collie.start().await.unwrap();
        let first = run.new_page().await.unwrap();
        let second = run.new_page().await.unwrap();
        let session = run.new_session().await.unwrap();
        let other = session.new_page().await.unwrap();
        for page in [&first, &second, &other] {
            page.goto(&form).await.unwrap();
        }

        first
            .evaluate("localStorage.setItem('kept', 'by the first page')")
            .await
            .unwrap();
        let shared = second
            .evaluate("localStorage.getItem('kept')")
            .await
            .unwrap();
        let apart = other
            .evaluate("localStorage.getItem('kept')")
            .await
            .unwrap();
        run.finish().await.unwrap();
        collie.close().await.unwrap();

        assert_eq!((shared, apart), (Some(json!("by the first page")), None));
    });
}

/// The `page-read-fields` client conformance case, linked: `page.read`
/// answers the declared fields in one call, read into the program's type,
/// which the oracle verifies, a number field a number and a field no
/// element gives `None`, with why.
/// Covers: story:11-77, operation:page.read
#[test]
fn page_read_answers_the_declared_fields_typed_in_one_call() {
    let _case =
        ClientCaseRun::start(case_logs().as_deref(), "page-read-fields", RustMode::Linked).unwrap();
    let env = TestEnv::new();
    let proving_ground = proving_ground(&env);
    let product = proving_ground.start_url("catalog/2").unwrap();
    let read = real_time_runtime().block_on(async {
        let collie = Collie::connect(options(&env)).await.unwrap();
        let run = collie.start().await.unwrap();
        let page = run.new_page().await.unwrap();
        page.goto(&product).await.unwrap();

        let read = page
            .read(product_fields())
            .typed::<Product>()
            .await
            .unwrap();
        run.finish().await.unwrap();
        collie.close().await.unwrap();
        read
    });

    let answer = proving_ground
        .judge_records("catalog/2", ToolStatus::Success, &[read.values.record()])
        .unwrap();
    assert_eq!(answer["outcome"], "verified", "{answer}");
    assert!(read.values.price.is_some(), "{read:?}");
    assert_eq!(read.values.missing, None);
    assert_eq!(
        read.unread,
        [UnreadField {
            field: "/missing".to_owned(),
            reason: UnreadReason::NoElement,
            text: None,
        }]
    );
}

/// The `page-read-fields` client conformance case through `blocking`:
/// the same fields, read into the same type, by a program without async.
/// Covers: story:11-77, operation:page.read
#[test]
fn page_read_answers_the_declared_fields_typed_through_blocking() {
    let _case =
        ClientCaseRun::start(case_logs().as_deref(), "page-read-fields", RustMode::Linked).unwrap();
    let env = TestEnv::new();
    let proving_ground = proving_ground(&env);
    let product = proving_ground.start_url("catalog/2").unwrap();
    let collie = blocking::Collie::connect(options(&env)).unwrap();
    let run = collie.start().send().unwrap();
    let page = run.new_page().send().unwrap();
    page.goto(&product).send().unwrap();

    let read = page
        .read(product_fields())
        .typed::<Product>()
        .send()
        .unwrap();
    run.finish().send().unwrap();
    collie.close().unwrap();

    let answer = proving_ground
        .judge_records("catalog/2", ToolStatus::Success, &[read.values.record()])
        .unwrap();
    assert_eq!(answer["outcome"], "verified", "{answer}");
    assert_eq!(read.values.missing, None);
    assert_eq!(read.unread.len(), 1, "{read:?}");
}

/// The `closed-page-raises-closed` client conformance case, linked: a
/// closed page, and a Locator built on it before, refuse their calls with
/// `closed` before sending them.
/// Covers: story:11-77, story:11-17
#[test]
fn a_closed_page_and_its_locators_refuse_their_calls_with_closed() {
    let _case = ClientCaseRun::start(
        case_logs().as_deref(),
        "closed-page-raises-closed",
        RustMode::Linked,
    )
    .unwrap();
    let env = TestEnv::new();
    let proving_ground = proving_ground(&env);
    let form = proving_ground.start_url("actionability/20").unwrap();
    real_time_runtime().block_on(async {
        let collie = Collie::connect(options(&env)).await.unwrap();
        let run = collie.start().await.unwrap();
        let page = run.new_page().await.unwrap();
        page.goto(&form).await.unwrap();
        let button = page.locator("#submit");
        page.close().await.unwrap();

        let navigation = page.goto(&form).await;
        let click = button.click().await;
        run.finish().await.unwrap();
        collie.close().await.unwrap();

        assert!(
            matches!(&navigation, Err(error) if error.code() == ErrorCode::CLOSED),
            "{navigation:?}"
        );
        assert!(
            matches!(&click, Err(error) if error.code() == ErrorCode::CLOSED),
            "{click:?}"
        );
    });
}

/// The `page-request-sends-the-sessions-cookie-through-its-proxy` client
/// conformance case, linked: on a Run whose `proxy.server` is the Proving
/// ground's open proxy, `page.request` sends the cookie a page of its
/// Session got, and leaves through that proxy, as the Proving ground's
/// record shows (ADR 0085).
/// Covers: story:11-87, operation:page.request
#[test]
fn page_request_sends_the_sessions_cookie_through_its_proxy() {
    let _case = ClientCaseRun::start(
        case_logs().as_deref(),
        "page-request-sends-the-sessions-cookie-through-its-proxy",
        RustMode::Linked,
    )
    .unwrap();
    let env = TestEnv::new();
    let proving_ground = proving_ground(&env);
    let open_proxy = &proving_ground.description().fixtures.proxies["open"];
    let storage = proving_ground.url("scope.test", "/storage?mark=page-request");
    let about = proving_ground.url("scope.test", "/about?by=page-request");
    let status = real_time_runtime().block_on(async {
        let collie = Collie::connect(options(&env)).await.unwrap();
        let run = collie
            .start()
            .settings(Settings::new().set(proxy::SERVER, format!("http://{open_proxy}")))
            .await
            .unwrap();
        let page = run.new_page().await.unwrap();
        page.goto(&storage).await.unwrap();

        let status = page.request(&about).await.unwrap().status();
        run.finish().await.unwrap();
        collie.close().await.unwrap();
        status
    });

    assert_eq!(status, 200);
    let sent: Vec<Vec<(String, String)>> = proving_ground
        .received_requests()
        .unwrap()
        .of_the_case()
        .filter(|request| {
            request.host == "scope.test" && request.target == "/about?by=page-request"
        })
        .map(|request| request.cookies.clone())
        .collect();
    assert_eq!(sent, [vec![("mark".to_owned(), "page-request".to_owned())]]);
    let observations = proving_ground.observations().unwrap();
    let relayed = observations["relays"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|relay| {
            relay["proxy"] == json!("open")
                && relay["outcome"] == json!("relayed")
                && relay["target"]
                    .as_str()
                    .is_some_and(|target| target.ends_with("/about?by=page-request"))
        });
    assert!(relayed, "{}", observations["relays"]);
}

/// The `page-request-follows-its-max-redirects-and-refuses-a-count-above-19`
/// client conformance case, linked, on the Proving ground's chain of 5
/// redirects: `max_redirects` 20 is refused with `setting_invalid` naming
/// the option, 0 answers the 3xx itself, 5 answers the chain's end, and 3
/// raises `too_many_redirects` naming the chain, its 3 hops and the option
/// with its count (ADR 0118).
/// Covers: story:11-87, operation:page.request, error:too_many_redirects, error:setting_invalid
#[test]
fn page_request_follows_its_max_redirects_and_refuses_a_count_above_19() {
    let _case = ClientCaseRun::start(
        case_logs().as_deref(),
        "page-request-follows-its-max-redirects-and-refuses-a-count-above-19",
        RustMode::Linked,
    )
    .unwrap();
    let env = TestEnv::new();
    let proving_ground = proving_ground(&env);
    let chain = proving_ground.url(FETCH_EDGE, "/redirect/hops/5");
    real_time_runtime().block_on(async {
        let collie = Collie::connect(options(&env)).await.unwrap();
        let run = collie.start().await.unwrap();
        let page = run.new_page().await.unwrap();

        let refused = page.request(&chain).max_redirects(20).await;
        let unfollowed = page.request(&chain).max_redirects(0).await.unwrap();
        let within = page.request(&chain).max_redirects(5).await.unwrap();
        let past = page.request(&chain).max_redirects(3).await;
        run.finish().await.unwrap();
        collie.close().await.unwrap();

        let (code, refused) = failure(refused);
        assert_eq!(code, "setting_invalid");
        assert_eq!(
            refused
                .details::<SettingInvalidDetails>()
                .map(|details| details.key),
            Some("max_redirects".to_owned()),
            "{refused:?}"
        );
        assert_eq!(unfollowed.status(), 302);
        assert!(
            unfollowed.url().ends_with("/redirect/hops/5"),
            "{unfollowed:?}"
        );
        assert_eq!(
            unfollowed.headers().get("location").map(String::as_str),
            Some("/redirect/hops/4")
        );
        assert_eq!(within.status(), 200);
        assert!(within.url().ends_with("/redirect/hops/0"), "{within:?}");
        let (code, past) = failure(past);
        assert_eq!(code, "too_many_redirects");
        let details = past
            .details::<TooManyRedirectsDetails>()
            .expect("too_many_redirects details");
        assert_eq!(
            (details.url, details.hops.len(), details.limit),
            (
                chain,
                3,
                RedirectLimit::Option {
                    name: "max_redirects".to_owned(),
                    count: 3,
                }
            )
        );
    });
}

/// A navigation's Response gives its head, and its text decodes the
/// document with the encoding the browser decoded it with; `ok` is true
/// for a 200.
/// Covers: story:11-84, story:11-85, operation:response.body
#[test]
fn a_navigations_response_gives_its_head_and_its_documents_text() {
    let env = TestEnv::new();
    let proving_ground = proving_ground(&env);
    let product = proving_ground.start_url("catalog/2").unwrap();
    real_time_runtime().block_on(async {
        let collie = Collie::connect(options(&env)).await.unwrap();
        let run = collie.start().await.unwrap();
        let page = run.new_page().await.unwrap();

        let response = page.goto(&product).await.unwrap().expect("a document");
        let text = response.text().await.unwrap();
        let body = response.body().await.unwrap();
        run.finish().await.unwrap();
        collie.close().await.unwrap();

        assert_eq!(
            (response.status(), response.url(), response.ok()),
            (200, product.as_str(), true)
        );
        let content_type = response
            .headers()
            .get("content-type")
            .cloned()
            .unwrap_or_default();
        assert!(content_type.starts_with("text/html"), "{content_type}");
        assert_eq!(response.encoding(), Some("UTF-8"));
        assert!(text.contains("<h1 class=\"name\">"), "{text}");
        assert_eq!(text, String::from_utf8_lossy(&body));
    });
}

/// `text()` honours a legacy charset, here `Shift_JIS` from the
/// `Content-Type`, and `json()` ignores a leading BOM, on the Responses
/// `page.request` answers; `json()` of a body that is no JSON fails with
/// `response_not_json`, naming the response's URL.
/// Covers: story:11-85, operation:page.request, operation:response.body
#[test]
fn text_honours_a_legacy_charset_and_json_ignores_a_bom() {
    let env = TestEnv::new();
    let proving_ground = proving_ground(&env);
    let shift_jis = proving_ground.url(FETCH_EDGE, "/text/shift-jis");
    let bom = proving_ground.url(FETCH_EDGE, "/json/bom");
    real_time_runtime().block_on(async {
        let collie = Collie::connect(options(&env)).await.unwrap();
        let run = collie.start().await.unwrap();
        let page = run.new_page().await.unwrap();

        let text_response = page.request(&shift_jis).await.unwrap();
        let japanese = text_response.text().await;
        let not_json = text_response.json::<Value>().await;
        let json = page.request(&bom).await.unwrap().json::<Value>().await;
        run.finish().await.unwrap();
        collie.close().await.unwrap();

        assert_eq!(japanese.unwrap(), "日本語");
        assert_eq!(json.unwrap(), json!({ "bom": true }));
        assert!(
            matches!(&not_json, Err(error) if error.code() == ErrorCode::RESPONSE_NOT_JSON && error.url() == Some(shift_jis.as_str())),
            "{not_json:?}"
        );
    });
}

/// The response `wait_for_request_finished` answers keeps its body once
/// the page has navigated away, where the navigation's own Response's
/// body is dropped: the waiter armed before the action catches its
/// request, and gives that request beside its response.
/// Covers: story:11-85, operation:page.wait_for_request_finished, operation:response.body
#[test]
fn a_finished_requests_response_reads_whole_after_the_page_moves_on() {
    let env = TestEnv::new();
    let proving_ground = proving_ground(&env);
    let product = proving_ground.start_url("catalog/2").unwrap();
    let then = proving_ground.start_url("actionability/20").unwrap();
    real_time_runtime().block_on(async {
        let collie = Collie::connect(options(&env)).await.unwrap();
        let run = collie.start().await.unwrap();
        let page = run.new_page().await.unwrap();

        let finished = page.wait_for_request_finished().url(UrlMatcher::Glob {
            pattern: product.clone(),
        });
        let (exchange, _) = tokio::join!(finished, page.goto(&product));
        let exchange = exchange.unwrap();
        page.goto(&then).await.unwrap();
        let text = exchange.response().text().await;
        run.finish().await.unwrap();
        collie.close().await.unwrap();

        assert_eq!(
            (
                exchange.request().url.as_str(),
                exchange.request().method.as_str()
            ),
            (product.as_str(), "GET")
        );
        assert_eq!(exchange.response().status(), 200);
        let text = text.unwrap();
        assert!(text.contains("<h1 class=\"name\">"), "{text}");
    });
}

/// A body cut mid-transfer raises `response_truncated` from `body()`,
/// `text()` and `json()`, never read as whole.
/// Covers: story:11-85, operation:response.body, error:response_truncated
#[test]
fn a_cut_body_raises_response_truncated_from_body_text_and_json() {
    let env = TestEnv::new();
    let proving_ground = proving_ground(&env);
    let cut = proving_ground.url(FETCH_EDGE, "/cut/content-length");
    real_time_runtime().block_on(async {
        let collie = Collie::connect(options(&env)).await.unwrap();
        let run = collie.start().await.unwrap();
        let page = run.new_page().await.unwrap();

        let response = page.request(&cut).await.unwrap();
        let codes = [
            failure(response.body().await).0,
            failure(response.text().await).0,
            failure(response.json::<Value>().await).0,
        ];
        run.finish().await.unwrap();
        collie.close().await.unwrap();

        assert_eq!(codes, ["response_truncated"; 3]);
    });
}

/// A body the browser dropped once the page navigated away raises
/// `body_unavailable`, whose fix names `page.wait_for_request_finished`.
/// Covers: story:11-85, operation:response.body, error:body_unavailable
#[test]
fn a_dropped_body_raises_body_unavailable_naming_the_waiter() {
    let env = TestEnv::new();
    let proving_ground = proving_ground(&env);
    let first = proving_ground.start_url("catalog/2").unwrap();
    let then = proving_ground.start_url("actionability/20").unwrap();
    real_time_runtime().block_on(async {
        let collie = Collie::connect(options(&env)).await.unwrap();
        let run = collie.start().await.unwrap();
        let page = run.new_page().await.unwrap();

        let left = page.goto(&first).await.unwrap().expect("a document");
        page.goto(&then).await.unwrap();
        let (code, error) = failure(left.text().await);
        run.finish().await.unwrap();
        collie.close().await.unwrap();

        assert_eq!(code, "body_unavailable");
        let fix = error.fix().unwrap_or_default();
        assert!(fix.contains("wait_for_request_finished"), "{error:?}");
    });
}

/// Back and forward answer the document's Response, and none at the end
/// of the history.
/// Covers: story:11-84, operation:page.go_back, operation:page.go_forward
#[test]
fn back_and_forward_answer_the_documents_response_and_none_past_the_history() {
    let env = TestEnv::new();
    let proving_ground = proving_ground(&env);
    let first = proving_ground.start_url("catalog/2").unwrap();
    let then = proving_ground.start_url("actionability/20").unwrap();
    real_time_runtime().block_on(async {
        let collie = Collie::connect(options(&env)).await.unwrap();
        let run = collie.start().await.unwrap();
        let page = run.new_page().await.unwrap();
        page.goto(&first).await.unwrap();
        page.goto(&then).await.unwrap();

        let back = page.go_back().await.unwrap();
        let forward = page.go_forward().await.unwrap();
        let past = page.go_forward().await.unwrap();
        run.finish().await.unwrap();
        collie.close().await.unwrap();

        let url = |response: Option<Response>| response.map(|response| response.url().to_owned());
        assert_eq!(
            (url(back), url(forward), past.is_none()),
            (Some(first), Some(then), true)
        );
    });
}
