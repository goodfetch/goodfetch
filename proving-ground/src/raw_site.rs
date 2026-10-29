//! The host names the raw server answers (#162, ADRs 0118 and 0119): the
//! lab's HTTP incidents, access-policy and identity sites, ported, the
//! fetch edge pages, the content-coding pages and the body-limits pages. Each is served over TLS,
//! where the raw server reads a connection's server name to know its host
//! before its first request, and over plain HTTP/1.1, where the first
//! request's `Host` names it, for a client such as a browser that does not
//! trust the Proving ground's authority; a plain HTTP/2 connection to one
//! of them is refused with `421`.
//!
//! A request to these hosts carrying a Web Bot Auth signature is checked as
//! it arrives against the key directory its `Signature-Agent` names, and the
//! check is recorded with the request.

mod body_limits;
mod coding;
mod directory;
mod edge;
mod identity;
mod incidents;
mod policy;
mod shelf;

use std::collections::HashMap;

use crate::observation;
use crate::raw::{self, Answer, Request};
use crate::sync::Mutex;
use crate::task::{Entry, Expected, Kind, Task};

/// The families' own host names; each family's tasks are served on
/// `<ordinal>.<host>`, and the fetch edge's pages on its named hosts.
const FAMILY_HOSTS: [&str; 6] = [
    incidents::HOST,
    policy::HOST,
    identity::HOST,
    coding::HOST,
    edge::HOST,
    body_limits::HOST,
];

/// Whether the raw server answers `host` (a host name without its port).
pub(crate) fn serves(host: &str) -> bool {
    family_of(host).is_some()
}

/// Whether `host` offers HTTP/1.1 alone in its TLS handshake's ALPN.
pub(crate) fn http1_only(host: &str) -> bool {
    host == edge::HTTP1_HOST || family_of(host) == Some(body_limits::HOST)
}

/// The other host names a leaf certificate of `host` names, so a client may
/// reuse its connection for them (RFC 9113, section 9.1.1).
pub(crate) fn also_named(host: &str) -> &'static [&'static str] {
    if host == edge::HTTP2_HOST {
        &[edge::MISDIRECTED_HOST]
    } else {
        &[]
    }
}

/// The host names the raw server answers that belong to no task.
pub(crate) fn hosts() -> Vec<String> {
    FAMILY_HOSTS
        .into_iter()
        .chain(edge::NAMED_HOSTS)
        .chain(policy::NAMED_HOSTS)
        .chain(body_limits::NAMED_HOSTS)
        .map(str::to_owned)
        .collect()
}

/// The families' tasks, in the task list's order.
pub(crate) fn tasks(seed: u64, port: Option<u16>) -> Vec<Task> {
    let mut tasks = incidents::tasks(seed, port);
    tasks.extend(policy::tasks(seed, port));
    tasks.extend(identity::tasks(seed, port));
    tasks.extend(coding::tasks(port));
    tasks.extend(body_limits::tasks(port));
    tasks
}

fn family_of(host: &str) -> Option<&'static str> {
    FAMILY_HOSTS.into_iter().find(|family| {
        host == *family
            || host
                .strip_suffix(family)
                .is_some_and(|prefix| prefix.ends_with('.'))
    })
}

/// A task of `family`, served on `<ordinal>.<family_host>` over TLS, starting
/// at `path`, on `port` when `serve` lists it, or on none when `tasks` does.
fn task(
    (family, family_host): (&'static str, &str),
    ordinal: u64,
    kind: Kind,
    path: &str,
    description: &'static str,
    expected: Expected,
    port: Option<u16>,
) -> Task {
    let host = format!("{ordinal}.{family_host}");
    let origin = port.map_or_else(
        || format!("https://{host}"),
        |port| format!("https://{host}:{port}"),
    );
    Task {
        entry: Entry {
            id: format!("{family}/{ordinal}"),
            family,
            kind,
            start_url: format!("{origin}{path}"),
            description,
            account: None,
            time_steps: None,
        },
        host,
        expected,
    }
}

/// The task ordinal a task's host name `<ordinal>.<family host>` names.
fn ordinal(host: &str) -> Option<u64> {
    host.split_once('.')?.0.parse().ok()
}

/// The pages of one `serve`, with what they remember: how many times each
/// path of each host was asked for.
pub(crate) struct RawSite {
    seed: u64,
    hits: Mutex<HashMap<(String, String), u64>>,
}

impl RawSite {
    pub(crate) fn new(seed: u64) -> Self {
        Self {
            seed,
            hits: Mutex::default(),
        }
    }

    /// How many times `host` was asked for `path` before, counting this
    /// request.
    fn hit(&self, host: &str, path: &str) -> u64 {
        let mut hits = self.hits.lock();
        let count = hits.entry((host.to_owned(), path.to_owned())).or_default();
        *count += 1;
        *count - 1
    }
}

impl raw::Pages for RawSite {
    async fn answer(&self, request: &Request) -> Answer {
        if let Some(verification) = directory::verify(request).await
            && let Some(recorded) = request.recorded
        {
            request.observed.verified(recorded, verification);
        }
        let host = observation::host_name(&request.authority);
        let earlier = self.hit(&host, request.path());
        match family_of(&host) {
            Some(incidents::HOST) => incidents::answer(self.seed, &host, request, earlier),
            Some(policy::HOST) => policy::answer(self.seed, &host, request),
            Some(identity::HOST) => identity::answer(self.seed, &host, request),
            Some(coding::HOST) => coding::answer(&host, request),
            Some(body_limits::HOST) => body_limits::answer(&host, request),
            Some(_) => edge::answer(&host, request, earlier),
            None => Answer::Page(raw::Page::text(404, "not found")),
        }
    }
}
