//! `goodfetch-scripted-engine` (ADR 0069): the Scripted engine as a
//! process, for the faults neither Product's executable can produce on
//! demand (a refused `hello`, a crash mid-call, garbage on stdout, a dead
//! PID in the runtime file, a dropped connection). It plays one script of
//! `test-support/fixtures/engine-scripts/` to one Client, records every
//! message it saw, and exits non-zero, with the failure's code and reason
//! on stderr, on a message no step expects or a step left unused.
//!
//! `--script <file> --mode <mode>` names the script, and the mode how the
//! Client reaches it:
//! - `stdio`: over its own stdin and stdout, as an Attached engine;
//! - `socket`: as a Daemon, on a Unix socket on macOS and Linux and on
//!   loopback TCP on Windows (ADR 0024), found through the Daemon's runtime
//!   file it writes into the per-major folder of `GOODFETCH_HOME`, holding
//!   the endpoint, `--token`, `--product-version` and its PID, and holding
//!   the Daemon's serving lock beside it while it plays, as a live Daemon
//!   does, so a Client takes it for running until it exits. It prints
//!   `{"address": …, "runtime_file": …}` once a Client can find it, and
//!   removes the file and the socket when the play ends, unless the script
//!   ends with `exit`, which leaves them as a crashed Daemon does;
//! - `tcp`: on loopback TCP on port 0, printing `{"address": …}`.
//!
//! The token check is the script's own `expect` of a `hello` carrying the
//! token, never a built-in.
//!
//! An SDK reaches it as an Attached engine instead: its Product's
//! engine-path variable (`GOODFETCH_COLLIE_ENGINE_PATH` or
//! `GOODFETCH_FOXHOUND_ENGINE_PATH`, ADR 0090) names this executable, which
//! the SDK starts as `attached-engine` with its environment map; the script
//! is then `scripted-engine/script.json` under the map's `GOODFETCH_HOME`,
//! and the record `scripted-engine/record.jsonl` beside it. A Client that
//! starts a Daemon's process as a Product's `daemon start` does, such as the
//! conformance harness, starts `daemon launched` instead: it plays the same
//! script in `socket` mode and, as a launched Daemon does, prints the
//! runtime file it wrote once a Client can find it.
//!
//! A test-only executable the Simulation never runs, it opens its own
//! stdio and sockets and writes its own stdout, the one writer to stdout
//! beyond `STYLE.md`'s three (ADRs 0060 and 0069).
#![expect(
    clippy::disallowed_types,
    reason = "the test-only Scripted engine opens its own sockets, outside `transport` (ADRs 0060 and 0069)"
)]

use std::cell::OnceCell;
use std::fs::File;
use std::io::{self, BufReader, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use goodfetch_test_support::{
    ATTACHED_RECORD, ATTACHED_SCRIPT, ConsoleState, DaemonEndpoint, Ending, RuntimeFile,
    SCRIPTED_DAEMON_TOKEN, Script, play,
};
use serde_json::{Value, json};

#[derive(Parser)]
#[command(
    name = "goodfetch-scripted-engine",
    about = "Plays one script of protocol messages to a Client, as a stand-in engine in tests",
    args_conflicts_with_subcommands = true,
    subcommand_negates_reqs = true
)]
struct Arguments {
    /// The script to play.
    #[arg(long, required = true)]
    script: Option<PathBuf>,
    /// How the Client reaches the engine.
    #[arg(long, value_enum, required = true)]
    mode: Option<Mode>,
    /// Where to write the record of every message, one JSON line each.
    #[arg(long)]
    record: Option<PathBuf>,
    /// The per-launch token the runtime file holds, in `--mode socket`.
    #[arg(long, default_value = SCRIPTED_DAEMON_TOKEN)]
    token: String,
    /// The Product version the runtime file holds, whose major names its
    /// per-major folder, in `--mode socket`.
    #[arg(long, default_value = env!("CARGO_PKG_VERSION"))]
    product_version: String,
    #[command(subcommand)]
    form: Option<Form>,
}

/// How the Client reaches the engine.
#[derive(Clone, Copy, ValueEnum)]
enum Mode {
    /// Over the engine's own stdin and stdout, as an Attached engine.
    Stdio,
    /// As a Daemon, found through the runtime file in `GOODFETCH_HOME`.
    Socket,
    /// On loopback TCP, at the address it prints.
    Tcp,
}

#[derive(Subcommand)]
enum Form {
    /// Runs as an SDK starts its Attached engine: plays
    /// `scripted-engine/script.json` under `GOODFETCH_HOME` over stdio.
    AttachedEngine,
    /// Runs as a Product's Daemon is started.
    #[command(subcommand)]
    Daemon(DaemonForm),
}

#[derive(Subcommand)]
enum DaemonForm {
    /// Runs as a Daemon's launched process: plays
    /// `scripted-engine/script.json` under `GOODFETCH_HOME` in `socket` mode
    /// and prints the runtime file once ready.
    Launched,
}

/// The line printed once a Client can find the Daemon.
#[derive(Clone, Copy)]
enum Ready {
    /// `{"address": …, "runtime_file": …}`, for a test that started it.
    Address,
    /// The runtime file's contents, as a launched Daemon prints them.
    RuntimeFile,
}

fn main() -> ExitCode {
    let arguments = Arguments::parse();
    match run(&arguments) {
        Ok(Ending::Finished | Ending::Closed) => ExitCode::SUCCESS,
        Ok(Ending::Exit(code)) => ExitCode::from(code),
        Err(failure) => {
            #[expect(
                clippy::print_stderr,
                reason = "the Scripted engine reports a failed play on its own stderr (ADRs 0060 and 0069)"
            )]
            {
                eprintln!("goodfetch-scripted-engine: {failure}");
            }
            ExitCode::FAILURE
        }
    }
}

type Failure = Box<dyn std::error::Error>;

fn run(arguments: &Arguments) -> Result<Ending, Failure> {
    let mut ready = Ready::Address;
    let (script, record, mode) = match (&arguments.form, &arguments.script, arguments.mode) {
        (Some(Form::AttachedEngine), _, _) => {
            let home = home()?;
            (
                home.join(ATTACHED_SCRIPT),
                Some(home.join(ATTACHED_RECORD)),
                Mode::Stdio,
            )
        }
        (Some(Form::Daemon(DaemonForm::Launched)), _, _) => {
            let home = home()?;
            ready = Ready::RuntimeFile;
            (
                home.join(ATTACHED_SCRIPT),
                Some(home.join(ATTACHED_RECORD)),
                Mode::Socket,
            )
        }
        (None, Some(script), Some(mode)) => (script.clone(), arguments.record.clone(), mode),
        (None, _, _) => return Err("invalid_arguments: --script and --mode name no play".into()),
    };
    let script = Script::read(&script).map_err(|error| format!("invalid_script: {error}"))?;
    let record: Box<dyn Write> = match record {
        Some(path) => Box::new(create(&path)?),
        None => Box::new(io::sink()),
    };
    match mode {
        Mode::Stdio => {
            #[expect(
                clippy::disallowed_methods,
                reason = "the test-only Scripted engine plays its script over its own stdio, outside `transport` (ADRs 0060 and 0069)"
            )]
            let (stdin, stdout) = (io::stdin().lock(), io::stdout().lock());
            Ok(play(&script, stdin, stdout, record)?)
        }
        Mode::Tcp => {
            let listener = Listener::Tcp(
                TcpListener::bind("127.0.0.1:0")
                    .map_err(|error| format!("listen_failed: {error}"))?,
            );
            announce(&json!({ "address": listener.endpoint()?.to_string() }))?;
            listener.play(&script, record)
        }
        Mode::Socket => {
            let daemon = Daemon::publish(&home()?, arguments, ready)
                .map_err(|error| format!("publish_failed: {error}"))?;
            let ending = daemon.listener.play(&script, record);
            if let Ok(Ending::Exit(_)) = ending {
                daemon.abandon();
                return ending;
            }
            // The play's own failure comes first: a test matches on its code.
            let withdrawn = daemon.withdraw();
            ending.and_then(|ending| withdrawn.map(|()| ending))
        }
    }
}

/// The Scripted engine published as a Daemon: listening, and named in its
/// runtime file.
struct Daemon {
    listener: Listener,
    endpoint: DaemonEndpoint,
    runtime_file: PathBuf,
    /// The socket's own folder, made outside the home when the socket's
    /// path in the home is too long for the OS.
    socket_folder: Option<tempfile::TempDir>,
    /// The serving lock, held until the process exits.
    _serving: std::fs::File,
}

impl Daemon {
    /// Listens, writes the runtime file and prints the line saying a Client
    /// can find it.
    #[expect(
        clippy::disallowed_methods,
        reason = "ADR 0140: a file opened on no counted row yet; the Shared paths audit moves it onto its counted function"
    )]
    fn publish(home: &Path, arguments: &Arguments, ready: Ready) -> Result<Self, Failure> {
        let runtime_file = RuntimeFile::path(home, &arguments.product_version);
        let folder = runtime_file
            .parent()
            .expect("invariant: a runtime file sits in a per-major folder");
        create_folder(folder)?;
        let serving = std::fs::File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(folder.join(goodfetch_test_support::DAEMON_SERVING_LOCK))?;
        serving.lock()?;
        let (listener, socket_folder) = Listener::daemon(folder)?;
        let endpoint = listener.endpoint()?;
        let contents = RuntimeFile {
            endpoint: endpoint.clone(),
            token: arguments.token.clone(),
            version: arguments.product_version.clone(),
            pid: std::process::id(),
            console: ConsoleState::Absent,
        };
        let daemon = Self {
            listener,
            endpoint: endpoint.clone(),
            runtime_file,
            socket_folder,
            _serving: serving,
        };
        contents.write(&daemon.runtime_file)?;
        match ready {
            Ready::Address => announce(&json!({
                "address": endpoint.to_string(),
                "runtime_file": daemon.runtime_file,
            }))?,
            Ready::RuntimeFile => announce(&serde_json::to_value(&contents)?)?,
        }
        Ok(daemon)
    }

    /// Removes the runtime file and the socket, as a Daemon that stops.
    fn withdraw(self) -> Result<(), Failure> {
        remove(&self.runtime_file)?;
        if let DaemonEndpoint::Unix(path) = &self.endpoint {
            remove(path)?;
        }
        Ok(())
    }

    /// Leaves the runtime file and a socket in the home, as a crashed
    /// Daemon does; only a socket folder outside the home goes. A killed
    /// engine runs no destructor, so its `TestEnv` removes that folder at
    /// teardown.
    fn abandon(self) {
        drop(self.socket_folder);
    }
}

/// Where the Scripted engine waits for its one Client.
enum Listener {
    Tcp(TcpListener),
    #[cfg(unix)]
    Unix(UnixListener),
}

impl Listener {
    /// A Daemon's listener: a Unix socket in `folder`, readable by its
    /// owner only, or in a folder of its own under `/tmp` when that path is
    /// longer than the OS takes (104 bytes on macOS), named with a space and
    /// a non-ASCII letter like a test's home (ADR 0073).
    #[cfg(unix)]
    fn daemon(folder: &Path) -> Result<(Self, Option<tempfile::TempDir>), Failure> {
        use std::os::unix::fs::PermissionsExt as _;
        let socket = folder.join(goodfetch_test_support::DAEMON_SOCKET);
        // The caller holds the serving lock, so a socket left here is a dead
        // Daemon's: a starting Daemon replaces its files (ADR 0024).
        remove(&socket)?;
        let (listener, path, own_folder) = match UnixListener::bind(&socket) {
            Ok(listener) => (listener, socket, None),
            Err(error) if error.kind() == io::ErrorKind::InvalidInput => {
                let own = tempfile::Builder::new()
                    .prefix(goodfetch_test_support::SOCKET_FOLDER_PREFIX)
                    .permissions(std::fs::Permissions::from_mode(0o700))
                    .tempdir_in("/tmp")?;
                let path = own.path().join(goodfetch_test_support::DAEMON_SOCKET);
                (UnixListener::bind(&path)?, path, Some(own))
            }
            Err(error) => return Err(error.into()),
        };
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        Ok((Self::Unix(listener), own_folder))
    }

    /// A Daemon's listener: loopback TCP, as on Windows (ADR 0024).
    #[cfg(not(unix))]
    fn daemon(_folder: &Path) -> Result<(Self, Option<tempfile::TempDir>), Failure> {
        Ok((Self::Tcp(TcpListener::bind("127.0.0.1:0")?), None))
    }

    fn endpoint(&self) -> io::Result<DaemonEndpoint> {
        match self {
            Self::Tcp(listener) => listener.local_addr().map(DaemonEndpoint::Tcp),
            #[cfg(unix)]
            Self::Unix(listener) => {
                let address = listener.local_addr()?;
                let path = address.as_pathname().ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidData, "the socket has no path")
                })?;
                Ok(DaemonEndpoint::Unix(path.to_owned()))
            }
        }
    }

    /// Plays `script` to the first Client to connect, accepting it only
    /// when a step first reads or writes, so a script that exits at once
    /// never waits for one.
    fn play(&self, script: &Script, record: impl Write) -> Result<Ending, Failure> {
        let connection = Connection {
            listener: self,
            stream: OnceCell::new(),
        };
        let ending = play(script, BufReader::new(&connection), &connection, record);
        connection.close();
        Ok(ending?)
    }
}

/// The one Client's connection, accepted on first use.
struct Connection<'a> {
    listener: &'a Listener,
    stream: OnceCell<Stream>,
}

impl Connection<'_> {
    fn stream(&self) -> io::Result<&Stream> {
        if let Some(stream) = self.stream.get() {
            return Ok(stream);
        }
        let stream = match &self.listener {
            Listener::Tcp(listener) => Stream::Tcp(listener.accept()?.0),
            #[cfg(unix)]
            Listener::Unix(listener) => Stream::Unix(listener.accept()?.0),
        };
        Ok(self.stream.get_or_init(|| stream))
    }

    /// Closes the connection both ways, so the Client reads its end at
    /// once.
    fn close(&self) {
        // A Client that already went leaves nothing to shut down.
        let _ = match self.stream.get() {
            Some(Stream::Tcp(stream)) => stream.shutdown(Shutdown::Both),
            #[cfg(unix)]
            Some(Stream::Unix(stream)) => stream.shutdown(Shutdown::Both),
            None => Ok(()),
        };
    }
}

enum Stream {
    Tcp(TcpStream),
    #[cfg(unix)]
    Unix(UnixStream),
}

impl Read for &Connection<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        match self.stream()? {
            Stream::Tcp(stream) => (&*stream).read(buffer),
            #[cfg(unix)]
            Stream::Unix(stream) => (&*stream).read(buffer),
        }
    }
}

impl Write for &Connection<'_> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        match self.stream()? {
            Stream::Tcp(stream) => (&*stream).write(buffer),
            #[cfg(unix)]
            Stream::Unix(stream) => (&*stream).write(buffer),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self.stream()? {
            Stream::Tcp(stream) => (&*stream).flush(),
            #[cfg(unix)]
            Stream::Unix(stream) => (&*stream).flush(),
        }
    }
}

/// Prints the one line saying where a Client reaches the engine.
fn announce(line: &Value) -> Result<(), Failure> {
    #[expect(
        clippy::disallowed_methods,
        reason = "the Scripted engine prints the address a test reads, its own stdout (ADRs 0060 and 0069)"
    )]
    let mut stdout = io::stdout().lock();
    writeln!(stdout, "{line}")?;
    stdout.flush()?;
    Ok(())
}

/// The Goodfetch home the Client's environment map names.
fn home() -> Result<PathBuf, Failure> {
    #[expect(
        clippy::disallowed_methods,
        reason = "the Scripted engine reads the Goodfetch home its Client's environment map gives it, as an engine's `main` does (ADR 0069)"
    )]
    let home = std::env::var_os("GOODFETCH_HOME");
    home.map(PathBuf::from)
        .ok_or_else(|| "no_home: GOODFETCH_HOME names no Goodfetch home".into())
}

fn create(path: &Path) -> Result<File, Failure> {
    #[expect(
        clippy::disallowed_methods,
        reason = "the Scripted engine is synchronous and starts no async worker to block (ADR 0069)"
    )]
    let file = File::create(path);
    file.map_err(|error| {
        format!(
            "record_failed: {} cannot be written: {error}",
            path.display()
        )
        .into()
    })
}

fn create_folder(folder: &Path) -> Result<(), Failure> {
    #[expect(
        clippy::disallowed_methods,
        reason = "the Scripted engine is synchronous and starts no async worker to block (ADR 0069)"
    )]
    let made = std::fs::create_dir_all(folder);
    Ok(made?)
}

fn remove(path: &Path) -> Result<(), Failure> {
    #[expect(
        clippy::disallowed_methods,
        reason = "the Scripted engine is synchronous and starts no async worker to block (ADR 0069)"
    )]
    let removed = std::fs::remove_file(path);
    match removed {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error.into()),
        _ => Ok(()),
    }
}
