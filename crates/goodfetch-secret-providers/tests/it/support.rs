//! What every provider's tests share: the runtime they run on, a parsed
//! Secret reference, a settings table, the failure a `secret_unavailable`
//! carries, and the search for canaries outside the stand-ins' stores.

use goodfetch_engine::secrets::SecretError;
use goodfetch_protocol::secret_reference::SecretReference;
use goodfetch_protocol::secrets::SecretFailure;
use goodfetch_test_support::{Canaries, Output, TestEnv};
use serde_json::{Map, Value};

/// A runtime of real time, the driver's timeouts on it: on a paused clock
/// the driver's timeout would fire before a stand-in answers (TESTING.md,
/// Time).
pub(crate) fn runtime() -> tokio::runtime::Runtime {
    goodfetch_runtime::current_thread()
        .enable_all()
        .build()
        .unwrap()
}

pub(crate) fn reference(text: &str) -> SecretReference {
    SecretReference::parse(text).unwrap()
}

/// `value`, a JSON object, as a settings table.
pub(crate) fn table(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        _ => panic!("settings are a table"),
    }
}

/// The failure of a `secret_unavailable`, with its message and fix.
pub(crate) fn unavailable(
    answer: Result<secrecy::SecretString, SecretError>,
) -> (SecretFailure, String) {
    match answer {
        Err(error @ SecretError::Unavailable { .. }) => {
            let message = error.to_string();
            match error {
                SecretError::Unavailable { failure, fix, .. } => {
                    (failure, format!("{message} {fix}"))
                }
                _ => unreachable!(),
            }
        }
        other => panic!("expected `secret_unavailable`, found {:?}", other.err()),
    }
}

/// Every one of `canaries` seen outside the stand-ins' stores: `env`'s
/// home, with its Audit log, and `held`, the test's other outputs.
pub(crate) fn leaks(
    canaries: &Canaries,
    env: &TestEnv,
    held: &[(&'static str, &[u8])],
) -> Vec<String> {
    let mut outputs = vec![Output::Path(env.home())];
    outputs.extend(
        held.iter()
            .map(|(name, bytes)| Output::Bytes { name, bytes }),
    );
    canaries
        .leaks(&outputs)
        .unwrap()
        .into_iter()
        .map(|hit| format!("{hit:?}"))
        .collect()
}
