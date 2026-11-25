//! The Memory site (ADR 0125): a test adapter of the fetch tiers'
//! [`Transport`], a site served from memory through their test seam,
//! [`FetchTiers::serving`], so a crawl runs with no network. A test gives
//! it its pages, as a function of each request's URL, and may give it a
//! latency and the robots.txt of some hosts; [`settings`] are the `fetch`
//! and `network` settings a crawl of it is sent with. Only ever a
//! dev-dependency, never shipped.

use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use goodfetch_fetch_tier::{Body, FetchTiers, Hop, Reuse, Sending, Sent, Transport};
use serde_json::{Value, json};
/// The URL each page of a Memory site is asked for.
pub use url::Url;

/// The address [`settings`] maps each host of a crawl to: a public one,
/// which the egress policy lets through with nothing weakened, and which no
/// connection reaches, since the crawl's fetch tiers serve the Memory site.
pub const ADDRESS: &str = "93.184.216.34";

/// The `fetch` and `network` settings of a crawl of a Memory site: the
/// `http` tier forced, the one tier an engine with no browser serves every
/// request on, and each of `hosts` mapped to [`ADDRESS`], so no name is
/// looked up.
pub fn settings<'a>(hosts: impl IntoIterator<Item = &'a str>) -> (Value, Value) {
    (
        json!({ "tier": "http" }),
        json!({ "resolve": resolve(hosts) }),
    )
}

/// The `network.resolve` setting that maps each of `hosts` to [`ADDRESS`].
pub fn resolve<'a>(hosts: impl IntoIterator<Item = &'a str>) -> Value {
    Value::Object(
        hosts
            .into_iter()
            .map(|host| (host.to_owned(), json!([ADDRESS])))
            .collect(),
    )
}

/// One answer of the site: its status, its content type, its other headers
/// and its body, which arrives in one chunk.
pub struct Page {
    status: u16,
    content_type: &'static str,
    headers: Vec<(String, String)>,
    body: Bytes,
}

impl Page {
    /// An HTML document, answered 200.
    pub fn html(body: impl Into<Bytes>) -> Self {
        Self::new(200, "text/html; charset=utf-8", body)
    }

    /// No such page: 404, with no body.
    pub fn not_found() -> Self {
        Self::new(404, "text/html; charset=utf-8", Bytes::new())
    }

    /// An answer of `status` whose body `body` is of `content_type`.
    pub fn new(status: u16, content_type: &'static str, body: impl Into<Bytes>) -> Self {
        Self {
            status,
            content_type,
            headers: Vec::new(),
            body: body.into(),
        }
    }

    /// The same answer with the status `status`.
    #[must_use]
    pub fn with_status(self, status: u16) -> Self {
        Self { status, ..self }
    }

    /// The same answer with the header `name: value` after its content type
    /// and length, and after the headers added before it.
    #[must_use]
    pub fn with_header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }

    fn sent(self) -> Sent {
        let mut headers = vec![
            ("content-type".to_owned(), self.content_type.to_owned()),
            ("content-length".to_owned(), self.body.len().to_string()),
        ];
        headers.extend(self.headers);
        Sent {
            status: self.status,
            headers,
            body: Box::new(Whole::new(self.body)),
            reused: false,
            decoded: false,
            certificate_exception: None,
        }
    }
}

/// The site: every request is answered with the page its pages function
/// gives for its URL, but a robots.txt, which is the file given for its
/// host, else 404; each answer comes after the site's latency, or ready at
/// once when it has none.
pub struct MemorySite {
    pages: Box<dyn Fn(&Url) -> Page + Send + Sync>,
    latency: Option<Duration>,
    robots_txt: Vec<(String, &'static str)>,
}

impl MemorySite {
    /// The site whose answer to a request of `url` is `pages(url)`, called
    /// as the request is sent and ready at once, with no robots.txt.
    pub fn new(pages: impl Fn(&Url) -> Page + Send + Sync + 'static) -> Self {
        Self {
            pages: Box::new(pages),
            latency: None,
            robots_txt: Vec::new(),
        }
    }

    /// The same site, each answer `latency` after its request, on the
    /// runtime's timer: a paused clock passes it once the engine is idle.
    /// A zero latency still waits on the timer, unlike no latency.
    #[must_use]
    pub fn with_latency(self, latency: Duration) -> Self {
        Self {
            latency: Some(latency),
            ..self
        }
    }

    /// The same site, `host` answering `file` as its robots.txt.
    #[must_use]
    pub fn with_robots_txt(mut self, host: &str, file: &'static str) -> Self {
        self.robots_txt.push((host.to_owned(), file));
        self
    }

    /// Fetch tiers that serve this site on every tier.
    pub fn tiers(self) -> FetchTiers {
        FetchTiers::serving(Arc::new(self))
    }

    fn answer(&self, url: &Url) -> Page {
        if url.path() != "/robots.txt" {
            return (self.pages)(url);
        }
        let file = self
            .robots_txt
            .iter()
            .find(|(host, _)| url.host_str() == Some(host));
        match file {
            Some((_, file)) => Page::new(200, "text/plain; charset=utf-8", *file),
            None => Page::new(404, "text/plain; charset=utf-8", "not found"),
        }
    }
}

impl Transport for MemorySite {
    fn send<'a>(&'a self, hop: &'a Hop, _reuse: Reuse) -> Sending<'a, Sent> {
        let sent = self.answer(&hop.url).sent();
        match self.latency {
            None => Box::pin(std::future::ready(Ok(sent))),
            Some(latency) => Box::pin(async move {
                tokio::time::sleep(latency).await;
                Ok(sent)
            }),
        }
    }
}

/// A body sent whole in one chunk; an empty one sends none.
pub struct Whole(Option<Bytes>);

impl Whole {
    /// The body `body`.
    pub fn new(body: impl Into<Bytes>) -> Self {
        Self(Some(body.into()))
    }
}

impl Body for Whole {
    fn chunk(&mut self) -> Sending<'_, Option<Bytes>> {
        let chunk = self.0.take().filter(|chunk| !chunk.is_empty());
        Box::pin(std::future::ready(Ok(chunk)))
    }
}
