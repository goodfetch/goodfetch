//! Two features that each build alone and clash together.

#[cfg(feature = "first")]
pub fn engine() -> u8 {
    1
}

#[cfg(feature = "second")]
pub fn engine() -> u8 {
    2
}
