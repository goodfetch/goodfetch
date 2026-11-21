//! A private module whose public items no public path reaches.

/// Not re-exported.
pub const LIMIT: u32 = 1;

/// Not re-exported.
pub struct Internal;

impl Internal {
    /// Not reachable.
    pub fn reach(&self) {}
}
