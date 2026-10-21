//! The settings files' versions (ADRs 0030, 0066 and 0098): each generated
//! refusal case of the Operator policy and user configuration through its
//! reader, the lowest minor a newer engine raises, and an Operator policy
//! found only for an older major. Versions are fabricated: no build carries
//! a version override (ADR 0059), so each engine here is a Product version
//! the reader is given.

use goodfetch_protocol::catalog::{Product, ProtocolVersion};
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::formats::{Compatibility, USER_CONFIGURATION};
use goodfetch_protocol::refusals::RefusalCase;
use goodfetch_protocol::settings::SettingsLayer;
use rstest::rstest;
use serde_json::json;

use super::engine;
use crate::home::Home;
use crate::settings::{SettingsError, SettingsFiles, operator_policy, set, user_configuration};

fn version(text: &str) -> ProductVersion {
    ProductVersion::parse(text).unwrap()
}

/// What a reader makes of `text`: `Ok` when it reads it, its Error code and
/// message otherwise.
fn read_as(format: &str, text: &str, engine: &ProductVersion) -> Result<(), String> {
    let read = match format {
        "operator-policy" => operator_policy(text, Product::Collie, engine).map(|_| ()),
        "user-configuration" => user_configuration(
            SettingsLayer::UserConfiguration,
            text,
            Product::Collie,
            engine,
        )
        .map(|_| ()),
        other => panic!("`{other}` is no settings file"),
    };
    read.map_err(|error| format!("{}: {error}", error.code()))
}

/// Each generated refusal case of the shared settings files, a fabricated
/// `version` and `lowest_minor` read by the engine the case names, or this
/// one: the rule's verdict, with the Error code and message snapshotted
/// (ADR 0066).
///
/// Covers: story:3-95, story:3-144, format:operator-policy, format:user-configuration, error:format_version_unsupported
#[rstest]
fn each_generated_refusal_of_a_settings_file_gets_its_verdict(
    #[files("../goodfetch-protocol/generated/refusals/operator-policy/*.json")]
    #[files("../goodfetch-protocol/generated/refusals/user-configuration/*.json")]
    #[mode = str]
    case: &str,
) {
    let case: RefusalCase = serde_json::from_str(case).unwrap();
    let engine = case.engine.map_or_else(engine, |engine| {
        version(&format!("{}.{}.0", engine.major, engine.minor))
    });
    let text = format!(
        "version = \"{}\"\nlowest_minor = {}\n",
        case.found,
        case.lowest_minor.unwrap_or(0)
    );

    let read = read_as(&case.format, &text, &engine);

    match case.compatibility {
        Compatibility::Compatible => assert_eq!(read, Ok(()), "{case:#?}"),
        _ => insta::assert_snapshot!(
            format!(
                "{}_{}_{}",
                case.format,
                case.found,
                case.lowest_minor.unwrap_or(0)
            ),
            read.unwrap_err()
        ),
    }
}

/// A newer engine that writes a user configuration key its minor added
/// raises the file's lowest minor to that minor, and the older engine of
/// the major then refuses the file, naming the version to install (ADR
/// 0098).
///
/// Covers: story:3-144, format:user-configuration
#[test]
fn a_newer_engine_writing_a_key_its_minor_added_raises_the_lowest_minor() {
    let text = "version = \"0.1\"\n\n[settings.crawl.retry]\nmax = 5 # Foxhound's\n";

    let written = set(
        text,
        "settings",
        "timeout.navigation",
        &json!(3000),
        &USER_CONFIGURATION,
        &version("2.3.0"),
        ProtocolVersion { major: 2, minor: 3 },
    )
    .unwrap();
    let older = read_as("user-configuration", &written, &version("2.2.4"));

    assert_eq!(
        (written.as_str(), older),
        (
            "version = \"0.1\"\nlowest_minor = 3\n\n[settings.crawl.retry]\nmax = 5 # Foxhound's\n\n[settings.timeout]\nnavigation = 3000\n",
            Err("format_version_unsupported: this user-configuration file (0.1) can be used from Goodfetch 2.3 on; install Goodfetch 2.3 or newer".to_owned())
        )
    );
}

/// An engine whose minor meets a file's lowest minor writes a key an older
/// minor added: the lowest minor stays, and every entry it does not write,
/// unknown ones included, keeps its bytes (ADR 0098).
///
/// Covers: story:3-144, format:user-configuration
#[test]
fn a_key_an_older_minor_added_leaves_the_lowest_minor_and_every_other_entry() {
    let text = "version = \"0.1\"\nlowest_minor = 3\n\n[settings]\nfrom_a_newer_minor = \"kept\"\n";

    let written = set(
        text,
        "settings",
        "timeout.navigation",
        &json!(3000),
        &USER_CONFIGURATION,
        &version("2.4.0"),
        ProtocolVersion { major: 2, minor: 1 },
    )
    .unwrap();

    assert_eq!(
        written,
        "version = \"0.1\"\nlowest_minor = 3\n\n[settings]\nfrom_a_newer_minor = \"kept\"\n\n[settings.timeout]\nnavigation = 3000\n"
    );
}

/// A new major that finds an Operator policy only for an older major
/// refuses to start, naming `upgrade-files`, so the operator's caps and
/// locks are never lost silently (ADR 0030); once the new major has its
/// own copy, it reads it.
///
/// Covers: story:3-94, format:operator-policy
#[rstest]
#[case::only_the_older_majors(false)]
#[case::its_own_copy_beside_it(true)]
fn a_new_major_refuses_an_older_majors_operator_policy_until_it_has_its_copy(#[case] copied: bool) {
    let home = tempfile::tempdir().unwrap();
    let older = home.path().join("2");
    std::fs::create_dir_all(&older).unwrap();
    std::fs::write(
        older.join("operator-policy.toml"),
        "version = \"0.1\"\n[lock.timeout]\nnavigation = 1000\n",
    )
    .unwrap();
    let home = Home::new(home.path().to_owned());
    let files = SettingsFiles::of(&home, &version("3.0.0"));
    if copied {
        std::fs::create_dir_all(home.root().join("3")).unwrap();
        std::fs::copy(older.join("operator-policy.toml"), files.operator_policy()).unwrap();
    }

    let loaded = files.load(Product::Collie);

    match loaded {
        Ok(layers) => assert!(copied, "{layers:?}"),
        Err(error) => {
            assert!(!copied, "{error}");
            assert!(
                matches!(error, SettingsError::OlderMajorPolicy { ref major, ref current, .. } if major == "2" && current == "3"),
                "{error:?}"
            );
            insta::assert_snapshot!(
                format!("{}: {error}", error.code())
                    .replace(&home.root().display().to_string(), "[home]")
            );
        }
    }
}
