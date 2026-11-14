//! One crawl with fastCRW, for `just compare` (#2217): its server,
//! `crw-server`, started as its own process on a free loopback port with a
//! configuration file this driver writes in the trial's home (`CRW_CONFIG`,
//! the server's override file; the user's `~/.config/crw` is kept out), and
//! one crawl job started on its API (`POST /v1/crawl`) from `--url`, polled
//! until it ends (`GET /v1/crawl/{id}`), as its users drive it. Its crawl
//! follows links on the first page's origin alone, port included, to a depth
//! of at most 10 and 1,000 pages, the API's ceilings; the comparison's
//! sites of 1,000 pages fit both. Its crawl awaits each page before the
//! next (`crw-crawl`'s `run_crawl_inner`), so a job has one request in
//! flight whatever `--concurrency` says; its per-host cap and its
//! concurrency are set to it all the same. With `--host`, the crawl request
//! sends it as each page's `Host` (its `headers`, which its crawl applies to
//! every page it fetches), so the Proving ground serves its family on the
//! loopback address. With `--proxy`, the server fetches every page and
//! robots.txt through the forward proxy at that address (`crawler.proxy`),
//! which reaches a host `--url` names on the Proving ground. `--scenario`
//! names the matrix's scenario:
//!
//! - `raw`: robots.txt not obeyed, no per-host pacing
//!   (`crawler.requests_per_second` 0, its "unlimited"), `--concurrency`
//!   as its per-host cap and its concurrency, no browser (`renderer.mode`
//!   `none`), each page's links its only format;
//! - `extraction`: `raw` with each page's main content as Markdown (its
//!   `onlyMainContent`), its URL, title and Markdown written to
//!   `./pages.jsonl`;
//! - `browser`: `raw` with each page rendered in `--browser`, launched
//!   headless by this driver and given to the server as its Chrome
//!   (`renderer.mode` `chrome`, `[renderer.chrome] ws_url`), each page asked
//!   to render (`renderJs`), and its anti-bot classifier's in-loop escalation
//!   off (`[renderer.antibot] escalate_in_failover`, its kill switch): at its
//!   default, the classifier reads each of the comparison's short rendered
//!   pages as a block ("minimal_text on small page") and fails it, and after
//!   a run of such failures its breaker for the host skips rendering there;
//! - `hostile`: the hostile pages' pass at each tool's defaults (#2439):
//!   nothing set in its crawler or renderer but `--proxy`, and a crawl
//!   request of `--url` alone, so its defaults hold: robots.txt obeyed, 10
//!   requests a second and one in flight on a host, depth 2 and 100 pages a
//!   job, and its `auto` renderer with no browser configured;
//!   `--concurrency` is not read. fastCRW reads robots.txt on a client of
//!   its own, which sends none of the request's `headers`, so this pass
//!   reaches its hosts by name through `--proxy` rather than by `--host`.
//!
//! Every scenario sets the server's API rate limit off (`server.rate_limit_rps`
//! 0), which counts this driver's polls and nothing the crawl sends, and
//! `CRW_ALLOW_LOOPBACK_FOR_TESTS`, its opt-in to loopback and private
//! addresses, since the Proving ground is on the loopback address.
//!
//! The working folder is the trial's home. It prints one line, `pages=<n>`,
//! the pages fastCRW crawled, and each page it failed on stderr, with the
//! server's own log; the comparison's same-work check reads the Proving
//! ground's own observations instead, and refuses a trial that missed a page.
//!
//!   goodfetch-compare-fastcrw --server <crw-server> \
//!     --url http://127.0.0.1:<port>/sites/1000/1 --host scale.test:<port> \
//!     --concurrency 16 --scenario raw
#![expect(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "a comparison side, no Product crate: it starts its server and browser as child processes and waits on them with std's types and sleeps, and binds a std `TcpListener` to pick its server's port, outside any engine's transport (ADR 0060)"
)]

use std::io::{self, BufWriter, Write as _};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitCode, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

/// The API's ceilings, `max_pages.min(1000)` and `max_depth.min(10)`:
/// asked for, so the site, not a default, ends a crawl.
const LIMIT: u32 = 1_000;
const DEPTH: u32 = 10;

/// How often the job is asked whether it has ended.
const POLL: Duration = Duration::from_millis(25);

/// How long the server and the browser have to start listening.
const START: Duration = Duration::from_secs(30);

struct Arguments {
    server: PathBuf,
    url: String,
    host: Option<String>,
    proxy: Option<String>,
    concurrency: usize,
    scenario: Scenario,
    browser: Option<PathBuf>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Scenario {
    Raw,
    Extraction,
    Browser,
    Hostile,
}

fn arguments() -> Result<Arguments, String> {
    let usage = "usage: goodfetch-compare-fastcrw --server <crw-server> --url <url> [--host <host:port>] [--proxy <host:port>] --concurrency <n> --scenario <raw|extraction|browser|hostile> [--browser <executable>]";
    let mut server = None;
    let mut url = None;
    let mut host = None;
    let mut proxy = None;
    let mut concurrency = None;
    let mut scenario = None;
    let mut browser = None;
    let mut given = std::env::args().skip(1);
    while let Some(name) = given.next() {
        let value = given.next().ok_or(usage)?;
        match name.as_str() {
            "--server" => server = Some(PathBuf::from(value)),
            "--url" => url = Some(value),
            "--host" => host = Some(value),
            "--proxy" => proxy = Some(value),
            "--browser" => browser = Some(PathBuf::from(value)),
            "--concurrency" => {
                concurrency = Some(
                    value
                        .parse()
                        .ok()
                        .filter(|concurrency| *concurrency > 0)
                        .ok_or("--concurrency takes a count of at least 1")?,
                );
            }
            "--scenario" => {
                scenario = Some(match value.as_str() {
                    "raw" => Scenario::Raw,
                    "extraction" => Scenario::Extraction,
                    "browser" => Scenario::Browser,
                    "hostile" => Scenario::Hostile,
                    other => return Err(format!("fastCRW runs no `{other}` scenario")),
                });
            }
            _ => return Err(usage.to_owned()),
        }
    }
    let scenario = scenario.ok_or(usage)?;
    if scenario == Scenario::Browser && browser.is_none() {
        return Err("the browser scenario takes --browser".to_owned());
    }
    Ok(Arguments {
        server: server.ok_or(usage)?,
        url: url.ok_or(usage)?,
        host,
        proxy,
        concurrency: concurrency.ok_or(usage)?,
        scenario,
        browser,
    })
}

/// A child process stopped and waited for when dropped.
struct Running {
    child: Child,
}

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A port nothing listens on now, for the server.
fn free_port() -> Result<u16, String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    let port = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .port();
    Ok(port)
}

/// The server's configuration for `arguments`' scenario, as TOML.
fn configuration(arguments: &Arguments, port: u16, chrome: Option<&str>) -> String {
    let mut text = format!("[server]\nhost = \"127.0.0.1\"\nport = {port}\nrate_limit_rps = 0\n");
    text.push_str("\n[crawler]\n");
    if let Some(proxy) = &arguments.proxy {
        text.push_str(&format!("proxy = \"http://{proxy}\"\n"));
    }
    if arguments.scenario == Scenario::Hostile {
        return text;
    }
    let concurrency = arguments.concurrency;
    text.push_str(&format!(
        "respect_robots_txt = false\nrequests_per_second = 0.0\nmax_concurrency = {concurrency}\nper_host_max_concurrent = {concurrency}\n"
    ));
    match chrome {
        Some(endpoint) => text.push_str(&format!(
            "\n[renderer]\nmode = \"chrome\"\n\n[renderer.chrome]\nws_url = \"{endpoint}\"\n\n[renderer.antibot]\nescalate_in_failover = false\n"
        )),
        None => text.push_str("\n[renderer]\nmode = \"none\"\n"),
    }
    text
}

/// The crawl request for `arguments`' scenario, from `--url`, with
/// `--host` as each page's `Host` when given.
fn request(arguments: &Arguments) -> Value {
    let start = arguments.url.as_str();
    let mut request = match arguments.scenario {
        Scenario::Raw => json!({
            "url": start,
            "limit": LIMIT,
            "maxDepth": DEPTH,
            "formats": ["links"],
        }),
        Scenario::Extraction => json!({
            "url": start,
            "limit": LIMIT,
            "maxDepth": DEPTH,
            "formats": ["markdown"],
            "onlyMainContent": true,
        }),
        Scenario::Browser => json!({
            "url": start,
            "limit": LIMIT,
            "maxDepth": DEPTH,
            "formats": ["links"],
            "renderer": "chrome",
            "renderJs": true,
        }),
        Scenario::Hostile => json!({ "url": start }),
    };
    if let Some(host) = &arguments.host {
        request["headers"] = json!({ "Host": host });
    }
    request
}

/// Launches `browser` headless with its profile under the home, and returns
/// it with its browser endpoint.
fn launch(browser: &Path) -> Result<(Running, String), String> {
    let profile = std::env::current_dir()
        .map_err(|error| error.to_string())?
        .join("chrome");
    let child = Command::new(browser)
        .args([
            "--headless",
            "--no-first-run",
            "--no-default-browser-check",
            "--remote-debugging-port=0",
        ])
        .arg(format!("--user-data-dir={}", profile.display()))
        .arg("about:blank")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("starting {}: {error}", browser.display()))?;
    let mut running = Running { child };
    let file = profile.join("DevToolsActivePort");
    let endpoint = listening(&mut running, "browser", || {
        let text = std::fs::read_to_string(&file).ok()?;
        let [port, path, ..] = text.lines().collect::<Vec<_>>()[..] else {
            return None;
        };
        let port = port.parse::<u16>().ok()?;
        Some(format!("ws://127.0.0.1:{port}{path}"))
    })?;
    Ok((running, endpoint))
}

/// Waits until `ready` gives a value, failing when `running`, the `what`
/// this driver started, exits first or has not listened within [`START`].
fn listening<T>(
    running: &mut Running,
    what: &str,
    mut ready: impl FnMut() -> Option<T>,
) -> Result<T, String> {
    let started = Instant::now();
    loop {
        if let Some(value) = ready() {
            return Ok(value);
        }
        if let Some(status) = running
            .child
            .try_wait()
            .map_err(|error| error.to_string())?
        {
            return Err(format!("the {what} exited before it listened: {status}"));
        }
        if started.elapsed() >= START {
            return Err(format!("the {what} did not listen within {START:?}"));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Starts the server with `configuration`, its log on this driver's stderr,
/// and waits until its API answers.
fn serve(
    agent: &ureq::Agent,
    server: &Path,
    configuration: &str,
    api: &str,
) -> Result<Running, String> {
    let home = std::env::current_dir().map_err(|error| error.to_string())?;
    let file = home.join("crw.toml");
    std::fs::write(&file, configuration).map_err(|error| error.to_string())?;
    let mut command = Command::new(server);
    // Only what this driver sets reaches its configuration.
    for (name, _) in std::env::vars_os() {
        let text = name.to_string_lossy();
        if text.starts_with("CRW_")
            || text == "RUST_LOG"
            || text.to_ascii_lowercase().ends_with("_proxy")
        {
            command.env_remove(&name);
        }
    }
    let child = command
        .env("CRW_CONFIG", &file)
        .env("CRW_USER_CONFIG_DIR", home.join("crw-user"))
        .env("CRW_ALLOW_LOOPBACK_FOR_TESTS", "1")
        .stdin(Stdio::null())
        .stdout(io::stderr())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|error| format!("starting {}: {error}", server.display()))?;
    let mut running = Running { child };
    listening(&mut running, "server", || {
        agent.get(format!("{api}/health")).call().ok().map(drop)
    })?;
    Ok(running)
}

/// Starts the crawl job and waits for it to end; returns its pages.
fn crawl(agent: &ureq::Agent, api: &str, request: &Value) -> Result<Vec<Value>, String> {
    let started: Value = agent
        .post(format!("{api}/v1/crawl"))
        .send_json(request)
        .map_err(|error| format!("starting the crawl: {error}"))?
        .body_mut()
        .read_json()
        .map_err(|error| error.to_string())?;
    let id = started["id"]
        .as_str()
        .ok_or_else(|| format!("the crawl started with no id: {started}"))?;
    let status = format!("{api}/v1/crawl/{id}");
    loop {
        let mut job: Value = agent
            .get(&status)
            .call()
            .map_err(|error| format!("asking the crawl: {error}"))?
            .body_mut()
            .with_config()
            .limit(u64::MAX)
            .read_json()
            .map_err(|error| error.to_string())?;
        match job["status"].as_str() {
            Some("scraping") => std::thread::sleep(POLL),
            Some("completed") => {
                return match job["data"].take() {
                    Value::Array(pages) => Ok(pages),
                    _ => Err("the completed crawl holds no data".to_owned()),
                };
            }
            _ => return Err(format!("the crawl ended without completing: {job}")),
        }
    }
}

/// Prints each failed page, writes the extraction scenario's pages, and
/// returns the count of pages crawled.
fn report(scenario: Scenario, pages: &[Value]) -> Result<usize, String> {
    let mut output = if scenario == Scenario::Extraction {
        let file = std::fs::File::create("pages.jsonl").map_err(|error| error.to_string())?;
        Some(BufWriter::new(file))
    } else {
        None
    };
    let mut crawled = 0;
    for page in pages {
        let metadata = &page["metadata"];
        if !page["block"].is_null() {
            eprintln!(
                "goodfetch-compare-fastcrw: {}: {} {}",
                metadata["sourceURL"], metadata["statusCode"], page["block"]["reason"]
            );
            continue;
        }
        crawled += 1;
        if let Some(output) = &mut output {
            let line = json!({
                "url": metadata["sourceURL"],
                "title": metadata["title"],
                "text": page["markdown"],
            });
            writeln!(output, "{line}").map_err(|error| error.to_string())?;
        }
    }
    if let Some(mut output) = output {
        output.flush().map_err(|error| error.to_string())?;
    }
    Ok(crawled)
}

fn run(arguments: &Arguments) -> Result<usize, String> {
    let browser = match &arguments.browser {
        Some(browser) if arguments.scenario == Scenario::Browser => Some(launch(browser)?),
        _ => None,
    };
    let port = free_port()?;
    let configuration = configuration(
        arguments,
        port,
        browser.as_ref().map(|(_, endpoint)| endpoint.as_str()),
    );
    let api = format!("http://127.0.0.1:{port}");
    // Straight to the server, whatever proxy the environment names.
    let agent: ureq::Agent = ureq::Agent::config_builder().proxy(None).build().into();
    let server = serve(&agent, &arguments.server, &configuration, &api)?;
    let pages = crawl(&agent, &api, &request(arguments))?;
    drop(server);
    drop(browser);
    report(arguments.scenario, &pages)
}

fn main() -> ExitCode {
    match arguments().and_then(|arguments| run(&arguments)) {
        Ok(pages) => {
            println!("pages={pages}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("goodfetch-compare-fastcrw: {error}");
            ExitCode::FAILURE
        }
    }
}
