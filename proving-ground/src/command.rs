//! The Proving ground's command line: `serve`, `tasks` and their arguments.

use std::net::SocketAddr;

use clap::{Args, Parser, ValueEnum};

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
    /// The network profile applied to every connection.
    #[arg(long, value_enum)]
    pub(crate) network_profile: NetworkProfile,
}

/// Which copy of the Proving ground runs (spec #152). The online copy is
/// #168's.
#[derive(Clone, Copy, ValueEnum)]
pub(crate) enum Copy {
    /// The local copy, which every published speed figure and every test uses.
    Local,
}

/// The added round-trip time and loss on every connection (spec #152). The
/// delayed and lossy profiles are #160's.
#[derive(Clone, Copy, ValueEnum)]
pub(crate) enum NetworkProfile {
    /// No added delay or loss.
    None,
}
