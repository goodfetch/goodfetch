//! Stub.

use super::{Page, Visit};
use crate::task::Task;

pub(super) const HOST: &str = "traps.test";
pub(super) const FAMILY: &str = "traps";

pub(super) fn tasks(_seed: u64, _port: u16) -> Vec<Task> {
    Vec::new()
}

#[derive(Default)]
pub(super) struct State;

pub(super) fn respond(_visit: &Visit<'_>, _state: &State) -> Page {
    Page::not_found()
}
