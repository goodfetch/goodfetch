use std::path::{Path, PathBuf};
use std::sync::Arc;

use rustls::pki_types::{CertificateDer, ServerName};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio_rustls::TlsConnector;

use super::{ServingCertificate, ServingCertificateError};
use crate::tasks::{BlockingPolicy, TaskOwner};

/// A self-signed certificate for `localhost`, and its key, both DER, and
/// the certificate as a client trusts it.
fn certificate() -> (Vec<u8>, Vec<u8>, CertificateDer<'static>) {
    let key = rcgen::KeyPair::generate().unwrap();
    let certificate = rcgen::CertificateParams::new(vec!["localhost".to_owned()])
        .unwrap()
        .self_signed(&key)
        .unwrap();
    (
        certificate.der().to_vec(),
        key.serialize_der(),
        certificate.der().clone(),
    )
}

/// `certificate` and `key` written in `folder`, as `cert.der` and `key.der`.
fn written(folder: &Path, certificate: &[u8], key: &[u8]) -> (PathBuf, PathBuf) {
    let files = (folder.join("cert.der"), folder.join("key.der"));
    replace(&files.0, certificate);
    replace(&files.1, key);
    files
}

/// `file` replaced by one holding `contents`, renamed over it as a
/// certificate's renewal replaces it.
#[expect(
    clippy::disallowed_methods,
    reason = "a test replaces its fixture files between its Connections, which wait for nothing else"
)]
fn replace(file: &Path, contents: &[u8]) {
    let staged = file.with_extension("new");
    std::fs::write(&staged, contents).unwrap();
    std::fs::rename(&staged, file).unwrap();
}

/// The certificate a client trusting only `trusted` is served by
/// `serving`'s acceptor now, if its handshake completes.
async fn served(
    serving: &ServingCertificate,
    owner: &TaskOwner,
    trusted: &CertificateDer<'static>,
) -> Option<CertificateDer<'static>> {
    let mut roots = rustls::RootCertStore::empty();
    roots.add(trusted.clone()).unwrap();
    let client = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_root_certificates(roots)
    .with_no_client_auth();
    let (client_side, server_side) = tokio::io::duplex(64 * 1024);
    let acceptor = serving.acceptor(owner).await;
    let server = async {
        let mut stream = acceptor.accept(server_side).await.ok()?;
        stream.write_all(b"served").await.ok()?;
        stream.shutdown().await.ok()
    };
    let client = async {
        let mut stream = TlsConnector::from(Arc::new(client))
            .connect(ServerName::try_from("localhost").unwrap(), client_side)
            .await
            .ok()?;
        let mut read = Vec::new();
        stream.read_to_end(&mut read).await.ok()?;
        assert_eq!(read, b"served");
        stream
            .get_ref()
            .1
            .peer_certificates()
            .and_then(|chain| chain.first().cloned())
    };
    let (_, presented) = tokio::join!(server, client);
    presented
}

#[test]
fn a_certificate_and_its_key_are_served() {
    let folder = tempfile::tempdir().unwrap();
    let (certificate, key, der) = certificate();
    let (certificate_file, key_file) = written(folder.path(), &certificate, &key);
    let owner = TaskOwner::root(BlockingPolicy::Inline);

    let serving = ServingCertificate::open(certificate_file, key_file).unwrap();

    let presented = goodfetch_test_support::runtime().block_on(served(&serving, &owner, &der));
    assert_eq!(presented, Some(der));
}

#[test]
fn files_that_cannot_be_served_are_refused_naming_the_one_at_fault() {
    let folder = tempfile::tempdir().unwrap();
    let (certificate, key, _) = certificate();
    let (_, other_key, _) = self::certificate();
    let (certificate_file, key_file) = written(folder.path(), &certificate, &key);
    let missing = folder.path().join("missing.der");
    let refused = |certificate: &Path, key: &Path| {
        ServingCertificate::open(certificate.to_owned(), key.to_owned())
            .map(|_| ())
            .map_err(|error| match error {
                ServingCertificateError::Certificate { file, .. } => ("certificate", file),
                ServingCertificateError::Key { file, .. } => ("key", file),
            })
    };
    let mismatched = folder.path().join("other-key.der");
    replace(&mismatched, &other_key);

    assert_eq!(
        [
            refused(&missing, &key_file),
            refused(&certificate_file, &missing),
            refused(&key_file, &key_file),
            refused(&certificate_file, &certificate_file),
            refused(&certificate_file, &mismatched),
        ],
        [
            Err(("certificate", missing.clone())),
            Err(("key", missing)),
            Err(("certificate", key_file)),
            Err(("key", certificate_file.clone())),
            Err(("key", mismatched)),
        ]
    );
}

#[test]
fn a_replaced_certificate_is_served_at_the_next_connection_and_an_unusable_one_never() {
    let folder = tempfile::tempdir().unwrap();
    let (first, first_key, first_der) = certificate();
    let (second, second_key, second_der) = certificate();
    let (certificate_file, key_file) = written(folder.path(), &first, &first_key);
    let owner = TaskOwner::root(BlockingPolicy::Inline);
    let serving = ServingCertificate::open(certificate_file.clone(), key_file.clone()).unwrap();

    let presented = goodfetch_test_support::runtime().block_on(async {
        let before = served(&serving, &owner, &first_der).await;
        // Half replaced: the new certificate with the old key serves
        // nothing, so the first one is still served.
        replace(&certificate_file, &second);
        let half_replaced = served(&serving, &owner, &first_der).await;
        replace(&key_file, &second_key);
        let replaced = served(&serving, &owner, &second_der).await;
        [before, half_replaced, replaced]
    });

    assert_eq!(
        presented,
        [Some(first_der.clone()), Some(first_der), Some(second_der)]
    );
}
