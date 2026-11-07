//! The `usage.*` operations every Product carries (ADR 0041): `usage.list`
//! reads every Limit of the user's Usage ledger, by scope, with the Usage
//! charged and held against it in its current period; `usage.set` adds a
//! Limit of the caller's own on its own work, which holds beside the others
//! so that it can only lower what applies; `usage.raise`, a person's
//! answer that the dispatch refuses with `person_required` to anything but
//! the Console and the CLI's interactive prompt, sets a Limit in place of
//! the ones of its scope, Usage kind, currency and period. A value only
//! Foxhound carries, such as the Usage kind `paid_fetch`, is refused by a
//! Collie engine with `not_in_product` (ADRs 0087 and 0095).

use std::sync::Arc;

use goodfetch_protocol::catalog::{Product, SharedType};
use goodfetch_protocol::connection::NOT_IN_PRODUCT;
use goodfetch_protocol::entries::product_of_value;
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::run::LimitSetter;
use goodfetch_protocol::run::{LimitScope, Period};
use goodfetch_protocol::usage::{
    CONNECTION, DAEMON, INSTALL, Limit, LimitUsage, PERSON, USAGE_LIST, USAGE_RAISE, USAGE_SET,
    UsageListParams, UsageListResult, UsageRaiseParams, UsageSetParams,
};
use jiff::tz::TimeZone;

use crate::call_error::CallError;
use crate::composition::{CallContext, Handler};
use crate::engine_context::CALLER;
use crate::home::major_name;
use crate::ledger::LedgerError;
use crate::product::Requested;

/// The `usage.*` handlers.
pub(crate) fn handlers() -> [Handler; 3] {
    [
        Handler::new(&USAGE_LIST, list),
        Handler::new(&USAGE_SET, set),
        Handler::new(&USAGE_RAISE, raise),
    ]
}

fn unavailable(error: &impl std::fmt::Display) -> CallError {
    CallError::protocol(
        ProtocolFailure::InternalError,
        format!("the Usage ledger cannot be read or written: {error}"),
    )
}

async fn list(_: UsageListParams, context: CallContext) -> Result<UsageListResult, CallError> {
    let engine = context.engine().clone();
    let product = context.identity().product;
    let owner = Arc::clone(engine.owner());
    let usage = owner
        .blocking(move || engine.ledger().usage(product, engine.now()))
        .await
        .map_err(|error| unavailable(&error))?;
    Ok(UsageListResult { limits: usage })
}

/// A Limit as `usage.set` and `usage.raise` name it.
struct Named {
    scope: String,
    scope_id: Option<String>,
    usage_kind: String,
    currency: Option<String>,
    period: Option<Period>,
    amount: u64,
}

async fn set(params: UsageSetParams, context: CallContext) -> Result<LimitUsage, CallError> {
    let named = Named {
        scope: params.scope,
        scope_id: params.scope_id,
        usage_kind: params.usage_kind,
        currency: params.currency,
        period: params.period,
        amount: params.amount,
    };
    let limit = limit_of(named, LimitSetter::Client, &context)?;
    write(&context, move |ledger, now| ledger.add_limit(&limit, now)).await
}

async fn raise(params: UsageRaiseParams, context: CallContext) -> Result<LimitUsage, CallError> {
    let named = Named {
        scope: params.scope,
        scope_id: params.scope_id,
        usage_kind: params.usage_kind,
        currency: params.currency,
        period: params.period,
        amount: params.amount,
    };
    let limit = limit_of(named, LimitSetter::Person, &context)?;
    write(&context, move |ledger, now| ledger.raise(&limit, now)).await
}

/// Writes a Limit through `change` and answers it with its Usage.
async fn write<F>(context: &CallContext, change: F) -> Result<LimitUsage, CallError>
where
    F: FnOnce(&crate::ledger::Ledger, jiff::Timestamp) -> Result<i64, LedgerError> + Send + 'static,
{
    let engine = context.engine().clone();
    let product: Product = context.identity().product;
    let owner = Arc::clone(engine.owner());
    let added = owner
        .blocking(move || {
            let now = engine.now();
            let id = change(engine.ledger(), now)?;
            engine.ledger().usage_of(id, product, now)
        })
        .await
        .map_err(|error| unavailable(&error))?;
    added.ok_or_else(|| unavailable(&"the Limit written is gone"))
}

/// The Limit `named` asks for, set by `set_by` on the caller's own work: a
/// scope ID other than the caller's own is refused, since a Client sets
/// Limits only on the work they cover (ADR 0041).
fn limit_of(named: Named, set_by: LimitSetter, context: &CallContext) -> Result<Limit, CallError> {
    let identity = context.identity();
    for (shared_type, name) in [
        (SharedType::LimitScope, &named.scope),
        (SharedType::UsageKind, &named.usage_kind),
    ] {
        match product_of_value(shared_type, name) {
            None => {
                return Err(CallError::protocol(
                    ProtocolFailure::InvalidParams,
                    format!("`{name}` is no {} this engine knows.", shared_type.as_str()),
                ));
            }
            Some(needs) if !identity.product.contains(needs) => {
                let (message, details) =
                    identity.not_in_product(Requested::Name(name), needs, false);
                return Err(CallError::new(&NOT_IN_PRODUCT, message, &details));
            }
            Some(_) => {}
        }
    }
    if let Some(currency) = &named.currency
        && !(currency.len() == 3 && currency.bytes().all(|byte| byte.is_ascii_uppercase()))
    {
        return Err(CallError::protocol(
            ProtocolFailure::InvalidParams,
            format!("`{currency}` is no ISO 4217 currency code."),
        ));
    }
    if let Some(period) = &named.period
        && TimeZone::get(&period.time_zone).is_err()
    {
        return Err(CallError::protocol(
            ProtocolFailure::InvalidParams,
            format!(
                "`{}` is no IANA time zone this engine knows.",
                period.time_zone
            ),
        ));
    }
    let own = match named.scope.as_str() {
        INSTALL => Some(String::new()),
        PERSON => Some(CALLER.to_owned()),
        DAEMON => Some(major_name(&identity.product_version)),
        CONNECTION => Some(context.runs().connection_scope().id),
        _ => None,
    };
    let id = match (named.scope_id, own) {
        (None, Some(own)) => own,
        (Some(id), Some(own)) if id == own => id,
        (Some(id), Some(_)) => {
            return Err(CallError::protocol(
                ProtocolFailure::InvalidParams,
                format!(
                    "A Limit on a `{}` is set on the caller's own, not on `{id}`: leave out `scope_id`.",
                    named.scope
                ),
            ));
        }
        (Some(id), None) => id,
        (None, None) => {
            return Err(CallError::protocol(
                ProtocolFailure::InvalidParams,
                format!(
                    "A Limit on a `{}` names which one in `scope_id`.",
                    named.scope
                ),
            ));
        }
    };
    Ok(Limit {
        scope: LimitScope {
            kind: named.scope,
            id,
        },
        usage_kind: named.usage_kind,
        currency: named.currency,
        period: named.period,
        amount: named.amount,
        must_understand: true,
        set_by,
    })
}
