//! The Scripted resolver (ADR 0067): an in-process sequence of DNS answers,
//! so rebinding and mixed answers are tested on every OS without a DNS
//! server.
#![expect(
    clippy::disallowed_types,
    reason = "test support depends on no Goodfetch crate, so not on the sync module loom replaces, and no loom model runs it (ADRs 0060, 0062)"
)]

use std::collections::VecDeque;
use std::fmt;
use std::net::IpAddr;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

/// One scripted lookup: the name the code under test must ask for and the
/// addresses it gets back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lookup {
    name: String,
    addresses: Vec<IpAddr>,
}

impl Lookup {
    /// The answer `addresses` to a lookup of `name`.
    pub fn new(name: &str, addresses: impl IntoIterator<Item = IpAddr>) -> Self {
        Self {
            name: name.to_owned(),
            addresses: addresses.into_iter().collect(),
        }
    }
}

/// How code under test broke a resolver's script: the panic payload of the
/// lookup, and of the resolver's drop when that panic was swallowed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LookupFailure {
    /// A lookup after the script's last.
    BeyondScript {
        /// The name asked for.
        name: String,
        /// How many lookups the script held.
        scripted: usize,
    },
    /// A lookup of another name than the script's next.
    OtherName {
        /// The script's next name.
        expected: String,
        /// The name asked for.
        name: String,
    },
}

/// A test's DNS answers, given out in order. The test keeps it and hands
/// the code under test a [`ResolverHandle`]. A lookup that breaks the script
/// panics with a [`LookupFailure`], and dropping the resolver panics with it
/// again, so a failure the code under test swallowed (a task's panic, a
/// caught unwind) still fails the test. A lookup left unasked is no failure:
/// code that keeps an answer for its TTL asks fewer times.
pub struct ScriptedResolver {
    state: Arc<Mutex<Script>>,
}

/// The lookups not yet asked for, and the first failure.
struct Script {
    lookups: VecDeque<Lookup>,
    scripted: usize,
    failure: Option<LookupFailure>,
}

impl ScriptedResolver {
    /// A resolver that answers `script`'s lookups, one per call, in order.
    pub fn new(script: impl IntoIterator<Item = Lookup>) -> Self {
        let lookups: VecDeque<Lookup> = script.into_iter().collect();
        Self {
            state: Arc::new(Mutex::new(Script {
                scripted: lookups.len(),
                lookups,
                failure: None,
            })),
        }
    }

    /// A handle the code under test resolves through.
    pub fn handle(&self) -> ResolverHandle {
        ResolverHandle {
            state: Arc::clone(&self.state),
        }
    }
}

impl Drop for ScriptedResolver {
    fn drop(&mut self) {
        if std::thread::panicking() {
            return;
        }
        let failure = lock(&self.state).failure.take();
        if let Some(failure) = failure {
            fail(failure);
        }
    }
}

/// What the code under test holds of a [`ScriptedResolver`]: cloned freely
/// and sent to other threads.
#[derive(Clone)]
pub struct ResolverHandle {
    state: Arc<Mutex<Script>>,
}

impl ResolverHandle {
    /// The script's next answer, which must be for `name`.
    ///
    /// # Panics
    ///
    /// With a [`LookupFailure`] when the script holds no more lookups or its
    /// next one is for another name.
    pub fn lookup(&self, name: &str) -> Vec<IpAddr> {
        let mut script = lock(&self.state);
        let failure = match script.lookups.front() {
            Some(next) if next.name == name => {
                return script
                    .lookups
                    .pop_front()
                    .map(|lookup| lookup.addresses)
                    .unwrap_or_default();
            }
            Some(next) => LookupFailure::OtherName {
                expected: next.name.clone(),
                name: name.to_owned(),
            },
            None => LookupFailure::BeyondScript {
                name: name.to_owned(),
                scripted: script.scripted,
            },
        };
        script.failure.get_or_insert_with(|| failure.clone());
        drop(script);
        fail(failure)
    }
}

impl fmt::Display for LookupFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BeyondScript { name, scripted } => write!(
                formatter,
                "the Scripted resolver was asked for {name} after its {scripted} scripted lookups"
            ),
            Self::OtherName { expected, name } => write!(
                formatter,
                "the Scripted resolver was asked for {name} where its script expected {expected}"
            ),
        }
    }
}

/// Fails the test with `failure` as its typed payload.
fn fail(failure: LookupFailure) -> ! {
    #[expect(
        clippy::print_stderr,
        reason = "the failure's reason, since a typed panic payload prints none (ADR 0069)"
    )]
    {
        eprintln!("{failure}");
    }
    std::panic::panic_any(failure)
}

/// The script, even after a panicking thread poisoned its lock.
fn lock(state: &Mutex<Script>) -> MutexGuard<'_, Script> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests;
