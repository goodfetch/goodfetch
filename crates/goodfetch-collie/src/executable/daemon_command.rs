//! The `daemon` group's commands, from both Products' executables (ADRs
//! 0024, 0049 and 0098): their command line and where each one runs.

use std::process::ExitCode;

use clap::Subcommand;
use goodfetch_cli::output::Human;
use goodfetch_engine::{Dependencies, Engine};

use super::{daemon, service};

#[derive(Subcommand)]
pub(super) enum DaemonCommand {
    /// Starts the Daemon, the major's pick, unless one runs.
    Start {
        /// Never stops it after `daemon.idle_timeout` with no Client.
        #[arg(long)]
        keep_alive: bool,
        /// Prints the result as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Stops the Daemon; its live Runs stop with it.
    Stop {
        /// Prints the result as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Stops the Daemon, then starts the major's pick.
    Restart {
        /// Never stops it after `daemon.idle_timeout` with no Client.
        #[arg(long)]
        keep_alive: bool,
        /// Prints the result as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Shows whether the Daemon runs, as which Product, and its Clients.
    Status {
        /// Prints the result as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Says whether the Daemon of the major, or the foreground Daemon at
    /// `daemon.address`, is ready to take a run, as its ops listener's
    /// `/readyz` does, for `exec` probes and systemd: exits 0 when it is, 1
    /// when it is not or none answers.
    Health {
        /// Says whether it is live instead, as `/livez` does.
        #[arg(long)]
        live: bool,
        /// Prints the result as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Registers the major's Daemon as a per-user service, which starts
    /// the major's pick: a launch agent on macOS and a systemd user unit on
    /// Linux, each started by the first connection to its socket, and a
    /// logon task on Windows.
    Install {
        /// Prints the result as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Removes the major's per-user service, stopping its Daemon.
    Uninstall {
        /// Prints the result as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Replaces the persistent token of the major's Daemon and the HTTP
    /// listener's token: an installed service's Daemon stops with every
    /// Connection that opened with the old one, and a Daemon started after
    /// it refuses the old one.
    RotateToken {
        /// Prints the result as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Prints the token the Daemon's HTTP listener takes, for a workflow
    /// tool's configuration: on stdout, and nowhere else.
    HttpListenerToken {
        /// Prints the result as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Runs the Daemon in the foreground, for Docker, Kubernetes and
    /// systemd: as this executable's Product, listening on
    /// `daemon.address`, with no runtime file and no idle stop, logging
    /// JSON lines to stdout; on `SIGTERM` it takes no run, checkpoints each
    /// run as resumable and exits within `daemon.drain_grace`.
    Run {
        /// Prints the Daemon's status as JSON once it listens, before its
        /// logs.
        #[arg(long)]
        json: bool,
        /// Runs under the reaper of a `daemon run` on Linux, which started it.
        #[arg(long, hide = true)]
        reaped: bool,
    },
    /// Runs as the Daemon a start or the installed service launched, from a
    /// versioned copy.
    #[command(hide = true)]
    Launched {
        /// The build of the versioned copy it runs from.
        #[arg(long)]
        copy: Option<String>,
        /// Never stops it after `daemon.idle_timeout` with no Client.
        #[arg(long)]
        keep_alive: bool,
        /// Started by the major's service: starts the pick in its place
        /// when it is not the pick, and takes the socket its service
        /// manager hands over.
        #[arg(long)]
        service: bool,
        /// A deployment variable the service passes, as `NAME=VALUE`;
        /// repeatable.
        #[arg(long = "environment", value_name = "NAME=VALUE")]
        environment: Vec<String>,
    },
}

/// Runs the `daemon` group's `command`.
pub(super) fn daemon_command(
    command: DaemonCommand,
    engine: &Engine,
    dependencies: Dependencies,
    machine: &daemon::Machine<'_>,
    human: &Human,
) -> ExitCode {
    match command {
        DaemonCommand::Start { keep_alive, json } => {
            daemon::start(machine, keep_alive, json, human)
        }
        DaemonCommand::Stop { json } => daemon::stop(machine, json, human),
        DaemonCommand::Restart { keep_alive, json } => {
            daemon::restart(machine, keep_alive, json, human)
        }
        DaemonCommand::Status { json } => daemon::status(machine, json, human),
        DaemonCommand::Health { live, json } => daemon::health(machine, live, json, human),
        DaemonCommand::Install { json } => service::install(machine, json, human),
        DaemonCommand::Uninstall { json } => service::uninstall(machine, json, human),
        DaemonCommand::RotateToken { json } => service::rotate_token(machine, json, human),
        DaemonCommand::HttpListenerToken { json } => {
            service::http_listener_token(machine, json, human)
        }
        DaemonCommand::Run { json, reaped } => {
            daemon::run(engine, dependencies, machine, reaped, json)
        }
        DaemonCommand::Launched {
            copy,
            keep_alive,
            service,
            environment: _,
        } => daemon::launched(
            engine,
            dependencies,
            machine,
            copy.as_deref(),
            keep_alive,
            service,
        ),
    }
}
