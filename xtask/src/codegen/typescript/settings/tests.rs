use serde_json::{Value, json};

use super::{foxhound_settings_module, scope_type, settings_module};
use crate::codegen::layer::Setting;

fn setting(name: &str, value: Value, clearable: bool, scopes: &[&str]) -> Setting {
    Setting {
        name: name.to_owned(),
        description: format!("What `{name}` shapes."),
        value,
        default: Value::Null,
        clearable,
        scopes: scopes.iter().map(|scope| (*scope).to_owned()).collect(),
    }
}

/// The text of the interface `name` in `module`.
fn interface<'a>(module: &'a str, name: &str) -> &'a str {
    let start = module
        .find(&format!("export interface {name} {{"))
        .unwrap_or_else(|| panic!("no interface {name} in:\n{module}"));
    let end = module[start..].find("\n}\n").unwrap() + start;
    &module[start..end]
}

#[test]
fn each_scope_types_the_settings_it_sets_by_their_camel_case_segments() {
    let settings = [
        setting(
            "timeout.navigation",
            json!({ "type": "integer", "minimum": 0 }),
            false,
            &["connection", "run"],
        ),
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

    let page = interface(&module, "PageSettings");
    assert!(
        page.contains("readonly extractor?: \"dom_smoothie\" | \"trafilatura\";"),
        "{page}"
    );
    assert!(
        !page.contains("proxy") && !page.contains("timeout"),
        "{page}"
    );
    let session = interface(&module, "SessionSettings");
    assert!(
        session.contains("readonly baseURL?: string | null;"),
        "{session}"
    );
    assert!(!session.contains("timeout"), "{session}");
    let run = interface(&module, "RunSettings");
    assert!(run.contains("readonly navigation?: number;"), "{run}");
    assert!(
        !module.contains("trustedCas") && !module.contains("trusted_cas"),
        "{module}"
    );
    assert!(
        module.contains("proxy: [\"proxy\", {\n    baseURL: \"base_url\",\n  }],"),
        "{module}"
    );
}

#[test]
fn a_value_schema_types_its_objects_lists_and_alternatives() {
    let settings = [setting(
        "proxy.providers",
        json!({
            "type": "object",
            "additionalProperties": {
                "type": "object",
                "required": ["url"],
                "properties": {
                    "url": { "type": "string", "format": "uri" },
                    "kinds": { "type": "array", "items": { "enum": ["a", "b"] } },
                    "price": { "oneOf": [{ "type": "integer" }, { "const": "free" }] },
                },
                "additionalProperties": false,
            },
        }),
        false,
        &["connection", "run"],
    )];

    let module = settings_module(&settings).unwrap();

    assert!(
        module.contains("readonly providers?: { readonly [key: string]: { readonly kinds?: readonly (\"a\" | \"b\")[]; readonly price?: number | \"free\"; readonly url: string } };"),
        "{module}"
    );
}

#[test]
fn a_value_schema_keyword_without_a_typescript_form_is_refused() {
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
fn only_a_scope_that_takes_settings_has_a_settings_type() {
    assert_eq!(scope_type("session").unwrap(), "SessionSettings");
    assert!(scope_type("locator").is_err());
}

#[test]
fn a_crawl_types_collie_and_foxhound_run_settings_and_a_label_only_its_own() {
    let settings = [
        setting(
            "fetch.tier",
            json!({ "enum": ["auto", "http", "browser"] }),
            false,
            &["connection", "run", "label"],
        ),
        setting(
            "politeness.domain.concurrency",
            json!({ "type": "integer", "minimum": 1 }),
            false,
            &["connection", "run"],
        ),
        setting(
            "content.extractor",
            json!({ "enum": ["dom_smoothie", "trafilatura"] }),
            false,
            &["connection", "run", "label", "session", "page"],
        ),
        setting(
            "proxy.base_url",
            json!({ "type": "string" }),
            true,
            &["connection", "run", "session"],
        ),
    ];

    let module = foxhound_settings_module(&settings).unwrap();

    let crawl = interface(&module, "CrawlSettings");
    for typed in [
        "readonly tier?:",
        "readonly concurrency?: number;",
        "readonly extractor?:",
        "readonly baseURL?: string | null;",
    ] {
        assert!(crawl.contains(typed), "{typed} in {crawl}");
    }
    let label = interface(&module, "LabelSettings");
    assert!(
        label.contains("readonly tier?: \"auto\" | \"http\" | \"browser\";")
            && label.contains("readonly extractor?:"),
        "{label}"
    );
    assert!(
        !label.contains("politeness") && !label.contains("proxy"),
        "{label}"
    );
    assert!(
        module.contains("from \"@goodfetch/collie-client/runtime\";"),
        "{module}"
    );
    assert!(!module.contains("interface RunSettings"), "{module}");
}

#[test]
fn foxhounds_connect_types_its_connection_settings_and_sends_every_scopes_by_them() {
    let settings = [
        setting(
            "timeout.run",
            json!({ "type": "integer", "minimum": 1 }),
            true,
            &["connection"],
        ),
        setting(
            "crawl.max_depth",
            json!({ "type": "integer", "minimum": 0 }),
            false,
            &["connection", "run"],
        ),
    ];

    let module = foxhound_settings_module(&settings).unwrap();

    let connection = interface(&module, "FoxhoundSettings");
    let crawl = interface(&module, "CrawlSettings");
    assert!(
        connection.contains("readonly run?: number | null;")
            && connection.contains("readonly maxDepth?: number;"),
        "{connection}"
    );
    assert!(!crawl.contains("readonly run?:"), "{crawl}");
    assert!(
        module.contains("settingsOnWire(settings: FoxhoundSettings | undefined)"),
        "{module}"
    );
}

#[test]
fn a_value_names_its_declared_fields_in_camel_case_and_the_spelling_sends_them_by_the_wire_names() {
    let settings = [
        setting(
            "session.page_check",
            json!({
                "type": "object",
                "properties": {
                    "login_page": { "type": ["string", "null"] },
                    "headers": {
                        "type": "object",
                        "additionalProperties": { "type": "string" },
                    },
                },
            }),
            true,
            &["run"],
        ),
        setting("session.name", json!({ "type": "string" }), false, &["run"]),
    ];

    let module = settings_module(&settings).unwrap();

    assert!(
        module.contains("readonly pageCheck?: { readonly headers?: { readonly [key: string]: string }; readonly loginPage?: string | null } | null;"),
        "{module}"
    );
    assert!(
        module.contains(
            "pageCheck: { setting: \"page_check\", value: { fields: { \"login_page\": \"loginPage\" } } },"
        ),
        "{module}"
    );
    assert!(module.contains("name: \"name\","), "{module}");
}

#[test]
fn two_fields_of_a_value_that_spell_one_name_are_refused() {
    let settings = [setting(
        "session.page_check",
        json!({
            "type": "object",
            "properties": {
                "login_page": { "type": "string" },
                "loginPage": { "type": "string" },
            },
        }),
        false,
        &["run"],
    )];

    let error = settings_module(&settings).unwrap_err();

    assert!(format!("{error:#}").contains("loginPage"), "{error:#}");
}
