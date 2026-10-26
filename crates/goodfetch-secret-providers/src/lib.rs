//! The built-in external Secret providers (spec #564; ADRs 0015, 0033 and
//! 0129): 1Password's, `op://vault/item/field`, and Bitwarden's and
//! Vaultwarden's, `bw:item/field`. They live apart from `goodfetch-engine`'s
//! `secrets`, which keeps the environment, the OS keychain, the encrypted
//! local file and the caller, since only a Product that offers them needs
//! their clients. Each fills the driver's two primitives,
//! [`SecretProvider::resolve`](goodfetch_engine::secrets::SecretProvider::resolve)
//! and `check_config`, and the driver does the rest: the moment of use, the
//! domain check, the timeout, the Audit log and the `Mask`.
//!
//! Each runs its vendor's CLI, the interface the vendor documents for
//! scripts, as an operator-configured command (`secret_providers.<scheme>.command`,
//! set only in configuration or an Operator policy), with the engine's own
//! environment (ADR 0083), so the CLI's own sign-in, such as
//! `OP_SERVICE_ACCOUNT_TOKEN` or `BW_SESSION`, reaches it. Each client is a
//! translation point (ADR 0127): what its CLI writes on a failure becomes a
//! [`ProviderFailure`](goodfetch_engine::secrets::ProviderFailure) there,
//! an unknown failure `Unrecognized`, which the driver answers with
//! `secret_unavailable`, `failure: "provider_failed"`.

pub mod bitwarden;
mod cli;
pub mod one_password;

pub use bitwarden::Bitwarden;
pub use one_password::OnePassword;
