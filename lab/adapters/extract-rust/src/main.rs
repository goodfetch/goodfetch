// Main-content extraction adapter for the lab's JSONL protocol. The extractor is chosen by the first argument:
// rs-trafilatura, dom-smoothie (Readability port), scraper-text (all body text, no extraction) or htmd (whole page to Markdown).
use serde_json::{Value, json};
use std::io::{BufRead, Read, Write};
use std::time::Instant;

fn read_page(path: &str) -> Result<String, String> {
    let file = std::fs::File::open(path).map_err(|error| error.to_string())?;
    let mut bytes = Vec::new();
    if path.ends_with(".gz") {
        flate2::read::GzDecoder::new(file).read_to_end(&mut bytes).map_err(|error| error.to_string())?;
    } else {
        std::io::BufReader::new(file).read_to_end(&mut bytes).map_err(|error| error.to_string())?;
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn extract(mode: &str, html: &str, url: Option<&str>) -> Result<String, String> {
    match mode {
        "rs-trafilatura" => rs_trafilatura::extract(html).map(|result| result.content_text).map_err(|error| error.to_string()),
        "dom-smoothie" => {
            let mut readability = dom_smoothie::Readability::new(html, url.filter(|u| u.starts_with("http")), None).map_err(|error| error.to_string())?;
            readability.parse().map(|article| article.text_content.to_string()).map_err(|error| error.to_string())
        }
        "scraper-text" => {
            let document = scraper::Html::parse_document(html);
            let body = scraper::Selector::parse("body").expect("valid selector");
            let root = document.select(&body).next().unwrap_or_else(|| document.root_element());
            Ok(root.text().collect::<Vec<_>>().join(" "))
        }
        "htmd" => htmd::convert(html).map_err(|error| error.to_string()),
        other => Err(format!("unknown extractor {other}")),
    }
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

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_default();
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        let Ok(request) = serde_json::from_str::<Value>(&line) else { break };
        let id = request["id"].clone();
        if request["version"] != 1 || !id.is_u64() {
            break;
        }
        match request["method"].as_str().unwrap_or("") {
            "hello" => respond(id, Ok(json!({"backend": mode, "protocol": 1, "capabilities": ["extract"], "runtime": "rust", "maxConcurrency": 1, "versions": {"rs-trafilatura": "0.2.2", "dom_smoothie": "0.18.2", "scraper": "0.24.0", "htmd": "0.5.5"}}))),
            "prepare" => respond(id, Ok(json!({"ready": true}))),
            "run" => {
                let params = &request["params"];
                let result = (|| {
                    let html = read_page(params["path"].as_str().ok_or("missing path")?)?;
                    let began = Instant::now();
                    let text = extract(&mode, &html, params["url"].as_str())?;
                    Ok(json!({"status": "ok", "text": text, "metrics": {"extractMs": began.elapsed().as_secs_f64() * 1000.0, "htmlBytes": html.len()}}))
                })();
                respond(id, result);
            }
            "close" => {
                respond(id, Ok(json!({"closed": true})));
                return;
            }
            _ => respond(id, Err("unknown method".into())),
        }
    }
}
