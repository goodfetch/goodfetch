//! The license read from a distribution's core metadata: its
//! `License-Expression`, else a `License` field that is an SPDX expression,
//! else the one license its Trove classifiers name without doubt.

use rstest::rstest;

use super::Metadata;

/// The license read from metadata headed by `headers`.
fn license(headers: &str) -> Option<String> {
    Metadata::parse(&format!(
        "Metadata-Version: 2.1\nName: example\nVersion: 1.0.0\n{headers}\nThe description.\n"
    ))
    .license
}

/// A `License` field in free text gives way to the license the classifiers
/// name, as sortedcontainers 2.4.0's `Apache 2.0` does.
#[rstest]
#[case::apache(
    "License: Apache 2.0\nClassifier: License :: OSI Approved :: Apache Software License\n",
    "Apache-2.0"
)]
#[case::mit(
    "License: MIT License\nClassifier: License :: OSI Approved :: MIT License\n",
    "MIT"
)]
#[case::isc(
    "License: ISC license\nClassifier: License :: OSI Approved :: ISC License (ISCL)\n",
    "ISC"
)]
#[case::psf(
    "License: PSF License\nClassifier: License :: OSI Approved :: Python Software Foundation License\n",
    "PSF-2.0"
)]
#[case::generic_classifier_beside_it(
    "License: Apache 2.0\nClassifier: License :: OSI Approved\nClassifier: License :: OSI Approved :: Apache Software License\n",
    "Apache-2.0"
)]
#[case::no_license_field("Classifier: License :: OSI Approved :: MIT License\n", "MIT")]
#[case::unknown_license_field(
    "License: UNKNOWN\nClassifier: License :: OSI Approved :: MIT License\n",
    "MIT"
)]
fn a_free_text_license_reads_as_its_classifier(#[case] headers: &str, #[case] read: &str) {
    assert_eq!(license(headers).as_deref(), Some(read));
}

/// A stated expression wins over the classifiers, which never override it.
#[rstest]
#[case::expression(
    "License-Expression: MIT OR Apache-2.0\nLicense: Apache 2.0\nClassifier: License :: OSI Approved :: Apache Software License\n",
    "MIT OR Apache-2.0"
)]
#[case::license_field_is_an_expression(
    "License: BSD-3-Clause\nClassifier: License :: OSI Approved :: MIT License\n",
    "BSD-3-Clause"
)]
fn a_stated_expression_is_kept(#[case] headers: &str, #[case] read: &str) {
    assert_eq!(license(headers).as_deref(), Some(read));
}

/// When the classifiers name no license without doubt, the free text is
/// kept, so the check still refuses it and shows what was stated.
#[rstest]
#[case::bsd_has_no_version("License: BSD\nClassifier: License :: OSI Approved :: BSD License\n")]
#[case::two_licenses(
    "License: Apache 2.0\nClassifier: License :: OSI Approved :: Apache Software License\nClassifier: License :: OSI Approved :: MIT License\n"
)]
#[case::one_unmapped(
    "License: Apache 2.0\nClassifier: License :: OSI Approved :: Apache Software License\nClassifier: License :: OSI Approved :: GNU General Public License (GPL)\n"
)]
#[case::no_classifier("License: Apache 2.0\nClassifier: Programming Language :: Python\n")]
fn a_free_text_license_without_a_clear_classifier_is_kept(#[case] headers: &str) {
    let stated = headers
        .lines()
        .find_map(|line| line.strip_prefix("License: "))
        .unwrap();

    assert_eq!(license(headers).as_deref(), Some(stated));
}

/// A distribution that states nothing and names no license reads as none.
#[test]
fn no_license_and_no_classifier_reads_as_none() {
    assert_eq!(
        license("Classifier: Programming Language :: Python\n"),
        None
    );
}
