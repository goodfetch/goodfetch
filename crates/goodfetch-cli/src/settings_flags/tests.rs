use clap::{Args as _, Command, FromArgMatches as _};
use goodfetch_test_support::TestEnv;
use rstest::rstest;
use serde_json::{Value, json};

use super::SettingsFlags;

/// The flags a command line gives, or clap's usage error.
fn parse(arguments: &[&str]) -> Result<SettingsFlags, clap::Error> {
    let matches = SettingsFlags::augment_args(Command::new("command"))
        .try_get_matches_from(std::iter::once("command").chain(arguments.iter().copied()))?;
    SettingsFlags::from_arg_matches(&matches)
}

/// The run layer of one `--set`.
fn set(flag: &str) -> Value {
    Value::Object(parse(&["--set", flag]).unwrap().run_layer())
}

/// The Settings case table (ADR 0083): a value is JSON when it parses as
/// JSON, text otherwise.
///
/// Covers: story:11-203
#[rstest]
#[case::text_that_is_no_json("ai.model=openai/gpt-5", json!({ "ai.model": "openai/gpt-5" }))]
#[case::a_number("timeout.navigation=45000", json!({ "timeout.navigation": 45000 }))]
#[case::a_json_object(
    r#"network.resolve={"staging.example":["127.0.0.1"]}"#,
    json!({ "network.resolve": { "staging.example": ["127.0.0.1"] } })
)]
#[case::null_clears("timeout.run=null", json!({ "timeout.run": null }))]
#[case::a_quoted_json_string(r#"ai.model="7""#, json!({ "ai.model": "7" }))]
#[case::text_holding_an_equals_sign("ai.model=a=b", json!({ "ai.model": "a=b" }))]
fn set_reads_its_value_as_json_when_it_parses_and_as_text_otherwise(
    #[case] flag: &str,
    #[case] expected: Value,
) {
    assert_eq!(set(flag), expected);
}

/// A duration setting takes milliseconds or a suffixed duration (ADR 0080's
/// milliseconds on the wire).
///
/// Covers: story:11-203
#[rstest]
#[case::milliseconds("45000", 45_000)]
#[case::suffixed_milliseconds("250ms", 250)]
#[case::seconds("45s", 45_000)]
#[case::minutes("2m", 120_000)]
#[case::hours("1h", 3_600_000)]
#[case::zero("0s", 0)]
fn a_duration_is_milliseconds_or_a_suffixed_duration(
    #[case] duration: &str,
    #[case] milliseconds: u64,
) {
    assert_eq!(
        set(&format!("timeout.navigation={duration}")),
        json!({ "timeout.navigation": milliseconds })
    );
}

/// Covers: story:11-203
#[rstest]
#[case::an_unknown_unit("45x")]
#[case::a_fraction("1.5s")]
#[case::a_negative_number("-5")]
#[case::no_number("s")]
#[case::nothing("")]
#[case::a_boolean("true")]
#[case::past_the_largest_duration("18446744073709551615h")]
fn a_bad_duration_is_a_usage_error(#[case] duration: &str) {
    let refused = parse(&["--set", &format!("timeout.navigation={duration}")]).unwrap_err();

    assert_eq!(
        (refused.kind(), refused.exit_code()),
        (clap::error::ErrorKind::ValueValidation, 2),
        "{refused}"
    );
}

/// Only a duration setting reads a suffix: other keys keep their text, and
/// an unknown key is the engine's to refuse.
///
/// Covers: story:11-203
#[rstest]
#[case::a_text_setting("ai.model=45s", json!({ "ai.model": "45s" }))]
#[case::an_unknown_key("timeout.navigation_=45s", json!({ "timeout.navigation_": "45s" }))]
fn only_a_duration_setting_reads_a_suffix(#[case] flag: &str, #[case] expected: Value) {
    assert_eq!(set(flag), expected);
}

/// The run layer of `--settings` naming a file that holds `text`, then
/// `more` arguments.
fn from_file(text: &str, more: &[&str]) -> Result<Value, clap::Error> {
    let env = TestEnv::new();
    let path = env.folder().join("settings.json");
    #[expect(
        clippy::disallowed_methods,
        reason = "the test writes the file the flag names"
    )]
    std::fs::write(&path, text).unwrap();
    let path = path.to_str().unwrap();
    let mut arguments = vec!["--settings", path];
    arguments.extend_from_slice(more);
    parse(&arguments).map(|flags| Value::Object(flags.run_layer()))
}

/// `--settings` takes a JSON object nested by key, as `settings` is
/// everywhere (ADR 0083), its durations read as `--set` reads them, and a
/// `--set` of the same key beats it.
///
/// Covers: story:11-203
#[test]
fn settings_reads_a_json_object_from_a_file_and_set_beats_it() {
    let run_layer = from_file(
        r#"{ "timeout": { "navigation": "45s", "first_byte": 1000 }, "ai.model": "openai/gpt-5" }"#,
        &["--set", "timeout.first_byte=2s"],
    )
    .unwrap();

    assert_eq!(
        run_layer,
        json!({
            "ai.model": "openai/gpt-5",
            "timeout.first_byte": 2000,
            "timeout.navigation": 45000,
        })
    );
}

/// Covers: story:11-203
#[rstest]
#[case::no_object("[1, 2]")]
#[case::no_json("timeout.navigation = 45000")]
#[case::a_bad_duration(r#"{ "timeout": { "navigation": "45x" } }"#)]
fn settings_that_are_no_json_object_are_a_usage_error(#[case] text: &str) {
    let refused = from_file(text, &[]).unwrap_err();

    assert_eq!(
        (refused.kind(), refused.exit_code()),
        (clap::error::ErrorKind::ValueValidation, 2),
        "{refused}"
    );
}

/// Covers: story:11-203
#[test]
fn settings_naming_no_file_is_a_usage_error() {
    let env = TestEnv::new();
    let missing = env.folder().join("missing.json");

    let refused = parse(&["--settings", missing.to_str().unwrap()]).unwrap_err();

    assert_eq!(
        (refused.kind(), refused.exit_code()),
        (clap::error::ErrorKind::ValueValidation, 2),
        "{refused}"
    );
}

proptest::proptest! {
    #[test]
    fn every_suffix_scales_its_number_to_milliseconds(
        number in 0_u64..1_000_000,
        (unit, scale) in proptest::sample::select(vec![("ms", 1_u64), ("s", 1_000), ("m", 60_000), ("h", 3_600_000)]),
    ) {
        proptest::prop_assert_eq!(
            set(&format!("timeout.first_byte={number}{unit}")),
            json!({ "timeout.first_byte": number * scale })
        );
    }
}
