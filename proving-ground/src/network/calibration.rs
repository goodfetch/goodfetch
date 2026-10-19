//! A profile's calibration, the lab's ported: before the run, one-byte
//! ping-pongs to an echo through the shaped link, each paired with a direct
//! one to another echo. The added delay is the median of each pair's shaped
//! minus direct round trip, so loopback's own cost and the host's drift are
//! not charged to the profile. It is within tolerance when it is off the
//! declared round trip by at most the larger of 10% and 2 ms; outside, a run
//! keeps its correctness results and loses its time metrics. A measurement
//! recorded with the run, never a test assertion (ADR 0064).
#![expect(
    clippy::disallowed_types,
    reason = "the calibration opens its own loopback echoes and connections, outside the engine's transport (ADR 0060)"
)]

use std::io;
use std::net::{Ipv4Addr, SocketAddr};
use std::time::Duration;

use serde::Serialize;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};

use crate::clock::Stopwatch;

/// The round trips measured each way, as the lab measures them.
pub(crate) const SAMPLES: usize = 50;

/// The least tolerance, for the shortest round trips.
const LEAST_TOLERANCE_MICROSECONDS: u64 = 2_000;

/// One direct round trip and the shaped one after it, in microseconds.
#[derive(Clone, Copy)]
pub(crate) struct Pair {
    pub(crate) direct: u64,
    pub(crate) shaped: u64,
}

/// Whether a run's time metrics stand.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TimeMetrics {
    /// The measured added delay is the declared one, within tolerance.
    Available,
    /// It is not: the run keeps its correctness results only.
    Unavailable,
}

/// What a calibration measured, recorded with the run.
#[derive(Debug, PartialEq, Eq, Serialize)]
pub(crate) struct Calibration {
    /// The declared added round trip.
    round_trip_microseconds: u64,
    samples: usize,
    shaped_median_microseconds: u64,
    shaped_p95_microseconds: u64,
    direct_median_microseconds: u64,
    /// The median of each pair's shaped minus direct round trip.
    added_median_microseconds: i64,
    within_tolerance: bool,
}

impl Calibration {
    /// Judges the measured pairs against the declared round trip.
    pub(crate) fn of(round_trip: Duration, pairs: &[Pair]) -> Self {
        let round_trip_microseconds = u64::try_from(round_trip.as_micros()).unwrap_or(u64::MAX);
        let mut shaped: Vec<u64> = pairs.iter().map(|pair| pair.shaped).collect();
        let mut direct: Vec<u64> = pairs.iter().map(|pair| pair.direct).collect();
        let mut added: Vec<i64> = pairs
            .iter()
            .map(|pair| signed(pair.shaped) - signed(pair.direct))
            .collect();
        shaped.sort_unstable();
        direct.sort_unstable();
        added.sort_unstable();
        let added_median_microseconds = median(&added);
        let tolerance = LEAST_TOLERANCE_MICROSECONDS.max(round_trip_microseconds / 10);
        Self {
            round_trip_microseconds,
            samples: pairs.len(),
            shaped_median_microseconds: median(&shaped),
            shaped_p95_microseconds: p95(&shaped),
            direct_median_microseconds: median(&direct),
            added_median_microseconds,
            within_tolerance: added_median_microseconds.abs_diff(signed(round_trip_microseconds))
                <= tolerance,
        }
    }

    /// Whether the run's time metrics stand.
    pub(crate) fn time_metrics(&self) -> TimeMetrics {
        if self.within_tolerance {
            TimeMetrics::Available
        } else {
            TimeMetrics::Unavailable
        }
    }
}

/// The lab's median of sorted values: the middle one, the upper of two.
fn median<T: Copy + Default>(sorted: &[T]) -> T {
    sorted.get(sorted.len() / 2).copied().unwrap_or_default()
}

/// The 95th percentile of sorted values, by the nearest rank.
fn p95(sorted: &[u64]) -> u64 {
    let rank = (sorted.len() * 95).div_ceil(100);
    sorted
        .get(rank.saturating_sub(1))
        .copied()
        .unwrap_or_default()
}

fn signed(microseconds: u64) -> i64 {
    i64::try_from(microseconds).unwrap_or(i64::MAX)
}

/// An echo that answers one connection on a loopback port: each byte it
/// receives comes back.
pub(crate) struct Echo(TcpListener);

impl Echo {
    pub(crate) async fn bind() -> io::Result<Self> {
        TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.map(Self)
    }

    pub(crate) fn address(&self) -> io::Result<SocketAddr> {
        self.0.local_addr()
    }

    /// Answers the first connection until it ends.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future closes the connection.
    pub(crate) async fn answer_one(self) -> io::Result<()> {
        let (mut stream, _) = self.0.accept().await?;
        stream.set_nodelay(true)?;
        let mut buffer = [0; 64];
        loop {
            let read = stream.read(&mut buffer).await?;
            if read == 0 {
                return Ok(());
            }
            stream.write_all(&buffer[..read]).await?;
        }
    }
}

/// Measures `samples` pairs: a one-byte round trip to the echo at `direct`,
/// then one to the echo reached through `shaped`, on one connection each,
/// which close at the end.
pub(crate) async fn measure(
    direct: SocketAddr,
    shaped: SocketAddr,
    samples: usize,
) -> io::Result<Vec<Pair>> {
    let mut direct = connect(direct).await?;
    let mut shaped = connect(shaped).await?;
    let mut pairs = Vec::with_capacity(samples);
    for _ in 0..samples {
        pairs.push(Pair {
            direct: round_trip(&mut direct).await?,
            shaped: round_trip(&mut shaped).await?,
        });
    }
    Ok(pairs)
}

async fn connect(address: SocketAddr) -> io::Result<TcpStream> {
    let stream = TcpStream::connect(address).await?;
    stream.set_nodelay(true)?;
    Ok(stream)
}

/// One byte there and back, in microseconds.
async fn round_trip(stream: &mut TcpStream) -> io::Result<u64> {
    let stopwatch = Stopwatch::start();
    stream.write_all(b"x").await?;
    let mut byte = [0; 1];
    stream.read_exact(&mut byte).await?;
    Ok(stopwatch.microseconds())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use rstest::rstest;
    use serde_json::json;

    use super::{Calibration, Pair, TimeMetrics};

    #[rstest]
    #[case::ten_percent_over(100, 109_000, TimeMetrics::Available)]
    #[case::ten_percent_under(100, 91_000, TimeMetrics::Available)]
    #[case::fifteen_percent_over(100, 115_000, TimeMetrics::Unavailable)]
    #[case::two_milliseconds_over_none(0, 1_500, TimeMetrics::Available)]
    #[case::three_milliseconds_over_none(0, 3_000, TimeMetrics::Unavailable)]
    fn the_tolerance_is_the_larger_of_ten_percent_and_two_milliseconds(
        #[case] round_trip_milliseconds: u64,
        #[case] added_microseconds: u64,
        #[case] time_metrics: TimeMetrics,
    ) {
        let pairs = [Pair {
            direct: 100,
            shaped: 100 + added_microseconds,
        }];

        let calibration = Calibration::of(Duration::from_millis(round_trip_milliseconds), &pairs);

        assert_eq!(calibration.time_metrics(), time_metrics);
    }

    #[test]
    fn a_calibration_records_its_medians_and_the_paired_added_delay() {
        let pairs = [
            Pair {
                direct: 100,
                shaped: 20_300,
            },
            Pair {
                direct: 300,
                shaped: 20_200,
            },
            Pair {
                direct: 200,
                shaped: 20_900,
            },
            Pair {
                direct: 150,
                shaped: 20_100,
            },
            Pair {
                direct: 120,
                shaped: 20_500,
            },
        ];

        let calibration = Calibration::of(Duration::from_millis(20), &pairs);

        assert_eq!(
            serde_json::to_value(&calibration).unwrap(),
            json!({
                "round_trip_microseconds": 20_000,
                "samples": 5,
                "shaped_median_microseconds": 20_300,
                "shaped_p95_microseconds": 20_900,
                "direct_median_microseconds": 150,
                "added_median_microseconds": 20_200,
                "within_tolerance": true,
            })
        );
    }
}
