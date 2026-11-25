//! The Sessions table against a Run that stops while a Session opens
//! (#1763), and a page that ends past its memory cap: an in-process engine
//! of the engine's operations, the page crate's and the test's own, on the
//! test `Clock` and `Entropy` with blocking work inline, whose test
//! operations drive the table themselves, step by step. One opens a Run's
//! default Session and stops the Run while the open waits for its record in
//! run state: a Session left in the table past its Run's clean-up would
//! hold the Run, its browsers and its egress relay forever. The other ends
//! a page past its memory cap as its watch does: a page found in neither
//! the open pages nor the ended ones would answer `closed`. A last one
//! reads which Sessions interception alone holds (#2435), and one which
//! Session's end the run file waits for (#2168). Another panics in a task
//! of a Session's owner, which closes that Session alone (#3263), and one
//! in a task of a browser's owner, which is that browser's crash (#3265).
#![expect(
    clippy::disallowed_methods,
    reason = "tests run on the test Clock and Entropy (ADR 0059)"
)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::task::Poll;
use std::time::Duration;

use goodfetch_engine::clock::Clock;
use goodfetch_engine::egress::CdpUrlOrigin;
use goodfetch_engine::entropy::Entropy;
use goodfetch_engine::home::{Home, RunStateLocation};
use goodfetch_engine::run_state::RunStateOptions;
use goodfetch_engine::runs::{EndedBy, Run, Start, Started};
use goodfetch_engine::settings::{Opening, RunSettings};
use goodfetch_engine::tasks::BlockingPolicy;
use goodfetch_engine::transport::{self, Network};
use goodfetch_engine::{
    CallContext, CallError, Channel, Composition, Contribution, Dependencies, Engine, Handler,
    Identity,
};
use goodfetch_protocol::catalog::{
    Operation, OperationClass, OperationKind, Product, Reach, Stability, schema_of,
};
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::governor::BROWSER_PAGE_MEMORY_CAP;
use goodfetch_protocol::page::PageId;
use goodfetch_protocol::run::{RunId, RunKind, Tags};
use goodfetch_protocol::settings::{SettingLimit, SettingsLayer};
use goodfetch_test_support::{OUTCOME_TARGET, TestEnv, capturing};
use rstest::rstest;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader, DuplexStream};
use tokio::sync::watch;

use super::life::{Ending, PageLife};
use super::{Opener, Sessions, held_browser, set_outside_code};
use crate::borrowed::CdpUrl;
use crate::dialogs::Dialogs;
use crate::interception::{HeldBrowser, Interception};
use crate::launch::Launcher;
use crate::page_value::ProgramPages;

/// Opens a Run's default Session and stops the Run while the open waits
/// for its record.
const STOPPED_OPEN: Operation = Operation {
    name: "test.stopped_open",
    description: "Stops a Run while its default Session opens.",
    class: OperationClass::Effect,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<Value>,
    result: schema_of::<Value>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: goodfetch_protocol::catalog::PROTOCOL_VERSION,
};

/// Ends a page past its memory cap and confirms its close, as the page's
/// watch does when its target's end arrives first.
const ENDED_PAST_CAP: Operation = Operation {
    name: "test.ended_past_cap",
    description: "Ends a page past its memory cap and reads it after.",
    ..STOPPED_OPEN
};

/// Opens a Session as its opener says and ends it.
const SESSION_ENDED: Operation = Operation {
    name: "test.session_ended",
    description: "Opens a Session and ends it, counting the commits its end made.",
    ..STOPPED_OPEN
};

/// Opens the Run's default Session and panics in a task of its owner.
const SESSION_PANICKED: Operation = Operation {
    name: "test.session_panicked",
    description: "Panics in a task of a Session's owner and calls the Session after.",
    ..STOPPED_OPEN
};

/// Panics in a task of the owner of a Session's browser.
const BROWSER_PANICKED: Operation = Operation {
    name: "test.browser_panicked",
    description: "Panics in a task of a browser's owner and reads what its Session and its pages meet.",
    ..STOPPED_OPEN
};

/// The Run `run_id`, started for the test's own operation.
async fn started(run_id: &RunId, context: &CallContext) -> Result<Run, CallError> {
    started_under(run_id, context, None).await
}

/// The Run `run_id`, started for the test's own operation under
/// `settings`, its resolved settings, when given.
async fn started_under(
    run_id: &RunId,
    context: &CallContext,
    settings: Option<RunSettings>,
) -> Result<Run, CallError> {
    let started = context
        .runs()
        .start(Start {
            run_id: Some(run_id.clone()),
            tags: Tags::default(),
            kind: RunKind::Collie,
            needs: Product::Collie,
            inputs: BTreeMap::new(),
            ended_by: EndedBy::Driver,
            initialize: None,
            policy_version: None,
            weakened: Vec::new(),
            binding: None,
            clean_up_digest: None,
            checkpoint_interval: None,
            timeout: None,
            settings,
            detached: false,
        })
        .await
        .map_err(goodfetch_engine::run::start_error)?;
    let Started::New(run) = started else {
        panic!("the Run began before");
    };
    Ok(run)
}

/// A Sessions table of its own for the test's operation.
fn table() -> Sessions {
    Sessions::new(
        (Vec::new(), Launcher::Executable),
        Interception::default(),
        Dialogs::default(),
        Arc::new(ProgramPages),
    )
}

/// `test.ended_past_cap`: starts the Run `run_id` and reaches it in a
/// Sessions table of its own, then ends one of its pages past its memory
/// cap, confirms the close and acts on its end, as the page's watch does
/// when its target's end comes before the closing call's answer. Answers
/// the code a call on the page raises after.
async fn ended_past_cap(params: Value, context: CallContext) -> Result<Value, CallError> {
    let run_id = RunId::parse(params["run_id"].as_str().unwrap()).unwrap();
    let run = started(&run_id, &context).await?;
    let sessions = table();
    sessions.reach(&run, &context).await?;
    let page_id = PageId("page-1".to_owned());
    let life = PageLife::new(page_id.clone(), None, watch::channel(0).1);
    let limit = SettingLimit {
        key: BROWSER_PAGE_MEMORY_CAP.name.to_owned(),
        value: json!(32 << 20),
        layer: SettingsLayer::Run,
        origin: None,
    };
    life.end(&Ending::MemoryExceeded {
        memory: 64 << 20,
        limit,
    })?;
    let confirmed = life.end(&Ending::Destroyed)?;

    sessions.page_ended(&page_id, &run, &life, confirmed).await;

    let raised = sessions
        .page(&page_id)
        .await
        .err()
        .map(|error| error.code());
    run.cancellation().cancel();
    Ok(json!({ "raised": raised }))
}

/// `test.stopped_open`: starts the Run `run_id`, reaches it in a Sessions
/// table of its own and opens its default Session, polled by hand until it
/// took its position and waits; then stops the Run and lets the table's
/// clean-up take the Run out before the open goes on. Answers whether the
/// open succeeded, and how many Sessions of the Run the table holds after.
async fn stopped_open(params: Value, context: CallContext) -> Result<Value, CallError> {
    let run_id = RunId::parse(params["run_id"].as_str().unwrap()).unwrap();
    let run = started(&run_id, &context).await?;
    let sessions = table();
    sessions.reach(&run, &context).await?;
    let positioned = async || {
        sessions
            .state
            .lock()
            .await
            .runs
            .get(run.id())
            .is_some_and(|runs| runs.last > 0)
    };
    let opened = {
        let mut open =
            std::pin::pin!(sessions.open_held(&run, None, None, Opener::Default, &context));
        loop {
            let polled = std::future::poll_fn(|waker| Poll::Ready(open.as_mut().poll(waker))).await;
            assert!(polled.is_pending(), "the open ended before the Run stopped");
            if positioned().await {
                break;
            }
            tokio::task::yield_now().await;
        }
        run.cancellation().cancel();
        while sessions.state.lock().await.runs.contains_key(run.id()) {
            tokio::task::yield_now().await;
        }
        open.await
    };
    let left = sessions.index.sessions_of(run.id()).len();
    Ok(json!({ "opened": opened.is_ok(), "left": left }))
}

/// `test.session_ended`: starts the Run `run_id`, reaches it in a
/// Sessions table of its own and opens a Session as `opener` says, the
/// Run's default one or one of its crawl's Session pool; then ends it as a
/// close or a retirement does. Answers how many commits of the Run's run
/// state its end made.
async fn session_ended(params: Value, context: CallContext) -> Result<Value, CallError> {
    let run_id = RunId::parse(params["run_id"].as_str().unwrap()).unwrap();
    let run = started(&run_id, &context).await?;
    let sessions = table();
    sessions.reach(&run, &context).await?;
    let opener = match params["opener"].as_str().unwrap() {
        "default" => Opener::Default,
        _ => Opener::Pool {
            address_ordinal: None,
        },
    };
    let position = sessions
        .open_held(&run, None, None, opener, &context)
        .await?;
    let session = super::session_id(run.id(), position);
    let open = sessions.index.session(&session).unwrap();
    let commits = run.state().commits();
    let before = *commits.borrow();

    sessions.session_ended(&open, &session).await.unwrap();

    let made = *commits.borrow() - before;
    run.cancellation().cancel();
    Ok(json!({ "commits": made }))
}

/// `test.session_panicked`: starts the Run `run_id`, reaches it in a
/// Sessions table of its own and opens its default Session, then panics in
/// a task of the Session's owner and lets the engine close the Session.
/// Answers whether the table still names it, what a call reading its pages
/// raises after, and whether the Run is still live.
async fn session_panicked(params: Value, context: CallContext) -> Result<Value, CallError> {
    let run_id = RunId::parse(params["run_id"].as_str().unwrap()).unwrap();
    let run = started(&run_id, &context).await?;
    let sessions = table();
    sessions.reach(&run, &context).await?;
    let position = sessions
        .open_held(&run, None, None, Opener::Default, &context)
        .await?;
    let session = super::session_id(run.id(), position);
    let open = sessions.index.session(&session).unwrap();

    open.owner
        .spawn(async { panic!("a bug in a task of the Session's owner") });
    drop(open);

    // Bounded, so a Session the panic leaves open fails the test, never hangs it.
    for _ in 0..10_000 {
        if sessions.index.session(&session).is_none() {
            break;
        }
        tokio::task::yield_now().await;
    }
    let named = sessions.index.session(&session).is_some();
    let raised = sessions.pages(&session, &context).await.err();
    let run_live = context.runs().live(run.id().clone()).await.is_ok();
    run.cancellation().cancel();
    Ok(json!({
        "session": session.0,
        "named": named,
        "raised": raised.as_ref().map(CallError::code),
        "details": raised.as_ref().and_then(CallError::details),
        "run_live": run_live,
    }))
}

/// Longer than a browser treated as crashed takes to say so, on the
/// paused clock, which moves on only once every task waits.
const CRASH_KNOWN_WITHIN: Duration = Duration::from_secs(60);

/// Plays a browser with a window on `pipe`, whose commands and answers
/// are JSON texts each ended by a NUL: it answers its version, a browser
/// context's creation and every other command with an empty result, until
/// the engine asks it to close or lets go of the pipe.
async fn played_browser(pipe: DuplexStream) {
    let (reader, mut writer) = tokio::io::split(pipe);
    let mut reader = BufReader::new(reader);
    let mut text = Vec::new();
    while reader.read_until(0, &mut text).await.unwrap() > 0 {
        if text.last() == Some(&0) {
            text.pop();
        }
        let command: Value = serde_json::from_slice(&text).unwrap();
        text.clear();
        let method = command["method"].as_str().unwrap();
        let result = match method {
            "Browser.getVersion" => json!({
                "protocolVersion": "1.3",
                "product": "Scripted/1.0",
                "revision": "0",
                "userAgent": "Scripted",
                "jsVersion": "0",
            }),
            "Target.createBrowserContext" => json!({ "browserContextId": "context" }),
            _ => json!({}),
        };
        let mut answer = json!({ "id": command["id"], "result": result });
        if let Some(session) = command.get("sessionId") {
            answer["sessionId"] = session.clone();
        }
        let mut written = serde_json::to_vec(&answer).unwrap();
        written.push(0);
        if writer.write_all(&written).await.is_err() || method == "Browser.close" {
            return;
        }
    }
}

/// `test.browser_panicked`: starts the Run `run_id` with its browsers
/// shown in a window, reaches it in a Sessions table of its own whose
/// browsers are scripted, and seats its default Session in the Run's first
/// browser; then panics in a task of that browser's owner. Answers how the
/// browser ended, as its pages read it, whether the Session took its crash,
/// and whether the Run is still live.
async fn browser_panicked(params: Value, context: CallContext) -> Result<Value, CallError> {
    let run_id = RunId::parse(params["run_id"].as_str().unwrap()).unwrap();
    let (settings, _) = context
        .engine()
        .settings()
        .open(Opening {
            run: Some(Product::Collie),
            settings: serde_json::from_value(json!({ "browser": { "headless": false } })).unwrap(),
            ..Opening::default()
        })
        .await
        .unwrap();
    let run = started_under(&run_id, &context, Some(settings)).await?;
    let (launcher, mut browsers) = Launcher::scripted(None);
    let sessions = Sessions::new(
        (Vec::new(), launcher),
        Interception::default(),
        Dialogs::default(),
        Arc::new(ProgramPages),
    );
    sessions.reach(&run, &context).await?;
    let position = sessions
        .open_held(&run, None, None, Opener::Default, &context)
        .await?;
    let open = sessions
        .index
        .session(&super::session_id(run.id(), position))
        .unwrap();
    let panicked = async {
        open.seat.open().await?;
        let browser = open.seat.browser(1).unwrap();
        browser
            .tasks()
            .spawn(async { panic!("a bug in a task of the browser's owner") });
        let ended = tokio::time::timeout(CRASH_KNOWN_WITHIN, browser.ended())
            .await
            .ok();
        // Bounded, so a crash the Session never takes fails the test, never
        // hangs it.
        for _ in 0..10_000 {
            if open.seat.crash_count_of(1).initialized() {
                break;
            }
            tokio::task::yield_now().await;
        }
        Ok::<_, CallError>(json!({
            "ended": ended.map(|lost| format!("{lost:?}")),
            "crash_taken": open.seat.crash_count_of(1).initialized(),
            "run_live": context.runs().live(run.id().clone()).await.is_ok(),
        }))
    };
    let played = async {
        if let Some(pipe) = browsers.next().await {
            played_browser(pipe).await;
        }
    };
    let (panicked, ()) = tokio::join!(
        async {
            let panicked = panicked.await;
            // The Run's end closes a browser the panic left open.
            run.cancellation().cancel();
            panicked
        },
        played
    );
    panicked
}

/// The engine's operations, the page crate's and the test's.
fn engine() -> Engine {
    let identity = Identity {
        product: Product::Collie,
        product_version: ProductVersion::parse("1.4.0").unwrap(),
        channel: Channel::CratesIo,
    };
    let composition = Composition::default()
        .with(goodfetch_engine::contribution())
        .with(crate::contribution(Vec::new()))
        .with(
            Contribution::new("test")
                .with_handler(Handler::new(&STOPPED_OPEN, stopped_open))
                .with_handler(Handler::new(&ENDED_PAST_CAP, ended_past_cap))
                .with_handler(Handler::new(&SESSION_ENDED, session_ended))
                .with_handler(Handler::new(&SESSION_PANICKED, session_panicked))
                .with_handler(Handler::new(&BROWSER_PANICKED, browser_panicked)),
        );
    Engine::new(identity, &composition).unwrap()
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

/// Runs the engine while a Client says `hello`, then calls `method` with
/// `params`, and answers the call's reply.
fn call(env: &TestEnv, method: &str, params: Value) -> Value {
    let engine = engine();
    goodfetch_test_support::runtime().block_on(async {
        let (network, dialer) = transport::in_memory();
        let client = async {
            let (reader, mut writer) = dialer.connect().await.unwrap().split();
            let mut lines = BufReader::new(reader).lines();
            let mut reply = async |id: i64, method: &str, params: Value| {
                let mut line = serde_json::to_vec(
                    &json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }),
                )
                .unwrap();
                line.push(b'\n');
                writer.write_all(&line).await.unwrap();
                loop {
                    let line = lines.next_line().await.unwrap().unwrap();
                    let message: Value = serde_json::from_str(&line).unwrap();
                    if message["id"] == json!(id) {
                        return message;
                    }
                }
            };
            let hello = reply(
                0,
                "hello",
                json!({
                    "protocol_version": "0.1",
                    "surface": "rust",
                    "needs": "collie",
                    "product_version": "1.4.0",
                }),
            )
            .await;
            assert_eq!(hello["result"]["product"], "collie");
            let answer = reply(1, method, params).await;
            drop(reply);
            writer.shutdown().await.unwrap();
            answer
        };
        let (served, answer) = tokio::join!(
            engine.run_attached(Network::InMemory(network), dependencies(env)),
            client
        );
        served.unwrap();
        answer
    })
}

/// A Run stopped while its default Session waits for its record in run
/// state leaves no Session of it in the table: the open, which took its
/// position before the Run stopped, finds the Run gone under the table's
/// lock and raises `run_not_found` instead of putting the Session in.
/// Covers: story:5-60
#[test]
fn a_default_session_whose_run_stops_while_it_opens_is_never_left_in_the_table() {
    let env = TestEnv::new();

    let reply = call(
        &env,
        "test.stopped_open",
        json!({ "run_id": "stopped-open" }),
    );

    assert_eq!(reply["result"], json!({ "opened": false, "left": 0 }));
}

/// A page the Resource governor ended past its memory cap, whose close is
/// confirmed, leaves the open pages straight for the ended ones its later
/// calls read, under one lock: a call on it raises `page_memory_exceeded`
/// again, never `closed`.
/// Covers: story:3-59
#[test]
fn a_page_ended_past_its_memory_cap_answers_page_memory_exceeded_once_its_close_is_confirmed() {
    let env = TestEnv::new();

    let reply = call(
        &env,
        "test.ended_past_cap",
        json!({ "run_id": "ended-past-cap" }),
    );

    assert_eq!(reply["result"], json!({ "raised": "page_memory_exceeded" }));
}

/// Interception alone holds a Session no egress relay is under, Borrowed
/// or launched, a launched one's unresolved names judged by whether a
/// proxy resolves them; a relayed Session gains no lookup of its own, its
/// relay checking each connection's address (#2435).
#[test]
fn interception_alone_holds_every_session_no_relay_is_under_and_never_a_relayed_one() {
    let cdp_url = CdpUrl {
        url: "ws://127.0.0.1:9222/devtools/browser/test".to_owned(),
        origin: CdpUrlOrigin {
            layer: SettingsLayer::Run,
            acting_for_model: false,
        },
    };
    let held = |borrowed, relayed, proxied| match held_browser(borrowed, relayed, proxied) {
        None => "relayed",
        Some(HeldBrowser::Borrowed(_)) => "borrowed",
        Some(HeldBrowser::Launched { proxied: false }) => "launched",
        Some(HeldBrowser::Launched { proxied: true }) => "launched behind a proxy",
    };

    let held_by = [
        held(None, true, false),
        held(None, true, true),
        held(None, false, false),
        held(None, false, true),
        held(Some(&cdp_url), false, false),
    ];

    assert_eq!(
        held_by,
        [
            "relayed",
            "relayed",
            "launched",
            "launched behind a proxy",
            "borrowed",
        ]
    );
}

/// A Session the Run's run file records, its default one, ends in a
/// commit of the run file, which records it closed; a Session of the
/// crawl's Session pool, which the run file never records, ends in none:
/// at `batch`, retiring one is no barrier that commits the open batch
/// (#2168).
#[rstest]
#[case::the_runs_default_session("default", 1)]
#[case::a_session_of_its_crawls_pool("pool", 0)]
fn only_a_session_the_run_file_records_commits_its_end(#[case] opener: &str, #[case] commits: u64) {
    let env = TestEnv::new();

    let reply = call(
        &env,
        "test.session_ended",
        json!({ "run_id": "session-ended", "opener": opener }),
    );

    assert_eq!(reply["result"], json!({ "commits": commits }), "{reply}");
}

/// A panic in a task of a Session's owner closes that Session alone: the
/// engine takes it out of the table, a later call on it raises
/// `engine_panicked` naming it, never `closed`, and its Run goes on
/// (ADR 0148, #3263).
#[test]
fn a_panic_in_a_sessions_task_closes_the_session_alone_and_its_calls_raise_engine_panicked() {
    let env = TestEnv::new();

    let reply = call(
        &env,
        "test.session_panicked",
        json!({ "run_id": "session-panicked" }),
    );

    let result = &reply["result"];
    assert_eq!(result["named"], json!(false), "{reply}");
    assert_eq!(result["raised"], json!("engine_panicked"), "{reply}");
    assert_eq!(result["details"]["ended"], json!("session"), "{reply}");
    assert_eq!(result["details"]["id"], result["session"], "{reply}");
    assert_eq!(result["run_live"], json!(true), "{reply}");
}

/// A panic in a task of a browser's owner is that browser's crash: the
/// browser's connection ends as a crash's, which its pages read as one and
/// raise `page_crashed` for, the Session seated in it takes the crash, the
/// Run trace records `engine_panicked` with `ended=browser`, and the Run
/// goes on (ADR 0148, #3265).
#[test]
fn a_panic_in_a_browsers_task_is_its_crash_and_its_run_goes_on() {
    let env = TestEnv::new();
    let (subscriber, captured) = capturing(&[(OUTCOME_TARGET, tracing::Level::TRACE)]);

    let reply = tracing::subscriber::with_default(subscriber, || {
        call(
            &env,
            "test.browser_panicked",
            json!({ "run_id": "browser-panicked" }),
        )
    });

    let result = &reply["result"];
    assert_eq!(result["ended"], json!("BrowserCrashed(None)"), "{reply}");
    assert_eq!(result["crash_taken"], json!(true), "{reply}");
    assert_eq!(result["run_live"], json!(true), "{reply}");
    let written = String::from_utf8(captured.written()).unwrap();
    let recorded: Vec<_> = written
        .lines()
        .filter(|line| line.contains("engine_panicked"))
        .collect();
    assert_eq!(recorded.len(), 1, "{written}");
    let ended = recorded[0].split("ended=").nth(1).unwrap_or_default();
    assert!(
        ended.trim_start_matches('"').starts_with("browser"),
        "{written}"
    );
}

/// A setting a program's own code sets at no scope is refused at a
/// Session's or a page's opening with the place that does set it, never
/// the Run, which refuses it too (ADR 0083): only configuration sets
/// `proxy.providers`, only an Operator policy `stealth.allowed_builds`,
/// only the project file `timeout.connect`; one some scope sets is left to that scope's own advice.
#[rstest]
#[case::configuration_alone(
    "proxy.providers",
    Some("set it in configuration or an Operator policy, which alone set it")
)]
#[case::an_operator_policy_alone("stealth.allowed_builds", Some("set it in an Operator policy"))]
#[case::the_project_file_alone(
    "timeout.connect",
    Some("set it in the project file, which `connect()` and the CLI read before they connect")
)]
#[case::one_a_session_sets("browser.access_mode", None)]
#[case::one_a_run_sets("timeout.navigation", None)]
fn a_setting_no_scope_sets_is_refused_with_where_it_is_set(
    #[case] key: &str,
    #[case] fix: Option<&str>,
) {
    assert_eq!(set_outside_code(key), fix);
}
