//! Seeded values: every value a page or an expected result holds is a pure
//! function of the seed and the value's place, drawn with `SplitMix64` written
//! here, so no dependency's update can change the data a seed gives (ADR
//! 0063: a seed pins data only while its generator stays the same).

/// The value at `place` for `seed`.
pub(crate) fn draw(seed: u64, place: &[u64]) -> u64 {
    place
        .iter()
        .fold(mix(seed), |state, part| mix(state ^ mix(*part)))
}

/// A value below `bound` at `place` for `seed`. The modulo's bias is below
/// one in 2^40 for the bounds the families use, which no expected result
/// depends on.
pub(crate) fn below(seed: u64, place: &[u64], bound: u64) -> u64 {
    draw(seed, place) % bound
}

/// `SplitMix64`'s output function over one step of its Weyl sequence.
fn mix(value: u64) -> u64 {
    let mut z = value.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}
