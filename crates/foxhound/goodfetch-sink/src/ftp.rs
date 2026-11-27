//! FTP/FTPS Files-mode Sink destination (ADRs 0008, 0026 and 0127; issue 1159):
//! uploads each complete Export part to a temporary sibling file
//! (`.tmp-<name>-<nonce>`) and atomically renames it (`RNFR`/`RNTO`) to its
//! final key so a reader never observes a half-written part.

use std::io::Cursor;

use suppaftp::types::FileType;
use suppaftp::{FtpError, FtpStream, Mode};
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

/// Files-mode [`FileDestination`] writing Export parts over FTP.
#[derive(Clone, Debug)]
pub struct FtpDestination {
    sink_name: String,
    host: String,
    port: u16,
    username: String,
    password: String,
    base_dir: String,
    prefix: String,
    next_tmp: u64,
}

impl FtpDestination {
    /// Parses an FTP destination URL:
    /// `ftp://[user:pass@]host[:port][/base_dir]`.
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
        if url.scheme() != "ftp" {
            return Err(FtpSinkError::Config(format!(
                "unsupported scheme `{}` (expected `ftp`)",
                url.scheme()
            )));
        }
        let host = url
            .host_str()
            .filter(|h| !h.is_empty())
            .ok_or_else(|| FtpSinkError::Config("missing FTP host".into()))?
            .to_owned();
        let port = url.port().unwrap_or(21);
        let username = if url.username().is_empty() {
            "anonymous".to_owned()
        } else {
            url.username().to_owned()
        };
        let password = url.password().unwrap_or("anonymous@").to_owned();
        let base_dir = url.path().trim_matches('/').to_owned();
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

    fn connect(&self) -> Result<FtpStream, FtpSinkError> {
        let mut ftp = FtpStream::connect((self.host.as_str(), self.port))?;
        ftp.set_mode(Mode::Passive);
        ftp.login(&self.username, &self.password)?;
        ftp.transfer_type(FileType::Binary)?;
        Ok(ftp)
    }
}

fn ensure_ftp_dirs(ftp: &mut FtpStream, dir: &str) {
    let mut current = String::new();
    for segment in dir.split('/').filter(|s| !s.is_empty()) {
        if !current.is_empty() {
            current.push('/');
        }
        current.push_str(segment);
        // FTP servers return 550 if the directory already exists; ignore here
        // and let the subsequent upload surface any real permission error.
        let _ = ftp.mkdir(&current);
    }
}

fn list_ftp_parts(
    ftp: &mut FtpStream,
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
        let _ = ftp.quit();
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
        let mut cursor = Cursor::new(bytes);
        ftp.put_file(&tmp_path, &mut cursor)?;
        if let Err(err) = ftp.rename(&tmp_path, &final_path) {
            let _ = ftp.rm(&tmp_path);
            let _ = ftp.quit();
            return Err(FtpSinkError::Ftp(err));
        }
        let _ = ftp.quit();
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
