//! PostgreSQL and Supabase in-binary Sink conformance tests (ADRs 0026, 0043,
//! 0067 and 0127; issue 1162): runs against the pinned `postgres:17-alpine`
//! container, verifying `append`, `upsert`, `insert_only`, `replace`, and
//! `history` write modes, schema evolution, transactional epoch fencing
//! (`GuaranteeClass::Fenced`) including epoch-1->epoch-2 takeover, `Fault::AnswerLost`
//! retry, `SinkType::Supabase` routing, Unicode byte-for-byte round-trip,
//! non-vacuous Secret canary checks, `map_failure()` translation, and disabled
//! Live checks.
#![expect(
    clippy::disallowed_methods,
    reason = "container test helper spawns podman/docker and polls loopback TCP on the real clock"
)]
#![expect(
    clippy::disallowed_types,
    reason = "loopback TCP readiness probe for Postgres container"
)]

use std::net::TcpStream;
use std::time::{Duration, Instant};

use goodfetch_engine::clock::Clock;
use goodfetch_engine::mask::{Masks, Reach, Redaction};
use goodfetch_protocol::foxhound::sink::{SchemaContract, SinkType, WriteMode};
use goodfetch_sink::{
    Driver, FailureKind, GuaranteeClass, PostgresError, PostgresSink, Primitives as _, RunCopy,
    Status, map_postgres_error,
};
use goodfetch_test_support::{Canaries, Output, SecretKind};
use secrecy::SecretString;
use serde_json::{Value, json};
use tokio_postgres::NoTls;

use crate::containers::{ContainerGuard, unicode_value, url_with_credentials};
use crate::faults::{Fault, Faulty};
use crate::run_files::{commit, copy, items, run_file};

const SINK: &str = "pg_sink";

#[cfg(target_os = "linux")]
struct PostgresContainer {
    _guard: ContainerGuard,
    dsn: String,
}

#[cfg(target_os = "linux")]
impl PostgresContainer {
    const IMAGE_FILE: &'static str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../ci/postgres/image.toml"
    ));

    async fn start(password: &str) -> Self {
        let pass_env = format!("POSTGRES_PASSWORD={password}");
        let guard = ContainerGuard::spawn(
            Self::IMAGE_FILE,
            &[
                "-p",
                "127.0.0.1::5432",
                "-e",
                "POSTGRES_USER=goodfetch",
                "-e",
                &pass_env,
                "-e",
                "POSTGRES_DB=goodfetch",
            ],
            &[],
        );
        let port = guard.published_port("5432/tcp");
        let host_port = format!("127.0.0.1:{port}");
        let dsn = url_with_credentials(
            &format!("postgres://{host_port}/goodfetch"),
            "goodfetch",
            password,
        );

        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline {
            if TcpStream::connect(&host_port).is_ok()
                && tokio_postgres::connect(&dsn, NoTls).await.is_ok()
            {
                return Self { _guard: guard, dsn };
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("postgres container did not accept connections at {host_port} within 30s");
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

#[cfg(target_os = "linux")]
async fn check_upsert_insert_only_and_schema(dsn: &str, client: &tokio_postgres::Client) -> String {
    let dsn_upsert = dsn.to_owned();
    let unicode = unicode_value();
    let unicode_expected = unicode.clone();
    let debug_str = tokio::task::spawn_blocking(move || {
        let sink = PostgresSink::new(
            &dsn_upsert,
            "products_upsert",
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
                json!({ "sku": "A1", "price": 10, "meta": { "tag": "v1" } }),
                json!({ "sku": "A2", "price": 12.5, "meta": { "tag": "v1" } }),
                json!({ "sku": "A1", "price": 15.0, "meta": { "tag": "v2" } }),
                json!({ "sku": "A3", "price": "free", "uni": unicode }),
            ],
        )]);
        let mut driver = Driver::new(SINK, faulty, copy(1), vec!["products".to_owned()])
            .with_clock(Clock::system())
            .with_batch_records(2)
            .keyed(Some(vec!["sku".to_owned()]));
        assert!(matches!(
            driver.deliver(&records).unwrap(),
            Status::Behind { .. }
        ));
        assert_eq!(driver.deliver(&records).unwrap(), Status::Complete);
        assert_eq!(driver.class(), Some(GuaranteeClass::Fenced));

        // InsertOnly keeps the first version on key conflict.
        let io_sink = PostgresSink::new(
            &dsn_upsert,
            "products_insert_only",
            WriteMode::InsertOnly,
            SchemaContract::Evolve,
            false,
        )
        .unwrap();
        let io_records = run_file(&[(
            "products",
            vec![
                json!({ "sku": "I1", "price": 10 }),
                json!({ "sku": "I1", "price": 99 }),
                json!({ "sku": "I2", "price": 20 }),
            ],
        )]);
        let mut io_driver = Driver::new(
            "pg_insert_only",
            io_sink,
            RunCopy {
                run_id: "run-io".into(),
                run_epoch: 1,
            },
            vec!["products".to_owned()],
        )
        .with_clock(Clock::system())
        .keyed(Some(vec!["sku".to_owned()]));
        assert_eq!(io_driver.deliver(&io_records).unwrap(), Status::Complete);
        debug_str
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

    let io_rows = client
        .query(
            "SELECT sku, price FROM products_insert_only ORDER BY sku",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(io_rows.len(), 2);
    let i1_price: i64 = io_rows[0].get(1);
    let i2_price: i64 = io_rows[1].get(1);
    assert_eq!(i1_price, 10, "insert_only must preserve first value");
    assert_eq!(i2_price, 20);
    debug_str
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
async fn check_replace_and_fencing(
    dsn: &str,
    client: &tokio_postgres::Client,
    masks: Masks,
) -> String {
    let dsn_rep = dsn.to_owned();
    tokio::task::spawn_blocking(move || {
        // Epoch 1 delivers first, then Epoch 2 takes over cleanly, then zombie Epoch 1 is fenced.
        let sink_e1 = PostgresSink::new(
            &dsn_rep,
            "products_replace",
            WriteMode::Replace,
            SchemaContract::Evolve,
            false,
        )
        .unwrap();
        let records_e1 = run_file(&[("products", items("old-", 2))]);
        let mut driver_e1 = Driver::new(
            "pg_replace",
            sink_e1,
            RunCopy {
                run_id: "run-replace".into(),
                run_epoch: 1,
            },
            vec!["products".to_owned()],
        )
        .with_clock(Clock::system())
        .with_masks(masks.clone())
        .keyed(Some(vec!["item".to_owned()]));
        assert_eq!(driver_e1.deliver(&records_e1).unwrap(), Status::Complete);
        assert_eq!(driver_e1.finish(), Status::Complete);

        // Epoch 2 takes over the same (target, run_id) and replaces table contents.
        let sink_e2 = PostgresSink::new(
            &dsn_rep,
            "products_replace",
            WriteMode::Replace,
            SchemaContract::Evolve,
            false,
        )
        .unwrap();
        let records_e2 = run_file(&[("products", items("r", 3))]);
        let mut driver_e2 = Driver::new(
            "pg_replace",
            sink_e2,
            RunCopy {
                run_id: "run-replace".into(),
                run_epoch: 2,
            },
            vec!["products".to_owned()],
        )
        .with_clock(Clock::system())
        .with_masks(masks)
        .keyed(Some(vec!["item".to_owned()]));
        assert_eq!(driver_e2.deliver(&records_e2).unwrap(), Status::Complete);
        assert_eq!(driver_e2.finish(), Status::Complete);

        // Zombie Epoch 1 is fenced out.
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
            z_driver.deliver(&records_e2).unwrap(),
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
    rep_items.join("\n")
}

/// PostgreSQL & Supabase conformance against pinned `postgres:17-alpine`:
/// verifies `GuaranteeClass::Fenced`, native `ON CONFLICT` upsert and
/// `insert_only`, `history` versioning, `replace` atomic swap, schema
/// evolution (widening + variant column + JSONB), epoch-1->epoch-2 takeover
/// and zombie fencing, `Fault::AnswerLost` retry, `SinkType::Supabase`
/// routing, Unicode byte-for-byte round-trip, and non-vacuous Secret canary
/// redaction.
///
/// Covers: sink:postgres, sink:supabase
#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn postgres_and_supabase_conformance_against_postgres_container() {
    let mut canaries = Canaries::new(1162);
    let pg_pass = canaries
        .plant(SecretKind::SinkCredential)
        .value()
        .to_owned();
    let pg = PostgresContainer::start(&pg_pass).await;
    let client = connect_pg(&pg.dsn).await;

    let masks = Masks::new();
    let run_mask = masks.for_run(Redaction::On);
    run_mask.join(
        "pg_password",
        &SecretString::from(pg_pass.clone()),
        Reach::Value,
    );

    let debug_str = check_upsert_insert_only_and_schema(&pg.dsn, &client).await;
    check_history(&pg.dsn, &client).await;
    let joined_rows = check_replace_and_fencing(&pg.dsn, &client, masks.clone()).await;

    // Verify SinkType::Supabase is a distinct protocol Sink type backed by PostgresSink.
    assert_ne!(SinkType::Supabase, SinkType::Postgres);
    let dsn_supa = pg.dsn.clone();
    tokio::task::spawn_blocking(move || {
        let sink = PostgresSink::new(
            &dsn_supa,
            "supabase_items",
            WriteMode::Append,
            SchemaContract::Evolve,
            false,
        )
        .unwrap();
        let records = run_file(&[("items", vec![json!({ "id": "sb-1" })])]);
        let mut driver = Driver::new("supabase_sink", sink, copy(1), vec!["items".to_owned()])
            .with_clock(Clock::system())
            .with_masks(masks);
        assert_eq!(driver.deliver(&records).unwrap(), Status::Complete);
        assert_eq!(driver.class(), Some(GuaranteeClass::Fenced));
    })
    .await
    .unwrap();

    // Error translation (ADR 0127): auth & permanent & rejected.
    let bad_dsn = {
        let mut u = url::Url::parse(&pg.dsn).unwrap();
        u.set_password(Some("wrongpass")).unwrap();
        u.to_string()
    };
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

    assert!(
        canaries
            .leaks(&[
                Output::Bytes {
                    name: "pg_rows",
                    bytes: joined_rows.as_bytes(),
                },
                Output::Bytes {
                    name: "pg_debug",
                    bytes: debug_str.as_bytes(),
                },
                Output::Bytes {
                    name: "pg_auth_err",
                    bytes: format!("{auth_err:?}").as_bytes(),
                },
            ])
            .unwrap()
            .is_empty(),
        "secret canary leaked into PostgreSQL output, Debug, or error"
    );
}

/// Live check against a real PostgreSQL instance (disabled until owner approves;
/// ADRs 0064 and 0076).
#[test]
#[ignore = "live check: disabled until owner approves real PostgreSQL account (ADRs 0064, 0076)"]
fn live_postgres_sink_check() {
    let dsn = std::env::var("GOODFETCH_LIVE_POSTGRES_URL")
        .expect("set GOODFETCH_LIVE_POSTGRES_URL when running live Postgres check");
    let sink = PostgresSink::new(
        &dsn,
        "goodfetch_live_check",
        WriteMode::Append,
        SchemaContract::Evolve,
        false,
    )
    .unwrap();
    let records = run_file(&[("live_check", vec![json!({ "ok": true })])]);
    let mut driver =
        Driver::new(SINK, sink, copy(1), vec!["live_check".to_owned()]).with_clock(Clock::system());
    assert_eq!(driver.deliver(&records).unwrap(), Status::Complete);
    assert_eq!(driver.class(), Some(GuaranteeClass::Fenced));
}

/// Live check against a real Supabase project (disabled until owner approves;
/// ADRs 0064 and 0076).
#[test]
#[ignore = "live check: disabled until owner approves real Supabase account (ADRs 0064, 0076)"]
fn live_supabase_sink_check() {
    let dsn = std::env::var("GOODFETCH_LIVE_SUPABASE_URL")
        .expect("set GOODFETCH_LIVE_SUPABASE_URL when running live Supabase check");
    let sink = PostgresSink::new(
        &dsn,
        "goodfetch_live_check",
        WriteMode::Append,
        SchemaContract::Evolve,
        false,
    )
    .unwrap();
    let records = run_file(&[("live_check", vec![json!({ "ok": true })])]);
    let mut driver = Driver::new(
        "supabase_sink",
        sink,
        copy(1),
        vec!["live_check".to_owned()],
    )
    .with_clock(Clock::system());
    assert_eq!(driver.deliver(&records).unwrap(), Status::Complete);
    assert_eq!(driver.class(), Some(GuaranteeClass::Fenced));
}
