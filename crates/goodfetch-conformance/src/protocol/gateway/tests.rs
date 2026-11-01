use rstest::rstest;
use serde_json::json;

use super::{GatewayAddress, collies_refusal_behind_the_gateway};

#[rstest]
#[case::an_https_origin("https://gateway.example", "wss://gateway.example/protocol", 443)]
#[case::an_http_origin_with_a_port("http://127.0.0.1:4100", "ws://127.0.0.1:4100/protocol", 4100)]
#[case::a_trailing_slash("http://127.0.0.1:4100/", "ws://127.0.0.1:4100/protocol", 4100)]
#[case::a_path(
    "https://cloud.example/gateway",
    "wss://cloud.example/gateway/protocol",
    443
)]
fn a_gateways_address_gives_its_websocket_and_port(
    #[case] address: &str,
    #[case] websocket: &str,
    #[case] port: u16,
) {
    let address = GatewayAddress::parse(address).unwrap();

    assert_eq!(
        (address.websocket().as_str(), address.port()),
        (websocket, port)
    );
}

#[rstest]
#[case::another_scheme("ftp://gateway.example")]
#[case::credentials("https://user:secret@gateway.example")]
#[case::a_query("https://gateway.example/?token=1")]
#[case::no_url("gateway.example:443")]
fn text_that_is_no_gateways_address_is_refused(#[case] address: &str) {
    assert!(GatewayAddress::parse(address).is_err());
}

/// Collie's engine names the install command in `install` wherever it is
/// not behind the gateway, whatever its channel; the OCI channel's,
/// `docker pull`, has no "install" in it.
#[rstest]
#[case::behind_the_gateway(None, true)]
#[case::on_the_crates_io_channel(
    Some("cargo install goodfetch-foxhound --locked --features executable"),
    false
)]
#[case::on_the_oci_channel(Some("docker pull ghcr.io/goodfetch/foxhound"), false)]
fn only_a_refusal_naming_no_install_comes_from_collies_engine_behind_the_gateway(
    #[case] install: Option<&str>,
    #[case] behind: bool,
) {
    // Collie's engine's own wording of the two `fix`es (ADR 0096).
    let fix = match install {
        Some(install) => format!(
            "Run it with Foxhound, which contains everything Collie does (`{install}`), or connect to a Foxhound engine."
        ),
        None => {
            "Only a run started for Foxhound carries `crawl.start`; start the run for Foxhound."
                .to_owned()
        }
    };
    let mut data = json!({
        "code": "not_in_product",
        "product": "collie",
        "product_version": "0.0.0",
        "needs": "foxhound",
        "requested": "crawl.start",
        "fix": fix,
        "retryable": false,
    });
    if let Some(install) = install {
        data["install"] = json!(install);
    }
    let answer = json!({"error": {"code": -32006, "message": "", "data": data}});

    assert_eq!(
        collies_refusal_behind_the_gateway(&answer, "crawl.start"),
        behind
    );
}
