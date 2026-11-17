//! The ops listener of an Attached engine that runs its Runs itself, at
//! `ops.attached_address` (ADR 0050): its probes, and `/metrics` behind the
//! ops token of the major's `ops.token`, the engine's Counted work among
//! them. Unset, the engine keeps no meter for one: it records on the no-op
//! meter, as it does without the setting.

use std::net::SocketAddr;
use std::process::ExitCode;
use std::sync::Arc;

use goodfetch_engine::Engine;
use goodfetch_engine::ops::OpsOptions;
use goodfetch_engine::settings::Resolved;
use goodfetch_engine::transport::listen_ops;
use goodfetch_protocol::ops::ATTACHED_ADDRESS;
use goodfetch_protocol::settings::SETTING_INVALID;

use super::daemon::Machine;
use super::metrics::Export;

/// What an Attached engine serves its ops listener with.
pub(super) struct AttachedOps {
    address: SocketAddr,
    token: String,
    export: Export,
}

impl AttachedOps {
    /// The ops listener `settings` ask for, if any: its address, the ops
    /// token of `machine`'s major, read or made, and the Prometheus
    /// export.
    ///
    /// # Errors
    ///
    /// The exit code of an engine whose `ops.attached_address` is no
    /// loopback `host:port`, or whose ops token or export cannot be made,
    /// each logged.
    pub(super) fn asked(
        settings: Option<&Resolved>,
        machine: &Machine<'_>,
    ) -> Result<Option<Self>, ExitCode> {
        let Some(address) =
            settings.and_then(|settings| settings.get::<Option<&str>>(&ATTACHED_ADDRESS))
        else {
            return Ok(None);
        };
        let address = address
            .parse::<SocketAddr>()
            .ok()
            .filter(|address| address.ip().is_loopback())
            .ok_or_else(|| {
                tracing::error!(
                    code = SETTING_INVALID.name,
                    key = ATTACHED_ADDRESS.name,
                    address,
                    "`ops.attached_address` is no loopback `host:port`, such as `127.0.0.1:9465`"
                );
                ExitCode::FAILURE
            })?;
        let token = machine.files().ops_token().map_err(|error| {
            tracing::error!(%error, "the ops token cannot be read or made");
            ExitCode::FAILURE
        })?;
        let export = Export::start(true, None, machine.identity.product).map_err(|error| {
            tracing::error!(code = SETTING_INVALID.name, %error, "the metrics' export cannot start");
            ExitCode::FAILURE
        })?;
        Ok(Some(Self {
            address,
            token,
            export,
        }))
    }

    /// `engine` recording on this export's meter.
    pub(super) fn engine(&self, engine: &Engine) -> Engine {
        engine.clone().with_metrics(self.export.metrics())
    }

    /// The ops listener, listening.
    ///
    /// # Errors
    ///
    /// Why it cannot listen at its address.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future before it resolves leaves no listener.
    pub(super) async fn listen(&self) -> Result<OpsOptions, String> {
        let listener = listen_ops(self.address).await.map_err(|error| {
            format!(
                "the Attached engine cannot listen at `ops.attached_address`, {}: {error}",
                self.address
            )
        })?;
        let scrape = self
            .export
            .scrape()
            .expect("invariant: an export started for Prometheus has its registry");
        tracing::info!(address = %self.address, "the Attached engine's ops listener serves");
        Ok(OpsOptions {
            listener,
            token: self.token.clone(),
            scrape: Arc::new(scrape),
        })
    }
}
