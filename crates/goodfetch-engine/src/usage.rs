//! The `usage.*` operations every Product carries (ADR 0041): `usage.list`
//! reads every Limit of the user's Usage ledger, by scope, with the Usage
//! charged and held against it in its current period; `usage.set` adds a
//! Limit of the caller's own on its own work, which holds beside the others
//! so that it can only lower what applies; `usage.raise`, a person's
//! answer that the dispatch refuses with `person_required` to anything but
//! the Kennel and the CLI's interactive prompt, sets a Limit in place of
//! the ones of its scope, Usage kind, currency and period. A value only
//! Foxhound carries, such as the Usage kind `paid_fetch`, is refused by a
//! Collie engine with `not_in_product` (ADRs 0087 and 0095). The Limits a
//! program gives a Run at its start are checked the same way, then given
//! to the Run's own scope, lowered and never raised by a later start that
//! names it (ADR 0102).

use std::sync::Arc;

use goodfetch_protocol::catalog::{Product, SharedType};
use goodfetch_protocol::connection::NOT_IN_PRODUCT;
use goodfetch_protocol::entries::product_of_value;
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::run::LimitSetter;
use goodfetch_protocol::run::{LimitScope, Period, RunId, RunLimit};
use goodfetch_protocol::usage::{
    CONNECTION, DAEMON, INSTALL, LIMIT_RAISE_REFUSED, Limit, LimitRaiseRefusedDetails, LimitUsage,
    PERSON, RUN, USAGE_LIST, USAGE_RAISE, USAGE_SET, UsageListParams, UsageListResult,
    UsageRaiseParams, UsageSetParams,
};
use jiff::tz::TimeZone;

use crate::call_error::CallError;
use crate::composition::{CallContext, Handler};
use crate::engine_context::CALLER;
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

/// The Limits a start gives its Run, each checked as `usage.set` checks a
/// Client's Limit, at most one per Usage kind and currency.
pub(crate) fn run_limits(
    given: Vec<RunLimit>,
    context: &CallContext,
) -> Result<Vec<RunLimit>, CallError> {
    for (index, limit) in given.iter().enumerate() {
        known(
            RUN,
            &limit.usage_kind,
            limit.currency.as_deref(),
            None,
            context,
        )?;
        let twice = given[..index].iter().any(|earlier| {
            earlier.usage_kind == limit.usage_kind && earlier.currency == limit.currency
        });
        if twice {
            return Err(CallError::protocol(
                ProtocolFailure::InvalidParams,
                format!(
                    "`limits` gives `{}`{} twice: give each Usage kind and currency once.",
                    limit.usage_kind,
                    limit
                        .currency
                        .as_deref()
                        .map(|currency| format!(" in {currency}"))
                        .unwrap_or_default()
                ),
            ));
        }
    }
    Ok(given)
}

/// Refuses with `limit_raise_refused` a start whose Limits on the Run
/// `run_id` raise one as it stands, writing nothing.
pub(crate) async fn check_run_limits(
    run_id: &RunId,
    given: &[RunLimit],
    context: &CallContext,
) -> Result<(), CallError> {
    if given.is_empty() {
        return Ok(());
    }
    let engine = context.engine().clone();
    let (run_id, given) = (run_id.as_str().to_owned(), given.to_vec());
    let owner = Arc::clone(engine.owner());
    owner
        .blocking(move || engine.ledger().check_run_limits(&run_id, &given))
        .await
        .map_err(|error| unavailable(&error))?
        .map_err(|refused| raise_refused(&refused))
}

/// Gives the Run `run_id` the Limits `given` at a start that `began` it or
/// named it again (ADR 0102).
pub(crate) async fn give_run_limits(
    run_id: &RunId,
    given: Vec<RunLimit>,
    began: bool,
    context: &CallContext,
) -> Result<(), CallError> {
    if given.is_empty() {
        return Ok(());
    }
    let engine = context.engine().clone();
    let run_id = run_id.as_str().to_owned();
    let owner = Arc::clone(engine.owner());
    owner
        .blocking(move || {
            engine
                .ledger()
                .give_run_limits(&run_id, &given, began, engine.now())
        })
        .await
        .map_err(|error| unavailable(&error))?
        .map_err(|refused| raise_refused(&refused))
}

/// `limit_raise_refused`, naming the Limit given and the one standing.
fn raise_refused(refused: &LimitRaiseRefusedDetails) -> CallError {
    let currency = refused
        .currency
        .as_deref()
        .map(|currency| format!(" in {currency}"))
        .unwrap_or_default();
    CallError::new(
        &LIMIT_RAISE_REFUSED,
        format!(
            "The Run's Limit on {}{currency} stands at {}, below the {} given: a program only lowers a Limit; ask a person to raise it.",
            refused.usage_kind, refused.standing, refused.given
        ),
        refused,
    )
}

/// The Limit `named` asks for, set by `set_by` on the caller's own work: a
/// scope ID other than the caller's own is refused, since a Client sets
/// Limits only on the work they cover (ADR 0041).
fn limit_of(named: Named, set_by: LimitSetter, context: &CallContext) -> Result<Limit, CallError> {
    known(
        &named.scope,
        &named.usage_kind,
        named.currency.as_deref(),
        named.period.as_ref(),
        context,
    )?;
    let identity = context.identity();
    let own = match named.scope.as_str() {
        INSTALL => Some(String::new()),
        PERSON => Some(CALLER.to_owned()),
        DAEMON => Some(identity.product_version.line().to_string()),
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

/// Refuses a Limit whose scope kind or Usage kind this engine does not
/// know (`invalid_params`) or its Product does not carry
/// (`not_in_product`), whose currency is no ISO 4217 code, or whose
/// period's time zone is no IANA one this engine knows.
fn known(
    scope: &str,
    usage_kind: &str,
    currency: Option<&str>,
    period: Option<&Period>,
    context: &CallContext,
) -> Result<(), CallError> {
    let identity = context.identity();
    for (shared_type, name) in [
        (SharedType::LimitScope, scope),
        (SharedType::UsageKind, usage_kind),
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
    if let Some(currency) = currency
        && !(currency.len() == 3 && currency.bytes().all(|byte| byte.is_ascii_uppercase()))
    {
        return Err(CallError::protocol(
            ProtocolFailure::InvalidParams,
            format!("`{currency}` is no ISO 4217 currency code."),
        ));
    }
    if let Some(period) = period
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
    Ok(())
}
