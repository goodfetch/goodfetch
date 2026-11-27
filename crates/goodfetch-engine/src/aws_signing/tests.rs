//! Signature Version 4 against AWS's own worked example.

use secrecy::SecretString;

use super::{Credentials, Signing, authorization};

/// AWS's own Signature Version 4 example, IAM's `ListUsers` (the
/// "Signature Version 4 signing process" pages of the General Reference):
/// the documented signature is the independent answer. The example's
/// secret key is built from parts so it reads as no key in the source.
#[test]
fn the_signature_is_aws_documented_example() {
    let credentials = Credentials {
        access_key_id: "AKIDEXAMPLE".to_owned(),
        secret_access_key: SecretString::from(
            ["wJalrXUtnFEMI", "/K7MDENG+bPxRfiCY", "EXAMPLEKEY"].concat(),
        ),
        session_token: None,
    };
    let headers = [
        (
            "content-type",
            "application/x-www-form-urlencoded; charset=utf-8".to_owned(),
        ),
        ("host", "iam.amazonaws.com".to_owned()),
        ("x-amz-date", "20150830T123600Z".to_owned()),
    ];

    let header = authorization(
        &Signing {
            method: "GET",
            path: "/",
            query: "Action=ListUsers&Version=2010-05-08",
            headers: &headers,
            body: b"",
            region: "us-east-1",
            service: "iam",
            amz_date: "20150830T123600Z",
        },
        &credentials,
    );

    assert_eq!(
        header,
        "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/us-east-1/iam/aws4_request, SignedHeaders=content-type;host;x-amz-date, Signature=5d672d79c15b13162d9279b0855cfba6789a8edb4c82c400e06b5924a6f2b5d7"
    );
}
