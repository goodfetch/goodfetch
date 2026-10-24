//! goodfetch-page's integration tests: its pure functions through the
//! crate's public interface, in process, and the call sites of the page's
//! value and response observer through a Recording gate on an in-process
//! engine's pages (spec #578's Testing Decisions; ADR 0125), and on those
//! pages a locator action's wait for the navigation it starts and a `goto`
//! the page's own navigation interrupts (ADR 0085), and the guard's
//! interception listener beside the page's own events (ADR 0019).

mod gesture_navigation;
mod guard_listener;
mod handler_page;
mod locator_fill;
mod navigation_gate;
mod page_navigation;
mod page_request;
mod robots_txt;
mod url_pattern;
