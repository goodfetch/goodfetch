//! `goodfetch-notification`'s channels against local receivers: an
//! `httpmock` server, or a socket that never answers, so each test binds
//! sockets and sits here rather than among the unit tests (TESTING.md,
//! Dependencies in tests).
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#![expect(
    clippy::unwrap_used,
    reason = "test code, where STYLE.md allows unwrap; Clippy's test exemption does not reach integration-test helpers"
)]

mod chat;
mod email;
mod email_api;
mod sns;
mod webhook;
