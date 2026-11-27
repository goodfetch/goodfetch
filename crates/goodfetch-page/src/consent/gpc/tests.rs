use goodfetch_engine::settings::Values;
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::page::SessionId;
use goodfetch_protocol::run::warning::GPC_FORCED;
use goodfetch_protocol::settings::SettingsLayer;
use goodfetch_protocol::settings::session_source::{BrowserName, SessionSource};
use rstest::rstest;
use serde_json::{Map, Value, json};

use super::{GpcMode, GpcPlan, GpcWarning, PresentedBrowser};

/// Covers: story:5-103, setting:consent.gpc
#[rstest]
#[case::chrome(PresentedBrowser::Chrome, false)]
#[case::edge(PresentedBrowser::Edge, false)]
#[case::safari(PresentedBrowser::Safari, false)]
#[case::firefox(PresentedBrowser::Firefox, false)]
#[case::brave(PresentedBrowser::Brave, true)]
fn auto_sends_gpc_only_when_the_presented_browser_would(
    #[case] browser: PresentedBrowser,
    #[case] expected_gpc: bool,
) {
    let plan = GpcPlan::resolve(GpcMode::Auto, browser, false);
    assert_eq!(plan.own_browser_gpc(), expected_gpc);
    assert!(!plan.header_only());
    assert_eq!(plan.warning(), None);
}

/// Covers: story:5-103, setting:consent.gpc
#[rstest]
#[case::chrome(PresentedBrowser::Chrome, true)]
#[case::edge(PresentedBrowser::Edge, true)]
#[case::safari(PresentedBrowser::Safari, true)]
#[case::firefox(PresentedBrowser::Firefox, true)]
#[case::brave(PresentedBrowser::Brave, false)]
fn on_forces_own_browser_and_warns_only_when_contradicting_identity(
    #[case] browser: PresentedBrowser,
    #[case] expects_warning: bool,
) {
    let plan = GpcPlan::resolve(GpcMode::On, browser, false);
    assert!(plan.own_browser_gpc());
    assert!(!plan.header_only());
    assert_eq!(plan.warning().is_some(), expects_warning);
    if let Some(warning) = plan.warning() {
        let session = SessionId("session-1".to_owned());
        let run_warning = warning.to_run_warning(&session);
        assert_eq!(run_warning.code, GPC_FORCED);
        assert_eq!(run_warning.sessions, vec!["session-1"]);
        assert!(run_warning.message.contains(browser.label()));
    }
}

/// Covers: story:5-103, setting:consent.gpc
#[test]
fn off_never_sends_gpc_even_for_browsers_that_send_it_by_default() {
    for browser in [
        PresentedBrowser::Chrome,
        PresentedBrowser::Brave,
        PresentedBrowser::Edge,
        PresentedBrowser::Safari,
        PresentedBrowser::Firefox,
    ] {
        for borrowed in [false, true] {
            let plan = GpcPlan::resolve(GpcMode::Off, browser, borrowed);
            assert_eq!(plan, GpcPlan::Off);
            assert!(!plan.own_browser_gpc());
            assert!(!plan.header_only());
            assert_eq!(plan.warning(), None);
        }
    }
}

/// Covers: story:5-103, setting:consent.gpc
#[test]
fn borrowed_browser_uses_header_only_when_forced_and_warns_that_js_property_is_absent() {
    assert_eq!(
        GpcPlan::resolve(GpcMode::Auto, PresentedBrowser::Brave, true),
        GpcPlan::Off
    );
    let forced = GpcPlan::resolve(GpcMode::On, PresentedBrowser::Chrome, true);
    assert_eq!(
        forced,
        GpcPlan::HeaderOnly {
            warning: GpcWarning::BorrowedHeaderOnly
        }
    );
    assert!(!forced.own_browser_gpc());
    assert!(forced.header_only());
    let session = SessionId("session-borrowed".to_owned());
    let warning = forced.warning().expect("warning").to_run_warning(&session);
    assert_eq!(warning.code, GPC_FORCED);
    assert_eq!(warning.sessions, vec!["session-borrowed"]);
    assert!(warning.message.contains("navigator.globalPrivacyControl"));
}

#[test]
fn source_maps_browser_names_to_presented_browser() {
    let brave = SessionSource::ProfileDirectory {
        browser: BrowserName::Brave,
        path: "/tmp/brave".to_owned(),
        chromium_profile: None,
        domains: Vec::new(),
    };
    let chrome = SessionSource::ProfileDirectory {
        browser: BrowserName::Chrome,
        path: "/tmp/chrome".to_owned(),
        chromium_profile: None,
        domains: Vec::new(),
    };
    assert_eq!(
        PresentedBrowser::of_source(Some(&brave)),
        PresentedBrowser::Brave
    );
    assert_eq!(
        PresentedBrowser::of_source(Some(&chrome)),
        PresentedBrowser::Chrome
    );
    assert_eq!(PresentedBrowser::of_source(None), PresentedBrowser::Chrome);
}

#[test]
fn session_setting_overrides_default_mode() {
    let mut map = Map::new();
    map.insert("consent.gpc".to_owned(), json!("on"));
    let own = Values::read(SettingsLayer::Run, &map, Product::Collie).expect("valid");
    assert_eq!(GpcMode::of(&own, None), GpcMode::On);

    let mut off_map = Map::new();
    off_map.insert("consent.gpc".to_owned(), Value::String("off".to_owned()));
    let off = Values::read(SettingsLayer::Run, &off_map, Product::Collie).expect("valid");
    assert_eq!(GpcMode::of(&off, None), GpcMode::Off);

    assert_eq!(GpcMode::of(&Values::default(), None), GpcMode::Auto);
}
