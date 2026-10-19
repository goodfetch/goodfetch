//! Routes, reaching a module its parent may not import.

pub(crate) fn route() -> u64 {
    super::super::clock::now()
}
