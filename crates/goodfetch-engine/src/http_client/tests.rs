//! Who may call the engine's HTTP client (ADR 0043): the modules
//! `modules.toml` lets import it, which the module check holds every import
//! to, each either `pinned_download` or the module of a kind of
//! Operator-configured endpoint (spec #564's two egress classes).

use std::collections::BTreeMap;

use rstest::rstest;

/// The crate's module rules, as the module check reads them.
const MODULES: &str = include_str!("../../modules.toml");

/// The module the HTTP client is.
const HTTP_CLIENT: &str = "http_client";

/// The kinds of destination ADR 0043 lets the engine connect to: pinned
/// downloads, which nobody configures, and each kind of Operator-configured
/// endpoint, the Clean-up rules' sources among them (ADR 0047: `daily` and
/// `upstream` make them configured destinations).
const ALLOWED_KINDS: [&str; 9] = [
    "pinned download",
    "Model provider",
    "Secret provider",
    "proxy",
    "Notification webhook",
    "OTLP",
    "Plugin index",
    "cdp_url",
    "Clean-up rules source",
];

/// Each module that calls the HTTP client, with the kind of destination it
/// connects to. A module that starts calling it joins here, with its kind.
const CALLERS: [(&str, &str); 2] = [
    ("pinned_download", "pinned download"),
    ("rule_sources", "Clean-up rules source"),
];

/// The modules `modules` lets import the HTTP client whose kind of
/// destination is none ADR 0043 allows.
fn callers_of_no_allowed_kind(modules: &str) -> Vec<String> {
    let callers: BTreeMap<&str, &str> = CALLERS.into_iter().collect();
    let document: toml_edit::DocumentMut = modules.parse().unwrap();
    document["modules"]
        .as_table()
        .unwrap()
        .iter()
        .filter(|(_, imports)| {
            imports
                .as_array()
                .unwrap()
                .iter()
                .any(|import| import.as_str() == Some(HTTP_CLIENT))
        })
        .map(|(module, _)| module)
        .filter(|module| {
            !callers
                .get(module)
                .is_some_and(|kind| ALLOWED_KINDS.contains(kind))
        })
        .map(str::to_owned)
        .collect()
}

/// Covers: story:4-81, row:SC-10
#[rstest]
#[case::the_crates_own_rules(MODULES, &[])]
#[case::a_module_of_no_allowed_kind(
    "[modules]\nbrowser_cache = [\"http_client\"]\nhttp_client = []\npinned_download = [\"http_client\"]\n",
    &["browser_cache"],
)]
fn every_caller_of_the_http_client_is_a_pinned_download_or_a_configured_kind(
    #[case] modules: &str,
    #[case] refused: &[&str],
) {
    assert_eq!(callers_of_no_allowed_kind(modules), refused);
}
