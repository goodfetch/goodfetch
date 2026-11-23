#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use rstest::rstest;
use serde_json::{Value, json};

use super::{POLITENESS_STORE, PolitenessStoreConfiguration, StoreAddressing};
use crate::catalog::{Authority, ValueKind};
use crate::secret_reference::SecretReference;

/// What `politeness.store`'s value kind says of `value`.
fn checked(value: &Value) -> Result<(), String> {
    POLITENESS_STORE.kind.check(value)
}

/// A store reached over TLS, with its namespace, its user and its password
/// as a Secret reference, reads back as the configuration it names.
///
/// Covers: setting:politeness.store
#[test]
fn a_store_over_tls_with_a_password_reference_is_read() {
    let value = json!({
        "namespace": "crawl",
        "addressing": { "kind": "server", "url": "rediss://valkey.example:6380" },
        "username": "crawler",
        "password": "env:VALKEY_PASSWORD",
    });

    let read = PolitenessStoreConfiguration::read(&value).unwrap();

    assert_eq!(checked(&value), Ok(()));
    assert_eq!(read.namespace.as_str(), "crawl");
    assert_eq!(read.username.as_deref(), Some("crawler"));
    assert_eq!(
        read.password,
        Some(SecretReference::parse("env:VALKEY_PASSWORD").unwrap())
    );
    let StoreAddressing::Server { url } = &read.addressing else {
        panic!("a server's addressing, read {:?}", read.addressing);
    };
    assert_eq!(
        (url.as_str(), url.tls()),
        ("rediss://valkey.example:6380", true)
    );
    assert_eq!(serde_json::to_value(&read).unwrap(), value);
}

/// TLS is required off loopback: a plaintext `redis://` address is taken
/// for a loopback host or a Unix socket only, since it would carry the
/// password and every key in clear (ADR 0053).
///
/// Covers: setting:politeness.store
#[rstest]
#[case::tls_anywhere("rediss://valkey.example:6380", true)]
#[case::loopback_v4("redis://127.0.0.1:6379", true)]
#[case::loopback_v4_range("redis://127.3.2.1", true)]
#[case::loopback_v6("redis://[::1]:6379/0", true)]
#[case::localhost("redis://localhost:6379", true)]
#[case::unix_socket("redis+unix:///run/valkey/valkey.sock", true)]
#[case::unix_scheme("unix:///run/valkey/valkey.sock", true)]
#[case::plaintext_host("redis://valkey.example:6379", false)]
#[case::plaintext_private_address("redis://10.0.0.5:6379", false)]
#[case::plaintext_any_address("redis://0.0.0.0:6379", false)]
#[case::localhost_lookalike("redis://localhost.example:6379", false)]
#[case::not_a_store("https://valkey.example", false)]
#[case::no_host("rediss://:6379", false)]
fn a_plaintext_address_is_taken_on_loopback_only(#[case] url: &str, #[case] taken: bool) {
    let value = json!({
        "namespace": "crawl",
        "addressing": { "kind": "server", "url": url },
    });

    assert_eq!(checked(&value).is_ok(), taken, "{:?}", checked(&value));
}

/// A plaintext address off loopback is refused with a message that asks
/// for TLS.
#[test]
fn a_plaintext_address_off_loopback_asks_for_tls() {
    let refused = checked(&json!({
        "namespace": "crawl",
        "addressing": { "kind": "server", "url": "redis://valkey.example:6379" },
    }));

    assert!(
        refused
            .as_ref()
            .is_err_and(|message| message.contains("rediss://")),
        "{refused:?}"
    );
}

/// The password is a Secret reference, never a value, and no refusal
/// repeats a password given in its place or inside an address (ADR 0015).
///
/// Covers: setting:politeness.store
#[rstest]
#[case::raw_password(json!({
    "namespace": "crawl",
    "addressing": { "kind": "server", "url": "rediss://valkey.example" },
    "password": "hunter2-secret",
}))]
#[case::password_in_the_address(json!({
    "namespace": "crawl",
    "addressing": { "kind": "server", "url": "rediss://crawler:hunter2-secret@valkey.example" },
}))]
#[case::password_in_a_query(json!({
    "namespace": "crawl",
    "addressing": { "kind": "server", "url": "redis+unix:///run/valkey.sock?pass=hunter2-secret" },
}))]
#[case::password_in_a_sentinel(json!({
    "namespace": "crawl",
    "addressing": {
        "kind": "sentinel",
        "service": "primary",
        "sentinels": ["rediss://:hunter2-secret@sentinel.example:26379"],
    },
}))]
fn a_password_is_a_secret_reference_and_never_repeated(#[case] value: Value) {
    let refused = checked(&value);

    assert!(
        refused
            .as_ref()
            .is_err_and(|message| !message.contains("hunter2")),
        "{refused:?}"
    );
}

/// A Sentinel deployment names its sentinels and the service whose primary
/// they elect; a Cluster names the nodes it is discovered from. Each
/// address follows the TLS rule, and neither list is empty.
///
/// Covers: setting:politeness.store
#[rstest]
#[case::sentinel(json!({
    "kind": "sentinel",
    "service": "primary",
    "sentinels": ["rediss://sentinel-1.example:26379", "rediss://sentinel-2.example:26379"],
}), true)]
#[case::cluster(json!({
    "kind": "cluster",
    "nodes": ["rediss://node-1.example:6379", "rediss://node-2.example:6379"],
}), true)]
#[case::local_cluster(json!({ "kind": "cluster", "nodes": ["redis://127.0.0.1:7000"] }), true)]
#[case::plaintext_sentinel(json!({
    "kind": "sentinel",
    "service": "primary",
    "sentinels": ["rediss://sentinel-1.example:26379", "redis://sentinel-2.example:26379"],
}), false)]
#[case::plaintext_node(json!({ "kind": "cluster", "nodes": ["redis://node-1.example:6379"] }), false)]
#[case::no_sentinel(json!({ "kind": "sentinel", "service": "primary", "sentinels": [] }), false)]
#[case::no_service(json!({
    "kind": "sentinel",
    "service": "",
    "sentinels": ["rediss://sentinel-1.example:26379"],
}), false)]
#[case::no_node(json!({ "kind": "cluster", "nodes": [] }), false)]
#[case::unknown_kind(json!({ "kind": "replica", "url": "rediss://valkey.example" }), false)]
fn sentinel_and_cluster_addressing_follow_the_same_rules(
    #[case] addressing: Value,
    #[case] taken: bool,
) {
    let value = json!({ "namespace": "crawl", "addressing": addressing });

    assert_eq!(checked(&value).is_ok(), taken, "{:?}", checked(&value));
}

/// A namespace is a non-empty name with no brace, which would make a hash
/// tag of its own and move a key's parts across Cluster slots.
#[rstest]
#[case::named("crawl-team.eu", true)]
#[case::empty("", false)]
#[case::opening_brace("a{b", false)]
#[case::closing_brace("a}b", false)]
fn a_namespace_is_a_name_without_braces(#[case] namespace: &str, #[case] taken: bool) {
    let value = json!({
        "namespace": namespace,
        "addressing": { "kind": "server", "url": "rediss://valkey.example" },
    });

    assert_eq!(checked(&value).is_ok(), taken, "{:?}", checked(&value));
}

/// The store is the machine's: set in user configuration or Operator
/// policy only, none by default, and refused as unbuilt until Politeness
/// reads it (ADR 0083).
#[test]
fn the_store_is_set_on_the_machine_and_none_by_default() {
    assert!(matches!(POLITENESS_STORE.authority, Authority::Machine));
    assert_eq!((POLITENESS_STORE.default)(), Value::Null);
    assert!(POLITENESS_STORE.clearable);
    assert!(matches!(POLITENESS_STORE.kind, ValueKind::Shape(_)));
    assert_eq!(POLITENESS_STORE.unbuilt, Some("politeness_store"));
}
