//! The processes a test starts (ADR 0069): the record every started process
//! joins, which stops each one still alive at teardown and fails the test,
//! and the address a child reports on its first line of output.

use std::cell::{RefCell, RefMut};
use std::fmt;
use std::io::{self, Read as _};
use std::path::Path;
use std::process::{Child, ChildStdout, ExitStatus};
use std::rc::Rc;
use std::sync::mpsc;
use std::time::Duration;

/// Every process a test started through test support, so none outlives its
/// test unnoticed.
#[derive(Default)]
pub struct ProcessTracker {
    processes: RefCell<Vec<Process>>,
}

impl ProcessTracker {
    /// Records a started child, and returns the handle the test keeps.
    pub fn record(&self, program: String, child: Child) -> Process {
        let process = Process {
            program,
            child: Rc::new(RefCell::new(child)),
        };
        self.processes.borrow_mut().push(process.clone());
        process
    }

    /// Stops every recorded process that is still alive, and returns their
    /// programs, in the order they started.
    pub fn stop_alive(&self) -> Vec<String> {
        let mut alive = Vec::new();
        for process in self.processes.borrow().iter() {
            let mut child = process.child();
            if matches!(child.try_wait(), Ok(None)) {
                // Best effort: a kill or wait that fails leaves nothing more
                // to do, and the test fails either way.
                let _ = child.kill();
                let _ = child.wait();
                alive.push(process.program.clone());
            }
        }
        alive
    }
}

/// How long a process running from a test's home has to end on its own at
/// teardown before it is killed: one whose stop was asked, such as a Daemon
/// whose `daemon stop` answered once it released its serving lock, may
/// still be exiting.
#[cfg(target_os = "linux")]
const FROM_HOME_DEADLINE: Duration = Duration::from_secs(5);

/// Kills every process running from `home`, its executable or its working
/// folder inside it, once [`FROM_HOME_DEADLINE`] passed without it ending,
/// and returns each one as `<program> (process <pid>, running from the
/// home)`: a process test support did not start, so not in the record,
/// such as the Daemon a CLI command starts in a process group of its own
/// (issue 1797). It reads `/proc`, so it runs on Linux only.
#[cfg(target_os = "linux")]
#[expect(
    clippy::disallowed_methods,
    reason = "a test helper polls for processes outside any async worker (ADR 0062)"
)]
pub(crate) fn stop_running_from(home: &Path) -> Vec<String> {
    // `/proc` names each path resolved; a home that cannot be is searched
    // as it is.
    let home = home.canonicalize().unwrap_or_else(|_| home.to_owned());
    let mut running = running_from(&home);
    let mut waited = Duration::ZERO;
    while !running.is_empty() && waited < FROM_HOME_DEADLINE {
        std::thread::sleep(STOP_POLL);
        waited += STOP_POLL;
        running = running_from(&home);
    }
    running
        .into_iter()
        .map(|(pid, program)| {
            // A failed kill is that of one that ended meanwhile; the test
            // fails either way.
            let _ = signal_process(pid, "KILL");
            format!("{program} (process {pid}, running from the home)")
        })
        .collect()
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn stop_running_from(_home: &Path) -> Vec<String> {
    Vec::new()
}

/// Each live process whose executable or working folder lies in `home`,
/// with its executable's file name, the test's own process aside.
#[cfg(target_os = "linux")]
#[expect(
    clippy::disallowed_methods,
    reason = "a test helper lists `/proc` outside any async worker (ADR 0062)"
)]
fn running_from(home: &Path) -> Vec<(u32, String)> {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    let own = std::process::id();
    let mut running: Vec<(u32, String)> = entries
        .flatten()
        .filter_map(|entry| {
            let pid: u32 = entry.file_name().to_str()?.parse().ok()?;
            // A process that ended, a zombie included, has no executable
            // to read, nor one of another user.
            let executable = std::fs::read_link(entry.path().join("exe")).ok()?;
            let folder = std::fs::read_link(entry.path().join("cwd")).ok();
            let from_home = executable.starts_with(home)
                || folder.is_some_and(|folder| folder.starts_with(home));
            (pid != own && from_home).then(|| {
                let program = executable
                    .file_name()
                    .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
                (pid, program)
            })
        })
        .collect();
    running.sort_unstable();
    running
}

/// Sends the process `pid` the signal `name` (`TERM`, `KILL`…) through
/// `kill`.
#[cfg(unix)]
#[expect(
    clippy::disallowed_methods,
    reason = "a test helper's blocking `kill`, run outside any async worker (ADR 0062)"
)]
fn signal_process(pid: u32, name: &str) -> io::Result<()> {
    let status = std::process::Command::new("kill")
        .args([format!("-{name}").as_str(), &pid.to_string()])
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "`kill -{name} {pid}` failed: {status}"
        )))
    }
}

/// A process a test started, which its `TestEnv` checks at teardown.
#[derive(Clone)]
pub struct Process {
    program: String,
    child: Rc<RefCell<Child>>,
}

impl Process {
    /// The program it runs, as the command named it.
    pub fn program(&self) -> &str {
        &self.program
    }

    /// The child, for its pipes and its identifier.
    ///
    /// # Panics
    ///
    /// If the child is already borrowed.
    pub fn child(&self) -> RefMut<'_, Child> {
        self.child.borrow_mut()
    }

    /// Closes its standard input and waits for it to exit.
    pub fn close_stdin_and_wait(&self) -> io::Result<ExitStatus> {
        let mut child = self.child();
        drop(child.stdin.take());
        child.wait()
    }

    /// Kills it and waits for it to exit.
    pub fn stop(&self) -> io::Result<ExitStatus> {
        let mut child = self.child();
        child.kill()?;
        child.wait()
    }

    /// Waits up to `deadline` for it to exit, and reaps it if it did:
    /// `None` when it still runs at the deadline.
    ///
    /// # Errors
    ///
    /// When looking at whether it exited fails.
    #[expect(
        clippy::disallowed_methods,
        reason = "a test helper times and polls its child's exit outside any async worker (ADR 0062)"
    )]
    pub fn wait_within(&self, deadline: Duration) -> io::Result<Option<ExitStatus>> {
        let started = std::time::Instant::now();
        loop {
            if let Some(status) = self.child().try_wait()? {
                return Ok(Some(status));
            }
            if started.elapsed() >= deadline {
                return Ok(None);
            }
            std::thread::sleep(STOP_POLL);
        }
    }

    /// Asks it to stop with `SIGINT` and waits up to `deadline` for it to
    /// exit, then kills it if it still runs; either way it is reaped. On
    /// Windows, which has no `SIGINT` to send it, it is killed at once.
    ///
    /// # Errors
    ///
    /// When the signal, the kill or the wait fails, or when it still ran at
    /// the deadline (`TimedOut`, once it is killed and reaped).
    pub fn interrupt_and_wait(&self, deadline: Duration) -> io::Result<ExitStatus> {
        if let Some(status) = self.child().try_wait()? {
            return Ok(status);
        }
        #[cfg(unix)]
        {
            self.interrupt()?;
            if let Some(status) = self.wait_within(deadline)? {
                return Ok(status);
            }
            self.stop()?;
            Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!(
                    "{} still ran {deadline:?} after SIGINT, so it was killed",
                    self.program
                ),
            ))
        }
        #[cfg(not(unix))]
        {
            let _ = deadline;
            self.stop()
        }
    }

    /// Sends it `SIGINT`, as Ctrl-C at a terminal does, through `kill -INT`,
    /// without waiting for it to exit.
    #[cfg(unix)]
    pub fn interrupt(&self) -> io::Result<()> {
        self.signal("INT")
    }

    /// Sends it `CTRL_BREAK_EVENT`, Windows' interrupt of a console
    /// program, without waiting for it to exit. The event goes to its
    /// process group, which [`TestEnv::spawn`] gave it alone; the test
    /// shares its console.
    ///
    /// [`TestEnv::spawn`]: crate::TestEnv::spawn
    #[cfg(windows)]
    #[expect(
        unsafe_code,
        reason = "GenerateConsoleCtrlEvent is a Windows C call, which Rust's standard library does not offer"
    )]
    pub fn interrupt(&self) -> io::Result<()> {
        use windows_sys::Win32::System::Console::{CTRL_BREAK_EVENT, GenerateConsoleCtrlEvent};

        let group = self.child().id();
        // SAFETY: the call takes no pointer; it names the group by the ID
        // of the child that leads it.
        if unsafe { GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, group) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    /// Sends it `SIGTERM`, as a service manager or `kill` does, through
    /// `kill -TERM`, without waiting for it to exit.
    #[cfg(unix)]
    pub fn terminate(&self) -> io::Result<()> {
        self.signal("TERM")
    }

    #[cfg(unix)]
    fn signal(&self, name: &str) -> io::Result<()> {
        let pid = self.child().id();
        signal_process(pid, name)
    }
}

/// The processes a test left alive after its teardown, which fail it: the
/// payload of the teardown's panic.
#[derive(Debug, PartialEq, Eq)]
pub struct ProcessesAlive {
    /// Each such process's program, in the order they started.
    pub programs: Vec<String>,
}

impl fmt::Display for ProcessesAlive {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "processes alive after the test's teardown, now stopped (ADR 0069): {}",
            self.programs.join(", ")
        )
    }
}

/// How often [`Process::wait_within`] looks at whether its process has
/// exited, and the teardown at whether a process still runs from the home.
const STOP_POLL: Duration = Duration::from_millis(10);

/// How long `reported_address` waits for the first line.
pub const REPORT_DEADLINE: Duration = Duration::from_secs(10);

/// The first line a child wrote: a JSON object with an `address` field.
#[derive(Debug, PartialEq)]
pub struct Reported {
    /// The `address` field: a bound address or a socket path.
    pub address: String,
    /// The whole line.
    pub line: serde_json::Value,
}

/// Reads the address a child reports on its first line of standard output,
/// waiting at most [`REPORT_DEADLINE`].
pub fn reported_address(child: &mut Child) -> Result<Reported, ReportedAddressError> {
    reported_address_within(child, REPORT_DEADLINE)
}

/// Reads the address a child reports on its first line of standard output,
/// waiting at most `deadline`. Its standard output must be piped; the bytes
/// after that line stay unread for the test.
pub fn reported_address_within(
    child: &mut Child,
    deadline: Duration,
) -> Result<Reported, ReportedAddressError> {
    let text = first_line_within(child, deadline)?;
    let line: serde_json::Value =
        serde_json::from_str(&text).map_err(|_| ReportedAddressError::NotJson(text.clone()))?;
    let address = line["address"]
        .as_str()
        .ok_or(ReportedAddressError::NoAddress(text))?
        .to_owned();
    Ok(Reported { address, line })
}

/// Reads a child's first line of standard output, waiting at most
/// `deadline`. The bytes after it stay unread, on the child's standard
/// output, which must be piped.
pub(crate) fn first_line_within(
    child: &mut Child,
    deadline: Duration,
) -> Result<String, ReportedAddressError> {
    let stdout = child.stdout.take().ok_or(ReportedAddressError::NoStdout)?;
    let (sender, receiver) = mpsc::sync_channel(1);
    #[expect(
        clippy::disallowed_methods,
        reason = "a test helper's reader thread, which ends when the child's output does (ADR 0062)"
    )]
    std::thread::spawn(move || {
        let mut stdout = stdout;
        let line = first_line(&mut stdout);
        let _ = sender.send((line, stdout));
    });
    let (line, stdout) = receiver
        .recv_timeout(deadline)
        .map_err(|_| ReportedAddressError::Deadline(deadline))?;
    child.stdout = Some(stdout);
    let line = line.map_err(ReportedAddressError::Read)?;
    line.ok_or(ReportedAddressError::Closed)
}

/// One line, read a byte at a time so nothing after it is taken; `None` if
/// the output closed first.
fn first_line(stdout: &mut ChildStdout) -> io::Result<Option<String>> {
    let mut line = Vec::new();
    let mut byte = [0_u8];
    loop {
        if stdout.read(&mut byte)? == 0 {
            return Ok(None);
        }
        if byte[0] == b'\n' {
            break;
        }
        line.push(byte[0]);
    }
    if line.last() == Some(&b'\r') {
        line.pop();
    }
    Ok(Some(String::from_utf8_lossy(&line).into_owned()))
}

/// Why a child reported no address.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ReportedAddressError {
    /// Its standard output is not piped.
    #[error("the child's standard output is not piped")]
    NoStdout,
    /// No line came within the deadline.
    #[error("the child wrote no line within {0:?}")]
    Deadline(Duration),
    /// Its output closed before a line.
    #[error("the child's output closed before its first line")]
    Closed,
    /// Reading its output failed.
    #[error("reading the child's output failed: {0}")]
    Read(io::Error),
    /// The line is not JSON.
    #[error("the child's first line is not JSON: {0}")]
    NotJson(String),
    /// The line has no `address` string.
    #[error("the child's first line has no `address` field: {0}")]
    NoAddress(String),
}
