//! The Product's connection.

/// A Usage kind.
pub const PAID_FETCHES: u32 = 1;

/// The Product's connection.
pub struct Foxhound {
    engine: u32,
}

impl Foxhound {
    /// Opens a connection.
    pub async fn connect() -> Self {
        Self { engine: 0 }
    }

    /// Closes it.
    pub async fn close(&self) {}

    pub(crate) fn engine_id(&self) -> u32 {
        self.engine
    }

    fn private(&self) {}

    #[doc(hidden)]
    pub fn hidden(&self) {}
}

#[cfg(test)]
impl Foxhound {
    pub fn fake() -> Self {
        Self { engine: 0 }
    }
}

impl Clone for Foxhound {
    fn clone(&self) -> Self {
        Self { engine: self.engine }
    }
}
