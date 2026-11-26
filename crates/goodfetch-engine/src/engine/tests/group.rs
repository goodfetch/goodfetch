use std::time::Duration;

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::point_in_time;
use goodfetch_test_support::TestEnv;
use serde_json::{Value, json};

use super::{Client, attached_on, engine_with, opened, owed, request};

/// The reply to `method`, called alone on `client`.
async fn called(client: &mut Client, id: i64, method: &str) -> Value {
    client.send(&[request(id, method)]).await;
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

/// Covers: operation:group.token, error:not_lead_daemon
#[test]
fn group_token_is_minted_only_once_the_group_is_formed() {
    let env = TestEnv::new();
    let engine = engine_with(Product::Collie, Some(crate::group::contribution()));

    let (refused, formed, first, second) = attached_on(&engine, &env, |mut client| async move {
        opened(&mut client).await;
        let refused = called(&mut client, 1, "group.token").await;
        let formed = called(&mut client, 2, "group.init").await;
        let first = called(&mut client, 3, "group.token").await;
        let second = called(&mut client, 4, "group.token").await;
        (refused, formed, first, second)
    });

    assert_eq!(refused["error"]["data"]["code"], json!("not_lead_daemon"));
    let minted = &first["result"];
    let token = minted["token"].as_str().unwrap();
    assert_eq!(token.len(), 64);
    assert!(token.bytes().all(|byte| byte.is_ascii_hexdigit()));
    assert_ne!(minted["token"], second["result"]["token"]);
    assert_eq!(
        minted["expires_at"],
        json!(point_in_time::to_rfc3339(env.start_time() + Duration::from_mins(30)).unwrap())
    );
    assert_eq!(minted["authority_key"], formed["result"]["authority_key"]);
}
