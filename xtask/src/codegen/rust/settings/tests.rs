use goodfetch_protocol::catalog::ValueKind;
use serde_json::{Value, json};

use super::module;
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

/// The declaration of the constant `name` in `module`, without its doc.
fn constant<'a>(module: &'a str, name: &str) -> &'a str {
    module
        .lines()
        .find(|line| line.starts_with(&format!("pub const {name}: ")))
        .unwrap_or_else(|| panic!("no constant {name} in:\n{module}"))
}

#[test]
fn each_setting_is_a_constant_in_modules_mirroring_its_key_with_its_scopes() {
    let settings = [
        setting(
            "timeout.navigation",
            ValueKind::Duration.schema(),
            false,
            &["connection", "run", "session", "page"],
        ),
        setting(
            "timeout.run",
            ValueKind::Duration.schema(),
            true,
            &["connection"],
        ),
        setting("tls.verify", json!({ "type": "boolean" }), false, &[]),
    ];

    let text = module("// header\n", &settings).unwrap();

    assert_eq!(
        (
            text.starts_with("// header\n"),
            text.contains("pub mod timeout {"),
            text.contains("pub mod tls"),
            text.contains("/// The `timeout.*` settings.\n"),
            text.contains(
                "/// What `timeout.run` shapes.\n///\n/// `timeout.run`. `None` clears it.\n"
            ),
        ),
        (true, true, false, true, true)
    );
    assert_eq!(
        constant(&text, "NAVIGATION"),
        "pub const NAVIGATION: crate::settings::Setting<::std::time::Duration> = crate::settings::Setting::new(\"timeout.navigation\", &[::goodfetch_protocol::catalog::SettingScope::Connection, ::goodfetch_protocol::catalog::SettingScope::Run, ::goodfetch_protocol::catalog::SettingScope::Session, ::goodfetch_protocol::catalog::SettingScope::Page]);"
    );
    assert_eq!(
        constant(&text, "RUN"),
        "pub const RUN: crate::settings::Setting<Option<::std::time::Duration>> = crate::settings::Setting::new(\"timeout.run\", &[::goodfetch_protocol::catalog::SettingScope::Connection]);"
    );
}

#[rstest::rstest]
#[case::boolean(json!({ "type": "boolean" }), false, "bool")]
#[case::count(json!({ "type": "integer", "minimum": 0 }), false, "u64")]
#[case::text(json!({ "type": "string" }), false, "String")]
#[case::clearable_text(json!({ "type": "string" }), true, "Option<String>")]
#[case::nullable_type_list(json!({ "type": ["integer", "null"] }), false, "Option<u64>")]
#[case::nullable_alternative(
    json!({ "anyOf": [{ "type": "string" }, { "type": "null" }] }),
    true,
    "Option<String>"
)]
#[case::list(json!({ "type": "array", "items": { "type": "string" } }), false, "Vec<String>")]
#[case::map(
    json!({ "type": "object", "additionalProperties": { "type": "boolean" } }),
    false,
    "::std::collections::BTreeMap<String, bool>"
)]
#[case::named_properties(
    json!({ "type": "object", "properties": { "a": { "type": "string" } } }),
    false,
    "::serde_json::Value"
)]
#[case::other_union(
    json!({ "anyOf": [{ "type": "string" }, { "type": "integer" }] }),
    false,
    "::serde_json::Value"
)]
fn a_value_schema_gives_the_constants_type(
    #[case] value: Value,
    #[case] clearable: bool,
    #[case] ty: &str,
) {
    let text = module("", &[setting("area.key", value, clearable, &["run"])]).unwrap();

    assert!(
        constant(&text, "KEY")
            .starts_with(&format!("pub const KEY: crate::settings::Setting<{ty}> = ")),
        "{text}"
    );
}

#[test]
fn a_choice_of_words_is_an_enum_of_the_module_that_sends_each_word() {
    let settings = [
        setting(
            "content.extractor",
            json!({ "type": "string", "enum": ["dom_smoothie", "trafilatura"] }),
            false,
            &["run", "page"],
        ),
        setting(
            "content.kinds",
            json!({ "type": "array", "items": { "enum": ["ai-input", "html"] } }),
            false,
            &["run"],
        ),
    ];

    let text = module("", &settings).unwrap();

    assert_eq!(
        (
            constant(&text, "EXTRACTOR").contains("Setting<Extractor>"),
            constant(&text, "KINDS").contains("Setting<Vec<KindsItem>>"),
            text.contains("pub enum Extractor {\n/// `dom_smoothie`.\nDomSmoothie,\n/// `trafilatura`.\nTrafilatura,\n}"),
            text.contains("Self::DomSmoothie => \"dom_smoothie\","),
            text.contains("pub enum KindsItem {\n/// `ai-input`.\nAiInput,\n"),
            text.contains("Self::AiInput => \"ai-input\","),
            text.contains("impl crate::settings::SettingValue for KindsItem {}"),
        ),
        (true, true, true, true, true, true, true)
    );
}

#[test]
fn two_words_of_one_variant_fail() {
    let settings = [setting(
        "content.mode",
        json!({ "enum": ["a-b", "a_b"] }),
        false,
        &["run"],
    )];

    let refused = module("", &settings).unwrap_err();

    assert!(
        format!("{refused:#}").contains("both the variant `AB`"),
        "{refused:#}"
    );
}

#[test]
fn a_choice_named_like_a_type_of_rusts_prelude_takes_choice_after_its_name() {
    let settings = [setting(
        "run_state.batch.sync",
        json!({ "enum": ["full", "normal"] }),
        false,
        &["run"],
    )];

    let text = module("", &settings).unwrap();

    assert_eq!(
        (
            constant(&text, "SYNC").contains("Setting<SyncChoice>"),
            text.contains("pub enum SyncChoice {"),
        ),
        (true, true)
    );
}
