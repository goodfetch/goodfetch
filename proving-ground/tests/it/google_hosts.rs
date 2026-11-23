//! The Google-host stand-in (#328, ADR 0074): on each Google host name a
//! test maps to the Proving ground, any path answers, and every request is
//! observed with its `X-Client-Data` header as sent, for the
//! quiet-browser test of the Goodfetch Chromium (ADR 0043).

use serde_json::{Value, json};

use crate::proving_ground::ProvingGround;
use crate::raw_clients;

const SEED: u64 = 328;

/// One host of each kind Chrome sends `X-Client-Data` to.
const GOOGLE_HOSTS: [&str; 5] = [
    "www.google.com",
    "www.youtube.com",
    "googleads.g.doubleclick.net",
    "pagead2.googlesyndication.com",
    "www.googleadservices.com",
];

/// A variations header as Chrome sends it: a base64 protobuf.
const CLIENT_DATA: &str = "CIa2yQEIpLbJAQipncoBCMSPywE=";

/// The `X-Client-Data` value each observed request to `host` carried, in
/// order, `None` for a request without it.
fn client_data(observations: &Value, host: &str) -> Vec<Option<String>> {
    observations["requests"]
        .as_array()
        .expect("the observations list requests")
        .iter()
        .filter(|request| request["host"] == host)
        .map(|request| {
            request["headers"]
                .as_array()
                .expect("a request lists its headers")
                .iter()
                .find(|header| {
                    header[0]
                        .as_str()
                        .is_some_and(|name| name.eq_ignore_ascii_case("x-client-data"))
                })
                .map(|header| header[1].as_str().expect("a header's value").to_owned())
        })
        .collect()
}

/// Covers: story:2-91
#[test]
fn every_google_host_answers_and_records_the_x_client_data_header_it_was_sent() {
    let proving_ground = ProvingGround::start(SEED);

    let statuses: Vec<_> = GOOGLE_HOSTS
        .iter()
        .map(|host| {
            let with = raw_clients::get(
                &proving_ground,
                host,
                "/generate_204",
                &[("X-Client-Data", CLIENT_DATA)],
            );
            let without = raw_clients::get(&proving_ground, host, "/", &[]);
            (*host, with.status(), without.status())
        })
        .collect();
    let observations = proving_ground.observations();

    assert_eq!(
        statuses,
        GOOGLE_HOSTS.map(|host| (host, Some(200), Some(200))),
        "every Google host answers any path"
    );
    for host in GOOGLE_HOSTS {
        assert_eq!(
            json!(client_data(&observations, host)),
            json!([CLIENT_DATA, null]),
            "{host}'s requests are observed with the header they carried"
        );
    }
}
