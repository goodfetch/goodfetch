//! A test's seeded draws (TESTING.md, Randomness): one `ChaCha20` stream per
//! use of the test's seed, keyed by the seed's 8 little-endian bytes and a
//! domain naming the use, so two uses of one seed (a crash VFS's power cuts
//! and a kill's moment in one test) draw unrelated values.

use chacha20::ChaCha20Rng;
use chacha20::rand_core::SeedableRng as _;

/// The stream of `seed` in `domain`, at most 24 bytes; the empty domain is
/// the seed's bytes then zeros, as `Moment::drawn` keyed it first.
pub(crate) fn stream(seed: u64, domain: &[u8]) -> ChaCha20Rng {
    assert!(
        domain.len() <= 24,
        "invariant: a seeded domain fits the key's last 24 bytes"
    );
    let mut key = [0_u8; 32];
    key[..8].copy_from_slice(&seed.to_le_bytes());
    key[8..8 + domain.len()].copy_from_slice(domain);
    ChaCha20Rng::from_seed(key)
}

#[cfg(test)]
mod tests {
    use chacha20::rand_core::Rng as _;

    use super::stream;

    #[test]
    fn two_domains_of_one_seed_draw_apart() {
        let first = stream(4242, b"crash-vfs").next_u64();
        let second = stream(4242, b"").next_u64();

        assert_ne!(first, second);
    }

    #[test]
    fn one_domain_of_one_seed_draws_alike() {
        assert_eq!(
            stream(4242, b"canaries").next_u64(),
            stream(4242, b"canaries").next_u64()
        );
    }
}
