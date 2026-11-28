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

/// Parses every `Setting::new(...)` / `Setting::deprecated(...)` declaration
/// from a generated Rust `settings.rs` file into `(wire_key -> sorted scope names)`.
fn parse_generated_rust_settings(source: &str) -> std::collections::BTreeMap<String, Vec<String>> {
    let mut found = std::collections::BTreeMap::new();
    let mut rest = source;
    while let Some(pos) = rest.find("Setting::") {
        rest = &rest[pos + "Setting::".len()..];
        let Some(after) = rest
            .strip_prefix("new(")
            .or_else(|| rest.strip_prefix("deprecated("))
        else {
            continue;
        };
        rest = after;
        let Some(first_quote) = rest.find('"') else {
            break;
        };
        rest = &rest[first_quote + 1..];
        let Some(end_quote) = rest.find('"') else {
            break;
        };
        let key = rest[..end_quote].to_owned();
        rest = &rest[end_quote + 1..];
        let Some(bracket_start) = rest.find("&[") else {
            break;
        };
        rest = &rest[bracket_start + 2..];
        let Some(bracket_end) = rest.find(']') else {
            break;
        };
        let mut scopes: Vec<String> = rest[..bracket_end]
            .split("SettingScope::")
            .skip(1)
            .map(|part| {
                part.chars()
                    .take_while(char::is_ascii_alphanumeric)
                    .collect::<String>()
                    .to_ascii_lowercase()
            })
            .collect();
        scopes.sort();
        found.insert(key, scopes);
        rest = &rest[bracket_end + 1..];
    }
    found
}

/// Completeness test (spec 611, issue 1500; ADR 0083): every Foxhound setting
/// key of the schema (`foxhound.json`) is present in Rust's generated settings
/// with its exact allowed scopes, configuration-only keys are omitted, and
/// Collie's Politeness and Site-preferences keys stay in `goodfetch-collie-client`.
#[test]
fn every_foxhound_setting_of_the_schema_is_present_with_its_allowed_scopes() {
    let catalog: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../goodfetch-protocol/generated/foxhound.json"
    ))
    .expect("foxhound.json parses");
    let foxhound_rust = parse_generated_rust_settings(include_str!("../generated/settings.rs"));
    let collie_rust = parse_generated_rust_settings(include_str!(
        "../../../../goodfetch-collie-client/src/generated/settings.rs"
    ));

    let mut expected_foxhound = std::collections::BTreeMap::new();
    let mut expected_collie = std::collections::BTreeMap::new();
    for entry in catalog["settings"].as_array().expect("settings array") {
        let name = entry["name"].as_str().expect("setting name").to_owned();
        let product = entry["product"].as_str().expect("setting product");
        let mut scopes: Vec<String> = entry["scopes"]
            .as_array()
            .expect("setting scopes")
            .iter()
            .map(|scope| scope.as_str().expect("scope str").to_owned())
            .collect();
        scopes.sort();
        if scopes.is_empty() {
            assert!(
                !foxhound_rust.contains_key(&name) && !collie_rust.contains_key(&name),
                "configuration-only setting `{name}` must not appear in SDK Settings"
            );
            continue;
        }
        match product {
            "foxhound" => {
                expected_foxhound.insert(name, scopes);
            }
            "collie" => {
                expected_collie.insert(name, scopes);
            }
            other => panic!("unexpected product `{other}` for setting `{name}`"),
        }
    }

    assert_eq!(foxhound_rust, expected_foxhound);
    assert_eq!(collie_rust, expected_collie);
    for key in [
        "politeness.proxy.concurrency",
        "politeness.proxy.rate",
        "politeness.sharing",
        "site_preferences.respect_use_signals",
    ] {
        assert!(
            collie_rust.contains_key(key),
            "Collie's `{key}` must stay in goodfetch-collie-client"
        );
    }
    assert!(!foxhound_rust.contains_key("politeness.degraded_max"));
    assert!(!collie_rust.contains_key("politeness.degraded_max"));
}
