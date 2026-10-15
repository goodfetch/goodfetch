//! Collie's blocking connection (ADR 0081), with `goodfetch_collie_client::blocking`'s values.

pub use goodfetch_collie_client::blocking::*;

use goodfetch_collie_client::{ConnectOptions, EngineInfo, Error, ExplainedSetting, Identities, SettingKey};

/// [`crate::Collie`], blocking.
#[derive(Debug, Clone)]
pub struct Collie {
    _private: (),
}

impl Collie {
    /// Starts or finds the engine and completes `hello` before returning.
    pub fn connect(options: ConnectOptions) -> Result<Self, Error> {
        let _ = options;
        unimplemented!()
    }

    /// The engine this connection reached.
    pub fn engine(&self) -> &EngineInfo {
        unimplemented!()
    }

    /// Starts a Run, as [`crate::Collie::start`] does.
    pub fn start(&self) -> Start {
        unimplemented!()
    }

    /// The raw catalog call on the connection; [`RawCall::send`] sends it.
    pub fn call(&self, operation: &str) -> RawCall {
        let _ = operation;
        unimplemented!()
    }

    /// The connection's effective settings, with the layer that set each.
    pub fn explain_settings(&self, keys: &[SettingKey]) -> Call<Vec<ExplainedSetting>> {
        let _ = keys;
        unimplemented!()
    }

    /// Closes the connection for every clone. Idempotent.
    pub fn close(&self) -> Call<()> {
        unimplemented!()
    }
}

impl Connection for Collie {
    fn engine(&self) -> &EngineInfo {
        Collie::engine(self)
    }

    fn identities(&self) -> Identities {
        unimplemented!()
    }

    fn call(&self, operation: &str) -> RawCall {
        Collie::call(self, operation)
    }

    fn explain_settings(&self, keys: &[SettingKey]) -> Call<Vec<ExplainedSetting>> {
        Collie::explain_settings(self, keys)
    }

    fn close(&self) -> Call<()> {
        Collie::close(self)
    }

    fn start(&self) -> Start {
        Collie::start(self)
    }
}
