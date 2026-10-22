use serde::Serialize;
use serde_json::json;

use super::{Fault, ObservationLog, Operation, Record, Redacted, ValueDigest};

/// A test's own operation, carrying a security value.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct SignIn {
    token: Redacted<String>,
}

#[test]
fn a_log_writes_one_json_object_per_record_in_order() {
    let mut log = ObservationLog::<SignIn>::new();
    log.push(Record::new("engine-a", Operation::Fence { epoch: 2 }).at(1500));
    log.push(Record::new(
        "engine-a",
        Operation::Fault(Fault::Kill {
            event: "delivery.commit".to_owned(),
            occurrence: 3,
        }),
    ));
    let mut output = Vec::new();

    log.write_ndjson(&mut output).unwrap();

    let lines: Vec<serde_json::Value> = String::from_utf8(output)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        lines,
        [
            json!({ "at": 1500, "host": "engine-a", "operation": { "fence": { "epoch": 2 } } }),
            json!({
                "at": null,
                "host": "engine-a",
                "operation": { "fault": { "kill": { "event": "delivery.commit", "occurrence": 3 } } },
            }),
        ]
    );
}

#[test]
fn a_security_value_is_written_redacted() {
    let mut log = ObservationLog::new();
    log.push(Record::new(
        "client",
        Operation::Other(SignIn {
            token: Redacted::new("gf_token_5f1e".to_owned()),
        }),
    ));
    let mut output = Vec::new();

    log.write_ndjson(&mut output).unwrap();

    let line: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(line["operation"]["other"]["token"], "[redacted]");
}

#[test]
fn two_security_values_compare_equal() {
    let first = SignIn {
        token: Redacted::new("gf_token_5f1e".to_owned()),
    };
    let second = SignIn {
        token: Redacted::new("gf_token_90ab".to_owned()),
    };

    assert_eq!(first, second);
}

#[test]
fn a_value_digest_is_its_sha256() {
    let digest = ValueDigest::of("abc");

    assert_eq!(
        digest.as_str(),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

/// A crash and a power cut are faults a test records, as a kill is.
#[test]
fn a_crash_and_a_power_cut_are_recorded_as_faults() {
    let records = [
        Record::<()>::new("engine", Operation::Fault(Fault::Crash)),
        Record::<()>::new("engine", Operation::Fault(Fault::PowerCut)),
    ];

    let lines: Vec<serde_json::Value> = records
        .iter()
        .map(|record| serde_json::to_value(record).unwrap()["operation"].clone())
        .collect();

    assert_eq!(
        lines,
        [
            serde_json::json!({ "fault": "crash" }),
            serde_json::json!({ "fault": "power_cut" }),
        ]
    );
}
