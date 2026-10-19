//! The connection record (ADR 0067's oracle of "no connection reached the
//! target"): every connection attempted to an address the Proving ground
//! watches (its own, each forward proxy's and the target's), with the time
//! step it came at, its source and its destination, and every connection a
//! forward proxy attempted for a relay, served in `GET /observations`'s
//! `connections` and `relays`.
//! Covers: story:2-82

use serde_json::{Value, json};

use crate::clients;
use crate::proving_ground::ProvingGround;

const SEED: u64 = 311;

/// The record's connections to the listener `name`.
fn connections_to(observations: &Value, name: &str) -> Vec<Value> {
    observations["connections"]
        .as_array()
        .expect("connections")
        .iter()
        .filter(|connection| connection["listener"] == name)
        .cloned()
        .collect()
}

#[test]
fn a_connection_to_the_target_is_recorded_with_its_time_step_source_and_destination() {
    let proving_ground = ProvingGround::start(SEED);
    let target = proving_ground.target();
    let time_step = proving_ground.set_time_step(&json!(3));

    let exchange = clients::raw(&target, b"GET / HTTP/1.1\r\nHost: target\r\n\r\n");

    let connections = connections_to(&proving_ground.observations(), "target");
    assert_eq!(time_step.status, 200);
    assert_eq!(exchange.status(), 200);
    assert_eq!(connections.len(), 1, "{connections:?}");
    assert_eq!(
        json!({
            "time_step": connections[0]["time_step"],
            "source": connections[0]["source"],
            "destination": connections[0]["destination"],
            "relay": connections[0]["relay"],
        }),
        json!({
            "time_step": 3,
            "source": exchange.source,
            "destination": target,
            "relay": null,
        })
    );
}

#[test]
fn a_target_nothing_reached_has_no_connection_recorded() {
    let proving_ground = ProvingGround::start(SEED);
    let url = format!("http://catalog.test:{}/catalog/1", proving_ground.port());
    let _ = proving_ground.get(&url);

    let observations = proving_ground.observations();

    assert_eq!(connections_to(&observations, "target"), Vec::<Value>::new());
    assert_ne!(connections_to(&observations, "site"), Vec::<Value>::new());
}

#[test]
fn a_connection_a_proxy_opened_names_its_relay() {
    let proving_ground = ProvingGround::start(SEED);
    let request = format!(
        "GET http://catalog.test:{0}/catalog/1 HTTP/1.1\r\nHost: catalog.test:{0}\r\n\r\n",
        proving_ground.port()
    );

    let exchange = clients::raw(&proving_ground.proxy("open"), request.as_bytes());

    let observations = proving_ground.observations();
    let relayed = connections_to(&observations, "site");
    let to_proxy = connections_to(&observations, "open");
    assert_eq!(exchange.status(), 200);
    assert_eq!(
        json!({
            "client": to_proxy[0]["source"],
            "relay": relayed[0]["relay"],
            "relay_source": observations["relays"][0]["source"],
            "relay_connection": observations["relays"][0]["connection"],
            "request_connection": observations["requests"][0]["connection"],
        }),
        json!({
            "client": exchange.source,
            "relay": 0,
            "relay_source": relayed[0]["source"],
            "relay_connection": to_proxy[0]["id"],
            "request_connection": relayed[0]["id"],
        })
    );
}

#[test]
fn a_relay_to_an_address_where_nothing_listens_is_recorded_as_refused() {
    let proving_ground = ProvingGround::start(SEED);
    let down = proving_ground.proxy("down");
    let request = format!("GET http://{down}/ HTTP/1.1\r\nHost: {down}\r\n\r\n");

    let exchange = clients::raw(&proving_ground.proxy("open"), request.as_bytes());

    let observations = proving_ground.observations();
    assert_eq!(exchange.status(), 502);
    assert_eq!(
        json!([
            observations["relays"][0]["destination"],
            observations["relays"][0]["outcome"]
        ]),
        json!([down, "connection_refused"])
    );
}
