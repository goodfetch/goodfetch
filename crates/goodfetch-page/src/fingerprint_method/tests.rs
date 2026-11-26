//! The Fingerprint method driver's pure part: which Sessions get a plan,
//! where they are placed, which primitive each method fills, the warning
//! each one leaves and the script `js_injection` writes.

use goodfetch_protocol::page::SessionId;
use goodfetch_protocol::run::warning::{
    FINGERPRINT_BASIC, FINGERPRINT_BASIC_FALLBACK, FINGERPRINT_JS_INJECTION, FINGERPRINT_NONE,
};
use goodfetch_protocol::settings::SettingsLayer;
use rstest::rstest;

use super::{FingerprintPlan, Method, init_script};
use crate::access_mode::{
    AccessMode, ChromiumBuild, FingerprintMethod, MethodSource, SessionIdentityPlan, StealthProfile,
};
use crate::cdp::{Emulated, Geolocation};
use crate::pool::Placement;

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
        }),
        signing: None,
    }
}

fn plan(method: Method, fell_back: bool, values: Emulated) -> FingerprintPlan {
    FingerprintPlan {
        method,
        fell_back,
        layer: SettingsLayer::Run,
        values,
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
/// whether its method fell back to `basic`.
#[rstest]
#[case::standard(
    identity(AccessMode::Standard, FingerprintMethod::Basic, MethodSource::Set),
    None
)]
#[case::the_fallback(
    identity(AccessMode::Stealth, FingerprintMethod::Basic, MethodSource::FellBack),
    Some(plan(Method::Basic, true, Emulated::default()))
)]
#[case::js_injection(
    identity(AccessMode::Stealth, FingerprintMethod::JsInjection, MethodSource::Set),
    Some(plan(Method::JsInjection, false, Emulated::default()))
)]
#[case::none(
    identity(AccessMode::Stealth, FingerprintMethod::None, MethodSource::Set),
    Some(plan(Method::None, false, Emulated::default()))
)]
#[case::engine(
    identity(AccessMode::Stealth, FingerprintMethod::Engine, MethodSource::Set),
    None
)]
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
    assert_eq!(plan(method, false, values()).placement(), Placement::Shared);
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
    let masked = plan(method, false, values());
    let unmasked = plan(method, false, Emulated::default());

    assert_eq!(
        (masked.emulated().is_some(), masked.init_script().is_some()),
        (overrides, script)
    );
    assert_eq!((unmasked.emulated(), unmasked.init_script()), (None, None));
}

/// Each method leaves its own warning on the Session, `basic`'s fallback
/// one apart from a chosen `basic`; `none`'s stands once for every
/// Session of the Run.
#[rstest]
#[case::none(Method::None, false, FINGERPRINT_NONE, true)]
#[case::js_injection(Method::JsInjection, false, FINGERPRINT_JS_INJECTION, false)]
#[case::basic(Method::Basic, false, FINGERPRINT_BASIC, false)]
#[case::the_fallback(Method::Basic, true, FINGERPRINT_BASIC_FALLBACK, false)]
fn each_method_leaves_its_warning(
    #[case] method: Method,
    #[case] fell_back: bool,
    #[case] code: &str,
    #[case] once: bool,
) {
    let session = SessionId("session-1".to_owned());

    let (warning, for_the_run) = plan(method, fell_back, Emulated::default()).warning(&session);

    assert_eq!(
        (warning.code.as_str(), for_the_run, warning.sessions),
        (
            code,
            once,
            if once {
                Vec::new()
            } else {
                vec!["session-1".to_owned()]
            }
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
