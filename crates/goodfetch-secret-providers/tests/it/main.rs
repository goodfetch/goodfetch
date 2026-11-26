//! `goodfetch-secret-providers` through the engine's Secret-provider
//! driver, each provider's CLI a local stand-in that answers as its vendor
//! documents (ADR 0062), so no account is needed.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#![expect(
    clippy::unwrap_used,
    reason = "test code, where STYLE.md allows unwrap; Clippy's test exemption does not reach integration-test helpers"
)]

#[cfg(unix)]
mod providers;

mod aws_access_keys;
mod azure_access_tokens;
mod clouds;
mod gcp_access_tokens;
mod support;
mod vault;
