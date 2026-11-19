//! A deprecated command warns on stderr with ADR 0030's text, which names
//! its replacement, and leaves stdout to its output (ADRs 0030 and 0099):
//! `sample echo`, deprecated since the protocol's first minor.

use goodfetch_test_support::{TestEnv, assert_cli_snapshot, cli};

use crate::client_conformance::COLLIE;

/// Covers: story:11-204, command:sample.echo
#[test]
fn attached() {
    let env = TestEnv::new();

    let checked = cli(&env, COLLIE)
        .args(["sample", "echo", "--text", "hello", "--json"])
        .json("SampleEchoResult")
        .unwrap();

    assert_cli_snapshot!(checked.output, @r#"
    exit: 0
    ----- stdout -----
    {
      "text": "hello"
    }
    ----- stderr -----
    warning: `sample echo` is deprecated. Since 0.1, removed no earlier than 1.0: A sample entry, which the real catalog replaces.
    [INSTANT] TRACE goodfetch::rpc::dispatch: request answered method="hello"
    [INSTANT] TRACE goodfetch::rpc::dispatch: request answered method="sample.echo"
    "#);
}
