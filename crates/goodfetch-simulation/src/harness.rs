//! The Simulation: turmoil's world, built from one seed, whose hosts boot
//! with a paused Clock, their own Entropy and a task owner whose blocking
//! work runs inline.

use std::cell::RefCell;
use std::future::Future;
use std::num::NonZeroUsize;
use std::rc::Rc;
use std::time::{Duration, SystemTime};

use goodfetch_engine::clock::Clock;
use goodfetch_engine::entropy::Entropy;
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};

use crate::seeds::{SeedStreams, seeded};

/// A run of hosts on turmoil's network, from one seed.
pub struct Simulation {
    world: turmoil::Sim<'static>,
    start: SystemTime,
    seeds: SeedStreams,
}

impl Simulation {
    /// A Simulation of `seed`, whose wall time starts at `start`.
    pub fn new(seed: u64, start: SystemTime) -> Self {
        let seeds = SeedStreams::derive(seed);
        let world = turmoil::Builder::new()
            .rng_seed(seeds.turmoil)
            .epoch(start)
            .build();
        Self {
            world,
            start,
            seeds,
        }
    }

    /// The seed of the Fault plan generator's stream of the Simulation of
    /// `seed`: proptest generates that Simulation's plans from it.
    pub fn fault_plan_seed(seed: u64) -> u64 {
        SeedStreams::derive(seed).fault_plan
    }

    /// Declares the host `name`, which `boot` runs, and runs again each
    /// time a restart brings it back: each boot reads wall time from where
    /// the Simulation is, and draws from the next seed of the host's
    /// stream.
    pub fn host<F, Fut>(&mut self, name: &str, boot: F)
    where
        F: Fn(Host) -> Fut + 'static,
        Fut: Future<Output = turmoil::Result> + 'static,
    {
        let start = self.start;
        let stream = Rc::new(RefCell::new(self.seeds.host()));
        let boot = Rc::new(boot);
        self.world.host(name, move || {
            let (stream, boot) = (Rc::clone(&stream), Rc::clone(&boot));
            async move {
                let entropy = seeded(stream.borrow_mut().next_u64());
                boot(Host::booted(start, entropy)).await
            }
        });
    }

    /// Declares the client `name`, which runs `run` once; the Simulation
    /// runs until every client is done.
    pub fn client<F, Fut>(&mut self, name: &str, run: F)
    where
        F: FnOnce(Host) -> Fut + 'static,
        Fut: Future<Output = turmoil::Result> + 'static,
    {
        let start = self.start;
        let mut stream = self.seeds.host();
        self.world.client(name, async move {
            let entropy = seeded(stream.next_u64());
            run(Host::booted(start, entropy)).await
        });
    }

    /// turmoil's world, for what [`Fault`] does not name.
    pub fn world(&mut self) -> &mut turmoil::Sim<'static> {
        &mut self.world
    }

    /// How far the Simulation's time has moved.
    pub fn elapsed(&self) -> Duration {
        self.world.elapsed()
    }

    /// Injects `fault` into the host `host`, now.
    ///
    /// # Panics
    ///
    /// When `host` names no host, or names a client, which turmoil neither
    /// crashes nor restarts.
    pub fn inject(&mut self, host: &str, fault: &Fault) {
        match fault {
            Fault::Crash => self.world.crash(host),
            Fault::Restart => self.world.bounce(host),
            Fault::Partition { peer } => self.world.partition(host, peer.as_str()),
            Fault::Heal { peer } => self.world.repair(host, peer.as_str()),
            Fault::Latency { peer, delay } => {
                self.world.set_link_latency(host, peer.as_str(), *delay);
            }
        }
    }

    /// Runs until every client is done, injecting each fault once the
    /// Simulation's time reaches its moment, those of one moment in their
    /// order; a fault whose moment the run does not reach is not injected.
    ///
    /// # Errors
    ///
    /// As [`Simulation::run`].
    ///
    /// # Panics
    ///
    /// As [`Simulation::inject`].
    pub fn run_injecting(
        &mut self,
        injections: impl IntoIterator<Item = Injection>,
    ) -> turmoil::Result {
        let mut injections: Vec<Injection> = injections.into_iter().collect();
        // A stable sort: one moment's faults keep their order.
        injections.sort_by_key(|injection| injection.at);
        let mut pending = injections.into_iter().peekable();
        loop {
            while let Some(injection) = pending.next_if(|next| next.at <= self.elapsed()) {
                self.inject(&injection.host, &injection.fault);
            }
            if self.step()? {
                return Ok(());
            }
        }
    }

    /// Moves every host one tick on; whether every client is done.
    ///
    /// # Errors
    ///
    /// The first host's failure.
    pub fn step(&mut self) -> turmoil::Result<bool> {
        self.world.step()
    }

    /// Runs until every client is done.
    ///
    /// # Errors
    ///
    /// The first host's failure, or turmoil's when the run outlasts its
    /// duration.
    pub fn run(&mut self) -> turmoil::Result {
        self.world.run()
    }
}

/// A fault turmoil injects into a host: the network's, and its process's.
/// turmoil 0.7.2 cannot lose messages already in flight, and the paused
/// Clock has no wall-time jump yet, so a Fault plan's `drop` and
/// `jump_wall` have no fault here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fault {
    /// The host's process crashes: its tasks stop, and the connections it
    /// held carry nothing more.
    Crash,
    /// The host's process starts again, as a new boot; a running one
    /// crashes first.
    Restart,
    /// The link between the host and `peer` loses each message sent on it.
    Partition {
        /// The host on the link's other end.
        peer: String,
    },
    /// The link between the host and `peer` carries messages again.
    Heal {
        /// The host on the link's other end.
        peer: String,
    },
    /// The link between the host and `peer` delays each message by
    /// `delay`.
    Latency {
        /// The host on the link's other end.
        peer: String,
        /// The delay.
        delay: Duration,
    },
}

/// A fault to inject into a host at a moment of the Simulation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Injection {
    /// The moment, from the Simulation's start.
    pub at: Duration,
    /// The host.
    pub host: String,
    /// The fault.
    pub fault: Fault,
}

/// What a host's boot runs with.
pub struct Host {
    clock: Clock,
    entropy: Entropy,
    owner: TaskOwner,
}

impl Host {
    /// Called on the host's runtime, where its paused Clock starts.
    fn booted(start: SystemTime, entropy: Entropy) -> Self {
        let since = turmoil::sim_elapsed().unwrap_or_default();
        #[expect(
            clippy::disallowed_methods,
            reason = "the Simulation is a test harness, whose hosts run on paused time (ADR 0060)"
        )]
        let clock = Clock::paused(start + since);
        Self {
            clock,
            entropy,
            owner: TaskOwner::root_on_cores(BlockingPolicy::Inline, NonZeroUsize::MIN),
        }
    }

    /// Its Clock, which reads the Simulation's time.
    pub fn clock(&self) -> &Clock {
        &self.clock
    }

    /// Its Entropy, drawn from this boot's seed.
    pub fn entropy(&mut self) -> &mut Entropy {
        &mut self.entropy
    }

    /// The owner of its tasks.
    pub fn owner(&self) -> &TaskOwner {
        &self.owner
    }
}
