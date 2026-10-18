//! Dispatch, importing a module it may not inside a use group.

mod route;

use crate::{clock, retry};

pub fn dispatch() -> u64 {
    route::route() + retry::retry() + clock::now()
}
