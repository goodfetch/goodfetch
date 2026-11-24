//! The shapes the sensor service answers in: Hyper Solutions' Akamai sensor
//! request and response and its error codes, as it documents them
//! (docs.hypersolutions.co).

use hyper::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::Failed;

/// The path of the sensor endpoint.
pub(super) const SENSOR: &str = "/v2/sensor";

/// The header that carries an account's key.
pub(super) const KEY_HEADER: &str = "x-api-key";

/// What an answered sensor costs, in dollars: the stand-in's own price, as
/// the service publishes none.
pub(super) const PRICE: &str = "0.0010";

/// The client hints of the browser a sensor is made for.
const SEC_CH_UA: &str =
    "\"Chromium\";v=\"140\", \"Not=A?Brand\";v=\"24\", \"Google Chrome\";v=\"140\"";

/// A sensor service.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Service {
    HyperSolutions,
}

/// The host name the service answers on.
pub(super) const HOST: &str = "hyper-solutions.test";

/// The body of a sensor call.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Sensor {
    pub(super) abck: String,
    pub(super) bmsz: String,
    pub(super) version: String,
    pub(super) page_url: String,
    pub(super) user_agent: String,
    /// Where the script is served, the client's address and its languages:
    /// asked of every call, read by none.
    #[serde(rename = "scriptUrl")]
    _script_url: String,
    #[serde(rename = "ip")]
    _ip: String,
    #[serde(rename = "acceptLanguage")]
    _accept_language: String,
    script: Option<String>,
    script_hash: Option<String>,
    context: Option<String>,
}

/// How a sensor call names its page's script.
pub(super) enum Named {
    /// By its text.
    Script(String),
    /// By its SHA-256, in lowercase hex.
    Hash(String),
    /// By the context of an earlier call.
    Context(String),
}

impl Sensor {
    /// How the call names its page's script, or why it does not.
    pub(super) fn named(&mut self) -> Result<Named, Failed> {
        match (
            self.script.take(),
            self.script_hash.take(),
            self.context.take(),
        ) {
            (Some(_), Some(_), _) | (_, Some(_), Some(_)) => {
                Err(invalid("scriptHash goes with no script and no context"))
            }
            (None, Some(hash), None) if !lowercase_sha256(&hash) => {
                Err(invalid("scriptHash is not a lowercase SHA-256"))
            }
            (None, Some(hash), None) => Ok(Named::Hash(hash)),
            (_, None, Some(context)) => Ok(Named::Context(context)),
            (Some(script), None, None) => Ok(Named::Script(script)),
            (None, None, None) => Err(script_required()),
        }
    }
}

fn lowercase_sha256(text: &str) -> bool {
    text.len() == 64
        && text
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

/// Whether `key` is written as an account's is: a UUID.
pub(super) fn is_key(key: &str) -> bool {
    key.len() == 36
        && key.bytes().enumerate().all(|(at, byte)| match at {
            8 | 13 | 18 | 23 => byte == b'-',
            _ => byte.is_ascii_hexdigit(),
        })
}

/// The error `code`, with the status the service documents for it; a code it
/// does not is a failure of its own, `500`.
pub(super) fn fail(code: &str) -> Failed {
    let status = match code {
        "missing_api_key" | "malformed_api_key" | "invalid_api_key" => StatusCode::UNAUTHORIZED,
        "no_plan_or_balance" | "plan_exhausted" | "plan_expired" | "balance_exhausted"
        | "balance_expired" => StatusCode::PAYMENT_REQUIRED,
        "api_key_disabled"
        | "product_not_enabled"
        | "no_domains_whitelisted"
        | "domain_not_whitelisted"
        | "access_denied" => StatusCode::FORBIDDEN,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    Failed {
        status,
        body: json!({
            "error": description(code),
            "code": code,
            "docs": format!("https://docs.hypersolutions.co/api-reference/errors#{code}"),
        }),
    }
}

/// What the error `code` says.
fn description(code: &str) -> &'static str {
    match code {
        "missing_api_key" => "The call has no x-api-key header.",
        "malformed_api_key" => "The key is not a UUID.",
        "invalid_api_key" => "The key is no account's.",
        "no_plan_or_balance" => "The account has no active plan and no balance.",
        "plan_exhausted" => "The plan's requests are used up.",
        "plan_expired" => "The plan has expired.",
        "balance_exhausted" => "The balance is used up.",
        "balance_expired" => "The balance has expired.",
        "api_key_disabled" => "The key is disabled.",
        "product_not_enabled" => "The account has no access to this product.",
        "no_domains_whitelisted" => "The key has no domain whitelisted.",
        "domain_not_whitelisted" => "The page's domain is not whitelisted for this key.",
        "access_denied" => "The call was denied.",
        _ => "The service failed.",
    }
}

/// The error of a call the endpoint does not read, which has no code.
pub(super) fn invalid(reason: &str) -> Failed {
    Failed {
        status: StatusCode::BAD_REQUEST,
        body: json!({ "error": reason }),
    }
}

/// The error of a call that names no script the service has.
pub(super) fn script_required() -> Failed {
    Failed {
        status: StatusCode::UNPROCESSABLE_ENTITY,
        body: json!({ "error": "script_required" }),
    }
}

/// The body of an answered sensor call.
pub(super) fn answered(payload: &str, context: &str) -> Value {
    json!({
        "payload": payload,
        "context": context,
        "headers": { "sec-ch-ua": SEC_CH_UA },
    })
}
