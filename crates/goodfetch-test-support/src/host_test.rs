//! A host program played by an ignored test of the running test binary
//! (ADR 0078): the test starts its own executable with that one test, and
//! reads back what the host wrote to stdout and stderr past what the Rust
//! test harness itself writes there, for the checks that a program using
//! the SDK sees nothing of it on either stream. TypeScript's and Python's
//! twins run a fixture program of their own.

use std::ffi::OsStr;
use std::io;
use std::process::{ExitStatus, Stdio};
use std::time::Duration;

use crate::env::TestEnv;

/// How long a host test may run before it counts as hung.
pub const HOST_DEADLINE: Duration = Duration::from_secs(60);

/// What a host test's process did.
#[derive(Debug)]
pub struct HostRun {
    /// How it exited.
    pub status: ExitStatus,
    /// What it wrote to stdout past the harness's own lines.
    pub stdout: String,
    /// What it wrote to stderr, all of it: the harness writes nothing there
    /// for a test that passes.
    pub stderr: String,
}

/// Runs the ignored test `host` of the running test binary, named by its
/// whole path (`logging::unlogged_host`), alone, with the test's
/// environment and `variables`, and returns what it wrote to stdout past the
/// harness's own lines and to stderr.
///
/// # Errors
///
/// When it cannot start or be read, or ran past [`HOST_DEADLINE`]
/// (`TimedOut`, once killed).
#[expect(
    clippy::disallowed_methods,
    reason = "a test helper reads its child's pipes on threads of its own, outside any async worker (ADR 0062)"
)]
pub fn run_host_test(
    env: &TestEnv,
    host: &str,
    variables: &[(&str, &OsStr)],
) -> io::Result<HostRun> {
    let mut command = env.process_command(std::env::current_exe()?);
    command
        .args([
            host,
            "--exact",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .envs(variables.iter().copied())
        .stderr(Stdio::piped());
    let process = env.spawn(&mut command)?;
    drop(process.child().stdin.take());
    let readers = {
        let mut child = process.child();
        let stdout = child
            .stdout
            .take()
            .map(|pipe| std::thread::spawn(|| read_all(pipe)));
        let stderr = child
            .stderr
            .take()
            .map(|pipe| std::thread::spawn(|| read_all(pipe)));
        (stdout, stderr)
    };
    let Some(status) = process.wait_within(HOST_DEADLINE)? else {
        process.stop()?;
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            format!("the host test {host} ran past {HOST_DEADLINE:?}"),
        ));
    };
    let joined = |reader: Option<std::thread::JoinHandle<io::Result<String>>>| {
        reader.map_or_else(
            || Ok(String::new()),
            |reader| {
                reader
                    .join()
                    .map_err(|_| io::Error::other("a pipe's reader panicked"))?
            },
        )
    };
    let stdout = joined(readers.0)?;
    let stderr = joined(readers.1)?;
    Ok(HostRun {
        status,
        stdout: past_the_harness(&stdout, host).to_owned(),
        stderr,
    })
}

/// Reads `pipe` to its end, lossily as UTF-8.
fn read_all(mut pipe: impl io::Read) -> io::Result<String> {
    let mut bytes = Vec::new();
    pipe.read_to_end(&mut bytes)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// What `stdout`, the output of the harness running the one test `host` on
/// one thread with its output shown, holds past the harness's own lines:
/// the text between `test <host> ... ` and the `ok` of a test that passed.
/// Output the harness did not frame so is returned whole, so a check that
/// it is empty fails on it.
#[must_use]
pub fn past_the_harness<'a>(stdout: &'a str, host: &str) -> &'a str {
    let start = format!("\nrunning 1 test\ntest {host} ... ");
    let Some(rest) = stdout.strip_prefix(&start) else {
        return stdout;
    };
    match rest.rfind("ok\n\ntest result: ok. 1 passed;") {
        Some(end) => &rest[..end],
        None => stdout,
    }
}

#[cfg(test)]
mod tests {
    use super::past_the_harness;

    const HOST: &str = "logging::unlogged_host";

    fn framed(written: &str) -> String {
        format!(
            "\nrunning 1 test\ntest {HOST} ... {written}ok\n\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 41 filtered out; finished in 0.21s\n\n"
        )
    }

    #[test]
    fn a_host_that_writes_nothing_leaves_nothing_past_the_harness() {
        assert_eq!(past_the_harness(&framed(""), HOST), "");
    }

    #[test]
    fn what_a_host_writes_is_all_that_is_left_with_or_without_a_newline() {
        assert_eq!(past_the_harness(&framed("leak"), HOST), "leak");
        assert_eq!(
            past_the_harness(&framed("a line\nok\n"), HOST),
            "a line\nok\n"
        );
    }

    #[test]
    fn output_the_harness_did_not_frame_is_returned_whole() {
        let failed = "\nrunning 1 test\ntest logging::unlogged_host ... FAILED\n";
        assert_eq!(past_the_harness(failed, HOST), failed);
        assert_eq!(past_the_harness("stray\n", HOST), "stray\n");
        let other = framed("").replace(HOST, "logging::other_host");
        assert_eq!(past_the_harness(&other, HOST), other);
    }
}
