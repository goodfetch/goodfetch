//! The JSON-RPC protocol suite (#172, Seam 3) against targets that speak
//! Goodfetch's protocol, each a real process started from the executable
//! given by path: the Scripted engine playing the suite, which is the
//! conforming target, and a scripted variant that drops a capability mid-suite,
//! which is the broken one (ADR 0069); then either Product's executable as an
//! Attached engine and as a Daemon, the suite unchanged (ADR 0096).

use std::path::Path;

use goodfetch_conformance::{
    EngineMode, EngineTarget, ProtocolSuite, ProtocolSuiteError, ProvingGround, SuiteError,
    run_protocol_suite,
};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::journey::FeatureId;
use goodfetch_protocol::result_document::{
    CapabilityResult, CaseOutcome, ConformanceDocument, LoadedDocument, Outcome, ResultDocument,
    TargetIdentity, read,
};
use goodfetch_test_support::{
    ATTACHED_RECORD, ATTACHED_SCRIPT, ProvingGround as LocalCopy, TestEnv, Tool,
    browser_cache_in_home, tools,
};
use serde_json::{Value, json};

use crate::schema_violations;

/// The Scripted engine's scripts this suite's tests play.
const SCRIPTS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/it/fixtures/protocol");

/// Each capability's name and outcome.
fn outcomes(document: &ConformanceDocument) -> Vec<(&str, Outcome)> {
    document
        .capabilities()
        .iter()
        .map(|capability| (capability.name(), capability.outcome()))
        .collect()
}

/// The outcome of each case, in the suite's order.
fn case_outcomes(document: &ConformanceDocument) -> Vec<(&str, CaseOutcome)> {
    document
        .capabilities()
        .iter()
        .flat_map(CapabilityResult::cases)
        .map(|case| (case.facts().id.as_str(), case.outcome()))
        .collect()
}

/// The target `tool` names, started in `mode` with the test's environment.
fn target(env: &TestEnv, tool: Tool, mode: EngineMode) -> EngineTarget {
    EngineTarget {
        executable: tools().unwrap().path(tool).unwrap().to_path_buf(),
        mode,
        environment: env.env().clone(),
        folder: env.folder().to_path_buf(),
    }
}

/// The Scripted engine, which plays `script` from the test's Goodfetch home
/// in `mode`, as either Product's executable is started.
fn scripted(env: &TestEnv, script: &str, mode: EngineMode) -> EngineTarget {
    let installed = env.home().join(ATTACHED_SCRIPT);
    std::fs::create_dir_all(installed.parent().unwrap()).unwrap();
    std::fs::copy(Path::new(SCRIPTS).join(script), &installed).unwrap();
    target(env, Tool::ScriptedEngine, mode)
}

/// What the Scripted engine recorded the harness sending.
fn sent(env: &TestEnv) -> Vec<Value> {
    std::fs::read_to_string(env.home().join(ATTACHED_RECORD))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(|entry| entry["from"] == "client")
        .map(|entry| entry["message"].clone())
        .collect()
}

const PROVED: [(&str, Outcome); 4] = [
    ("connection.hello", Outcome::Proved),
    ("engine.info", Outcome::Proved),
    ("protocol.failures", Outcome::Proved),
    ("cancellation", Outcome::Proved),
];

/// Covers: story:2-50, story:2-53
#[test]
fn the_scripted_engine_playing_the_suite_is_proved_in_every_capability() {
    let env = TestEnv::new();
    let copy = LocalCopy::start(&env, env.seed()).unwrap();
    let proving_ground = ProvingGround::local(&copy.address().to_string());
    let target = scripted(&env, "conforming-attached.json", EngineMode::Attached);

    let document =
        run_protocol_suite(&ProtocolSuite::embedded(), &target, &proving_ground).unwrap();

    assert_eq!(outcomes(&document), PROVED);
}

/// Covers: story:2-50
#[test]
fn a_target_that_drops_cancellation_mid_suite_fails_it_and_never_proves_it() {
    let env = TestEnv::new();
    let copy = LocalCopy::start(&env, env.seed()).unwrap();
    let proving_ground = ProvingGround::local(&copy.address().to_string());
    let target = scripted(
        &env,
        "drops-cancellation-attached.json",
        EngineMode::Attached,
    );

    let document =
        run_protocol_suite(&ProtocolSuite::embedded(), &target, &proving_ground).unwrap();

    assert_eq!(
        (outcomes(&document), case_outcomes(&document)),
        (
            vec![
                ("connection.hello", Outcome::Proved),
                ("engine.info", Outcome::Proved),
                ("protocol.failures", Outcome::Proved),
                ("cancellation", Outcome::Failed),
            ],
            vec![
                (
                    "hello-is-answered-with-the-engines-product",
                    CaseOutcome::Passed
                ),
                (
                    "engine-info-names-the-engine-hello-named",
                    CaseOutcome::Passed
                ),
                ("an-unknown-method-is-method-not-found", CaseOutcome::Passed),
                (
                    "params-the-operation-does-not-take-are-invalid-params",
                    CaseOutcome::Passed
                ),
                (
                    "a-call-in-flight-cancelled-ends-with-call-cancelled",
                    CaseOutcome::Failed
                ),
                (
                    "cancelling-an-answered-call-leaves-the-connection-answering",
                    CaseOutcome::NotReached
                ),
            ]
        )
    );
}

/// Covers: story:2-50, story:2-53, story:2-54
#[test]
fn the_document_names_the_engine_hello_reported_and_carries_each_cases_evidence() {
    let env = TestEnv::new();
    let copy = LocalCopy::start(&env, env.seed()).unwrap();
    let proving_ground = ProvingGround::local(&copy.address().to_string());
    let target = scripted(
        &env,
        "drops-cancellation-attached.json",
        EngineMode::Attached,
    );

    let document =
        run_protocol_suite(&ProtocolSuite::embedded(), &target, &proving_ground).unwrap();

    let header = document.header();
    assert_eq!(
        (
            header.suite.name.as_str(),
            header.target.name.as_str(),
            &header.target.identity,
            header.target.product,
            header.proving_ground.seed,
            header.platform.os.as_str(),
        ),
        (
            "protocol",
            "goodfetch-scripted-engine",
            &TargetIdentity::Version("0.0.0".to_owned()),
            Some(Product::Collie),
            env.seed(),
            std::env::consts::OS,
        )
    );
    let cancelled = &document.capabilities()[3].cases()[0];
    let entry = &cancelled.facts().entries[0];
    assert_eq!(
        (
            entry.mode.as_deref(),
            cancelled
                .facts()
                .covers
                .iter()
                .map(FeatureId::as_str)
                .collect::<Vec<_>>(),
            entry
                .checks
                .last()
                .map(|check| (check.name.as_str(), check.passed)),
            entry.evidence.messages.clone(),
        ),
        (
            Some("attached"),
            vec!["story:2-50", "story:2-53", "error:call_cancelled"],
            Some((
                "the target answers the case's calls (the target closed the Connection)",
                false
            )),
            vec![
                json!({ "from": "client", "message": { "jsonrpc": "2.0", "id": 5, "method": "events.subscribe", "params": {} } }),
                json!({ "from": "client", "message": { "jsonrpc": "2.0", "method": "$/cancel", "params": { "id": 5 } } }),
            ],
        )
    );
    let written = ResultDocument::Conformance(document.clone());
    assert_eq!(schema_violations(&written), Vec::<String>::new());
    assert_eq!(
        read(&serde_json::to_vec(&written).unwrap()).unwrap(),
        LoadedDocument::Conformance(document)
    );
}

/// Covers: story:2-50
#[test]
fn the_harness_announces_the_suites_product_in_hello() {
    let env = TestEnv::new();
    let copy = LocalCopy::start(&env, env.seed()).unwrap();
    let proving_ground = ProvingGround::local(&copy.address().to_string());
    let target = scripted(&env, "conforming-attached.json", EngineMode::Attached);

    run_protocol_suite(&ProtocolSuite::embedded(), &target, &proving_ground).unwrap();

    assert_eq!(
        sent(&env)[0]["params"],
        json!({ "protocol_version": "0.1", "surface": "rust", "needs": "collie", "product_version": "0.0.0" })
    );
}

/// Covers: story:2-50, story:2-53
#[test]
fn the_scripted_engine_as_a_daemon_playing_the_suite_is_proved_in_every_capability() {
    let env = TestEnv::new();
    let copy = LocalCopy::start(&env, env.seed()).unwrap();
    let proving_ground = ProvingGround::local(&copy.address().to_string());
    let target = scripted(&env, "conforming-daemon.json", EngineMode::Daemon);

    let document =
        run_protocol_suite(&ProtocolSuite::embedded(), &target, &proving_ground).unwrap();

    assert_eq!(outcomes(&document), PROVED);
}

/// The Daemon's token, which the harness sends in `hello`, never reaches the
/// document: a Release cell uploads it with its JUnit files, and its secret
/// scan refused it there (ADR 0067). The evidence keeps the message, the
/// token's value redacted.
///
/// Covers: story:2-50
#[test]
fn a_daemons_token_reaches_no_document_and_its_hello_keeps_the_rest() {
    let env = TestEnv::new();
    let copy = LocalCopy::start(&env, env.seed()).unwrap();
    let proving_ground = ProvingGround::local(&copy.address().to_string());
    let target = scripted(&env, "conforming-daemon.json", EngineMode::Daemon);

    let document =
        run_protocol_suite(&ProtocolSuite::embedded(), &target, &proving_ground).unwrap();

    let written = serde_json::to_string(&ResultDocument::Conformance(document.clone())).unwrap();
    let sent_tokens: Vec<Value> = sent(&env)
        .into_iter()
        .filter(|message| message["method"] == "hello")
        .map(|hello| hello["params"]["token"].clone())
        .collect();
    let kept_tokens: Vec<Value> = document
        .capabilities()
        .iter()
        .flat_map(CapabilityResult::cases)
        .flat_map(|case| case.facts().entries.iter())
        .flat_map(|entry| entry.evidence.messages.iter())
        .filter(|kept| kept["from"] == "client" && kept["message"]["method"] == "hello")
        .map(|kept| kept["message"]["params"]["token"].clone())
        .collect();
    assert_eq!(
        (
            written.contains("scripted-daemon-token"),
            sent_tokens.first(),
            kept_tokens.first(),
        ),
        (
            false,
            Some(&json!("scripted-daemon-token")),
            Some(&json!("[redacted]")),
        )
    );
}

/// The suite unchanged against Collie's executable, given by path, as an
/// Attached engine and as a Daemon.
///
/// Covers: story:2-50, story:2-53, story:2-54
#[rstest::rstest]
fn the_suite_runs_unchanged_against_collies_attached_engine_and_daemon(
    #[values(EngineMode::Attached, EngineMode::Daemon)] mode: EngineMode,
) {
    let env = TestEnv::new();
    let copy = LocalCopy::start(&env, env.seed()).unwrap();
    let proving_ground = ProvingGround::local(&copy.address().to_string());
    let target = target(&env, Tool::Collie, mode);

    let document =
        run_protocol_suite(&ProtocolSuite::embedded(), &target, &proving_ground).unwrap();
    crate::keep(
        &document,
        &format!("protocol-collie-{mode:?}").to_lowercase(),
    );

    assert_eq!(
        (
            outcomes(&document),
            document.header().target.name.as_str(),
            document.header().target.product,
        ),
        (PROVED.to_vec(), "goodfetch-collie", Some(Product::Collie))
    );
}

/// The fetch suite against Foxhound's Attached engine: a crawl request
/// under `fetch.tier: http` is fetched over HTTP, decoded, and handed to
/// its handler as a page `page.formats` reads, the site seeing one request;
/// a cut body fails its attempt and never reaches the handler, a body its
/// connection's close delimits does, and a request the site never
/// processed is sent once more, counted nowhere; its redirect chains
/// and refresh stubs are followed as the browser follows them; and the
/// requests a site's `Retry-After` past the bound held, ended `failed`, are
/// re-admitted by `run.retry_failed` once the site is back.
///
/// Covers: story:5-124, story:5-125, story:5-126, story:5-127, story:5-128, story:6-307
#[test]
fn the_fetch_suite_is_proved_against_foxhounds_attached_engine() {
    let env = TestEnv::new();
    let copy = LocalCopy::start(&env, env.seed()).unwrap();
    let proving_ground = ProvingGround::local(&copy.address().to_string());
    let target = target(&env, Tool::Foxhound, EngineMode::Attached);

    let document = run_protocol_suite(&ProtocolSuite::fetch(), &target, &proving_ground).unwrap();
    crate::keep(&document, "fetch-foxhound-attached");

    assert_eq!(
        (outcomes(&document), case_outcomes(&document)),
        (
            vec![
                ("fetch.tier.http", Outcome::Proved),
                ("fetch.edge.http", Outcome::Proved)
            ],
            vec![
                (
                    "a-request-fetched-over-http-hands-its-handler-the-document-on-a-page",
                    CaseOutcome::Passed
                ),
                (
                    "a-cut-body-fails-its-attempt-with-response-truncated-and-never-reaches-the-handler",
                    CaseOutcome::Passed
                ),
                (
                    "a-body-its-connections-close-delimits-reaches-the-handler-whole",
                    CaseOutcome::Passed
                ),
                (
                    "a-request-the-site-never-processed-is-sent-once-more-and-counted-nowhere",
                    CaseOutcome::Passed
                ),
                (
                    "a-redirect-loop-fails-with-too-many-redirects-at-a-cost-of-fetch-redirect-max-and-one-requests",
                    CaseOutcome::Passed
                ),
                (
                    "each-redirect-status-sends-its-next-hop-with-the-method-and-body-fetch-specifies",
                    CaseOutcome::Passed
                ),
                (
                    "a-3xx-fetch-does-not-follow-fails-with-response-not-ok-unless-crawl-handle-statuses-lists-it",
                    CaseOutcome::Passed
                ),
                (
                    "each-redirect-hop-goes-through-the-navigation-gate",
                    CaseOutcome::Passed
                ),
                (
                    "a-redirect-to-an-admitted-key-or-out-of-scope-ends-skipped",
                    CaseOutcome::Passed
                ),
                (
                    "a-redirects-retry-after-delays-its-next-hop-or-past-the-bound-fails-with-response-unavailable",
                    CaseOutcome::Passed
                ),
                (
                    "requests-held-past-politeness-retry-after-max-end-failed-and-retry-failed-re-admits-them",
                    CaseOutcome::Passed
                ),
                (
                    "a-refresh-stub-within-fetch-refresh-max-delay-hands-the-handler-its-target",
                    CaseOutcome::Passed
                ),
                (
                    "a-redirect-the-scope-the-domain-policy-or-the-address-check-refuses-ends-skipped-live",
                    CaseOutcome::Passed
                ),
            ]
        ),
        "{}",
        serde_json::to_string_pretty(&document).unwrap()
    );
}

/// The delivery suite against Foxhound's executable, given by path, as an
/// Attached engine and as a Daemon: a lease that ends unanswered is
/// cancelled and its work item delivered again, an answer in time commits
/// once, a late answer changes nothing, and a fabricated `hello` is refused
/// on a Connection of its own.
///
/// Covers: story:3-3, story:3-40, story:3-41, story:3-132
#[rstest::rstest]
fn the_delivery_suite_is_proved_against_foxhounds_attached_engine_and_daemon(
    #[values(EngineMode::Attached, EngineMode::Daemon)] mode: EngineMode,
) {
    let env = TestEnv::new();
    let copy = LocalCopy::start(&env, env.seed()).unwrap();
    let proving_ground = ProvingGround::local(&copy.address().to_string());
    let target = target(&env, Tool::Foxhound, mode);

    let document =
        run_protocol_suite(&ProtocolSuite::delivery(), &target, &proving_ground).unwrap();
    crate::keep(
        &document,
        &format!("delivery-foxhound-{mode:?}").to_lowercase(),
    );

    assert_eq!(
        (outcomes(&document), case_outcomes(&document)),
        (
            vec![
                ("connection.hello", Outcome::Proved),
                ("delivery.at_least_once", Outcome::Proved),
                ("delivery.exactly_once", Outcome::Proved),
            ],
            vec![
                (
                    "a-fabricated-hello-is-refused-before-any-other-message-and-the-connection-closes",
                    CaseOutcome::Passed
                ),
                (
                    "a-work-item-whose-lease-ends-unanswered-is-delivered-again",
                    CaseOutcome::Passed
                ),
                (
                    "a-work-item-answered-in-time-commits-once",
                    CaseOutcome::Passed
                ),
                (
                    "an-answer-after-the-lease-ended-is-a-late-commit-and-refused",
                    CaseOutcome::Passed
                ),
            ]
        ),
        "{}",
        serde_json::to_string_pretty(&document).unwrap()
    );
}

/// The layout of run state a reset suite's target keeps its Runs in.
#[derive(Clone, Copy, Debug)]
enum Layout {
    PerRun,
    PerInstall,
}

/// ADR 0001's reset gate against Collie's executable, given by path, in
/// either layout of run state (ADR 0002): as an Attached engine and a
/// Daemon in the per-run layout, and as a Daemon in the per-install one. A
/// discarded Run's planted Secret canaries, a token, a password and the
/// Session cookie a login set, are found nowhere in the Goodfetch home, and
/// a later Run under its Run ID finds none of its state and is sent none of
/// its cookies. The cookie case's browser runs from the pinned build laid
/// out in the home's own Browser cache, which the search takes in as a
/// person's home holds it; `egress.offline` refuses a download of it, so a
/// missing build fails at once rather than fetching some 150 MB within the
/// harness's wait for `run.new_page` (fix-queue 124).
///
/// Covers: story:3-39, operation:run.discard
#[rstest::rstest]
#[case::attached_per_run(EngineMode::Attached, Layout::PerRun)]
#[case::daemon_per_run(EngineMode::Daemon, Layout::PerRun)]
#[case::daemon_per_install(EngineMode::Daemon, Layout::PerInstall)]
fn the_reset_gate_is_proved_against_collie_in_either_layout(
    #[case] mode: EngineMode,
    #[case] layout: Layout,
) {
    let env = TestEnv::new();
    browser_cache_in_home(&env).unwrap();
    let mut settings = String::from("egress.offline = true\n");
    if let Layout::PerInstall = layout {
        settings.push_str("run_state.layout = \"per_install\"\n");
    }
    env.user_configuration(&settings);
    let copy = LocalCopy::start(&env, env.seed()).unwrap();
    let proving_ground = ProvingGround::local(&copy.address().to_string());
    let target = target(&env, Tool::Collie, mode);

    let document = run_protocol_suite(&ProtocolSuite::reset(), &target, &proving_ground).unwrap();
    crate::keep(
        &document,
        &format!("reset-collie-{mode:?}-{layout:?}").to_lowercase(),
    );

    assert_eq!(
        (outcomes(&document), case_outcomes(&document)),
        (
            vec![("run.discard", Outcome::Proved)],
            vec![
                (
                    "a-discarded-run-leaves-none-of-its-secret-values-in-the-home",
                    CaseOutcome::Passed
                ),
                (
                    "a-discarded-runs-session-cookie-reaches-no-later-run",
                    CaseOutcome::Passed
                ),
            ]
        ),
        "{}",
        serde_json::to_string_pretty(&document).unwrap()
    );
}

/// The start table's suite proves `run.start` on Collie's Attached engine:
/// a start under a new Run ID begins it, one under a stopped Run's ID
/// resumes it, one under a finished Run's ID answers it as it ended, one
/// under a live Run's ID is `run_in_use`, and one under a listed Run's ID
/// whose run file is gone is `run_state_unavailable`.
///
/// Covers: story:3-166, story:3-167
#[test]
fn the_start_table_is_proved_against_collie() {
    let env = TestEnv::new();
    let copy = LocalCopy::start(&env, env.seed()).unwrap();
    let proving_ground = ProvingGround::local(&copy.address().to_string());
    let target = target(&env, Tool::Collie, EngineMode::Attached);

    let document =
        run_protocol_suite(&ProtocolSuite::run_start(), &target, &proving_ground).unwrap();
    crate::keep(&document, "run-start-collie");

    assert_eq!(
        (outcomes(&document), case_outcomes(&document)),
        (
            vec![("run.start", Outcome::Proved)],
            vec![
                ("a-start-under-a-new-run-id-begins-it", CaseOutcome::Passed),
                (
                    "a-start-under-a-stopped-runs-id-resumes-it",
                    CaseOutcome::Passed
                ),
                (
                    "a-start-under-the-id-of-a-run-whose-engine-was-killed-resumes-it",
                    CaseOutcome::Passed
                ),
                (
                    "a-start-under-a-finished-runs-id-answers-it-as-it-ended",
                    CaseOutcome::Passed
                ),
                (
                    "a-start-under-a-live-runs-id-is-run-in-use",
                    CaseOutcome::Passed
                ),
                (
                    "a-start-under-a-listed-runs-id-whose-state-is-gone-is-run-state-unavailable",
                    CaseOutcome::Passed
                ),
            ]
        ),
        "{}",
        serde_json::to_string_pretty(&document).unwrap()
    );
}

/// ADR 0102's start table rows a crawl reaches pass against Foxhound's
/// executable as an Attached engine: a start under a finished crawl's ID
/// answers it calling no handler, a start that differs from the crawl
/// it names fails with `run_conflict` listing what differs, and a start
/// under a crawl its Limit stopped fails with `limit_reached`.
///
/// Covers: story:3-166, story:3-167
#[test]
fn the_start_table_of_a_crawl_is_proved_against_foxhound() {
    let env = TestEnv::new();
    let copy = LocalCopy::start(&env, env.seed()).unwrap();
    let proving_ground = ProvingGround::local(&copy.address().to_string());
    let target = target(&env, Tool::Foxhound, EngineMode::Attached);

    let document =
        run_protocol_suite(&ProtocolSuite::run_start_crawl(), &target, &proving_ground).unwrap();
    crate::keep(&document, "run-start-crawl-foxhound");

    assert_eq!(
        (outcomes(&document), case_outcomes(&document)),
        (
            vec![("crawl.start", Outcome::Proved)],
            vec![
                (
                    "a-crawl-start-under-a-finished-crawls-id-answers-it-calling-no-handler",
                    CaseOutcome::Passed
                ),
                (
                    "a-crawl-start-under-a-crawls-id-with-other-seeds-is-run-conflict-naming-seeds",
                    CaseOutcome::Passed
                ),
                (
                    "a-collie-start-under-a-crawls-id-is-run-conflict-naming-product-kind-and-seeds",
                    CaseOutcome::Passed
                ),
                (
                    "a-start-under-the-id-of-a-crawl-its-limit-stopped-is-limit-reached",
                    CaseOutcome::Passed
                ),
            ]
        ),
        "{}",
        serde_json::to_string_pretty(&document).unwrap()
    );
}

/// ADR 0003's fencing passes against Foxhound's executable as an Attached
/// engine: once a second copy resumed the Run's Checkpoint at the next
/// run epoch, a destination that compares and sets refuses the first
/// copy's Checkpoints and Exports.
///
/// Covers: story:3-43
#[test]
fn the_fenced_copy_has_its_writes_refused_against_foxhound() {
    let env = TestEnv::new();
    let copy = LocalCopy::start(&env, env.seed()).unwrap();
    let proving_ground = ProvingGround::local(&copy.address().to_string());
    let target = target(&env, Tool::Foxhound, EngineMode::Attached);

    let document =
        run_protocol_suite(&ProtocolSuite::two_copies(), &target, &proving_ground).unwrap();
    crate::keep(&document, "two-copies-foxhound");

    assert_eq!(
        (outcomes(&document), case_outcomes(&document)),
        (
            vec![("run.resume", Outcome::Proved)],
            vec![(
                "the-fenced-copy-has-its-checkpoints-and-exports-refused-where-the-destination-compares-and-sets",
                CaseOutcome::Passed
            )]
        ),
        "{}",
        serde_json::to_string_pretty(&document).unwrap()
    );
}

/// The protocol suite's `engine.info` case with Foxhound as its Product, as
/// a suite whose contract needs Foxhound declares it.
fn foxhound_suite() -> ProtocolSuite {
    ProtocolSuite::read(
        r#"
        product = "foxhound"

        [suite]
        name = "protocol-on-foxhound"
        version = "1"

        [[capability]]
        name = "engine.info"

        [[capability.case]]
        id = "engine-info-names-the-engine-hello-named"
        covers = []
        "#,
    )
    .unwrap()
}

/// Covers: story:2-50
#[rstest::rstest]
fn a_suite_that_needs_foxhound_runs_on_foxhounds_executable(
    #[values(EngineMode::Attached, EngineMode::Daemon)] mode: EngineMode,
) {
    let env = TestEnv::new();
    let copy = LocalCopy::start(&env, env.seed()).unwrap();
    let proving_ground = ProvingGround::local(&copy.address().to_string());
    let target = target(&env, Tool::Foxhound, mode);

    let document = run_protocol_suite(&foxhound_suite(), &target, &proving_ground).unwrap();

    assert_eq!(
        (outcomes(&document), document.header().target.product),
        (
            vec![("engine.info", Outcome::Proved)],
            Some(Product::Foxhound)
        )
    );
}

/// Covers: story:2-50
#[test]
fn collies_executable_refuses_a_suite_that_needs_foxhound_with_not_in_product() {
    let env = TestEnv::new();
    let copy = LocalCopy::start(&env, env.seed()).unwrap();
    let proving_ground = ProvingGround::local(&copy.address().to_string());
    let target = target(&env, Tool::Collie, EngineMode::Attached);

    let refused = run_protocol_suite(&foxhound_suite(), &target, &proving_ground).unwrap_err();

    let ProtocolSuiteError::HelloRefused { error } = refused else {
        panic!("not a refused hello: {refused}");
    };
    assert_eq!(error["data"]["code"], json!("not_in_product"));
}

/// Covers: story:2-50
#[rstest::rstest]
fn a_suite_whose_contract_collie_carries_is_not_run_on_foxhounds_executable(
    #[values(EngineMode::Attached, EngineMode::Daemon)] mode: EngineMode,
) {
    let env = TestEnv::new();
    let copy = LocalCopy::start(&env, env.seed()).unwrap();
    let proving_ground = ProvingGround::local(&copy.address().to_string());
    let target = target(&env, Tool::Foxhound, mode);

    let refused =
        run_protocol_suite(&ProtocolSuite::embedded(), &target, &proving_ground).unwrap_err();

    assert!(
        matches!(
            refused,
            ProtocolSuiteError::OtherProduct {
                suite: Product::Collie,
                engine: Product::Foxhound
            }
        ),
        "{refused}"
    );
}

#[test]
fn a_case_id_the_harness_does_not_run_is_refused() {
    let refused = ProtocolSuite::read(
        r#"
        product = "collie"

        [suite]
        name = "protocol"
        version = "1"

        [[capability]]
        name = "engine.info"

        [[capability.case]]
        id = "engine-info-is-fast"
        covers = []
        "#,
    )
    .unwrap_err();

    assert!(
        matches!(
            &refused,
            SuiteError::UnknownCase { case } if case == "engine-info-is-fast"
        ),
        "{refused}"
    );
}

#[test]
fn a_suite_narrowed_to_some_capabilities_keeps_them_in_its_own_order() {
    let narrowed = ProtocolSuite::embedded()
        .with_capabilities(&["cancellation", "engine.info"])
        .unwrap();

    assert_eq!(
        narrowed
            .case_list()
            .capabilities
            .iter()
            .map(|capability| capability.name.as_str())
            .collect::<Vec<_>>(),
        ["engine.info", "cancellation"]
    );
}

#[test]
fn a_capability_the_suite_does_not_declare_is_refused() {
    let refused = ProtocolSuite::embedded()
        .with_capabilities(&["engine.info", "engine.speed"])
        .unwrap_err();

    assert!(
        matches!(
            &refused,
            SuiteError::UnknownCapability { suite, capability }
                if suite == "protocol" && capability == "engine.speed"
        ),
        "{refused:?}"
    );
}
