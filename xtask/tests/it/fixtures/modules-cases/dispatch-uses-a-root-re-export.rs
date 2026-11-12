//! Dispatch, importing a module it may not through the root's re-export.

mod route;

use crate::Tick;

pub fn dispatch() -> u64 {
    let _beat = Tick;
    route::route()
}
