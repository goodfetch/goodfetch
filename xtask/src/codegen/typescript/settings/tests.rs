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
