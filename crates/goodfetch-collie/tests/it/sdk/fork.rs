//! The `connection-used-after-fork` client conformance case (ADR 0078;
//! issue 3062): a Connection belongs to the process that opened it. A child
//! created by `fork` inherits a copy of its parent's Connection and of the
//! Run under it, whose engine and tasks stay the parent's: each use in the
//! child fails with `connection_forked` at once, before it sends anything,
//! telling it to connect in the child, and a Connection the child opens
//! works. Linked, the child's async uses fail; attached, the client crate's
//! Connection to the Scripted engine, playing the Attached engine, fails
//! the child's blocking uses, and the Scripted engine's record shows the
//! child sent it nothing. Python's `test_forks_and_interrupts.py` is its
//! twin. Windows has no `fork`, so the case runs on Unix alone
//! (`platforms = ["unix"]`).
#![expect(
    unsafe_code,
    reason = "the test forks its own process, which only `fork` does, and ends the child with `_exit` (STYLE.md, `unsafe`)"
)]

use std::future::{Future as _, IntoFuture};
use std::path::Path;
use std::pin::pin;
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use goodfetch_collie::{Collie, Error, RunId, Tags};
use goodfetch_collie_client::transport::{AttachedEngine, Endpoint, Network};
use goodfetch_collie_client::{ClientConnection, ConnectionRuntime, blocking};
use goodfetch_conformance::{ClientCaseRun, RustMode};
use goodfetch_protocol::run::Lifecycle;
use goodfetch_test_support::{ATTACHED_RECORD, TestEnv, case_logs, real_time_runtime};
use serde_json::{Value, json};

use super::options;
use super::scripted::{attached_hello, client_methods, install_script};

/// The Run the parent starts before it forks.
const NIGHTLY: &str = "nightly";

/// How long the child may take to report.
const PATIENCE: Duration = Duration::from_secs(60);

/// What a use of the parent's Connection fails with in the child.
const FIX: &str = "Connect in the child process: a Connection and every Run, Session or page under it belong to the process that opened them.";

/// The case, linked: in the child, starting a Run, finishing and closing
/// the parent's Run, a raw call and closing the parent's Connection each
/// fail with `connection_forked` at once, while a Connection the child
/// opens to its own linked engine starts and finishes a Run; the parent's
/// Run and Connection still work after.
/// Covers: story:11-23, error:connection_forked
#[test]
fn a_connection_used_after_fork_fails_in_the_child_which_connects_on_its_own_linked() {
    let _case = ClientCaseRun::start(
        case_logs().as_deref(),
        "connection-used-after-fork",
        RustMode::Linked,
    )
    .unwrap();
    let (env, child_env) = (TestEnv::new(), TestEnv::new());
    let nightly = RunId::parse(NIGHTLY).unwrap();
    let (collie, run) = real_time_runtime().block_on(async {
        let collie = Collie::connect(options(&env)).await.unwrap();
        let run = collie.start().run_id(&nightly).await.unwrap();
        (collie, run)
    });

    let reported = in_a_forked_child(&env.folder().join("child.json"), || {
        let uses = [
            failure(at_once(collie.start()).map(drop)),
            failure(at_once(run.finish()).map(drop)),
            failure(at_once(collie.call("config.explain")).map(drop)),
            failure(at_once(run.close())),
            failure(at_once(collie.close())),
        ];
        let own =
            goodfetch_collie::blocking::Collie::connect(options(&child_env)).and_then(|own| {
                let report = own.start().send()?.finish().send()?;
                own.close()?;
                Ok(report.lifecycle)
            });
        json!({ "uses": uses, "own": format!("{own:?}") })
    });

    let parent = real_time_runtime().block_on(async {
        let report = run.finish().await.map(|report| report.lifecycle);
        (report, collie.close().await)
    });
    assert_eq!(
        reported,
        json!({
            "uses": [
                forked(Some("run.start")),
                forked(Some("run.finish")),
                forked(Some("config.explain")),
                forked(Some("run.abort")),
                forked(None),
            ],
            "own": "Ok(Finished)",
        })
    );
    assert!(
        matches!(parent, (Ok(Lifecycle::Finished), Ok(()))),
        "{parent:?}"
    );
}

/// The case, attached: in the child, starting a Run, finishing and closing
/// the parent's Run, a raw call and closing the parent's Connection, each
/// through `blocking`, fail with `connection_forked`, naming each call's
/// operation as async calls do, and send the Scripted engine nothing,
/// while the child's own Connection to a Scripted engine of its own starts
/// and finishes a Run; the parent's close of its Run still reaches its
/// engine.
/// Covers: story:11-23, error:connection_forked
#[test]
fn a_connection_used_after_fork_fails_in_the_child_which_connects_on_its_own_attached() {
    let (env, child_env) = (TestEnv::new(), TestEnv::new());
    let engine = scripted_engine(&env, "run/closed-while-running.json");
    let child_engine = scripted_engine(&child_env, "run/started-and-finished.json");
    let nightly = RunId::parse(NIGHTLY).unwrap();
    let connection = blocking::ClientConnection::from(open(&engine));
    let run = connection
        .start()
        .run_id(&nightly)
        .tags(Tags::parse(["shop"]).unwrap())
        .send()
        .unwrap();

    let reported = in_a_forked_child(&env.folder().join("child.json"), || {
        let uses = [
            failure(connection.start().send().map(drop)),
            failure(run.finish().send().map(drop)),
            failure(connection.raw_call("config.explain").send().map(drop)),
            failure(run.close()),
            failure(connection.close()),
        ];
        let own = blocking::ClientConnection::from(open(&child_engine));
        let lifecycle = own
            .start()
            .send()
            .and_then(|run| run.finish().send())
            .map(|report| report.lifecycle);
        let closed = own.close();
        json!({ "uses": uses, "own": format!("{lifecycle:?} {closed:?}") })
    });

    let closed = (run.close(), connection.close());
    assert_eq!(
        reported,
        json!({
            "uses": [
                forked(Some("run.start")),
                forked(Some("run.finish")),
                forked(Some("config.explain")),
                forked(Some("run.abort")),
                forked(None),
            ],
            "own": "Ok(Finished) Ok(())",
        })
    );
    assert!(matches!(closed, (Ok(()), Ok(()))), "{closed:?}");
    assert_eq!(
        client_methods(&env.home().join(ATTACHED_RECORD)),
        ["hello", "run.start", "run.abort"]
    );
    assert_eq!(
        client_methods(&child_env.home().join(ATTACHED_RECORD)),
        ["hello", "run.start", "run.finish"]
    );
}

/// What a use of the parent's Connection in the child fails with: the
/// call's operation, when it names one, then the fix.
fn forked(operation: Option<&str>) -> Value {
    json!({
        "code": "connection_forked",
        "group": "Program",
        "retryable": false,
        "operation": operation,
        "fix": FIX,
    })
}

/// How `used` ended, as the child reports it.
fn failure(used: Result<(), Error>) -> Value {
    match used {
        Ok(()) => json!("succeeded"),
        Err(error) => json!({
            "code": error.code().as_str(),
            "group": format!("{:?}", error.group()),
            "retryable": error.retryable(),
            "operation": error.operation(),
            "fix": error.fix(),
        }),
    }
}

/// What `future` ends with on its first poll, which no runtime drives: in
/// the child, the parent's runtime has no thread left to answer it.
fn at_once<F: IntoFuture>(future: F) -> F::Output {
    let mut future = pin!(future.into_future());
    match future
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
    {
        Poll::Ready(output) => output,
        Poll::Pending => panic!("the use waits for the parent's tasks"),
    }
}

/// Forks the test's process, runs `child` in the child, which writes what
/// it returns to `report` and exits without running any destructor of the
/// parent's values, and returns that report once the child exited, or
/// fails once [`PATIENCE`] ran out, the child then killed.
#[expect(
    clippy::disallowed_methods,
    reason = "the child writes its report, and the parent reads it, outside any async worker"
)]
fn in_a_forked_child(report: &Path, child: impl FnOnce() -> Value) -> Value {
    // SAFETY: the child runs only `child`, whose panic it catches, then
    // `_exit`s: it never returns past the fork into the test harness.
    let pid = unsafe { libc::fork() };
    assert!(pid >= 0, "{}", std::io::Error::last_os_error());
    if pid == 0 {
        let reported = std::panic::catch_unwind(std::panic::AssertUnwindSafe(child))
            .unwrap_or_else(|panic| {
                let message = panic
                    .downcast_ref::<&str>()
                    .map(ToString::to_string)
                    .or_else(|| panic.downcast_ref::<String>().cloned());
                json!({ "panicked": message })
            });
        let written = std::fs::write(report, reported.to_string());
        // SAFETY: `_exit` ends the child at once, running no destructor
        // and no handler `atexit` registered in the parent.
        unsafe { libc::_exit(i32::from(written.is_err())) }
    }
    let status = waited(pid);
    assert_eq!(status, Some(0), "the child ended with {status:?}");
    serde_json::from_str(&std::fs::read_to_string(report).unwrap()).unwrap()
}

/// The exit code of the child `pid` once it exited, `None` when a signal
/// ended it, or a failure once [`PATIENCE`] ran out, the child then killed.
#[expect(
    clippy::disallowed_methods,
    reason = "the parent waits for its child on its own thread, outside any async worker"
)]
fn waited(pid: libc::pid_t) -> Option<i32> {
    let mut status = 0;
    let mut waited = Duration::ZERO;
    loop {
        // SAFETY: `status` outlives the call, which writes one `c_int`.
        let ended = unsafe { libc::waitpid(pid, &raw mut status, libc::WNOHANG) };
        assert!(ended >= 0, "{}", std::io::Error::last_os_error());
        if ended == pid {
            return libc::WIFEXITED(status).then(|| libc::WEXITSTATUS(status));
        }
        if waited >= PATIENCE {
            // SAFETY: `pid` is the child this test forked, not yet reaped.
            unsafe {
                libc::kill(pid, libc::SIGKILL);
                libc::waitpid(pid, &raw mut status, 0);
            }
            panic!("the child did not report within {PATIENCE:?}: it waits for its parent's tasks");
        }
        std::thread::sleep(Duration::from_millis(10));
        waited += Duration::from_millis(10);
    }
}

/// The client crate's Connection to the Attached engine `engine`, on a
/// runtime it owns, so it works between the test's blocking calls.
fn open(engine: &Endpoint) -> ClientConnection {
    let one = std::num::NonZeroUsize::MIN;
    let owned = goodfetch_runtime::named(goodfetch_runtime::Threads {
        workers: Some(one),
        blocking: one,
    })
    .unwrap();
    real_time_runtime().block_on(async {
        let (connection, _said) = ClientConnection::open(
            ConnectionRuntime::owned(owned),
            &Network::Os,
            engine,
            attached_hello(),
        )
        .await
        .unwrap();
        connection
    })
}

/// The Scripted engine as the Attached engine of `env`'s home, playing
/// `script` with this crate's `hello`.
fn scripted_engine(env: &TestEnv, script: &str) -> Endpoint {
    Endpoint::Attached(AttachedEngine {
        executable: install_script(env, script),
        environment: env.env_os(),
    })
}
