//! Reads and opens the Exposed Daemon's TLS termination settings
//! (`daemon.tls.cert_file`, `daemon.tls.key_file`, `daemon.tls.upstream`,
//! `daemon.trusted_proxies`, ADR 0049) shared by `daemon run` (`foreground`)
//! and the Daemon's HTTP listener (`http_listener`).

use std::path::PathBuf;

use goodfetch_engine::endpoint::Termination;
use goodfetch_engine::settings::{Resolved, SettingsError};
use goodfetch_engine::tls::{ServingCertificate, ServingCertificateError};
use goodfetch_protocol::catalog::Setting;
use goodfetch_protocol::daemon::{
    TLS_CERT_FILE, TLS_KEY_FILE, TLS_UPSTREAM, TRUSTED_PROXIES, TrustedProxy,
};
use goodfetch_protocol::settings::SettingsLayer;

/// Where a Daemon's TLS is terminated, as its settings say.
pub(super) enum Terminating {
    /// Nowhere: it listens on loopback.
    Plain,
    /// By the Daemon, with these files.
    Files(TlsFiles),
    /// In front of it (`daemon.tls.upstream`), these proxies' `X-Forwarded-*`
    /// read (`daemon.trusted_proxies`).
    Upstream(Vec<TrustedProxy>),
}

impl Terminating {
    /// Opens the configured TLS termination, reading the certificate files
    /// when configured.
    pub(super) fn open(&self) -> Result<Termination, SettingsError> {
        match self {
            Self::Plain => Ok(Termination::Plain),
            Self::Files(files) => files.open().map(Termination::Tls),
            Self::Upstream(proxies) => Ok(Termination::Upstream(proxies.clone())),
        }
    }
}

/// The certificate and key files an Exposed Daemon serves TLS with, and where
/// each was set.
pub(super) struct TlsFiles {
    certificate: PathBuf,
    certificate_layer: SettingsLayer,
    key: PathBuf,
    key_layer: SettingsLayer,
}

impl TlsFiles {
    /// The certificate they hold, read now, before the Daemon serves:
    /// `setting_invalid` naming the file at fault otherwise.
    fn open(&self) -> Result<ServingCertificate, SettingsError> {
        ServingCertificate::open(self.certificate.clone(), self.key.clone()).map_err(|error| {
            let (key, layer) = match error {
                ServingCertificateError::Certificate { .. } => {
                    (TLS_CERT_FILE.name, self.certificate_layer)
                }
                ServingCertificateError::Key { .. } => (TLS_KEY_FILE.name, self.key_layer),
            };
            SettingsError::Invalid {
                key: key.to_owned(),
                layer: Some(layer),
                reason: error.to_string(),
            }
        })
    }
}

/// Reads `daemon.tls.*` from `settings` without yet opening the files.
pub(super) fn read_termination(settings: &Resolved) -> Result<Terminating, SettingsError> {
    let files = tls_files(
        file_setting(settings, &TLS_CERT_FILE),
        file_setting(settings, &TLS_KEY_FILE),
    )?;
    let upstream = settings
        .get::<bool>(&TLS_UPSTREAM)
        .then(|| setting_layer(settings, &TLS_UPSTREAM));
    let proxies = trusted_proxies(settings)?;
    terminating(files, upstream, proxies)
}

/// Where TLS is terminated, from the Daemon's TLS files, `upstream`'s layer
/// when `daemon.tls.upstream` is true, and the proxies `daemon.trusted_proxies`
/// lists, if any.
fn terminating(
    files: Option<TlsFiles>,
    upstream: Option<SettingsLayer>,
    proxies: Option<(Vec<TrustedProxy>, SettingsLayer)>,
) -> Result<Terminating, SettingsError> {
    match (files, upstream, proxies) {
        (None, None, None) => Ok(Terminating::Plain),
        (Some(files), None, None) => Ok(Terminating::Files(files)),
        (None, Some(_), proxies) => Ok(Terminating::Upstream(
            proxies.map(|(proxies, _)| proxies).unwrap_or_default(),
        )),
        (Some(_), Some(layer), _) => Err(invalid(
            TLS_UPSTREAM.name,
            layer,
            format!(
                "TLS is terminated either by the Daemon or upstream: unset `{}`, or unset `{}` and `{}`",
                TLS_UPSTREAM.name, TLS_CERT_FILE.name, TLS_KEY_FILE.name
            ),
        )),
        (_, None, Some((_, layer))) => Err(invalid(
            TRUSTED_PROXIES.name,
            layer,
            format!(
                "`{}` is read only behind TLS terminated upstream: set `{}` too, or unset this one",
                TRUSTED_PROXIES.name, TLS_UPSTREAM.name
            ),
        )),
    }
}

/// Reads and opens `daemon.tls.*` from `settings`.
pub(in crate::executable) fn open_termination(
    settings: &Resolved,
) -> Result<Termination, SettingsError> {
    read_termination(settings)?.open()
}

fn setting_layer(settings: &Resolved, setting: &Setting) -> SettingsLayer {
    settings
        .effective(setting)
        .map_or(SettingsLayer::Default, |effective| effective.layer)
}

fn invalid(key: &str, layer: SettingsLayer, reason: String) -> SettingsError {
    SettingsError::Invalid {
        key: key.to_owned(),
        layer: Some(layer),
        reason,
    }
}

fn trusted_proxies(
    settings: &Resolved,
) -> Result<Option<(Vec<TrustedProxy>, SettingsLayer)>, SettingsError> {
    let layer = setting_layer(settings, &TRUSTED_PROXIES);
    let proxies = TrustedProxy::list(settings.value(&TRUSTED_PROXIES))
        .map_err(|reason| invalid(TRUSTED_PROXIES.name, layer, reason))?;
    Ok((!proxies.is_empty()).then_some((proxies, layer)))
}

fn tls_files(
    certificate: Option<(String, SettingsLayer)>,
    key: Option<(String, SettingsLayer)>,
) -> Result<Option<TlsFiles>, SettingsError> {
    match (certificate, key) {
        (None, None) => Ok(None),
        (Some((certificate, certificate_layer)), Some((key, key_layer))) => Ok(Some(TlsFiles {
            certificate: PathBuf::from(certificate),
            certificate_layer,
            key: PathBuf::from(key),
            key_layer,
        })),
        (Some((_, layer)), None) => Err(invalid(
            TLS_CERT_FILE.name,
            layer,
            format!(
                "a certificate file serves TLS only with its key: set `{}` too, or unset this one",
                TLS_KEY_FILE.name
            ),
        )),
        (None, Some((_, layer))) => Err(invalid(
            TLS_KEY_FILE.name,
            layer,
            format!(
                "a key file serves TLS only with its certificate: set `{}` too, or unset this one",
                TLS_CERT_FILE.name
            ),
        )),
    }
}

fn file_setting(settings: &Resolved, setting: &Setting) -> Option<(String, SettingsLayer)> {
    settings
        .get::<Option<&str>>(setting)
        .map(|file| (file.to_owned(), setting_layer(settings, setting)))
}
