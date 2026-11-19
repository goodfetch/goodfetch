use std::time::Duration;

use goodfetch_protocol::catalog::SettingScope;
use goodfetch_protocol::settings::{SettingNotApplicableDetails, SettingsLayer};
use serde_json::json;

use super::{Settings, browser, checkpoint, content, timeout};
use crate::ErrorCode;

#[test]
fn settings_nest_by_segment_with_durations_in_whole_milliseconds_rounded_up_and_none_as_null() {
    let settings = Settings::new()
        .set(timeout::NAVIGATION, Duration::from_micros(1500))
        .set(checkpoint::INTERVAL, None)
        .set(
            browser::BLOCK_RESOURCES,
            vec![browser::BlockResourcesItem::Image],
        )
        .set(content::EXTRACTOR, content::Extractor::DomSmoothie);

    let sent = settings.within(SettingScope::Run).unwrap();

    assert_eq!(
        serde_json::Value::Object(sent),
        json!({
            "browser": { "block_resources": ["image"] },
            "checkpoint": { "interval": null },
            "content": { "extractor": "dom_smoothie" },
            "timeout": { "navigation": 2 },
        })
    );
}

#[test]
fn a_key_set_again_takes_its_last_value() {
    let settings = Settings::new()
        .set(content::EXTRACTOR, content::Extractor::DomSmoothie)
        .set(content::EXTRACTOR, content::Extractor::Trafilatura);

    let sent = settings.within(SettingScope::Page).unwrap();

    assert_eq!(
        serde_json::Value::Object(sent),
        json!({ "content": { "extractor": "trafilatura" } })
    );
}

#[test]
fn no_settings_send_an_empty_member() {
    assert!(
        Settings::new()
            .within(SettingScope::Session)
            .unwrap()
            .is_empty()
    );
}

#[rstest::rstest]
#[case::connection_only_on_a_run(
    Settings::new().set(timeout::RUN, Duration::from_secs(60)),
    SettingScope::Run,
    "timeout.run",
    "set it in the settings of the connection"
)]
#[case::a_sessions_on_a_page(
    Settings::new().set(browser::ACCESS_MODE, browser::AccessMode::Stealth),
    SettingScope::Page,
    "browser.access_mode",
    "set it in the settings of the connection, a Run's start or a Session's opening"
)]
fn a_key_outside_the_scope_is_refused_naming_where_it_is_set(
    #[case] settings: Settings,
    #[case] scope: SettingScope,
    #[case] key: &str,
    #[case] fix: &str,
) {
    let refused = settings.within(scope).unwrap_err();

    assert_eq!(
        (
            refused.code(),
            refused.details::<SettingNotApplicableDetails>()
        ),
        (
            ErrorCode::SETTING_NOT_APPLICABLE,
            Some(SettingNotApplicableDetails {
                key: key.to_owned(),
                layer: SettingsLayer::Run,
                fix: fix.to_owned(),
            })
        )
    );
}
