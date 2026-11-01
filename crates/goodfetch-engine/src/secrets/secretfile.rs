//! The encrypted local file as a Secret provider (spec #564), for a host
//! with no OS keychain: `secretfile:name` is the file `name.age` in Goodfetch
//! home's `secrets` folder, an age file, binary or armored, encrypted to an
//! X25519 identity of `secrets/identity.txt` (as `age-keygen -o` writes
//! it), a file only its owner may read: mode `0600` on Unix, an access list
//! naming only its owner, SYSTEM and Administrators on Windows. It protects
//! copies and backups of the home, not the value from another process of
//! the same user. A value ends at its last line ending, which `echo`
//! leaves.
//!
//! It is a translation point (ADR 0127): what age and the file system
//! answer becomes a [`ProviderFailure`] in [`translate`] and
//! [`translate_io`] and nowhere else, and their own text is never logged.
#![expect(
    clippy::disallowed_methods,
    reason = "the files are read on the tasks module's blocking policy, never on an async worker"
)]

#[cfg(windows)]
mod acl;

use std::fs::File;
use std::io::{self, BufReader, Read as _};
use std::path::{Path, PathBuf};
use std::str::FromStr as _;
use std::sync::Arc;

use age::armor::ArmoredReader;
use age::x25519;
use age::{DecryptError, Decryptor};
use goodfetch_protocol::secret_reference::SecretReference;
use secrecy::SecretString;
use secrecy::zeroize::Zeroizing;

use super::{ConfigProblem, PartyText, ProviderFailure, Resolving, SecretProvider};
use crate::durable;
use crate::tasks::TaskOwner;

/// The identity file's name in the folder.
pub(super) const IDENTITY: &str = "identity.txt";

/// The `secretfile:` provider over one folder, whose reads block under
/// `owner`.
pub(super) struct SecretFiles {
    folder: PathBuf,
    owner: Arc<TaskOwner>,
}

impl SecretFiles {
    pub(super) fn new(folder: PathBuf, owner: Arc<TaskOwner>) -> Self {
        Self { folder, owner }
    }
}

impl SecretProvider for SecretFiles {
    fn resolve<'a>(&'a self, reference: &'a SecretReference) -> Resolving<'a> {
        let folder = self.folder.clone();
        let name = reference.rest().to_owned();
        Box::pin(self.owner.blocking(move || read(&folder, &name)))
    }

    fn check_config(&self) -> Result<(), ConfigProblem> {
        Ok(())
    }
}

/// The value of the file `name` in `folder`.
fn read(folder: &Path, name: &str) -> Result<SecretString, ProviderFailure> {
    let identity_path = folder.join(IDENTITY);
    let file = folder.join(format!("{name}.age"));
    let identities = identities(&identity_path)?;
    let recipient = identities
        .first()
        .map(|identity| identity.to_public().to_string())
        .expect("invariant: `identities` gives at least one identity");
    let opened = File::open(&file).map_err(|error| match error.kind() {
        io::ErrorKind::NotFound => ProviderFailure::Missing,
        _ => translate_io(&error, &file),
    })?;
    let decryptor = Decryptor::new_buffered(ArmoredReader::new(BufReader::new(opened)))
        .map_err(|error| translate(error, &file, &identity_path, &recipient))?;
    let mut reader = decryptor
        .decrypt(
            identities
                .iter()
                .map(|identity| identity as &dyn age::Identity),
        )
        .map_err(|error| translate(error, &file, &identity_path, &recipient))?;
    let mut plaintext = Zeroizing::new(Vec::new());
    reader
        .read_to_end(&mut plaintext)
        .map_err(|error| translate(DecryptError::Io(error), &file, &identity_path, &recipient))?;
    let text = std::str::from_utf8(&plaintext).map_err(|_| ProviderFailure::Refused {
        fix: format!(
            "`{}` holds no UTF-8 text; encrypt the value as text.",
            file.display()
        ),
    })?;
    let text = text
        .strip_suffix("\r\n")
        .or_else(|| text.strip_suffix('\n'))
        .unwrap_or(text);
    Ok(SecretString::from(text.to_owned()))
}

/// Makes the file `name` in `folder` hold `value`, encrypted to the
/// folder's identity, unless the file exists: the identity too is made
/// where there is none, only its owner's. Each file appears whole or not
/// at all, and one that exists is never replaced, so two makers keep the
/// first one's. It blocks: run it under a blocking policy.
///
/// # Errors
///
/// As a read's: an identity others may read is `NotConfigured`, with the
/// `chmod` that closes it, and what the file system refuses is translated.
pub(super) fn make(folder: &Path, name: &str, value: &SecretString) -> Result<(), ProviderFailure> {
    use secrecy::ExposeSecret as _;
    use std::io::Write as _;

    let identity_path = folder.join(IDENTITY);
    durable::create_folder(folder).map_err(|error| translate_io(&error, folder))?;
    if !identity_path.exists() {
        let identity = x25519::Identity::generate();
        let text = Zeroizing::new(format!(
            "# public key: {}\n{}\n",
            identity.to_public(),
            identity.to_string().expose_secret()
        ));
        publish(folder, &identity_path, text.as_bytes())?;
    }
    let file = folder.join(format!("{name}.age"));
    if file.exists() {
        return Ok(());
    }
    let identities = identities(&identity_path)?;
    let recipient = identities
        .first()
        .map(x25519::Identity::to_public)
        .expect("invariant: `identities` gives at least one identity");
    let encryptor =
        age::Encryptor::with_recipients(std::iter::once(&recipient as &dyn age::Recipient))
            .expect("invariant: one X25519 recipient encrypts");
    let mut encrypted = Vec::new();
    let mut writer = encryptor
        .wrap_output(&mut encrypted)
        .map_err(|error| translate_io(&error, &file))?;
    writer
        .write_all(value.expose_secret().as_bytes())
        .and_then(|()| writer.finish().map(drop))
        .map_err(|error| translate_io(&error, &file))?;
    publish(folder, &file, &encrypted)
}

/// Writes `bytes` to `path` in `folder` through a temporary file only its
/// owner may read, synced, then linked where no file is, so a reader never
/// sees half of one; a file another maker put there first is kept. The
/// folder is synced after the link, so the file survives a power cut once
/// this answers (ADR 0002), whichever maker linked it.
fn publish(folder: &Path, path: &Path, bytes: &[u8]) -> Result<(), ProviderFailure> {
    use std::io::Write as _;

    let failed = |error: io::Error| translate_io(&error, path);
    // A temporary file is `0600` from its creation on Unix.
    let mut temporary = tempfile::NamedTempFile::new_in(folder).map_err(failed)?;
    temporary.write_all(bytes).map_err(failed)?;
    temporary.as_file().sync_all().map_err(failed)?;
    match temporary.persist_noclobber(path) {
        Ok(_) => {}
        Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(failed(error.error)),
    }
    durable::sync_folder(folder);
    Ok(())
}

/// The X25519 identities of the identity file at `path`, which only its
/// owner may read.
fn identities(path: &Path) -> Result<Vec<x25519::Identity>, ProviderFailure> {
    let not_configured = |fix: String| ProviderFailure::NotConfigured(ConfigProblem { fix });
    let create = || {
        format!(
            "Create the identity with `age-keygen -o {path}` and make it its owner's alone (`chmod 600 {path}`), then encrypt each value to its public key.",
            path = path.display()
        )
    };
    match owner_only(path) {
        Ok(true) => {}
        Ok(false) => return Err(not_configured(widened(path))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(not_configured(create()));
        }
        Err(error) => return Err(translate_io(&error, path)),
    }
    let mut text = Zeroizing::new(String::new());
    File::open(path)
        .and_then(|mut file| file.read_to_string(&mut text))
        .map_err(|error| translate_io(&error, path))?;
    let identities: Vec<x25519::Identity> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| x25519::Identity::from_str(line).ok())
        .collect();
    if identities.is_empty() {
        return Err(not_configured(format!(
            "`{}` holds no age X25519 identity (`AGE-SECRET-KEY-1…`). {}",
            path.display(),
            create()
        )));
    }
    Ok(identities)
}

/// The fix for an identity file others may read.
#[cfg(unix)]
fn widened(path: &Path) -> String {
    format!(
        "`{path}` may be read by other users; run `chmod 600 {path}`, then try again.",
        path = path.display()
    )
}

/// The fix for an identity file others may read.
#[cfg(windows)]
fn widened(path: &Path) -> String {
    format!(
        "`{path}` may be read by other users; run `icacls \"{path}\" /inheritance:r /grant:r \"%USERNAME%:F\"`, then try again.",
        path = path.display()
    )
}

/// Whether only the file's owner may read it: on Unix, no permission bit
/// for its group or others.
#[cfg(unix)]
fn owner_only(path: &Path) -> io::Result<bool> {
    use std::os::unix::fs::PermissionsExt as _;
    let shared = std::fs::metadata(path)?.permissions().mode() & 0o077;
    Ok(shared == 0)
}

/// Whether only the file's owner may read it: on Windows, its access list
/// grants only its owner, SYSTEM and Administrators.
#[cfg(windows)]
fn owner_only(path: &Path) -> io::Result<bool> {
    acl::owner_only(path)
}

/// What age's failure to decrypt `file` with the identity of `identity`,
/// whose public key is `recipient`, means for a resolution: the translation
/// table (ADR 0127). A failure it does not know is logged at `warn` by its
/// kind, never its text.
pub(super) fn translate(
    error: DecryptError,
    file: &Path,
    identity: &Path,
    recipient: &str,
) -> ProviderFailure {
    let again = || {
        format!(
            "encrypt the value again with `age -r {recipient} -o {}`",
            file.display()
        )
    };
    let refused = |fix: String| ProviderFailure::Refused { fix };
    match error {
        DecryptError::NoMatchingKeys => refused(format!(
            "`{}` is not encrypted to the identity of `{}`; {}.",
            file.display(),
            identity.display(),
            again()
        )),
        DecryptError::UnknownFormat => refused(format!(
            "`{}` is not an age file; {}.",
            file.display(),
            again()
        )),
        DecryptError::InvalidHeader
        | DecryptError::InvalidMac
        | DecryptError::DecryptionFailed
        | DecryptError::KeyDecryptionFailed => {
            refused(format!("`{}` is damaged; {}.", file.display(), again()))
        }
        DecryptError::ExcessiveWork { .. } => refused(format!(
            "`{}` is encrypted to a passphrase; {}.",
            file.display(),
            again()
        )),
        DecryptError::Io(error) if error.kind() == io::ErrorKind::UnexpectedEof => {
            refused(format!("`{}` is cut short; {}.", file.display(), again()))
        }
        DecryptError::Io(error) => translate_io(&error, file),
    }
}

/// What a file system failure reading `path` means for a resolution. A
/// failure it does not know is logged at `warn` by its kind, never its
/// text.
pub(super) fn translate_io(error: &io::Error, path: &Path) -> ProviderFailure {
    match error.kind() {
        io::ErrorKind::PermissionDenied => ProviderFailure::Refused {
            fix: format!(
                "This user may not read `{}`; give it back to this user, then try again.",
                path.display()
            ),
        },
        kind => {
            tracing::warn!(
                provider = "secretfile",
                failure = %kind,
                "an unrecognized failure reading an encrypted local file"
            );
            ProviderFailure::Unrecognized(PartyText::new(error.to_string()))
        }
    }
}
