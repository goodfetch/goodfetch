//! Handlers for `identity.list`, `identity.import`, `identity.delete`,
//! `identity.export`, and `identity.export_storage_state` (ADRs 0015, 0035,
//! 0056 and 0099; ticket #736).

use goodfetch_engine::{CallContext, Contribution, Handler};
use goodfetch_protocol::page::identity::{
    IDENTITY_DELETE, IDENTITY_EXPORT, IDENTITY_EXPORT_STORAGE_STATE, IDENTITY_IMPORT,
    IDENTITY_LIST, IdentityDeleteParams, IdentityExportParams, IdentityExportStorageStateParams,
    IdentityImportParams, IdentityListParams,
};

use super::in_call;
use crate::cdp::launch::Launcher;
use crate::saved_identity;

/// Adds the `identity.*` handlers to `contribution`.
pub(super) fn with_identity_handlers(
    contribution: Contribution,
    launcher: Launcher,
) -> Contribution {
    contribution
        .with_handler(Handler::new(
            &IDENTITY_LIST,
            |_params: IdentityListParams, context: CallContext| {
                in_call((IDENTITY_LIST.name, context.form()), async move {
                    saved_identity::list(&context).await
                })
            },
        ))
        .with_handler(Handler::new(
            &IDENTITY_IMPORT,
            move |params: IdentityImportParams, context: CallContext| {
                let launcher = launcher.clone();
                in_call((IDENTITY_IMPORT.name, context.form()), async move {
                    saved_identity::import_identity(params, &launcher, &context).await
                })
            },
        ))
        .with_handler(Handler::new(
            &IDENTITY_DELETE,
            |params: IdentityDeleteParams, context: CallContext| {
                in_call((IDENTITY_DELETE.name, context.form()), async move {
                    saved_identity::delete(params, &context).await
                })
            },
        ))
        .with_handler(Handler::new(
            &IDENTITY_EXPORT,
            |params: IdentityExportParams, context: CallContext| {
                in_call((IDENTITY_EXPORT.name, context.form()), async move {
                    saved_identity::export_bundle(params, &context).await
                })
            },
        ))
        .with_handler(Handler::new(
            &IDENTITY_EXPORT_STORAGE_STATE,
            |params: IdentityExportStorageStateParams, context: CallContext| {
                in_call(
                    (IDENTITY_EXPORT_STORAGE_STATE.name, context.form()),
                    async move { saved_identity::export_storage_state(params, &context).await },
                )
            },
        ))
}
