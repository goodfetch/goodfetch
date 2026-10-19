//! One direction of a shaped connection, the lab's latency proxy ported: its
//! bytes leave in the order they arrived, each chunk half the profile's round
//! trip after it arrived, so the two directions add the whole round trip. A
//! lost chunk leaves a retransmission timeout later and holds back every
//! chunk behind it, as TCP's in-order delivery does; which chunks are lost is
//! drawn from the seed. Each delay is scheduled on Tokio's clock, which a test
//! pauses.

use std::io;
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};
use tokio::sync::mpsc;

use crate::clock;
use crate::generator;

use super::Profile;

/// The most bytes read at once.
const CHUNK: usize = 16 * 1024;

/// The most chunks in flight in one direction; a full queue stops reading,
/// as a full TCP window does (STYLE.md: every buffer is bounded).
const IN_FLIGHT: usize = 1024;

/// Linux's least retransmission timeout, which a lost chunk waits on top of
/// the round trip (RFC 6298's timeout with Linux's 200 ms floor).
const LEAST_RETRANSMISSION_TIMEOUT: Duration = Duration::from_millis(200);

/// The place in the seed's draws where the loss of chunks is drawn.
const LOSS_PLACE: u64 = 0x4c4f_5353;

/// Which way a link carries a connection's bytes.
#[derive(Clone, Copy)]
pub(crate) enum Direction {
    /// From the client to the Proving ground.
    Up = 0,
    /// From the Proving ground to the client.
    Down = 1,
}

/// One direction of one shaped connection.
pub(crate) struct Link {
    delay: Duration,
    retransmission: Duration,
    loss_per_million: u32,
    seed: u64,
    connection: u64,
    direction: Direction,
}

impl Link {
    /// The link of `profile` in `direction` of the `connection`-th
    /// connection, its losses drawn from `seed`.
    pub(crate) fn new(profile: &Profile, seed: u64, connection: u64, direction: Direction) -> Self {
        Self {
            delay: profile.round_trip / 2,
            retransmission: LEAST_RETRANSMISSION_TIMEOUT + profile.round_trip,
            loss_per_million: profile.loss.per_million(),
            seed,
            connection,
            direction,
        }
    }

    /// Forwards `source`'s bytes to `target` until `source` ends, then shuts
    /// `target` down once the last chunk has left.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future drops the chunks still in flight.
    pub(crate) async fn forward<R, W>(&self, source: R, target: W) -> io::Result<()>
    where
        R: AsyncRead + Unpin,
        W: AsyncWrite + Unpin,
    {
        let (queue, mut in_flight) = mpsc::channel(IN_FLIGHT);
        let mut source = source;
        let mut target = target;
        let read = async move {
            let mut ordinal = 0_u64;
            loop {
                let mut chunk = vec![0; CHUNK];
                let read = source.read(&mut chunk).await?;
                if read == 0 {
                    break;
                }
                chunk.truncate(read);
                let mut due = clock::now() + self.delay;
                if self.lost(ordinal) {
                    due += self.retransmission;
                }
                ordinal += 1;
                if queue.send((due, chunk)).await.is_err() {
                    break;
                }
            }
            Ok::<_, io::Error>(())
        };
        let write = async move {
            while let Some((due, chunk)) = in_flight.recv().await {
                tokio::time::sleep_until(due).await;
                target.write_all(&chunk).await?;
            }
            target.shutdown().await
        };
        tokio::try_join!(read, write).map(drop)
    }

    /// Whether the `ordinal`-th chunk is lost, drawn from the seed.
    fn lost(&self, ordinal: u64) -> bool {
        self.loss_per_million > 0
            && generator::below(
                self.seed,
                &[LOSS_PLACE, self.connection, self.direction as u64, ordinal],
                1_000_000,
            ) < u64::from(self.loss_per_million)
    }
}

#[cfg(test)]
#[expect(
    clippy::disallowed_methods,
    reason = "each test runs on the paused runtime `tokio::test` builds, TESTING.md's test Clock (ADR 0060)"
)]
mod tests {
    use std::time::Duration;

    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _, duplex};

    use super::{Direction, Link};
    use crate::clock;
    use crate::network::{Loss, Profile};

    fn link(round_trip_milliseconds: u64, loss: &str) -> Link {
        let profile = Profile::new(
            "test",
            Duration::from_millis(round_trip_milliseconds),
            loss.parse::<Loss>().unwrap(),
        );
        Link::new(&profile, 160, 0, Direction::Up)
    }

    #[tokio::test(start_paused = true)]
    async fn a_chunk_leaves_half_the_round_trip_after_it_arrived() {
        let (mut client, inbound) = duplex(1024);
        let (outbound, mut server) = duplex(1024);
        let link = link(100, "0");
        let sent = clock::now();

        let (forwarded, received) = tokio::join!(link.forward(inbound, outbound), async {
            client.write_all(b"ping").await.unwrap();
            drop(client);
            let mut bytes = Vec::new();
            server.read_to_end(&mut bytes).await.unwrap();
            (bytes, clock::now() - sent)
        });

        forwarded.unwrap();
        assert_eq!(received, (b"ping".to_vec(), Duration::from_millis(50)));
    }

    /// Writes each chunk after its pause, then ends; returns what the far
    /// end read, each read with the milliseconds since the first write.
    async fn send(link: &Link, chunks: &[(u64, &[u8])]) -> Vec<(Vec<u8>, u128)> {
        let (mut client, inbound) = duplex(1024);
        let (outbound, mut server) = duplex(1024);
        let start = clock::now();
        let write = async {
            for (pause, chunk) in chunks {
                tokio::time::sleep(Duration::from_millis(*pause)).await;
                client.write_all(chunk).await.unwrap();
            }
            drop(client);
        };
        let read = async {
            let mut reads = Vec::new();
            let mut buffer = [0; 64];
            loop {
                let read = server.read(&mut buffer).await.unwrap();
                if read == 0 {
                    return reads;
                }
                reads.push((buffer[..read].to_vec(), (clock::now() - start).as_millis()));
            }
        };
        let (forwarded, (), reads) = tokio::join!(link.forward(inbound, outbound), write, read);
        forwarded.unwrap();
        reads
    }

    #[tokio::test(start_paused = true)]
    async fn chunks_leave_in_order_each_at_its_own_time() {
        let link = link(100, "0");

        let reads = send(&link, &[(0, b"first"), (30, b"second")]).await;

        assert_eq!(reads, [(b"first".to_vec(), 50), (b"second".to_vec(), 80)]);
    }

    #[tokio::test(start_paused = true)]
    async fn a_zero_round_trip_adds_no_delay() {
        let link = link(0, "0");

        let reads = send(&link, &[(0, b"ping")]).await;

        assert_eq!(reads, [(b"ping".to_vec(), 0)]);
    }

    #[tokio::test(start_paused = true)]
    async fn a_lost_chunk_waits_a_retransmission_timeout_and_holds_back_the_next() {
        // Every chunk is lost: each waits 200 ms plus the round trip on top
        // of its half round trip, and the second leaves after the first.
        let link = link(100, "100");

        let reads = send(&link, &[(0, b"first"), (30, b"second")]).await;

        assert_eq!(reads, [(b"first".to_vec(), 350), (b"second".to_vec(), 380)]);
    }

    #[tokio::test(start_paused = true)]
    async fn the_same_seed_loses_the_same_chunks() {
        let link = link(100, "50");
        let chunks: Vec<(u64, &[u8])> = (0..8).map(|_| (10, b"x".as_slice())).collect();

        let first = send(&link, &chunks).await;
        let second = send(&link, &chunks).await;

        assert_eq!(first, second);
    }
}
