//! Who may call the engine's HTTP client (ADR 0043): the modules
//! `modules.toml` lets import it, which the module check holds every import
//! to, each either `pinned_download` or the module of a kind of
//! Operator-configured endpoint that `egress`'s `CONFIGURED_KINDS` classes
//! (spec #564's two egress classes).

use std::collections::BTreeMap;

use rstest::rstest;

use crate::egress::{CONFIGURED_KINDS, ConfiguredKind};

/// The crate's module rules, as the module check reads them.
const MODULES: &str = include_str!("../../modules.toml");

/// The module the HTTP client is.
const HTTP_CLIENT: &str = "http_client";

/// The kind of destination a caller of the HTTP client connects to: ADR
/// 0043's pinned downloads, which nobody configures, or a kind of
/// Operator-configured endpoint.
#[derive(Clone, Copy, Debug)]
enum Destination {
    PinnedDownload,
    Configured(ConfiguredKind),
}

impl Destination {
    /// Whether ADR 0043 lets the engine connect to it: a pinned download,
    /// or a kind the egress policy's table classes.
    fn is_classified(self) -> bool {
        match self {
            Self::PinnedDownload => true,
            Self::Configured(kind) => CONFIGURED_KINDS.iter().any(|row| row.kind == kind),
        }
    }
}

/// Each module that calls the HTTP client, with the kind of destination it
/// connects to. A module that starts calling it joins here, with its kind.
const CALLERS: [(&str, Destination); 4] = [
    ("pinned_download", Destination::PinnedDownload),
    (
        "rule_sources",
        Destination::Configured(ConfiguredKind::CleanUpRulesSource),
    ),
    ("otlp", Destination::Configured(ConfiguredKind::Otlp)),
    (
        "secret_endpoint",
        Destination::Configured(ConfiguredKind::SecretProvider),
    ),
];

/// The modules `modules` lets import the HTTP client whose kind of
/// destination is none ADR 0043 allows.
fn callers_of_no_allowed_kind(modules: &str) -> Vec<String> {
    let callers: BTreeMap<&str, Destination> = CALLERS.into_iter().collect();
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
                .is_some_and(|destination| destination.is_classified())
        })
        .map(str::to_owned)
        .collect()
}

/// Covers: story:4-81, story:4-16, row:SC-10
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

/// The addresses [`super::lookup`] gives `host` under a mapping of
/// `staging.proving.test` and of the address `127.0.0.1`.
fn looked_up(host: &str) -> Option<Vec<std::net::SocketAddr>> {
    let mapping = crate::dns::HostMapping::default()
        .with("staging.proving.test", vec!["192.0.2.7".parse().unwrap()])
        .with("127.0.0.1", vec!["192.0.2.8".parse().unwrap()]);
    goodfetch_test_support::runtime()
        .block_on(super::lookup(host, 443, &mapping))
        .ok()
}

/// A mapped name answers whatever its case, IDNA spelling and trailing dot,
/// as the engine's resolver (`dns::Dns`) answers it, so a host matched there
/// is never missed here.
/// Covers: setting:network.resolve
#[rstest]
#[case::as_listed("staging.proving.test")]
#[case::fully_qualified("staging.proving.test.")]
#[case::upper_case("Staging.Proving.TEST.")]
fn a_mapped_name_answers_whatever_its_case_and_trailing_dot(#[case] host: &str) {
    assert_eq!(
        looked_up(host),
        Some(vec!["192.0.2.7:443".parse().unwrap()])
    );
}

/// An address is its own answer, whatever the mapping lists for it, as the
/// engine's resolver has it: a mapping holds host names (ADR 0059).
/// Covers: setting:network.resolve
#[test]
fn an_address_answers_itself_whatever_the_mapping_lists() {
    assert_eq!(
        looked_up("127.0.0.1"),
        Some(vec!["127.0.0.1:443".parse().unwrap()])
    );
}

/// A name the system resolver answers never waits for a thread of Tokio's
/// blocking pool (#2870): with the runtime's one blocking thread held,
/// `localhost` answers with a loopback address while it is still held.
#[test]
fn a_system_lookup_never_waits_for_a_blocking_thread() {
    let (found, held_throughout) = goodfetch_test_support::runtime_with_one_blocking_thread()
        .block_on(goodfetch_test_support::with_the_blocking_pool_held(
            super::lookup("localhost", 443, &crate::dns::HostMapping::default()),
        ));

    let loopback = found.is_ok_and(|found| found.iter().any(|address| address.ip().is_loopback()));
    assert_eq!((loopback, held_throughout), (true, true));
}
