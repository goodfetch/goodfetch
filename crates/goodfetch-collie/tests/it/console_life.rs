//! The Console and the Daemon's life on Collie's executable (ADRs 0031 and
//! 0041; spec #591, stories 4 and 9): an Attached engine serves its own
//! Console only when its script gives it a Console address, and nothing
//! listens once the script ends; links to the Console name
//! `console.public_url` when set, and carry no credential.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#![expect(
    clippy::disallowed_types,
    reason = "a test dials the Console's port from outside the engine to see that nothing listens (ADR 0060)"
)]

use std::net::{SocketAddr, TcpStream};

use goodfetch_protocol::catalog::Product;
use goodfetch_test_support::MESSAGE_DEADLINE;
use goodfetch_test_support::{RuntimeFile, TestEnv, cli};
use serde_json::{Value, json};

use crate::daemon::StopsTheDaemon;
use crate::daemon_client;
use crate::sign_in::{agent, sign_in};
use goodfetch_test_client::{Attached, hello};

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// This commit's Product version.
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The `hello` reply's result of `attached`, which it answers first.
fn hello_result(attached: &mut Attached) -> Value {
    attached.send(&hello(Product::Collie));
    let reply = attached.next().unwrap();
    assert_eq!(reply["id"], 0, "{reply}");
    reply["result"].clone()
}

/// The address `url`, `http://<host>:<port>/…`, names.
fn address_of(url: &str) -> SocketAddr {
    let authority = url
        .strip_prefix("http://")
        .unwrap()
        .split(['/', '#'])
        .next()
        .unwrap();
    authority.parse().unwrap()
}

/// A script that asks for its own Console starts its Attached engine with a
/// Console address: the engine serves its Console there, under the
/// production headers, signs in through the link its `hello` answers, and
/// stops listening once the script ends.
///
/// Covers: story:12-4, command:attached-engine
#[test]
fn an_attached_engine_given_a_console_address_serves_its_console_until_its_script_ends() {
    let env = TestEnv::new();
    let mut attached = Attached::start_with(&env, COLLIE, MESSAGE_DEADLINE, |command| {
        command.args(["--console-address", "127.0.0.1:0"]);
    });

    let link = hello_result(&mut attached)["console_link"]
        .as_str()
        .expect("the hello reply names the Console's sign-in link")
        .to_owned();
    let (url, token) = link.split_once("#token=").unwrap();
    let page = agent().get(url).call().unwrap();
    let headers = page.headers();
    assert_eq!(page.status().as_u16(), 200);
    assert!(
        headers["content-security-policy"]
            .to_str()
            .unwrap()
            .starts_with("default-src 'none'; script-src 'self';"),
        "{headers:?}"
    );
    assert_eq!(headers["referrer-policy"], "no-referrer");
    let (signed_in, cookie) = sign_in(url, token);
    assert_eq!((signed_in, cookie.is_some()), (204, true));

    let (_, ended) = attached.finish();

    assert!(ended, "the Attached engine exits once its script ends");
    let address = address_of(url);
    assert!(
        TcpStream::connect(address).is_err(),
        "nothing listens at {address} once the script ended"
    );
}

/// An Attached engine started without a Console address serves none, even
/// when the user configuration sets `console.address`, as the test home's
/// does: that address is the Daemon's.
///
/// Covers: story:12-4, setting:console.address, command:attached-engine
#[test]
fn an_attached_engine_without_a_console_address_serves_none() {
    let env = TestEnv::new();
    env.user_configuration("console.address = \"127.0.0.1:0\"\n");
    let mut attached = Attached::start(&env, COLLIE);

    let result = hello_result(&mut attached);

    assert_eq!(result["console_link"], Value::Null, "{result}");
    let (_, ended) = attached.finish();
    assert!(ended);
}

/// A run's link to its page in the Console names `console.public_url`, the
/// address the person set, and holds neither the Daemon's token nor a
/// sign-in's cookie, so a forwarded link grants nothing.
///
/// Covers: story:12-9, setting:console.public_url, operation:run.start, operation:run.finish
#[test]
fn a_runs_link_names_the_public_url_and_carries_no_credential() {
    let env = TestEnv::new();
    env.user_configuration(
        "console.address = \"127.0.0.1:0\"\nconsole.public_url = \"https://console.example.test/\"\n",
    );
    let _stops = StopsTheDaemon::of(&env, COLLIE);
    let started = cli(&env, COLLIE)
        .args(["daemon", "start", "--json"])
        .run()
        .unwrap();
    assert_eq!(started.exit, Some(0), "{}", started.stderr);
    let found = RuntimeFile::read(&RuntimeFile::path(env.home(), VERSION)).unwrap();
    let mut client = daemon_client(&found);

    client.call(1, "run.start", &json!({ "run_id": "linked" }));
    let finished = client.call(2, "run.finish", &json!({ "run_id": "linked" }));

    let link = finished["result"]["console_url"].as_str().unwrap();
    assert_eq!(link, "https://console.example.test/runs/linked");
    assert!(!link.contains(&found.token), "{link}");
    assert!(
        !link.contains("token") && !link.contains("goodfetch_console"),
        "{link}"
    );
}
