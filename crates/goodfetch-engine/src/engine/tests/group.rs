use std::time::Duration;

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::point_in_time;
use goodfetch_test_support::TestEnv;
use serde_json::{Value, json};

use super::{Client, attached_on, engine_with, opened, owed, request};

/// The reply to `method`, called alone on `client`.
async fn called(client: &mut Client, id: i64, method: &str) -> Value {
    let mut call = request(id, method);
    if method == "group.init" {
        call["params"] = json!({ "name": "lead", "peer_address": "127.0.0.1:0" });
    }
    client.send(&[call]).await;
    owed(client).await.expect("the engine answers the call")
}

/// Covers: operation:group.init, error:group_exists
#[test]
fn group_init_forms_one_group_at_lead_epoch_one_and_refuses_a_second() {
    let env = TestEnv::new();
    let engine = engine_with(Product::Collie, Some(crate::group::contribution()));

    let (formed, again) = attached_on(&engine, &env, |mut client| async move {
        opened(&mut client).await;
        let formed = called(&mut client, 1, "group.init").await;
        let again = called(&mut client, 2, "group.init").await;
        (formed, again)
    });

    assert_eq!(formed["result"]["lead_epoch"], json!(1));
    let authority_key = formed["result"]["authority_key"].as_str().unwrap();
    assert!(authority_key.starts_with("sha256/"), "{authority_key}");
    assert_eq!(again["error"]["data"]["code"], json!("group_exists"));
}

/// Covers: error:not_lead_daemon
#[test]
fn group_token_is_refused_before_a_group_is_formed() {
    let env = TestEnv::new();
    let engine = engine_with(Product::Collie, Some(crate::group::contribution()));

    let refused = attached_on(&engine, &env, |mut client| async move {
        opened(&mut client).await;
        called(&mut client, 1, "group.token").await
    });

    assert_eq!(refused["error"]["data"]["code"], json!("not_lead_daemon"));
}

/// Covers: operation:group.token
#[test]
fn group_token_mints_a_thirty_minute_token_for_the_groups_authority() {
    let env = TestEnv::new();
    let engine = engine_with(Product::Collie, Some(crate::group::contribution()));

    let (formed, minted) = attached_on(&engine, &env, |mut client| async move {
        opened(&mut client).await;
        let formed = called(&mut client, 1, "group.init").await;
        let minted = called(&mut client, 2, "group.token").await;
        (formed, minted)
    });

    let minted = &minted["result"];
    let token = minted["token"].as_str().unwrap();
    assert_eq!(token.len(), 64);
    assert!(token.bytes().all(|byte| byte.is_ascii_hexdigit()));
    assert_eq!(
        minted["expires_at"],
        json!(point_in_time::to_rfc3339(env.start_time() + Duration::from_mins(30)).unwrap())
    );
    assert_eq!(minted["authority_key"], formed["result"]["authority_key"]);
}
