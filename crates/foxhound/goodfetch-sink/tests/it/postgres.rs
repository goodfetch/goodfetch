//! PostgreSQL and Supabase in-binary Sink conformance tests (ADRs 0026, 0043,
//! 0067 and 0127; issue 1162): runs against the pinned `postgres:17-alpine`
//! container, verifying `append`, `upsert`, `insert_only`, `replace`, and
//! `history` write modes, schema evolution, transactional epoch fencing
//! (`GuaranteeClass::Fenced`), Unicode byte-for-byte round-trip, Secret canary
//! checks, `map_failure()` translation, and disabled Live checks.
#![expect(
    clippy::disallowed_methods,
    reason = "container test helper spawns podman/docker and polls loopback TCP on the real clock"
)]
#![expect(
    clippy::disallowed_types,
    reason = "loopback TCP readiness probe for Postgres container"
)]

use std::net::TcpStream;
use std::process::Command;
use std::time::{Duration, Instant};

use goodfetch_engine::clock::Clock;
use goodfetch_engine::mask::{Masks, Reach, Redaction};
use goodfetch_protocol::foxhound::sink::{SchemaContract, WriteMode};
use goodfetch_sink::{
    Driver, FailureKind, GuaranteeClass, PostgresError, PostgresSink, Primitives as _, RunCopy,
    Status, map_postgres_error,
};
use goodfetch_test_support::{Canaries, Output, SecretKind, unicode_record};
use secrecy::SecretString;
use serde_json::{Value, json};
use tokio_postgres::NoTls;

use crate::run_files::{commit, copy, items, run_file};

const SINK: &str = "pg_sink";

#[cfg(target_os = "linux")]
struct PostgresContainer {
    engine: &'static str,
    container: String,
    dsn: String,
}

#[cfg(target_os = "linux")]
impl PostgresContainer {
    const IMAGE_FILE: &'static str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../ci/postgres/image.toml"
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
            toml::from_str(Self::IMAGE_FILE).expect("ci/postgres/image.toml is valid TOML");
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
                "127.0.0.1::5432",
                "-e",
                "POSTGRES_USER=goodfetch",
                "-e",
                "POSTGRES_PASSWORD=goodfetchpass",
                "-e",
                "POSTGRES_DB=goodfetch",
            ])
            .arg(&image)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{engine} cannot start {image}: {}; `ci/postgres/fetch.sh` pulls it",
            String::from_utf8_lossy(&out.stderr).trim()
        );
        let container = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        let port_out = Command::new(engine)
            .args(["port", &container, "5432/tcp"])
            .output()
            .unwrap();
        let host_port = String::from_utf8_lossy(&port_out.stdout)
            .lines()
            .next()
            .unwrap()
            .trim()
            .to_owned();
        let dsn = format!("postgres://goodfetch:goodfetchpass@{host_port}/goodfetch");

        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline {
            if TcpStream::connect(&host_port).is_ok()
                && tokio_postgres::connect(&dsn, NoTls).await.is_ok()
            {
                return Self {
                    engine,
                    container,
                    dsn,
                };
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("postgres container did not accept connections at {dsn} within 30s");
    }
}

#[cfg(target_os = "linux")]
impl Drop for PostgresContainer {
    fn drop(&mut self) {
        let _ = Command::new(self.engine)
            .args(["rm", "-f", &self.container])
            .output();
    }
}

#[cfg(target_os = "linux")]
async fn connect_pg(dsn: &str) -> tokio_postgres::Client {
    let (client, conn) = tokio_postgres::connect(dsn, NoTls).await.unwrap();
    tokio::spawn(async move {
        let _ = conn.await;
    });
    client
}

fn unicode_value() -> Value {
    unicode_record()
        .fields
        .iter()
        .map(|field| (field.name.clone(), Value::String(field.text.clone())))
        .collect::<serde_json::Map<_, _>>()
        .into()
}

#[cfg(target_os = "linux")]
async fn check_upsert_and_schema(dsn: &str, client: &tokio_postgres::Client) {
    let dsn_upsert = dsn.to_owned();
    let unicode = unicode_value();
    let unicode_expected = unicode.clone();
    tokio::task::spawn_blocking(move || {
        let sink = PostgresSink::new(
            &dsn_upsert,
            "products_upsert",
            WriteMode::Upsert,
            SchemaContract::Evolve,
            false,
        )
        .unwrap();
        let records = run_file(&[(
            "products",
            vec![
                json!({ "sku": "A1", "price": 10, "meta": { "tag": "v1" } }),
                json!({ "sku": "A2", "price": 12.5, "meta": { "tag": "v1" } }),
                json!({ "sku": "A1", "price": 15.0, "meta": { "tag": "v2" } }),
                json!({ "sku": "A3", "price": "free", "uni": unicode }),
            ],
        )]);
        let mut driver = Driver::new(SINK, sink, copy(1), vec!["products".to_owned()])
            .with_clock(Clock::system())
            .keyed(Some(vec!["sku".to_owned()]));
        assert_eq!(driver.deliver(&records).unwrap(), Status::Complete);
        assert_eq!(driver.class(), Some(GuaranteeClass::Fenced));
    })
    .await
    .unwrap();

    let rows = client
        .query(
            "SELECT sku, price, price__v_text, meta, uni FROM products_upsert ORDER BY sku",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(rows.len(), 3, "A1 upserted in place");
    let a1_price: f64 = rows[0].get(1);
    let a1_meta: Value = rows[0].get(3);
    assert!((a1_price - 15.0).abs() < f64::EPSILON);
    assert_eq!(a1_meta, json!({ "tag": "v2" }));
    let a3_variant: String = rows[2].get(2);
    let a3_uni: Value = rows[2].get(4);
    assert_eq!(a3_variant, "free");
    assert_eq!(a3_uni, unicode_expected);
}

#[cfg(target_os = "linux")]
async fn check_history(dsn: &str, client: &tokio_postgres::Client) {
    let dsn_hist = dsn.to_owned();
    tokio::task::spawn_blocking(move || {
        let sink = PostgresSink::new(
            &dsn_hist,
            "products_history",
            WriteMode::History,
            SchemaContract::Evolve,
            false,
        )
        .unwrap();
        let records = run_file(&[("products", vec![json!({ "sku": "H1", "price": 10 })])]);
        let mut driver = Driver::new(
            "pg_history",
            sink,
            RunCopy {
                run_id: "run-history".into(),
                run_epoch: 1,
            },
            vec!["products".to_owned()],
        )
        .with_clock(Clock::system())
        .keyed(Some(vec!["sku".to_owned()]));
        assert_eq!(driver.deliver(&records).unwrap(), Status::Complete);

        commit(
            &records,
            "products",
            2,
            &[json!({ "sku": "H1", "price": 20 })],
        );
        assert_eq!(driver.deliver(&records).unwrap(), Status::Complete);
    })
    .await
    .unwrap();

    let hist_rows = client
        .query(
            "SELECT price, _goodfetch_valid_from, _goodfetch_valid_to
             FROM products_history ORDER BY _goodfetch_record_number",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(hist_rows.len(), 2);
    let v1_to: Option<String> = hist_rows[0].get(2);
    let v2_to: Option<String> = hist_rows[1].get(2);
    assert!(v1_to.is_some(), "v1 must be closed");
    assert!(v2_to.is_none(), "v2 must be current");
}

#[cfg(target_os = "linux")]
async fn check_replace_and_fencing(dsn: &str, client: &tokio_postgres::Client) {
    let dsn_rep = dsn.to_owned();
    let mut canaries = Canaries::new(1162);
    let secret_canary = canaries.plant(SecretKind::ProxyPassword).value().to_owned();
    let masks = Masks::new();
    let run_mask = masks.for_run(Redaction::On);
    run_mask.join(
        "pg_password",
        &SecretString::from(secret_canary),
        Reach::Value,
    );
    tokio::task::spawn_blocking(move || {
        let sink = PostgresSink::new(
            &dsn_rep,
            "products_replace",
            WriteMode::Replace,
            SchemaContract::Evolve,
            false,
        )
        .unwrap();
        let records = run_file(&[("products", items("r", 3))]);
        let mut driver = Driver::new(
            "pg_replace",
            sink,
            RunCopy {
                run_id: "run-replace".into(),
                run_epoch: 2,
            },
            vec!["products".to_owned()],
        )
        .with_clock(Clock::system())
        .with_masks(masks)
        .keyed(Some(vec!["item".to_owned()]));
        assert_eq!(driver.deliver(&records).unwrap(), Status::Complete);
        assert_eq!(driver.finish(), Status::Complete);

        let zombie = PostgresSink::new(
            &dsn_rep,
            "products_replace",
            WriteMode::Append,
            SchemaContract::Evolve,
            false,
        )
        .unwrap();
        let mut z_driver = Driver::new(
            "pg_replace",
            zombie,
            RunCopy {
                run_id: "run-replace".into(),
                run_epoch: 1,
            },
            vec!["products".to_owned()],
        );
        assert_eq!(
            z_driver.deliver(&records).unwrap(),
            Status::Superseded { run_epoch: 2 }
        );
    })
    .await
    .unwrap();

    let rep_rows = client
        .query("SELECT item FROM products_replace ORDER BY item", &[])
        .await
        .unwrap();
    let rep_items: Vec<String> = rep_rows.iter().map(|r| r.get(0)).collect();
    assert_eq!(rep_items, vec!["r1", "r2", "r3"]);
    let joined = rep_items.join("\n");
    assert!(
        canaries
            .leaks(&[Output::Bytes {
                name: "pg_rows",
                bytes: joined.as_bytes(),
            }])
            .unwrap()
            .is_empty(),
        "secret canary leaked into PostgreSQL rows"
    );
}

/// PostgreSQL & Supabase conformance against pinned `postgres:17-alpine`:
/// verifies `GuaranteeClass::Fenced`, native `ON CONFLICT` upsert and
/// `insert_only`, `history` versioning, `replace` atomic swap, schema
/// evolution (widening + variant column + JSONB), zombie fencing, Unicode
/// byte-for-byte round-trip, and Secret canary checks.
#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn postgres_and_supabase_conformance_against_postgres_container() {
    let pg = PostgresContainer::start().await;
    let client = connect_pg(&pg.dsn).await;

    check_upsert_and_schema(&pg.dsn, &client).await;
    check_history(&pg.dsn, &client).await;
    check_replace_and_fencing(&pg.dsn, &client).await;

    // 4. Error translation (ADR 0127): auth & permanent & rejected.
    let bad_dsn = pg.dsn.replace("goodfetchpass", "wrongpass");
    let auth_err = tokio::task::spawn_blocking(move || {
        let mut bad = PostgresSink::new(
            &bad_dsn,
            "products_upsert",
            WriteMode::Append,
            SchemaContract::Evolve,
            false,
        )
        .unwrap();
        let err = bad.probe(&RunCopy {
            run_id: "r".into(),
            run_epoch: 1,
        });
        bad.map_failure(err.unwrap_err())
    })
    .await
    .unwrap();
    assert_eq!(auth_err.kind(), FailureKind::Auth);
    assert_eq!(
        map_postgres_error(PostgresError::RejectedRecord("bad".into())).kind(),
        FailureKind::RejectedRecords
    );
    assert_eq!(
        map_postgres_error(PostgresError::Config("empty".into())).kind(),
        FailureKind::Permanent
    );
}

/// Live check against a real PostgreSQL instance (disabled until owner approves;
/// ADRs 0064 and 0076).
#[test]
#[ignore = "live check: disabled until owner approves real PostgreSQL account (ADRs 0064, 0076)"]
fn live_postgres_sink_check() {
    let dsn = std::env::var("GOODFETCH_LIVE_POSTGRES_URL")
        .expect("set GOODFETCH_LIVE_POSTGRES_URL when running live Postgres check");
    let _ = PostgresSink::new(
        &dsn,
        "records",
        WriteMode::Append,
        SchemaContract::Evolve,
        false,
    )
    .unwrap();
}

/// Live check against a real Supabase project (disabled until owner approves;
/// ADRs 0064 and 0076).
#[test]
#[ignore = "live check: disabled until owner approves real Supabase account (ADRs 0064, 0076)"]
fn live_supabase_sink_check() {
    let dsn = std::env::var("GOODFETCH_LIVE_SUPABASE_URL")
        .expect("set GOODFETCH_LIVE_SUPABASE_URL when running live Supabase check");
    let _ = PostgresSink::new(
        &dsn,
        "records",
        WriteMode::Append,
        SchemaContract::Evolve,
        false,
    )
    .unwrap();
}
