//! Toxiproxy between a test and a stand-in on loopback (ADR 0060): a cut
//! proxy refuses connections, latency delays each answer by at least the set
//! amount, a partition drops data, and `heal` restores each. The Toxiproxy
//! process joins the test's record and is gone once the value drops. The
//! proxies on extra loopback addresses are in `extra_loopback`.
#![expect(
    clippy::disallowed_types,
    reason = "the tests serve an echo stand-in and reach it through Toxiproxy over plain loopback sockets"
)]

use std::io::{self, BufRead as _, BufReader, Write as _};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::{Duration, Instant};

use goodfetch_test_support::{ProcessesAlive, TestEnv, Toxiproxy, ToxiproxyError};

/// The loopback address every test without extra addresses uses.
pub(crate) const LOOPBACK: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);

/// How long an answer that should come may take.
pub(crate) const ANSWER_DEADLINE: Duration = Duration::from_secs(10);

/// How long a partitioned link stays silent before the test judges it
/// silent: shorter than the deadline above, since only the silence is
/// awaited.
pub(crate) const SILENCE: Duration = Duration::from_secs(1);

/// The latency the test sets.
const LATENCY: Duration = Duration::from_millis(300);

/// Starts an echo stand-in on loopback, on threads of its own for as long as
/// the test's process runs, and answers its address: each line a connection
/// sends comes back as it was sent.
pub(crate) fn echo() -> SocketAddr {
    let listener = TcpListener::bind((LOOPBACK, 0)).unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming().map_while(Result::ok) {
            std::thread::spawn(move || -> io::Result<()> {
                let mut writer = stream.try_clone()?;
                for line in BufReader::new(stream).lines() {
                    writeln!(writer, "{}", line?)?;
                }
                Ok(())
            });
        }
    });
    address
}

/// What a line sent to `address` meets.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Reach {
    /// The connection is refused.
    Refused,
    /// The line comes back within the wait.
    Answered,
    /// Nothing comes back within the wait.
    Silent,
}

/// Connects to `address`, sends a line and waits up to `wait` for it to come
/// back.
pub(crate) fn reach(address: SocketAddr, wait: Duration) -> Reach {
    let stream = match TcpStream::connect_timeout(&address, ANSWER_DEADLINE) {
        Ok(stream) => stream,
        Err(error) if error.kind() == io::ErrorKind::ConnectionRefused => return Reach::Refused,
        Err(error) => panic!("connecting to {address} failed otherwise than refused: {error}"),
    };
    stream.set_read_timeout(Some(wait)).unwrap();
    writeln!(&stream, "ping").unwrap();
    let mut line = String::new();
    match BufReader::new(&stream).read_line(&mut line) {
        Ok(_) if line == "ping\n" => Reach::Answered,
        Ok(_) => panic!("{address} answered {line:?}, not the line sent"),
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
            ) =>
        {
            Reach::Silent
        }
        Err(error) => panic!("reading from {address} failed: {error}"),
    }
}

/// How long a line sent to `address` takes to come back.
fn answered_after(address: SocketAddr) -> Duration {
    let started = Instant::now();
    assert_eq!(reach(address, ANSWER_DEADLINE), Reach::Answered);
    started.elapsed()
}

#[test]
fn a_cut_proxy_refuses_connections_until_healed() {
    let env = TestEnv::new();
    let toxiproxy = Toxiproxy::start(&env).unwrap();
    let proxy = toxiproxy.proxy("store", echo(), LOOPBACK).unwrap();
    assert_eq!(proxy.ip(), LOOPBACK);
    assert_eq!(reach(proxy, ANSWER_DEADLINE), Reach::Answered);

    toxiproxy.cut("store").unwrap();
    assert_eq!(reach(proxy, ANSWER_DEADLINE), Reach::Refused);

    toxiproxy.heal("store").unwrap();
    assert_eq!(reach(proxy, ANSWER_DEADLINE), Reach::Answered);
}

#[test]
fn latency_delays_an_answer_by_at_least_the_set_amount() {
    let env = TestEnv::new();
    let toxiproxy = Toxiproxy::start(&env).unwrap();
    let proxy = toxiproxy.proxy("store", echo(), LOOPBACK).unwrap();

    toxiproxy.latency("store", LATENCY).unwrap();
    let slowed = answered_after(proxy);
    assert!(slowed >= LATENCY, "answered after {slowed:?}");

    toxiproxy.heal("store").unwrap();
    let healed = answered_after(proxy);
    assert!(healed < LATENCY, "still answered after {healed:?}");
}

#[test]
fn a_partitioned_proxy_drops_data_until_healed() {
    let env = TestEnv::new();
    let toxiproxy = Toxiproxy::start(&env).unwrap();
    let proxy = toxiproxy.proxy("peer", echo(), LOOPBACK).unwrap();

    toxiproxy.partition("peer").unwrap();
    assert_eq!(reach(proxy, SILENCE), Reach::Silent);

    toxiproxy.heal("peer").unwrap();
    assert_eq!(reach(proxy, ANSWER_DEADLINE), Reach::Answered);
}

#[test]
fn a_call_on_an_unknown_proxy_fails_with_toxiproxy_s_answer() {
    let env = TestEnv::new();
    let toxiproxy = Toxiproxy::start(&env).unwrap();

    let error = toxiproxy.cut("nowhere").unwrap_err();

    assert!(
        matches!(error, ToxiproxyError::Refused { status: 404, .. }),
        "{error}"
    );
}

#[test]
fn a_program_that_cannot_start_is_named() {
    let env = TestEnv::new();
    let program = env.folder().join("no-toxiproxy-here");

    let error = Toxiproxy::start_program(&env, &program).unwrap_err();

    assert!(
        matches!(&error, ToxiproxyError::Start { program: named, .. } if named == &program),
        "{error}"
    );
}

#[test]
fn toxiproxy_is_gone_once_the_value_drops() {
    let env = TestEnv::new();
    let toxiproxy = Toxiproxy::start(&env).unwrap();
    let process = toxiproxy.process().clone();

    drop(toxiproxy);

    assert!(process.wait_within(Duration::ZERO).unwrap().is_some());
}

#[test]
fn toxiproxy_left_running_fails_the_teardown() {
    let mut program = None;

    let failure = catch_unwind(AssertUnwindSafe(|| {
        let env = TestEnv::new();
        let toxiproxy = Toxiproxy::start(&env).unwrap();
        program = Some(toxiproxy.process().program().to_owned());
        // Its process stays alive past its value, as a leaked one would.
        std::mem::forget(toxiproxy);
    }))
    .unwrap_err();

    let alive = failure.downcast::<ProcessesAlive>().unwrap();
    let program = program.unwrap();
    assert!(program.starts_with("toxiproxy-server"), "{program}");
    assert_eq!(alive.programs, [program]);
}
