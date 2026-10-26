use serde_json::json;

use super::setting_keys;

#[test]
fn a_starts_settings_name_each_nested_key_with_its_prefixes() {
    let settings =
        json!({"crawl": {"max_requests": 10, "scope": {"include": ["a"]}}, "timeout": {"run": 5}});

    let mut keys = setting_keys(settings.as_object().unwrap());

    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "crawl",
            "crawl.max_requests",
            "crawl.scope",
            "crawl.scope.include",
            "timeout",
            "timeout.run",
        ]
    );
}
