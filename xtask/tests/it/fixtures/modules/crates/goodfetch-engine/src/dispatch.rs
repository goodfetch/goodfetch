//! Dispatch.

mod route;

pub fn dispatch() -> u64 {
    route::route()
}
