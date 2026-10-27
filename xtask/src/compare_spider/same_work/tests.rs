use serde_json::json;

use super::work_of;

/// The site's work is every `GET` of its pages the Proving ground answered
/// `200` on its host, a page fetched twice counted twice in requests and
/// bytes; the robots.txt, a `404`, another host and a `HEAD` are not.
#[test]
fn a_trials_work_is_its_answered_page_requests_on_the_sites_host() {
    let request = |host: &str, method: &str, target: &str, status: u16, sent: u64| json!({ "host": host, "method": method, "target": target, "status": status, "bytes": { "received": 90, "sent": sent } });
    let observations = json!({
        "requests": [
            request("scale.test:8080", "GET", "/robots.txt", 404, 50),
            request("scale.test:8080", "GET", "/sites/2/1", 200, 600),
            request("scale.test", "GET", "/sites/2/2", 200, 610),
            request("scale.test:8080", "GET", "/sites/2/2", 200, 610),
            request("scale.test:8080", "GET", "/sites/2/3", 404, 70),
            request("scale.test:8080", "HEAD", "/sites/2/1", 200, 120),
            request("catalog.test:8080", "GET", "/sites/2/1", 200, 600),
        ],
        "unrecorded_requests": 0,
    });

    let work = work_of(&observations, "scale.test", "/sites/2/").unwrap();

    assert_eq!(
        (
            work.pages.into_iter().collect::<Vec<_>>(),
            work.requests,
            work.bytes
        ),
        (
            vec!["/sites/2/1".to_owned(), "/sites/2/2".to_owned()],
            3,
            1820
        )
    );
}

#[test]
fn unrecorded_requests_make_a_trials_work_unknown() {
    let observations = json!({ "requests": [], "unrecorded_requests": 3 });

    assert!(work_of(&observations, "scale.test", "/sites/2/").is_err());
}
