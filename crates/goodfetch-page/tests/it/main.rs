//! goodfetch-page's integration tests: its pure functions through the
//! crate's public interface, in process, and the call sites of the page's
//! value and response observer through a Recording gate on an in-process
//! engine's pages (spec #578's Testing Decisions; ADR 0125), and on those
//! pages a locator action's wait for the navigation it starts (ADR 0085).

mod gesture_navigation;
mod handler_page;
mod navigation_gate;
mod robots_txt;
mod url_pattern;
