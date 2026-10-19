//! Network profiles (ADR 0025, spec #152): the added round-trip time and loss
//! on every connection to the Proving ground, applied by one shaper and
//! calibrated before `serve` reports its address. `tc netem` shapes the
//! loopback interface on Linux, for published figures; the lab's user-space
//! latency proxy, ported, shapes it on any OS without privileges, for
//! unpublished runs. The calibration is a measurement recorded with the run,
//! never a test assertion (ADR 0064): a run outside tolerance keeps its
//! correctness results and loses its time metrics.

mod calibration;
mod link;
mod netem;
mod proxy;

use std::fmt;
use std::future;
use std::net::{Ipv4Addr, SocketAddr};
use std::str::FromStr;
use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use serde::Serialize;

use self::calibration::{Calibration, Echo, SAMPLES, TimeMetrics};
use self::netem::Netem;
use self::proxy::Proxy;

/// The network one `serve` runs on: as it is, or shaped by a profile and
/// calibrated.
pub(crate) struct Network {
    shaped: Option<Shaped>,
}

struct Shaped {
    profile: Profile,
    calibration: Calibration,
    shaper: Shaping,
}

enum Shaping {
    Proxy(Proxy),
    Netem(Netem),
}

impl Network {
    /// No added delay or loss, and nothing to calibrate.
    pub(crate) fn unshaped() -> Self {
        Self { shaped: None }
    }

    /// Calibrates `profile` through a latency proxy of its own, then binds
    /// the proxy clients connect to at `address`.
    pub(crate) async fn proxy(profile: Profile, seed: u64, address: SocketAddr) -> Result<Self> {
        let calibration = {
            let direct = Echo::bind()
                .await
                .context("binding the calibration's echo")?;
            let shaped = Echo::bind()
                .await
                .context("binding the calibration's echo")?;
            let proxy = Proxy::bind(loopback(), profile, seed).await?;
            let (direct_address, shaped_address) = (direct.address()?, shaped.address()?);
            let measured = async {
                tokio::try_join!(
                    direct.answer_one(),
                    shaped.answer_one(),
                    calibration::measure(direct_address, proxy.address(), SAMPLES),
                )
            };
            let pairs = tokio::select! {
                measured = measured => measured.context("calibrating the latency proxy")?.2,
                () = proxy.forward(shaped_address) => bail!("the calibration's latency proxy stopped"),
            };
            Calibration::of(profile.round_trip, &pairs)
        };
        let proxy = Proxy::bind(address, profile, seed).await?;
        Ok(Self {
            shaped: Some(Shaped {
                profile,
                calibration,
                shaper: Shaping::Proxy(proxy),
            }),
        })
    }

    /// Calibrates `profile` with `tc netem` on a loopback echo of its own,
    /// then shapes `port`.
    pub(crate) async fn netem(profile: Profile, port: u16) -> Result<Self> {
        if !cfg!(target_os = "linux") {
            bail!("`tc netem` exists only on Linux; use the latency proxy (`--shaper proxy`)");
        }
        let calibration = {
            let direct = Echo::bind()
                .await
                .context("binding the calibration's echo")?;
            let shaped = Echo::bind()
                .await
                .context("binding the calibration's echo")?;
            let (direct_address, shaped_address) = (direct.address()?, shaped.address()?);
            let netem = Netem::apply(&profile, shaped_address.port()).await?;
            let measured = tokio::try_join!(
                direct.answer_one(),
                shaped.answer_one(),
                calibration::measure(direct_address, shaped_address, SAMPLES),
            );
            netem.remove().await?;
            Calibration::of(
                profile.round_trip,
                &measured.context("calibrating `tc netem`")?.2,
            )
        };
        let netem = Netem::apply(&profile, port).await?;
        Ok(Self {
            shaped: Some(Shaped {
                profile,
                calibration,
                shaper: Shaping::Netem(netem),
            }),
        })
    }

    /// The address clients connect to, when it is a latency proxy's rather
    /// than the server's own.
    pub(crate) fn proxy_address(&self) -> Option<SocketAddr> {
        match &self.shaped {
            Some(Shaped {
                shaper: Shaping::Proxy(proxy),
                ..
            }) => Some(proxy.address()),
            _ => None,
        }
    }

    /// The profile and its calibration, as recorded with the run.
    pub(crate) fn report(&self) -> Report<'_> {
        match &self.shaped {
            None => Report {
                profile: "none",
                shaped: None,
            },
            Some(shaped) => Report {
                profile: shaped.profile.name,
                shaped: Some(ShapedReport {
                    round_trip_microseconds: u64::try_from(shaped.profile.round_trip.as_micros())
                        .unwrap_or(u64::MAX),
                    loss_percent: shaped
                        .profile
                        .loss
                        .to_string()
                        .parse()
                        .expect("invariant: a loss prints as a JSON number"),
                    shaper: match shaped.shaper {
                        Shaping::Proxy(_) => "proxy",
                        Shaping::Netem(_) => "netem",
                    },
                    calibration: &shaped.calibration,
                    time_metrics: shaped.calibration.time_metrics(),
                }),
            },
        }
    }

    /// Forwards each connection to `upstream` through the latency proxy;
    /// with no proxy, waits forever.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future stops accepting connections.
    pub(crate) async fn forward(&self, upstream: SocketAddr) {
        match &self.shaped {
            Some(Shaped {
                shaper: Shaping::Proxy(proxy),
                ..
            }) => proxy.forward(upstream).await,
            _ => future::pending().await,
        }
    }

    /// Undoes the shaping: removes `tc netem` from the loopback interface.
    pub(crate) async fn stop(self) -> Result<()> {
        match self.shaped {
            Some(Shaped {
                shaper: Shaping::Netem(netem),
                ..
            }) => netem.remove().await,
            _ => Ok(()),
        }
    }
}

/// Port 0 of the IPv4 loopback address.
fn loopback() -> SocketAddr {
    SocketAddr::from((Ipv4Addr::LOCALHOST, 0))
}

/// The network as `serve`'s first line records it.
#[derive(Serialize)]
pub(crate) struct Report<'a> {
    profile: &'static str,
    #[serde(flatten)]
    shaped: Option<ShapedReport<'a>>,
}

#[derive(Serialize)]
struct ShapedReport<'a> {
    round_trip_microseconds: u64,
    loss_percent: serde_json::Number,
    shaper: &'static str,
    calibration: &'a Calibration,
    /// A run whose calibration is outside tolerance keeps its correctness
    /// results and loses its time metrics.
    time_metrics: TimeMetrics,
}

/// A declared network profile: its name, added round-trip time and loss.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Profile {
    name: &'static str,
    round_trip: Duration,
    loss: Loss,
}

impl Profile {
    pub(crate) fn new(name: &'static str, round_trip: Duration, loss: Loss) -> Self {
        Self {
            name,
            round_trip,
            loss,
        }
    }
}

/// The share of packets lost in each direction, in parts per million: a
/// percentage of at most 100 with at most four decimals.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Loss(u32);

impl Loss {
    /// Parts per million of a percentage point.
    const PER_PERCENT: u32 = 10_000;
    const ALL: u32 = 100 * Self::PER_PERCENT;

    /// Whether any packet is lost.
    pub(crate) fn is_some(self) -> bool {
        self.0 > 0
    }

    /// The share lost, in parts per million.
    fn per_million(self) -> u32 {
        self.0
    }
}

impl FromStr for Loss {
    type Err = String;

    /// Reads a percentage such as `0`, `1` or `0.5`.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let refused =
            || format!("`{text}` is not a percentage from 0 to 100 with at most four decimals");
        let (whole, fraction) = text.split_once('.').unwrap_or((text, ""));
        let digits =
            |part: &str| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit());
        if !digits(whole) || (text.contains('.') && !digits(fraction)) || fraction.len() > 4 {
            return Err(refused());
        }
        let whole: u32 = whole.parse().map_err(|_| refused())?;
        let fraction: u32 = format!("{fraction:0<4}").parse().map_err(|_| refused())?;
        let per_million = whole
            .checked_mul(Self::PER_PERCENT)
            .and_then(|whole| whole.checked_add(fraction))
            .filter(|per_million| *per_million <= Self::ALL)
            .ok_or_else(refused)?;
        Ok(Self(per_million))
    }
}

impl fmt::Display for Loss {
    /// The percentage, with no trailing zero: `0`, `1`, `0.5`.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let whole = self.0 / Self::PER_PERCENT;
        let fraction = self.0 % Self::PER_PERCENT;
        if fraction == 0 {
            write!(formatter, "{whole}")
        } else {
            let fraction = format!("{fraction:04}");
            write!(formatter, "{whole}.{}", fraction.trim_end_matches('0'))
        }
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::Loss;

    #[rstest]
    #[case::zero("0", "0")]
    #[case::one("1", "1")]
    #[case::a_half("0.5", "0.5")]
    #[case::four_decimals("2.0625", "2.0625")]
    #[case::trailing_zeros("1.50", "1.5")]
    #[case::all("100", "100")]
    fn a_loss_reads_and_prints_as_a_percentage(#[case] text: &str, #[case] printed: &str) {
        let loss: Loss = text.parse().unwrap();

        assert_eq!(loss.to_string(), printed);
    }

    #[rstest]
    #[case::empty("")]
    #[case::over_all("100.5")]
    #[case::five_decimals("0.00001")]
    #[case::a_sign("-1")]
    #[case::a_percent_sign("1%")]
    #[case::no_fraction_digits("1.")]
    #[case::no_whole_digits(".5")]
    fn a_loss_that_is_no_percentage_is_refused(#[case] text: &str) {
        assert!(text.parse::<Loss>().is_err());
    }
}
