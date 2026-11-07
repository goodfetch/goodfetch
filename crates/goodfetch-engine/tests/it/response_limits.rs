//! The limits on what the engine reads, on its own HTTP client against the
//! Proving ground (ADR 0067, SC-05, pitfall H14): every page of the
//! hostile-input limits table ends in `response_limit_exceeded` naming its
//! setting, the decompression bombs, the body past 64 MiB, the endless
//! bodies and the JSON nested past 128, while the reader holds no more than
//! the limit; a normal page and JSON at the depth pass. A body read as a
//! download, chunk by chunk to a file, is never counted.
//! Covers: row:SC-05, error:response_limit_exceeded

use std::net::IpAddr;

use goodfetch_engine::clock::Clock;
use goodfetch_engine::egress::Policy;
use goodfetch_engine::happy_eyeballs::Delays;
use goodfetch_engine::http_client::{BodyError, PageTraffic, Resolve, Response, Waiter};
use goodfetch_engine::response_limits::Limits;
use goodfetch_engine::tls::{CertificateExceptions, ClientCertificates, Trust};
use goodfetch_protocol::response_limits::LimitSetting;
use goodfetch_test_support::{
    ExpectedError, HostileTables, JsonChannel, LimitSetting as TableSetting, ProvingGround,
    ReceivedRequests, ResourceLimit, ServedBy, TestEnv,
};
use rustls::pki_types::CertificateDer;
use rustls::pki_types::pem::PemObject as _;
use serde_json::{Value, json};

const SEED: u64 = 644;

/// 64 MiB, `network.max_body_size`'s default.
const MAX_BODY_SIZE: u64 = 64 * 1024 * 1024;

/// What a run may reach: every host name the Proving ground serves, at its
/// address, under its certificate authority.
fn traffic(proving_ground: &ProvingGround) -> PageTraffic {
    let description = proving_ground.description();
    let resolve = description
        .resolve
        .iter()
        .fold(Resolve::new(), |resolve, (host, address)| {
            resolve.with(host.clone(), vec![address.parse::<IpAddr>().unwrap()])
        });
    let authority = || vec![CertificateDer::from_pem_file(&description.ca).unwrap()];
    PageTraffic {
        egress: Policy::new(&["*"], &[] as &[&str])
            .unwrap()
            .with_private_ranges(&[proving_ground.address().ip().to_string()])
            .unwrap(),
        resolve,
        local: None,
        proxy: None,
        trust: Trust::default_roots()
            .with_trusted_cas("body-limits.test", authority())
            .unwrap()
            .with_trusted_cas("bombs.test", authority())
            .unwrap(),
        exceptions: CertificateExceptions::default(),
        client_certificates: ClientCertificates::default(),
        cookies: Vec::new(),
        clock: Clock::system(),
        delays: Delays::default(),
    }
}

/// The start URL of the Proving ground's task `id`.
fn start_url(proving_ground: &ProvingGround, id: &str) -> String {
    proving_ground.start_url(id).unwrap()
}

/// The protocol's setting a table's setting names.
fn setting(table: TableSetting) -> LimitSetting {
    match table {
        TableSetting::MaxBodySize => LimitSetting::MaxBodySize,
        TableSetting::MaxDecompressionRatio => LimitSetting::MaxDecompressionRatio,
        TableSetting::MaxJsonDepth => LimitSetting::MaxJsonDepth,
    }
}

/// What reading `url`'s answer under the default limits gave: its body, or
/// JSON when the case reads JSON, and the setting it passed, if any.
async fn read(traffic: &PageTraffic, url: &str, json: bool) -> Result<usize, String> {
    let mut response = traffic
        .get(url, Waiter::Call)
        .await
        .map_err(|error| format!("{error:?}"))?;
    let read = if json {
        response
            .json(url, &Limits::default())
            .await
            .map(|value| value.to_string().len())
    } else {
        response
            .body(url, &Limits::default())
            .await
            .map(|body| body.len())
    };
    read.map_err(|error| match error {
        BodyError::Limit(exceeded) => {
            format!("{}={}", exceeded.setting.setting().name, exceeded.limit)
        }
        other => format!("{other:?}"),
    })
}

/// Every case of the committed limits table that a Proving ground page
/// serves and that names a setting, read through the engine's client:
/// each is refused naming its setting and its value, every miss collected
/// before the test fails. The cases come from the table alone, so a case
/// it gains runs here too; the test only checks that every setting is
/// among them.
#[test]
fn every_page_of_the_limits_table_ends_in_response_limit_exceeded_naming_its_setting() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let traffic = traffic(&proving_ground);
    let tables = HostileTables::committed().unwrap();
    let cases: Vec<_> = tables
        .limits
        .cases
        .iter()
        .filter_map(|case| match (&case.served_by, &case.expected) {
            (ServedBy::Page(task), ExpectedError::ResponseLimitExceeded { setting }) => {
                let json = matches!(
                    case.input,
                    ResourceLimit::DeepNesting {
                        channel: JsonChannel::ResponseJson,
                        ..
                    }
                );
                Some((case.name.clone(), task.clone(), *setting, json))
            }
            _ => None,
        })
        .collect();
    let runtime = goodfetch_test_support::runtime();

    let misses: Vec<String> = runtime.block_on(async {
        let mut misses = Vec::new();
        for (name, task, table_setting, json) in &cases {
            let expected = setting(*table_setting);
            let limit = match expected {
                LimitSetting::MaxBodySize => 67_108_864,
                LimitSetting::MaxDecompressionRatio => 100,
                LimitSetting::MaxJsonDepth => 128,
            };
            let url = start_url(&proving_ground, task);
            let read = read(&traffic, &url, *json).await;
            let wanted = format!("{}={limit}", expected.setting().name);
            if read.as_ref().err() != Some(&wanted) {
                misses.push(format!("{name} ({task}): wanted {wanted}, got {read:?}"));
            }
        }
        misses
    });

    for setting in [
        TableSetting::MaxBodySize,
        TableSetting::MaxDecompressionRatio,
        TableSetting::MaxJsonDepth,
    ] {
        assert!(
            cases.iter().any(|(_, _, served, _)| *served == setting),
            "the table serves no case naming {setting:?}"
        );
    }
    assert_eq!(misses, Vec::<String>::new());
}

#[test]
fn a_normal_compressed_page_and_json_at_the_depth_are_read() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let traffic = traffic(&proving_ground);
    let safe = start_url(&proving_ground, "bombs/6");
    let at_the_depth = start_url(&proving_ground, "body-limits/5");
    let runtime = goodfetch_test_support::runtime();

    let (page, json) = runtime.block_on(async {
        let mut page = traffic.get(&safe, Waiter::Call).await.unwrap();
        let page = page.body(&safe, &Limits::default()).await.unwrap();
        let mut json = traffic.get(&at_the_depth, Waiter::Call).await.unwrap();
        let json = json.json(&at_the_depth, &Limits::default()).await.unwrap();
        (page, json)
    });

    let page = String::from_utf8(page).unwrap();
    assert!(page.contains("<article class=\"record\""), "{page}");
    let mut innermost = &json;
    while let Value::Array(items) = innermost {
        innermost = &items[0];
    }
    assert_eq!(
        innermost,
        &json!({ "id": "B-5", "text": "Nested as deep as the engine keeps." })
    );
}

/// The bytes the Proving ground sent for the requests to `host`, as its
/// connection record has them once each answer ended.
fn sent_to(proving_ground: &ProvingGround, host: &str) -> Vec<u64> {
    let observations = proving_ground.observations().unwrap();
    ReceivedRequests::of(&observations)
        .every()
        .filter(|request| {
            request["host"]
                .as_str()
                .is_some_and(|at| at.starts_with(host))
        })
        .filter_map(|request| request["delivery"]["body_sent"].as_u64())
        .collect()
}

/// Covers: setting:network.max_body_size
#[test]
fn an_endless_body_is_stopped_at_the_size_the_run_sets_holding_no_more() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let traffic = traffic(&proving_ground);
    let endless = start_url(&proving_ground, "body-limits/2");
    let limits = Limits {
        max_body_size: 1024 * 1024,
        ..Limits::default()
    };
    let runtime = goodfetch_test_support::runtime();

    let host = endless
        .trim_start_matches("https://")
        .split(':')
        .next()
        .unwrap()
        .to_owned();

    let (refused, sent) = runtime.block_on(async {
        let mut response = traffic.get(&endless, Waiter::Call).await.unwrap();
        let refused = response.body(&endless, &limits).await;
        drop(response);
        (refused, delivered(&proving_ground, &host).await)
    });

    let Err(BodyError::Limit(exceeded)) = refused else {
        panic!("expected a limit, got {refused:?}");
    };
    assert_eq!(
        (exceeded.setting, exceeded.limit),
        (LimitSetting::MaxBodySize, 1024 * 1024)
    );
    assert!(
        sent < 16 * 1024 * 1024,
        "the client stopped reading near the limit, the server sent {sent} bytes"
    );
}

/// The bytes sent for the one request to `host`, once the Proving ground
/// recorded how its answer ended.
async fn delivered(proving_ground: &ProvingGround, host: &str) -> u64 {
    for _ in 0..200 {
        if let Some(sent) = sent_to(proving_ground, host).first() {
            return *sent;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("the Proving ground recorded no delivery for {host}");
}

/// Covers: setting:network.max_decompression_ratio, setting:network.max_json_depth
#[test]
fn a_run_reads_a_bomb_under_a_raised_ratio_and_refuses_json_past_a_lowered_depth() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let traffic = traffic(&proving_ground);
    let bomb = start_url(&proving_ground, "bombs/1");
    let at_the_depth = start_url(&proving_ground, "body-limits/5");
    let set = Limits {
        max_decompression_ratio: 100_000,
        max_json_depth: 4,
        ..Limits::default()
    };
    let runtime = goodfetch_test_support::runtime();

    let (bomb, deep) = runtime.block_on(async {
        let mut response = traffic.get(&bomb, Waiter::Call).await.unwrap();
        let bomb = response.body(&bomb, &set).await.map(|body| body.len());
        let mut response = traffic.get(&at_the_depth, Waiter::Call).await.unwrap();
        let deep = response.json(&at_the_depth, &set).await;
        (bomb, deep)
    });

    assert_eq!(bomb.unwrap(), 10 * 1024 * 1024);
    let Err(BodyError::Limit(exceeded)) = deep else {
        panic!("expected a limit, got {deep:?}");
    };
    assert_eq!(
        (exceeded.setting, exceeded.limit),
        (LimitSetting::MaxJsonDepth, 4)
    );
}

#[test]
fn a_body_read_as_a_download_is_never_counted() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let traffic = traffic(&proving_ground);
    let past_the_size = start_url(&proving_ground, "body-limits/1");
    let runtime = goodfetch_test_support::runtime();

    let downloaded = runtime.block_on(async {
        let mut response: Response = traffic.get(&past_the_size, Waiter::Call).await.unwrap();
        let mut total = 0_u64;
        while let Some(bytes) = response.chunk().await.unwrap() {
            total += bytes.len() as u64;
        }
        total
    });

    assert_eq!(downloaded, MAX_BODY_SIZE + 1);
}
