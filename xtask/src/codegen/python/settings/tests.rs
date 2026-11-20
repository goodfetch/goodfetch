use goodfetch_protocol::catalog::ValueKind;
use serde_json::{Value, json};

use super::{scope_type, settings_module};
use crate::codegen::layer::Setting;

fn setting(name: &str, value: Value, clearable: bool, scopes: &[&str]) -> Setting {
    Setting {
        name: name.to_owned(),
        description: format!("What `{name}` shapes."),
        value,
        default: Value::Null,
        clearable,
        scopes: scopes.iter().map(|scope| (*scope).to_owned()).collect(),
        deprecated: None,
    }
}

/// The text of the class `name` in `module`, its last line's end included.
fn class<'a>(module: &'a str, name: &str) -> &'a str {
    let start = module
        .find(&format!("\nclass {name}(TypedDict):\n"))
        .unwrap_or_else(|| panic!("no class {name} in:\n{module}"))
        + 1;
    let end = module[start..]
        .find("\n\n\n")
        .map_or(module.len(), |end| end + start + 1);
    &module[start..end]
}

#[test]
fn each_scope_types_the_settings_it_sets_by_their_snake_case_segments() {
    let settings = [
        setting(
            "content.extractor",
            json!({ "enum": ["dom_smoothie", "trafilatura"] }),
            false,
            &["connection", "run", "session", "page"],
        ),
        setting(
            "proxy.base_url",
            json!({ "type": "string", "pattern": "^x" }),
            true,
            &["connection", "run", "session"],
        ),
        setting(
            "tls.trusted_cas",
            json!({ "type": "array", "items": { "type": "string" } }),
            false,
            &[],
        ),
    ];

    let module = settings_module(&settings).unwrap();

    let page = class(&module, "PageSettings");
    assert!(
        page.contains("    content: NotRequired[PageContentSettings]\n"),
        "{page}"
    );
    assert!(!page.contains("proxy"), "{page}");
    assert!(
        class(&module, "PageContentSettings")
            .contains("    extractor: NotRequired[Literal[\"dom_smoothie\", \"trafilatura\"]]\n"),
        "{module}"
    );
    assert!(
        class(&module, "SessionProxySettings")
            .contains("    base_url: NotRequired[str | None]\n    \"\"\"What `proxy.base_url` shapes.\n\n    `proxy.base_url`.\n    \"\"\"\n"),
        "{module}"
    );
    assert!(
        module.find("class SessionProxySettings").unwrap()
            < module.find("class SessionSettings").unwrap(),
        "an area's class comes before the class that names it:\n{module}"
    );
    assert!(
        class(&module, "Settings").contains("    proxy: NotRequired[ConnectionProxySettings]\n")
    );
    assert!(!module.contains("trusted_cas"), "{module}");
}

#[test]
fn a_duration_is_a_timedelta_and_none_only_clears_a_setting_that_can_be_cleared() {
    let settings = [
        setting(
            "timeout.navigation",
            ValueKind::Duration.schema(),
            false,
            &["connection", "run"],
        ),
        setting(
            "checkpoint.interval",
            ValueKind::Duration.schema(),
            true,
            &["run"],
        ),
        setting(
            "crawl.max_depth",
            ValueKind::Count.schema(),
            false,
            &["run"],
        ),
    ];

    let module = settings_module(&settings).unwrap();

    assert!(
        module.contains("\nfrom datetime import timedelta\n"),
        "{module}"
    );
    assert!(
        class(&module, "RunTimeoutSettings").contains("    navigation: NotRequired[timedelta]\n"),
        "{module}"
    );
    assert!(
        class(&module, "RunCheckpointSettings")
            .contains("    interval: NotRequired[timedelta | None]\n"),
        "{module}"
    );
    assert!(
        class(&module, "RunCrawlSettings").contains("    max_depth: NotRequired[int]\n"),
        "{module}"
    );
}

#[test]
fn a_value_schema_types_its_objects_and_tagged_alternatives_once_each() {
    let matched = json!({
        "anyOf": [
            {
                "oneOf": [
                    {
                        "type": "object",
                        "description": "A glob.",
                        "properties": {
                            "kind": { "const": "glob", "type": "string" },
                            "pattern": { "type": "string", "description": "The glob." },
                        },
                        "required": ["kind", "pattern"],
                        "additionalProperties": false,
                    },
                    {
                        "type": "object",
                        "properties": {
                            "kind": { "const": "css", "type": "string" },
                            "selector": { "type": "string" },
                            "domains": { "type": ["array", "null"], "items": { "type": "string" } },
                        },
                        "required": ["kind", "selector"],
                        "additionalProperties": false,
                    },
                ],
            },
            { "type": "null" },
        ],
    });
    let settings = [setting(
        "session.page_check",
        json!({
            "type": "object",
            "properties": { "login_page": matched, "expected_page": matched },
            "additionalProperties": false,
        }),
        true,
        &["run"],
    )];

    let module = settings_module(&settings).unwrap();

    assert!(
        class(&module, "RunSessionSettings")
            .contains("    page_check: NotRequired[SessionPageCheck | None]\n"),
        "{module}"
    );
    let page_check = class(&module, "SessionPageCheck");
    assert!(
        page_check.contains(
            "    login_page: NotRequired[SessionPageCheckGlob | SessionPageCheckCSS | None]\n"
        ) && page_check.contains(
            "    expected_page: NotRequired[SessionPageCheckGlob | SessionPageCheckCSS | None]\n"
        ),
        "{page_check}"
    );
    assert_eq!(
        module.matches("class SessionPageCheckGlob(").count(),
        1,
        "{module}"
    );
    let glob = class(&module, "SessionPageCheckGlob");
    assert!(
        glob.contains("    \"\"\"A glob.\"\"\"\n")
            && glob.contains("    kind: Literal[\"glob\"]\n")
            && glob.contains("    pattern: str\n    \"\"\"The glob.\"\"\"\n"),
        "{glob}"
    );
    assert!(
        class(&module, "SessionPageCheckCSS")
            .contains("    domains: NotRequired[list[str] | None]\n"),
        "{module}"
    );
}

#[test]
fn a_value_schema_types_its_maps_and_untagged_alternatives() {
    let settings = [
        setting(
            "tls.client_certificates",
            json!({
                "type": "object",
                "additionalProperties": {
                    "type": "object",
                    "properties": { "key": { "type": "string" } },
                    "required": ["key"],
                    "additionalProperties": false,
                },
            }),
            false,
            &["run"],
        ),
        setting(
            "run_state.batch.pages",
            json!({ "oneOf": [{ "type": "integer", "minimum": 1 }, { "const": "none" }] }),
            false,
            &["run"],
        ),
        setting(
            "network.resolve",
            json!({
                "type": "object",
                "additionalProperties": { "type": "array", "items": { "type": "string", "format": "ip" } },
            }),
            false,
            &["run"],
        ),
    ];

    let module = settings_module(&settings).unwrap();

    assert!(
        class(&module, "RunTlsSettings").contains(
            "    client_certificates: NotRequired[Mapping[str, TlsClientCertificatesEntry]]\n"
        ),
        "{module}"
    );
    assert!(
        class(&module, "RunRunStateBatchSettings")
            .contains("    pages: NotRequired[int | Literal[\"none\"]]\n"),
        "{module}"
    );
    assert!(
        class(&module, "RunNetworkSettings")
            .contains("    resolve: NotRequired[Mapping[str, list[str]]]\n"),
        "{module}"
    );
    assert!(
        module.contains("\nfrom collections.abc import Mapping\n"),
        "{module}"
    );
}

#[test]
fn a_value_schema_keyword_without_a_python_form_is_refused() {
    let settings = [setting(
        "x.y",
        json!({ "type": "string", "$ref": "#/$defs/Y" }),
        false,
        &["run"],
    )];

    let error = settings_module(&settings).unwrap_err();

    assert!(format!("{error:#}").contains("`$ref`"), "{error:#}");
}

#[test]
fn a_segment_a_typed_dict_cannot_declare_is_refused() {
    let settings = [setting(
        "proxy.from",
        json!({ "type": "string" }),
        false,
        &["run"],
    )];

    let error = settings_module(&settings).unwrap_err();

    assert!(format!("{error:#}").contains("`proxy.from`"), "{error:#}");
}

#[test]
fn a_scope_that_sets_nothing_has_an_empty_settings_type() {
    let module = settings_module(&[]).unwrap();

    assert!(
        class(&module, "PageSettings").ends_with("(ADR 0083).\n    \"\"\"\n"),
        "{module}"
    );
    assert!(
        module.contains(
            "\nfrom collections.abc import Mapping\nfrom typing import Final, TypedDict\n"
        ),
        "{module}"
    );
    assert!(!module.contains("from datetime"), "{module}");
}

#[test]
fn only_a_scope_that_takes_settings_has_a_settings_type() {
    assert_eq!(scope_type("session").unwrap(), "SessionSettings");
    assert!(scope_type("locator").is_err());
}

#[test]
fn a_deprecated_setting_carries_its_deprecation_in_its_docstring_and_the_registry() {
    let mut old = setting(
        "timeout.wait",
        json!({ "type": "integer" }),
        false,
        &["run"],
    );
    old.deprecated = Some(goodfetch_protocol::catalog::Deprecation {
        since: goodfetch_protocol::catalog::ProtocolVersion { major: 0, minor: 1 },
        replacement: Some("timeout.navigation".to_owned()),
        earliest_removal: goodfetch_protocol::catalog::ProtocolVersion { major: 1, minor: 0 },
        note: "Gone.".to_owned(),
    });
    let text = "Since 0.1, removed no earlier than 1.0: use `timeout.navigation` instead. Gone.";

    let module = settings_module(&[old]).unwrap();

    assert!(
        class(&module, "RunTimeoutSettings").contains(&format!(
            "    `timeout.wait`.\n\n    Deprecated. {text}\n    \"\"\""
        )),
        "{module}"
    );
    assert!(
        module.ends_with(&format!(
            "\n\nDEPRECATED_SETTINGS: Final[Mapping[str, str]] = {{\n    \"timeout.wait\": {text:?},\n}}\n\"\"\"Each deprecated setting, by its dotted wire key, with its deprecation's one text.\"\"\"\n"
        )),
        "{module}"
    );
}
