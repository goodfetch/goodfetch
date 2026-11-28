//! Unit tests for lending handshake validation and command classification
//! (ADR 0089; ticket #749).

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use goodfetch_engine::endpoint::Authorities;
use serde_json::json;
use tokio_tungstenite::tungstenite::http::StatusCode;

use super::{CommandDecision, classify_command, validate_handshake};

fn loopback() -> Authorities {
    Authorities::loopback(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 43210))
}

#[test]
fn handshake_requires_valid_host_origin_and_token() {
    let auth = loopback();
    let token = "deadbeef0123456789abcdef";

    // Valid Host, no Origin (standard library client), valid Bearer token -> Ok
    assert!(
        validate_handshake(
            Some("127.0.0.1:43210"),
            None,
            Some(&format!("Bearer {token}")),
            &auth,
            token
        )
        .is_ok()
    );

    // Valid localhost Host + matching loopback Origin -> Ok
    assert!(
        validate_handshake(
            Some("localhost:43210"),
            Some("http://localhost:43210"),
            Some(&format!("Bearer {token}")),
            &auth,
            token
        )
        .is_ok()
    );

    // Missing Host -> 403
    assert_eq!(
        validate_handshake(None, None, Some(&format!("Bearer {token}")), &auth, token)
            .unwrap_err()
            .0,
        StatusCode::FORBIDDEN
    );

    // Foreign Host (DNS rebinding) -> 403
    assert_eq!(
        validate_handshake(
            Some("evil.example:43210"),
            None,
            Some(&format!("Bearer {token}")),
            &auth,
            token
        )
        .unwrap_err()
        .0,
        StatusCode::FORBIDDEN
    );

    // Foreign Origin -> 403
    assert_eq!(
        validate_handshake(
            Some("127.0.0.1:43210"),
            Some("https://evil.example"),
            Some(&format!("Bearer {token}")),
            &auth,
            token
        )
        .unwrap_err()
        .0,
        StatusCode::FORBIDDEN
    );

    // Missing token -> 401
    assert_eq!(
        validate_handshake(Some("127.0.0.1:43210"), None, None, &auth, token)
            .unwrap_err()
            .0,
        StatusCode::UNAUTHORIZED
    );

    // Wrong token -> 401
    assert_eq!(
        validate_handshake(
            Some("127.0.0.1:43210"),
            None,
            Some("Bearer wrong-token"),
            &auth,
            token
        )
        .unwrap_err()
        .0,
        StatusCode::UNAUTHORIZED
    );
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "tests the complete ADR 0089 command classification table in one case"
)]
fn command_classification_enforces_adr_0089_table() {
    let ctx = "ctx-1";

    // Browser.close ends attach, never closes browser.
    assert_eq!(
        classify_command("Browser.close", None, &json!({}), ctx, false),
        CommandDecision::EndAttach
    );

    // Download behaviour is a no-op.
    assert_eq!(
        classify_command("Browser.setDownloadBehavior", None, &json!({}), ctx, false),
        CommandDecision::NoOp
    );
    assert_eq!(
        classify_command(
            "Page.setDownloadBehavior",
            Some("s1"),
            &json!({}),
            ctx,
            false
        ),
        CommandDecision::NoOp
    );

    // Refused dangerous commands.
    for method in [
        "Target.createBrowserContext",
        "Target.disposeBrowserContext",
        "Target.attachToBrowserTarget",
        "Target.exposeDevToolsProtocol",
        "Extensions.loadUnpacked",
        "Browser.crash",
        "Browser.crashGpuProcess",
        "Security.setIgnoreCertificateErrors",
    ] {
        assert!(
            matches!(
                classify_command(method, None, &json!({}), ctx, false),
                CommandDecision::Refuse(_)
            ),
            "expected {method} to be refused"
        );
    }

    // Context rescoping: omitted or matching ctx -> RescopeContext; another ctx -> Refuse.
    assert_eq!(
        classify_command("Storage.getCookies", None, &json!({}), ctx, false),
        CommandDecision::RescopeContext
    );
    assert_eq!(
        classify_command(
            "Storage.getCookies",
            None,
            &json!({ "browserContextId": "ctx-1" }),
            ctx,
            false
        ),
        CommandDecision::RescopeContext
    );
    assert!(matches!(
        classify_command(
            "Storage.getCookies",
            None,
            &json!({ "browserContextId": "ctx-other" }),
            ctx,
            false
        ),
        CommandDecision::Refuse(_)
    ));
    assert!(matches!(
        classify_command(
            "Target.createTarget",
            None,
            &json!({ "url": "about:blank", "browserContextId": "ctx-other" }),
            ctx,
            false
        ),
        CommandDecision::Refuse(_)
    ));

    // Browser-level fail-closed for unclassed commands.
    assert!(matches!(
        classify_command("Browser.setPermissionUnknown", None, &json!({}), ctx, false),
        CommandDecision::Refuse(_)
    ));
    assert_eq!(
        classify_command("Browser.getVersion", None, &json!({}), ctx, false),
        CommandDecision::Pass
    );

    // Target-level commands pass on verified sessions.
    assert_eq!(
        classify_command("Runtime.enable", Some("s1"), &json!({}), ctx, false),
        CommandDecision::Pass
    );

    // Shared-host restrictions (#1377).
    assert!(matches!(
        classify_command("DOM.setFileInputFiles", Some("s1"), &json!({}), ctx, true),
        CommandDecision::Refuse(_)
    ));
    assert!(matches!(
        classify_command(
            "Page.navigate",
            Some("s1"),
            &json!({ "url": "file:///etc/passwd" }),
            ctx,
            true
        ),
        CommandDecision::Refuse(_)
    ));
    assert_eq!(
        classify_command(
            "Page.navigate",
            Some("s1"),
            &json!({ "url": "https://example.test/" }),
            ctx,
            true
        ),
        CommandDecision::Pass
    );
}
