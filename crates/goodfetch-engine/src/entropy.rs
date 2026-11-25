//! The `Entropy` handle (ADR 0059): every random choice the engine makes,
//! such as the random part of a ULID, from one portable seeded generator,
//! `ChaCha20`. Its seed comes from the OS in production and from the test's
//! seed in tests ([`Entropy::seeded`]). Keys, tokens and run data keys never
//! come from it, but from the OS generator.

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

    /// An `Entropy` of its own on the stream `stream` of this one's seed,
    /// which draws nothing from this one, so the draws of every other owner
    /// stay where they were.
    #[must_use]
    pub fn stream(&self, stream: u64) -> Self {
        let mut generator = ChaCha20Rng::from_seed(self.0.get_seed());
        generator.set_stream(stream);
        Self(generator)
    }
}
