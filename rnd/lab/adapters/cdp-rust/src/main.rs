// Rust CDP driver for the lab's JSONL adapter protocol (src/worker-server.ts is the Node counterpart).
mod browser;
mod cdp;
mod scripts;
mod task;

use browser::Browser;
use cdp::{Connection, Page};
use serde_json::{Value, json};
use std::io::Write;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, BufReader};

const CAPABILITIES: [&str; 8] = ["http-catalog", "js-catalog", "form", "clicks", "actionability", "shadow", "primitives", "navigation"];

fn respond(id: Value, result: Result<Value, String>) {
    let message = match result {
        Ok(result) => json!({"version": 1, "id": id, "result": result}),
        Err(error) => json!({"version": 1, "id": id, "error": error}),
    };
    let mut stdout = std::io::stdout().lock();
    let _ = writeln!(stdout, "{message}");
    let _ = stdout.flush();
}

async fn run(connection: Arc<Connection>, input: Value) -> Result<Value, String> {
    let timeout = input["timeoutMs"].as_u64().ok_or("missing timeoutMs")?;
    let mut page = Page::open(connection, timeout).await?;
    let output = task::run(&mut page, &input).await;
    let closed = page.close().await;
    let mut output = output?;
    closed?;
    if output["status"] == "ok" {
        output["metrics"]["cdpCommands"] = json!(page.commands);
    }
    Ok(output)
}

#[tokio::main]
async fn main() {
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut connection: Option<Arc<Connection>> = None;
    let mut browser: Option<Browser> = None;
    let mut tasks = tokio::task::JoinSet::new();
    while let Ok(Some(line)) = lines.next_line().await {
        if line.len() > 128_000 {
            break;
        }
        let Ok(request) = serde_json::from_str::<Value>(&line) else { break };
        let id = request["id"].clone();
        if request["version"] != 1 || !id.is_u64() {
            break;
        }
        let params = request["params"].clone();
        match request["method"].as_str().unwrap_or("") {
            "hello" => respond(
                id,
                Ok(json!({"backend": "rust-cdp", "protocol": 1, "capabilities": CAPABILITIES, "runtime": "rust tokio", "maxConcurrency": 16,
                    "lifecycle": "harness or own browser per block; target per task", "versions": {"cdp": "direct", "tokio-tungstenite": "0.29.0"}})),
            ),
            "prepare" => {
                let result = async {
                    let endpoint = match params["cdpEndpoint"].as_str() {
                        Some(endpoint) => endpoint.to_string(),
                        None => {
                            let executable = params["executablePath"].as_str().ok_or("executablePath or cdpEndpoint required")?;
                            let launched = Browser::launch(executable).await?;
                            let endpoint = launched.ws_endpoint.clone();
                            browser = Some(launched);
                            endpoint
                        }
                    };
                    let connected = Connection::connect(&endpoint).await?;
                    if params["insecureTls"].as_bool().unwrap_or(false) {
                        connected.send("Security.setIgnoreCertificateErrors", json!({"ignore": true}), None).await?;
                    }
                    connection = Some(connected);
                    Ok(json!({"ready": true}))
                }
                .await;
                respond(id, result);
            }
            "run" => {
                while tasks.try_join_next().is_some() {}
                match connection.clone() {
                    None => respond(id, Err("adapter not prepared".into())),
                    Some(connection) => {
                        tasks.spawn(async move { respond(id, run(connection, params).await) });
                    }
                }
            }
            "close" => {
                while tasks.join_next().await.is_some() {}
                if let Some(mut launched) = browser.take() {
                    launched.close().await;
                }
                respond(id, Ok(json!({"closed": true})));
                return;
            }
            _ => respond(id, Err("unknown method".into())),
        }
    }
    if let Some(mut launched) = browser.take() {
        launched.close().await;
    }
}
