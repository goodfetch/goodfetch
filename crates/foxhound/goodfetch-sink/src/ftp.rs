//! FTP/FTPS Files-mode Sink destination (ADRs 0008, 0026 and 0127; issue 1159):
//! uploads each complete Export part to a temporary sibling file
//! (`.tmp-<name>-<nonce>`) and atomically renames it (`RNFR`/`RNTO`) to its
//! final key so a reader never observes a half-written part.

use std::fmt;
use std::io::Cursor;
use std::sync::Arc;

use secrecy::{ExposeSecret as _, SecretString};
use suppaftp::types::FileType;
use suppaftp::{FtpError, FtpStream, Mode, RustlsConnector, RustlsFtpStream};
use url::Url;

use crate::contract::{Failure, FailureKind, RunCopy};
use crate::files::{FileDestination, FileProbe, PutOutcome, probe_from_keys};

/// Errors returned by [`FtpDestination`] before translation.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum FtpSinkError {
    /// Invalid FTP destination URL.
    #[error("invalid FTP Sink configuration: {0}")]
    Config(String),
    /// Protocol or transport error from `suppaftp`.
    #[error("FTP error: {0}")]
    Ftp(#[from] FtpError),
}

enum FtpConn {
    Plain(FtpStream),
    Tls(Box<RustlsFtpStream>),
}

impl FtpConn {
    fn mkdir(&mut self, path: &str) {
        match self {
            Self::Plain(s) => {
                let _ = s.mkdir(path);
            }
            Self::Tls(s) => {
                let _ = s.mkdir(path);
            }
        }
    }

    fn nlst(&mut self, path: Option<&str>) -> Result<Vec<String>, FtpError> {
        match self {
            Self::Plain(s) => s.nlst(path),
            Self::Tls(s) => s.nlst(path),
        }
    }

    fn put_file(&mut self, path: &str, bytes: &[u8]) -> Result<u64, FtpError> {
        let mut cursor = Cursor::new(bytes);
        match self {
            Self::Plain(s) => s.put_file(path, &mut cursor),
            Self::Tls(s) => s.put_file(path, &mut cursor),
        }
    }

    fn rename(&mut self, src: &str, dst: &str) -> Result<(), FtpError> {
        match self {
            Self::Plain(s) => s.rename(src, dst),
            Self::Tls(s) => s.rename(src, dst),
        }
    }

    fn rm(&mut self, path: &str) {
        match self {
            Self::Plain(s) => {
                let _ = s.rm(path);
            }
            Self::Tls(s) => {
                let _ = s.rm(path);
            }
        }
    }

    fn quit(&mut self) {
        match self {
            Self::Plain(s) => {
                let _ = s.quit();
            }
            Self::Tls(s) => {
                let _ = s.quit();
            }
        }
    }
}

/// Files-mode [`FileDestination`] writing Export parts over FTP or FTPS.
#[derive(Clone)]
pub struct FtpDestination {
    sink_name: String,
    host: String,
    port: u16,
    secure: bool,
    username: String,
    password: SecretString,
    base_dir: String,
    prefix: String,
    next_tmp: u64,
}

impl fmt::Debug for FtpDestination {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FtpDestination")
            .field("sink_name", &self.sink_name)
            .field("host", &self.host)
            .field("port", &self.port)
            .field("secure", &self.secure)
            .field("username", &self.username)
            .field("password", &"<redacted>")
            .field("base_dir", &self.base_dir)
            .field("prefix", &self.prefix)
            .finish_non_exhaustive()
    }
}

impl FtpDestination {
    /// Parses an FTP/FTPS destination URL:
    /// `ftp://[user:pass@]host[:port][/base_dir]` or `ftps://...` (explicit TLS).
    ///
    /// # Errors
    ///
    /// [`FtpSinkError::Config`] if the URL, scheme or host is invalid.
    pub fn from_url(
        sink_name: impl Into<String>,
        destination: &str,
        target_prefix: Option<&str>,
    ) -> Result<Self, FtpSinkError> {
        let url = Url::parse(destination)
            .map_err(|err| FtpSinkError::Config(format!("invalid URL: {err}")))?;
        let secure = match url.scheme() {
            "ftp" => false,
            "ftps" => true,
            other => {
                return Err(FtpSinkError::Config(format!(
                    "unsupported scheme `{other}` (expected `ftp` or `ftps`)"
                )));
            }
        };
        let host = url
            .host_str()
            .filter(|h| !h.is_empty())
            .ok_or_else(|| FtpSinkError::Config("missing FTP host".into()))?
            .to_owned();
        let port = url.port().unwrap_or(21);
        let username = if url.username().is_empty() {
            "anonymous".to_owned()
        } else {
            crate::contract::percent_decode(url.username())
        };
        let password = SecretString::from(
            url.password()
                .map_or_else(|| "anonymous@".to_owned(), crate::contract::percent_decode),
        );
        let base_dir = url.path().trim_matches('/').to_owned();
        let prefix = target_prefix
            .unwrap_or_default()
            .trim_matches('/')
            .to_owned();
        Ok(Self {
            sink_name: sink_name.into(),
            host,
            port,
            secure,
            username,
            password,
            base_dir,
            prefix,
            next_tmp: 1,
        })
    }

    fn remote_path(&self, key: &str) -> String {
        let trimmed = key.trim_start_matches('/');
        if self.base_dir.is_empty() {
            trimmed.to_owned()
        } else {
            format!("{}/{trimmed}", self.base_dir)
        }
    }

    fn connect(&self) -> Result<FtpConn, FtpSinkError> {
        if self.secure {
            let tls_config = suppaftp::rustls::ClientConfig::builder_with_provider(Arc::new(
                suppaftp::rustls::crypto::ring::default_provider(),
            ))
            .with_safe_default_protocol_versions()
            .map_err(|err| FtpSinkError::Config(format!("TLS config error: {err}")))?
            .with_root_certificates(suppaftp::rustls::RootCertStore::from_iter(
                webpki_roots::TLS_SERVER_ROOTS.iter().cloned(),
            ))
            .with_no_client_auth();
            let stream = RustlsFtpStream::connect((self.host.as_str(), self.port))?;
            let mut secure =
                stream.into_secure(RustlsConnector::from(Arc::new(tls_config)), &self.host)?;
            secure.set_mode(Mode::Passive);
            secure.login(self.username.as_str(), self.password.expose_secret())?;
            secure.transfer_type(FileType::Binary)?;
            Ok(FtpConn::Tls(Box::new(secure)))
        } else {
            let mut ftp = FtpStream::connect((self.host.as_str(), self.port))?;
            ftp.set_mode(Mode::Passive);
            ftp.login(self.username.as_str(), self.password.expose_secret())?;
            ftp.transfer_type(FileType::Binary)?;
            Ok(FtpConn::Plain(ftp))
        }
    }
}

fn ensure_ftp_dirs(ftp: &mut FtpConn, dir: &str) {
    let mut current = String::new();
    for segment in dir.split('/').filter(|s| !s.is_empty()) {
        if !current.is_empty() {
            current.push('/');
        }
        current.push_str(segment);
        ftp.mkdir(&current);
    }
}

fn list_ftp_parts(
    ftp: &mut FtpConn,
    remote_root: &str,
    rel_root: &str,
    run_epoch: u64,
    out: &mut Vec<String>,
) {
    let Ok(datasets) = ftp.nlst(Some(remote_root)) else {
        return;
    };
    let epoch_dir = format!("epoch-{run_epoch}");
    for dataset_entry in datasets {
        let dataset = dataset_entry
            .rsplit('/')
            .next()
            .unwrap_or(&dataset_entry)
            .trim();
        if dataset.is_empty() || dataset == "." || dataset == ".." {
            continue;
        }
        let remote_epoch = format!("{remote_root}/{dataset}/{epoch_dir}");
        let Ok(files) = ftp.nlst(Some(&remote_epoch)) else {
            continue;
        };
        for file_entry in files {
            let filename = file_entry.rsplit('/').next().unwrap_or(&file_entry).trim();
            if filename.is_empty() || filename.starts_with(".tmp-") {
                continue;
            }
            out.push(format!("{rel_root}/{dataset}/{epoch_dir}/{filename}"));
        }
    }
}

impl FileDestination for FtpDestination {
    type Error = FtpSinkError;

    fn probe(&mut self, run: &RunCopy) -> Result<FileProbe, Self::Error> {
        let mut ftp = self.connect()?;
        let rel_root = if self.prefix.is_empty() {
            format!("{}/{}", run.run_id, self.sink_name)
        } else {
            format!("{}/{}/{}", self.prefix, run.run_id, self.sink_name)
        };
        let remote_root = self.remote_path(&rel_root);
        let mut files = Vec::new();
        list_ftp_parts(&mut ftp, &remote_root, &rel_root, run.run_epoch, &mut files);
        ftp.quit();
        Ok(probe_from_keys(&self.prefix, run, &self.sink_name, files))
    }

    fn put_part(
        &mut self,
        key: &str,
        bytes: Vec<u8>,
        _run: &RunCopy,
    ) -> Result<PutOutcome, Self::Error> {
        let mut ftp = self.connect()?;
        let final_path = self.remote_path(key);
        let (parent, filename) = final_path
            .rsplit_once('/')
            .map_or(("", final_path.as_str()), |(p, f)| (p, f));
        if !parent.is_empty() {
            ensure_ftp_dirs(&mut ftp, parent);
        }
        let nonce = self.next_tmp;
        self.next_tmp = self.next_tmp.wrapping_add(1);
        let tmp_path = if parent.is_empty() {
            format!(".tmp-{filename}-{nonce}")
        } else {
            format!("{parent}/.tmp-{filename}-{nonce}")
        };
        ftp.put_file(&tmp_path, &bytes)?;
        if let Err(err) = ftp.rename(&tmp_path, &final_path) {
            ftp.rm(&tmp_path);
            ftp.quit();
            return Err(FtpSinkError::Ftp(err));
        }
        ftp.quit();
        Ok(PutOutcome::Written)
    }

    fn map_failure(&self, error: Self::Error) -> Failure {
        map_ftp_error(error)
    }
}

/// Translates [`FtpSinkError`] into the driver's [`Failure`] (ADR 0127).
pub fn map_ftp_error(error: FtpSinkError) -> Failure {
    match error {
        FtpSinkError::Config(msg) => Failure::new(FailureKind::Permanent, msg),
        FtpSinkError::Ftp(err) => {
            let text = err.to_string();
            match err {
                FtpError::ConnectionError(_) => Failure::new(FailureKind::Transient, text),
                FtpError::UnexpectedResponse(resp) => match resp.status.code() {
                    530 | 532 | 534 => Failure::new(FailureKind::Auth, text),
                    400..=499 => Failure::new(FailureKind::Transient, text),
                    500..=599 => Failure::new(FailureKind::Permanent, text),
                    _ => {
                        tracing::warn!(error = %text, "unrecognized FTP response; treating as transient");
                        Failure::new(FailureKind::Unrecognized, text)
                    }
                },
                _ => {
                    tracing::warn!(error = %text, "unrecognized FTP failure; treating as transient");
                    Failure::new(FailureKind::Unrecognized, text)
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;
    use suppaftp::Status;
    use suppaftp::types::Response;

    use super::*;

    fn reply_err(status: Status) -> FtpSinkError {
        FtpSinkError::Ftp(FtpError::UnexpectedResponse(Response::new(
            status,
            b"reply".to_vec(),
        )))
    }

    #[rstest]
    #[case::config(FtpSinkError::Config("bad url".into()), FailureKind::Permanent)]
    #[case::connection(
        FtpSinkError::Ftp(FtpError::ConnectionError(std::io::Error::other("reset"))),
        FailureKind::Transient
    )]
    #[case::auth_530(reply_err(Status::NotLoggedIn), FailureKind::Auth)]
    #[case::transient_421(reply_err(Status::NotAvailable), FailureKind::Transient)]
    #[case::permanent_550(reply_err(Status::FileUnavailable), FailureKind::Permanent)]
    #[case::unrecognized_reply(reply_err(Status::CommandOk), FailureKind::Unrecognized)]
    #[case::unrecognized_bad_response(
        FtpSinkError::Ftp(FtpError::BadResponse),
        FailureKind::Unrecognized
    )]
    fn map_ftp_error_classifies_known_and_unrecognized(
        #[case] err: FtpSinkError,
        #[case] expected: FailureKind,
    ) {
        assert_eq!(map_ftp_error(err).kind(), expected);
    }

    #[test]
    fn debug_redacts_ftp_password_and_accepts_ftps_scheme() {
        let dest = FtpDestination::from_url(
            "f",
            "ftps://alice:super-secret-ftp-pass@example.test:21/exports",
            None,
        )
        .unwrap();
        assert!(dest.secure);
        let rendered = format!("{dest:?}");
        assert!(!rendered.contains("super-secret-ftp-pass"));
        assert!(rendered.contains("<redacted>"));
    }
}
