//! Two additive features that build in every combination.

#[cfg(feature = "first")]
pub fn first() -> u8 {
    1
}

#[cfg(feature = "second")]
pub fn second() -> u8 {
    2
}
