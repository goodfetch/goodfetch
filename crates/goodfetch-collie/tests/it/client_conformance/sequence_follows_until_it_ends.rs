//! A sequence's command run with `--follow` prints what the call sends as
//! it arrives, one JSON object per line under `--json`, and ends when the
//! sequence ends (ADR 0099): `page body` sends a stored body in chunks of
//! 64 KiB, one per credit the CLI grants, then answers.

use goodfetch_test_support::{TestEnv, cli};
use serde_json::json;

use crate::client_conformance::COLLIE;
use crate::commands::{start, store_body};

/// A body of 150 KiB: three chunks.
const SIZE: usize = 150 * 1024;

/// Covers: story:11-198, command:page.body
#[test]
fn attached() {
    let env = TestEnv::new();
    start(&env, "first");
    let body = store_body(&env, "first", &vec![b'x'; SIZE]);

    let checked = cli(&env, COLLIE)
        .args([
            "page", "body", "--run-id", "first", "--body", &body, "--follow", "--json",
        ])
        .json_lines("Chunk")
        .unwrap();

    let offsets: Vec<_> = checked
        .documents
        .iter()
        .map(|chunk| chunk["offset"].clone())
        .collect();
    assert_eq!(
        (checked.output.exit, offsets),
        (Some(0), vec![json!(0), json!(65_536), json!(131_072)])
    );
}
