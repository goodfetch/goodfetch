//! A Daemon's token in its contracts: plain text in JSON and in its
//! schema, never in `Debug` (STYLE.md).

use schemars::SchemaGenerator;
use serde_json::json;

use super::{DaemonToken, HttpListenerToken, TrustedProxy};

#[test]
fn a_token_prints_in_json_and_never_in_debug() {
    let printed = HttpListenerToken {
        token: DaemonToken::new("not-a-real-token"),
    };

    let (debug, json) = (format!("{printed:?}"), serde_json::to_value(&printed));

    assert_eq!(
        (debug.contains("not-a-real-token"), json.ok()),
        (false, Some(json!({ "token": "not-a-real-token" })))
    );
}

/// The token's schema is the plain text the field held before the
/// newtype, inline, so the bundle names no type for it.
#[test]
fn a_tokens_schema_is_plain_text() {
    let mut generator = SchemaGenerator::default();

    let schema = serde_json::to_value(generator.subschema_for::<HttpListenerToken>()).ok();
    let token = generator
        .definitions()
        .get("HttpListenerToken")
        .map(|definition| definition["properties"]["token"].clone());

    assert_eq!(
        (
            token.map(|token| (token["type"].clone(), token.get("$ref").is_some())),
            generator.definitions().contains_key("DaemonToken"),
            schema,
        ),
        (
            Some((json!("string"), false)),
            false,
            Some(json!({ "$ref": "#/$defs/HttpListenerToken" })),
        )
    );
}

/// `daemon.trusted_proxies` takes addresses and CIDR ranges, and refuses a
/// host name, whose address a proxy's peer address is never compared to.
#[test]
fn a_trusted_proxy_is_an_address_or_a_range_and_never_a_name() {
    let read = |value| {
        TrustedProxy::list(&value)
            .map(|proxies| proxies.iter().map(ToString::to_string).collect::<Vec<_>>())
    };

    assert_eq!(
        (
            read(json!(["10.0.0.0/8", "192.0.2.7", "[::1]", "fd00::/8"])),
            read(json!(null)),
            read(json!(["ingress.example"])).map_err(|error| error.contains("`ingress.example`")),
            read(json!(["10.0.0.0/33"])).is_err(),
            read(json!("10.0.0.1")).is_err(),
        ),
        (
            Ok(vec![
                "10.0.0.0/8".to_owned(),
                "192.0.2.7/32".to_owned(),
                "::1/128".to_owned(),
                "fd00::/8".to_owned(),
            ]),
            Ok(Vec::new()),
            Err(true),
            true,
            true,
        )
    );
}

/// A peer is trusted when its address is in a listed range, an IPv4 peer
/// reached over a dual-stack socket as its mapped IPv6 address among them.
#[test]
fn a_trusted_proxy_holds_the_addresses_of_its_range() {
    let proxy = |entry| TrustedProxy::parse(entry).unwrap();
    let holds = |entry, address: &str| proxy(entry).holds(address.parse().unwrap());

    assert_eq!(
        [
            holds("10.0.0.0/8", "10.200.3.4"),
            holds("10.0.0.0/8", "11.0.0.1"),
            holds("192.0.2.7", "192.0.2.7"),
            holds("192.0.2.7", "192.0.2.8"),
            holds("127.0.0.1", "::ffff:127.0.0.1"),
            holds("fd00::/8", "fd12::1"),
            holds("fd00::/8", "10.0.0.1"),
            holds("0.0.0.0/0", "203.0.113.9"),
        ],
        [true, false, true, false, true, true, false, true]
    );
}
