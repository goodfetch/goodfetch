//! The Fingerprint method driver's pure part: which Sessions get a plan,
//! where they are placed, which primitive each method fills, the warning
//! each one leaves, what its record says and the script `js_injection`
//! writes.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

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
use crate::cdp::{Emulated, Geolocation};
use crate::pool::Placement;

const SET: MethodSource = MethodSource::Set(SettingsLayer::Run);
const FELL_BACK: MethodSource = MethodSource::FellBack;

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
            generated_identity: false,
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
/// off the browser's `Runtime` domain.
fn plan(method: Method, source: MethodSource, values: Emulated) -> FingerprintPlan {
    FingerprintPlan {
        method,
        source,
        stealth: SettingsLayer::Run,
        values,
        runtime: RuntimeDomain::Refused,
    }
}

fn values() -> Emulated {
    Emulated {
        time_zone: Some("Europe/Paris".to_owned()),
        locale: Some("fr-FR".to_owned()),
        geolocation: Some(Geolocation {
            latitude: 48.85,
            longitude: 2.35,
            accuracy: 10.0,
        }),
        core_count: Some(8),
    }
}

/// Only a `stealth` Session with a built method gets a plan, which keeps
/// where its method came from.
#[rstest]
#[case::standard(identity(AccessMode::Standard, FingerprintMethod::Basic, SET), None)]
#[case::the_fallback(
    identity(AccessMode::Stealth, FingerprintMethod::Basic, FELL_BACK),
    Some(plan(Method::Basic, FELL_BACK, Emulated::default()))
)]
#[case::js_injection(
    identity(AccessMode::Stealth, FingerprintMethod::JsInjection, SET),
    Some(plan(Method::JsInjection, SET, Emulated::default()))
)]
#[case::none(
    identity(AccessMode::Stealth, FingerprintMethod::None, SET),
    Some(plan(Method::None, SET, Emulated::default()))
)]
#[case::engine(identity(AccessMode::Stealth, FingerprintMethod::Engine, SET), None)]
fn a_stealth_session_with_a_built_method_has_a_plan(
    #[case] identity: SessionIdentityPlan,
    #[case] expected: Option<FingerprintPlan>,
) {
    assert_eq!(FingerprintPlan::of(&identity), expected);
}

/// Every built method shares a browser process as contexts.
#[rstest]
fn every_built_method_shares_a_browser_process(
    #[values(Method::None, Method::JsInjection, Method::Basic)] method: Method,
) {
    assert_eq!(plan(method, SET, values()).placement(), Placement::Shared);
}

/// `basic` shows its values through the browser's overrides and
/// `js_injection` through its script; `none` shows the machine's own, and
/// no method writes anything while it has no value to show.
#[rstest]
#[case::basic(Method::Basic, true, false)]
#[case::js_injection(Method::JsInjection, false, true)]
#[case::none(Method::None, false, false)]
fn each_method_fills_its_own_primitive(
    #[case] method: Method,
    #[case] overrides: bool,
    #[case] script: bool,
) {
    let masked = plan(method, SET, values());
    let unmasked = plan(method, SET, Emulated::default());

    assert_eq!(
        (masked.emulated().is_some(), masked.init_script().is_some()),
        (overrides, script)
    );
    assert_eq!((unmasked.emulated(), unmasked.init_script()), (None, None));
}

/// Each method leaves its own warning on the Session, `basic`'s fallback
/// one apart from a chosen `basic`; `none`'s stands once for every
/// Session of the Run, and a masking method without values says its pages
/// show the machine's own.
#[rstest]
#[case::none(Method::None, SET, values(), FINGERPRINT_NONE, true, &[], false)]
#[case::js_injection(Method::JsInjection, SET, values(), FINGERPRINT_JS_INJECTION, false, &["session-1"], false)]
#[case::basic(Method::Basic, SET, values(), FINGERPRINT_BASIC, false, &["session-1"], false)]
#[case::the_fallback(Method::Basic, FELL_BACK, values(), FINGERPRINT_BASIC_FALLBACK, false, &["session-1"], false)]
#[case::basic_without_values(Method::Basic, SET, Emulated::default(), FINGERPRINT_BASIC, false, &["session-1"], true)]
#[case::none_without_values(Method::None, SET, Emulated::default(), FINGERPRINT_NONE, true, &[], false)]
fn each_method_leaves_its_warning(
    #[case] method: Method,
    #[case] source: MethodSource,
    #[case] values: Emulated,
    #[case] code: &str,
    #[case] once: bool,
    #[case] sessions: &[&str],
    #[case] machine_values: bool,
) {
    let session = SessionId("session-1".to_owned());

    let (warning, for_the_run) = plan(method, source, values).warning(&session);

    assert_eq!(
        (
            warning.code.as_str(),
            for_the_run,
            warning.sessions,
            warning
                .message
                .contains("no Identity profile gives them values yet"),
        ),
        (
            code,
            once,
            sessions
                .iter()
                .map(|&session| session.to_owned())
                .collect::<Vec<_>>(),
            machine_values
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
    let plan = plan(method, source, Emulated::default());

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
        ..plan(Method::Basic, SET, values())
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
        locale: Some("fr\"});alert(1);//".to_owned()),
        core_count: Some(4),
        ..Emulated::default()
    })
    .expect("a script");

    assert!(script.contains(r#""fr\"});alert(1);//""#), "{script}");
    assert!(script.contains("\"hardwareConcurrency\", 4)"), "{script}");
    assert!(!script.contains("timeZone"), "{script}");
    assert!(!script.contains("geolocation"), "{script}");
}
