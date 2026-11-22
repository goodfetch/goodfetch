//! `resolve` as a table over Access modes, Session sources and browser
//! families: each case gives a Session's own settings, its Run's and its
//! user configuration's, and the plan or the refusal it opens with.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_engine::settings::{Layers, Resolved, Values, resolve as resolve_settings};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::secret_reference::SecretReference;
use goodfetch_protocol::settings::SettingsLayer;
use goodfetch_protocol::settings::session_source::SessionSource;
use rstest::rstest;
use serde_json::{Map, Value, json};

use super::{
    AccessMode, BrowserFamily, ChromiumBuild, FingerprintMethod, Refusal, SessionIdentityPlan,
    SessionSettings, SigningPlan, StealthProfile, resolve,
};

/// The values `settings` sets in `layer`.
fn values(layer: SettingsLayer, settings: &Value) -> Values {
    let nested: Map<String, Value> = serde_json::from_value(settings.clone()).expect("an object");
    Values::read(layer, &nested, Product::Collie).expect("the settings read")
}

/// A Run's resolved settings, from its user configuration and its own.
fn run(user: &Value, run: &Value) -> Resolved {
    let layers = Layers {
        user_configuration: values(SettingsLayer::UserConfiguration, user),
        run: values(SettingsLayer::Run, run),
        ..Layers::default()
    };
    resolve_settings(&layers, Product::Collie, Product::Collie).expect("the Run's settings resolve")
}

fn source(value: &Value) -> SessionSource {
    serde_json::from_value(value.clone()).expect("a Session source")
}

fn standard(layer: SettingsLayer) -> SessionIdentityPlan {
    SessionIdentityPlan {
        mode: AccessMode::Standard,
        layer,
        profile: None,
        signing: None,
    }
}

/// An `identified` plan chosen in `layer`, signing with `env:BOT_KEY`, set
/// in `key_layer`, for the directory at `https://bot.example`.
fn identified(layer: SettingsLayer, key_layer: SettingsLayer) -> SessionIdentityPlan {
    SessionIdentityPlan {
        mode: AccessMode::Identified,
        layer,
        profile: None,
        signing: Some(SigningPlan {
            key: SecretReference::parse("env:BOT_KEY").expect("a Secret reference"),
            layer: key_layer,
            agent: "https://bot.example".to_owned(),
        }),
    }
}

/// A `stealth` plan chosen in `layer`: `stealth`'s defaults, as `set`
/// changes them.
fn stealth(layer: SettingsLayer, set: impl FnOnce(&mut StealthProfile)) -> SessionIdentityPlan {
    let mut profile = StealthProfile {
        fingerprint_method: FingerprintMethod::Engine,
        generated_identity: true,
        os: Vec::new(),
        device: Vec::new(),
        brand: Vec::new(),
        human_input: true,
        chromium_build: ChromiumBuild::Goodfetch,
        headless: false,
    };
    set(&mut profile);
    SessionIdentityPlan {
        mode: AccessMode::Stealth,
        layer,
        profile: Some(profile),
        signing: None,
    }
}

fn defaults(_: &mut StealthProfile) {}

const RUN: SettingsLayer = SettingsLayer::Run;
const USER: SettingsLayer = SettingsLayer::UserConfiguration;
const NOTHING: Value = Value::Null;

/// Covers: story:8-2, story:8-3, story:8-4, story:8-5, story:8-6, story:8-9, story:8-10, story:8-102
/// Covers: setting:browser.access_mode, setting:stealth.fingerprint_method, setting:stealth.generated_identity, setting:browser.headless_shell
/// Covers: setting:stealth.identity.os, setting:stealth.identity.device, setting:stealth.identity.brand
/// Covers: setting:stealth.human_input, setting:stealth.chromium_build
/// Covers: setting:identified.signing_key, setting:identified.signature_agent
#[rstest]
// Modes, on a fresh Session of the Chromium family.
#[case::nothing_set_is_standard(json!({}), json!({}), json!({}), NOTHING, BrowserFamily::Chromium, Ok(standard(SettingsLayer::Default)))]
#[case::identified_from_the_run(json!({}), json!({ "browser": { "access_mode": "identified" }, "identified": { "signing_key": "env:BOT_KEY", "signature_agent": "https://bot.example" } }), json!({}), NOTHING, BrowserFamily::Chromium, Ok(identified(RUN, RUN)))]
#[case::stealth_takes_its_defaults(json!({}), json!({ "browser": { "access_mode": "stealth" } }), json!({}), NOTHING, BrowserFamily::Chromium, Ok(stealth(RUN, defaults)))]
#[case::stealth_keeps_the_items_set(
    json!({ "browser": { "access_mode": "stealth" }, "stealth": { "human_input": false, "identity": { "os": ["windows"], "device": ["desktop"] } } }),
    json!({ "browser": { "headless": true }, "stealth": { "chromium_build": "clearcote" } }),
    json!({}),
    NOTHING,
    BrowserFamily::Chromium,
    Ok(stealth(RUN, |profile| {
        profile.human_input = false;
        profile.os = vec!["windows".to_owned()];
        profile.device = vec!["desktop".to_owned()];
        profile.headless = true;
        profile.chromium_build = ChromiumBuild::Clearcote;
    }))
)]
#[case::stealth_with_its_fingerprint_switched_off(json!({ "browser": { "access_mode": "stealth" }, "stealth": { "fingerprint_method": "none" } }), json!({}), json!({}), NOTHING, BrowserFamily::Chromium, Ok(stealth(RUN, |profile| profile.fingerprint_method = FingerprintMethod::None)))]
#[case::a_fingerprint_method_makes_it_stealth(json!({ "stealth": { "fingerprint_method": "basic" } }), json!({}), json!({}), NOTHING, BrowserFamily::Chromium, Ok(stealth(RUN, |profile| profile.fingerprint_method = FingerprintMethod::Basic)))]
#[case::an_inherited_fingerprint_method_makes_it_stealth(
    json!({}),
    json!({ "stealth": { "fingerprint_method": "js_injection", "generated_identity": false } }),
    json!({}),
    NOTHING,
    BrowserFamily::Chromium,
    Ok(stealth(RUN, |profile| {
        profile.fingerprint_method = FingerprintMethod::JsInjection;
        profile.generated_identity = false;
    }))
)]
#[case::a_closer_fingerprint_method_overrules_an_inherited_mode(json!({ "stealth": { "fingerprint_method": "engine" } }), json!({ "browser": { "access_mode": "standard" } }), json!({}), NOTHING, BrowserFamily::Chromium, Ok(stealth(RUN, defaults)))]
#[case::a_closer_mode_overrules_an_inherited_fingerprint_method(json!({ "browser": { "access_mode": "identified" } }), json!({ "stealth": { "fingerprint_method": "engine" } }), json!({ "identified": { "signing_key": "env:BOT_KEY", "signature_agent": "https://bot.example" } }), NOTHING, BrowserFamily::Chromium, Ok(identified(RUN, USER)))]
#[case::an_explicit_standard_beside_a_fingerprint_method(json!({ "browser": { "access_mode": "standard" }, "stealth": { "fingerprint_method": "basic" } }), json!({}), json!({}), NOTHING, BrowserFamily::Chromium, Err(Refusal::ModeBesideMethod { mode: AccessMode::Standard, layer: RUN }))]
#[case::an_identified_beside_a_fingerprint_method_in_one_layer(json!({}), json!({}), json!({ "browser": { "access_mode": "identified" }, "stealth": { "fingerprint_method": "engine" } }), NOTHING, BrowserFamily::Chromium, Err(Refusal::ModeBesideMethod { mode: AccessMode::Identified, layer: USER }))]
// Stealth items under `standard` and `identified`.
#[case::a_stealth_item_set_on_a_standard_session(json!({ "stealth": { "human_input": true } }), json!({}), json!({}), NOTHING, BrowserFamily::Chromium, Err(Refusal::ItemOnStandard { key: "stealth.human_input" }))]
#[case::no_fingerprint_set_on_a_standard_session(json!({ "stealth": { "fingerprint_method": "none" } }), json!({}), json!({}), NOTHING, BrowserFamily::Chromium, Err(Refusal::ItemOnStandard { key: "stealth.fingerprint_method" }))]
#[case::an_inherited_stealth_item_yields_to_standard(json!({}), json!({ "stealth": { "human_input": true, "identity": { "brand": ["chrome"] } } }), json!({}), NOTHING, BrowserFamily::Chromium, Ok(standard(SettingsLayer::Default)))]
#[case::a_masking_item_set_on_an_identified_session(json!({ "stealth": { "generated_identity": true } }), json!({ "browser": { "access_mode": "identified" } }), json!({}), NOTHING, BrowserFamily::Chromium, Err(Refusal::MaskingOnIdentified { key: "stealth.generated_identity" }))]
#[case::an_inherited_masking_item_yields_to_identified(json!({ "browser": { "access_mode": "identified" } }), json!({ "stealth": { "human_input": true, "chromium_build": "clearcote" }, "identified": { "signing_key": "env:BOT_KEY", "signature_agent": "https://bot.example" } }), json!({}), NOTHING, BrowserFamily::Chromium, Ok(identified(RUN, RUN)))]
// What an `identified` Session signs with.
#[case::identified_signs_with_its_own_key(json!({ "browser": { "access_mode": "identified" }, "identified": { "signing_key": "env:BOT_KEY", "signature_agent": "https://bot.example" } }), json!({}), json!({}), NOTHING, BrowserFamily::Chromium, Ok(identified(RUN, RUN)))]
#[case::identified_without_a_key(json!({ "browser": { "access_mode": "identified" }, "identified": { "signature_agent": "https://bot.example" } }), json!({}), json!({}), NOTHING, BrowserFamily::Chromium, Err(Refusal::UnsignedIdentified { key: "identified.signing_key", layer: RUN }))]
#[case::identified_without_an_agent(json!({}), json!({}), json!({ "browser": { "access_mode": "identified" }, "identified": { "signing_key": "env:BOT_KEY" } }), NOTHING, BrowserFamily::Chromium, Err(Refusal::UnsignedIdentified { key: "identified.signature_agent", layer: USER }))]
#[case::a_masking_item_is_refused_before_a_missing_key(json!({ "browser": { "access_mode": "identified" }, "stealth": { "human_input": true } }), json!({}), json!({}), NOTHING, BrowserFamily::Chromium, Err(Refusal::MaskingOnIdentified { key: "stealth.human_input" }))]
#[case::a_signing_item_set_on_a_standard_session(json!({ "identified": { "signing_key": "env:BOT_KEY" } }), json!({}), json!({}), NOTHING, BrowserFamily::Chromium, Err(Refusal::SigningNotIdentified { key: "identified.signing_key", mode: AccessMode::Standard }))]
#[case::a_signing_item_set_on_a_stealth_session(json!({ "browser": { "access_mode": "stealth" }, "identified": { "signature_agent": "https://bot.example" } }), json!({}), json!({}), NOTHING, BrowserFamily::Chromium, Err(Refusal::SigningNotIdentified { key: "identified.signature_agent", mode: AccessMode::Stealth }))]
#[case::inherited_signing_items_yield_to_standard(json!({}), json!({ "identified": { "signing_key": "env:BOT_KEY", "signature_agent": "https://bot.example" } }), json!({}), NOTHING, BrowserFamily::Chromium, Ok(standard(SettingsLayer::Default)))]
// Sources.
#[case::stealth_on_a_saved_identity(json!({ "browser": { "access_mode": "stealth" } }), json!({}), json!({}), json!({ "kind": "saved_identity", "name": "shop" }), BrowserFamily::Chromium, Ok(stealth(RUN, defaults)))]
#[case::identified_on_a_saved_identity(json!({ "browser": { "access_mode": "identified" } }), json!({ "identified": { "signing_key": "env:BOT_KEY", "signature_agent": "https://bot.example" } }), json!({}), json!({ "kind": "saved_identity", "name": "shop" }), BrowserFamily::Chromium, Ok(identified(RUN, RUN)))]
#[case::identified_at_a_state_file_import(json!({ "browser": { "access_mode": "identified" } }), json!({}), json!({}), json!({ "kind": "state_file", "path": "state.json" }), BrowserFamily::Chromium, Err(Refusal::IdentifiedAt { layer: RUN, source: "state_file" }))]
#[case::identified_at_a_cookie_header_import(json!({}), json!({}), json!({ "browser": { "access_mode": "identified" } }), json!({ "kind": "cookie_header", "header": "a=b", "domain": "example.com" }), BrowserFamily::Chromium, Err(Refusal::IdentifiedAt { layer: USER, source: "cookie_header" }))]
#[case::identified_at_an_installed_browser_import(json!({}), json!({ "browser": { "access_mode": "identified" } }), json!({}), json!({ "kind": "installed_browser", "browser": "edge", "domains": ["example.com"] }), BrowserFamily::Chromium, Err(Refusal::IdentifiedAt { layer: RUN, source: "installed_browser" }))]
#[case::identified_on_a_profile_directory(json!({ "browser": { "access_mode": "identified" } }), json!({}), json!({}), json!({ "kind": "profile_directory", "path": "/profile", "browser": "chrome", "domains": ["example.com"] }), BrowserFamily::Chromium, Err(Refusal::IdentifiedAt { layer: RUN, source: "profile_directory" }))]
#[case::stealth_at_an_import(json!({ "browser": { "access_mode": "stealth" } }), json!({}), json!({}), json!({ "kind": "state_file", "path": "state.json" }), BrowserFamily::Chromium, Ok(stealth(RUN, defaults)))]
#[case::stealth_on_a_firefox_profile_directory(json!({ "stealth": { "fingerprint_method": "engine" } }), json!({}), json!({}), json!({ "kind": "profile_directory", "path": "/profile", "browser": "firefox", "domains": ["example.com"] }), BrowserFamily::Firefox, Ok(stealth(RUN, defaults)))]
#[case::stealth_on_webkit(json!({ "browser": { "access_mode": "stealth" } }), json!({}), json!({}), json!({ "kind": "installed_browser", "browser": "safari", "domains": ["example.com"] }), BrowserFamily::WebKit, Err(Refusal::StealthOnWebKit { layer: RUN }))]
#[case::an_inherited_fingerprint_method_on_webkit(json!({}), json!({}), json!({ "stealth": { "fingerprint_method": "basic" } }), json!({ "kind": "profile_directory", "path": "/profile", "browser": "safari", "domains": ["example.com"] }), BrowserFamily::WebKit, Err(Refusal::StealthOnWebKit { layer: USER }))]
#[case::standard_on_webkit(json!({}), json!({}), json!({}), json!({ "kind": "installed_browser", "browser": "safari", "domains": ["example.com"] }), BrowserFamily::WebKit, Ok(standard(SettingsLayer::Default)))]
// A Borrowed browser stays `standard`.
#[case::stealth_set_where_a_borrowed_browser_opens(json!({ "browser": { "access_mode": "stealth" } }), json!({}), json!({}), json!({ "kind": "cdp_url", "url": "http://127.0.0.1:9222" }), BrowserFamily::Chromium, Err(Refusal::OnBorrowed { key: "browser.access_mode", source: "cdp_url" }))]
#[case::identified_set_where_a_borrowed_browser_opens(json!({ "browser": { "access_mode": "identified" } }), json!({}), json!({}), json!({ "kind": "running_browser", "browser": "chrome", "domains": ["example.com"] }), BrowserFamily::Chromium, Err(Refusal::OnBorrowed { key: "browser.access_mode", source: "running_browser" }))]
#[case::a_fingerprint_method_set_where_a_borrowed_browser_opens(json!({ "stealth": { "fingerprint_method": "engine" } }), json!({}), json!({}), json!({ "kind": "cdp_url", "url": "http://127.0.0.1:9222" }), BrowserFamily::Chromium, Err(Refusal::OnBorrowed { key: "stealth.fingerprint_method", source: "cdp_url" }))]
#[case::an_inherited_stealth_yields_to_a_borrowed_browser(json!({}), json!({ "browser": { "access_mode": "stealth" }, "stealth": { "human_input": true } }), json!({}), json!({ "kind": "cdp_url", "url": "http://127.0.0.1:9222" }), BrowserFamily::Chromium, Ok(standard(SettingsLayer::Default)))]
#[case::a_signing_item_set_where_a_borrowed_browser_opens(json!({ "identified": { "signing_key": "env:BOT_KEY" } }), json!({}), json!({}), json!({ "kind": "cdp_url", "url": "http://127.0.0.1:9222" }), BrowserFamily::Chromium, Err(Refusal::OnBorrowed { key: "identified.signing_key", source: "cdp_url" }))]
#[case::standard_set_where_a_borrowed_browser_opens(json!({ "browser": { "access_mode": "standard" } }), json!({}), json!({}), json!({ "kind": "cdp_url", "url": "http://127.0.0.1:9222" }), BrowserFamily::Chromium, Ok(standard(SettingsLayer::Default)))]
// The headless shell (#3224) runs only headless, and never under `stealth`.
#[case::the_headless_shell_opens_standard(json!({}), json!({ "browser": { "headless_shell": true } }), json!({}), NOTHING, BrowserFamily::Chromium, Ok(standard(SettingsLayer::Default)))]
#[case::the_headless_shell_beside_a_headful_browser(json!({}), json!({ "browser": { "headless_shell": true, "headless": false } }), json!({}), NOTHING, BrowserFamily::Chromium, Err(Refusal::ShellNotHeadless { layer: RUN }))]
#[case::a_configured_headless_shell_beside_a_headful_run(json!({}), json!({ "browser": { "headless": false } }), json!({ "browser": { "headless_shell": true } }), NOTHING, BrowserFamily::Chromium, Err(Refusal::ShellNotHeadless { layer: USER }))]
#[case::a_configured_headless_shell_under_stealth(json!({ "browser": { "access_mode": "stealth" } }), json!({}), json!({ "browser": { "headless_shell": true } }), NOTHING, BrowserFamily::Chromium, Err(Refusal::ShellUnderStealth { layer: USER }))]
#[case::the_headless_shell_yields_to_a_borrowed_browser(json!({}), json!({ "browser": { "headless_shell": true, "headless": false } }), json!({}), json!({ "kind": "cdp_url", "url": "http://127.0.0.1:9222" }), BrowserFamily::Chromium, Ok(standard(SettingsLayer::Default)))]
fn a_session_opens_in_the_access_mode_its_settings_and_source_resolve_to(
    #[case] own: Value,
    #[case] run_settings: Value,
    #[case] user: Value,
    #[case] source_value: Value,
    #[case] family: BrowserFamily,
    #[case] expected: Result<SessionIdentityPlan, Refusal>,
) {
    let own = values(SettingsLayer::Run, &own);
    let resolved = run(&user, &run_settings);
    let opened_from = (!source_value.is_null()).then(|| source(&source_value));
    let settings = SessionSettings::new(&own, Some(&resolved));

    assert_eq!(resolve(&settings, opened_from.as_ref(), family), expected);
}

/// A Session on a Run started without settings opens `standard`, and its
/// own settings still choose its mode.
#[test]
fn a_run_without_settings_leaves_the_session_its_own() {
    let nothing = Values::default();
    let own = values(
        SettingsLayer::Run,
        &json!({ "stealth": { "fingerprint_method": "basic" } }),
    );

    assert_eq!(
        resolve(
            &SessionSettings::new(&nothing, None),
            None,
            BrowserFamily::Chromium
        ),
        Ok(standard(SettingsLayer::Default))
    );
    assert_eq!(
        resolve(
            &SessionSettings::new(&own, None),
            None,
            BrowserFamily::Chromium
        ),
        Ok(stealth(RUN, |profile| profile.fingerprint_method =
            FingerprintMethod::Basic))
    );
}

/// A Session runs on its source's browser family: Chromium for a fresh one
/// and for a file, Firefox's and `WebKit`'s for theirs.
#[rstest]
#[case::fresh(NOTHING, BrowserFamily::Chromium)]
#[case::a_state_file(json!({ "kind": "state_file", "path": "state.json" }), BrowserFamily::Chromium)]
#[case::a_brave_profile(json!({ "kind": "profile_directory", "path": "/p", "browser": "brave", "domains": [] }), BrowserFamily::Chromium)]
#[case::a_firefox_profile(json!({ "kind": "profile_directory", "path": "/p", "browser": "firefox", "domains": [] }), BrowserFamily::Firefox)]
#[case::an_installed_safari(json!({ "kind": "installed_browser", "browser": "safari", "domains": [] }), BrowserFamily::WebKit)]
#[case::a_running_firefox(json!({ "kind": "running_browser", "browser": "firefox", "domains": [] }), BrowserFamily::Firefox)]
#[case::a_cdp_url(json!({ "kind": "cdp_url", "url": "http://127.0.0.1:9222" }), BrowserFamily::Chromium)]
fn a_session_runs_on_its_sources_browser_family(
    #[case] source_value: Value,
    #[case] expected: BrowserFamily,
) {
    let opened_from = (!source_value.is_null()).then(|| source(&source_value));

    assert_eq!(BrowserFamily::of(opened_from.as_ref()), expected);
}

/// A `standard` and an `identified` Session open; a `stealth` one is
/// refused with `capability_unavailable` (ADR 0083) until the tickets that
/// build its effects land, in the layer that chose it.
#[rstest]
#[case::standard(standard(SettingsLayer::Default), Ok(()))]
#[case::identified(identified(USER, USER), Ok(()))]
#[case::stealth(stealth(RUN, defaults), Err(Refusal::Unbuilt { mode: AccessMode::Stealth, layer: RUN }))]
fn only_a_built_access_mode_opens(
    #[case] plan: SessionIdentityPlan,
    #[case] expected: Result<(), Refusal>,
) {
    assert_eq!(plan.built().map(|_| ()), expected);
}
