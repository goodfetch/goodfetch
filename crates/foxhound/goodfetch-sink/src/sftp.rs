//! SFTP Files-mode Sink destination (ADRs 0008, 0026 and 0127; issue 1159):
//! writes each complete Export part to a temporary sibling file
//! (`.tmp-<name>-<nonce>`) and atomically renames it to its final key so a
//! reader never observes a half-written part.

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use russh::client::{self, Config, Handle, Handler};
use russh::keys::PublicKeyOrCertificate;
use russh_sftp::client::SftpSession;
use russh_sftp::protocol::StatusCode;
use url::Url;

use crate::contract::{Failure, FailureKind, RunCopy};
use crate::files::{FileDestination, FileProbe, PutOutcome, probe_from_keys};

const SFTP_TIMEOUT: Duration = Duration::from_secs(30);

/// Errors returned by [`SftpDestination`] before translation.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SftpError {
    /// Invalid SFTP destination URL or missing credentials.
    #[error("invalid SFTP Sink configuration: {0}")]
    Config(String),
    /// Authentication or host-key verification failed.
    #[error("SFTP authentication failed: {0}")]
    Auth(String),
    /// SSH transport error from `russh`.
    #[error("SSH error: {0}")]
    Ssh(#[from] russh::Error),
    /// SFTP protocol error from `russh-sftp`.
    #[error("SFTP error: {0}")]
    Sftp(#[from] russh_sftp::client::error::Error),
    /// No active Tokio runtime available when called synchronously.
    #[error("SFTP Sink requires an active Tokio runtime")]
    NoRuntime,
}

struct SftpHandler {
    accept_unknown_host: bool,
}

impl Handler for SftpHandler {
    type Error = russh::Error;

    fn check_server_key(
        &mut self,
        _server_public_key: &PublicKeyOrCertificate,
    ) -> impl Future<Output = Result<bool, Self::Error>> + Send {
        std::future::ready(Ok(self.accept_unknown_host))
    }
}

/// Files-mode [`FileDestination`] writing Export parts over SFTP.
#[derive(Clone, Debug)]
pub struct SftpDestination {
    sink_name: String,
    host: String,
    port: u16,
    username: String,
    password: String,
    base_dir: String,
    prefix: String,
    accept_unknown_host: bool,
    next_tmp: u64,
}

impl SftpDestination {
    /// Parses an SFTP destination URL:
    /// `sftp://user:pass@host[:port][/base_dir][?accept_unknown_host=true]`.
    ///
    /// # Errors
    ///
    /// [`SftpError::Config`] if the URL, scheme, host or username is invalid.
    pub fn from_url(
        sink_name: impl Into<String>,
        destination: &str,
        target_prefix: Option<&str>,
    ) -> Result<Self, SftpError> {
        let url = Url::parse(destination)
            .map_err(|err| SftpError::Config(format!("invalid URL: {err}")))?;
        if url.scheme() != "sftp" {
            return Err(SftpError::Config(format!(
                "unsupported scheme `{}` (expected `sftp`)",
                url.scheme()
            )));
        }
        let host = url
            .host_str()
            .filter(|h| !h.is_empty())
            .ok_or_else(|| SftpError::Config("missing SFTP host".into()))?
            .to_owned();
        let port = url.port().unwrap_or(22);
        let username = url.username().to_owned();
        if username.is_empty() {
            return Err(SftpError::Config("missing SFTP username".into()));
        }
        let password = url.password().unwrap_or_default().to_owned();
        let base_dir = url.path().trim_matches('/').to_owned();
        let params: BTreeMap<String, String> = url
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        let accept_unknown_host = params
            .get("accept_unknown_host")
            .is_some_and(|v| v == "true" || v == "1");
        let prefix = target_prefix
            .unwrap_or_default()
            .trim_matches('/')
            .to_owned();
        Ok(Self {
            sink_name: sink_name.into(),
            host,
            port,
            username,
            password,
            base_dir,
            prefix,
            accept_unknown_host,
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

    async fn connect(&self) -> Result<(Handle<SftpHandler>, SftpSession), SftpError> {
        let config = Arc::new(Config {
            inactivity_timeout: Some(SFTP_TIMEOUT),
            ..Config::default()
        });
        let handler = SftpHandler {
            accept_unknown_host: self.accept_unknown_host,
        };
        let mut handle = client::connect(config, (self.host.as_str(), self.port), handler).await?;
        let auth = handle
            .authenticate_password(self.username.as_str(), self.password.as_str())
            .await?;
        if !auth.success() {
            return Err(SftpError::Auth("password authentication rejected".into()));
        }
        let channel = handle.channel_open_session().await?;
        channel.request_subsystem(true, "sftp").await?;
        let session = SftpSession::new(channel.into_stream()).await?;
        Ok((handle, session))
    }

    async fn probe_async(&self, run: &RunCopy) -> Result<FileProbe, SftpError> {
        let (_handle, session) = self.connect().await?;
        let rel_root = if self.prefix.is_empty() {
            format!("{}/{}", run.run_id, self.sink_name)
        } else {
            format!("{}/{}/{}", self.prefix, run.run_id, self.sink_name)
        };
        let remote_root = self.remote_path(&rel_root);
        let mut files = Vec::new();
        collect_sftp_files(&session, &remote_root, &rel_root, &mut files).await?;
        let _ = session.close().await;
        Ok(probe_from_keys(&self.prefix, run, &self.sink_name, files))
    }

    async fn put_part_async(&mut self, key: &str, bytes: &[u8]) -> Result<PutOutcome, SftpError> {
        let (_handle, session) = self.connect().await?;
        let final_path = self.remote_path(key);
        let (parent, filename) = split_parent_file(&final_path);
        if !parent.is_empty() {
            ensure_sftp_dirs(&session, parent).await?;
        }
        let nonce = self.next_tmp;
        self.next_tmp = self.next_tmp.wrapping_add(1);
        let tmp_path = if parent.is_empty() {
            format!(".tmp-{filename}-{nonce}")
        } else {
            format!("{parent}/.tmp-{filename}-{nonce}")
        };
        session.write(&tmp_path, bytes).await?;
        let renamed = match session.posix_rename(&tmp_path, &final_path).await {
            Ok(true) => Ok(()),
            Ok(false) => session.rename(&tmp_path, &final_path).await,
            Err(err) => Err(err),
        };
        if let Err(err) = renamed {
            let _ = session.remove_file(&tmp_path).await;
            let _ = session.close().await;
            return Err(SftpError::Sftp(err));
        }
        let _ = session.close().await;
        Ok(PutOutcome::Written)
    }
}

fn split_parent_file(path: &str) -> (&str, &str) {
    path.rsplit_once('/').map_or(("", path), |(p, f)| (p, f))
}

async fn ensure_sftp_dirs(session: &SftpSession, dir: &str) -> Result<(), SftpError> {
    let mut current = String::new();
    for segment in dir.split('/').filter(|s| !s.is_empty()) {
        if !current.is_empty() {
            current.push('/');
        }
        current.push_str(segment);
        match session.create_dir(&current).await {
            Ok(()) => {}
            Err(russh_sftp::client::error::Error::Status(status))
                if matches!(
                    status.status_code,
                    StatusCode::Failure | StatusCode::PermissionDenied
                ) =>
            {
                if !session.try_exists(&current).await.unwrap_or(false) {
                    return Err(SftpError::Sftp(russh_sftp::client::error::Error::Status(
                        status,
                    )));
                }
            }
            Err(err) => return Err(SftpError::Sftp(err)),
        }
    }
    Ok(())
}

async fn collect_sftp_files(
    session: &SftpSession,
    remote_root: &str,
    rel_root: &str,
    out: &mut Vec<String>,
) -> Result<(), SftpError> {
    let mut stack = vec![(remote_root.to_owned(), rel_root.to_owned())];
    while let Some((remote_dir, rel_dir)) = stack.pop() {
        let entries = match session.read_dir(&remote_dir).await {
            Ok(entries) => entries,
            Err(russh_sftp::client::error::Error::Status(status))
                if status.status_code == StatusCode::NoSuchFile =>
            {
                continue;
            }
            Err(err) => return Err(SftpError::Sftp(err)),
        };
        for entry in entries {
            let name = entry.file_name();
            if name == "." || name == ".." || name.starts_with(".tmp-") {
                continue;
            }
            let child_remote = format!("{remote_dir}/{name}");
            let child_rel = format!("{rel_dir}/{name}");
            if entry.file_type().is_dir() {
                stack.push((child_remote, child_rel));
            } else {
                out.push(child_rel);
            }
        }
    }
    Ok(())
}

impl FileDestination for SftpDestination {
    type Error = SftpError;

    fn probe(&mut self, run: &RunCopy) -> Result<FileProbe, Self::Error> {
        let handle = tokio::runtime::Handle::try_current().map_err(|_| SftpError::NoRuntime)?;
        handle.block_on(self.probe_async(run))
    }

    fn put_part(
        &mut self,
        key: &str,
        bytes: Vec<u8>,
        _run: &RunCopy,
    ) -> Result<PutOutcome, Self::Error> {
        let handle = tokio::runtime::Handle::try_current().map_err(|_| SftpError::NoRuntime)?;
        handle.block_on(self.put_part_async(key, &bytes))
    }

    fn map_failure(&self, error: Self::Error) -> Failure {
        map_sftp_error(error)
    }
}

/// Translates [`SftpError`] into the driver's [`Failure`] (ADR 0127).
pub fn map_sftp_error(error: SftpError) -> Failure {
    match error {
        SftpError::Config(msg) => Failure::new(FailureKind::Permanent, msg),
        SftpError::NoRuntime => Failure::new(
            FailureKind::Permanent,
            "SFTP Sink requires an active Tokio runtime",
        ),
        SftpError::Auth(msg) => Failure::new(FailureKind::Auth, msg),
        SftpError::Ssh(err) => {
            let text = err.to_string();
            match err {
                russh::Error::UnknownKey | russh::Error::NotAuthenticated => {
                    Failure::new(FailureKind::Auth, text)
                }
                russh::Error::Disconnect
                | russh::Error::IO(_)
                | russh::Error::InactivityTimeout
                | russh::Error::KeepaliveTimeout => Failure::new(FailureKind::Transient, text),
                _ => {
                    tracing::warn!(error = %text, "unrecognized SFTP SSH failure; treating as transient");
                    Failure::new(FailureKind::Unrecognized, text)
                }
            }
        }
        SftpError::Sftp(err) => {
            let text = err.to_string();
            match err {
                russh_sftp::client::error::Error::Status(status) => match status.status_code {
                    StatusCode::PermissionDenied => Failure::new(FailureKind::Auth, text),
                    StatusCode::NoSuchFile | StatusCode::BadMessage | StatusCode::OpUnsupported => {
                        Failure::new(FailureKind::Permanent, text)
                    }
                    StatusCode::ConnectionLost | StatusCode::NoConnection => {
                        Failure::new(FailureKind::Transient, text)
                    }
                    _ => {
                        tracing::warn!(error = %text, "unrecognized SFTP status; treating as transient");
                        Failure::new(FailureKind::Unrecognized, text)
                    }
                },
                russh_sftp::client::error::Error::IO(_)
                | russh_sftp::client::error::Error::Timeout => {
                    Failure::new(FailureKind::Transient, text)
                }
                _ => {
                    tracing::warn!(error = %text, "unrecognized SFTP failure; treating as transient");
                    Failure::new(FailureKind::Unrecognized, text)
                }
            }
        }
    }
}
