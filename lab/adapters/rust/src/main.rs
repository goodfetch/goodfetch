use scraper::{Html, Selector};
use serde_json::{Value, json};
use spider::website::Website;
use std::io::{self, BufRead, Write};

#[tokio::main]
async fn main() {
    let mode = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "spider-http".into());
    let mut tasks = tokio::task::JoinSet::new();
    for line in io::stdin().lock().lines() {
        let line = match line {
            Ok(line) if line.len() < 128_000 => line,
            _ => break,
        };
        let request: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(_) => break,
        };
        let id = request["id"].clone();
        if request["version"] != 1 || !id.is_u64() {
            break;
        }
        let method = request["method"].as_str().unwrap_or("");
        let params = &request["params"];
        if method == "run" {
            while tasks.try_join_next().is_some() {}
            if tasks.len() >= 128 {
                respond(id, Err("outstanding request limit".into()));
                continue;
            }
            let input = params.clone();
            let run_mode = mode.clone();
            tasks.spawn(async move {
                respond(id, run(&input, &run_mode).await);
            });
            continue;
        }
        let result: Result<Value, String> = match method {
            "hello" => Ok(
                json!({"backend":mode,"protocol":1,"capabilities":if mode == "spider-http" {vec!["http-catalog"]} else {vec!["http-catalog","js-catalog"]},"versions":{"spider":"2.53.9"},"chromeFeature":cfg!(any(feature="chrome",feature="smart")),"smartFeature":cfg!(feature="smart"),"lifecycle":"website per task; browser managed by Spider"}),
            ),
            "prepare" => {
                if (mode == "spider-chrome" && !cfg!(feature = "chrome"))
                    || (mode == "spider-smart" && !cfg!(feature = "smart"))
                {
                    Err("wrong compile-time feature set".into())
                } else if mode != "spider-http"
                    && (params["executablePath"].as_str().is_none()
                        || params["executablePath"].as_str()
                            != std::env::var("CHROME_BIN").ok().as_deref())
                {
                    Err("browser path must be configured at process spawn".into())
                } else {
                    Ok(json!({"ready":true}))
                }
            }
            "close" => {
                while tasks.join_next().await.is_some() {}
                Ok(json!({"closed":true}))
            }
            _ => Err("unknown method".into()),
        };
        respond(id, result);
        if method == "close" {
            break;
        }
    }
    while tasks.join_next().await.is_some() {}
}
fn respond(id: Value, result: Result<Value, String>) {
    let response = match result {
        Ok(value) => json!({"version":1,"id":id,"result":value}),
        Err(error) => json!({"version":1,"id":id,"error":error}),
    };
    let mut stdout = io::stdout().lock();
    writeln!(stdout, "{}", response).ok();
    stdout.flush().ok();
}

async fn run(params: &Value, mode: &str) -> Result<Value, String> {
    if params["scenario"] != "http-catalog"
        && (params["scenario"] != "js-catalog" || mode == "spider-http")
    {
        return Ok(json!({"status":"unsupported","reason":"unsupported crawler scenario"}));
    }
    let url = params["url"].as_str().ok_or("missing url")?;
    let start = std::time::Instant::now();
    let mut site = Website::new(url);
    site.configuration.with_concurrency_limit(Some(1));
    site.configuration.with_respect_robots_txt(params["respectRobots"].as_bool().unwrap_or(false));
    site.configuration.with_danger_accept_invalid_certs(params["insecureTls"].as_bool().unwrap_or(false));
    site.configuration
        .with_request_timeout(Some(std::time::Duration::from_millis(
            params["timeoutMs"].as_u64().unwrap_or(15000),
        )));
    site.configuration.with_limit(
        params["pages"]
            .as_u64()
            .unwrap_or(3)
            .try_into()
            .map_err(|_| "invalid page budget")?,
    );
    #[cfg(any(feature = "chrome", feature = "smart"))]
    if mode != "spider-http" {
        site.configuration.cache = false;
        site.configuration
            .with_viewport(Some(spider::configuration::Viewport {
                width: 1280,
                height: 720,
                ..Default::default()
            }));
        site.configuration
            .with_wait_for_selector(Some(spider::configuration::WaitForSelector {
                selector: "#catalog[data-ready=\"true\"]".into(),
                timeout: Some(std::time::Duration::from_secs(10)),
            }));
    }
    // Public streaming API with explicit drain: scrape_raw's completion branch can
    // finish before its buffered final page is consumed (observed by the oracle).
    let mut rx = site.subscribe(1024);
    let (done_tx, mut done_rx) = tokio::sync::oneshot::channel::<()>();
    let crawl = async {
        if mode == "spider-http" {
            site.crawl_raw().await;
        } else if mode == "spider-smart" {
            site.crawl_smart().await;
        } else {
            site.crawl().await;
        }
        let _ = done_tx.send(());
    };
    let collect = async {
        let mut pages = Vec::new();
        loop {
            tokio::select! {
                result = rx.recv() => match result { Ok(page) => pages.push(page), Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => return Err(format!("page stream lost {n} pages")), Err(_) => break },
                _ = &mut done_rx => {
                    loop { match rx.try_recv() { Ok(page) => pages.push(page), Err(tokio::sync::broadcast::error::TryRecvError::Lagged(n)) => return Err(format!("page stream lost {n} pages")), Err(_) => break } }
                    break;
                }
            }
        }
        Ok(pages)
    };
    let (_, pages) = tokio::join!(crawl, collect);
    let selector = Selector::parse("article[data-id]").map_err(|e| e.to_string())?;
    let mut records = Vec::new();
    {
        for page in pages? {
            let document = Html::parse_document(&page.get_html());
            for element in document.select(&selector) {
                let id = element.value().attr("data-id").unwrap_or("");
                let price: i64 = element
                    .value()
                    .attr("data-price")
                    .unwrap_or("")
                    .parse()
                    .map_err(|e: std::num::ParseIntError| e.to_string())?;
                records.push(json!({"id":id,"price":price}));
            }
        }
    }
    Ok(
        json!({"status":"ok","records":records,"metrics":{"taskMs":start.elapsed().as_secs_f64()*1000.0}}),
    )
}
