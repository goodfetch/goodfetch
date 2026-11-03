//! The Needs you items in the menu (ADR 0141): each item a submenu that
//! says what waits and offers the actions the item allows that the menu
//! can take, each through the operation it calls for.
//!
//! - **Raise the Limit** (`usage.raise`): to twice what the Limit allows
//!   now, the one amount a menu can offer without asking for a number.
//! - **Accept the consent walls** of a domain
//!   (`needs_you.allow_consent_domain`).
//! - **Deny** a domain (`needs_you.deny_wall_domain`), with the reason the
//!   Audit log records: that a person denied it from the Kennel.
//! - **Remove** a domain from the refetch domains
//!   (`needs_you.remove_refetch_domain`), unless only `*` lists it, which
//!   takes a list of domains to replace `*` that a menu cannot ask for.
//! - **Dismiss** (`needs_you.dismiss`).
//!
//! The other actions an item may list (`run.resume`, `certs.accept`,
//! `certs.forget`) are not offered here.

use std::collections::BTreeMap;

use goodfetch_protocol::needs_you::{
    CONSENT_WALL, NEEDS_YOU_ALLOW_CONSENT_DOMAIN, NEEDS_YOU_DENY_WALL_DOMAIN, NEEDS_YOU_DISMISS,
    NEEDS_YOU_REMOVE_REFETCH_DOMAIN, NeedsYouAllowConsentDomainParams,
    NeedsYouDenyWallDomainParams, NeedsYouDismissParams, NeedsYouItem,
    NeedsYouRemoveRefetchDomainParams, RUN_STOPPED_BY_LIMIT, WALL_REFETCH_STOPPED,
};
use goodfetch_protocol::run::ReachedLimit;
use goodfetch_protocol::usage::{USAGE_RAISE, UsageRaiseParams};
use serde_json::Value;

use super::{Choice, Row};
use crate::client::Call;
use crate::text;

/// The most Needs you items the menu lists; the others are counted.
const MENU_ITEMS: usize = 10;

/// The most characters of an item's line, which opens its submenu.
const ITEM_MOST: usize = 60;

/// The reason a denial from the Kennel gives, which the Audit log records.
pub(super) const DENIAL_REASON: &str = "Denied by a person from the Kennel's menu";

/// An action that resolves a Needs you item.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Action {
    /// Raise the Limit that stopped a Run.
    RaiseLimit,
    /// Accept the consent walls of the item's domain.
    AllowConsentDomain,
    /// Deny the item's domain.
    DenyWallDomain,
    /// Remove the item's domain from the refetch domains.
    RemoveRefetchDomain,
    /// Dismiss the item.
    Dismiss,
}

/// Adds the menu's rows of `items`, longest waiting first, each with the
/// actions it allows when `controls` is on.
pub(super) fn rows(items: &BTreeMap<String, NeedsYouItem>, controls: bool, rows: &mut Vec<Row>) {
    if items.is_empty() {
        return;
    }
    let mut listed: Vec<&NeedsYouItem> = items.values().collect();
    listed.sort_by(|one, other| (&one.since, &one.id).cmp(&(&other.since, &other.id)));
    let more = listed.len().saturating_sub(MENU_ITEMS);
    listed.truncate(MENU_ITEMS);
    rows.push(Row::Separator);
    rows.push(Row::Text("Needs you".to_owned()));
    for item in listed {
        let mut inner = vec![Row::Text(item.summary.clone())];
        if controls {
            inner.extend(
                resolutions(item)
                    .into_iter()
                    .map(|(action, label, _)| Row::Choice {
                        label,
                        choice: Choice::NeedsYou {
                            item: item.id.clone(),
                            action,
                        },
                    }),
            );
        }
        rows.push(Row::Submenu {
            label: text::shown(&label(item), ITEM_MOST),
            rows: inner,
        });
    }
    if more > 0 {
        rows.push(Row::Text(format!(
            "{more} more {}",
            if more == 1 { "item" } else { "items" }
        )));
    }
}

/// The call that resolves `item` through `action`, if the item allows it.
pub(super) fn call(item: &NeedsYouItem, action: Action) -> Option<Call> {
    resolutions(item)
        .into_iter()
        .find(|(offered, _, _)| *offered == action)
        .map(|(_, _, call)| call)
}

/// What the line that opens an item's submenu says: a few words of the
/// kinds the Kennel acts on, the engine's summary for the others.
fn label(item: &NeedsYouItem) -> String {
    let domain = detail(item, "domain");
    match (item.kind.as_str(), domain) {
        (CONSENT_WALL, Some(domain)) => format!("Consent wall on {domain}"),
        (WALL_REFETCH_STOPPED, Some(domain)) => format!("Refetch stopped on {domain}"),
        (RUN_STOPPED_BY_LIMIT, _) => match (stopped_run(item), reached(item)) {
            (Some(run), Some(limit)) => {
                format!("{run} stopped by a Limit on {}", limit.usage_kind)
            }
            _ => item.summary.clone(),
        },
        _ => item.summary.clone(),
    }
}

/// The Recipe or the Run ID of the Run a Limit stopped.
fn stopped_run(item: &NeedsYouItem) -> Option<String> {
    detail(item, "recipe")
        .map(str::to_owned)
        .or_else(|| item.run.as_ref().map(|run| run.as_str().to_owned()))
}

/// The actions of `item` the menu offers, in the item's order, each with
/// its label and its call.
fn resolutions(item: &NeedsYouItem) -> Vec<(Action, String, Call)> {
    item.actions
        .iter()
        .filter_map(|name| resolution(item, name))
        .collect()
}

/// The action the operation `name` is for `item`, if the menu offers it.
fn resolution(item: &NeedsYouItem, name: &str) -> Option<(Action, String, Call)> {
    let domain = detail(item, "domain").map(str::to_owned);
    if name == USAGE_RAISE.name {
        let limit = reached(item)?;
        let now = item.details.get("amount").and_then(Value::as_u64)?;
        // From nothing, twice is nothing: the amount is the person's to
        // choose, in the CLI.
        let amount = now.checked_mul(2).filter(|amount| *amount > 0)?;
        let label = format!(
            "Raise the Limit to {}",
            shown_amount(amount, limit.currency.as_deref())
        );
        let call = Call::RaiseLimit(UsageRaiseParams {
            scope: limit.scope.kind,
            scope_id: Some(limit.scope.id),
            usage_kind: limit.usage_kind,
            currency: limit.currency,
            period: limit.period,
            amount,
        });
        Some((Action::RaiseLimit, label, call))
    } else if name == NEEDS_YOU_ALLOW_CONSENT_DOMAIN.name {
        let domain = domain?;
        let label = format!("Accept the consent walls of {domain}");
        let call = Call::AllowConsentDomain(NeedsYouAllowConsentDomainParams { domain });
        Some((Action::AllowConsentDomain, label, call))
    } else if name == NEEDS_YOU_DENY_WALL_DOMAIN.name {
        let domain = domain?;
        let label = format!("Deny {domain}: no Run reads past its Walls");
        let call = Call::DenyWallDomain(NeedsYouDenyWallDomainParams {
            domain,
            reason: DENIAL_REASON.to_owned(),
        });
        Some((Action::DenyWallDomain, label, call))
    } else if name == NEEDS_YOU_REMOVE_REFETCH_DOMAIN.name {
        let domain = domain?;
        let only_star = item.details.get("only_star").and_then(Value::as_bool);
        if only_star != Some(false) {
            return None;
        }
        let label = format!("Remove {domain} from the refetch domains");
        let call = Call::RemoveRefetchDomain(NeedsYouRemoveRefetchDomainParams {
            domain,
            instead_of_star: None,
        });
        Some((Action::RemoveRefetchDomain, label, call))
    } else if name == NEEDS_YOU_DISMISS.name {
        let call = Call::Dismiss(NeedsYouDismissParams {
            item: item.id.clone(),
        });
        Some((Action::Dismiss, "Dismiss".to_owned(), call))
    } else {
        None
    }
}

/// The text detail `name` of `item`.
fn detail<'a>(item: &'a NeedsYouItem, name: &str) -> Option<&'a str> {
    item.details.get(name).and_then(Value::as_str)
}

/// The Limit that stopped the Run of a `run_stopped_by_limit` item.
fn reached(item: &NeedsYouItem) -> Option<ReachedLimit> {
    serde_json::from_value(item.details.get("limit")?.clone()).ok()
}

/// `amount` as a person reads it: a count, or millionths of `currency`.
fn shown_amount(amount: u64, currency: Option<&str>) -> String {
    let Some(currency) = currency else {
        return amount.to_string();
    };
    let whole = amount / 1_000_000;
    let fraction = amount % 1_000_000;
    if fraction == 0 {
        format!("{whole} {currency}")
    } else {
        let digits = format!("{fraction:06}");
        format!("{whole}.{} {currency}", digits.trim_end_matches('0'))
    }
}
