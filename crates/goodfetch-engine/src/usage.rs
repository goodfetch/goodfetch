//! The `usage.*` operations every Product carries (ADR 0041): `usage.list`
//! reads every Limit of the user's Usage ledger, by scope, with the Usage
//! charged and held against it in its current period; `usage.set` adds a
//! Limit of the caller's own, which holds beside the others so that it can
//! only lower what applies. A value only Foxhound carries, such as the
//! Usage kind `paid_fetch`, is refused by a Collie engine with
//! `not_in_product` (ADRs 0087 and 0095); raising a Limit is a person's
//! answer, which this module does not offer.

use std::sync::Arc;

use goodfetch_protocol::catalog::{Product, SharedType};
use goodfetch_protocol::connection::NOT_IN_PRODUCT;
use goodfetch_protocol::entries::product_of_value;
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::run::LimitScope;
use goodfetch_protocol::usage::{
    CONNECTION, DAEMON, INSTALL, Limit, LimitSetter, LimitUsage, PERSON, USAGE_LIST, USAGE_SET,
    UsageListParams, UsageListResult, UsageSetParams,
};
use jiff::tz::TimeZone;

use crate::composition::{CallContext, CallError, Handler};
use crate::home::major_name;
use crate::product::Requested;
use crate::runs::CALLER;

/// The `usage.*` handlers.
pub(crate) fn handlers() -> [Handler; 2] {
    [
        Handler::new(&USAGE_LIST, list),
        Handler::new(&USAGE_SET, set),
    ]
}

fn unavailable(error: &impl std::fmt::Display) -> CallError {
    CallError::protocol(
        ProtocolFailure::InternalError,
        format!("the Usage ledger cannot be read or written: {error}"),
    )
}

async fn list(_: UsageListParams, context: CallContext) -> Result<UsageListResult, CallError> {
    let limits = context.runs().limits().clone();
    let product = context.identity().product;
    let owner = Arc::clone(&limits.owner);
    let usage = owner
        .blocking(move || limits.ledger.usage(product, limits.now()))
        .await
        .map_err(|error| unavailable(&error))?;
    Ok(UsageListResult { limits: usage })
}

async fn set(params: UsageSetParams, context: CallContext) -> Result<LimitUsage, CallError> {
    let identity = context.identity();
    for (shared_type, name) in [
        (SharedType::LimitScope, &params.scope),
        (SharedType::UsageKind, &params.usage_kind),
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
    if let Some(currency) = &params.currency
        && !(currency.len() == 3 && currency.bytes().all(|byte| byte.is_ascii_uppercase()))
    {
        return Err(CallError::protocol(
            ProtocolFailure::InvalidParams,
            format!("`{currency}` is no ISO 4217 currency code."),
        ));
    }
    if let Some(period) = &params.period
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
    let runs = context.runs();
    let limits = runs.limits().clone();
    let id = match (params.scope_id.clone(), params.scope.as_str()) {
        (Some(id), _) => id,
        (None, INSTALL) => String::new(),
        (None, PERSON) => CALLER.to_owned(),
        (None, DAEMON) => major_name(&identity.product_version),
        (None, CONNECTION) => limits.connection(runs.connection()).id,
        (None, scope) => {
            return Err(CallError::protocol(
                ProtocolFailure::InvalidParams,
                format!("A Limit on a `{scope}` names which one in `scope_id`."),
            ));
        }
    };
    let limit = Limit {
        scope: LimitScope {
            kind: params.scope,
            id,
        },
        usage_kind: params.usage_kind,
        currency: params.currency,
        period: params.period,
        amount: params.amount,
        must_understand: true,
        set_by: LimitSetter::Client,
    };
    let product: Product = identity.product;
    let owner = Arc::clone(&limits.owner);
    let added = owner
        .blocking(move || {
            let now = limits.now();
            let id = limits.ledger.add_limit(&limit, now)?;
            limits.ledger.usage_of(id, product, now)
        })
        .await
        .map_err(|error| unavailable(&error))?;
    added.ok_or_else(|| unavailable(&"the Limit added is gone"))
}
