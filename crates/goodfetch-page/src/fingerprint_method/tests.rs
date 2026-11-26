//! The Fingerprint method driver's pure part: which Sessions get a plan,
//! where they are placed, which primitive each method fills with the
//! Session's Identity profile, the warning each one leaves, what its
//! record says and the script `js_injection` writes.
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
use rstest::rstest;
use serde_json::{Value, json};

use super::{FingerprintPlan, Method, init_script};
use crate::access_mode::{
    AccessMode, ChromiumBuild, FingerprintMethod, MethodSource, RuntimeDomain, SessionIdentityPlan,
    StealthProfile,
};
use crate::cdp::{Emulated, Geolocation, ScreenSize, UserAgentOverride};
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
fn profile() -> IdentityProfile {
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
    Some(plan(Method::Basic, FELL_BACK, Some(profile())))
)]
#[case::js_injection(
    identity(AccessMode::Stealth, FingerprintMethod::JsInjection, SET),
    Some(plan(Method::JsInjection, SET, Some(profile())))
)]
#[case::none(
    identity(AccessMode::Stealth, FingerprintMethod::None, SET),
    Some(plan(Method::None, SET, Some(profile())))
)]
#[case::engine(identity(AccessMode::Stealth, FingerprintMethod::Engine, SET), None)]
fn a_stealth_session_with_a_built_method_has_a_plan(
    #[case] identity: SessionIdentityPlan,
    #[case] expected: Option<FingerprintPlan>,
) {
    assert_eq!(FingerprintPlan::of(&identity, Some(profile())), expected);
}

/// Every built method shares a browser process as contexts.
#[rstest]
fn every_built_method_shares_a_browser_process(
    #[values(Method::None, Method::JsInjection, Method::Basic)] method: Method,
) {
    assert_eq!(
        plan(method, SET, Some(profile())).placement(),
        Placement::Shared
    );
}

/// `basic` shows its Identity profile through the browser's overrides and
/// `js_injection` through its script; `none` shows the machine's own, and
/// no method writes anything without a profile, `stealth.generated_identity`
/// off.
#[rstest]
#[case::basic(Method::Basic, true, false)]
#[case::js_injection(Method::JsInjection, false, true)]
#[case::none(Method::None, false, false)]
fn each_method_fills_its_own_primitive(
    #[case] method: Method,
    #[case] overrides: bool,
    #[case] script: bool,
) {
    let (emulated, init) = plan(method, SET, Some(profile())).primitives(Some(PRODUCT));

    assert_eq!((emulated.is_some(), init.is_some()), (overrides, script));
    assert_eq!(
        plan(method, SET, None).primitives(Some(PRODUCT)),
        (None, None)
    );
}

/// `basic` shows the profile's user agent and client hints at the
/// browser's own version, which no built method changes, and its locale,
/// core count, screen, and the exit's time zone and geolocation.
#[test]
fn basic_shows_the_profile_at_the_browsers_version() {
    let profile = at_the_exit();

    let (emulated, _) = plan(Method::Basic, SET, Some(profile.clone())).primitives(Some(PRODUCT));

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
            emulated.time_zone.as_deref(),
            emulated.geolocation,
            emulated.core_count,
            emulated.screen,
        ),
        (
            Some("fr-FR"),
            Some("Europe/Paris"),
            Some(Geolocation {
                latitude: 48.85,
                longitude: 2.35,
                accuracy: 10.0,
            }),
            Some(profile.navigator.hardware_concurrency),
            Some(ScreenSize {
                width: profile.screen.width,
                height: profile.screen.height,
            }),
        )
    );
}

/// A browser whose product names no version it can read keeps the
/// profile's drawn one.
#[rstest]
#[case::unknown(None)]
#[case::unreadable(Some("Chrome"))]
fn an_unread_version_keeps_the_drawn_one(#[case] product: Option<&str>) {
    let profile = profile();

    let (emulated, _) = plan(Method::Basic, SET, Some(profile.clone())).primitives(product);

    assert_eq!(
        emulated.and_then(|values| values.user_agent.map(|agent| agent.user_agent)),
        Some(profile.user_agent)
    );
}

/// Each method leaves its own warning on the Session, `basic`'s fallback
/// one apart from a chosen `basic`; `none`'s stands once for every
/// Session of the Run. A masking method without a profile says its pages
/// show the browser's own values, naming `stealth.generated_identity`, and
/// one whose profile has no exit's time zone says its time zone stays the
/// machine's.
#[rstest]
#[case::none(Method::None, SET, Some(at_the_exit()), FINGERPRINT_NONE, true, &[], (false, false))]
#[case::js_injection(Method::JsInjection, SET, Some(at_the_exit()), FINGERPRINT_JS_INJECTION, false, &["session-1"], (false, false))]
#[case::basic(Method::Basic, SET, Some(at_the_exit()), FINGERPRINT_BASIC, false, &["session-1"], (false, false))]
#[case::the_fallback(Method::Basic, FELL_BACK, Some(at_the_exit()), FINGERPRINT_BASIC_FALLBACK, false, &["session-1"], (false, false))]
#[case::basic_without_a_profile(Method::Basic, SET, None, FINGERPRINT_BASIC, false, &["session-1"], (true, false))]
#[case::basic_before_its_exit(Method::Basic, SET, Some(profile()), FINGERPRINT_BASIC, false, &["session-1"], (false, true))]
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
        ..plan(Method::Basic, SET, Some(profile()))
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
        }),
        ..Emulated::default()
    })
    .expect("a script");

    assert!(script.contains(r#""Mozilla\"});alert(1);//""#), "{script}");
    assert!(script.contains(r#""platform", "Win32")"#), "{script}");
    assert!(script.contains(r#""fr\"});alert(1);//""#), "{script}");
    assert!(script.contains("\"hardwareConcurrency\", 4)"), "{script}");
    assert!(script.contains("\"width\", 1920)"), "{script}");
    assert!(!script.contains("timeZone"), "{script}");
    assert!(!script.contains("geolocation"), "{script}");
}
