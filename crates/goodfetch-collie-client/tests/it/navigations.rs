//! A member whose operation answers one field returns that field's value
//! (issue 2517): a navigation answers the document's response, or `None`
//! when it has none, and an evaluation the function's value, with the
//! Scripted engine as the Daemon playing the corpus's script the
//! TypeScript SDK plays too.
//! Covers: story:11-84

use goodfetch_collie_client::transport::{Endpoint, Network};
use goodfetch_collie_client::{Connection, ConnectionRuntime};
use goodfetch_test_support::TestEnv;
use serde_json::json;

use crate::scripted_daemon::{daemon_hello, scripted_daemon};

#[test]
fn a_navigation_answers_its_response_or_none_and_an_evaluation_its_value() {
    let env = TestEnv::new();
    let (process, endpoint) = scripted_daemon(&env, "page/navigations.json");
    let runtime = goodfetch_test_support::runtime();
    runtime.block_on(async {
        let (connection, _said) = Connection::open(
            ConnectionRuntime::driven(tokio::runtime::Handle::current()),
            &Network::Os,
            &Endpoint::Daemon(endpoint),
            daemon_hello(),
        )
        .await
        .unwrap();
        let run = connection.start().await.unwrap();
        let page = run.new_page().await.unwrap();

        let missing = page.goto("https://shop.test/missing").await.unwrap();
        let reloaded = page.reload().await.unwrap();
        let answered = page.evaluate("6 * 7").await.unwrap();
        let left_out = page.evaluate("undefined").await.unwrap();
        page.close().await.unwrap();
        run.finish().await.unwrap();
        connection.close().await.unwrap();

        let missing = missing.expect("a 404 is a response");
        assert_eq!(
            (
                missing.status(),
                missing.url(),
                reloaded.is_none(),
                answered,
                left_out
            ),
            (
                404,
                "https://shop.test/missing",
                true,
                Some(json!(42)),
                None
            )
        );
    });
    let status = process.child().wait().unwrap();
    assert!(
        status.success(),
        "the Scripted engine played its script whole"
    );
}
