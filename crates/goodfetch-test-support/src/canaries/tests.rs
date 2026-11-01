use std::collections::BTreeSet;

use rstest::rstest;
use serde::Deserialize;

use super::*;
use crate::env::TestEnv;

/// The shared fixture set the TypeScript and Python searches read too.
#[derive(Deserialize)]
struct Fixture {
    canaries: Vec<FixtureCanary>,
    outputs: Vec<FixtureOutput>,
    hits: Vec<FixtureHit>,
}

#[derive(Deserialize)]
struct FixtureCanary {
    id: String,
    kind: SecretKind,
    value: String,
    #[serde(default)]
    planted_in: Vec<String>,
}

#[derive(Deserialize)]
struct FixtureOutput {
    name: String,
    text: String,
}

#[derive(Debug, PartialEq, Eq, Deserialize)]
struct FixtureHit {
    canary: String,
    kind: SecretKind,
    encoding: Encoding,
    output: String,
    offset: usize,
    placement: Placement,
}

fn fixture() -> Fixture {
    serde_json::from_str(include_str!(
        "../../../../test-support/fixtures/canaries/search.json"
    ))
    .unwrap()
}

fn seen(hit: &Hit) -> FixtureHit {
    FixtureHit {
        canary: hit.canary.clone(),
        kind: hit.kind,
        encoding: hit.encoding,
        output: hit.output.clone(),
        offset: hit.offset,
        placement: hit.placement,
    }
}

#[test]
fn two_plants_never_return_the_same_value() {
    let mut canaries = Canaries::new(379);
    let mut values = BTreeSet::new();

    for _ in 0..40 {
        for kind in SecretKind::ALL {
            values.insert(canaries.plant(kind).value().to_owned());
        }
    }

    assert_eq!(values.len(), 40 * SecretKind::ALL.len());
}

#[test]
fn the_same_seed_plants_the_same_values() {
    let mut first = Canaries::new(7);
    let mut second = Canaries::new(7);

    let firsts: Vec<String> = SecretKind::ALL
        .iter()
        .map(|kind| first.plant(*kind).value().to_owned())
        .collect();
    let seconds: Vec<String> = SecretKind::ALL
        .iter()
        .map(|kind| second.plant(*kind).value().to_owned())
        .collect();

    assert_eq!(firsts, seconds);
}

#[rstest]
#[case::typed_password(
    SecretKind::TypedPassword,
    r#"^gfcanary-password-[0-9a-f]{16}"\\\+/=&$"#
)]
#[case::totp_seed(SecretKind::TotpSeed, r"^GFCANARYTOTP[A-Z2-7]{20}$")]
#[case::proxy_password(
    SecretKind::ProxyPassword,
    r#"^gfcanary-proxy-password-[0-9a-f]{16}"\\\+/=&$"#
)]
#[case::one_time_code(SecretKind::OneTimeCode, r"^gfcanary-code-[0-9a-f]{16}$")]
#[case::http_authentication_password(
    SecretKind::HttpAuthenticationPassword,
    r#"^gfcanary-http-password-[0-9a-f]{16}"\\\+/=&$"#
)]
#[case::header_value(
    SecretKind::HeaderValue,
    r#"^gfcanary-header-value-[0-9a-f]{16}"\\\+/=&$"#
)]
#[case::caller_answer(
    SecretKind::CallerAnswer,
    r#"^gfcanary-caller-answer-[0-9a-f]{16}"\\\+/=&$"#
)]
#[case::model_api_key(SecretKind::ModelApiKey, r"^gfcanary-model-key-[0-9a-f]{32}$")]
#[case::webhook_signing_secret(
    SecretKind::WebhookSigningSecret,
    r#"^gfcanary-webhook-secret-[0-9a-f]{16}"\\\+/=&$"#
)]
#[case::sink_credential(
    SecretKind::SinkCredential,
    r#"^gfcanary-sink-credential-[0-9a-f]{16}"\\\+/=&$"#
)]
#[case::session_cookie(SecretKind::SessionCookie, r"^gfcanary-session-[0-9a-f]{32}$")]
#[case::endpoint_token(SecretKind::EndpointToken, r"^gfcanary-token-[0-9a-f]{32}$")]
#[case::run_data_key(SecretKind::RunDataKey, r"^gfcanary-run-key-[0-9a-f]{15}$")]
fn each_kind_plants_a_value_of_its_shape(#[case] kind: SecretKind, #[case] shape: &str) {
    let mut canaries = Canaries::new(379);

    let canary = canaries.plant(kind);

    assert!(
        regex::Regex::new(shape).unwrap().is_match(canary.value()),
        "{} does not have the shape {shape}",
        canary.id()
    );
}

#[test]
fn a_run_data_key_canary_is_a_32_byte_key() {
    let mut canaries = Canaries::new(379);

    let canary = canaries.plant(SecretKind::RunDataKey);

    assert_eq!(canary.value().len(), 32);
}

#[test]
fn a_canarys_debug_form_never_shows_its_value() {
    let mut canaries = Canaries::new(379);
    let canary = canaries.plant(SecretKind::EndpointToken).clone();

    let debug = format!("{canary:?} {canaries:?}");

    assert!(!debug.contains("gfcanary"), "{debug}");
}

#[rstest]
#[case::raw("raw.log")]
#[case::json_escaped("json-escaped.log")]
#[case::url_encoded("url-encoded.log")]
#[case::url_encoded_as_a_browser_encodes_a_form("form-urlencoded.log")]
#[case::url_encoded_as_encode_uri_component_does("uri-component.log")]
#[case::base64("base64.log")]
#[case::base64url("base64url.log")]
#[case::hex("hex.log")]
#[case::uppercase_hex("upper-hex.log")]
#[case::base64_inside_a_basic_header("proxy-authorization.log")]
#[case::an_encoding_that_leaves_the_value_unchanged("session.log")]
#[case::where_it_was_planted("vault.json")]
#[case::only_a_different_canary("other-canary.log")]
fn search_finds_each_hit_of_the_shared_fixture(#[case] name: &str) {
    let fixture = fixture();
    let mut canaries = Canaries::new(379);
    for canary in &fixture.canaries {
        canaries.add(Canary::known(&canary.id, canary.kind, &canary.value));
        for output in &canary.planted_in {
            canaries.record_planted(&canary.id, output);
        }
    }
    let output = fixture
        .outputs
        .iter()
        .find(|output| output.name == name)
        .unwrap();

    let hits = canaries
        .search(&[Output::Bytes {
            name: &output.name,
            bytes: output.text.as_bytes(),
        }])
        .unwrap();

    let expected: Vec<&FixtureHit> = fixture
        .hits
        .iter()
        .filter(|hit| hit.output == name)
        .collect();
    let seen: Vec<FixtureHit> = hits.iter().map(seen).collect();
    assert_eq!(seen.iter().collect::<Vec<_>>(), expected);
}

#[test]
fn search_names_the_file_and_offset_of_a_planted_value_in_a_folder() {
    let env = TestEnv::new();
    let mut canaries = Canaries::new(379);
    let canary = canaries.plant(SecretKind::ModelApiKey).clone();
    let folder = env.folder().join("outputs");
    std::fs::create_dir_all(folder.join("logs")).unwrap();
    std::fs::write(folder.join("clean.txt"), "nothing here").unwrap();
    let file = folder.join("logs").join("engine.log");
    std::fs::write(&file, format!("key={}\n", canary.value())).unwrap();

    let hits = canaries.search(&[Output::Path(&folder)]).unwrap();

    assert_eq!(
        hits.iter().map(seen).collect::<Vec<_>>(),
        vec![FixtureHit {
            canary: canary.id().to_owned(),
            kind: SecretKind::ModelApiKey,
            encoding: Encoding::Raw,
            output: file.display().to_string(),
            offset: 4,
            placement: Placement::Leaked,
        }]
    );
}

#[test]
fn search_finds_nothing_in_output_holding_only_another_sets_canary() {
    let mut ours = Canaries::new(1);
    ours.plant(SecretKind::TypedPassword);
    let mut theirs = Canaries::new(2);
    let other = theirs.plant(SecretKind::TypedPassword).value().to_owned();

    let hits = ours
        .search(&[Output::Bytes {
            name: "theirs.log",
            bytes: other.as_bytes(),
        }])
        .unwrap();

    assert_eq!(hits, Vec::new());
}

#[test]
fn search_of_a_missing_path_fails_with_the_path() {
    let env = TestEnv::new();
    let canaries = Canaries::new(379);
    let missing = env.folder().join("missing.log");

    let error = canaries.search(&[Output::Path(&missing)]).unwrap_err();

    assert!(matches!(error, CanaryError::Read { path, .. } if path == missing));
}

#[test]
fn leaks_leave_out_planted_copies() {
    let mut canaries = Canaries::new(379);
    let id = canaries.plant(SecretKind::SinkCredential).id().to_owned();
    canaries.record_planted(&id, "secrets.toml");
    let value = canaries.canary(&id).unwrap().value().to_owned();

    let leaks = canaries
        .leaks(&[
            Output::Bytes {
                name: "secrets.toml",
                bytes: value.as_bytes(),
            },
            Output::Bytes {
                name: "export.csv",
                bytes: value.as_bytes(),
            },
        ])
        .unwrap();

    assert_eq!(
        leaks
            .iter()
            .map(|hit| hit.output.as_str())
            .collect::<Vec<_>>(),
        vec!["export.csv"]
    );
}

/// Spec #564's short-value rule (ADR 0067): a short canary has seven
/// characters, the length under which the engine masks a value only where
/// it was typed and in what it keeps.
#[rstest]
#[case::typed_password(SecretKind::TypedPassword, r#"^[a-km-z]{4}"\\\+$"#)]
#[case::one_time_code(SecretKind::OneTimeCode, r"^[a-km-z]{7}$")]
fn a_short_canary_has_seven_characters_of_its_kinds_alphabet(
    #[case] kind: SecretKind,
    #[case] shape: &str,
) {
    let mut canaries = Canaries::new(379);

    let canary = canaries.plant_short(kind);

    assert!(
        regex::Regex::new(shape).unwrap().is_match(canary.value()),
        "{} does not have the shape {shape}",
        canary.id()
    );
}

/// A page read the test captured is searched for every canary but a short
/// one, which a read may hold by chance and the engine leaves in it.
#[test]
fn a_read_is_searched_for_every_canary_but_a_short_one() {
    let mut canaries = Canaries::new(379);
    let long = canaries.plant(SecretKind::EndpointToken).value().to_owned();
    let short = canaries
        .plant_short(SecretKind::TypedPassword)
        .value()
        .to_owned();
    let text = format!("{long} {short}");

    let leaks: Vec<(String, String)> = canaries
        .leaks(&[
            Output::Read {
                name: "page read",
                bytes: text.as_bytes(),
            },
            Output::Bytes {
                name: "run state",
                bytes: text.as_bytes(),
            },
        ])
        .unwrap()
        .into_iter()
        .filter(|hit| hit.encoding == Encoding::Raw)
        .map(|hit| (hit.canary, hit.output))
        .collect();

    assert_eq!(
        leaks,
        vec![
            ("canary-1".to_owned(), "page read".to_owned()),
            ("canary-1".to_owned(), "run state".to_owned()),
            ("canary-2".to_owned(), "run state".to_owned()),
        ]
    );
}
