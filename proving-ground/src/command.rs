//! The Proving ground's command line: `serve`, `tasks` and their arguments.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use clap::{Args, Parser, ValueEnum};

use crate::network::Loss;

/// The Proving ground: Goodfetch's seeded website, its task list and its oracle.
#[derive(Parser)]
#[command(version = crate::version::VERSION)]
pub(crate) enum Command {
    /// Serves every page family, the task list and the oracle until stopped;
    /// the first output line is one JSON object with `address`, `version` and
    /// `seed`.
    Serve(Serve),
    /// Prints the task list `serve` serves for a seed, as one JSON line with
    /// each start URL on no port, and exits; `xtask journeys tasks` commits
    /// it (ADR 0063).
    Tasks(Tasks),
}

#[derive(Args)]
pub(crate) struct Tasks {
    /// The seed the tasks' expected results are generated from.
    #[arg(long)]
    pub(crate) seed: u64,
}

#[derive(Args)]
pub(crate) struct Serve {
    /// The seed every page and expected result is generated from.
    #[arg(long)]
    pub(crate) seed: u64,
    /// Which copy to serve.
    #[arg(long, value_enum)]
    pub(crate) copy: Copy,
    /// The address to bind; port 0 binds a free port, which the output line
    /// reports.
    #[arg(long)]
    pub(crate) address: SocketAddr,
    /// The network profile applied to every connection, calibrated before
    /// the output line, which records it.
    #[arg(long, value_enum)]
    pub(crate) network_profile: NetworkProfile,
    /// The percentage of packets a shaped profile loses in each direction,
    /// such as `1` or `0.5`.
    #[arg(long, default_value = "0")]
    pub(crate) loss: Loss,
    /// What applies a shaped profile: the user-space latency proxy on any
    /// OS, without privileges, or `tc netem` on Linux, for published
    /// figures, with `CAP_NET_ADMIN`.
    #[arg(long, value_enum, default_value_t = Shaper::Proxy)]
    pub(crate) shaper: Shaper,
    /// A folder whose files `browser-builds.test` serves, each at its path
    /// below it, read once at the start; without it, the host answers
    /// every request `404`.
    #[arg(long)]
    pub(crate) browser_builds: Option<PathBuf>,
    /// The online copy's vendors, a TOML file with one section per vendor
    /// it carries (`online.rs`); required by `--copy online`, refused by
    /// `--copy local`.
    #[arg(long)]
    pub(crate) online_config: Option<PathBuf>,
}

/// Which copy of the Proving ground runs (spec #152).
#[derive(Clone, Copy, ValueEnum)]
pub(crate) enum Copy {
    /// The local copy, which every published speed figure and every test uses.
    Local,
    /// The online copy (#168), behind real vendor protections on a domain
    /// Goodfetch owns, which verifies each vendor's verdict on the server
    /// side.
    Online,
}

/// The added round-trip time on every connection (ADR 0025's profiles).
#[derive(Clone, Copy, ValueEnum)]
pub(crate) enum NetworkProfile {
    /// The network as it is: nothing added, nothing shaped or calibrated.
    None,
    /// No added round trip, through the shaper.
    #[value(name = "rtt-0")]
    Rtt0,
    /// 20 ms of added round-trip time.
    #[value(name = "rtt-20")]
    Rtt20,
    /// 100 ms of added round-trip time.
    #[value(name = "rtt-100")]
    Rtt100,
    /// 300 ms of added round-trip time.
    #[value(name = "rtt-300")]
    Rtt300,
}

impl NetworkProfile {
    /// The profile's name and added round trip, unless it is `none`.
    pub(crate) fn round_trip(self) -> Option<(&'static str, Duration)> {
        let (name, milliseconds) = match self {
            Self::None => return None,
            Self::Rtt0 => ("rtt-0", 0),
            Self::Rtt20 => ("rtt-20", 20),
            Self::Rtt100 => ("rtt-100", 100),
            Self::Rtt300 => ("rtt-300", 300),
        };
        Some((name, Duration::from_millis(milliseconds)))
    }
}

/// What applies a shaped profile.
#[derive(Clone, Copy, ValueEnum)]
pub(crate) enum Shaper {
    /// The user-space latency proxy, on any OS.
    Proxy,
    /// `tc netem` on Linux's loopback interface.
    Netem,
}
