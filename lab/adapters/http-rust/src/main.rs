// HTTP catalog adapter for the lab's JSONL protocol, on reqwest or wreq (one per build).
// Same crawl as node-http: one GET per page, records parsed from the HTML, next link followed, no retries.
use serde_json::{Value, json};
use std::io::Write;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, BufReader};

#[cfg(feature = "reqwest")]
const STACK: &str = "reqwest";
#[cfg(feature = "wreq")]
const STACK: &str = "wreq";

#[cfg(feature = "reqwest")]
async fn fetch(url: &str, timeout: Duration, insecure: bool) -> Result<String, String> {
    let client = reqwest::Client::builder().timeout(timeout).danger_accept_invalid_certs(insecure).build().map_err(|error| error.to_string())?;
    let response = client.get(url).send().await.map_err(|error| error.to_string())?;
    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status().as_u16()));
    }
    response.text().await.map_err(|error| error.to_string())
}

#[cfg(feature = "wreq")]
async fn fetch(url: &str, timeout: Duration, insecure: bool) -> Result<String, String> {
    let client = wreq::Client::builder().timeout(timeout).redirect(wreq::redirect::Policy::limited(10)).tls_cert_verification(!insecure).build().map_err(|error| error.to_string())?;
    let response = client.get(url).send().await.map_err(|error| error.to_string())?;
    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status().as_u16()));
    }
    response.text().await.map_err(|error| error.to_string())
}

fn records(html: &str) -> Vec<Value> {
    let mut found = Vec::new();
    for part in html.split("<article data-id=\"").skip(1) {
        let Some((id, rest)) = part.split_once('"') else { continue };
        let Some(price) = rest.split("data-price=\"").nth(1).and_then(|p| p.split('"').next()).and_then(|p| p.parse::<f64>().ok()) else { continue };
        found.push(json!({"id": id, "price": price}));
    }
    found
}

fn next_link(html: &str, base: &str) -> Option<String> {
    let href = html.split("class=\"next\" href=\"").nth(1)?.split('"').next()?;
    if href.starts_with("http") {
        return Some(href.to_string());
    }
    let origin = base.splitn(4, '/').take(3).collect::<Vec<_>>().join("/");
    Some(format!("{origin}{href}"))
}

async fn run(task: Value) -> Result<Value, String> {
    if task["scenario"] != "http-catalog" {
        return Ok(json!({"status": "unsupported", "reason": "unsupported scenario"}));
    }
    let start = Instant::now();
    let timeout = Duration::from_millis(task["timeoutMs"].as_u64().unwrap_or(15000));
    let pages = task["pages"].as_u64().unwrap_or(1);
    let mut url = Some(task["url"].as_str().ok_or("missing url")?.to_string());
    let mut all = Vec::new();
    let mut visited = 0;
    while let Some(current) = url {
        if visited > pages {
            return Err("pagination exceeded contract".into());
        }
        visited += 1;
        let html = fetch(&current, timeout, task["insecureTls"].as_bool().unwrap_or(false)).await?;
        all.extend(records(&html));
        url = next_link(&html, &current);
    }
    Ok(json!({"status": "ok", "records": all, "metrics": {"taskMs": start.elapsed().as_secs_f64() * 1000.0, "pages": visited}}))
}

fn respond(id: Value, result: Result<Value, String>) {
    let message = match result {
        Ok(result) => json!({"version": 1, "id": id, "result": result}),
        Err(error) => json!({"version": 1, "id": id, "error": error}),
    };
    let mut stdout = std::io::stdout().lock();
    let _ = writeln!(stdout, "{message}");
    let _ = stdout.flush();
}

#[tokio::main]
async fn main() {
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut tasks = tokio::task::JoinSet::new();
    while let Ok(Some(line)) = lines.next_line().await {
        let Ok(request) = serde_json::from_str::<Value>(&line) else { break };
        let id = request["id"].clone();
        if request["version"] != 1 || !id.is_u64() {
            break;
        }
        match request["method"].as_str().unwrap_or("") {
            "hello" => respond(id, Ok(json!({"backend": format!("{STACK}-http"), "protocol": 1, "capabilities": ["http-catalog"], "runtime": "rust tokio", "maxConcurrency": 128, "lifecycle": "client per task, no retries", "versions": {STACK: if STACK == "reqwest" { "0.13.5" } else { "6.0.0-rc.31" }}}))),
            "prepare" => respond(id, Ok(json!({"ready": true}))),
            "run" => {
                while tasks.try_join_next().is_some() {}
                let params = request["params"].clone();
                tasks.spawn(async move { respond(id, run(params).await) });
            }
            "close" => {
                while tasks.join_next().await.is_some() {}
                respond(id, Ok(json!({"closed": true})));
                return;
            }
            _ => respond(id, Err("unknown method".into())),
        }
    }
}
