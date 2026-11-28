//! MySQL/MariaDB, DuckDB and SMTP in-binary Sink conformance tests (ADRs 0026,
//! 0043, 0067 and 0127; issue 1163):
//! - DuckDB: embedded file, `GuaranteeClass::Fenced`, `upsert`, `insert_only`,
//!   `history` and `replace`, schema evolution, epoch-1->epoch-2 takeover and
//!   zombie fencing, `Fault::AnswerLost` retry, Unicode byte-for-byte
//!   round-trip, Secret canary redaction, `map_failure()`.
//! - SMTP: loopback SMTP receiver, `GuaranteeClass::AtLeastOnce`,
//!   `X-Goodfetch-Delivery-Id` and `X-Goodfetch-Run-Epoch` headers, decoded
//!   MIME body Unicode byte-for-byte round-trip, non-vacuous Secret canary
//!   redaction, `map_failure()`.
//! - MySQL/MariaDB: pinned `mariadb:11-ubi9` container, `GuaranteeClass::Fenced`,
//!   `upsert`, `insert_only`, `history` and `replace`, schema evolution,
//!   epoch-1->epoch-2 takeover and zombie fencing, `Fault::AnswerLost` retry,
//!   Unicode byte-for-byte round-trip, non-vacuous Secret canary redaction,
//!   `map_failure()`.
#![expect(
    clippy::disallowed_methods,
    reason = "container test helper spawns podman/docker and polls loopback TCP on the real clock"
)]
#![expect(
    clippy::disallowed_types,
    reason = "loopback TCP readiness probe and in-process SMTP test receiver"
)]

use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use goodfetch_engine::clock::Clock;
use goodfetch_engine::mask::{Masks, Reach, Redaction};
use goodfetch_protocol::foxhound::sink::{SchemaContract, WriteMode};
use goodfetch_sink::{
    Driver, DuckDbError, DuckDbSink, FailureKind, GuaranteeClass, MySqlError, MySqlSink, RunCopy,
    SmtpError, SmtpSink, Status, map_duckdb_error, map_mysql_error, map_smtp_error,
};
use goodfetch_test_support::{Canaries, Output, SecretKind};
use mysql_async::prelude::Queryable as _;
use secrecy::SecretString;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::TcpListener;

use crate::containers::{ContainerGuard, unicode_value, url_with_credentials};
use crate::faults::{Fault, Faulty};
use crate::run_files::{commit, copy, items, run_file};

type ItemRow = (String, Option<f64>, Option<String>, Option<String>);

fn decode_quoted_printable(input: &str) -> Vec<u8> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'=' {
            if i + 2 < bytes.len() && bytes[i + 1] == b'\r' && bytes[i + 2] == b'\n' {
                i += 3;
                continue;
            }
            if i + 1 < bytes.len() && bytes[i + 1] == b'\n' {
                i += 2;
                continue;
            }
            if i + 2 < bytes.len()
                && let Ok(hex_str) = std::str::from_utf8(&bytes[i + 1..i + 3])
                && let Ok(val) = u8::from_str_radix(hex_str, 16)
            {
                out.push(val);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    out
}

fn decode_base64_body(input: &str) -> Vec<u8> {
    fn val(b: u8) -> Option<u8> {
        match b {
            b'A'..=b'Z' => Some(b - b'A'),
            b'a'..=b'z' => Some(b - b'a' + 26),
            b'0'..=b'9' => Some(b - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let bytes: Vec<u8> = input
        .bytes()
        .filter(|b| !b.is_ascii_whitespace() && *b != b'=')
        .collect();
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    for chunk in bytes.chunks(4) {
        let a = val(chunk[0]).unwrap();
        let b = val(chunk[1]).unwrap();
        out.push((a << 2) | (b >> 4));
        if chunk.len() > 2 {
            let c = val(chunk[2]).unwrap();
            out.push((b << 4) | (c >> 2));
            if chunk.len() > 3 {
                let d = val(chunk[3]).unwrap();
                out.push((c << 6) | d);
            }
        }
    }
    out
}

fn extract_smtp_jsonl_body(raw: &str) -> Vec<Value> {
    let (headers, body) = raw
        .split_once("\r\n\r\n")
        .or_else(|| raw.split_once("\n\n"))
        .expect("SMTP message must have header/body separator");
    let lower_headers = headers.to_ascii_lowercase();
    let decoded = if lower_headers.contains("content-transfer-encoding: base64") {
        String::from_utf8(decode_base64_body(body)).unwrap()
    } else if lower_headers.contains("content-transfer-encoding: quoted-printable") {
        String::from_utf8(decode_quoted_printable(body)).unwrap()
    } else {
        body.to_owned()
    };
    decoded
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

/// DuckDB Sink conformance: Fenced, `upsert` + `insert_only` + schema evolution +
/// `history` + `replace` + epoch-1->epoch-2 takeover + zombie fencing +
/// `Fault::AnswerLost` retry + Unicode byte-for-byte round-trip + Secret canary.
///
/// Covers: sink:duckdb
#[test]
#[expect(
    clippy::too_many_lines,
    reason = "end-to-end DuckDB conformance exercises all write modes, fault retry, fencing, Unicode, and canary checks"
)]
fn duckdb_sink_conformance_fenced_write_modes_and_unicode() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("sink.duckdb");
    let dsn = format!("duckdb://{}", db_path.display());

    let mut canaries = Canaries::new(11631);
    let canary_val = canaries
        .plant(SecretKind::SinkCredential)
        .value()
        .to_owned();
    let masks = Masks::new();
    let run_mask = masks.for_run(Redaction::On);
    run_mask.join("duck_token", &SecretString::from(canary_val), Reach::Value);

    let unicode = unicode_value();
    let sink = DuckDbSink::open(
        &dsn,
        "products",
        WriteMode::Upsert,
        SchemaContract::Evolve,
        false,
    )
    .unwrap();
    let debug_str = format!("{sink:?}");
    let faulty = Faulty::new(sink, Fault::AnswerLost(1));
    let records = run_file(&[(
        "products",
        vec![
            json!({ "sku": "D1", "price": 10 }),
            json!({ "sku": "D1", "price": 14.5 }),
            json!({ "sku": "D2", "price": "free", "uni": unicode }),
        ],
    )]);
    let mut driver = Driver::new("duck", faulty, copy(1), vec!["products".to_owned()])
        .with_clock(Clock::system())
        .with_masks(masks.clone())
        .with_batch_records(2)
        .keyed(Some(vec!["sku".to_owned()]));
    assert!(matches!(
        driver.deliver(&records).unwrap(),
        Status::Behind { .. }
    ));
    assert_eq!(driver.deliver(&records).unwrap(), Status::Complete);
    assert_eq!(driver.class(), Some(GuaranteeClass::Fenced));

    // InsertOnly preserves first version.
    let io_sink = DuckDbSink::open(
        &dsn,
        "products_io",
        WriteMode::InsertOnly,
        SchemaContract::Evolve,
        false,
    )
    .unwrap();
    let io_recs = run_file(&[(
        "products",
        vec![
            json!({ "sku": "I1", "price": 10 }),
            json!({ "sku": "I1", "price": 99 }),
            json!({ "sku": "I2", "price": 20 }),
        ],
    )]);
    let mut io_driver = Driver::new(
        "duck_io",
        io_sink,
        RunCopy {
            run_id: "run-duck-io".into(),
            run_epoch: 1,
        },
        vec!["products".to_owned()],
    )
    .keyed(Some(vec!["sku".to_owned()]));
    assert_eq!(io_driver.deliver(&io_recs).unwrap(), Status::Complete);

    // History closes previous version.
    let h_sink = DuckDbSink::open(
        &dsn,
        "products_hist",
        WriteMode::History,
        SchemaContract::Evolve,
        false,
    )
    .unwrap();
    let h_recs = run_file(&[("products", vec![json!({ "sku": "H1", "price": 10 })])]);
    let mut h_driver = Driver::new(
        "duck_hist",
        h_sink,
        RunCopy {
            run_id: "run-duck-hist".into(),
            run_epoch: 1,
        },
        vec!["products".to_owned()],
    )
    .keyed(Some(vec!["sku".to_owned()]));
    assert_eq!(h_driver.deliver(&h_recs).unwrap(), Status::Complete);
    commit(
        &h_recs,
        "products",
        2,
        &[json!({ "sku": "H1", "price": 25 })],
    );
    assert_eq!(h_driver.deliver(&h_recs).unwrap(), Status::Complete);

    // Replace swaps target atomically at finish, and Epoch 2 takes over from
    // Epoch 1 while fencing out zombie Epoch 1.
    let r_sink_e1 = DuckDbSink::open(
        &dsn,
        "products_rep",
        WriteMode::Replace,
        SchemaContract::Evolve,
        false,
    )
    .unwrap();
    let r_recs_e1 = run_file(&[("products", items("old-", 1))]);
    let mut r_driver_e1 = Driver::new(
        "duck_rep",
        r_sink_e1,
        RunCopy {
            run_id: "run-duck-rep".into(),
            run_epoch: 1,
        },
        vec!["products".to_owned()],
    )
    .keyed(Some(vec!["item".to_owned()]));
    assert_eq!(r_driver_e1.deliver(&r_recs_e1).unwrap(), Status::Complete);
    assert_eq!(r_driver_e1.finish(), Status::Complete);

    let r_sink = DuckDbSink::open(
        &dsn,
        "products_rep",
        WriteMode::Replace,
        SchemaContract::Evolve,
        false,
    )
    .unwrap();
    let r_recs = run_file(&[("products", items("dr-", 2))]);
    let mut r_driver = Driver::new(
        "duck_rep",
        r_sink,
        RunCopy {
            run_id: "run-duck-rep".into(),
            run_epoch: 2,
        },
        vec!["products".to_owned()],
    )
    .keyed(Some(vec!["item".to_owned()]));
    assert_eq!(r_driver.deliver(&r_recs).unwrap(), Status::Complete);
    assert_eq!(r_driver.finish(), Status::Complete);

    let zombie = DuckDbSink::open(
        &dsn,
        "products_rep",
        WriteMode::Append,
        SchemaContract::Evolve,
        false,
    )
    .unwrap();
    let mut z_driver = Driver::new(
        "duck_rep",
        zombie,
        RunCopy {
            run_id: "run-duck-rep".into(),
            run_epoch: 1,
        },
        vec!["products".to_owned()],
    );
    assert_eq!(
        z_driver.deliver(&r_recs).unwrap(),
        Status::Superseded { run_epoch: 2 }
    );

    drop(driver);
    drop(r_driver_e1);
    drop(z_driver);
    drop(io_driver);
    drop(h_driver);
    drop(r_driver);

    let conn = duckdb::Connection::open(&db_path).unwrap();
    let mut stmt = conn
        .prepare("SELECT sku, price, price__v_text, uni FROM products ORDER BY sku")
        .unwrap();
    let rows: Vec<ItemRow> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].0, "D1");
    assert!((rows[0].1.unwrap() - 14.5).abs() < f64::EPSILON);
    assert_eq!(rows[1].2.as_deref(), Some("free"));
    let decoded_uni: Value = serde_json::from_str(rows[1].3.as_deref().unwrap()).unwrap();
    assert_eq!(decoded_uni, unicode);

    let io_price: i64 = conn
        .query_row("SELECT price FROM products_io WHERE sku = 'I1'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(io_price, 10);

    let mut h_stmt = conn
        .prepare("SELECT _goodfetch_valid_to FROM products_hist ORDER BY _goodfetch_record_number")
        .unwrap();
    let h_valid_to: Vec<Option<String>> = h_stmt
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(h_valid_to.len(), 2);
    assert!(h_valid_to[0].is_some(), "v1 must be closed");
    assert!(h_valid_to[1].is_none(), "v2 must be current");

    let rep_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM products_rep", [], |r| r.get(0))
        .unwrap();
    assert_eq!(rep_count, 2);

    assert!(
        canaries
            .leaks(&[Output::Bytes {
                name: "duck_debug",
                bytes: debug_str.as_bytes(),
            }])
            .unwrap()
            .is_empty(),
        "secret canary leaked into DuckDB Debug"
    );
    assert_eq!(
        map_duckdb_error(DuckDbError::RejectedRecord("bad".into())).kind(),
        FailureKind::RejectedRecords
    );
    assert_eq!(
        map_duckdb_error(DuckDbError::Config("empty".into())).kind(),
        FailureKind::Permanent
    );
}

/// Minimal loopback SMTP server capturing every received DATA message.
async fn start_loopback_smtp() -> (u16, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let messages = Arc::new(Mutex::new(Vec::<String>::new()));
    let captured = Arc::clone(&messages);
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let captured = Arc::clone(&captured);
            tokio::spawn(async move {
                let (read_half, mut write_half) = stream.into_split();
                let mut reader = BufReader::new(read_half);
                let _ = write_half.write_all(b"220 localhost ESMTP ready\r\n").await;
                let mut line = String::new();
                loop {
                    line.clear();
                    if reader.read_line(&mut line).await.unwrap_or(0) == 0 {
                        break;
                    }
                    let upper = line.trim().to_ascii_uppercase();
                    if upper.starts_with("EHLO") {
                        let _ = write_half
                            .write_all(b"250-localhost\r\n250-AUTH PLAIN LOGIN\r\n250 8BITMIME\r\n")
                            .await;
                    } else if upper.starts_with("AUTH") {
                        let _ = write_half
                            .write_all(b"235 Authentication successful\r\n")
                            .await;
                    } else if upper.starts_with("MAIL FROM:") || upper.starts_with("RCPT TO:") {
                        let _ = write_half.write_all(b"250 OK\r\n").await;
                    } else if upper == "DATA" {
                        let _ = write_half
                            .write_all(b"354 End data with <CR><LF>.<CR><LF>\r\n")
                            .await;
                        let mut data = String::new();
                        loop {
                            let mut dline = String::new();
                            if reader.read_line(&mut dline).await.unwrap_or(0) == 0 {
                                break;
                            }
                            if dline == ".\r\n" || dline == ".\n" {
                                break;
                            }
                            data.push_str(&dline);
                        }
                        captured.lock().unwrap().push(data);
                        let _ = write_half.write_all(b"250 Queued\r\n").await;
                    } else if upper == "QUIT" {
                        let _ = write_half.write_all(b"221 Bye\r\n").await;
                        break;
                    } else {
                        let _ = write_half.write_all(b"250 OK\r\n").await;
                    }
                }
            });
        }
    });
    (port, messages)
}

/// SMTP Sink conformance: proves `GuaranteeClass::AtLeastOnce` with
/// `X-Goodfetch-Delivery-Id` and `X-Goodfetch-Run-Epoch` headers so a receiver
/// can deduplicate, decodes the MIME body to assert byte-for-byte Unicode
/// round-trip, and masks Secret canaries in credentials and `Debug` output.
///
/// Covers: sink:smtp
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn smtp_sink_proves_at_least_once_with_delivery_id_headers() {
    let (port, captured) = start_loopback_smtp().await;
    let mut canaries = Canaries::new(1163);
    let smtp_secret = canaries
        .plant(SecretKind::SinkCredential)
        .value()
        .to_owned();
    let url = url_with_credentials(
        &format!(
            "smtp://127.0.0.1:{port}?from=crawler@example.test&to=inbox@example.test&allow_insecure=true"
        ),
        "mailer",
        &smtp_secret,
    );

    let masks = Masks::new();
    let run_mask = masks.for_run(Redaction::On);
    run_mask.join("smtp_pass", &SecretString::from(smtp_secret), Reach::Value);

    let unicode = unicode_value();
    let expected_unicode = unicode.clone();
    let url_clone = url.clone();
    let resolved = goodfetch_engine::settings::resolve(
        &goodfetch_engine::settings::Layers::default(),
        goodfetch_protocol::catalog::Product::Foxhound,
        goodfetch_protocol::catalog::Product::Foxhound,
    )
    .unwrap();
    let endpoint = goodfetch_engine::notification_endpoint::NotificationEndpoint::of(
        &resolved,
        Clock::system(),
        goodfetch_engine::tasks::TaskOwner::root(goodfetch_engine::tasks::BlockingPolicy::Inline)
            .blocking_threads(),
    );
    let (class, debug_str) = tokio::task::spawn_blocking(move || {
        let sink = SmtpSink::from_url("mail_sink", &url_clone, None, endpoint).unwrap();
        let debug_str = format!("{sink:?}");
        let records = run_file(&[("alerts", vec![json!({ "msg": "hello" }), unicode])]);
        let mut driver = Driver::new("mail_sink", sink, copy(3), vec!["alerts".to_owned()])
            .with_clock(Clock::system())
            .with_masks(masks);
        assert_eq!(driver.deliver(&records).unwrap(), Status::Complete);
        (driver.class(), debug_str)
    })
    .await
    .unwrap();
    assert_eq!(class, Some(GuaranteeClass::AtLeastOnce));

    let msgs = captured.lock().unwrap().clone();
    assert_eq!(msgs.len(), 1);
    let raw = &msgs[0];
    assert!(
        raw.contains("X-Goodfetch-Delivery-Id: run-1/mail_sink/alerts/1-2"),
        "missing delivery ID header: {raw}"
    );
    assert!(
        raw.contains("X-Goodfetch-Run-Epoch: 3"),
        "missing run epoch header: {raw}"
    );
    let decoded_records = extract_smtp_jsonl_body(raw);
    assert_eq!(decoded_records.len(), 2);
    assert_eq!(decoded_records[0], json!({ "msg": "hello" }));
    assert_eq!(decoded_records[1], expected_unicode);
    assert!(
        canaries
            .leaks(&[
                Output::Bytes {
                    name: "smtp_message",
                    bytes: raw.as_bytes(),
                },
                Output::Bytes {
                    name: "smtp_debug",
                    bytes: debug_str.as_bytes(),
                },
            ])
            .unwrap()
            .is_empty(),
        "secret canary leaked into SMTP message or Debug"
    );
    assert_eq!(
        map_smtp_error(SmtpError::Config("bad".into())).kind(),
        FailureKind::Permanent
    );
}

#[cfg(target_os = "linux")]
struct MariaDbContainer {
    _guard: ContainerGuard,
    dsn: String,
}

#[cfg(target_os = "linux")]
impl MariaDbContainer {
    const IMAGE_FILE: &'static str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../ci/mariadb/image.toml"
    ));

    async fn start(password: &str) -> Self {
        let pass_env = format!("MARIADB_PASSWORD={password}");
        let guard = ContainerGuard::spawn(
            Self::IMAGE_FILE,
            &[
                "-p",
                "127.0.0.1::3306",
                "-e",
                "MARIADB_ROOT_PASSWORD=rootpass",
                "-e",
                "MARIADB_DATABASE=goodfetch",
                "-e",
                "MARIADB_USER=goodfetch",
                "-e",
                &pass_env,
            ],
            &[],
        );
        let port = guard.published_port("3306/tcp");
        let host_port = format!("127.0.0.1:{port}");
        let dsn = url_with_credentials(
            &format!("mysql://{host_port}/goodfetch"),
            "goodfetch",
            password,
        );

        let deadline = Instant::now() + Duration::from_secs(45);
        while Instant::now() < deadline {
            if TcpStream::connect(&host_port).is_ok()
                && let Ok(opts) = mysql_async::Opts::from_url(&dsn)
            {
                let pool = mysql_async::Pool::new(opts);
                let ok = pool.get_conn().await.is_ok();
                let _ = pool.disconnect().await;
                if ok {
                    return Self { _guard: guard, dsn };
                }
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        panic!("mariadb container did not accept connections at {host_port} within 45s");
    }
}

/// MySQL/MariaDB Sink conformance against pinned `mariadb:11-ubi9`:
/// proves `GuaranteeClass::Fenced`, `upsert` + `insert_only` + schema evolution +
/// `history` + `replace` + epoch-1->epoch-2 takeover + zombie fencing +
/// `Fault::AnswerLost` retry + Unicode byte-for-byte round-trip + non-vacuous
/// Secret canary redaction + ADR 0127 `map_failure()`.
///
/// Covers: sink:mysql
#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[expect(
    clippy::too_many_lines,
    reason = "end-to-end MariaDB conformance exercises all write modes, fault retry, fencing, Unicode, and canary checks"
)]
async fn mysql_mariadb_sink_conformance_against_mariadb_container() {
    let mut canaries = Canaries::new(11632);
    let my_pass = canaries
        .plant(SecretKind::SinkCredential)
        .value()
        .to_owned();
    let db = MariaDbContainer::start(&my_pass).await;
    let dsn = db.dsn.clone();
    let unicode = unicode_value();
    let unicode_expected = unicode.clone();

    let masks = Masks::new();
    let run_mask = masks.for_run(Redaction::On);
    run_mask.join("mysql_pass", &SecretString::from(my_pass), Reach::Value);

    let dsn_clone = dsn.clone();
    let debug_str = tokio::task::spawn_blocking(move || {
        let sink = MySqlSink::new(
            &dsn_clone,
            "products_my",
            WriteMode::Upsert,
            SchemaContract::Evolve,
            false,
        )
        .unwrap();
        let debug_str = format!("{sink:?}");
        let faulty = Faulty::new(sink, Fault::AnswerLost(1));
        let records = run_file(&[(
            "products",
            vec![
                json!({ "sku": "M1", "price": 10 }),
                json!({ "sku": "M1", "price": 19.5 }),
                json!({ "sku": "M2", "price": "free", "uni": unicode }),
            ],
        )]);
        let mut driver = Driver::new("my_sink", faulty, copy(1), vec!["products".to_owned()])
            .with_clock(Clock::system())
            .with_masks(masks.clone())
            .with_batch_records(2)
            .keyed(Some(vec!["sku".to_owned()]));
        assert!(matches!(
            driver.deliver(&records).unwrap(),
            Status::Behind { .. }
        ));
        assert_eq!(driver.deliver(&records).unwrap(), Status::Complete);
        assert_eq!(driver.class(), Some(GuaranteeClass::Fenced));

        // InsertOnly preserves first value.
        let io_sink = MySqlSink::new(
            &dsn_clone,
            "products_io",
            WriteMode::InsertOnly,
            SchemaContract::Evolve,
            false,
        )
        .unwrap();
        let io_recs = run_file(&[(
            "products",
            vec![
                json!({ "sku": "I1", "price": 10 }),
                json!({ "sku": "I1", "price": 99 }),
                json!({ "sku": "I2", "price": 20 }),
            ],
        )]);
        let mut io_driver = Driver::new(
            "my_io",
            io_sink,
            RunCopy {
                run_id: "run-my-io".into(),
                run_epoch: 1,
            },
            vec!["products".to_owned()],
        )
        .keyed(Some(vec!["sku".to_owned()]));
        assert_eq!(io_driver.deliver(&io_recs).unwrap(), Status::Complete);

        // History mode closes earlier version.
        let h_sink = MySqlSink::new(
            &dsn_clone,
            "products_hist",
            WriteMode::History,
            SchemaContract::Evolve,
            false,
        )
        .unwrap();
        let h_recs = run_file(&[("products", items("h", 1))]);
        let mut h_driver = Driver::new(
            "my_hist",
            h_sink,
            RunCopy {
                run_id: "run-my-hist".into(),
                run_epoch: 1,
            },
            vec!["products".to_owned()],
        )
        .keyed(Some(vec!["item".to_owned()]));
        assert_eq!(h_driver.deliver(&h_recs).unwrap(), Status::Complete);
        commit(&h_recs, "products", 2, &[json!({ "item": "h1" })]);
        assert_eq!(h_driver.deliver(&h_recs).unwrap(), Status::Complete);

        // Replace mode swaps staged rows at finish, and Epoch 2 takes over
        // from Epoch 1 while fencing out zombie Epoch 1.
        let r_sink_e1 = MySqlSink::new(
            &dsn_clone,
            "products_rep",
            WriteMode::Replace,
            SchemaContract::Evolve,
            false,
        )
        .unwrap();
        let r_recs_e1 = run_file(&[("products", items("old-", 1))]);
        let mut r_driver_e1 = Driver::new(
            "my_rep",
            r_sink_e1,
            RunCopy {
                run_id: "run-my-rep".into(),
                run_epoch: 1,
            },
            vec!["products".to_owned()],
        )
        .with_masks(masks.clone())
        .keyed(Some(vec!["item".to_owned()]));
        assert_eq!(r_driver_e1.deliver(&r_recs_e1).unwrap(), Status::Complete);
        assert_eq!(r_driver_e1.finish(), Status::Complete);

        let r_sink = MySqlSink::new(
            &dsn_clone,
            "products_rep",
            WriteMode::Replace,
            SchemaContract::Evolve,
            false,
        )
        .unwrap();
        let r_recs = run_file(&[("products", items("mr-", 2))]);
        let mut r_driver = Driver::new(
            "my_rep",
            r_sink,
            RunCopy {
                run_id: "run-my-rep".into(),
                run_epoch: 2,
            },
            vec!["products".to_owned()],
        )
        .with_masks(masks)
        .keyed(Some(vec!["item".to_owned()]));
        assert_eq!(r_driver.deliver(&r_recs).unwrap(), Status::Complete);
        assert_eq!(r_driver.finish(), Status::Complete);

        let zombie = MySqlSink::new(
            &dsn_clone,
            "products_rep",
            WriteMode::Append,
            SchemaContract::Evolve,
            false,
        )
        .unwrap();
        let mut z_driver = Driver::new(
            "my_rep",
            zombie,
            RunCopy {
                run_id: "run-my-rep".into(),
                run_epoch: 1,
            },
            vec!["products".to_owned()],
        );
        assert_eq!(
            z_driver.deliver(&r_recs).unwrap(),
            Status::Superseded { run_epoch: 2 }
        );

        debug_str
    })
    .await
    .unwrap();

    let pool = mysql_async::Pool::new(mysql_async::Opts::from_url(&dsn).unwrap());
    let mut conn = pool.get_conn().await.unwrap();
    let rows: Vec<ItemRow> = conn
        .query("SELECT `sku`, `price`, `price__v_text`, `uni` FROM `products_my` ORDER BY `sku`")
        .await
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert!((rows[0].1.unwrap() - 19.5).abs() < f64::EPSILON);
    assert_eq!(rows[1].2.as_deref(), Some("free"));
    let decoded_uni: Value = serde_json::from_str(rows[1].3.as_deref().unwrap()).unwrap();
    assert_eq!(decoded_uni, unicode_expected);

    let io_rows: Vec<(String, i64)> = conn
        .query("SELECT `sku`, `price` FROM `products_io` ORDER BY `sku`")
        .await
        .unwrap();
    assert_eq!(io_rows, vec![("I1".to_owned(), 10), ("I2".to_owned(), 20)]);

    let hist_rows: Vec<Option<String>> = conn
        .query(
            "SELECT `_goodfetch_valid_to` FROM `products_hist` ORDER BY `_goodfetch_record_number`",
        )
        .await
        .unwrap();
    assert_eq!(hist_rows.len(), 2);
    assert!(hist_rows[0].is_some(), "v1 must be closed");
    assert!(hist_rows[1].is_none(), "v2 must be current");

    let rep_rows: Vec<String> = conn
        .query("SELECT `item` FROM `products_rep` ORDER BY `item`")
        .await
        .unwrap();
    assert_eq!(rep_rows, vec!["mr-1", "mr-2"]);

    drop(conn);
    let _ = pool.disconnect().await;

    assert!(
        canaries
            .leaks(&[
                Output::Bytes {
                    name: "mysql_debug",
                    bytes: debug_str.as_bytes(),
                },
                Output::Bytes {
                    name: "mysql_rep_rows",
                    bytes: rep_rows.join("\n").as_bytes(),
                },
            ])
            .unwrap()
            .is_empty(),
        "secret canary leaked into MySQL Debug or rows"
    );
    assert_eq!(
        map_mysql_error(MySqlError::RejectedRecord("bad".into())).kind(),
        FailureKind::RejectedRecords
    );
    assert_eq!(
        map_mysql_error(MySqlError::Config("empty".into())).kind(),
        FailureKind::Permanent
    );
}

/// Live check against a real MySQL/MariaDB server (disabled until owner
/// approves a real account; ADRs 0064 and 0076).
#[test]
#[ignore = "live check: disabled until owner approves real MySQL account (ADRs 0064, 0076)"]
fn live_mysql_sink_check() {
    let dsn = std::env::var("GOODFETCH_LIVE_MYSQL_URL")
        .expect("set GOODFETCH_LIVE_MYSQL_URL when running live MySQL check");
    let sink = MySqlSink::new(
        &dsn,
        "goodfetch_live_check",
        WriteMode::Append,
        SchemaContract::Evolve,
        false,
    )
    .unwrap();
    let records = run_file(&[("live_check", vec![json!({ "ok": true })])]);
    let mut driver = Driver::new("mysql_live", sink, copy(1), vec!["live_check".to_owned()])
        .with_clock(Clock::system());
    assert_eq!(driver.deliver(&records).unwrap(), Status::Complete);
    assert_eq!(driver.class(), Some(GuaranteeClass::Fenced));
}

/// Live check against a real SMTP relay (disabled until owner approves a real
/// account; ADRs 0064 and 0076).
#[test]
#[ignore = "live check: disabled until owner approves real SMTP account (ADRs 0064, 0076)"]
fn live_smtp_sink_check() {
    let url = std::env::var("GOODFETCH_LIVE_SMTP_URL")
        .expect("set GOODFETCH_LIVE_SMTP_URL when running live SMTP check");
    let resolved = goodfetch_engine::settings::resolve(
        &goodfetch_engine::settings::Layers::default(),
        goodfetch_protocol::catalog::Product::Foxhound,
        goodfetch_protocol::catalog::Product::Foxhound,
    )
    .unwrap();
    let endpoint = goodfetch_engine::notification_endpoint::NotificationEndpoint::of(
        &resolved,
        Clock::system(),
        goodfetch_engine::tasks::TaskOwner::root(goodfetch_engine::tasks::BlockingPolicy::Inline)
            .blocking_threads(),
    );
    let sink = SmtpSink::from_url("smtp_live", &url, None, endpoint).unwrap();
    let records = run_file(&[("live_check", vec![json!({ "ok": true })])]);
    let mut driver = Driver::new("smtp_live", sink, copy(1), vec!["live_check".to_owned()])
        .with_clock(Clock::system());
    assert_eq!(driver.deliver(&records).unwrap(), Status::Complete);
    assert_eq!(driver.class(), Some(GuaranteeClass::AtLeastOnce));
}
