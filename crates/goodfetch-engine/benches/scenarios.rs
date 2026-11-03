//! The engine's benchmark scenarios (ADR 0071), shared by its Callgrind and
//! DHAT targets: framing and decoding a fixed batch of lines, and
//! dispatching `hello` then a fixed number of `engine.info` calls over the
//! in-memory network on the tests' paused current-thread runtime, on a
//! Goodfetch home made and removed outside the measured run, and
//! writing a fixed number of rows through a run file's writer, queued
//! together so they commit in one group, with SQLite's full sync, in a
//! temporary folder (ADR 0071's run-state writes), and judging a fixed batch
//! of URLs under a domain policy of a thousand entries (spec #564's egress
//! decision), and masking a 1 MiB stream of event and log lines against
//! fifty resolved values, and joining one value a thousand times (spec
//! #564's `Mask`). Inputs are literals; no network, no sleep, no work whose
//! amount depends on timing.

use std::io::Write as _;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::Path;

use goodfetch_engine::clock::Clock;
use goodfetch_engine::egress::{Judgment, Policy, Request, RequestKind, judge, judge_addresses};
use goodfetch_engine::entropy::Entropy;
use goodfetch_engine::home::{Home, RunStateLocation};
use goodfetch_engine::mask::{Mask, Masks, Reach, Redaction, Scope};
use goodfetch_engine::rpc::framing::{Frame, Framer, MAX_MESSAGE_BYTES, decode};
use goodfetch_engine::run_state::{RunState, RunStateOptions, RunStateTables};
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_engine::transport::{self, Network};
use goodfetch_engine::{Channel, Composition, Dependencies, Engine, Identity};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::ProductVersion;
use secrecy::SecretString;
use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};

const HELLO: &[u8] = br#"{"jsonrpc":"2.0","id":0,"method":"hello","params":{"protocol_version":"0.1","surface":"rust","needs":"collie","product_version":"0.0.0"}}"#;

/// `count` request lines of `engine.info`, ids 1 to `count`.
pub(crate) fn engine_info_lines(count: usize) -> Vec<u8> {
    let mut lines = Vec::new();
    for id in 1..=count {
        lines.extend_from_slice(
            format!(r#"{{"jsonrpc":"2.0","id":{id},"method":"engine.info","params":{{}}}}"#)
                .as_bytes(),
        );
        lines.push(b'\n');
    }
    lines
}

/// Frames `bytes` and decodes each line; returns how many decoded.
pub(crate) fn frame_and_decode(bytes: &[u8]) -> usize {
    let work = goodfetch_counted_work::CountedWork::default();
    let mut framer = Framer::new(MAX_MESSAGE_BYTES);
    let mut lines = Vec::new();
    framer.feed(bytes, &mut lines);
    lines
        .iter()
        .filter(|frame| matches!(frame, Frame::Line(line) if decode(line, &work).is_ok()))
        .count()
}

/// The dispatch benchmark's input: `count` lines from [`engine_info_lines`]
/// and the Goodfetch home the engine runs on, a temporary folder its setup
/// makes and its teardown removes, so neither shows in its figures.
pub(crate) fn dispatch_input(count: usize) -> (Vec<u8>, TempDir) {
    let home = tempfile::tempdir().expect("invariant: the OS gives a temporary folder");
    (engine_info_lines(count), home)
}

/// The dispatch benchmark's teardown: removes its home, and returns how many
/// replies came back.
pub(crate) fn dispatch_done((replies, home): (usize, TempDir)) -> usize {
    drop(home);
    replies
}

/// Opens a Connection to a Collie engine with only the engine's own
/// operations on the Goodfetch home `home`, sends `hello` and `calls` (lines
/// from [`engine_info_lines`]), and returns how many replies came back.
pub(crate) fn dispatch(calls: &[u8], home: &Path) -> usize {
    let identity = Identity {
        product: Product::Collie,
        product_version: ProductVersion::parse("0.0.0").expect("invariant: a literal version"),
        channel: Channel::CratesIo,
    };
    let composition = Composition::default().with(goodfetch_engine::contribution());
    let engine = Engine::new(identity, &composition).expect("invariant: one handler each");
    goodfetch_test_support::runtime().block_on(async {
        let (network, dialer) = transport::in_memory();
        let client = async {
            let (reader, mut writer) = dialer
                .connect()
                .await
                .expect("invariant: the engine listens")
                .split();
            let send = async {
                writer.write_all(HELLO).await.expect("invariant: in memory");
                writer.write_all(b"\n").await.expect("invariant: in memory");
                writer.write_all(calls).await.expect("invariant: in memory");
                writer.shutdown().await.expect("invariant: in memory");
            };
            let receive = async {
                let mut lines = BufReader::new(reader).lines();
                let mut replies = 0;
                while let Ok(Some(_)) = lines.next_line().await {
                    replies += 1;
                }
                replies
            };
            tokio::join!(send, receive).1
        };
        let dependencies = Dependencies {
            environment: std::collections::BTreeMap::new(),
            keychain: goodfetch_engine::secrets::Keychain::absent(),
            home: Home::new(home.to_owned()),
            clock: Clock::paused(std::time::UNIX_EPOCH),
            entropy: Entropy::seeded(0),
            blocking: BlockingPolicy::Inline,
            run_state_location: RunStateLocation::PerMajor,
            run_state: RunStateOptions::default(),
            politeness: goodfetch_engine::politeness::PolitenessOptions::default(),
        };
        let (served, replies) = tokio::join!(
            engine.run_attached(Network::InMemory(network), dependencies),
            client
        );
        served.expect("invariant: the engine got its Connection");
        replies
    })
}

/// A table the run-state benchmark writes.
const ROWS: RunStateTables = RunStateTables {
    name: "bench",
    statements: &[
        "CREATE TABLE IF NOT EXISTS rows (value INTEGER NOT NULL, data TEXT NOT NULL) STRICT",
    ],
};

/// Opens a run file in a temporary folder, writes `count` rows through its
/// writer, queued together so they commit in one group, and returns how
/// many it acknowledged.
pub(crate) fn run_state_writes(count: usize) -> usize {
    let folder = tempfile::tempdir().expect("invariant: the OS gives a temporary folder");
    goodfetch_test_support::runtime().block_on(async {
        let owner = TaskOwner::root(BlockingPolicy::Inline);
        let state = RunState::open(
            folder.path().join("run.sqlite"),
            &RunStateOptions::default(),
            vec![ROWS],
            1,
            &owner,
        )
        .await
        .expect("invariant: a temporary folder takes a run file");
        let writes = (0..count).map(|value| {
            let state = state.clone();
            async move {
                state
                    .write(move |connection| {
                        connection.execute(
                            "INSERT INTO rows (value, data) VALUES (?1, ?2)",
                            (
                                i64::try_from(value).unwrap_or(i64::MAX),
                                "a Record of a few dozen bytes, as a crawl stages",
                            ),
                        )
                    })
                    .await
                    .is_ok()
            }
        });
        let acknowledged = join_all(writes)
            .await
            .into_iter()
            .filter(|acknowledged| *acknowledged)
            .count();
        drop(state);
        owner.wait().await;
        acknowledged
    })
}

/// Polls every future of `futures` on this task until each answered, so
/// the writes queue together.
async fn join_all<F: Future>(futures: impl Iterator<Item = F>) -> Vec<F::Output> {
    let mut pending: Vec<_> = futures.map(|future| Some(Box::pin(future))).collect();
    let mut answers: Vec<Option<F::Output>> = pending.iter().map(|_| None).collect();
    std::future::poll_fn(|context| {
        let mut waiting = false;
        for (slot, answer) in pending.iter_mut().zip(answers.iter_mut()) {
            if let Some(future) = slot {
                match future.as_mut().poll(context) {
                    std::task::Poll::Ready(output) => {
                        *answer = Some(output);
                        *slot = None;
                    }
                    std::task::Poll::Pending => waiting = true,
                }
            }
        }
        if waiting {
            std::task::Poll::Pending
        } else {
            std::task::Poll::Ready(())
        }
    })
    .await;
    answers
        .into_iter()
        .map(|answer| answer.expect("invariant: every future answered"))
        .collect()
}

/// A domain policy of `entries` entries, nine in ten allowed and one in ten
/// blocked, each a host of its own, and the URLs a run requests under it:
/// for each tenth entry, an allowed host, a subdomain of it, the blocked
/// host, a host no entry lists and an address in decimal.
pub(crate) fn egress_policy(entries: usize) -> (Policy, Vec<String>) {
    let host = |entry: usize| format!("site-{entry}.example.test");
    let blocked: Vec<String> = (0..entries).step_by(10).map(host).collect();
    let allowed: Vec<String> = (0..entries)
        .filter(|entry| entry % 10 != 0)
        .map(host)
        .collect();
    let policy = Policy::new(&allowed, &blocked)
        .expect("invariant: every entry the scenario writes is a host");
    let urls = (0..entries)
        .step_by(10)
        .flat_map(|entry| {
            [
                format!("https://{}/products?page=2", host(entry + 1)),
                format!("https://www.shop.{}/cart", host(entry + 1)),
                format!("https://{}/", host(entry)),
                format!("https://unlisted-{entry}.example.test/"),
                format!("http://{}/", 2_130_706_433 + entry),
            ]
        })
        .collect();
    (policy, urls)
}

/// Judges every URL of `urls` under `policy`, as requests of the engine's
/// own HTTP client, and returns how many it allowed.
pub(crate) fn judge_all(policy: &Policy, urls: &[String]) -> usize {
    urls.iter()
        .filter(|url| {
            let request = Request {
                url,
                kind: RequestKind::Fetch,
            };
            matches!(judge(&request, policy), Judgment::Allowed(_))
        })
        .count()
}

/// The domain policy of [`egress_policy`] with `ranges` entries of
/// `egress.private_ranges`, one in two a CIDR block of the private ranges
/// and one in two a host name, and the requests a run makes under it,
/// each URL with the addresses its host resolved to: [`egress_policy`]'s
/// URLs, each resolved to a public address, then for each tenth range a
/// host the list names resolved to a private address, an unlisted host
/// resolved to a private address, an address a listed block holds and a
/// cloud-metadata address.
pub(crate) fn egress_policy_with_private_ranges(
    entries: usize,
    ranges: usize,
) -> (Policy, Vec<(String, Vec<SocketAddr>)>) {
    let (policy, urls) = egress_policy(entries);
    let block = |range: usize| {
        let [high, low] = u16::try_from(range).unwrap_or(u16::MAX).to_be_bytes();
        Ipv4Addr::new(10, high, low, 0)
    };
    let private_host = |range: usize| format!("internal-{range}.corp.test");
    let listed: Vec<String> = (0..ranges)
        .map(|range| {
            if range % 2 == 0 {
                format!("{}/24", block(range))
            } else {
                private_host(range)
            }
        })
        .collect();
    let policy = policy
        .with_private_ranges(&listed)
        .expect("invariant: every range the scenario writes parses");
    let at = |address: IpAddr| vec![SocketAddr::new(address, 443)];
    let public = IpAddr::V4(Ipv4Addr::new(93, 184, 215, 14));
    let requests = urls
        .into_iter()
        .map(|url| (url, at(public)))
        .chain((0..ranges).step_by(10).flat_map(|range| {
            let inside = IpAddr::V4(Ipv4Addr::from(u32::from(block(range)) + 7));
            [
                (
                    format!("https://{}/status", private_host(range + 1)),
                    at(inside),
                ),
                (format!("https://unlisted-{range}.corp.test/"), at(inside)),
                (format!("https://{inside}/health"), at(inside)),
                (
                    "http://169.254.169.254/latest/meta-data/".to_owned(),
                    at(IpAddr::V4(Ipv4Addr::new(169, 254, 169, 254))),
                ),
            ]
        }))
        .collect();
    (policy, requests)
}

/// Judges every request of `requests` under `policy` as [`judge_all`]
/// does, then the addresses each allowed one resolved to, as its
/// connection is judged, and returns how many it allowed.
pub(crate) fn judge_all_with_addresses(
    policy: &Policy,
    requests: &[(String, Vec<SocketAddr>)],
) -> usize {
    requests
        .iter()
        .filter(|(url, addresses)| {
            let request = Request {
                url,
                kind: RequestKind::Fetch,
            };
            matches!(judge(&request, policy), Judgment::Allowed(_))
                && judge_addresses(url, addresses, policy).is_ok()
        })
        .count()
}

/// What masking a stream measures: the engine's masks holding `values`
/// resolved values of one Run, each of 10 to 25 characters, the Run's
/// mask that keeps them, and a stream of `bytes` bytes of alternating
/// event and log lines, one line in four holding a value raw, in base64 or
/// in hex (spec #564's `Mask` budget).
pub(crate) fn mask_input(values: usize, bytes: usize) -> (Masks, Mask, Vec<u8>) {
    let masks = Masks::new();
    let mask = masks.for_run(Redaction::On);
    let value = |number: usize| format!("value-{number:03}-{}", "x".repeat(number % 16));
    for number in 0..values {
        mask.join(
            &format!("secret_{number}"),
            &SecretString::from(value(number)),
            Reach::Value,
        );
    }
    let mut stream = Vec::with_capacity(bytes + 256);
    let mut line = 0_usize;
    while stream.len() < bytes {
        let text = match line % 8 {
            0 => value(line % values),
            4 => data_encoding::BASE64.encode(format!("user:{}", value(line % values)).as_bytes()),
            6 => data_encoding::HEXLOWER.encode(value(line % values).as_bytes()),
            _ => format!("page {line} of the catalog, 24 items in stock"),
        };
        let written = if line.is_multiple_of(2) {
            format!(r#"{{"event":"page.console","data":{{"text":"{text}","line":{line}}}}}"#)
        } else {
            format!("2026-10-24T12:00:00Z INFO goodfetch::page: read {text}")
        };
        stream.extend_from_slice(written.as_bytes());
        stream.push(b'\n');
        line += 1;
    }
    stream.truncate(bytes);
    (masks, mask, stream)
}

/// Writes `stream` through a writer of `masks`, as the log writers do, in
/// chunks of 4 KiB, and returns how many bytes it wrote.
pub(crate) fn mask_stream(masks: &Masks, stream: &[u8]) -> usize {
    let mut writer = masks.writer(Vec::with_capacity(stream.len()), Scope::KeptOutput);
    for chunk in stream.chunks(4096) {
        writer
            .write_all(chunk)
            .expect("invariant: a Vec takes every write");
    }
    writer
        .finish()
        .expect("invariant: a Vec takes every write")
        .len()
}

/// Joins one resolved value of one Run `joins` times, as each Session's
/// open resolves the same proxy password (spec #564's `Mask`: a value
/// already in it does not join again), and returns how many joins added
/// it.
pub(crate) fn mask_joins_of_one_value(joins: usize) -> usize {
    let masks = Masks::new();
    let mask = masks.for_run(Redaction::On);
    let value = SecretString::from("a proxy password, 25 long");
    (0..joins)
        .filter(|_| mask.join("proxy.password", &value, Reach::Value))
        .count()
}
