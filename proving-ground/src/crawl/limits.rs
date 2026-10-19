//! Stub.

use super::{Page, Visit};
use crate::task::Task;

pub(super) const HOST: &str = "limits.test";
pub(super) const FAMILY: &str = "limits";

#[derive(Default)]
pub(super) struct State;

/// A held answer.
pub(crate) struct Held;

pub(super) fn tasks(_seed: u64, _port: u16) -> Vec<Task> {
    Vec::new()
}

pub(super) fn respond(_visit: &Visit<'_>, _state: &State) -> Page {
    Page::not_found()
}

impl Held {
    pub(crate) async fn release(self) {}
}
