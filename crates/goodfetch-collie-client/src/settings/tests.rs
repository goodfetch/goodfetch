use std::time::Duration;

use goodfetch_protocol::catalog::SettingScope;
use goodfetch_protocol::run::durability::BatchThreshold;
use goodfetch_protocol::settings::page_check::{PageCheck, PageMatch};
use goodfetch_protocol::settings::session_source::SessionSource;
use goodfetch_protocol::settings::{SettingNotApplicableDetails, SettingsLayer};
use goodfetch_test_support::logged;
use serde_json::json;

use super::{
    Settings, browser, checkpoint, content, egress, governor, network, run_state, sample, session,
    timeout,
};
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
fn a_whole_number_takes_a_bare_literal_whether_or_not_it_can_be_cleared() {
    let settings = Settings::new()
        .set(network::MAX_JSON_DEPTH, 64)
        .set(governor::browser::MEMORY_CAP, 2_048);

    let sent = settings.within(SettingScope::Run).unwrap();

    assert_eq!(
        serde_json::Value::Object(sent),
        json!({
            "governor": { "browser": { "memory_cap": 2048 } },
            "network": { "max_json_depth": 64 },
        })
    );
}

#[test]
fn a_shape_of_the_protocol_goes_as_its_json() {
    let settings = Settings::new()
        .set(
            session::PAGE_CHECK,
            PageCheck {
                login_page: Some(PageMatch::Css {
                    selector: "#login".to_owned(),
                }),
                expected_page: None,
            },
        )
        .set(
            session::SOURCE,
            SessionSource::SavedIdentity {
                name: "shop".to_owned(),
            },
        )
        .set(run_state::batch::BYTES, BatchThreshold::Never)
        .set(egress::LOCAL_ADDRESSES, None);

    let sent = settings.within(SettingScope::Run).unwrap();

    assert_eq!(
        serde_json::Value::Object(sent),
        json!({
            "egress": { "local_addresses": null },
            "run_state": { "batch": { "bytes": "none" } },
            "session": {
                "page_check": { "login_page": { "kind": "css", "selector": "#login" } },
                "source": { "kind": "saved_identity", "name": "shop" },
            },
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

/// Setting a deprecated key warns once per process through `tracing`, and
/// sends it as any other (ADR 0099).
///
/// Covers: story:11-182
#[test]
#[expect(
    deprecated,
    reason = "the sample's deprecated key is what the warning is for"
)]
fn setting_a_deprecated_key_warns_once_per_process_through_tracing() {
    let (sent, log) = logged(|| {
        let first = Settings::new().set(sample::ECHO_REPEATS, 2_u64);
        let _again = Settings::new().set(sample::ECHO_REPEATS, 3_u64);
        let _other = Settings::new().set(timeout::NAVIGATION, Duration::from_secs(1));
        first.within(SettingScope::Run).unwrap()
    });

    let shown = String::from_utf8_lossy(&log);
    assert_eq!(
        shown
            .matches(
                "The setting `sample.echo_repeats` is deprecated. Since 0.1, removed no earlier than 1.0: A sample setting, which the real catalog replaces."
            )
            .count(),
        1,
        "{shown}"
    );
    assert_eq!(shown.matches("is deprecated").count(), 1, "{shown}");
    assert!(shown.contains("WARN"), "{shown}");
    assert_eq!(
        serde_json::Value::Object(sent),
        json!({ "sample": { "echo_repeats": 2 } })
    );
}
