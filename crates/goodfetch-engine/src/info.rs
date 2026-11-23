//! The engine's own operations, `engine.*`, `run.*`, `config.*`, `daemon.*`,
//! `key_value_store.*` with its table in every run file, `effect.open` and
//! `effect.close` with the Effect journal's tables and the Run report's
//! uncertain Effects, `download.save_as`
//! with the tables of a Run's kept files, `audit.*`, `certs.*`,
//! `needs_you.*` with the storage entries of "Needs you", the streaming
//! calls `events.subscribe` and `page.body`, and the switchable controls,
//! the `security.*` keys and `egress.private_ranges`, which every Product's
//! composition lists first.

use std::sync::Arc;

use goodfetch_protocol::audit::PRIVATE_ADDRESSES_ALLOWED;
use goodfetch_protocol::catalog::PROTOCOL_VERSION;
use goodfetch_protocol::egress::EGRESS_PRIVATE_RANGES;
use goodfetch_protocol::engine::{ENGINE_INFO, EngineInfo, EngineInfoParams};
use goodfetch_protocol::security::SECURITY_CONTROLS;

use crate::call_error::CallError;
use crate::composition::{CallContext, Contribution, Handler};
use crate::effect::Effects;
use crate::engine_context::CALLER;
use crate::{
    audit_operations, certs_operations, config, daemon, effect, key_value_store, needs_you,
    needs_you_operations, robots_txt, rpc, run, run_files, run_files_operations,
    signing_key_operations, usage,
};

/// What the engine adds to every composition.
pub fn contribution() -> Contribution {
    contribution_sharing(&Effects::default())
}

/// What the engine adds to every composition, its declared Effects kept in
/// `effects`, the send window its Runs' commands mark (ADR 0106).
pub(crate) fn contribution_sharing(effects: &Effects) -> Contribution {
    let contribution = run::handlers()
        .into_iter()
        .chain(config::handlers())
        .chain(daemon::handlers())
        .chain(rpc::handlers())
        .chain(usage::handlers())
        .chain(key_value_store::handlers())
        .chain(effect::handlers(effects))
        .chain(run_files_operations::handlers())
        .chain(audit_operations::handlers())
        .chain(certs_operations::handlers())
        .chain(needs_you_operations::handlers())
        .chain(signing_key_operations::handlers())
        .fold(
            Contribution::new("goodfetch-engine")
                .with_handler(Handler::new(&ENGINE_INFO, engine_info))
                .with_run_state_tables(key_value_store::TABLES)
                .with_run_state_tables(run_files::TABLES)
                .with_run_state_tables(robots_txt::TABLES)
                .with_run_state_tables(effect::TABLES)
                .with_report_part(Arc::new(effect::Report))
                .with_send_window(Arc::new(effects.clone()))
                .with_needs_you_source(needs_you::storage::levels())
                .with_needs_you_source(needs_you::storage::runs_stopped())
                .with_needs_you_source(needs_you::limits::runs_stopped())
                .with_needs_you_source(needs_you::consent::walls())
                .with_needs_you_source(needs_you::refetch::stops())
                .with_needs_you_source(needs_you::certificates::changed(CALLER)),
            Contribution::with_handler,
        );
    SECURITY_CONTROLS
        .into_iter()
        .fold(contribution, Contribution::with_control)
        .with_recorded_listing(&EGRESS_PRIVATE_RANGES, &PRIVATE_ADDRESSES_ALLOWED, "ranges")
}

async fn engine_info(_: EngineInfoParams, context: CallContext) -> Result<EngineInfo, CallError> {
    let identity = context.identity();
    Ok(EngineInfo {
        product: identity.product,
        product_version: identity.product_version.clone(),
        protocol_version: PROTOCOL_VERSION,
        product_major: identity.product_version.major(),
    })
}
