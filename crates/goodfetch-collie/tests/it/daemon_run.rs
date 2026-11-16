//! `daemon run`, the foreground Daemon, through Collie's executable (spec
//! #213's Seam 1; ADR 0049): it listens on the configured
//! `daemon.address`, writes no runtime file, never stops idle and logs
//! JSON lines to stdout; it refuses an address that is not loopback; and on
//! `SIGTERM` it checkpoints each run as resumable and exits, its runs
//! resuming on its next start. On Linux it runs as PID 1 of a PID
//! namespace, a container's, and reaps every process orphaned there.
#![expect(
    clippy::disallowed_methods,
    reason = "the tests start processes, signal them, and read the Daemon's files and logs themselves; its SQL statements are its own, on no Shared path (ADR 0140)"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::io::{BufRead as _, BufReader};
use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::mpsc;
use std::time::Duration;

use goodfetch_protocol::home::{DaemonFile, RUN_FILE};
use goodfetch_test_client::DaemonClient;
use goodfetch_test_client::HomeLayout;
use goodfetch_test_support::{DaemonEndpoint, Process, RuntimeFile, TestEnv, record_command_in};
use serde_json::{Value, json};

use crate::daemon_hello;

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// This commit's Product version, whose major folder is `0.0`.
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// How long a test waits for a line the Daemon must log; a failure, never
/// a measure.
const LOG_DEADLINE: Duration = Duration::from_secs(60);

/// The `daemon.address` a test configures: loopback on port 0, so the
/// Daemon takes a free port itself and names it in its log (TESTING.md: no
/// fixed ports). A port the test found free and released could be taken by
/// another process before the Daemon binds it (FQ49).
pub(crate) const ANY_PORT: &str = "127.0.0.1:0";

/// Whether `endpoint` is where a Daemon configured with [`ANY_PORT`] serves:
/// loopback, on a port it took, which is not its default one.
pub(crate) fn took_a_free_port(endpoint: &DaemonEndpoint) -> bool {
    let default = (goodfetch_protocol::daemon::ADDRESS.default)();
    matches!(endpoint, DaemonEndpoint::Tcp(bound)
        if bound.ip() == Ipv4Addr::LOCALHOST
            && bound.port() != 0
            && default.as_str() != Some(bound.to_string().as_str()))
}

/// A foreground Daemon the test started, with its stdout's lines read on a
/// thread of their own. Once it exits, `daemon run` goes into the test's
/// command record with its exit code (ADR 0072).
pub(crate) struct Foreground {
    /// The test's folder, which holds its command record.
    folder: PathBuf,
    pub(crate) process: Process,
    lines: mpsc::Receiver<String>,
    /// Every line it wrote to stdout so far.
    pub(crate) logged: Vec<String>,
}

impl Foreground {
    /// Starts `executable daemon run` in `env`, with `variables` added to
    /// its environment.
    pub(crate) fn start(env: &TestEnv, executable: &str, variables: &[(&str, &str)]) -> Self {
        let mut command = env.process_command(executable);
        command
            .args(["daemon", "run"])
            .envs(variables.iter().copied());
        Self::spawned(env, &mut command)
    }

    /// Starts `command`, which runs `daemon run`, in `env`.
    pub(crate) fn spawned(env: &TestEnv, command: &mut std::process::Command) -> Self {
        let process = env.spawn(command.stderr(Stdio::null())).unwrap();
        let stdout = process.child().stdout.take().unwrap();
        let (sent, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { return };
                if sent.send(line).is_err() {
                    return;
                }
            }
        });
        Self {
            folder: env.folder().to_owned(),
            process,
            lines,
            logged: Vec::new(),
        }
    }

    /// Its next line on stdout, which must be one JSON document.
    pub(crate) fn first_line(&mut self) -> Value {
        let line = self
            .lines
            .recv_timeout(LOG_DEADLINE)
            .unwrap_or_else(|_| panic!("no line on its stdout: {:#?}", self.logged));
        self.logged.push(line.clone());
        serde_json::from_str(&line).unwrap()
    }

    /// The fields of the first log record from here on whose message is
    /// `message`.
    pub(crate) fn record(&mut self, message: &str) -> Value {
        loop {
            let line = self
                .lines
                .recv_timeout(LOG_DEADLINE)
                .unwrap_or_else(|_| panic!("no `{message}` in its log: {:#?}", self.logged));
            self.logged.push(line.clone());
            if let Ok(record) = serde_json::from_str::<Value>(&line)
                && record["fields"]["message"] == json!(message)
            {
                return record["fields"].clone();
            }
        }
    }

    /// The fields of the next refusal of a Connection it logs from here on,
    /// or `None` when it logs none within the deadline.
    pub(crate) fn refusal(&mut self) -> Option<Value> {
        while let Ok(line) = self.lines.recv_timeout(LOG_DEADLINE) {
            self.logged.push(line.clone());
            if let Ok(record) = serde_json::from_str::<Value>(&line)
                && record["target"] == json!("goodfetch::rpc::refusal")
            {
                return Some(record["fields"].clone());
            }
        }
        None
    }

    /// Where it serves, as it logged once it did.
    pub(crate) fn serving(&mut self) -> DaemonEndpoint {
        let serves = self.record("the foreground Daemon serves");
        DaemonEndpoint::Tcp(serves["endpoint"].as_str().unwrap().parse().unwrap())
    }

    /// Sends it `SIGTERM`, and waits for it to exit, reading the rest of its
    /// log; whether it exited with 0.
    pub(crate) fn terminate(&mut self) -> bool {
        self.signal();
        self.exited()
    }

    /// Sends it `SIGTERM`, through `kill -TERM`.
    pub(crate) fn signal(&self) {
        let pid = self.process.child().id().to_string();
        let sent = std::process::Command::new("kill")
            .args(["-TERM", &pid])
            .status()
            .unwrap();
        assert!(sent.success(), "`kill -TERM {pid}` failed");
    }

    /// Waits for it to exit, reading the rest of its log, and records
    /// `daemon run` with its exit code; whether it exited with 0.
    pub(crate) fn exited(&mut self) -> bool {
        let exited = self.process.child().wait().unwrap();
        while let Ok(line) = self.lines.recv_timeout(LOG_DEADLINE) {
            self.logged.push(line);
        }
        record_command_in(&self.folder, &["daemon", "run"], exited.code()).unwrap();
        exited.success()
    }
}

/// The persistent token Clients of the foreground Daemon open with.
pub(crate) fn token(env: &TestEnv) -> String {
    std::fs::read_to_string(HomeLayout::of(env.home()).daemon_file(DaemonFile::Token)).unwrap()
}

/// A Client of the foreground Daemon at `endpoint`.
pub(crate) fn opened(env: &TestEnv, endpoint: &DaemonEndpoint) -> DaemonClient {
    DaemonClient::opened(endpoint, &daemon_hello(&token(env)))
}

fn run_folder(env: &TestEnv, run_id: &str) -> PathBuf {
    HomeLayout::of(env.home()).run_folder(run_id)
}

/// The run row of `run_id`'s run file: its lifecycle, stop reason and run
/// epoch.
fn run_row(env: &TestEnv, run_id: &str) -> Value {
    let file = rusqlite::Connection::open(run_folder(env, run_id).join(RUN_FILE)).unwrap();
    let row: String = file
        .query_row(
            "SELECT json_object('lifecycle', lifecycle, 'stop_reason', json(stop_reason), 'epoch', epoch) FROM run",
            [],
            |row| row.get(0),
        )
        .unwrap();
    serde_json::from_str(&row).unwrap()
}

/// The foreground Daemon listens on the address user configuration gives,
/// logs JSON lines to stdout, writes no runtime file and is still serving
/// long after its idle timeout with no Client connected.
///
/// Covers: story:3-98, command:daemon.run, setting:daemon.address
#[test]
fn daemon_run_listens_on_the_configured_address_logs_json_to_stdout_and_never_stops_idle() {
    let env = TestEnv::new();
    env.user_configuration(&format!(
        "daemon.address = \"{ANY_PORT}\"\ndaemon.idle_timeout = 100\n"
    ));

    let mut daemon = Foreground::start(&env, COLLIE, &[]);
    let endpoint = daemon.serving();
    std::thread::sleep(Duration::from_secs(1));
    let mut client = opened(&env, &endpoint);
    let status = client.call(1, "daemon.status", &json!({}));
    drop(client);
    let exited = daemon.terminate();

    let not_json: Vec<&String> = daemon
        .logged
        .iter()
        .filter(|line| serde_json::from_str::<Value>(line).is_err())
        .collect();
    assert_eq!(
        (
            took_a_free_port(&endpoint),
            status["result"]["kept_alive"].clone(),
            RuntimeFile::path(env.home(), VERSION).exists(),
            not_json,
            exited,
        ),
        (true, json!(true), false, Vec::<&String>::new(), true,)
    );
}

/// Until the Daemon terminates TLS, `daemon run` refuses an address that is
/// not loopback, and starts nothing.
///
/// Covers: story:3-98, error:setting_invalid
#[test]
fn daemon_run_refuses_an_address_that_is_not_loopback() {
    let env = TestEnv::new();
    env.user_configuration("daemon.address = \"0.0.0.0:7900\"\n");

    let mut daemon = Foreground::start(&env, COLLIE, &[]);
    let refusal = daemon.record("`daemon.address` is not valid in user_configuration: `0.0.0.0:7900` is not a loopback address: `daemon run` listens only on loopback until the Daemon terminates TLS; put a TLS-terminating proxy in front of a loopback address");

    let exited = daemon.process.child().wait().unwrap();
    assert_eq!(
        (exited.code(), refusal["code"].clone()),
        (Some(1), json!("setting_invalid"))
    );
}

/// On `SIGTERM`, under a short drain grace from the environment, the
/// foreground Daemon stops each live run with a Checkpoint, resumable, and
/// exits; its next start resumes both runs by their Run IDs, at the
/// next run epoch, and each finishes.
///
/// Covers: story:3-99, setting:daemon.drain_grace, operation:run.resume
#[test]
fn on_sigterm_daemon_run_checkpoints_each_run_and_its_next_start_resumes_them() {
    let env = TestEnv::new();
    env.user_configuration(&format!("daemon.address = \"{ANY_PORT}\"\n"));
    let grace = [("GOODFETCH_DAEMON_DRAIN_GRACE", "5000")];

    let mut first = Foreground::start(&env, COLLIE, &grace);
    let endpoint = first.serving();
    let mut client = opened(&env, &endpoint);
    for (id, run_id) in [(1, "first"), (2, "second")] {
        let started = client.call(id, "run.start", &json!({ "run_id": run_id }));
        assert!(started.get("result").is_some(), "{started}");
    }
    let drained = first.terminate();
    drop(client);
    let stopped = ["first", "second"].map(|run_id| {
        (
            run_row(&env, run_id),
            run_folder(&env, run_id).join("checkpoint.sqlite").is_file(),
        )
    });

    let mut second = Foreground::start(&env, COLLIE, &grace);
    let endpoint = second.serving();
    let mut client = opened(&env, &endpoint);
    let resumed = [(3, "first"), (4, "second")].map(|(id, run_id)| {
        let resumed = client.call(id, "run.resume", &json!({ "run_id": run_id }));
        let finished = client.call(id + 10, "run.finish", &json!({ "run_id": run_id }));
        (
            resumed["result"]["started_as"].clone(),
            finished["result"]["lifecycle"].clone(),
            run_row(&env, run_id)["epoch"].clone(),
        )
    });
    drop(client);
    let exited = second.terminate();

    let drained_run = (
        json!({ "lifecycle": "stopped", "stop_reason": { "kind": "drain" }, "epoch": 1 }),
        true,
    );
    let resumed_run = (json!("resumed"), json!("finished"), json!(2));
    assert_eq!(
        (drained, stopped, resumed, exited),
        (
            true,
            [drained_run.clone(), drained_run],
            [resumed_run.clone(), resumed_run],
            true
        )
    );
}

/// The foreground Daemon as PID 1 of a container (ADR 0049).
#[cfg(target_os = "linux")]
mod pid_one {
    use std::path::Path;
    use std::process::Stdio;
    use std::time::{Duration, Instant};

    use goodfetch_test_support::TestEnv;

    use super::{ANY_PORT, COLLIE, Foreground};

    /// The test-only copy of util-linux's `unshare` that `ci/os-services.sh
    /// setup` installs, root-owned, with an AppArmor profile that gives it
    /// user namespaces under Ubuntu's restriction on unprivileged ones, which
    /// stays on for every other program, `/usr/bin/unshare` included (ADR
    /// 0049; TESTING.md, Storage, network and OS services).
    const TEST_UNSHARE: &str = "/usr/local/libexec/goodfetch-test-tools/unshare";

    /// A command running the setup's `unshare`, or the one on `PATH` on a
    /// host the setup never ran on. It fails the test at once, naming the
    /// restriction, when that `unshare` cannot map the test's user to root
    /// in a user namespace of its own, rather than leave a Daemon that never
    /// started.
    fn unshare(env: &TestEnv) -> std::process::Command {
        let program = if Path::new(TEST_UNSHARE).exists() {
            TEST_UNSHARE
        } else {
            "unshare"
        };
        let probe = env
            .process_command(program)
            .args(["--user", "--map-root-user", "true"])
            .stdin(Stdio::null())
            .output()
            .unwrap_or_else(|error| panic!("`{program}` did not run: {error}"));
        assert!(
            probe.status.success(),
            "`{program}` cannot map the test's user to root in a user namespace of its own \
             ({}). On Ubuntu, `kernel.apparmor_restrict_unprivileged_userns=1` refuses it to \
             every program that no AppArmor profile gives `userns`: run `ci/os-services.sh \
             setup`, which installs `{TEST_UNSHARE}` with such a profile",
            String::from_utf8_lossy(&probe.stderr).trim()
        );
        env.process_command(program)
    }

    /// How long the test waits for every orphan to be reaped; a failure,
    /// never a measure.
    const DEADLINE: Duration = Duration::from_secs(60);

    /// A process as `/proc/<pid>/stat` gives it: its name, state and parent.
    struct Stat {
        pid: u32,
        name: String,
        state: char,
        parent: u32,
    }

    fn stat(pid: u32) -> Option<Stat> {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        let (name, rest) = stat.split_once(" (")?.1.rsplit_once(") ")?;
        let mut fields = rest.split(' ');
        let state = fields.next()?.chars().next()?;
        let parent = fields.next()?.parse().ok()?;
        Some(Stat {
            pid,
            name: name.to_owned(),
            state,
            parent,
        })
    }

    /// Every process under `root`, as its name and whether it is a zombie.
    fn under(root: u32) -> Vec<(String, bool)> {
        let all: Vec<Stat> = std::fs::read_dir("/proc")
            .unwrap()
            .filter_map(|entry| entry.ok()?.file_name().to_str()?.parse().ok())
            .filter_map(stat)
            .collect();
        let mut found = vec![root];
        let mut under = Vec::new();
        while let Some(parent) = found.pop() {
            for child in all.iter().filter(|stat| stat.parent == parent) {
                found.push(child.pid);
                under.push((child.name.clone(), child.state == 'Z'));
            }
        }
        under.sort();
        under
    }

    /// Run as PID 1 of a PID namespace of its own, as a container runs it,
    /// with no `tini`, `daemon run` reaps every process orphaned in it: once
    /// they exited, only its own two processes are left, the reaper and the
    /// Daemon, none of them a zombie, and it drains on `SIGTERM` there too.
    /// The orphans stand in for a browser's descendants, which the kernel
    /// gives to PID 1 the same way when the browser exits first: this
    /// version pins no browser for Linux yet.
    ///
    /// Covers: story:3-100
    #[test]
    fn as_pid_one_of_a_container_daemon_run_reaps_every_orphan() {
        let env = TestEnv::new();
        env.user_configuration(&format!("daemon.address = \"{ANY_PORT}\"\n"));
        let mut command = unshare(&env);
        command
            .args([
                "--user",
                "--map-root-user",
                "--pid",
                "--fork",
                "--kill-child",
                "--mount-proc",
            ])
            .arg(COLLIE)
            .args(["daemon", "run"]);
        let mut daemon = Foreground::spawned(&env, &mut command);
        daemon.serving();
        let unshare = daemon.process.child().id();
        let pid_one = std::fs::read_to_string(format!("/proc/{unshare}/task/{unshare}/children"))
            .unwrap()
            .trim()
            .to_owned();
        // A shell in the namespace starts two `sleep`s in the background and
        // exits at once, leaving them to PID 1.
        let orphaned = std::process::Command::new("nsenter")
            .args([
                "--target",
                &pid_one,
                "--user",
                "--pid",
                "--preserve-credentials",
            ])
            .args(["sh", "-c", "sleep 1 & sleep 1 &"])
            .status()
            .unwrap();

        let began = Instant::now();
        let left = loop {
            let left = under(unshare);
            let only_its_own = left
                .iter()
                .all(|(name, zombie)| name.starts_with("goodfetch-colli") && !zombie);
            if only_its_own || began.elapsed() > DEADLINE {
                break left;
            }
            std::thread::sleep(Duration::from_millis(100));
        };
        let sent = std::process::Command::new("kill")
            .args(["-TERM", &pid_one])
            .status()
            .unwrap();
        let exited = daemon.process.child().wait().unwrap();

        assert_eq!(
            (orphaned.success(), left, sent.success(), exited.success()),
            (
                true,
                vec![
                    ("goodfetch-colli".to_owned(), false),
                    ("goodfetch-colli".to_owned(), false),
                ],
                true,
                true,
            )
        );
    }
}
