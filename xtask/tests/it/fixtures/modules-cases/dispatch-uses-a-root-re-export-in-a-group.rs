//! Dispatch, importing a module it may not through the root's re-export,
//! inside a use group.

mod route;

use crate::{Tick, retry};

pub fn dispatch() -> u64 {
    let _beat = Tick;
    route::route() + retry::retry()
}
