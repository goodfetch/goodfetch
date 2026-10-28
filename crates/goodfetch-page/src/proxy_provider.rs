//! The Proxy-provider driver (spec #599; ADRs 0021, 0041, 0101 and 0129):
//! what every Proxy provider shares, written once. [`ProxyDriver`] owns the
//! validation of `proxy.kind` and `proxy.country` against what the provider
//! offers, the sticky lifetime (the provider's longest unless
//! `proxy.sticky_lifetime` asks for less), one proxy per Session for its
//! life, the `proxy_exit_may_change` warning, and the bytes per proxy kind
//! and their price that a Session's proxy charges ([`Tally`], [`priced`]).
//! A provider fills only the two primitives of [`ProxyProvider`],
//! `username` and `format_lifetime`, beside the [`Offer`] it describes
//! itself with. [`list`] is the provider of the user's own servers;
//! [`bright_data`], [`oxylabs`] and [`apify`] are the built-in adapters of
//! those providers, each with its translation point (ADR 0127) reading the
//! provider's failures into a [`failure::ProviderFailure`].

pub(crate) mod apify;
pub(crate) mod bright_data;
pub(crate) mod failure;
pub(crate) mod list;
pub(crate) mod oxylabs;

use std::time::Duration;

use std::sync::{Arc, OnceLock};

use data_encoding::HEXLOWER;
use goodfetch_protocol::page::proxy::ProxyProviderConfiguration;
use sha2::{Digest as _, Sha256};

use goodfetch_engine::CallError;
use goodfetch_engine::ledger::Refusal as LedgerRefusal;
use goodfetch_engine::run::{limit_not_understood, limit_reached};
use goodfetch_engine::runs::{ChargeError, Run};
use goodfetch_protocol::catalog::PriceValue;
use goodfetch_protocol::page::proxy::{CountryCode, ProxyKind, ProxyServer};
use goodfetch_protocol::run::RunWarning;
use goodfetch_protocol::run::warning::PROXY_EXIT_MAY_CHANGE;
use goodfetch_protocol::usage::PROXY_BYTES;
use tokio::sync::{Mutex, oneshot};

/// A GB, the unit a metered proxy is priced in.
const GB: u128 = 1_000_000_000;

/// One proxy a provider offers: the kinds and countries it serves.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Endpoint {
    /// The proxy.
    pub(crate) server: ProxyServer,
    /// The proxy kinds it serves, the first its default.
    pub(crate) kinds: Vec<ProxyKind>,
    /// The countries its exits are in; none when it serves any.
    pub(crate) countries: Option<Vec<CountryCode>>,
    /// The longest a Session's exit through it stays the same; none when
    /// it never changes, as a `list` server's.
    pub(crate) longest_lifetime: Option<Duration>,
}

/// What a provider offers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Offer {
    /// Its proxies, the first's first kind the provider's default.
    pub(crate) endpoints: Vec<Endpoint>,
}

/// The name a provider's sticky session takes for the Session `session`:
/// the first 16 hex digits of its SHA-256, which every provider's session
/// syntax takes (letters and digits, no `-`, at most 50 characters) and
/// which names no Run to the provider.
pub(crate) fn sticky_session(session: &str) -> String {
    let digest = Sha256::digest(session.as_bytes());
    HEXLOWER.encode(&digest[..8])
}

/// The endpoints of a hosted provider: its one proxy, `configuration`'s
/// `endpoint` or else `public`, serving each of `kinds` with its longest
/// sticky lifetime, in any country.
pub(crate) fn hosted(
    configuration: &ProxyProviderConfiguration,
    public: &str,
    kinds: &[(ProxyKind, Duration)],
) -> Offer {
    let server = configuration.endpoint.clone().unwrap_or_else(|| {
        ProxyServer::parse(public).expect("invariant: a provider's public proxy is a proxy URL")
    });
    Offer {
        endpoints: kinds
            .iter()
            .map(|(kind, longest)| Endpoint {
                server: server.clone(),
                kinds: vec![*kind],
                countries: None,
                longest_lifetime: Some(*longest),
            })
            .collect(),
    }
}

/// The account `configuration` names, which `name`'s sticky username
/// builds on.
///
/// # Errors
///
/// What is missing, for `setting_invalid`.
pub(crate) fn account(
    configuration: &ProxyProviderConfiguration,
    name: &str,
    what: &str,
) -> Result<String, String> {
    configuration.username.clone().ok_or_else(|| {
        format!(
            "the Proxy provider `{name}` builds its sticky username on the account: set `proxy.providers.{name}.username` to {what}, and `.password` to the Secret reference of its password"
        )
    })
}

/// What a provider's sticky username names for one Session.
#[derive(Clone, Debug)]
pub(crate) struct Sticky<'a> {
    /// The Session's ID, which names its sticky session.
    pub(crate) session: &'a str,
    /// Its proxy kind.
    pub(crate) kind: ProxyKind,
    /// Its exit's country, if one was asked for.
    pub(crate) country: Option<&'a CountryCode>,
    /// Its sticky lifetime in the provider's spelling, if the exit can
    /// change.
    pub(crate) lifetime: Option<String>,
}

/// A Proxy provider's two primitives (ADR 0129), beside its offer.
pub(crate) trait ProxyProvider: Send + Sync {
    /// What it offers.
    fn offer(&self) -> &Offer;

    /// The username a Session gives the proxy for `sticky`; none for none.
    fn username(&self, sticky: &Sticky<'_>) -> Option<String>;

    /// `lifetime` in the provider's spelling.
    fn format_lifetime(&self, lifetime: Duration) -> String;
}

/// What a Session asks of its provider, from its settings.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Wanted {
    /// `proxy.kind`.
    pub(crate) kind: Option<ProxyKind>,
    /// `proxy.country`.
    pub(crate) country: Option<CountryCode>,
    /// `proxy.sticky_lifetime`.
    pub(crate) sticky_lifetime: Option<Duration>,
}

/// The one proxy a Session got, for its life.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Chosen {
    /// The proxy.
    pub(crate) server: ProxyServer,
    /// Its proxy kind.
    pub(crate) kind: ProxyKind,
    /// The country asked for.
    pub(crate) country: Option<CountryCode>,
    /// The username the Session gives it.
    pub(crate) username: Option<String>,
    /// How long its exit stays the same; none when it never changes.
    pub(crate) lifetime: Option<Duration>,
}

/// Why a Session gets no proxy from its provider.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// `proxy.kind` names a kind the provider does not offer.
    KindNotOffered {
        provider: String,
        asked: ProxyKind,
        offered: Vec<ProxyKind>,
    },
    /// `proxy.country` names a country the provider does not offer for
    /// the kind.
    CountryNotOffered {
        provider: String,
        asked: CountryCode,
        kind: ProxyKind,
        offered: Vec<CountryCode>,
    },
    /// The provider offers no proxy at all.
    Empty { provider: String },
    /// A currency Limit on `proxy_bytes` holds the Run, and the provider
    /// has no price in its currency.
    Unpriced { provider: String, currency: String },
}

/// The driver of one Proxy provider.
pub(crate) struct ProxyDriver {
    name: String,
    provider: Box<dyn ProxyProvider>,
}

impl ProxyDriver {
    /// The driver of `provider`, named `name`.
    pub(crate) fn new(name: &str, provider: Box<dyn ProxyProvider>) -> Self {
        Self {
            name: name.to_owned(),
            provider,
        }
    }

    /// The proxy of the Session `session`, the `ordinal`-th of its Run, as
    /// `wanted` asks.
    ///
    /// # Errors
    ///
    /// A [`Refusal`] when the provider does not offer what is asked.
    pub(crate) fn choose(
        &self,
        wanted: &Wanted,
        session: &str,
        ordinal: u64,
    ) -> Result<Chosen, Refusal> {
        let offer = self.provider.offer();
        let provider = || self.name.clone();
        let Some(default) = offer
            .endpoints
            .first()
            .and_then(|endpoint| endpoint.kinds.first())
        else {
            return Err(Refusal::Empty {
                provider: provider(),
            });
        };
        let kind = wanted.kind.unwrap_or(*default);
        let of_kind: Vec<&Endpoint> = offer
            .endpoints
            .iter()
            .filter(|endpoint| endpoint.kinds.contains(&kind))
            .collect();
        if of_kind.is_empty() {
            let mut offered: Vec<ProxyKind> = offer
                .endpoints
                .iter()
                .flat_map(|endpoint| endpoint.kinds.iter().copied())
                .collect();
            offered.sort_unstable();
            offered.dedup();
            return Err(Refusal::KindNotOffered {
                provider: provider(),
                asked: kind,
                offered,
            });
        }
        let serving: Vec<&Endpoint> = match &wanted.country {
            None => of_kind.clone(),
            Some(country) => of_kind
                .iter()
                .copied()
                .filter(|endpoint| {
                    endpoint
                        .countries
                        .as_ref()
                        .is_none_or(|countries| countries.contains(country))
                })
                .collect(),
        };
        let Some(count) = u64::try_from(serving.len()).ok().filter(|count| *count > 0) else {
            let mut offered: Vec<CountryCode> = of_kind
                .iter()
                .flat_map(|endpoint| endpoint.countries.iter().flatten().cloned())
                .collect();
            offered.sort_unstable();
            offered.dedup();
            return Err(Refusal::CountryNotOffered {
                provider: provider(),
                asked: wanted
                    .country
                    .clone()
                    .expect("invariant: only a country leaves no proxy of the kind"),
                kind,
                offered,
            });
        };
        let turn = usize::try_from(ordinal.saturating_sub(1) % count)
            .expect("invariant: a turn is below the endpoints' count");
        let endpoint = serving[turn];
        let lifetime = endpoint.longest_lifetime.map(|longest| {
            wanted
                .sticky_lifetime
                .map_or(longest, |asked| asked.min(longest))
        });
        let username = self.provider.username(&Sticky {
            session,
            kind,
            country: wanted.country.as_ref(),
            lifetime: lifetime.map(|lifetime| self.provider.format_lifetime(lifetime)),
        });
        Ok(Chosen {
            server: endpoint.server.clone(),
            kind,
            country: wanted.country.clone(),
            username,
            lifetime,
        })
    }

    /// The provider's name.
    pub(crate) fn name(&self) -> &str {
        &self.name
    }
}

impl Chosen {
    /// The `proxy_exit_may_change` warning of the Session `session`, which
    /// has lived `lived`, if it outlived its sticky lifetime.
    pub(crate) fn exit_warning(&self, session: &str, lived: Duration) -> Option<RunWarning> {
        let lifetime = self.lifetime.filter(|lifetime| lived > *lifetime)?;
        Some(RunWarning {
            code: PROXY_EXIT_MAY_CHANGE.to_owned(),
            message: format!(
                "The Session outlived its proxy's sticky lifetime of {} s: its exit, the address a site sees, may have changed under the same cookies; open a new Session, or set a longer `proxy.sticky_lifetime` if the provider allows one.",
                lifetime.as_secs()
            ),
            sessions: vec![session.to_owned()],
            folder: None,
        })
    }
}

/// The price a Session of `provider`, priced at `price`, charges, when the
/// Run is held by currency Limits on `proxy_bytes` in `currencies`.
///
/// # Errors
///
/// [`Refusal::Unpriced`] when one of them is in a currency the price is
/// not in.
pub(crate) fn priced(
    provider: &str,
    currencies: &[&str],
    price: Option<&PriceValue>,
) -> Result<Option<PriceValue>, Refusal> {
    match currencies
        .iter()
        .find(|currency| price.is_none_or(|price| price.currency != **currency))
    {
        Some(currency) => Err(Refusal::Unpriced {
            provider: provider.to_owned(),
            currency: (*currency).to_owned(),
        }),
        None => Ok(price.cloned()),
    }
}

/// What a Session's proxy carried, and what of it was charged.
#[derive(Clone, Debug)]
pub(crate) struct Tally {
    price: Option<PriceValue>,
    bytes: u128,
    charged: u64,
}

impl Tally {
    /// A Session's tally, its bytes priced at `price` per GB.
    pub(crate) fn new(price: Option<PriceValue>) -> Self {
        Self {
            price,
            bytes: 0,
            charged: 0,
        }
    }

    /// Adds `bytes` the proxy carried: the bytes to charge, and the price
    /// to charge for them in millionths of the price's currency, so that
    /// the price charged in all is that of every byte so far, rounded up.
    pub(crate) fn add(&mut self, bytes: u64) -> (u64, Option<u64>) {
        self.bytes += u128::from(bytes);
        let Some(price) = &self.price else {
            return (bytes, None);
        };
        let due = (self.bytes * u128::from(price.millionths)).div_ceil(GB);
        let due = u64::try_from(due).unwrap_or(u64::MAX);
        let owed = due.saturating_sub(self.charged);
        self.charged = due;
        (bytes, Some(owed))
    }

    /// The price's currency, if it has one.
    pub(crate) fn currency(&self) -> Option<&str> {
        self.price.as_ref().map(|price| price.currency.as_str())
    }
}

/// What a Session's provider proxy charges its Run (ADRs 0021, 0041 and
/// 0101): every byte on its connections, as `proxy_bytes` under its proxy
/// kind, and their price, reserved as paid, in its currency.
pub(crate) struct ProxyUsage {
    run: Run,
    kind: ProxyKind,
    tally: Mutex<Tally>,
    /// The Limit's refusal that stopped the proxy, which the Session's
    /// failed calls raise in place of the browser's network error.
    refusal: OnceLock<CallError>,
}

impl ProxyUsage {
    /// The usage of a Session of `run` whose proxy is of `kind`, priced at
    /// `price` per GB.
    pub(crate) fn new(run: Run, kind: ProxyKind, price: Option<PriceValue>) -> Self {
        Self {
            run,
            kind,
            tally: Mutex::new(Tally::new(price)),
            refusal: OnceLock::new(),
        }
    }

    /// The Limit's refusal that stopped the proxy, once one did.
    pub(crate) fn refusal(&self) -> Option<&CallError> {
        self.refusal.get()
    }

    /// Keeps `error`'s refusal, a Limit's, for the Session's failed calls.
    fn refused(&self, error: &ChargeError) {
        let refusal = match error {
            ChargeError::Refused(LedgerRefusal::Reached(reached)) => limit_reached(reached),
            ChargeError::Refused(LedgerRefusal::NotUnderstood(limit)) => {
                limit_not_understood(limit.as_ref().clone())
            }
            _ => return,
        };
        let _ = self.refusal.set(refusal);
    }

    /// Charges `bytes` the proxy carried on a task of the Run, which runs
    /// to its end even when the caller stops waiting for its answer, as a
    /// relayed connection does when its Session's proxy closes or its Run
    /// ends: the provider bills those bytes all the same, and the Run waits
    /// for its tasks before it writes its report. Answers whether the
    /// connection may carry more, false once a Limit refused them, when it
    /// must carry no more.
    pub(crate) fn charge_to_the_end(self: &Arc<Self>, bytes: u64) -> oneshot::Receiver<bool> {
        let (answer, answered) = oneshot::channel();
        if bytes == 0 {
            let _ = answer.send(true);
            return answered;
        }
        let usage = Arc::clone(self);
        self.run.spawn(async move {
            let _ = answer.send(usage.charge(bytes).await);
        });
        answered
    }

    /// Charges `bytes` the proxy carried: whether it may carry more, false
    /// once a Limit refused them. Dropping the future may leave them
    /// uncounted ([`Run::charge`]'s cancel safety), so only
    /// [`ProxyUsage::charge_to_the_end`]'s task awaits it.
    async fn charge(&self, bytes: u64) -> bool {
        if bytes == 0 {
            return true;
        }
        // The tally counts the bytes and what they owe under its lock, which
        // is let go before the charge is awaited (STYLE.md: no lock across
        // an `.await`); charges add up whatever order they land in.
        let (bytes, price) = {
            let mut tally = self.tally.lock().await;
            let (bytes, owed) = tally.add(bytes);
            let price = match (owed, tally.currency()) {
                (Some(owed), Some(currency)) if owed > 0 => Some((currency.to_owned(), owed)),
                _ => None,
            };
            (bytes, price)
        };
        let price = price
            .as_ref()
            .map(|(currency, owed)| (currency.as_str(), *owed));
        match self
            .run
            .charge_part(PROXY_BYTES, self.kind.as_str(), bytes, price)
            .await
        {
            Ok(()) => true,
            Err(error) => {
                tracing::debug!(%error, "a Session's proxy bytes were refused");
                self.refused(&error);
                false
            }
        }
    }
}

#[cfg(test)]
mod tests;
