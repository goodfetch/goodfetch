use std::collections::BTreeMap;

use super::{FormatName, FormatVersion, ProductVersion, ReleasesFile};

/// One release as the release's pull request to `main` appends it.
const ONE_RELEASE: &str = r#"
[[release]]
version = "0.3.0"
formats = { checkpoint = "0.2", usage-ledger = "1.4" }

[[release.collie.executables]]
target = "x86_64-unknown-linux-musl"
name = "goodfetch-collie-x86_64-unknown-linux-musl.tar.xz"
url = "https://github.com/goodfetch/goodfetch/releases/download/v0.3.0/goodfetch-collie-x86_64-unknown-linux-musl.tar.xz"
sha256 = "0000000000000000000000000000000000000000000000000000000000000001"
attestation = { url = "https://github.com/goodfetch/goodfetch/releases/download/v0.3.0/goodfetch-collie-x86_64-unknown-linux-musl.tar.xz.sigstore.json", sha256 = "0000000000000000000000000000000000000000000000000000000000000002" }

[[release.collie.npm]]
name = "@goodfetch/collie"
url = "https://registry.npmjs.org/@goodfetch/collie/-/collie-0.3.0.tgz"
sha256 = "0000000000000000000000000000000000000000000000000000000000000003"
attestation = { url = "https://registry.npmjs.org/-/npm/v1/attestations/@goodfetch/collie@0.3.0", sha256 = "0000000000000000000000000000000000000000000000000000000000000004" }

[release.collie.helm_chart]
name = "ghcr.io/goodfetch/charts/goodfetch-collie"
url = "https://ghcr.io/v2/goodfetch/charts/goodfetch-collie/manifests/0.3.0"
sha256 = "0000000000000000000000000000000000000000000000000000000000000005"
attestation = { url = "https://ghcr.io/v2/goodfetch/charts/goodfetch-collie/manifests/sha256-5.att", sha256 = "0000000000000000000000000000000000000000000000000000000000000006" }

[release.collie.image]
name = "ghcr.io/goodfetch/goodfetch-collie"
url = "https://ghcr.io/v2/goodfetch/goodfetch-collie/manifests/0.3.0"
sha256 = "0000000000000000000000000000000000000000000000000000000000000007"
attestation = { url = "https://ghcr.io/v2/goodfetch/goodfetch-collie/manifests/sha256-7.att", sha256 = "0000000000000000000000000000000000000000000000000000000000000008" }

[release.foxhound.helm_chart]
name = "ghcr.io/goodfetch/charts/goodfetch-foxhound"
url = "https://ghcr.io/v2/goodfetch/charts/goodfetch-foxhound/manifests/0.3.0"
sha256 = "0000000000000000000000000000000000000000000000000000000000000009"
attestation = { url = "https://ghcr.io/v2/goodfetch/charts/goodfetch-foxhound/manifests/sha256-9.att", sha256 = "000000000000000000000000000000000000000000000000000000000000000a" }

[release.foxhound.image]
name = "ghcr.io/goodfetch/goodfetch-foxhound"
url = "https://ghcr.io/v2/goodfetch/goodfetch-foxhound/manifests/0.3.0"
sha256 = "000000000000000000000000000000000000000000000000000000000000000b"
attestation = { url = "https://ghcr.io/v2/goodfetch/goodfetch-foxhound/manifests/sha256-b.att", sha256 = "000000000000000000000000000000000000000000000000000000000000000c" }

[release.collie_client.npm]
name = "@goodfetch/collie-client"
url = "https://registry.npmjs.org/@goodfetch/collie-client/-/collie-client-0.3.0.tgz"
sha256 = "000000000000000000000000000000000000000000000000000000000000000d"
attestation = { url = "https://registry.npmjs.org/-/npm/v1/attestations/@goodfetch/collie-client@0.3.0", sha256 = "000000000000000000000000000000000000000000000000000000000000000e" }

[release.collie_client.pypi]
name = "goodfetch_collie_client-0.3.0-py3-none-any.whl"
url = "https://files.pythonhosted.org/packages/goodfetch_collie_client-0.3.0-py3-none-any.whl"
sha256 = "000000000000000000000000000000000000000000000000000000000000000f"
attestation = { url = "https://pypi.org/integrity/goodfetch-collie-client/0.3.0/goodfetch_collie_client-0.3.0-py3-none-any.whl/provenance", sha256 = "0000000000000000000000000000000000000000000000000000000000000010" }

[release.collie_client.crates_io]
name = "goodfetch-collie-client"
url = "https://static.crates.io/crates/goodfetch-collie-client/goodfetch-collie-client-0.3.0.crate"
sha256 = "0000000000000000000000000000000000000000000000000000000000000011"
attestation = { url = "https://github.com/goodfetch/goodfetch/releases/download/v0.3.0/goodfetch-collie-client-0.3.0.crate.sigstore.json", sha256 = "0000000000000000000000000000000000000000000000000000000000000012" }

[[release.plugins]]
target = "aarch64-unknown-linux-musl"
name = "ghcr.io/goodfetch/plugins/sink-postgres"
url = "https://ghcr.io/v2/goodfetch/plugins/sink-postgres/manifests/0.3.0"
sha256 = "0000000000000000000000000000000000000000000000000000000000000013"
attestation = { url = "https://ghcr.io/v2/goodfetch/plugins/sink-postgres/manifests/sha256-13.att", sha256 = "0000000000000000000000000000000000000000000000000000000000000014" }
"#;

fn parse(text: &str) -> Result<ReleasesFile, toml::de::Error> {
    toml::from_str(text)
}

#[test]
fn a_file_with_no_release_records_none() {
    let releases = parse("# Nothing is released yet.\n").unwrap();

    assert_eq!(releases, ReleasesFile::default());
}

#[test]
fn a_release_records_its_product_version_and_the_format_versions_it_shipped() {
    let releases = parse(ONE_RELEASE).unwrap();

    let release = &releases.release[0];
    assert_eq!(release.version, ProductVersion::new(0, 3, 0));
    assert_eq!(
        release.formats,
        BTreeMap::from([
            (
                FormatName::try_from("checkpoint".to_owned()).unwrap(),
                FormatVersion::new(0, 2)
            ),
            (
                FormatName::try_from("usage-ledger".to_owned()).unwrap(),
                FormatVersion::new(1, 4)
            ),
        ])
    );
}

#[test]
fn a_release_records_each_artifact_with_its_digest_and_attestation() {
    let releases = parse(ONE_RELEASE).unwrap();

    let executable = &releases.release[0].collie.executables[0];
    assert_eq!(executable.target, "x86_64-unknown-linux-musl");
    assert_eq!(
        executable.sha256.as_str(),
        "0000000000000000000000000000000000000000000000000000000000000001"
    );
    assert_eq!(
        executable.attestation.url.as_str(),
        "https://github.com/goodfetch/goodfetch/releases/download/v0.3.0/goodfetch-collie-x86_64-unknown-linux-musl.tar.xz.sigstore.json"
    );
}

#[rstest::rstest]
#[case::uppercase_digest(
    "sha256 = \"0000000000000000000000000000000000000000000000000000000000000001\"",
    "sha256 = \"000000000000000000000000000000000000000000000000000000000000000A\""
)]
#[case::short_digest(
    "sha256 = \"0000000000000000000000000000000000000000000000000000000000000001\"",
    "sha256 = \"01\""
)]
#[case::plain_http_url(
    "url = \"https://github.com/goodfetch/goodfetch/releases/download/v0.3.0/goodfetch-collie-x86_64-unknown-linux-musl.tar.xz\"",
    "url = \"http://github.com/goodfetch/goodfetch/releases/download/v0.3.0/goodfetch-collie-x86_64-unknown-linux-musl.tar.xz\""
)]
#[case::release_candidate("version = \"0.3.0\"", "version = \"0.3.0-rc.1\"")]
#[case::format_version_with_a_patch("checkpoint = \"0.2\"", "checkpoint = \"0.2.1\"")]
#[case::format_name_with_a_capital("usage-ledger = \"1.4\"", "Usage-ledger = \"1.4\"")]
#[case::unknown_key("version = \"0.3.0\"", "version = \"0.3.0\"\nchannel = \"stable\"")]
fn a_malformed_entry_is_refused(#[case] from: &str, #[case] to: &str) {
    assert_eq!(ONE_RELEASE.matches(from).count(), 1, "{from}");
    let text = ONE_RELEASE.replace(from, to);

    let parsed = parse(&text);

    assert!(parsed.is_err());
}
