use goodfetch_protocol::connection::HelloParams;

use super::*;

#[test]
fn a_hello_reads_back_as_the_protocols_params() {
    let hello = Hello::needing(Product::Foxhound)
        .id("hello")
        .surface(Surface::SdkTypescript)
        .token("secret");

    let message = hello.message();
    let params: HelloParams = serde_json::from_value(message["params"].clone()).unwrap();

    assert_eq!(
        (
            &message["method"],
            &message["id"],
            params.needs,
            params.token.as_deref(),
            params.protocol_version
        ),
        (
            &json!(HELLO),
            &json!("hello"),
            Some(Product::Foxhound),
            Some("secret"),
            PROTOCOL_VERSION
        )
    );
}

#[test]
fn the_newer_minor_is_above_the_protocol_version_of_the_same_major() {
    let newer = newer_minor();

    assert_eq!(
        (newer.major, newer > PROTOCOL_VERSION),
        (PROTOCOL_VERSION.major, true)
    );
}
