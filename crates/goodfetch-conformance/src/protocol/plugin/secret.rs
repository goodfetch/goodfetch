//! The Secret provider's cases (`conformance/secret/cases.toml`): the
//! reference each case sends with `secret.resolve`, and the checks of what an
//! implementation answers for it.

use goodfetch_protocol::plugin::secret::{SECRET_RESOLVE, SecretResolveResult};
use goodfetch_protocol::result_document::CaseCheck;
use serde::Deserialize as _;
use serde_json::{Value, json};

/// The method every case calls.
pub(super) const METHOD: &str = SECRET_RESOLVE;

/// The reference sent to `secret.resolve`.
const REFERENCE: &str = "plugin:conformance:item";

/// The params of `secret.resolve`, with a field a newer engine adds when
/// `newer`.
pub(super) fn params(newer: bool) -> Value {
    let mut params = json!({ "reference": REFERENCE });
    if newer {
        params["added_in_a_minor"] = json!(1);
    }
    params
}

/// The checks of `answer`, what an implementation answered for
/// `secret.resolve`.
pub(super) fn checks(answer: &Value) -> Vec<CaseCheck> {
    let result = answer
        .get("result")
        .and_then(|result| SecretResolveResult::deserialize(result).ok());
    vec![
        CaseCheck {
            name: "the answer is `{ value }` and no other field".to_owned(),
            passed: result.is_some(),
        },
        CaseCheck {
            name: "the resolved value is non-empty".to_owned(),
            passed: result
                .as_ref()
                .is_some_and(|result| !result.value.is_empty()),
        },
    ]
}
