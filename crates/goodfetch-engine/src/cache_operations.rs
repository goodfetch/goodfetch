//! The `cache.*` operations both Products carry (ADR 0046): `cache.stats`
//! counts the caller's entries in the Response cache of the engine's
//! major, `cache.clear` deletes them, all or by domain, Run or partition,
//! and `cache.verify` lists those whose body file is missing or was
//! changed. Another person's entries on a shared Daemon are never counted,
//! deleted or read. A cache of another format version than the engine
//! reads is refused with `format_version_unsupported` (ADR 0066).

use goodfetch_protocol::formats::RESPONSE_CACHE;
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::response_cache::{
    CACHE_CLEAR, CACHE_STATS, CACHE_VERIFY, CacheClearParams, CacheCleared, CacheStats,
    CacheStatsParams, CacheVerified, CacheVerifyParams,
};

use crate::call_error::CallError;
use crate::composition::{CallContext, Handler};
use crate::engine_context::CALLER;
use crate::host_name;
use crate::response_cache::{Clearing, ResponseCacheError};
use crate::run::format_refusal;

/// The `cache.*` handlers.
pub(crate) fn handlers() -> [Handler; 3] {
    [
        Handler::new(&CACHE_STATS, stats),
        Handler::new(&CACHE_CLEAR, clear),
        Handler::new(&CACHE_VERIFY, verify),
    ]
}

async fn stats(_: CacheStatsParams, context: CallContext) -> Result<CacheStats, CallError> {
    context
        .engine()
        .response_cache()
        .stats(CALLER)
        .await
        .map_err(|error| refusal(error, "read"))
}

async fn clear(params: CacheClearParams, context: CallContext) -> Result<CacheCleared, CallError> {
    let domain = params
        .domain
        .map(|domain| {
            host_name::key(&domain).ok_or_else(|| {
                CallError::protocol(
                    ProtocolFailure::InvalidParams,
                    format!("`{domain}` names no host"),
                )
            })
        })
        .transpose()?;
    let clearing = Clearing {
        person: Some(CALLER.to_owned()),
        domain,
        run_id: params.run_id,
        partition: params.partition,
        country: params.country,
        language: params.language,
        device: params.device,
    };
    context
        .engine()
        .response_cache()
        .clear(clearing)
        .await
        .map_err(|error| refusal(error, "written"))
}

async fn verify(_: CacheVerifyParams, context: CallContext) -> Result<CacheVerified, CallError> {
    context
        .engine()
        .response_cache()
        .verify(CALLER)
        .await
        .map_err(|error| refusal(error, "read"))
}

/// The failure a call answers when the cache could not be `done`.
fn refusal(error: ResponseCacheError, done: &str) -> CallError {
    match error {
        ResponseCacheError::Format {
            found,
            compatibility,
        } => format_refusal(&RESPONSE_CACHE, &found, compatibility),
        other => CallError::protocol(
            ProtocolFailure::InternalError,
            format!("the Response cache cannot be {done}: {other}"),
        ),
    }
}
