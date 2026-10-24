//! The endpoint rule (ADRs 0017, 0049, 0050 and 0067; spec #564's
//! `endpoint`) through Collie's executable: the reader of the Audit log that
//! every listener's endpoint-table test checks its refusals with, each
//! listener's loopback default, the regression tests of the advisories whose
//! bypasses came from trusting loopback, and the Secret canary that finds an
//! endpoint token in no output.
#![expect(
    clippy::disallowed_methods,
    reason = "the tests read the Daemon's files and wait for its Audit log from their own threads"
)]

use std::collections::BTreeMap;
use std::time::Instant;

use goodfetch_test_support::{
    AuditUnreadable, Canaries, ListenerKind, MESSAGE_DEADLINE, Rule, SecretKind, TestEnv,
};
use serde_json::{Value, json};

use crate::lasting_sign_ins::audit_entries;

/// The reason the Daemon's socket, an Attached engine's private endpoint
/// and the ops listener record `rule`'s refusal with: a listener keeps no
/// token it no longer takes, so an expired, revoked, out-of-scope or other
/// listener's token is a wrong one to it, and a token it cannot read where
/// it takes one is a missing one.
pub(crate) fn reason(rule: Rule) -> Option<&'static str> {
    Some(match rule {
        Rule::TokenMissing | Rule::TokenInQueryString | Rule::OriginMissingWithoutHeaderToken => {
            "token_missing"
        }
        Rule::TokenWrong
        | Rule::TokenFromAnotherListener
        | Rule::TokenExpired
        | Rule::TokenRevoked
        | Rule::TokenScopedElsewhere => "token_wrong",
        Rule::HostForeign => "host_foreign",
        Rule::OriginForeign => "origin_foreign",
        Rule::HelloNotFirst => "hello_not_first",
        Rule::OriginMissing
        | Rule::ProbesAnswerStatusOnly
        | Rule::CsrfTokenMissing
        | Rule::SignInRevoked
        | Rule::TokenForAnotherFile
        | Rule::SignInRateExceeded => return None,
    })
}

/// How many of the Audit log's entries of `env`'s home record `listener`'s
/// refusal for `reason`.
fn refusals(env: &TestEnv, listener: &str, reason: &str) -> usize {
    audit_entries(env)
        .iter()
        .filter(|entry: &&Value| {
            entry["kind"] == json!("endpoint_authentication_failed")
                && entry["details"] == json!({ "listener": listener, "reason": reason })
        })
        .count()
}

/// The verifier of `listener`'s endpoint-table run: each refused row holds
/// one more `endpoint_authentication_failed` entry of the Audit log of
/// `env`'s home, `{listener, reason}`, than before it, waited for until
/// `MESSAGE_DEADLINE`.
pub(crate) fn audit_verifier<'env>(
    env: &'env TestEnv,
    listener: &'static str,
) -> impl FnMut(ListenerKind, Rule) -> Result<bool, AuditUnreadable> + 'env {
    let mut seen: BTreeMap<&'static str, usize> = BTreeMap::new();
    move |_, rule| {
        let Some(reason) = reason(rule) else {
            return Err(AuditUnreadable::new(format!(
                "{rule:?} is no refusal of {listener}"
            )));
        };
        let earlier = seen.get(reason).copied().unwrap_or(0);
        let deadline = Instant::now() + MESSAGE_DEADLINE;
        loop {
            let now = refusals(env, listener, reason);
            if now > earlier || Instant::now() > deadline {
                seen.insert(reason, now);
                return Ok(now > earlier);
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
}

/// Writes a new ops token of `canaries` to the ops token's file of `env`'s
/// home, as an operator provisions it before the Daemon starts (ADR 0050),
/// and returns it.
pub(crate) fn provision_ops_token(env: &TestEnv, canaries: &mut Canaries) -> String {
    let token = canaries.plant(SecretKind::EndpointToken).value().to_owned();
    let folder = env.home().join("0.0");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("ops.token"), &token).unwrap();
    token
}
