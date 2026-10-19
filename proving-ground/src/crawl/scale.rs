//! Stub.

use super::{Page, Visit};
use crate::task::Task;

pub(super) const HOST: &str = "scale.test";
pub(super) const FAMILY: &str = "scale";

pub(super) fn tasks(_seed: u64, _port: u16) -> Vec<Task> {
    Vec::new()
}

pub(super) fn respond(_visit: &Visit<'_>) -> Page {
    Page::not_found()
}
