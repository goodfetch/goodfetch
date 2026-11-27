//! `plugin.search` and `plugin.index_import` RPC handlers (ADR 0033,
//! issue 919).
//!
//! - `plugin.search` (a read, every surface) queries the local cached Plugin
//!   index, returning each matching Plugin's kinds, signer identity and
//!   conformance results.
//! - `plugin.index_import` (person-only) verifies a signed Plugin index
//!   offline against the engine's trust root (bundled in production, injected
//!   in tests, ADR 0059), persists it, and records `plugin_index_imported` in
//!   the Audit log.

use goodfetch_protocol::audit::Actor;
use goodfetch_protocol::plugin::index::{
    PLUGIN_INDEX_IMPORT, PLUGIN_SEARCH, PluginIndexImportParams, PluginIndexImported,
    PluginSearchParams, PluginSearchResult,
};

use crate::audit;
use crate::call_error::CallError;
use crate::composition::{CallContext, Handler};
use crate::plugin_index::{self, IndexError, PluginIndexStore};

/// The `plugin.search` and `plugin.index_import` handlers.
pub(crate) fn handlers() -> [Handler; 2] {
    [
        Handler::new(&PLUGIN_SEARCH, search_operation),
        Handler::new(&PLUGIN_INDEX_IMPORT, import_operation),
    ]
}

async fn search_operation(
    params: PluginSearchParams,
    context: CallContext,
) -> Result<PluginSearchResult, CallError> {
    let engine = context.engine();
    let now = engine.clock().timestamp();
    let folder = engine.home().plugin_index();
    let blocking = engine.owner().blocking_threads();
    let status = blocking
        .run(move || PluginIndexStore::new(folder).status(now))
        .await
        .map_err(IndexError::into_call_error)?;
    Ok(plugin_index::search_status(&status, &params))
}

async fn import_operation(
    params: PluginIndexImportParams,
    context: CallContext,
) -> Result<PluginIndexImported, CallError> {
    let engine = context.engine();
    let now = engine.clock().timestamp();
    let folder = engine.home().plugin_index();
    let trust = engine.plugin_index_trust_root().clone();
    let blocking = engine.owner().blocking_threads();
    let bundle = params.bundle;
    let applied = blocking
        .run(move || PluginIndexStore::new(folder).apply(bundle, &trust, now))
        .await
        .map_err(IndexError::into_call_error)?;
    let origin = audit::Origin {
        actor: Actor::Person {
            name: context.person(),
        },
        surface: Some(context.surface()),
        layer: None,
        run_id: None,
    };
    plugin_index::record_import(engine.audit(), &origin, &applied, now).await
}
