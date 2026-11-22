//! The `Entropy` handle (ADR 0059): every random choice the engine makes,
//! such as the random part of a ULID, from one portable seeded generator,
//! `ChaCha20`. Its seed comes from the OS in production and from the test's
//! seed in tests ([`Entropy::seeded`]). Keys, tokens and run data keys never
//! come from it, but from the OS generator. A live Run's [`RunEntropy`] is
//! the stream each of its owners forks its own from (ADR 0145).

use std::sync::Arc;

use chacha20::ChaCha20Rng;
use chacha20::rand_core::{Rng as _, SeedableRng as _};

/// Where the engine draws its random choices.
#[derive(Debug)]
pub struct Entropy(ChaCha20Rng);

/// The OS gave no seed.
#[derive(Debug, thiserror::Error)]
#[error("the OS generator gave no seed: {0}")]
pub struct NoSeed(getrandom::Error);

impl Entropy {
    /// Seeded from the OS generator, which only `main` builds.
    pub fn from_os() -> Result<Self, NoSeed> {
        let mut key = [0; 32];
        getrandom::fill(&mut key).map_err(NoSeed)?;
        Ok(Self(ChaCha20Rng::from_seed(key)))
    }

    /// The test version, seeded from `seed`, so a test draws the same
    /// choices every run. `disallowed-methods` refuses it outside tests and
    /// bench targets.
    pub fn seeded(seed: u64) -> Self {
        let mut key = [0; 32];
        key[..8].copy_from_slice(&seed.to_le_bytes());
        Self(ChaCha20Rng::from_seed(key))
    }

    /// A draw of 64 random bits.
    pub fn next_u64(&mut self) -> u64 {
        self.0.next_u64()
    }

    /// An `Entropy` of its own for another owner, seeded from this one, so
    /// that each owner's draws stay in one order.
    #[must_use]
    pub fn fork(&mut self) -> Self {
        let mut key = [0; 32];
        self.0.fill_bytes(&mut key);
        Self(ChaCha20Rng::from_seed(key))
    }
}

/// A live Run's entropy (ADRs 0059 and 0145): the stream the engine forked
/// for it when it went live, which each of its owners, such as its driver,
/// forks an `Entropy` of its own from, in the order they ask. Clones share
/// the stream.
#[derive(Clone, Debug)]
pub struct RunEntropy(Arc<tokio::sync::Mutex<Entropy>>);

impl RunEntropy {
    /// The entropy of a Run whose stream is `stream`.
    pub fn new(stream: Entropy) -> Self {
        Self(Arc::new(tokio::sync::Mutex::new(stream)))
    }

    /// An `Entropy` of its own for one of the Run's owners, forked from the
    /// Run's stream.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses only the answer.
    pub async fn fork(&self) -> Entropy {
        self.0.lock().await.fork()
    }
}
