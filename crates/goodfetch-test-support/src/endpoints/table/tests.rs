use rstest::rstest;

use super::{EndpointTable, ListenerKind, Rule, TableError};

/// A table whose rows give every listener a column.
const GOOD: &str = r#"
[rule.token_missing]
summary = "A first message or request without a token."
adrs = [17]
listeners.daemon_socket = { expect = "closed" }
listeners.private_endpoint = { expect = "closed" }
listeners.ops_listener = { expect = "status", status = 401 }
listeners.daemon_websocket = { expect = "closed" }
listeners.exposed_daemon = { expect = "closed" }
listeners.console = { expect = "status", status = 401 }
listeners.content_origin = { expect = "status", status = 401 }
listeners.proxy_relay = { expect = "challenge" }
"#;

#[test]
fn the_committed_table_passes_its_check() {
    let table = EndpointTable::parse(super::COMMITTED);

    assert!(table.is_ok(), "{table:?}");
}

#[test]
fn a_table_giving_every_listener_a_row_passes() {
    let table = EndpointTable::parse(GOOD);

    assert!(table.is_ok(), "{table:?}");
}

#[test]
fn a_row_that_applies_to_no_listener_fails() {
    let source = format!("{GOOD}\n[rule.token_wrong]\nsummary = \"A wrong token.\"\nadrs = [17]\n");

    let table = EndpointTable::parse(&source);

    assert_eq!(
        table.unwrap_err(),
        TableError::RowWithoutListener {
            rule: Rule::TokenWrong
        }
    );
}

#[test]
fn a_listener_column_with_no_row_fails() {
    let source = r#"
[rule.token_missing]
summary = "A first message or request without a token."
adrs = [17]
listeners.daemon_socket = { expect = "closed" }
listeners.private_endpoint = { expect = "closed" }
"#;

    let table = EndpointTable::parse(source);

    assert_eq!(
        table.unwrap_err(),
        TableError::ListenerWithoutRow {
            listener: ListenerKind::OpsListener
        }
    );
}

#[rstest]
#[case::a_closed_connection_on_http(
    "token_wrong",
    "ops_listener",
    r#"{ expect = "closed" }"#,
    Rule::TokenWrong,
    ListenerKind::OpsListener
)]
#[case::a_status_on_a_json_rpc_stream(
    "token_wrong",
    "daemon_socket",
    r#"{ expect = "status", status = 401 }"#,
    Rule::TokenWrong,
    ListenerKind::DaemonSocket
)]
#[case::a_status_that_is_no_refusal(
    "token_wrong",
    "ops_listener",
    r#"{ expect = "status", status = 200 }"#,
    Rule::TokenWrong,
    ListenerKind::OpsListener
)]
#[case::a_header_rule_on_a_json_rpc_stream(
    "host_foreign",
    "daemon_socket",
    r#"{ expect = "closed" }"#,
    Rule::HostForeign,
    ListenerKind::DaemonSocket
)]
#[case::a_status_for_a_websockets_first_message(
    "token_wrong",
    "daemon_websocket",
    r#"{ expect = "status", status = 401 }"#,
    Rule::TokenWrong,
    ListenerKind::DaemonWebsocket
)]
#[case::a_closed_connection_for_a_websockets_handshake(
    "origin_foreign",
    "daemon_websocket",
    r#"{ expect = "closed" }"#,
    Rule::OriginForeign,
    ListenerKind::DaemonWebsocket
)]
#[case::a_missing_origin_on_a_json_rpc_stream(
    "origin_missing",
    "exposed_daemon",
    r#"{ expect = "closed" }"#,
    Rule::OriginMissing,
    ListenerKind::ExposedDaemon
)]
#[case::a_closed_connection_for_a_sign_in_cookie(
    "token_wrong",
    "console",
    r#"{ expect = "closed" }"#,
    Rule::TokenWrong,
    ListenerKind::Console
)]
#[case::a_status_for_a_signed_in_websockets_first_message(
    "hello_not_first",
    "console",
    r#"{ expect = "status", status = 400 }"#,
    Rule::HelloNotFirst,
    ListenerKind::Console
)]
#[case::a_refusal_rule_answering_with_a_status_only(
    "token_wrong",
    "ops_listener",
    r#"{ expect = "status_only" }"#,
    Rule::TokenWrong,
    ListenerKind::OpsListener
)]
#[case::a_probe_rule_refusing_with_a_status(
    "probes_answer_status_only",
    "ops_listener",
    r#"{ expect = "status", status = 404 }"#,
    Rule::ProbesAnswerStatusOnly,
    ListenerKind::OpsListener
)]
#[case::a_sign_in_route_rule_on_a_listener_without_sign_in_routes(
    "sign_in_rate_exceeded",
    "daemon_websocket",
    r#"{ expect = "status", status = 429 }"#,
    Rule::SignInRateExceeded,
    ListenerKind::DaemonWebsocket
)]
#[case::a_closed_connection_for_a_sign_in_route(
    "csrf_token_missing",
    "console",
    r#"{ expect = "closed" }"#,
    Rule::CsrfTokenMissing,
    ListenerKind::Console
)]
#[case::a_revoked_sign_in_on_a_listener_without_sign_ins(
    "sign_in_revoked",
    "ops_listener",
    r#"{ expect = "status", status = 401 }"#,
    Rule::SignInRevoked,
    ListenerKind::OpsListener
)]
#[case::a_closed_connection_for_a_signed_address(
    "token_wrong",
    "content_origin",
    r#"{ expect = "closed" }"#,
    Rule::TokenWrong,
    ListenerKind::ContentOrigin
)]
#[case::a_first_message_rule_on_a_signed_address(
    "hello_not_first",
    "content_origin",
    r#"{ expect = "status", status = 400 }"#,
    Rule::HelloNotFirst,
    ListenerKind::ContentOrigin
)]
#[case::a_query_token_rule_on_a_signed_address(
    "token_in_query_string",
    "content_origin",
    r#"{ expect = "status", status = 401 }"#,
    Rule::TokenInQueryString,
    ListenerKind::ContentOrigin
)]
#[case::another_files_signature_on_a_listener_without_files(
    "token_for_another_file",
    "ops_listener",
    r#"{ expect = "status", status = 401 }"#,
    Rule::TokenForAnotherFile,
    ListenerKind::OpsListener
)]
#[case::an_accepted_attempt_that_must_be_refused(
    "token_wrong",
    "ops_listener",
    r#"{ expect = "accepted" }"#,
    Rule::TokenWrong,
    ListenerKind::OpsListener
)]
#[case::an_accepted_missing_origin_on_a_websocket(
    "origin_missing",
    "daemon_websocket",
    r#"{ expect = "accepted" }"#,
    Rule::OriginMissing,
    ListenerKind::DaemonWebsocket
)]
#[case::a_cookie_token_rule_on_a_json_rpc_stream(
    "origin_missing_without_header_token",
    "daemon_socket",
    r#"{ expect = "closed" }"#,
    Rule::OriginMissingWithoutHeaderToken,
    ListenerKind::DaemonSocket
)]
#[case::a_challenge_from_a_listener_that_is_no_proxy(
    "token_wrong",
    "ops_listener",
    r#"{ expect = "challenge" }"#,
    Rule::TokenWrong,
    ListenerKind::OpsListener
)]
#[case::a_challenge_for_a_wrong_token(
    "token_wrong",
    "proxy_relay",
    r#"{ expect = "challenge" }"#,
    Rule::TokenWrong,
    ListenerKind::ProxyRelay
)]
#[case::a_closed_connection_from_a_proxy(
    "token_wrong",
    "proxy_relay",
    r#"{ expect = "closed" }"#,
    Rule::TokenWrong,
    ListenerKind::ProxyRelay
)]
#[case::a_header_rule_on_a_proxy(
    "host_foreign",
    "proxy_relay",
    r#"{ expect = "status", status = 403 }"#,
    Rule::HostForeign,
    ListenerKind::ProxyRelay
)]
fn a_cell_its_listener_cannot_try_fails(
    #[case] rule_name: &str,
    #[case] listener_name: &str,
    #[case] cell: &str,
    #[case] rule: Rule,
    #[case] listener: ListenerKind,
) {
    let source = format!(
        "{GOOD}\n[rule.{rule_name}]\nsummary = \"A row.\"\nadrs = [17]\nlisteners.{listener_name} = {cell}\n"
    );

    let table = EndpointTable::parse(&source);

    assert_eq!(table.unwrap_err(), TableError::CellUnfit { rule, listener });
}

#[test]
fn a_rule_the_runner_does_not_know_fails_to_parse() {
    let source = format!(
        "{GOOD}\n[rule.token_stolen]\nsummary = \"A row.\"\nadrs = [17]\nlisteners.daemon_socket = {{ expect = \"closed\" }}\n"
    );

    let table = EndpointTable::parse(&source);

    assert!(
        matches!(table, Err(TableError::Unreadable { .. })),
        "{table:?}"
    );
}

/// The committed JSON Schema of the table's format.
fn committed_schema() -> serde_json::Value {
    serde_json::from_str(include_str!(
        "../../../../../test-support/fixtures/schemas/endpoint-table.schema.json"
    ))
    .unwrap()
}

#[test]
fn the_committed_table_meets_its_committed_schema() {
    let table: serde_json::Value = toml::from_str(super::COMMITTED).unwrap();

    let errors: Vec<String> = jsonschema::validator_for(&committed_schema())
        .unwrap()
        .iter_errors(&table)
        .map(|error| error.to_string())
        .collect();

    assert_eq!(errors, Vec::<String>::new());
}

#[test]
fn the_committed_schema_refuses_a_listener_the_table_has_no_kind_for() {
    let table: serde_json::Value =
        toml::from_str(&GOOD.replace("daemon_socket", "mcp_listener")).unwrap();

    let valid = jsonschema::validator_for(&committed_schema())
        .unwrap()
        .is_valid(&table);

    assert!(!valid);
}
