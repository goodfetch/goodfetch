//! The `hello` a test's Client says first (ADRs 0030 and 0095), built from
//! `goodfetch-protocol`'s `HelloParams`, `PROTOCOL_VERSION` and method name,
//! so no test spells the protocol version or a field's name: a protocol
//! bump changes them here, at compile time.

use goodfetch_protocol::catalog::{PROTOCOL_VERSION, Product, ProtocolVersion};
use goodfetch_protocol::connection::{HELLO, HelloParams, Surface};
use serde_json::{Value, json};

use crate::home::product_version;

/// A `hello` request: the protocol version this commit speaks, the Python
/// SDK's surface, no token, the commit's Product version and the id `0`,
/// unless changed.
#[derive(Clone, Debug, PartialEq)]
pub struct Hello {
    id: Value,
    params: HelloParams,
}

impl Hello {
    /// A `hello` of a Client that needs `needs`.
    pub fn needing(needs: Product) -> Self {
        Self {
            id: json!(0),
            params: HelloParams {
                protocol_version: PROTOCOL_VERSION,
                surface: Surface::SdkPython,
                form: None,
                token: None,
                needs: Some(needs),
                product_version: product_version(),
                log_level: None,
                client_surface: None,
                keep_daemon_alive: false,
            },
        }
    }

    /// The same `hello` under the request id `id`.
    #[must_use]
    pub fn id(mut self, id: impl Into<Value>) -> Self {
        self.id = id.into();
        self
    }

    /// The same `hello` from `surface`.
    #[must_use]
    pub fn surface(mut self, surface: Surface) -> Self {
        self.params.surface = surface;
        self
    }

    /// The same `hello` carrying the Daemon's or a private endpoint's
    /// token `token`.
    #[must_use]
    pub fn token(mut self, token: impl Into<String>) -> Self {
        self.params.token = Some(token.into());
        self
    }

    /// The same `hello` speaking `version`, such as a newer minor than the
    /// engine's.
    #[must_use]
    pub fn protocol_version(mut self, version: ProtocolVersion) -> Self {
        self.params.protocol_version = version;
        self
    }

    /// The same `hello` with its params changed by `change`, for a field
    /// the other methods leave alone.
    #[must_use]
    pub fn with(mut self, change: impl FnOnce(&mut HelloParams)) -> Self {
        change(&mut self.params);
        self
    }

    /// Its params, as a call's `params`.
    pub fn params(&self) -> Value {
        serde_json::to_value(&self.params).expect("invariant: a hello's params serialize")
    }

    /// The JSON-RPC request.
    pub fn message(&self) -> Value {
        json!({ "jsonrpc": "2.0", "id": self.id, "method": HELLO, "params": self.params() })
    }
}

/// The JSON-RPC `hello` request of a Client that needs `needs`, with
/// [`Hello::needing`]'s defaults.
pub fn hello(needs: Product) -> Value {
    Hello::needing(needs).message()
}

/// The protocol version one minor above the one this commit speaks, which
/// an engine refuses.
pub fn newer_minor() -> ProtocolVersion {
    ProtocolVersion {
        minor: PROTOCOL_VERSION.minor + 1,
        ..PROTOCOL_VERSION
    }
}

#[cfg(test)]
mod tests;
