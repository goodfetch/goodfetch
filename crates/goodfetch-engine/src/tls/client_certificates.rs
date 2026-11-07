//! Client certificates (ADRs 0015 and 0042): `tls.client_certificates`
//! maps a host or a registrable domain with its subdomains to the file of a
//! certificate and the Secret reference of its private key, so a site that
//! requires mutual TLS works with nothing installed on the machine. The
//! most specific listed domain of a host gives its certificate.
//!
//! The key is resolved through the Run's Secret-provider driver when the
//! first handshake with a host of the domain starts, never before, bound to
//! that domain, recorded in the Audit log and held for the rest of the Run;
//! the certificate file is read at the same moment. A host the Run reached
//! through an invalid-certificate exception gets no client certificate, as
//! it gets no Secret.

use std::cell::Cell;
use std::collections::BTreeMap;
use std::sync::Arc;

use goodfetch_protocol::run::RunId;
use goodfetch_protocol::secrets::SecretInputRefusedReason;
use goodfetch_protocol::settings::{SETTING_INVALID, SettingInvalidDetails, SettingsLayer};
use goodfetch_protocol::tls::{ClientCertificateConfiguration, TLS_CLIENT_CERTIFICATES};
use rustls::SignatureScheme;
use rustls::client::ResolvesClientCert;
use rustls::crypto::ring;
use rustls::sign::CertifiedKey;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Value;
use tokio::sync::OnceCell;

use super::{covered, domain_key, read_certificates, read_private_key};
use crate::call_error::CallError;
use crate::secrets::{Purpose, RunSecrets, SecretError};

/// The client certificates `tls.client_certificates` lists for a Run's
/// connections, and those its handshakes loaded. None by default. Clones
/// share what was loaded.
#[derive(Clone, Debug, Default)]
pub struct ClientCertificates {
    /// Each listed domain's certificate, by the domain as the URL parser
    /// leaves a host. Clones share what each loaded.
    listed: Arc<BTreeMap<String, Listed>>,
    /// The Run's Secrets, which resolve each key; none when nothing is
    /// listed.
    secrets: Option<RunSecrets>,
    /// The settings layer the value came from.
    layer: Option<SettingsLayer>,
    /// The Run whose connections present them.
    run_id: Option<RunId>,
}

/// One listed domain's certificate, and its certificate and key once a
/// handshake loaded them: the first handshake of the domain loads them, and
/// every later or concurrent one waits for that load.
#[derive(Debug)]
struct Listed {
    entry: ClientCertificateConfiguration,
    loaded: OnceCell<Arc<Loaded>>,
}

/// A listed domain's certificate and key, loaded: for rustls, and as its
/// files hold them.
#[derive(Debug)]
struct Loaded {
    certified: Arc<CertifiedKey>,
    files: IdentityFiles,
}

/// A listed client certificate as its files hold it: the certificate
/// file's bytes and the value of its key's reference, which is held for
/// the Run as every value the driver resolved is.
#[derive(Clone)]
pub struct IdentityFiles {
    /// The listed domain that gives it.
    pub domain: String,
    /// The certificate file's bytes, PEM.
    pub certificate: Arc<[u8]>,
    /// The value of its key's reference, PEM.
    pub key: SecretString,
}

impl std::fmt::Debug for IdentityFiles {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("IdentityFiles")
            .field("domain", &self.domain)
            .finish_non_exhaustive()
    }
}

/// Why a listed client certificate cannot be presented.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ClientCertificateFailure {
    /// Its key's reference gave no value, or the value may not go to the
    /// host, such as one reached through an invalid-certificate exception.
    #[error(transparent)]
    Secret(SecretError),
    /// Its entry, its certificate file or its key cannot be used:
    /// `setting_invalid` naming `tls.client_certificates`. The reason never
    /// holds the key.
    #[error("the client certificate of `{domain}` cannot be used: {reason}")]
    Unusable {
        /// The listed domain, as written.
        domain: String,
        /// The settings layer the value came from.
        layer: Option<SettingsLayer>,
        /// What is wrong with it.
        reason: String,
    },
}

impl ClientCertificateFailure {
    /// The `setting_invalid` the call that waited for the request answers
    /// a [`Self::Unusable`] failure with; a [`Self::Secret`] failure is
    /// answered as the driver's own (`run::secret_error`), so none.
    pub fn setting_invalid(&self) -> Option<CallError> {
        let Self::Unusable { layer, .. } = self else {
            return None;
        };
        Some(CallError::new(
            &SETTING_INVALID,
            self.to_string(),
            &SettingInvalidDetails {
                key: TLS_CLIENT_CERTIFICATES.name.to_owned(),
                layer: *layer,
                reason: self.to_string(),
            },
        ))
    }
}

impl ClientCertificates {
    /// The client certificates `setting`, the resolved value of
    /// `tls.client_certificates` from `layer`, lists for the Run `run_id`,
    /// whose keys `secrets` resolves.
    ///
    /// # Errors
    ///
    /// [`ClientCertificateFailure::Unusable`] when an entry names no host
    /// or is not a certificate file and a Secret reference.
    pub fn listed(
        setting: &Value,
        secrets: RunSecrets,
        layer: Option<SettingsLayer>,
        run_id: Option<RunId>,
    ) -> Result<Self, ClientCertificateFailure> {
        let mut listed = BTreeMap::new();
        for (domain, value) in setting.as_object().into_iter().flatten() {
            let unusable = |reason: String| ClientCertificateFailure::Unusable {
                domain: domain.clone(),
                layer,
                reason,
            };
            let key = domain_key(domain)
                .ok_or_else(|| unusable("it names no host or domain".to_owned()))?;
            let entry = ClientCertificateConfiguration::read(value).map_err(unusable)?;
            listed.insert(
                key,
                Listed {
                    entry,
                    loaded: OnceCell::new(),
                },
            );
        }
        Ok(Self {
            listed: Arc::new(listed),
            secrets: Some(secrets),
            layer,
            run_id,
        })
    }

    /// The most specific listed domain of `host`, with its certificate.
    fn entry_of(&self, host: &str) -> Option<(&str, &Listed)> {
        let found = Cell::new(None);
        covered(host, |domain| {
            found.set(
                self.listed
                    .get_key_value(domain)
                    .map(|(domain, entry)| (domain.as_str(), entry)),
            );
            found.get().is_some()
        });
        found.get()
    }

    /// The listed domain whose certificate `host` presents, its most
    /// specific: none when no listed domain covers it. A TLS library that
    /// picks a client certificate without the server name keeps one client
    /// per listed domain, and sends each host to its domain's.
    pub fn scope_of(&self, host: &str) -> Option<String> {
        self.entry_of(host).map(|(domain, _)| domain.to_owned())
    }

    /// The settings layer `tls.client_certificates` came from, if one did.
    pub fn layer(&self) -> Option<SettingsLayer> {
        self.layer
    }

    /// The client certificate a handshake with `host` presents, loaded now
    /// if no handshake has: none when no listed domain covers the host.
    ///
    /// # Errors
    ///
    /// [`ClientCertificateFailure::Secret`] when its key's reference gives
    /// no value or the host was reached through an exception;
    /// [`ClientCertificateFailure::Unusable`] when its file or its key
    /// cannot be used.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future abandons the load; a key the driver already
    /// resolved stays held for the Run.
    pub async fn for_host(
        &self,
        host: &str,
    ) -> Result<Option<Arc<CertifiedKey>>, ClientCertificateFailure> {
        Ok(self
            .loaded_for(host)
            .await?
            .map(|loaded| Arc::clone(&loaded.certified)))
    }

    /// The client certificate a handshake with `host` presents as its files
    /// hold it, with the listed domain that gives it, loaded now if no
    /// handshake has: none when no listed domain covers the host. For a TLS
    /// library that takes a client identity as its files' bytes rather than
    /// as a [`CertifiedKey`], such as `wreq`'s BoringSSL.
    ///
    /// # Errors
    ///
    /// As [`Self::for_host`]'s.
    ///
    /// # Cancel safety
    ///
    /// As [`Self::for_host`]'s.
    pub async fn files_for_host(
        &self,
        host: &str,
    ) -> Result<Option<IdentityFiles>, ClientCertificateFailure> {
        Ok(self
            .loaded_for(host)
            .await?
            .map(|loaded| loaded.files.clone()))
    }

    /// What the listed domain of `host` loaded, loading it now if no
    /// handshake has.
    async fn loaded_for(
        &self,
        host: &str,
    ) -> Result<Option<Arc<Loaded>>, ClientCertificateFailure> {
        let (Some((domain, listed)), Some(secrets)) = (self.entry_of(host), &self.secrets) else {
            return Ok(None);
        };
        if secrets.exception_hosts().reached(host).await.is_some() {
            return Err(ClientCertificateFailure::Secret(
                self.refused(host)
                    .expect("invariant: a listed domain covers the host"),
            ));
        }
        listed
            .loaded
            .get_or_try_init(|| self.load(host, domain, &listed.entry, secrets))
            .await
            .map(|loaded| Some(Arc::clone(loaded)))
    }

    /// Loads the certificate `entry` lists for `domain`, with its key
    /// resolved for a handshake with `host`.
    async fn load(
        &self,
        host: &str,
        domain: &str,
        entry: &ClientCertificateConfiguration,
        secrets: &RunSecrets,
    ) -> Result<Arc<Loaded>, ClientCertificateFailure> {
        let unusable = |reason: String| ClientCertificateFailure::Unusable {
            domain: domain.to_owned(),
            layer: self.layer,
            reason,
        };
        let file = tokio::fs::read(&entry.certificate).await.map_err(|error| {
            unusable(format!(
                "its certificate file `{}` cannot be read: {error}",
                entry.certificate
            ))
        })?;
        let chain = read_certificates(&file).map_err(|error| {
            unusable(format!(
                "its certificate file `{}`: {error}",
                entry.certificate
            ))
        })?;
        let domains = [domain.to_owned()];
        let purpose = Purpose {
            placeholder: TLS_CLIENT_CERTIFICATES.name,
            domain: Some(host),
            domains: &domains,
            layer: self.layer,
            run_id: self.run_id.as_ref(),
        };
        let value = secrets
            .resolve(&entry.key, &purpose)
            .await
            .map_err(ClientCertificateFailure::Secret)?;
        let key = read_private_key(value.expose_secret().as_bytes())
            .map_err(|error| unusable(format!("the value of its key's reference: {error}")))?;
        let certified = CertifiedKey::from_der(chain, key, &ring::default_provider())
            .map_err(|_| unusable("its key is not the key of its certificate".to_owned()))?;
        Ok(Arc::new(Loaded {
            certified: Arc::new(certified),
            files: IdentityFiles {
                domain: domain.to_owned(),
                certificate: Arc::from(file),
                key: value,
            },
        }))
    }

    /// The refusal of the client certificate listed for `host`, which was
    /// reached through an invalid-certificate exception: none when no listed
    /// domain covers it.
    pub fn refused(&self, host: &str) -> Option<SecretError> {
        let (_, listed) = self.entry_of(host)?;
        Some(SecretError::InputRefused {
            placeholder: TLS_CLIENT_CERTIFICATES.name.to_owned(),
            reference: listed.entry.key.clone(),
            domain: host.to_owned(),
            reason: SecretInputRefusedReason::InvalidCertificateException,
        })
    }
}

/// What a connection's handshake presents when its server asks for a
/// client certificate.
#[derive(Debug)]
pub(crate) struct Presenting(pub(crate) Arc<CertifiedKey>);

impl ResolvesClientCert for Presenting {
    fn resolve(
        &self,
        _root_hint_subjects: &[&[u8]],
        _sigschemes: &[SignatureScheme],
    ) -> Option<Arc<CertifiedKey>> {
        Some(Arc::clone(&self.0))
    }

    fn has_certs(&self) -> bool {
        true
    }
}
