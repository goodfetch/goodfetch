//! The trust a scenario served over TLS (#2284) gives each tool: the
//! Proving ground's certificate authority, read to a PEM file the trial
//! holds, handed to the other tools through the environment variables
//! their TLS stacks read, and to Foxhound through its user configuration.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous: a trial writes the tool's files with blocking calls (STYLE.md)"
)]

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::Path;
use std::process::Command;

use anyhow::{Context as _, Result};
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::home::{USER_CONFIGURATION_FILE, per_major};

/// The Proving ground's certificate authority, which signs each host's
/// leaf it serves over TLS, written to a PEM file the trial holds.
pub(crate) fn certificate_authority(address: &str) -> Result<tempfile::NamedTempFile> {
    let pem = ureq::get(&format!("http://{address}/certificate-authority"))
        .call()
        .context("reading the Proving ground's certificate authority")?
        .body_mut()
        .read_to_string()
        .context("reading the Proving ground's certificate authority")?;
    let mut file = tempfile::Builder::new()
        .suffix(".pem")
        .tempfile()
        .context("creating the certificate authority's file")?;
    file.write_all(pem.as_bytes())
        .context("writing the certificate authority's file")?;
    Ok(file)
}

/// Has a tool other than Foxhound verify the site's certificate against
/// `authority`: OpenSSL, Python and, on Linux, rustls's platform verifier
/// read `SSL_CERT_FILE` (in place of the system's roots), Node.js adds
/// `NODE_EXTRA_CA_CERTS` to its own. macOS's verifier ignores
/// `SSL_CERT_FILE`, so Spider and crawlee-rs there reject the site's
/// certificate.
pub(crate) fn in_environment(command: &mut Command, authority: &Path) {
    command
        .env("SSL_CERT_FILE", authority)
        .env("NODE_EXTRA_CA_CERTS", authority);
}

/// Writes Foxhound's user configuration in `home`, trusting `authority`
/// for each of `hosts`: `tls.trusted_cas` has Configuration authority, so
/// a Run's settings cannot carry it (ADR 0042).
pub(crate) fn in_foxhound_configuration(
    home: &Path,
    hosts: &[String],
    authority: &Path,
) -> Result<()> {
    let authority = authority
        .to_str()
        .with_context(|| format!("the authority's path {} is no text", authority.display()))?;
    // A JSON string is a TOML basic string.
    let authority = serde_json::to_string(authority)?;
    let mut configuration = String::from("version = \"0.1\"\n\n[settings]\n");
    for host in hosts {
        let host = serde_json::to_string(host)?;
        writeln!(configuration, "tls.trusted_cas.{host} = [{authority}]")?;
    }
    let version = ProductVersion::parse(env!("CARGO_PKG_VERSION"))
        .context("reading the workspace's Product version")?;
    let major = per_major(home, &version);
    std::fs::create_dir_all(&major).with_context(|| format!("creating {}", major.display()))?;
    let file = major.join(USER_CONFIGURATION_FILE);
    std::fs::write(&file, configuration).with_context(|| format!("writing {}", file.display()))
}
