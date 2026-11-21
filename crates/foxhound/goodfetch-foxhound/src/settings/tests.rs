use std::time::Duration;

use goodfetch_protocol::catalog::SettingScope;
use goodfetch_protocol::foxhound::frontier::PathBudget;
use goodfetch_protocol::settings::{SettingNotApplicableDetails, SettingsLayer};
use serde_json::json;

use super::{Settings, content, crawl, fetch, politeness, session, timeout};
use crate::ErrorCode;

#[test]
fn a_label_sets_foxhounds_and_collies_label_keys_nested_by_segment() {
    let label = Settings::new()
        .set(fetch::TIER, fetch::Tier::Http)
        .set(fetch::ENCODING, None)
        .set(crawl::KEEP_URL_FRAGMENT, true)
        .set(timeout::HOOK, Duration::from_secs(2))
        .set(content::EXTRACTOR, content::Extractor::Trafilatura)
        .set(session::SOURCE, None);

    let sent = label.within(SettingScope::Label).unwrap();

    assert_eq!(
        serde_json::Value::Object(sent),
        json!({
            "content": { "extractor": "trafilatura" },
            "crawl": { "keep_url_fragment": true },
            "fetch": { "encoding": null, "tier": "http" },
            "session": { "source": null },
            "timeout": { "hook": 2000 },
        })
    );
}

#[test]
fn a_key_a_label_does_not_set_fails_naming_the_scopes_that_do() {
    let label = Settings::new()
        .set(fetch::TIER, fetch::Tier::Browser)
        .set(politeness::domain::CONCURRENCY, 2);

    let refused = label.within(SettingScope::Label).unwrap_err();

    assert_eq!(
        (
            refused.code(),
            refused.details::<SettingNotApplicableDetails>()
        ),
        (
            ErrorCode::SETTING_NOT_APPLICABLE,
            Some(SettingNotApplicableDetails {
                key: "politeness.domain.concurrency".to_owned(),
                layer: SettingsLayer::Run,
                fix: "set it in the settings of the connection or a Run's start".to_owned(),
            })
        )
    );
}

#[test]
fn a_crawl_sets_collies_run_keys_beside_foxhounds() {
    let crawl = Settings::new()
        .set(timeout::NAVIGATION, Duration::from_secs(30))
        .set(crawl::MAX_DEPTH, 3)
        .set(
            crawl::PATH_BUDGETS,
            vec![PathBudget {
                pattern: "/shop?*".to_owned(),
                max_requests: 50,
            }],
        )
        .set(fetch::TIER, fetch::Tier::Auto);

    let sent = crawl.within(SettingScope::Run).unwrap();

    assert_eq!(
        serde_json::Value::Object(sent),
        json!({
            "crawl": {
                "max_depth": 3,
                "path_budgets": [{ "pattern": "/shop?*", "max_requests": 50 }],
            },
            "fetch": { "tier": "auto" },
            "timeout": { "navigation": 30000 },
        })
    );
}
