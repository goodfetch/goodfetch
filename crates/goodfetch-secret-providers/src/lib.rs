//! The built-in external Secret providers (spec #564; ADRs 0015, 0033 and
//! 0129): 1Password's, `op://vault/item/field`, Bitwarden's and
//! Vaultwarden's, `bw:item/field`, `HashiCorp` Vault's and `OpenBao`'s,
//! `vault:mount/path#field`, AWS Secrets Manager's, `aws-sm:name#key`, GCP
//! Secret Manager's, `gcp-sm:project/secret/version`, and Azure Key
//! Vault's, `azure-kv:vault/secret`. They live apart from
//! `goodfetch-engine`'s `secrets`, which keeps the environment, the OS
//! keychain, the encrypted local file and the caller, since only a Product
//! that offers them needs their clients. Each fills the driver's two
//! primitives,
//! [`SecretProvider::resolve`](goodfetch_engine::secrets::SecretProvider::resolve)
//! and `check_config`, and the driver does the rest: the moment of use, the
//! domain check, the timeout, the Audit log and the `Mask`. A Product's
//! engine registers them all through [`contribution`], each built for each
//! Run from the Run's `secret_providers.<scheme>.*`.
//!
//! 1Password and Bitwarden each run their vendor's CLI, the interface the vendor documents for
//! scripts, as an operator-configured command (`secret_providers.<scheme>.command`,
//! set only in configuration or an Operator policy), with the engine's own
//! environment (ADR 0083), so the CLI's own sign-in, such as
//! `OP_SERVICE_ACCOUNT_TOKEN` or `BW_SESSION`, reaches it. Vault and the
//! three cloud managers read their server's HTTP API through the engine's
//! own HTTP client, as Operator-configured endpoints
//! (`secret_providers.vault.address`, `secret_providers.aws_sm.*`,
//! `secret_providers.gcp_sm.endpoint`, `secret_providers.azure_kv.*`), with
//! the credentials in the engine's environment. Each client is a
//! translation point (ADR 0127): what its party answers on a failure
//! becomes a [`ProviderFailure`](goodfetch_engine::secrets::ProviderFailure)
//! there, an unknown failure `Unrecognized`, which the driver answers with
//! `secret_unavailable`, `failure: "provider_failed"`.

pub mod aws_sm;
pub mod azure_kv;
pub mod bitwarden;
mod cli;
mod contribution;
mod endpoint;
pub mod gcp_sm;
pub mod one_password;
pub mod vault;

pub use aws_sm::AwsSecretsManager;
pub use azure_kv::AzureKeyVault;
pub use bitwarden::Bitwarden;
pub use contribution::contribution;
pub use gcp_sm::GcpSecretManager;
pub use one_password::OnePassword;
pub use vault::Vault;
