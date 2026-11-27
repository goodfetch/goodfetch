//! The Fingerprint method driver: which Sessions get a plan, where they
//! are placed, which primitive each method fills with the Session's
//! Identity profile, the warning each one leaves, what its record says,
//! the script `js_injection` writes, and what a page's out-of-process
//! frames and dedicated workers take before they run.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_identity_profile::{
    Constraints, EngineFamily, ExitFacts, IdentityProfile, IdentitySeed, generate,
};
use goodfetch_protocol::page::SessionId;
use goodfetch_protocol::run::warning::{
    FINGERPRINT_BASIC, FINGERPRINT_BASIC_FALLBACK, FINGERPRINT_JS_INJECTION, FINGERPRINT_NONE,
    STEALTH_RUNTIME_ALLOWED,
};
use goodfetch_protocol::settings::SettingsLayer;
use goodfetch_test_support::runtime;
use rstest::rstest;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use super::{FingerprintPlan, Method, WPT_IDENTITY_SEED, children, init_script, wpt_launch_args};
use crate::access_mode::{
    AccessMode, ChromiumBuild, FingerprintMethod, MethodSource, RuntimeDomain, SessionIdentityPlan,
    StealthProfile,
};
use crate::cdp::launch::Window;
use crate::cdp::{CdpPage, Emulated, Geolocation, ScreenSize, TargetSession, UserAgentOverride};
use crate::live_run::played_page;
use crate::page_children::{Child, ChildKind};
use crate::pool::Placement;

const SET: MethodSource = MethodSource::Set(SettingsLayer::Run);
const FELL_BACK: MethodSource = MethodSource::FellBack;

/// What a Chromium browser reports as its product.
const PRODUCT: &str = "HeadlessChrome/131.0.6778.85";

/// A plan chosen in the Run's settings, whose method is `method`, from
/// `source`.
fn identity(
    mode: AccessMode,
    method: FingerprintMethod,
    source: MethodSource,
) -> SessionIdentityPlan {
    SessionIdentityPlan {
        mode,
        layer: SettingsLayer::Run,
        profile: (mode == AccessMode::Stealth).then(|| StealthProfile {
            fingerprint_method: method,
            method_source: source,
            family: EngineFamily::Chromium,
            generated_identity: true,
            os: Vec::new(),
            device: Vec::new(),
            brand: Vec::new(),
            human_input: false,
            chromium_build: ChromiumBuild::Goodfetch,
            headless: true,
            runtime: RuntimeDomain::Refused,
        }),
        signing: None,
    }
}

/// A plan whose `stealth` the Run's settings chose, which keeps its pages
/// off the browser's `Runtime` domain, showing `identity`.
fn plan(
    method: Method,
    source: MethodSource,
    identity: Option<IdentityProfile>,
) -> FingerprintPlan {
    FingerprintPlan {
        method,
        source,
        stealth: SettingsLayer::Run,
        identity: identity.map(Box::new),
        runtime: RuntimeDomain::Refused,
        window: Window::Headless,
    }
}

/// A Windows desktop Chrome drawn with `exit`'s facts.
fn drawn(exit: ExitFacts) -> IdentityProfile {
    let constraints = Constraints::narrowed(
        EngineFamily::Chromium,
        &["windows".to_owned()],
        &["desktop".to_owned()],
        &["chrome".to_owned()],
    )
    .expect("the population holds a Windows desktop Chrome");
    generate(IdentitySeed(7), &constraints, exit)
}

/// One drawn before its exit is known: no time zone nor geolocation.
fn identity_profile() -> IdentityProfile {
    drawn(ExitFacts::default())
}

/// One drawn behind an exit in Paris.
fn at_the_exit() -> IdentityProfile {
    drawn(ExitFacts {
        language: Some("fr-FR".to_owned()),
        time_zone: Some("Europe/Paris".to_owned()),
        geolocation: Some(goodfetch_identity_profile::Geolocation {
            latitude: 48.85,
            longitude: 2.35,
            accuracy: 10.0,
        }),
        ..ExitFacts::default()
    })
}

/// Only a `stealth` Session with a built method gets a plan, which keeps
/// where its method came from and the Identity profile it shows.
#[rstest]
#[case::standard(identity(AccessMode::Standard, FingerprintMethod::Basic, SET), None)]
#[case::the_fallback(
    identity(AccessMode::Stealth, FingerprintMethod::Basic, FELL_BACK),
    Some(plan(Method::Basic, FELL_BACK, Some(identity_profile())))
)]
#[case::js_injection(
    identity(AccessMode::Stealth, FingerprintMethod::JsInjection, SET),
    Some(plan(Method::JsInjection, SET, Some(identity_profile())))
)]
#[case::none(
    identity(AccessMode::Stealth, FingerprintMethod::None, SET),
    Some(plan(Method::None, SET, Some(identity_profile())))
)]
#[case::engine(identity(AccessMode::Stealth, FingerprintMethod::Engine, SET), None)]
fn a_stealth_session_with_a_built_method_has_a_plan(
    #[case] identity: SessionIdentityPlan,
    #[case] expected: Option<FingerprintPlan>,
) {
    assert_eq!(
        FingerprintPlan::of(&identity, Some(identity_profile())),
        expected
    );
}

/// Every built method shares a browser process as contexts.
#[rstest]
fn every_built_method_shares_a_browser_process(
    #[values(Method::None, Method::JsInjection, Method::Basic)] method: Method,
) {
    assert_eq!(
        plan(method, SET, Some(identity_profile())).placement(),
        Placement::Shared
    );
}

/// `basic` shows its Identity profile through the browser's overrides and
/// `js_injection` through its script; `none` shows the machine's own, and
/// no method writes anything without an Identity profile,
/// `stealth.generated_identity` off.
#[rstest]
#[case::basic(Method::Basic, Some(identity_profile()), (true, false))]
#[case::js_injection(Method::JsInjection, Some(identity_profile()), (false, true))]
#[case::none(Method::None, Some(identity_profile()), (false, false))]
#[case::basic_without_an_identity_profile(Method::Basic, None, (false, false))]
#[case::js_injection_without_an_identity_profile(Method::JsInjection, None, (false, false))]
fn each_method_fills_its_own_primitive(
    #[case] method: Method,
    #[case] identity: Option<IdentityProfile>,
    #[case] (overrides, script): (bool, bool),
) {
    let (emulated, init) = plan(method, SET, identity).primitives(Some(PRODUCT));

    assert_eq!((emulated.is_some(), init.is_some()), (overrides, script));
}

/// `basic` shows the profile's user agent and client hints at the
/// browser's own version, which no built method changes, and its locale,
/// core count, screen, and the exit's time zone and geolocation.
/// Covers: story:8-13
#[test]
fn basic_shows_the_profile_at_the_browsers_version() {
    let identity = at_the_exit();

    let (emulated, _) = plan(Method::Basic, SET, Some(identity.clone())).primitives(Some(PRODUCT));

    let emulated = emulated.expect("values to show");
    let agent = emulated.user_agent.expect("a user agent");
    let hints = agent.client_hints.expect("client hints");
    assert!(
        agent.user_agent.contains("Chrome/131.0.0.0"),
        "{}",
        agent.user_agent
    );
    assert_eq!(
        (
            hints.ua_full_version.as_str(),
            hints.platform.as_str(),
            agent.platform.as_str(),
            agent.accept_language.as_deref(),
        ),
        ("131.0.6778.85", "Windows", "Win32", Some("fr-FR,fr;q=0.9"))
    );
    assert_eq!(
        (
            emulated.locale.as_deref(),
            emulated.languages.as_slice(),
            emulated.time_zone.as_deref(),
            emulated.geolocation,
            emulated.core_count,
            emulated.screen,
        ),
        (
            Some("fr-FR"),
            &["fr-FR".to_owned(), "fr".to_owned()][..],
            Some("Europe/Paris"),
            Some(Geolocation {
                latitude: 48.85,
                longitude: 2.35,
                accuracy: 10.0,
            }),
            Some(identity.navigator.hardware_concurrency),
            Some(ScreenSize {
                width: identity.screen.width,
                height: identity.screen.height,
                avail_width: identity.screen.avail_width,
                avail_height: identity.screen.avail_height,
            }),
        )
    );
    let injected = plan(
        Method::JsInjection,
        MethodSource::Set(SettingsLayer::UserConfiguration),
        Some(identity),
    );
    let script = injected
        .primitives(Some("HeadlessChrome/131.0.6778.85"))
        .1
        .expect("js_injection script");
    assert!(
        script.contains(r#"Object.freeze(["fr-FR","fr"])"#),
        "expected secondary languages in js_injection script: {script}"
    );
}

/// A browser whose product names no version it can read keeps the
/// profile's drawn one.
#[rstest]
#[case::unknown(None)]
#[case::unreadable(Some("Chrome"))]
fn an_unread_version_keeps_the_drawn_one(#[case] product: Option<&str>) {
    let identity = identity_profile();

    let (emulated, _) = plan(Method::Basic, SET, Some(identity.clone())).primitives(product);

    assert_eq!(
        emulated.and_then(|values| values.user_agent.map(|agent| agent.user_agent)),
        Some(identity.user_agent)
    );
}

/// Each method leaves its own warning on the Session, `basic`'s fallback
/// one apart from a chosen `basic`; `none`'s stands once for every
/// Session of the Run. A masking method without a profile says its pages
/// show the browser's own values, naming `stealth.generated_identity`, and
/// one whose profile has no exit's time zone says its time zone stays the
/// machine's.
/// Covers: story:8-12
/// Covers: story:8-14
/// Covers: story:8-107
#[rstest]
#[case::none(Method::None, SET, Some(at_the_exit()), FINGERPRINT_NONE, true, &[], (false, false))]
#[case::js_injection(Method::JsInjection, SET, Some(at_the_exit()), FINGERPRINT_JS_INJECTION, false, &["session-1"], (false, false))]
#[case::basic(Method::Basic, SET, Some(at_the_exit()), FINGERPRINT_BASIC, false, &["session-1"], (false, false))]
#[case::the_fallback(Method::Basic, FELL_BACK, Some(at_the_exit()), FINGERPRINT_BASIC_FALLBACK, false, &["session-1"], (false, false))]
#[case::basic_without_a_profile(Method::Basic, SET, None, FINGERPRINT_BASIC, false, &["session-1"], (true, false))]
#[case::basic_before_its_exit(Method::Basic, SET, Some(identity_profile()), FINGERPRINT_BASIC, false, &["session-1"], (false, true))]
#[case::none_without_a_profile(Method::None, SET, None, FINGERPRINT_NONE, true, &[], (false, false))]
fn each_method_leaves_its_warning(
    #[case] method: Method,
    #[case] source: MethodSource,
    #[case] identity: Option<IdentityProfile>,
    #[case] code: &str,
    #[case] once: bool,
    #[case] sessions: &[&str],
    #[case] (browser_values, machine_time_zone): (bool, bool),
) {
    let session = SessionId("session-1".to_owned());

    let (warning, for_the_run) = plan(method, source, identity).warning(&session);

    assert_eq!(
        (
            warning.code.as_str(),
            for_the_run,
            warning.sessions,
            warning.message.contains("`stealth.generated_identity`"),
            warning.message.contains("time zone"),
        ),
        (
            code,
            once,
            sessions
                .iter()
                .map(|&session| session.to_owned())
                .collect::<Vec<_>>(),
            browser_values,
            machine_time_zone,
        )
    );
}

/// The record of a Session's choice names its method, whether it fell
/// back, and the layer that chose the method: the one that set it, or the
/// one that chose `stealth` for the fallback.
/// Covers: story:8-94
#[rstest]
#[case::set_in_the_user_configuration(
    Method::JsInjection,
    MethodSource::Set(SettingsLayer::UserConfiguration),
    json!({ "session_id": "session-1", "method": "js_injection", "fell_back": false, "runtime_allowed": false }),
    SettingsLayer::UserConfiguration
)]
#[case::the_fallback(
    Method::Basic,
    FELL_BACK,
    json!({ "session_id": "session-1", "method": "basic", "fell_back": true, "runtime_allowed": false }),
    SettingsLayer::Run
)]
fn the_record_names_the_layer_that_chose_the_method(
    #[case] method: Method,
    #[case] source: MethodSource,
    #[case] details: Value,
    #[case] layer: SettingsLayer,
) {
    let plan = plan(method, source, None);

    assert_eq!(
        (
            plan.details(&SessionId("session-1".to_owned())),
            plan.layer()
        ),
        (details, layer)
    );
}

/// A `stealth` Session's pages refuse what needs the browser's `Runtime`
/// domain, naming `stealth.allow_runtime`; once it allows the domain, they
/// refuse nothing, the Session's Run report carries its warning and its
/// record says so (#599's Q2).
/// Covers: setting:stealth.allow_runtime
#[rstest]
#[case::kept_off(
    RuntimeDomain::Refused,
    Some("`stealth.allow_runtime: true`"),
    None,
    false
)]
#[case::allowed(RuntimeDomain::Allowed, None, Some(STEALTH_RUNTIME_ALLOWED), true)]
fn the_runtime_domain_opens_only_where_the_setting_allows_it(
    #[case] runtime: RuntimeDomain,
    #[case] fix: Option<&str>,
    #[case] warned: Option<&str>,
    #[case] recorded: bool,
) {
    let session = SessionId("session-1".to_owned());
    let plan = FingerprintPlan {
        runtime,
        ..plan(Method::Basic, SET, Some(identity_profile()))
    };

    let refused = plan
        .runtime_refusal()
        .map(|error| error.details().map(|details| details["fix"].clone()));
    let warning = plan.runtime_warning(&session);

    assert_eq!(
        (
            refused,
            warning
                .as_ref()
                .map(|warning| (warning.code.as_str(), warning.sessions.clone())),
            plan.details(&session)["runtime_allowed"].clone(),
        ),
        (
            fix.map(|fix| Some(json!(fix))),
            warned.map(|code| (code, vec!["session-1".to_owned()])),
            json!(recorded),
        )
    );
}

/// The methods a page's child of kind `kind` takes on its own session,
/// before it runs, from the Fingerprint method `plan`, the browser
/// answering each, or refusing the first when `refused`; and
/// whether the child may run.
fn taken_by_child(
    plan: Option<&FingerprintPlan>,
    kind: ChildKind,
    refused: bool,
) -> (Vec<String>, bool) {
    runtime().block_on(async {
        let (driver, mut played, mut connection) = played_page();
        let on_child = children(plan, &CdpPage::of(&driver));
        let child = Child {
            kind,
            parent: TargetSession("page".to_owned()),
            session: TargetSession("child".to_owned()),
            target: "child-target".to_owned(),
            url: "https://example.test/".to_owned(),
            gone: CancellationToken::new(),
        };
        let held = on_child(&child);
        tokio::pin!(held);
        let mut methods = Vec::new();
        let runs = loop {
            tokio::select! {
                biased;
                runs = &mut held => break runs,
                command = played.command() => {
                    assert_eq!(command["sessionId"], json!("child"), "{command}");
                    let method = command["method"].as_str().unwrap_or_default().to_owned();
                    let answer = if refused {
                        json!({ "id": command["id"], "sessionId": command["sessionId"], "error": { "code": -32000, "message": "refused" } })
                    } else if method == "Page.addScriptToEvaluateOnNewDocument" {
                        json!({ "id": command["id"], "sessionId": command["sessionId"], "result": { "identifier": "1" } })
                    } else {
                        json!({ "id": command["id"], "sessionId": command["sessionId"], "result": {} })
                    };
                    methods.push(method);
                    assert!(played.send(&[answer]).await);
                }
                () = &mut connection => panic!("the page's connection ended before the test"),
            }
        };
        drop((on_child, driver, played));
        connection.await;
        (methods, runs)
    })
}

/// A page's out-of-process frame takes `basic`'s values on its own
/// session, all of the page's but the screen, which the browser sets only
/// on a top-level target, and `js_injection`'s script; a dedicated worker
/// takes the user agent through `Network`, the time zone and the core
/// count, and no script; a Session with no method has its children take
/// nothing (ADR 0035's rows).
/// Covers: story:8-13
#[rstest]
#[case::basic_in_a_frame(Method::Basic, ChildKind::Frame, &[
    "Emulation.setUserAgentOverride",
    "Emulation.setTimezoneOverride",
    "Emulation.setLocaleOverride",
    "Emulation.setGeolocationOverride",
    "Emulation.setHardwareConcurrencyOverride",
])]
#[case::basic_in_a_worker(Method::Basic, ChildKind::Worker, &[
    "Network.setUserAgentOverride",
    "Emulation.setTimezoneOverride",
    "Emulation.setHardwareConcurrencyOverride",
])]
#[case::js_injection_in_a_frame(Method::JsInjection, ChildKind::Frame, &["Page.addScriptToEvaluateOnNewDocument"])]
#[case::js_injection_in_a_worker(Method::JsInjection, ChildKind::Worker, &[])]
fn a_pages_children_take_its_methods_values_before_they_run(
    #[case] method: Method,
    #[case] kind: ChildKind,
    #[case] methods: &[&str],
) {
    let plan = plan(method, SET, Some(at_the_exit()));

    assert_eq!(
        taken_by_child(Some(&plan), kind, false),
        (
            methods.iter().map(|method| (*method).to_owned()).collect(),
            true
        )
    );
}

/// A child of a Session without a plan takes nothing and runs.
#[rstest]
fn a_child_without_a_plan_takes_nothing(
    #[values(ChildKind::Frame, ChildKind::Worker)] kind: ChildKind,
) {
    assert_eq!(taken_by_child(None, kind, false), (Vec::new(), true));
}

/// A child that refuses its values does not run, so it never shows the
/// machine's own ones.
#[rstest]
fn a_child_that_refuses_its_values_does_not_run(
    #[values(ChildKind::Frame, ChildKind::Worker)] kind: ChildKind,
) {
    let plan = plan(Method::Basic, SET, Some(at_the_exit()));

    let (methods, runs) = taken_by_child(Some(&plan), kind, true);

    assert_eq!((methods.len(), runs), (1, false));
}

/// `js_injection`'s script writes each value as a JSON literal, so a value
/// cannot break out of it, and leaves out the values it has not.
#[test]
fn the_script_writes_each_value_as_a_literal() {
    let script = init_script(&Emulated {
        user_agent: Some(UserAgentOverride {
            user_agent: "Mozilla\"});alert(1);//".to_owned(),
            accept_language: None,
            platform: "Win32".to_owned(),
            client_hints: None,
        }),
        locale: Some("fr\"});alert(1);//".to_owned()),
        core_count: Some(4),
        screen: Some(ScreenSize {
            width: 1920,
            height: 1080,
            avail_width: 1920,
            avail_height: 1040,
        }),
        ..Emulated::default()
    })
    .expect("a script");

    assert!(script.contains(r#""Mozilla\"});alert(1);//""#), "{script}");
    assert!(script.contains(r#""platform", "Win32")"#), "{script}");
    assert!(script.contains(r#""fr\"});alert(1);//""#), "{script}");
    assert!(script.contains("\"hardwareConcurrency\", 4)"), "{script}");
    assert!(script.contains("\"width\", 1920)"), "{script}");
    assert!(script.contains("\"availHeight\", 1040)"), "{script}");
    assert!(!script.contains("timeZone"), "{script}");
    assert!(!script.contains("geolocation"), "{script}");
}

#[test]
fn wpt_launch_args_for_the_same_seed_are_byte_identical() {
    let first = serde_json::to_vec_pretty(&wpt_launch_args(WPT_IDENTITY_SEED)).unwrap();
    let second = serde_json::to_vec_pretty(&wpt_launch_args(WPT_IDENTITY_SEED)).unwrap();
    assert_eq!(first, second);
}

#[rstest]
#[case::none("none")]
#[case::js_injection("js_injection")]
fn wpt_launch_args_carry_engine_flags_without_per_process_arguments(#[case] method: &str) {
    let doc = serde_json::to_value(wpt_launch_args(WPT_IDENTITY_SEED)).unwrap();
    assert_eq!(doc["identity_seed"], json!(WPT_IDENTITY_SEED.0));
    assert!(
        doc["methods"].get("engine").is_none(),
        "engine method waits on Goodfetch Chromium (#969): {doc}"
    );
    let args: Vec<&str> = doc["methods"][method]["args"]
        .as_array()
        .expect("args array")
        .iter()
        .map(|v| v.as_str().expect("string arg"))
        .collect();
    assert!(
        args.contains(&"--disable-blink-features=AutomationControlled"),
        "{method} missing engine launch flag: {args:?}"
    );
    assert!(
        args.contains(&"--disable-back-forward-cache"),
        "{method} missing engine launch flag: {args:?}"
    );
    assert!(
        args.contains(&"--webrtc-ip-handling-policy=disable_non_proxied_udp"),
        "{method} missing engine launch flag: {args:?}"
    );
    assert!(
        args.contains(&"--host-resolver-rules=MAP *.invalid ~NOTFOUND"),
        "{method} missing engine launch flag: {args:?}"
    );
    assert!(
        !args.contains(&"--remote-debugging-pipe"),
        "{method} must not carry engine pipe flag for WPT: {args:?}"
    );
    assert!(
        !args.iter().any(|arg| arg.starts_with("--user-data-dir=")),
        "{method} must not carry per-run user-data-dir for WPT: {args:?}"
    );
    assert!(
        !args.contains(&"about:blank"),
        "{method} must not carry initial about:blank URL for WPT: {args:?}"
    );
}

#[test]
fn wpt_launch_args_include_init_script_only_for_js_injection() {
    let doc = serde_json::to_value(wpt_launch_args(WPT_IDENTITY_SEED)).unwrap();
    assert!(
        doc["methods"]["none"].get("init_script").is_none(),
        "none method has no init_script: {doc}"
    );
    let script = doc["methods"]["js_injection"]["init_script"]
        .as_str()
        .expect("js_injection init_script");
    let user_agent = doc["identity_profile"]["user_agent"]
        .as_str()
        .expect("drawn user_agent");
    assert!(
        script.contains(user_agent),
        "js_injection init_script must embed the drawn Identity profile's user agent"
    );
}

/// The committed `chromium/launch-args.json` matches what the engine emits at
/// [`WPT_IDENTITY_SEED`]. `xtask codegen --check` checks digests without
/// building `goodfetch-page` (ADR 0072); this test catches any transitive
/// drift in the engine's launch arguments or identity profile population.
#[test]
fn the_committed_wpt_launch_args_match_the_engine() {
    const COMMITTED: &str = include_str!("../../../../chromium/launch-args.json");
    let value = serde_json::to_value(wpt_launch_args(WPT_IDENTITY_SEED)).unwrap();
    let mut expected = serde_json::to_string_pretty(&value).unwrap();
    expected.push('\n');
    assert_eq!(
        COMMITTED, expected,
        "run `just codegen` and commit what it writes"
    );
}

#[test]
fn changing_the_wpt_identity_seed_changes_the_generated_launch_args() {
    let first = serde_json::to_vec_pretty(&wpt_launch_args(IdentitySeed(1))).unwrap();
    let second = serde_json::to_vec_pretty(&wpt_launch_args(IdentitySeed(2))).unwrap();
    assert_ne!(first, second);

    let first_doc: Value = serde_json::from_slice(&first).unwrap();
    let second_doc: Value = serde_json::from_slice(&second).unwrap();
    assert_ne!(first_doc["identity_seed"], second_doc["identity_seed"]);
    assert_ne!(
        first_doc["methods"]["js_injection"]["init_script"],
        second_doc["methods"]["js_injection"]["init_script"]
    );
}
