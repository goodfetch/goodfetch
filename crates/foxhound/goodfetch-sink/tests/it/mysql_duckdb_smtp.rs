//! MySQL/MariaDB, DuckDB and SMTP in-binary Sink conformance tests (ADRs 0026,
//! 0043, 0067 and 0127; issue 1163):
//! - DuckDB: embedded file, `GuaranteeClass::Fenced`, upsert/history/replace,
//!   schema evolution, zombie fencing, Unicode byte-for-byte round-trip, Secret
//!   canaries, `map_failure()`.
//! - SMTP: loopback SMTP receiver, `GuaranteeClass::AtLeastOnce`,
//!   `X-Goodfetch-Delivery-Id` and `X-Goodfetch-Run-Epoch` headers, Unicode
//!   round-trip, Secret canaries, `map_failure()`.
//! - MySQL/MariaDB: pinned `mariadb:11-ubi9` container, `GuaranteeClass::Fenced`,
//!   upsert/history/replace, schema evolution, zombie fencing, Unicode
//!   round-trip, Secret canaries, `map_failure()`.
#![expect(
    clippy::disallowed_methods,
    reason = "container test helper spawns podman/docker and polls loopback TCP on the real clock"
)]
#![expect(
    clippy::disallowed_types,
    reason = "loopback TCP readiness probe and in-process SMTP test receiver"
)]

use std::net::TcpStream;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use goodfetch_engine::clock::Clock;
use goodfetch_engine::mask::{Masks, Reach, Redaction};
use goodfetch_protocol::foxhound::sink::{SchemaContract, WriteMode};
use goodfetch_sink::{
    Driver, DuckDbError, DuckDbSink, FailureKind, GuaranteeClass, MySqlError, MySqlSink, RunCopy,
    SmtpError, SmtpSink, Status, map_duckdb_error, map_mysql_error, map_smtp_error,
};
use goodfetch_test_support::{Canaries, Output, SecretKind, unicode_record};
use mysql_async::prelude::Queryable as _;
use secrecy::SecretString;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::TcpListener;

use crate::run_files::{commit, copy, items, run_file};

type ItemRow = (String, Option<f64>, Option<String>, Option<String>);

fn unicode_value() -> Value {
    unicode_record()
        .fields
        .iter()
        .map(|field| (field.name.clone(), Value::String(field.text.clone())))
        .collect::<serde_json::Map<_, _>>()
        .into()
}

/// DuckDB Sink conformance: Fenced, upsert + schema evolution + history +
/// replace + zombie fencing + Unicode byte-for-byte round-trip + Secret canary.
#[test]
fn duckdb_sink_conformance_fenced_write_modes_and_unicode() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("sink.duckdb");
    let dsn = format!("duckdb://{}", db_path.display());

    let unicode = unicode_value();
    let sink = DuckDbSink::open(
        &dsn,
        "products",
        WriteMode::Upsert,
        SchemaContract::Evolve,
        false,
    )
    .unwrap();
    let records = run_file(&[(
        "products",
        vec![
            json!({ "sku": "D1", "price": 10 }),
            json!({ "sku": "D1", "price": 14.5 }),
            json!({ "sku": "D2", "price": "free", "uni": unicode }),
        ],
    )]);
    let mut driver = Driver::new("duck", sink, copy(2), vec!["products".to_owned()])
        .with_clock(Clock::system())
        .keyed(Some(vec!["sku".to_owned()]));
    assert_eq!(driver.deliver(&records).unwrap(), Status::Complete);
    assert_eq!(driver.class(), Some(GuaranteeClass::Fenced));

    // Zombie epoch 1 is fenced out.
    let zombie = DuckDbSink::open(
        &dsn,
        "products",
        WriteMode::Append,
        SchemaContract::Evolve,
        false,
    )
    .unwrap();
    let mut z_driver = Driver::new("duck", zombie, copy(1), vec!["products".to_owned()]);
    assert_eq!(
        z_driver.deliver(&records).unwrap(),
        Status::Superseded { run_epoch: 2 }
    );
    drop(driver);
    drop(z_driver);

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
                            .write_all(b"250-localhost\r\n250 8BITMIME\r\n")
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
/// can deduplicate, round-trips Unicode, and masks Secret canaries.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn smtp_sink_proves_at_least_once_with_delivery_id_headers() {
    let (port, captured) = start_loopback_smtp().await;
    let url = format!(
        "smtp://127.0.0.1:{port}?from=crawler@example.test&to=inbox@example.test&allow_insecure=true"
    );

    let mut canaries = Canaries::new(1163);
    let smtp_secret = canaries.plant(SecretKind::ProxyPassword).value().to_owned();
    let masks = Masks::new();
    let run_mask = masks.for_run(Redaction::On);
    run_mask.join("smtp_pass", &SecretString::from(smtp_secret), Reach::Value);

    let unicode = unicode_value();
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
    let class = tokio::task::spawn_blocking(move || {
        let sink = SmtpSink::from_url("mail_sink", &url_clone, None, endpoint).unwrap();
        let records = run_file(&[("alerts", vec![json!({ "msg": "hello" }), unicode])]);
        let mut driver = Driver::new("mail_sink", sink, copy(3), vec!["alerts".to_owned()])
            .with_clock(Clock::system())
            .with_masks(masks);
        assert_eq!(driver.deliver(&records).unwrap(), Status::Complete);
        driver.class()
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
    assert!(
        canaries
            .leaks(&[Output::Bytes {
                name: "smtp_message",
                bytes: raw.as_bytes(),
            }])
            .unwrap()
            .is_empty(),
        "secret canary leaked into SMTP message"
    );
    assert_eq!(
        map_smtp_error(SmtpError::Config("bad".into())).kind(),
        FailureKind::Permanent
    );
}

#[cfg(target_os = "linux")]
struct MariaDbContainer {
    engine: &'static str,
    container: String,
    dsn: String,
}

#[cfg(target_os = "linux")]
impl MariaDbContainer {
    const IMAGE_FILE: &'static str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../ci/mariadb/image.toml"
    ));

    async fn start() -> Self {
        let engine = ["docker", "podman"]
            .into_iter()
            .find(|c| {
                Command::new(c)
                    .arg("info")
                    .output()
                    .is_ok_and(|o| o.status.success())
            })
            .expect("no container engine (docker/podman) answers");
        let pin: toml::Table =
            toml::from_str(Self::IMAGE_FILE).expect("ci/mariadb/image.toml is valid TOML");
        let image = format!(
            "{}@{}",
            pin["repository"].as_str().unwrap(),
            pin["digest"].as_str().unwrap()
        );
        let out = Command::new(engine)
            .args([
                "run",
                "--detach",
                "--rm",
                "-p",
                "127.0.0.1::3306",
                "-e",
                "MARIADB_ROOT_PASSWORD=rootpass",
                "-e",
                "MARIADB_DATABASE=goodfetch",
                "-e",
                "MARIADB_USER=goodfetch",
                "-e",
                "MARIADB_PASSWORD=goodfetchpass",
            ])
            .arg(&image)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{engine} cannot start {image}: {}; `ci/mariadb/fetch.sh` pulls it",
            String::from_utf8_lossy(&out.stderr).trim()
        );
        let container = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        let port_out = Command::new(engine)
            .args(["port", &container, "3306/tcp"])
            .output()
            .unwrap();
        let host_port = String::from_utf8_lossy(&port_out.stdout)
            .lines()
            .next()
            .unwrap()
            .trim()
            .to_owned();
        let dsn = format!("mysql://goodfetch:goodfetchpass@{host_port}/goodfetch");

        let deadline = Instant::now() + Duration::from_secs(45);
        while Instant::now() < deadline {
            if TcpStream::connect(&host_port).is_ok()
                && let Ok(opts) = mysql_async::Opts::from_url(&dsn)
            {
                let pool = mysql_async::Pool::new(opts);
                let ok = pool.get_conn().await.is_ok();
                let _ = pool.disconnect().await;
                if ok {
                    return Self {
                        engine,
                        container,
                        dsn,
                    };
                }
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        panic!("mariadb container did not accept connections at {dsn} within 45s");
    }
}

#[cfg(target_os = "linux")]
impl Drop for MariaDbContainer {
    fn drop(&mut self) {
        let _ = Command::new(self.engine)
            .args(["rm", "-f", &self.container])
            .output();
    }
}

/// MySQL/MariaDB Sink conformance against pinned `mariadb:11-ubi9`:
/// proves `GuaranteeClass::Fenced`, upsert + schema evolution + history +
/// zombie fencing + Unicode byte-for-byte round-trip + ADR 0127 `map_failure()`.
#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mysql_mariadb_sink_conformance_against_mariadb_container() {
    let db = MariaDbContainer::start().await;
    let dsn = db.dsn.clone();
    let unicode = unicode_value();
    let unicode_expected = unicode.clone();

    let dsn_clone = dsn.clone();
    tokio::task::spawn_blocking(move || {
        let sink = MySqlSink::new(
            &dsn_clone,
            "products_my",
            WriteMode::Upsert,
            SchemaContract::Evolve,
            false,
        )
        .unwrap();
        let records = run_file(&[(
            "products",
            vec![
                json!({ "sku": "M1", "price": 10 }),
                json!({ "sku": "M1", "price": 19.5 }),
                json!({ "sku": "M2", "price": "free", "uni": unicode }),
            ],
        )]);
        let mut driver = Driver::new("my_sink", sink, copy(2), vec!["products".to_owned()])
            .with_clock(Clock::system())
            .keyed(Some(vec!["sku".to_owned()]));
        assert_eq!(driver.deliver(&records).unwrap(), Status::Complete);
        assert_eq!(driver.class(), Some(GuaranteeClass::Fenced));

        // Zombie epoch 1 is fenced out.
        let zombie = MySqlSink::new(
            &dsn_clone,
            "products_my",
            WriteMode::Append,
            SchemaContract::Evolve,
            false,
        )
        .unwrap();
        let mut z_driver = Driver::new("my_sink", zombie, copy(1), vec!["products".to_owned()]);
        assert_eq!(
            z_driver.deliver(&records).unwrap(),
            Status::Superseded { run_epoch: 2 }
        );

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
    drop(conn);
    let _ = pool.disconnect().await;

    assert_eq!(
        map_mysql_error(MySqlError::RejectedRecord("bad".into())).kind(),
        FailureKind::RejectedRecords
    );
    assert_eq!(
        map_mysql_error(MySqlError::Config("empty".into())).kind(),
        FailureKind::Permanent
    );
}
