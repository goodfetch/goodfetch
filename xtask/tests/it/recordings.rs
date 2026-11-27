//! The Model recordings check (ADR 0065): every committed recording keeps
//! only the allowlisted headers, carries no key in its query, no request or
//! organization ID the recorder should have replaced and no string shaped
//! like a provider key, and every cell of the matrix has its recording or
//! says why it waits.
//!
//! Each case copies the committed matrix, points one cell at a clean
//! recording and breaks one thing. No case recording sits in a
//! `recordings/` folder of the repository, which the check itself and the
//! Proving ground's shape test read, and the key-shaped strings are built
//! here, so no committed file holds one.

use crate::workspace::{Workspace, finding};

const MATRIX: &str = "test-support/fixtures/model-recordings.toml";

const RECORDING: &str = "crates/adapter/tests/recordings/text.json";

const WAITING_CELL: &str = "[openai-compatible.direct]\ntext = { waits = \"recorder\" }";

const RECORDED_CELL: &str = "[openai-compatible.direct]\ntext = { recording = \"crates/adapter/tests/recordings/text.json\", via = \"vercel-ai-gateway\" }";

/// The committed matrix with its first cell recorded, and the clean
/// recording that cell names.
fn workspace() -> Workspace {
    let workspace = Workspace::from_fixture("empty");
    let committed = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join(MATRIX),
    )
    .unwrap();
    assert_eq!(committed.matches(WAITING_CELL).count(), 1);
    workspace.write(
        MATRIX,
        committed.replace(WAITING_CELL, RECORDED_CELL).as_bytes(),
    );
    workspace.add_fixture_file("model-recordings-cases/clean.json", RECORDING);
    workspace
}

#[test]
fn clean_recordings_and_a_whole_matrix_pass() {
    let workspace = workspace();

    let outcome = workspace.xtask(&["recordings", "check"]);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stderr);
    assert_eq!(outcome.code, 0);
    assert!(
        outcome
            .notes()
            .contains(&finding("waits", "openai-compatible.direct")),
        "{}",
        outcome.stdout
    );
}

#[test]
fn a_request_header_outside_the_allowlist_fails() {
    let workspace = workspace();
    workspace.edit(
        RECORDING,
        r#""headers": { "content-type": "application/json" }"#,
        r#""headers": { "content-type": "application/json", "Authorization": "Bearer <key>" }"#,
    );

    let outcome = workspace.xtask(&["recordings", "check"]);

    assert_eq!(outcome.findings(), vec![finding("header", RECORDING)]);
    assert_eq!(outcome.code, 1);
}

#[test]
fn a_response_header_outside_the_allowlist_fails() {
    let workspace = workspace();
    workspace.edit(
        RECORDING,
        r#""retry-after": "20""#,
        r#""retry-after": "20", "openai-organization": "<organization-id>""#,
    );

    let outcome = workspace.xtask(&["recordings", "check"]);

    assert_eq!(outcome.findings(), vec![finding("header", RECORDING)]);
    assert_eq!(outcome.code, 1);
}

#[test]
fn a_key_query_parameter_fails() {
    let workspace = workspace();
    workspace.edit(
        RECORDING,
        r#""path": "/v1/chat/completions",
        "headers""#,
        r#""path": "/v1/chat/completions?alt=json&api_key=<key>",
        "headers""#,
    );

    let outcome = workspace.xtask(&["recordings", "check"]);

    assert_eq!(
        outcome.findings(),
        vec![finding("key-parameter", RECORDING)]
    );
    assert_eq!(outcome.code, 1);
}

#[test]
fn an_unreplaced_request_id_fails() {
    let workspace = workspace();
    workspace.edit(
        RECORDING,
        r#""request_id": "<request-id>""#,
        r#""request_id": "req_011CSHoEeqs5C35K2UUqR7Fy""#,
    );

    let outcome = workspace.xtask(&["recordings", "check"]);

    assert_eq!(
        outcome.findings(),
        vec![finding("unreplaced-id", RECORDING)]
    );
    assert_eq!(outcome.code, 1);
}

#[test]
fn an_unreplaced_organization_id_fails() {
    let workspace = workspace();
    workspace.edit(
        RECORDING,
        "in organization <organization-id>",
        "in organization org-a1B2c3D4e5F6g7H8i9J0k1L2",
    );

    let outcome = workspace.xtask(&["recordings", "check"]);

    assert_eq!(
        outcome.findings(),
        vec![finding("unreplaced-id", RECORDING)]
    );
    assert_eq!(outcome.code, 1);
}

/// One string of each shape gitleaks gives a provider's key, built here so
/// no committed file holds one.
fn key_shaped() -> [String; 5] {
    [
        format!("sk-proj-{}T3BlbkFJ{}", "a".repeat(74), "b".repeat(74)),
        format!("sk-{}T3BlbkFJ{}", "c".repeat(20), "d".repeat(20)),
        format!("sk-ant-api03-{}AA", "e".repeat(93)),
        format!("sk-ant-admin01-{}AA", "f".repeat(93)),
        format!("AIza{}", "g".repeat(35)),
    ]
}

#[test]
fn a_string_shaped_like_a_provider_key_fails() {
    for key in key_shaped() {
        let workspace = workspace();
        workspace.edit(
            RECORDING,
            r#""content": "Say hello.""#,
            &format!(r#""content": "Say hello. {key}""#),
        );

        let outcome = workspace.xtask(&["recordings", "check"]);

        assert_eq!(
            outcome.findings(),
            vec![finding("key-shaped", RECORDING)],
            "{}",
            &key[..8]
        );
        assert_eq!(outcome.code, 1);
    }
}

#[test]
fn a_matrix_cell_whose_recording_is_missing_fails() {
    let workspace = workspace();
    workspace.remove(RECORDING);

    let outcome = workspace.xtask(&["recordings", "check"]);

    assert_eq!(
        outcome.findings(),
        vec![finding("missing-cell", "openai-compatible.direct.text")]
    );
    assert_eq!(outcome.code, 1);
}

#[test]
fn a_cell_left_out_of_the_matrix_fails() {
    let workspace = workspace();
    workspace.edit(
        MATRIX,
        "[anthropic-messages.direct]\ntext = { waits = \"recorder\" }\n",
        "[anthropic-messages.direct]\n",
    );

    let outcome = workspace.xtask(&["recordings", "check"]);

    assert_eq!(outcome.findings(), vec![finding("matrix", MATRIX)]);
    assert_eq!(outcome.code, 1);
}

#[test]
fn scrub_removes_forbidden_headers_key_params_and_provider_ids_so_check_passes() {
    let workspace = workspace();
    let raw = serde_json::json!({
        "exchanges": [{
            "request": {
                "method": "POST",
                "path": "/v1/chat/completions?alt=json&api_key=not-real",
                "headers": {
                    "Content-Type": "application/json",
                    "Authorization": "Bearer placeholder",
                    "x-api-key": "placeholder",
                    "anthropic-version": "2023-06-01"
                },
                "body": {
                    "model": "openai/gpt-5.4-nano",
                    "messages": [{"role": "user", "content": "Say hello."}]
                }
            },
            "response": {
                "status": 200,
                "headers": {
                    "Content-Type": "application/json",
                    "Retry-After": "10",
                    "openai-organization": "org-a1B2c3D4e5F6g7H8i9J0k1L2",
                    "set-cookie": "session=secret",
                    "x-request-id": "req_011CSHoEeqs5C35K2UUqR7Fy"
                },
                "body": {
                    "id": "chatcmpl-1",
                    "request_id": "req_011CSHoEeqs5C35K2UUqR7Fy",
                    "organization_id": "org-a1B2c3D4e5F6g7H8i9J0k1L2",
                    "note": "served for org-a1B2c3D4e5F6g7H8i9J0k1L2 on req_011CSHoEeqs5C35K2UUqR7Fy"
                }
            }
        }]
    });
    workspace.write("raw-exchange.json", raw.to_string().as_bytes());

    let scrub = workspace.xtask(&[
        "recordings",
        "scrub",
        "--in",
        "raw-exchange.json",
        "--out",
        RECORDING,
    ]);
    assert_eq!(scrub.code, 0, "{}{}", scrub.stdout, scrub.stderr);

    let scrubbed: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(workspace.path().join(RECORDING)).unwrap())
            .unwrap();
    let req = &scrubbed["exchanges"][0]["request"];
    assert_eq!(req["path"], "/v1/chat/completions?alt=json");
    assert!(req["headers"].get("content-type").is_some());
    assert!(req["headers"].get("anthropic-version").is_some());
    assert!(req["headers"].get("Authorization").is_none());
    assert!(req["headers"].get("authorization").is_none());
    assert!(req["headers"].get("x-api-key").is_none());

    let resp = &scrubbed["exchanges"][0]["response"];
    assert!(resp["headers"].get("content-type").is_some());
    assert!(resp["headers"].get("retry-after").is_some());
    assert!(resp["headers"].get("openai-organization").is_none());
    assert!(resp["headers"].get("set-cookie").is_none());
    assert!(resp["headers"].get("x-request-id").is_none());
    assert_eq!(resp["body"]["request_id"], "<request-id>");
    assert_eq!(resp["body"]["organization_id"], "<organization-id>");

    let checked = workspace.xtask(&["recordings", "check"]);
    assert_eq!(checked.findings(), vec![], "{}", checked.stderr);
    assert_eq!(checked.code, 0);
}

#[test]
fn scrub_refuses_when_provider_key_remains_in_body() {
    let workspace = workspace();
    workspace.remove(RECORDING);
    let key = &key_shaped()[0];
    let raw = serde_json::json!({
        "exchanges": [{
            "request": {
                "method": "POST",
                "path": "/v1/chat/completions",
                "headers": { "content-type": "application/json" },
                "body": { "prompt": format!("leaked {key}") }
            },
            "response": {
                "status": 200,
                "headers": { "content-type": "application/json" },
                "body": { "ok": true }
            }
        }]
    });
    workspace.write("raw-leaked.json", raw.to_string().as_bytes());

    let scrub = workspace.xtask(&[
        "recordings",
        "scrub",
        "--in",
        "raw-leaked.json",
        "--out",
        RECORDING,
    ]);
    assert_eq!(scrub.code, 1, "{}{}", scrub.stdout, scrub.stderr);
    assert_eq!(scrub.findings(), vec![finding("key-shaped", RECORDING)]);
    assert!(!workspace.path().join(RECORDING).exists());
}
