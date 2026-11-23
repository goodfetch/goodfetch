//! The seed streams (ADR 0060): one Simulation seed, and a stream derived
//! from it for each consumer, so drawing more from one never shifts what
//! another draws.

use goodfetch_engine::entropy::Entropy;

/// The streams a Simulation seed derives.
pub(crate) struct SeedStreams {
    /// turmoil's own: its network's latencies, losses and host order.
    pub(crate) turmoil: u64,
    /// The Fault plan generator's.
    pub(crate) fault_plan: u64,
    /// The hosts': each host forks its own stream from it.
    hosts: Entropy,
}

impl SeedStreams {
    pub(crate) fn derive(seed: u64) -> Self {
        let mut root = seeded(seed);
        Self {
            turmoil: root.fork().next_u64(),
            fault_plan: root.fork().next_u64(),
            hosts: root.fork(),
        }
    }

    /// The next host's stream, in the order the scenario declares its
    /// hosts.
    pub(crate) fn host(&mut self) -> Entropy {
        self.hosts.fork()
    }
}

/// The seeded `Entropy` a Simulation draws from, which only tests build.
pub(crate) fn seeded(seed: u64) -> Entropy {
    #[expect(
        clippy::disallowed_methods,
        reason = "the Simulation is a test harness, whose chance comes from its seed (ADR 0060)"
    )]
    Entropy::seeded(seed)
}
