//! Caller secret resolution through `goodfetch-collie` (`ConnectOptions::resolve_secret()`
//! and `Collie::secret_requests()`, ADRs 0067, 0092 and 0104; issue 1281):
//! - Without `ConnectOptions::resolve_secret()`, a `caller:<name>` Secret reference
//!   fails with `secret_unavailable` whose `fix` names `ConnectOptions::resolve_secret()`.
//! - With `ConnectOptions::resolve_secret()`, `collie.secret_requests()` receives the
//!   `secrets.resolve` request and answers it so the Run starts and finishes cleanly.

use goodfetch_collie::{Collie, ErrorCode};
use goodfetch_protocol::secrets::{SecretFailure, SecretUnavailableDetails};
use goodfetch_test_support::{TestEnv, real_time_runtime};

use super::options;

/// Without `ConnectOptions::resolve_secret()`, receiving `secrets.resolve`
/// surfaces `secret_unavailable` naming the option (issue 1281).
/// Covers: story:11-98, error:secret_unavailable
#[test]
fn without_resolve_secret_a_caller_reference_fails_with_secret_unavailable_naming_the_option() {
    let env = TestEnv::new();
    env.user_configuration("run_state.key_reference = \"caller:run_key\"\n");

    let err = real_time_runtime().block_on(async {
        let collie = Collie::connect(options(&env)).await.unwrap();
        let err = collie.start().await.unwrap_err();
        collie.close().await.unwrap();
        err
    });

    assert_eq!(err.code(), ErrorCode::SECRET_UNAVAILABLE);
    let details: SecretUnavailableDetails = err.details().expect("secret_unavailable details");
    assert_eq!(details.provider, "caller");
    assert_eq!(details.failure, SecretFailure::ResolverFailed);
    assert!(
        details.fix.contains("ConnectOptions::resolve_secret()"),
        "expected fix to name ConnectOptions::resolve_secret(), got: {}",
        details.fix
    );
}

/// With `ConnectOptions::resolve_secret()`, `Collie::secret_requests()` receives
/// `secrets.resolve` and answers it for `caller:<name>` (issue 1281).
/// Covers: story:11-98
#[test]
fn connect_options_resolve_secret_answers_caller_references_on_linked_collie() {
    let env = TestEnv::new();
    env.user_configuration("run_state.key_reference = \"caller:run_key\"\n");
    let fake_key = data_encoding::BASE64.encode(&[0x5a_u8; 32]);

    let seen = real_time_runtime().block_on(async {
        let collie = Collie::connect(options(&env).resolve_secret())
            .await
            .unwrap();
        let mut requests = collie.secret_requests().unwrap();

        let resolver = async {
            let mut names = Vec::new();
            while let Some(item) = requests.next().await {
                let req = item.unwrap();
                names.push(req.name().to_owned());
                req.answer(&fake_key).unwrap();
            }
            names
        };

        let caller = async {
            let run = collie.start().await.unwrap();
            run.finish().await.unwrap();
            collie.close().await.unwrap();
        };

        let (names, ()) = tokio::join!(resolver, caller);
        names
    });

    assert_eq!(seen, vec!["run_key".to_owned()]);
}
