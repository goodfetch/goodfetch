use std::time::Duration;

use goodfetch_protocol::catalog::SettingScope;
use goodfetch_protocol::foxhound::frontier::PathBudget;
use goodfetch_protocol::page::ExtractorName;
use goodfetch_protocol::settings::{SettingNotApplicableDetails, SettingsLayer};
use serde_json::json;

use super::{Settings, content, crawl, fetch, politeness, session, site_preferences, timeout};
use crate::ErrorCode;

#[test]
fn a_label_sets_foxhounds_and_collies_label_keys_nested_by_segment() {
    let label = Settings::new()
        .set(fetch::TIER, fetch::Tier::Http)
        .set(fetch::ENCODING, None)
        .set(crawl::KEEP_URL_FRAGMENT, true)
        .set(timeout::HOOK, Duration::from_secs(2))
        .set(content::EXTRACTOR, ExtractorName::TRAFILATURA)
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

#[test]
fn politeness_and_site_preferences_extend_collies_keys() {
    let crawl = Settings::new()
        // Collie's own Politeness & Site-preferences keys, re-exported in Foxhound's modules.
        .set(politeness::SHARING, politeness::Sharing::Shared)
        .set(politeness::DELEGATION, true)
        .set(politeness::proxy::CONCURRENCY, 4)
        .set(politeness::proxy::RATE, 10)
        .set(site_preferences::RESPECT_USE_SIGNALS, false)
        // Foxhound's own Politeness & Site-preferences keys beside them (ADRs 0086, 0118).
        .set(politeness::domain::CONCURRENCY, 2)
        .set(politeness::domain::DELAY, Duration::from_millis(500))
        .set(politeness::backoff::ENABLED, true)
        .set(politeness::backoff::INITIAL, Duration::from_secs(2))
        .set(politeness::backoff::MAX, Duration::from_secs(60))
        .set(politeness::ip::CONCURRENCY, 4)
        .set(politeness::retry_after::MAX, Duration::from_mins(15))
        .set(site_preferences::RESPECT_ROBOTS_TXT, true)
        .set(site_preferences::MAX_CRAWL_DELAY, Duration::from_secs(30))
        .set(site_preferences::ROBOTS_TXT_USER_AGENT, "operator-bot");

    let sent = crawl.within(SettingScope::Run).unwrap();

    assert_eq!(
        serde_json::Value::Object(sent),
        json!({
            "politeness": {
                "backoff": { "enabled": true, "initial": 2000, "max": 60000 },
                "delegation": true,
                "domain": { "concurrency": 2, "delay": 500 },
                "ip": { "concurrency": 4 },
                "proxy": { "concurrency": 4, "rate": 10 },
                "retry_after": { "max": 900_000 },
                "sharing": "shared",
            },
            "site_preferences": {
                "max_crawl_delay": 30000,
                "respect_robots_txt": true,
                "respect_use_signals": false,
                "robots_txt_user_agent": "operator-bot",
            },
        })
    );
}
