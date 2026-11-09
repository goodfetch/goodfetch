//! Secret references' grammar (ADRs 0015 and 0092): the one form in which
//! a credential reaches the engine, `provider:rest`, for every provider
//! (`env:NAME`, `keychain:service/account`, `secretfile:name`,
//! `caller:name`, `op://vault/item/field`, `bw:item/field`,
//! `vault:mount/path#field`, `aws-sm:name#key`,
//! `gcp-sm:project/secret/version`, `azure-kv:vault/secret` and
//! `plugin:<name>:rest`), which a setting's value kind checks.
//!
//! A refusal never repeats the text it refused: a raw password given where a
//! reference belongs reaches no message.

use std::fmt;
use std::str::FromStr;

/// The longest reference, in bytes.
const MAXIMUM_LENGTH: usize = 1_024;

/// The providers a Secret reference names, by the scheme it opens with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Provider {
    /// `env:NAME`: a variable of the engine's environment.
    Environment,
    /// `keychain:service/account`: the OS keychain.
    Keychain,
    /// `secretfile:name`: the encrypted local file.
    SecretFile,
    /// `caller:name`: the program that opened the Run, through its
    /// `resolveSecret`.
    Caller,
    /// `op://vault/item/field`, or `op://vault/item/section/field`: 1Password.
    OnePassword,
    /// `bw:item/field`: Bitwarden and Vaultwarden.
    Bitwarden,
    /// `vault:mount/path#field`: Vault, by `HashiCorp`, and `OpenBao`.
    Vault,
    /// `aws-sm:name#key`, or `aws-sm:name` for the whole secret: AWS
    /// Secrets Manager.
    AwsSecretsManager,
    /// `gcp-sm:project/secret/version`: GCP Secret Manager.
    GcpSecretManager,
    /// `azure-kv:vault/secret`: Azure Key Vault.
    AzureKeyVault,
    /// `plugin:<name>:rest`: a Secret provider Plugin (ADR 0033).
    Plugin,
}

impl Provider {
    /// Every provider, in declaration order.
    pub const ALL: [Self; 11] = [
        Self::Environment,
        Self::Keychain,
        Self::SecretFile,
        Self::Caller,
        Self::OnePassword,
        Self::Bitwarden,
        Self::Vault,
        Self::AwsSecretsManager,
        Self::GcpSecretManager,
        Self::AzureKeyVault,
        Self::Plugin,
    ];

    /// Its scheme as a reference writes it, such as `env` or `aws-sm`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Environment => "env",
            Self::Keychain => "keychain",
            Self::SecretFile => "secretfile",
            Self::Caller => "caller",
            Self::OnePassword => "op",
            Self::Bitwarden => "bw",
            Self::Vault => "vault",
            Self::AwsSecretsManager => "aws-sm",
            Self::GcpSecretManager => "gcp-sm",
            Self::AzureKeyVault => "azure-kv",
            Self::Plugin => "plugin",
        }
    }

    /// The shape of its references, for a refusal's message.
    pub fn shape(self) -> &'static str {
        match self {
            Self::Environment => "env:NAME",
            Self::Keychain => "keychain:service/account",
            Self::SecretFile => "secretfile:name",
            Self::Caller => "caller:name",
            Self::OnePassword => "op://vault/item/field",
            Self::Bitwarden => "bw:item/field",
            Self::Vault => "vault:mount/path#field",
            Self::AwsSecretsManager => "aws-sm:name#key",
            Self::GcpSecretManager => "gcp-sm:project/secret/version",
            Self::AzureKeyVault => "azure-kv:vault/secret",
            Self::Plugin => "plugin:<name>:rest",
        }
    }

    /// Whether `rest`, what follows the scheme, is of this provider's shape.
    fn takes(self, rest: &str) -> bool {
        match self {
            Self::Environment => is_variable_name(rest),
            Self::SecretFile | Self::Caller => is_name(rest),
            Self::Keychain | Self::Bitwarden | Self::AzureKeyVault => segments(rest, 2..=2),
            Self::OnePassword => segments(rest, 3..=4),
            Self::GcpSecretManager => segments(rest, 3..=3),
            Self::Vault => rest
                .split_once('#')
                .is_some_and(|(path, field)| segments(path, 2..) && is_field(field)),
            Self::AwsSecretsManager => match rest.split_once('#') {
                Some((name, key)) => !name.is_empty() && is_field(key),
                None => !rest.is_empty(),
            },
            Self::Plugin => rest.split_once(':').is_some_and(|(name, rest)| {
                !rest.is_empty()
                    && name.starts_with(|first: char| first.is_ascii_lowercase())
                    && name.bytes().all(|byte| {
                        byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'
                    })
            }),
        }
    }
}

/// An environment variable's name: a letter or `_`, then letters, digits
/// and `_`.
fn is_variable_name(text: &str) -> bool {
    text.starts_with(|first: char| first.is_ascii_alphabetic() || first == '_')
        && text
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

/// A name of the encrypted local file or of the caller's: letters, digits,
/// `.`, `_` and `-`.
fn is_name(text: &str) -> bool {
    !text.is_empty()
        && text
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

/// A field after `#`: not empty, with no second `#`.
fn is_field(text: &str) -> bool {
    !text.is_empty() && !text.contains('#')
}

/// Whether `text` is `count` segments joined by `/`, none of them empty.
fn segments(text: &str, count: impl std::ops::RangeBounds<usize>) -> bool {
    let parts: Vec<&str> = text.split('/').collect();
    count.contains(&parts.len()) && parts.iter().all(|part| !part.is_empty())
}

/// Why a text is no Secret reference. No variant holds the text, so a raw
/// value given where a reference belongs reaches no message.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ReferenceError {
    /// It names no provider: no `provider:` opens it.
    #[error("it is no Secret reference: no provider such as `env:` opens it")]
    NoProvider,
    /// Its scheme is no provider's.
    #[error("it opens with no provider's scheme")]
    UnknownProvider,
    /// What follows the scheme is not of the provider's shape.
    #[error("it is no reference of the `{}` provider, whose shape is `{}`", .0.as_str(), .0.shape())]
    Malformed(Provider),
    /// It holds a space or a control character.
    #[error("it holds a space or a control character")]
    NotPrintable,
    /// It is longer than any reference.
    #[error("it is longer than 1024 bytes")]
    TooLong,
}

/// A Secret reference: the name by which a run points to a credential
/// without holding its value, `provider:rest`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SecretReference {
    text: String,
    provider: Provider,
}

impl SecretReference {
    /// Reads `text` as a Secret reference.
    ///
    /// # Errors
    ///
    /// Why it is none, never repeating it.
    pub fn parse(text: &str) -> Result<Self, ReferenceError> {
        if text.len() > MAXIMUM_LENGTH {
            return Err(ReferenceError::TooLong);
        }
        if text
            .chars()
            .any(|character| character.is_whitespace() || character.is_control())
        {
            return Err(ReferenceError::NotPrintable);
        }
        let (provider, rest) = if let Some(rest) = text.strip_prefix("op://") {
            (Provider::OnePassword, rest)
        } else {
            let (scheme, rest) = text.split_once(':').ok_or(ReferenceError::NoProvider)?;
            let provider = Provider::ALL
                .into_iter()
                .filter(|provider| *provider != Provider::OnePassword)
                .find(|provider| provider.as_str() == scheme)
                .ok_or(ReferenceError::UnknownProvider)?;
            (provider, rest)
        };
        if !provider.takes(rest) {
            return Err(ReferenceError::Malformed(provider));
        }
        Ok(Self {
            text: text.to_owned(),
            provider,
        })
    }

    /// The provider it names.
    pub fn provider(&self) -> Provider {
        self.provider
    }

    /// What follows its scheme: `NAME` of `env:NAME`, `vault/item/field` of
    /// `op://vault/item/field`.
    pub fn rest(&self) -> &str {
        let scheme = match self.provider {
            Provider::OnePassword => "op://".len(),
            provider => provider.as_str().len() + 1,
        };
        &self.text[scheme..]
    }

    /// Its text.
    pub fn as_str(&self) -> &str {
        &self.text
    }
}

impl fmt::Display for SecretReference {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.text)
    }
}

impl FromStr for SecretReference {
    type Err = ReferenceError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Self::parse(text)
    }
}

/// A Secret reference's values' JSON Schema pattern: a provider's scheme
/// and a rest with no space; the engine checks each provider's shape.
pub const REFERENCE_PATTERN: &str =
    "^(?:(?:env|keychain|secretfile|caller|bw|vault|aws-sm|gcp-sm|azure-kv|plugin):|op://)[^\\s]+$";

#[cfg(test)]
mod tests;
