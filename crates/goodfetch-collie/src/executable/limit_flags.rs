//! A start's Limits on the command line (ADRs 0041 and 0080): `--limit
//! <kind>=<amount>`, repeatable, the Run's own Limits, which a start that
//! names the Run again only lowers (ADR 0102). A count is a whole number,
//! such as `requests=500`; a price is a decimal followed by its currency's
//! ISO 4217 code, with or without a space, such as `paid_fetch=5USD` or
//! `paid_fetch=0.50 USD`, sent in millionths of the currency. A malformed
//! value is a usage error clap reports naming `--limit` while it reads the
//! command line, before any engine call; the engine checks the Usage kind
//! and whether it takes a currency. Only a command whose operation takes
//! `limits` offers the flag; anywhere else clap refuses it as it does any
//! unknown flag.

use clap::Args;
use goodfetch_collie_client::Money;
use goodfetch_protocol::run::RunLimit;
use serde_json::Value;

/// The Limit flag of a command whose operation takes `limits`.
#[derive(Args, Clone, Debug, Default, PartialEq)]
pub(super) struct LimitFlags {
    /// Holds the Run to AMOUNT of the Usage kind KIND: a count, such as
    /// `requests=500`, or a price with its currency, such as
    /// `paid_fetch=0.50USD`; repeatable, and a start that names the Run
    /// again only lowers a Limit
    #[arg(long = "limit", value_name = "KIND=AMOUNT", value_parser = limit)]
    limit: Vec<RunLimit>,
}

impl LimitFlags {
    /// The `limits` param they give, none when no `--limit` was given.
    pub(super) fn limits(&self) -> Option<Value> {
        (!self.limit.is_empty()).then(|| {
            serde_json::to_value(&self.limit).expect("invariant: Run Limits encode as JSON")
        })
    }
}

/// Reads `--limit`'s `kind=amount`.
fn limit(text: &str) -> Result<RunLimit, String> {
    let (kind, amount) = text
        .split_once('=')
        .filter(|(kind, _)| !kind.is_empty())
        .ok_or_else(|| {
            "expected `kind=amount`, such as `requests=500` or `paid_fetch=0.50USD`".to_owned()
        })?;
    let currency_at = amount
        .find(|character: char| character.is_ascii_alphabetic())
        .unwrap_or(amount.len());
    let (number, currency) = amount.split_at(currency_at);
    if currency.is_empty() {
        let count = number
            .parse::<u64>()
            .ok()
            .filter(|_| number.bytes().all(|byte| byte.is_ascii_digit()))
            .ok_or_else(|| {
                format!(
                    "`{amount}` is no amount: give a whole number, such as `{kind}=500`, or a price with its currency, such as `{kind}=0.50USD`"
                )
            })?;
        return Ok(RunLimit {
            usage_kind: kind.to_owned(),
            currency: None,
            amount: count,
        });
    }
    let money = Money::new(number.trim_end(), currency)
        .map_err(|error| format!("`{amount}` is no price: {error}"))?;
    Ok(RunLimit {
        usage_kind: kind.to_owned(),
        currency: Some(money.currency().to_owned()),
        amount: money.millionths(),
    })
}

#[cfg(test)]
mod tests;
