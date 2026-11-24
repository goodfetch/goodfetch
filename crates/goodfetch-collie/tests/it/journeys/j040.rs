//! J-040, "Expose the Daemon over TLS from certificate files, and renew its
//! certificate with no restart", on the `cli` surface (ADRs 0049 and 0063;
//! ticket #667): an operator sees `daemon run` refuse an address that is
//! not loopback while no TLS is configured, names a certificate and its
//! key, runs Collie's shipped executable as `daemon run` again, and a
//! program of theirs, a Client speaking the protocol over TLS, connects
//! before and after the certificate is renewed in place.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use goodfetch_protocol::result_document::ResultDocument;
use goodfetch_test_support::JourneyRun;
use rustls::pki_types::CertificateDer;
use serde_json::{Value, json};

use crate::daemon_run::{Foreground, token};
use crate::tls::{Certificate, hello_over_lines, renew, serving_tls};

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// Covers: story:4-25, command:daemon.run, setting:daemon.tls.cert_file, setting:daemon.tls.key_file
#[test]
fn cli() {
    let mut run = JourneyRun::start("J-040", "cli", Path::new(COLLIE), ResultDocument::journey)
        .expect("J-040's cli variant starts");
    let (first, renewed) = (Certificate::generated(), Certificate::generated());
    let mut refusal = (Value::Null, None);
    let mut daemon: Option<(Foreground, SocketAddr, [PathBuf; 2])> = None;
    let mut before: (Option<CertificateDer<'static>>, Value) = (None, Value::Null);
    let mut after: (Option<CertificateDer<'static>>, Value) = (None, Value::Null);
    let mut exited = false;

    run.step(
        "Set `daemon.address` to an address that is not loopback, with no TLS configured, and run the Daemon in the foreground (`daemon run`)",
        |run| {
            run.env()
                .user_configuration("daemon.address = \"0.0.0.0:7900\"\n");
            let mut refused = Foreground::start(run.env(), run.engine().to_str().unwrap(), &[]);
            let record = refused.first_record("code", "setting_invalid");
            refusal = (record, refused.process.child().wait().unwrap().code());
        },
    );
    run.step(
        "Name a certificate and its key in `daemon.tls.cert_file` and `daemon.tls.key_file`, and run the Daemon in the foreground again",
        |run| daemon = Some(serving_tls(run.env(), &first)),
    );
    run.step(
        "Connect a program to the Daemon over TLS and open its Connection with the Daemon's token",
        |run| {
            let (_, address, _) = daemon.as_ref().expect("the Daemon serves");
            before = hello_over_lines(*address, &first.trusted, &token(run.env()));
        },
    );
    run.step(
        "Replace the certificate and key files with a renewed certificate while the Daemon serves, and connect the program again",
        |run| {
            let (_, address, files) = daemon.as_ref().expect("the Daemon serves");
            renew(files, &renewed);
            after = hello_over_lines(*address, &renewed.trusted, &token(run.env()));
            if let Some((serving, ..)) = daemon.as_mut() {
                exited = serving.terminate();
            }
        },
    );

    run.check(
        "The first `daemon run` refused the address with `setting_invalid` naming `daemon.tls.cert_file`, `daemon.tls.key_file` and `daemon.tls.upstream`, and exited with a failure",
        |_| {
            let (record, code) = &refusal;
            let message = record["message"].as_str().unwrap_or_default();
            assert_eq!(
                (
                    *code,
                    [
                        "`daemon.address`",
                        "`daemon.tls.cert_file`",
                        "`daemon.tls.key_file`",
                        "`daemon.tls.upstream`",
                    ]
                    .map(|key| message.contains(key)),
                ),
                (Some(1), [true; 4]),
                "{record}"
            );
        },
    );
    run.check(
        "Over TLS, the Daemon presented the named certificate and answered the program's `hello`",
        |_| {
            assert_eq!(
                (&before.0, &before.1["result"]["product"]),
                (&Some(first.trusted.clone()), &json!("collie")),
                "{}",
                before.1
            );
        },
    );
    run.check(
        "After the replacement, the same Daemon, never restarted, presented the renewed certificate and answered the program's next `hello`, and it exits cleanly when stopped",
        |_| {
            assert_eq!(
                (&after.0, &after.1["result"]["product"], exited),
                (&Some(renewed.trusted.clone()), &json!("collie"), true),
                "{}",
                after.1
            );
        },
    );
    run.finish()
        .expect("J-040's document is written")
        .assert_proved();
}
