//! A Plugin's calls back into the engine (ADR 0033): `net.fetch`, an HTTP
//! request the engine sends for the Plugin under its own checks, and the
//! Error codes of a call back the engine refuses and of a `net.fetch` that
//! got no response.
//!
//! A Plugin calls back only while it serves a call the engine made to it,
//! naming that call's `call_id`; the call back acts within that call's
//! run, lease and Session. The engine sends the request without the
//! Session's cookies, follows no redirect (a redirect is answered, and the
//! Plugin asks for its location as a new `net.fetch`), and checks each
//! destination under the egress class ADR 0033 gives it.

use std::borrow::Cow;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};

use crate::catalog::{Entries, ErrorCodeDefinition, ErrorGroup, ProtocolVersion};
use crate::jsonrpc::RequestId;

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// The Plugin's method that asks the engine to send an HTTP request.
pub const NET_FETCH: &str = "net.fetch";

/// What `net.fetch` asks the engine to send.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NetFetchParams {
    /// The id of the engine's call the Plugin is serving.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub call_id: RequestId,
    /// The HTTP method, such as `GET`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub method: String,
    /// The absolute URL, `http` or `https`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub url: String,
    /// The request's headers, in order. The engine sets `Host`,
    /// `Content-Length` and the connection's own headers itself and
    /// refuses a request that sets them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub headers: Vec<NetFetchHeader>,
    /// The request's body, when it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub body: Option<NetFetchBody>,
}

/// One header of a `net.fetch` request or response.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NetFetchHeader {
    /// Its name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub name: String,
    /// Its value; in a response, a value that is no UTF-8 is read with each
    /// invalid sequence replaced.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub value: String,
}

/// What the engine answers to `net.fetch`: the response it received, a
/// redirect included, never followed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NetFetchResult {
    /// The response's status code.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub status: u16,
    /// The response's headers, in order.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub headers: Vec<NetFetchHeader>,
    /// The response's body, whole.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub body: NetFetchBody,
}

/// The bytes of a `net.fetch` body: on the wire, in Base64 (RFC 4648, with
/// padding).
#[derive(Clone, Default, PartialEq, Eq)]
pub struct NetFetchBody(Vec<u8>);

/// Why a text is no body: it is no Base64 with padding.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("a body is its bytes in Base64 (RFC 4648, with padding)")]
pub struct NetFetchBodyInvalid;

impl NetFetchBody {
    /// The body `bytes`.
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    /// Reads `text`, bytes in Base64.
    ///
    /// # Errors
    ///
    /// When `text` is no Base64 with padding.
    pub fn from_base64(text: &str) -> Result<Self, NetFetchBodyInvalid> {
        data_encoding::BASE64
            .decode(text.as_bytes())
            .map(Self)
            .map_err(|_| NetFetchBodyInvalid)
    }

    /// Its bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.0
    }

    /// Its bytes, owned.
    pub fn into_bytes(self) -> Vec<u8> {
        self.0
    }
}

impl std::fmt::Debug for NetFetchBody {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "NetFetchBody({} bytes)", self.0.len())
    }
}

impl Serialize for NetFetchBody {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&data_encoding::BASE64.encode(&self.0))
    }
}

impl<'de> Deserialize<'de> for NetFetchBody {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = Cow::<str>::deserialize(deserializer)?;
        Self::from_base64(&text).map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for NetFetchBody {
    fn schema_name() -> Cow<'static, str> {
        "NetFetchBody".into()
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "The bytes of a `net.fetch` body, in Base64 (RFC 4648, with padding).",
            "type": "string",
            "contentEncoding": "base64",
        })
    }
}

/// A call back the engine refused (ADR 0033).
pub const PLUGIN_CALL_BACK_REFUSED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "plugin_call_back_refused",
    description: "The engine refused a Plugin's call back: it came while the Plugin served no call the engine made to it, it named another call's `call_id`, or it asked for what the Plugin may not do, such as a `net.fetch` other than `GET` or `HEAD` toward a destination checked as a page's, which is an Effect, or a request that sets a header the engine sets itself. Call back only while serving a call, with that call's `call_id`.",
    group: ErrorGroup::Refused,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// A `net.fetch` the engine sent that got no response.
pub const PLUGIN_FETCH_FAILED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "plugin_fetch_failed",
    description: "The engine sent a Plugin's `net.fetch` and got no response: its host had no address, no connection to it could be opened, or the connection broke before the whole response arrived. Nothing about the destination's egress check failed; ask again later, or check that the host is up and reachable from the engine's machine.",
    group: ErrorGroup::Unavailable,
    retryable: true,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// The Error codes of a call back.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[],
    events: &[],
    handler_requests: &[],
    error_codes: &[PLUGIN_CALL_BACK_REFUSED, PLUGIN_FETCH_FAILED],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[],
    hook_stages: &[],
    shared_type_values: &[],
};

#[cfg(test)]
mod tests {
    use rstest::rstest;
    use serde_json::{Value, json};

    use super::NetFetchBody;

    /// A body travels as Base64 with padding, and any other text is no body.
    #[rstest]
    #[case::empty(b"".as_slice(), "")]
    #[case::text(b"Dune".as_slice(), "RHVuZQ==")]
    #[case::binary(&[0, 255, 128], "AP+A")]
    fn a_body_travels_as_base64(#[case] bytes: &[u8], #[case] wire: &str) {
        let body = NetFetchBody::new(bytes.to_vec());

        assert_eq!(serde_json::to_value(&body).ok(), Some(json!(wire)));
        assert_eq!(
            serde_json::from_value::<NetFetchBody>(json!(wire)).ok(),
            Some(body)
        );
    }

    #[rstest]
    #[case::unpadded(json!("RHVuZQ"))]
    #[case::url_alphabet(json!("AP-A"))]
    #[case::not_text(json!([1, 2]))]
    fn a_body_that_is_no_base64_is_refused(#[case] wire: Value) {
        assert!(serde_json::from_value::<NetFetchBody>(wire).is_err());
    }
}
