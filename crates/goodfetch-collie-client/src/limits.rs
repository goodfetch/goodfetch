//! Limits in code (ADRs 0041 and 0080): what a connection and a Run's start
//! take, typed per Usage kind, with a price as an exact [`Money`]. A
//! `Money` turns into millionths of its currency when it is made, so an
//! amount that does not turn exactly fails there, before anything is sent,
//! and no float ever reaches a cap. No member raises a Limit: only a person
//! does.

use goodfetch_protocol::run::RunLimit;
use goodfetch_protocol::usage;

/// Millionths in one unit of a currency.
const MILLIONTHS: u64 = 1_000_000;

/// Decimals a millionth carries.
const DECIMALS: usize = 6;

/// A Usage kind a Limit caps, by its catalog name.
///
/// The constants are the Usage kinds every Product counts; a Product crate
/// names its own with [`UsageKind::named`], as Foxhound does its paid
/// fetches (ADR 0087). An engine refuses a kind its Product does not count
/// with `not_in_product`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct UsageKind(&'static str);

impl UsageKind {
    /// Model tokens, input and output together.
    pub const MODEL_TOKENS: Self = Self(usage::MODEL_TOKENS);
    /// Model calls.
    pub const MODEL_CALLS: Self = Self(usage::MODEL_CALLS);
    /// Browser time, counted in milliseconds.
    pub const BROWSER_TIME: Self = Self(usage::BROWSER_TIME);
    /// Bytes through a proxy.
    pub const PROXY_BYTES: Self = Self(usage::PROXY_BYTES);
    /// Paid challenge solves.
    pub const PAID_SOLVES: Self = Self(usage::PAID_SOLVES);
    /// Requests.
    pub const REQUESTS: Self = Self(usage::REQUESTS);
    /// Page crashes.
    pub const PAGE_CRASHES: Self = Self(usage::PAGE_CRASHES);

    /// The Usage kind the catalog names `name`, for a Product crate's own.
    #[must_use]
    pub const fn named(name: &'static str) -> Self {
        Self(name)
    }

    /// The kind's catalog name, as the wire carries it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        self.0
    }
}

/// A price: an amount exact to a millionth of its currency.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Money {
    millionths: u64,
    currency: String,
}

impl Money {
    /// `amount`, digits with at most one decimal point such as `"0.50"`, of
    /// `currency`, an ISO 4217 code such as `"USD"`.
    ///
    /// # Errors
    ///
    /// [`MoneyError`] when `amount` is no plain decimal, is finer than a
    /// millionth, or is more millionths than the wire carries, or when
    /// `currency` is not three capital letters.
    pub fn new(amount: &str, currency: &str) -> Result<Self, MoneyError> {
        if currency.len() != 3 || !currency.bytes().all(|byte| byte.is_ascii_uppercase()) {
            return Err(MoneyError::NotACurrency);
        }
        Ok(Self {
            millionths: millionths(amount)?,
            currency: currency.to_owned(),
        })
    }

    /// The amount in millionths of its currency, as the wire carries it.
    #[must_use]
    pub const fn millionths(&self) -> u64 {
        self.millionths
    }

    /// The currency's ISO 4217 code.
    #[must_use]
    pub fn currency(&self) -> &str {
        &self.currency
    }
}

/// Why [`Money::new`] refused an amount.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum MoneyError {
    /// The amount is not digits with at most one decimal point.
    #[error("a Money amount is digits with at most one decimal point, such as \"0.50\"")]
    NotADecimal,
    /// The amount has a non-zero digit past the sixth decimal.
    #[error("a Money amount is exact to a millionth: give at most six decimals")]
    FinerThanAMillionth,
    /// The amount is more millionths than the wire carries.
    #[error("a Money amount is at most 18446744073709.551615, the millionths the wire carries")]
    TooLarge,
    /// The currency is not an ISO 4217 code.
    #[error("a Money currency is an ISO 4217 code: three capital letters, such as USD")]
    NotACurrency,
}

/// The Limits a program sets on its connection or on a Run at its start
/// (ADR 0080): a count or a price per Usage kind. Each only lowers what
/// covers that work.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Limits {
    limits: Vec<RunLimit>,
}

impl Limits {
    /// No Limit.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Caps `kind` at `amount` of it, in place of a count given before.
    #[must_use]
    pub fn count(self, kind: UsageKind, amount: u64) -> Self {
        self.with(RunLimit {
            usage_kind: kind.name().to_owned(),
            currency: None,
            amount,
        })
    }

    /// Caps what `kind` may cost at `price`, in place of a price given
    /// before in the same currency.
    #[must_use]
    pub fn price(self, kind: UsageKind, price: Money) -> Self {
        self.with(RunLimit {
            usage_kind: kind.name().to_owned(),
            currency: Some(price.currency),
            amount: price.millionths,
        })
    }

    /// `self` with `limit`, which replaces one on the same Usage kind and
    /// currency: the wire takes one Limit for each.
    fn with(mut self, limit: RunLimit) -> Self {
        let same = |given: &RunLimit| {
            given.usage_kind == limit.usage_kind && given.currency == limit.currency
        };
        match self.limits.iter_mut().find(|given| same(given)) {
            Some(given) => *given = limit,
            None => self.limits.push(limit),
        }
        self
    }
}

/// The Limits as the wire lists them, a price in millionths of its
/// currency, in the order given.
impl From<Limits> for Vec<RunLimit> {
    fn from(limits: Limits) -> Self {
        limits.limits
    }
}

/// `amount` in millionths: digits, then at most one point and digits, with
/// no non-zero digit past the sixth decimal.
fn millionths(amount: &str) -> Result<u64, MoneyError> {
    let (units, decimals) = match amount.split_once('.') {
        Some((units, decimals)) => (units, decimals),
        None => (amount, "0"),
    };
    let digits = |part: &str| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit());
    if !digits(units) || !digits(decimals) {
        return Err(MoneyError::NotADecimal);
    }
    let (kept, past) = decimals.split_at(decimals.len().min(DECIMALS));
    if past.bytes().any(|byte| byte != b'0') {
        return Err(MoneyError::FinerThanAMillionth);
    }
    let fraction = format!("{kept:0<DECIMALS$}")
        .parse::<u64>()
        .expect("invariant: six ASCII digits parse as a u64");
    let whole = units
        .bytes()
        .try_fold(0_u64, |total, digit| {
            total.checked_mul(10)?.checked_add(u64::from(digit - b'0'))
        })
        .ok_or(MoneyError::TooLarge)?;
    whole
        .checked_mul(MILLIONTHS)
        .and_then(|whole| whole.checked_add(fraction))
        .ok_or(MoneyError::TooLarge)
}

#[cfg(test)]
mod tests;
