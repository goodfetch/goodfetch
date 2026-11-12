//! Routes, reaching the root's re-export through `super`.

pub(crate) fn route() -> u64 {
    let _beat = super::super::Tick;
    0
}
