//! ADR 0056's Exposed Daemon row, through Collie's executable: a Client of
//! an Exposed Daemon runs on another machine than the Daemon, so a
//! loopback `cdp_url` it gives, in the run layer or in the project
//! configuration it sends, is refused with `cdp_url_not_allowed` before
//! any connection, where an Attached engine's program opens it.
#![expect(
    clippy::disallowed_methods,
    reason = "the `cdp_url` stand-in accepts on a thread of its own"
)]
#![expect(
    clippy::disallowed_types,
    reason = "the `cdp_url` stand-in listens and counts outside the engine (ADR 0060)"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use goodfetch_test_client::Attached;
use goodfetch_test_support::TestEnv;
use serde_json::{Value, json};

use crate::daemon_run::{ANY_PORT, Foreground, opened};

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// What `cdp_url_not_allowed` asks for.
const FIX: &str =
    "set the address in Operator policy or in user configuration, or give a public address";

/// A loopback listener that counts the connections it accepts and drops
/// each: where a `cdp_url` points, to tell whether the engine connected.
fn stand_in() -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let accepted = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&accepted);
    std::thread::spawn(move || {
        for stream in listener.incoming().map_while(Result::ok) {
            counted.fetch_add(1, Ordering::SeqCst);
            drop(stream);
        }
    });
    (
        format!("ws://127.0.0.1:{port}/devtools/browser/stand-in"),
        accepted,
    )
}

/// The Error code of a call's error and its details.
fn error(answer: &Value) -> (Value, Value) {
    let mut details = answer["error"]["data"].clone();
    let code = details["code"].take();
    if let Some(details) = details.as_object_mut() {
        details.remove("code");
        details.remove("retryable");
    }
    (code, details)
}

/// On an Exposed Daemon behind TLS terminated upstream, a loopback
/// `cdp_url` from the program's own code, and one in the project
/// configuration its Client sends, are refused with `cdp_url_not_allowed`
/// before any connection; the same `cdp_url` from an Attached engine's
/// program reaches the address (positive control), where the stand-in,
/// no browser, ends it.
///
/// Covers: story:4-25, error:cdp_url_not_allowed, operation:run.new_session
#[test]
fn on_an_exposed_daemon_a_loopback_cdp_url_is_refused() {
    let env = TestEnv::new();
    let (url, accepted) = stand_in();
    env.user_configuration(&format!(
        "daemon.address = \"{ANY_PORT}\"\ndaemon.tls.upstream = true\n"
    ));
    let mut daemon = Foreground::start(&env, COLLIE, &[]);
    let endpoint = daemon.serving();
    let mut client = opened(&env, &endpoint);
    let source = json!({ "kind": "cdp_url", "url": url });
    let project = json!({
        "path": "goodfetch.toml",
        "settings": { "session": { "source": source } },
    });

    let own_started = client.call(1, "run.start", &json!({ "run_id": "own" }));
    let own = client.call(
        2,
        "run.new_session",
        &json!({ "run_id": "own", "source": source }),
    );
    let project_started = client.call(
        3,
        "run.start",
        &json!({ "run_id": "project", "project": project }),
    );
    let from_project = client.call(4, "run.new_page", &json!({ "run_id": "project" }));
    let refused_before_any_connection = accepted.load(Ordering::SeqCst);
    drop(client);
    let exited = daemon.terminate();

    let mut attached = Attached::start(&env, COLLIE);
    attached.open(&goodfetch_test_client::Hello::needing(
        goodfetch_protocol::catalog::Product::Collie,
    ));
    let _ = attached.ask(5, "run.start", &json!({ "run_id": "attached" }));
    let reached = attached.ask(
        6,
        "run.new_session",
        &json!({ "run_id": "attached", "source": source }),
    );
    let _ = attached.finish();

    let refused = |layer: &str| {
        (
            json!("cdp_url_not_allowed"),
            json!({ "url": url, "layer": layer, "rule": "loopback", "address": "127.0.0.1", "fix": FIX }),
        )
    };
    assert_eq!(
        (
            own_started.get("result").is_some(),
            error(&own),
            project_started.get("result").is_some(),
            error(&from_project),
            refused_before_any_connection,
            exited,
            error(&reached).0 != json!("cdp_url_not_allowed"),
            accepted.load(Ordering::SeqCst) > 0,
        ),
        (
            true,
            refused("run"),
            true,
            refused("project_configuration"),
            0,
            true,
            true,
            true,
        ),
        "{own_started} {project_started} {reached}"
    );
}
