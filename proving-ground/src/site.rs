//! What the Proving ground answers to one request: a page family's page on
//! that family's host names, and the Effect its form carries out, and on any
//! other host name its own task list (`GET /tasks`), oracle (`POST /oracle`),
//! time step (`PUT /time-step`), which a task's harness sets and scheduled
//! content and drifting pages follow, the `limited` proxy's hold
//! (`PUT /limited-proxy-hold`), certificate authority
//! (`GET /certificate-authority`), observations (`GET /observations`), the
//! replay proxy's refusals (`GET /replay`), the Fake model's calls
//! (`GET /model-calls`) and the revocation of the portal's sessions
//! (`POST /revoke-sessions`), which a harness asks for while a tool is
//! stopped.

use crate::interface::ROUTES;
use std::collections::BTreeMap;
use std::sync::Arc;

use hyper::body::Bytes;
use hyper::header::HeaderMap;
use hyper::http::uri::PathAndQuery;
use hyper::{Method, StatusCode, Uri};
use serde::{Deserialize, Serialize};

use crate::access;
use crate::adversarial;
use crate::agent;
use crate::browser_builds::{self, BrowserBuilds};
use crate::browser_pages;
use crate::catalog;
use crate::certificates;
use crate::clean_up;
use crate::consent;
use crate::crawl::{self, Families};
use crate::encodings;
use crate::exchange::{Reply, Request};
use crate::extraction;
use crate::files;
use crate::handmade;
use crate::hostile;
use crate::languages;
use crate::login::canaries::{self, Canaries, Plant};
use crate::login::{self, Logins};
use crate::model::{self, Answers, FakeModel, Subject};
use crate::observation::{self, Observations, Recorded, Report};
use crate::online::{self, Online};
use crate::oracle::{self, Finding, Submission};
use crate::page_load;
use crate::portal::{self, Portal};
use crate::protection;
use crate::push::Push;
use crate::raw_site;
use crate::rendered;
use crate::replay::{self, Refusals};
use crate::schedule;
use crate::task::{Task, TaskList};
use crate::version::VERSION;

/// The site of one `serve`: its seed and the port it bound.
pub(crate) struct Site {
    seed: u64,
    port: u16,
    /// The families' host names: each family's own and each of its tasks'.
    hosts: BTreeMap<String, Option<String>>,
    /// The seed's certificate authority and client certificate.
    pems: Pems,
    observations: Arc<Observations>,
    logins: Logins,
    canaries: Canaries,
    portal: Portal,
    model: FakeModel,
    refusals: Refusals,
    /// What the Crawl families saw crawls do.
    crawl: Families,
    /// The files `browser-builds.test` serves.
    builds: BrowserBuilds,
    /// The online copy's vendors, which `challenges.test` carries; `None`
    /// on the local copy.
    online: Option<Online>,
}

/// The Proving ground's time step, as `PUT /time-step` sets it.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct TimeStep {
    step: u64,
}

/// Whether the `limited` proxy holds the requests it counts, as
/// `PUT /limited-proxy-hold` sets it.
#[derive(Serialize, Deserialize)]
struct LimitedHold {
    held: bool,
}

/// The families' own host names, each family's tasks served on
/// `<ordinal>.<host>`.
const FAMILY_HOSTS: [&str; 4] = [catalog::HOST, rendered::HOST, login::HOST, schedule::HOST];

/// What the site hands a client of the seed's certificates, each as PEM.
pub(crate) struct Pems {
    /// The certificate authority every leaf chains to.
    pub(crate) certificate_authority: String,
    /// The client certificate the hosts that require mutual TLS accept.
    pub(crate) client_certificate: String,
    /// Its private key, PKCS #8.
    pub(crate) client_key: String,
}

impl Site {
    pub(crate) fn new(
        seed: u64,
        port: u16,
        pems: Pems,
        observations: Arc<Observations>,
        builds: BrowserBuilds,
        online: Option<Online>,
    ) -> Self {
        Self {
            seed,
            port,
            hosts: Self::hosts(seed),
            pems,
            observations,
            logins: Logins::default(),
            canaries: Canaries::default(),
            portal: Portal::new(seed),
            model: FakeModel::new(seed),
            refusals: Refusals::default(),
            crawl: Families::new(),
            builds,
            online,
        }
    }

    /// Every host name the site serves, in order.
    pub(crate) fn host_names(&self) -> impl Iterator<Item = &str> {
        self.hosts.keys().map(String::as_str)
    }

    /// The seed's certificate authority, as PEM.
    pub(crate) fn certificate_authority(&self) -> &str {
        &self.pems.certificate_authority
    }

    /// Every host name a page family is served on, each with the task its
    /// requests belong to, if any: the family's own, and each task's. Each
    /// has its own leaf certificate, and only these are observed.
    pub(crate) fn hosts(seed: u64) -> BTreeMap<String, Option<String>> {
        let mut hosts: BTreeMap<_, _> = FAMILY_HOSTS
            .into_iter()
            .chain([
                agent::HOST,
                browser_builds::HOST,
                model::HOST,
                encodings::HOST,
                extraction::HOST,
                files::HOST,
                languages::HOST,
                clean_up::HOST,
                clean_up::FRAME_HOST,
                clean_up::PAYMENT_HOST,
                consent::HOST,
                consent::FRAME_HOST,
                page_load::HOST,
                online::HOST,
            ])
            .chain(page_load::AD_HOSTS)
            .map(|host| (host.to_owned(), None))
            .collect();
        hosts.extend(
            Self::every_task(seed, None)
                .into_iter()
                .map(|task| (task.host, Some(task.entry.id))),
        );
        hosts.extend(
            portal::hosts()
                .into_iter()
                .chain(handmade::hosts())
                .map(|(host, task)| (host, Some(task))),
        );
        hosts.extend(adversarial::hosts());
        hosts.extend(protection::hosts());
        hosts.extend(access::hosts());
        hosts.extend(browser_pages::hosts());
        hosts.extend(crawl::hosts(seed));
        hosts.extend(raw_site::hosts().into_iter().map(|host| (host, None)));
        hosts.extend(hostile::hosts());
        hosts.extend(canaries::hosts());
        hosts.extend(certificates::hosts());
        hosts
    }

    /// Every family's tasks, as this `serve` lists them.
    fn tasks(&self) -> Vec<Task> {
        Self::every_task(self.seed, Some(self.port))
    }

    /// Every family's tasks for `seed`, in the task list's order, their start
    /// URLs on `port`, or on none when listed without serving.
    pub(crate) fn every_task(seed: u64, port: Option<u16>) -> Vec<Task> {
        let mut tasks = catalog::tasks(seed, port);
        tasks.extend(rendered::tasks(seed, port));
        tasks.extend(login::tasks(seed, port));
        tasks.extend(schedule::tasks(seed, port));
        tasks.extend(portal::tasks(seed, port));
        tasks.extend(handmade::tasks(seed, port));
        tasks.extend(agent::tasks(seed, port));
        tasks.extend(replay::tasks());
        tasks.extend(extraction::tasks(seed, port));
        tasks.extend(encodings::tasks(seed, port));
        tasks.extend(languages::tasks(seed, port));
        tasks.extend(files::tasks(seed, port));
        tasks.extend(adversarial::tasks(seed, port));
        tasks.extend(protection::tasks(seed, port));
        tasks.extend(access::tasks(seed, port));
        tasks.extend(browser_pages::tasks(seed, port));
        tasks.extend(crawl::tasks(seed, port));
        tasks.extend(raw_site::tasks(seed, port));
        tasks.extend(hostile::tasks(seed, port));
        tasks.extend(canaries::tasks(seed, port));
        tasks
    }

    /// The reply of an Extraction family (#165) to a request on `name`, one
    /// of its host names; `None` when `name` is another family's.
    fn extraction(&self, name: &str, request: &Request<'_>) -> Option<Reply> {
        let (method, path, query) = (request.method, request.path, request.query);
        let of = |family: &str| name == family || name.ends_with(&format!(".{family}"));
        let ok = |(content_type, body)| (StatusCode::OK, content_type, body);
        let html = |page: String| ok(("text/html; charset=utf-8", page.into_bytes()));
        let page = if of(extraction::HOST) {
            if let Some(location) = extraction::moved(path).filter(|_| *method == Method::GET) {
                return Some(Reply::found(&location));
            }
            extraction::page(self.seed, path).map(ok)
        } else if of(encodings::HOST) {
            encodings::page(self.seed, path).map(ok)
        } else if of(languages::HOST) {
            languages::page(self.seed, name, path, query).map(html)
        } else if of(files::HOST) {
            files::page(self.seed, path).map(|file| (file.status, file.content_type, file.body))
        } else {
            return None;
        };
        Some(match (method, page) {
            (&Method::GET, Some((status, content_type, body))) => Reply {
                status,
                content_type,
                headers: Vec::new(),
                body: body.into(),
            },
            _ => Reply::not_found(),
        })
    }

    /// Answers one request as [`Self::respond`] does, with how long the
    /// server holds the reply first, which only a Crawl family asks for.
    pub(crate) fn respond_held(
        &self,
        host: &str,
        request: &Request<'_>,
    ) -> (Reply, Option<crawl::Held>) {
        let name = observation::host_name(host);
        if self.hosts.contains_key(&name) {
            let visit = crawl::Request {
                method: request.method,
                host: &name,
                authority: host,
                path: request.path,
                query: request.query,
                cookies: request.cookies,
                peer: request.peer,
                tls: request.tls,
            };
            if let Some(page) = self.crawl.respond(self.seed, &visit, &self.observations) {
                return page.into_reply();
            }
        }
        let held = browser_pages::hold(&name, request.path)
            .or_else(|| access::hold(&name, request.path))
            .map(crawl::Held::after);
        (self.respond(host, request), held)
    }

    /// The connection a request asks the server to hold open, given its
    /// `Host` header: one of the `live` family's WebSocket and server-sent
    /// events stream, or the `access` family's page events WebSocket.
    pub(crate) fn push(host: &str, method: &Method, path: &str) -> Option<Push> {
        let name = observation::host_name(host);
        browser_pages::push(&name, method, path).or_else(|| access::push(&name, method, path))
    }

    /// The whole response, as bytes, of a request to `path` on `host` (its
    /// `Host` header) whose head hyper cannot write; `None` for any other
    /// request.
    pub(crate) fn raw_response(&self, host: &str, path: &str) -> Option<Vec<u8>> {
        hostile::raw_response(self.seed, &observation::host_name(host), path)
    }

    /// Whether a TLS connection to `name`, a host name without its port, is
    /// offered HTTP/1.1 alone, so a page whose head hyper cannot write is
    /// reached over it.
    pub(crate) fn http1_only(name: &str) -> bool {
        hostile::http1_only(name)
    }

    /// Answers one request, given its `Host` header.
    pub(crate) fn respond(&self, host: &str, request: &Request<'_>) -> Reply {
        let name = observation::host_name(host);
        if raw_site::serves(&name) {
            // The raw server answers these over TLS and plain HTTP/1.1,
            // never over plain HTTP/2.
            return Reply::error(StatusCode::MISDIRECTED_REQUEST, "tls_required");
        }
        if let Some(reply) = self.builds.respond(&name, request) {
            return reply;
        }
        if let Some((reply, recorded)) = browser_pages::respond(self.seed, &name, request) {
            if let Some(recorded) = recorded {
                self.observations.record(host, recorded);
            }
            return reply;
        }
        if let Some((reply, recorded)) = adversarial::respond(self.seed, host, &name, request)
            .or_else(|| protection::respond(self.seed, &name, request))
            .or_else(|| access::respond(self.seed, &name, request))
            .or_else(|| hostile::respond(self.seed, self.port, &name, request))
        {
            if let Some(recorded) = recorded {
                self.observations.record(host, recorded);
            }
            return reply;
        }
        if let Some((reply, effect)) = self
            .portal
            .respond(&name, request)
            .or_else(|| handmade::respond(self.seed, self.port, &name, request))
        {
            if let Some(effect) = effect {
                self.observations.effect(host, effect);
            }
            return reply;
        }
        if let Some(reply) = canaries::respond(self.seed, &self.canaries, &name, request) {
            return reply;
        }
        if let Some(reply) = certificates::respond(&name, request.path) {
            return reply;
        }
        if let Some(reply) = clean_up::respond(&name, request) {
            return reply;
        }
        if let Some(reply) = consent::respond(&name, request) {
            return reply;
        }
        if let Some(reply) = page_load::respond(self.port, &name, request) {
            return reply;
        }
        if name == model::HOST || name == online::HOST {
            // The Fake model answers its own paths (`Self::model`), and the
            // online copy its vendors' (`Self::challenge`).
            return Reply::not_found();
        }
        if replay::serves(&name) {
            return self.replay(request.method, &name, request.path);
        }
        if self.hosts.contains_key(&name) && agent::serves(&name) {
            return self.agent(host, &name, request);
        }
        if self.hosts.contains_key(&name) {
            if let Some(reply) = self.extraction(&name, request) {
                return reply;
            }
            let family = FAMILY_HOSTS
                .into_iter()
                .find(|family| {
                    name == *family
                        || name
                            .strip_suffix(family)
                            .is_some_and(|prefix| prefix.ends_with('.'))
                })
                .expect("invariant: every served host name is a family's or its task's");
            return match family {
                catalog::HOST => self.catalog(host, request),
                rendered::HOST => rendered::respond(self.seed, request),
                login::HOST => login::respond(self.seed, &self.logins, request),
                _ => schedule::respond(self.seed, self.observations.time_step(), request),
            };
        }
        self.control(request)
    }

    /// Answers a request on a host name no page family is served on, such
    /// as the bound address: the landing of a redirect to the Proving
    /// ground's own address, or one of its own paths.
    fn control(&self, request: &Request<'_>) -> Reply {
        if request.method == Method::GET
            && let Some((reply, task_host, report)) = hostile::private_landing(request.path)
        {
            self.observations
                .record(&task_host, Recorded::Report(report));
            return reply;
        }
        match (request.method, request.path) {
            (&Method::GET, path) if path == ROUTES.tasks => Reply::json(
                StatusCode::OK,
                &TaskList::new(VERSION, self.seed, &self.tasks()),
            ),
            (&Method::POST, path) if path == ROUTES.oracle => self.judge(request.body),
            (&Method::PUT, path) if path == ROUTES.time_step => self.set_time_step(request.body),
            (&Method::PUT, "/limited-proxy-hold") => self.set_limited_hold(request.body),
            (&Method::POST, "/canaries") => self.plant(request),
            (&Method::GET, "/canaries") => self.canaries.document(),
            (&Method::POST, "/revoke-sessions") => {
                self.portal.revoke();
                Reply::no_content()
            }
            (&Method::GET, "/replay") => Reply {
                status: StatusCode::OK,
                content_type: "application/json",
                headers: Vec::new(),
                body: self.refusals.document().into(),
            },
            (&Method::GET, "/model-calls") => Reply {
                status: StatusCode::OK,
                content_type: "application/json",
                headers: Vec::new(),
                body: self.model.document().into(),
            },
            (&Method::GET, path) if path == ROUTES.observations => Reply {
                status: StatusCode::OK,
                content_type: "application/json",
                headers: Vec::new(),
                body: self.observations.document().into(),
            },
            (&Method::GET, "/request-count") => Reply::json(
                StatusCode::OK,
                &serde_json::json!({ "requests": self.observations.request_count() }),
            ),
            (&Method::GET, "/requests-of-the-case") => match request.parameter("host") {
                Some(host) => Reply::json(
                    StatusCode::OK,
                    &serde_json::json!({
                        "requests": self.observations.requests_of_the_case(&host)
                    }),
                ),
                None => Reply::error(StatusCode::BAD_REQUEST, "malformed_requests_of_the_case"),
            },
            (&Method::GET, path)
                if let Some(size) = path
                    .strip_prefix(ROUTES.article_answers)
                    .and_then(|rest| rest.strip_prefix('/')) =>
            {
                match crate::crawl::article_answers(self.seed, size) {
                    Some(answers) => Reply::json(StatusCode::OK, &answers),
                    None => Reply::not_found(),
                }
            }
            (&Method::GET, "/network-fixtures") => match self.observations.network_fixtures() {
                Some(fixtures) => Reply::json(StatusCode::OK, &fixtures),
                None => Reply::not_found(),
            },
            (&Method::GET, "/certificate-authority") => Reply {
                status: StatusCode::OK,
                content_type: "application/x-pem-file",
                headers: Vec::new(),
                body: self.pems.certificate_authority.clone().into(),
            },
            (&Method::GET, path) if path == ROUTES.client_certificate => Reply {
                status: StatusCode::OK,
                content_type: "application/x-pem-file",
                headers: Vec::new(),
                body: self.pems.client_certificate.clone().into(),
            },
            (&Method::GET, path) if path == ROUTES.client_key => Reply {
                status: StatusCode::OK,
                content_type: "application/x-pem-file",
                headers: Vec::new(),
                body: self.pems.client_key.clone().into(),
            },
            _ => Reply::not_found(),
        }
    }

    /// Plants a Secret canary for a test on a loopback address, never for a
    /// request from elsewhere, so the online copy never hands one out.
    fn plant(&self, request: &Request<'_>) -> Reply {
        if !request.peer.is_loopback() {
            return Reply::error(StatusCode::FORBIDDEN, "canary_plant_refused");
        }
        match serde_json::from_slice::<Plant>(request.body) {
            Ok(plant) => self.canaries.plant(self.seed, &plant),
            Err(_) => Reply::error(StatusCode::BAD_REQUEST, "malformed_plant"),
        }
    }

    /// Records each planted Secret canary a request carries in its target,
    /// a header or its body, and a sighting outside its flow as a leak
    /// against the canary's task.
    pub(crate) fn sight_canaries<'a>(
        &self,
        host: &str,
        target: &'a str,
        headers: impl Iterator<Item = (&'a str, &'a [u8])>,
        body: &'a [u8],
    ) {
        let name = observation::host_name(host);
        let parts = canaries::parts(target, headers, body);
        for leak in self.canaries.sight(&name, target, &parts) {
            self.observations.record(
                &leak.task_host,
                Recorded::Report(Report {
                    page: leak.target,
                    check: "secret_canary_leaked".to_owned(),
                    observed: leak.observed,
                }),
            );
        }
    }

    /// Answers a request to the catalog family, recording the Effect of an
    /// order for the request's task.
    fn catalog(&self, host: &str, request: &Request<'_>) -> Reply {
        match (request.method, request.path) {
            (&Method::POST, "/orders") => match catalog::order(request.body) {
                Some((effect, page)) => {
                    self.observations.effect(host, effect);
                    Reply::html(page)
                }
                None => Reply::error(StatusCode::BAD_REQUEST, "unknown_product"),
            },
            (&Method::GET, path) => {
                catalog::page(self.seed, path).map_or_else(Reply::not_found, Reply::html)
            }
            _ => Reply::not_found(),
        }
    }

    /// Answers a request to the `agent` family: its pages at the current
    /// time step, and the Effects their script posts, recorded with it.
    fn agent(&self, host: &str, name: &str, request: &Request<'_>) -> Reply {
        let step = self.observations.time_step();
        match (request.method, request.path) {
            (&Method::POST, agent::EFFECTS_PATH) => match agent::effect(request.body, step) {
                Some(effect) => {
                    self.observations.effect(host, effect);
                    Reply::json(StatusCode::OK, &serde_json::json!({ "time_step": step }))
                }
                None => Reply::error(StatusCode::BAD_REQUEST, "malformed_effect"),
            },
            (&Method::GET, path) => {
                agent::page(self.seed, name, step, path).map_or_else(Reply::not_found, Reply::html)
            }
            _ => Reply::not_found(),
        }
    }

    /// The Fake model's answer to a `POST` on its host name to `target`
    /// (path and query); `None` for any other request, which [`Self::respond`]
    /// answers.
    pub(crate) fn model(
        &self,
        method: &Method,
        host: &str,
        target: &str,
        body: &Bytes,
    ) -> Option<model::Answer> {
        if method != Method::POST || observation::host_name(host) != model::HOST {
            return None;
        }
        let step = self.observations.time_step();
        let subjects: Vec<Subject> = self
            .tasks()
            .into_iter()
            .map(|task| {
                let id = task.entry.id;
                let records = agent::answer(self.seed, &id, step).unwrap_or_else(|| {
                    let effects = task.expected.effects.iter().map(serde_json::to_value);
                    task.expected
                        .records
                        .iter()
                        .map(serde_json::to_value)
                        .chain(effects)
                        .filter_map(|value| match value {
                            Ok(serde_json::Value::Object(fields)) => Some(fields),
                            _ => None,
                        })
                        .collect()
                });
                Subject {
                    fault: agent::fault(self.seed, &id),
                    answers: Answers {
                        records,
                        verbs: agent::verbs(self.seed, &id, step),
                    },
                    id,
                    description: task.entry.description,
                    host: task.host,
                }
            })
            .collect();
        self.model.answer(target, body, &subjects)
    }

    /// The online copy's answer to a request on `challenges.test` to
    /// `path`, recording the vendor's verdict it read; `None` on the local
    /// copy, for another host or a path of no configured vendor, which
    /// [`Self::respond`] answers.
    pub(crate) async fn challenge(
        &self,
        method: &Method,
        host: &str,
        path: &str,
        headers: &HeaderMap,
        body: &Bytes,
    ) -> Option<Reply> {
        let online = self.online.as_ref()?;
        if observation::host_name(host) != online::HOST {
            return None;
        }
        let (reply, verdict) = online.respond(method, path, headers, body).await?;
        if let Some(verdict) = verdict {
            self.observations.record(host, Recorded::Verdict(verdict));
        }
        Some(reply)
    }

    /// Answers a request to an archived host on its own name, as through a
    /// tunnel: its archived page, else a refusal it records.
    fn replay(&self, method: &Method, host: &str, path: &str) -> Reply {
        self.archived(method, host, path, || format!("{host}{path}"))
    }

    /// Answers a request in absolute form, as a browser sends one to its
    /// proxy: an archived page, else a refusal it records.
    pub(crate) fn proxy(&self, method: &Method, uri: &Uri) -> Reply {
        let host = uri.host().unwrap_or_default();
        let target = uri.path_and_query().map_or("/", PathAndQuery::as_str);
        self.archived(method, host, target, || uri.to_string())
    }

    /// The archived page a `GET` to `host` and `target` asks for; else a
    /// refusal of the request `named`, recorded.
    fn archived(
        &self,
        method: &Method,
        host: &str,
        target: &str,
        named: impl FnOnce() -> String,
    ) -> Reply {
        if method == Method::GET
            && let Some(page) = replay::page(host, target)
        {
            return Reply::html(page.to_owned());
        }
        self.refusals.record(method.as_str(), named());
        Reply::error(StatusCode::NOT_FOUND, "not_in_archive")
    }

    /// Whether a `CONNECT` to `authority` may open its tunnel: only to an
    /// archived host's port 443. A refused one is recorded.
    pub(crate) fn tunnel(&self, authority: &str) -> bool {
        let open = replay::tunnels(authority);
        if !open {
            self.refusals.record("CONNECT", authority.to_owned());
        }
        open
    }

    /// Sets the time step scheduled content and drifting pages follow, which
    /// the connection record stamps each later entry with.
    fn set_time_step(&self, body: &Bytes) -> Reply {
        let Ok(time_step) = serde_json::from_slice::<TimeStep>(body) else {
            return Reply::error(StatusCode::BAD_REQUEST, "malformed_time_step");
        };
        self.observations.set_time_step(time_step.step);
        Reply::json(StatusCode::OK, &time_step)
    }

    /// Sets whether the `limited` proxy holds each request it counts until
    /// released, which a harness turns on to send two at once and off to
    /// let the held one go on.
    fn set_limited_hold(&self, body: &Bytes) -> Reply {
        let Ok(hold) = serde_json::from_slice::<LimitedHold>(body) else {
            return Reply::error(StatusCode::BAD_REQUEST, "malformed_limited_proxy_hold");
        };
        self.observations.set_limited_hold(hold.held);
        Reply::json(StatusCode::OK, &hold)
    }

    /// Judges a submission against its task's expected result.
    fn judge(&self, body: &Bytes) -> Reply {
        let Ok(submission) = serde_json::from_slice::<Submission>(body) else {
            return Reply::error(StatusCode::BAD_REQUEST, "malformed_submission");
        };
        let tasks = self.tasks();
        match tasks.iter().find(|task| task.entry.id == submission.task) {
            Some(task) => {
                let effects = self.observations.effects_of(&task.entry.id);
                let findings = self.observations.findings_of(&task.entry.id);
                let submitted = submission.record_ids();
                let requests = self.observations.requests_of(&task.entry.id);
                let now = self
                    .portal
                    .expected_now(&task.entry.id)
                    .or_else(|| access::expected_now(&task.entry.id, &self.observations));
                let expected = now.as_ref().unwrap_or(&task.expected);
                let answer = oracle::judge(expected, submission, &effects, &requests, findings)
                    .refuted_by(self.reported(&task.entry.id));
                let answer = answer.refuted_by(browser_pages::findings(&task.entry.id, &requests));
                if task.entry.family == access::FAMILY {
                    let answer = access::judge(&task.entry.id, answer, &self.observations);
                    return Reply::json(StatusCode::OK, &answer);
                }
                if task.entry.id == protection::TIER_MOVE_TASK {
                    let answer =
                        protection::judge_tier_move(answer, &submitted, &self.observations);
                    return Reply::json(StatusCode::OK, &answer);
                }
                Reply::json(StatusCode::OK, &answer)
            }
            None => Reply::error(StatusCode::NOT_FOUND, "unknown_task"),
        }
    }

    /// What the pages of `task` reported beyond having run: each a finding,
    /// since a page's script reports only what must not have worked.
    fn reported(&self, task: &str) -> Vec<Finding> {
        self.observations
            .reports_of(task)
            .into_iter()
            .filter(|report| report.check != observation::RAN)
            .map(|report| Finding {
                check: report.check,
                at: report.page,
                expected: None,
                observed: report.observed.into(),
            })
            .collect()
    }
}
