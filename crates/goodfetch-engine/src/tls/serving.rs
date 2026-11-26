//! The certificate an Exposed Daemon serves TLS with (ADR 0049): the
//! files `daemon.tls.cert_file` and `daemon.tls.key_file` name, read once
//! before it serves and again at the first Connection after either is
//! replaced, so a renewed certificate is served with no restart. A
//! replacement that cannot be used leaves the last good one served.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, PoisonError};
use std::time::SystemTime;

use rustls::ServerConfig;
use tokio_rustls::TlsAcceptor;

use super::{read_certificates, read_private_key};
use crate::sync::{Mutex, MutexGuard};
use crate::tasks::TaskOwner;

/// The certificate and key files a listener serves TLS with, and what it
/// read in them last.
pub struct ServingCertificate {
    files: Arc<ServingFiles>,
    current: Mutex<Current>,
}

impl std::fmt::Debug for ServingCertificate {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ServingCertificate")
            .field("certificate", &self.files.certificate)
            .field("key", &self.files.key)
            .finish_non_exhaustive()
    }
}

/// Why a serving certificate's files cannot be served.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum ServingCertificateError {
    /// The certificate file cannot be read, or holds no certificate.
    #[error("the certificate file `{}` cannot be used: {reason}", file.display())]
    Certificate {
        /// The file, as its setting names it.
        file: PathBuf,
        /// What is wrong with it.
        reason: String,
    },
    /// The key file cannot be read, holds no private key, or holds one that
    /// is not the certificate's.
    #[error("the key file `{}` cannot be used: {reason}", file.display())]
    Key {
        /// The file, as its setting names it.
        file: PathBuf,
        /// What is wrong with it; it never holds the key.
        reason: String,
    },
}

/// The two files, as their settings name them.
#[derive(Debug)]
struct ServingFiles {
    certificate: PathBuf,
    key: PathBuf,
}

/// What a listener serves with now, and the files' state it was read in.
struct Current {
    stamp: Stamp,
    acceptor: TlsAcceptor,
}

/// The state of both files, which changes when either is replaced or
/// written to.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Stamp {
    certificate: FileStamp,
    key: FileStamp,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FileStamp {
    modified: Option<SystemTime>,
    length: u64,
    /// Its inode, which a file renamed over it changes, as a Kubernetes
    /// Secret's volume or `certbot` replaces it.
    #[cfg(unix)]
    inode: u64,
}

impl FileStamp {
    fn of(metadata: &std::fs::Metadata) -> Self {
        Self {
            modified: metadata.modified().ok(),
            length: metadata.len(),
            #[cfg(unix)]
            inode: std::os::unix::fs::MetadataExt::ino(metadata),
        }
    }
}

impl ServingCertificate {
    /// The certificate of `certificate` with the private key of `key`, read
    /// now: before the listener serves, outside any runtime's worker.
    ///
    /// # Errors
    ///
    /// [`ServingCertificateError`] when either file cannot be read or used,
    /// or the key is not the certificate's.
    pub fn open(certificate: PathBuf, key: PathBuf) -> Result<Self, ServingCertificateError> {
        let files = Arc::new(ServingFiles { certificate, key });
        let current = files.read()?;
        Ok(Self {
            files,
            current: Mutex::new(current),
        })
    }

    /// What accepts a Connection's TLS handshake now: what the files hold,
    /// read again on `owner`'s blocking policy when either changed since
    /// the last read, or the last good one when they hold nothing usable.
    pub async fn acceptor(&self, owner: &TaskOwner) -> TlsAcceptor {
        let files = Arc::clone(&self.files);
        let stamp = match owner.blocking(move || files.stamp()).await {
            Ok(stamp) => stamp,
            Err(error) => {
                tracing::warn!(%error, "the serving certificate's files cannot be read: the last one read is served");
                return self.lock().acceptor.clone();
            }
        };
        {
            let current = self.lock();
            if current.stamp == stamp {
                return current.acceptor.clone();
            }
        }
        let files = Arc::clone(&self.files);
        let read = owner.blocking(move || files.read()).await;
        let mut current = self.lock();
        match read {
            Ok(read) => {
                tracing::info!(
                    certificate = %self.files.certificate.display(),
                    "the replaced serving certificate is served"
                );
                *current = read;
            }
            Err(error) => {
                tracing::warn!(%error, "the replaced serving certificate cannot be used: the last one read is served");
                // Read again once either file changes again, not at every
                // Connection.
                current.stamp = stamp;
            }
        }
        current.acceptor.clone()
    }

    /// What it serves with now; it is read or replaced in one step, never
    /// held across an `.await`.
    fn lock(&self) -> MutexGuard<'_, Current> {
        self.current.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl ServingFiles {
    /// Both files' state now.
    fn stamp(&self) -> io::Result<Stamp> {
        let metadata = |file: &Path| {
            #[expect(
                clippy::disallowed_methods,
                reason = "it runs before the listener serves or on the tasks module's blocking policy"
            )]
            std::fs::metadata(file)
        };
        Ok(Stamp {
            certificate: FileStamp::of(&metadata(&self.certificate)?),
            key: FileStamp::of(&metadata(&self.key)?),
        })
    }

    /// What both files hold, and their state.
    fn read(&self) -> Result<Current, ServingCertificateError> {
        let certificate_error = |reason: String| ServingCertificateError::Certificate {
            file: self.certificate.clone(),
            reason,
        };
        let key_error = |reason: String| ServingCertificateError::Key {
            file: self.key.clone(),
            reason,
        };
        // Taken first, so a replacement during the reads is read again; when
        // a file is missing, reading each below tells which.
        let stamp = self.stamp();
        let read = |file: &Path| {
            #[expect(
                clippy::disallowed_methods,
                reason = "it runs before the listener serves or on the tasks module's blocking policy"
            )]
            std::fs::read(file)
        };
        let chain = read(&self.certificate)
            .map_err(|error| certificate_error(error.to_string()))
            .and_then(|file| {
                read_certificates(&file).map_err(|error| certificate_error(error.to_string()))
            })?;
        let key = read(&self.key)
            .map_err(|error| key_error(error.to_string()))
            .and_then(|file| {
                read_private_key(&file).map_err(|error| key_error(error.to_string()))
            })?;
        let config =
            ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
                .with_safe_default_protocol_versions()
                .map_err(|error| certificate_error(error.to_string()))?
                .with_no_client_auth()
                .with_single_cert(chain, key)
                .map_err(|_| key_error("it holds no key of the certificate".to_owned()))?;
        Ok(Current {
            stamp: stamp.map_err(|error| certificate_error(error.to_string()))?,
            acceptor: TlsAcceptor::from(Arc::new(config)),
        })
    }
}

#[cfg(test)]
mod tests;
