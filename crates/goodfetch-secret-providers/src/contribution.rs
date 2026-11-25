//! The crate's contribution to a Product's engine (ADR 0087): each built-in
//! external provider, built for each Run from the Run's
//! `secret_providers.<scheme>.*` and the engine's environment.

use std::sync::Arc;

use goodfetch_engine::Contribution;
use goodfetch_engine::secrets::{ProviderFactory, ProviderInputs, SecretProvider};
use goodfetch_protocol::secret_reference::Provider;

use crate::aws_sm::AwsSecretsManager;
use crate::azure_kv::AzureKeyVault;
use crate::bitwarden::Bitwarden;
use crate::gcp_sm::GcpSecretManager;
use crate::one_password::OnePassword;
use crate::vault::Vault;

/// The external providers of ADR 0015's list, by the scheme each answers.
pub fn contribution() -> Contribution {
    let factories: [(Provider, ProviderFactory); 6] = [
        (
            Provider::OnePassword,
            factory(|inputs| {
                Arc::new(OnePassword::configured(
                    inputs.settings,
                    inputs.environment.clone(),
                ))
            }),
        ),
        (
            Provider::Bitwarden,
            factory(|inputs| {
                Arc::new(Bitwarden::configured(
                    inputs.settings,
                    inputs.environment.clone(),
                ))
            }),
        ),
        (
            Provider::Vault,
            factory(|inputs| Arc::new(Vault::configured(inputs))),
        ),
        (
            Provider::AwsSecretsManager,
            factory(|inputs| Arc::new(AwsSecretsManager::configured(inputs))),
        ),
        (
            Provider::GcpSecretManager,
            factory(|inputs| Arc::new(GcpSecretManager::configured(inputs))),
        ),
        (
            Provider::AzureKeyVault,
            factory(|inputs| Arc::new(AzureKeyVault::configured(inputs))),
        ),
    ];
    factories.into_iter().fold(
        Contribution::new("goodfetch-secret-providers"),
        |contribution, (provider, factory)| contribution.with_secret_provider(provider, factory),
    )
}

/// `build` as a [`ProviderFactory`].
fn factory(
    build: impl for<'a> Fn(&ProviderInputs<'a>) -> Arc<dyn SecretProvider> + Send + Sync + 'static,
) -> ProviderFactory {
    Arc::new(build)
}
