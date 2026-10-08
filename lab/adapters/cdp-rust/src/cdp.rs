// Chrome DevTools Protocol connection and page. The command sequence per page operation is the
// contract shared with the Node reference driver (adapters/cdp-node/cdp.ts).
use crate::scripts;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>>;

pub struct Connection {
    outgoing: mpsc::UnboundedSender<String>,
    next_id: AtomicU64,
    pending: Pending,
    events: broadcast::Sender<Value>,
}

impl Connection {
    pub async fn connect(url: &str) -> Result<Arc<Connection>, String> {
        let (socket, _) = tokio_tungstenite::connect_async(url).await.map_err(|error| format!("cannot connect to {url}: {error}"))?;
        let (mut sink, mut stream) = socket.split();
        let (outgoing, mut queue) = mpsc::unbounded_channel::<String>();
        let pending: Pending = Arc::default();
        let (events, _) = broadcast::channel(4096);
        tokio::spawn(async move {
            while let Some(text) = queue.recv().await {
                if sink.send(Message::text(text)).await.is_err() {
                    break;
                }
            }
        });
        let reader_pending = pending.clone();
        let reader_events = events.clone();
        tokio::spawn(async move {
            while let Some(Ok(message)) = stream.next().await {
                let Message::Text(text) = message else { continue };
                let Ok(value) = serde_json::from_str::<Value>(text.as_str()) else { continue };
                if let Some(id) = value["id"].as_u64() {
                    let waiter = reader_pending.lock().expect("pending lock").remove(&id);
                    if let Some(waiter) = waiter {
                        let result = match value.get("error") {
                            Some(error) => Err(error["message"].as_str().unwrap_or("CDP error").to_string()),
                            None => Ok(value.get("result").cloned().unwrap_or(Value::Null)),
                        };
                        let _ = waiter.send(result);
                    }
                } else {
                    let _ = reader_events.send(value);
                }
            }
            for (_, waiter) in reader_pending.lock().expect("pending lock").drain() {
                let _ = waiter.send(Err("CDP connection closed".into()));
            }
        });
        Ok(Arc::new(Connection { outgoing, next_id: AtomicU64::new(1), pending, events }))
    }

    pub async fn send(&self, method: &str, params: Value, session_id: Option<&str>) -> Result<Value, String> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (resolve, result) = oneshot::channel();
        self.pending.lock().expect("pending lock").insert(id, resolve);
        let mut message = json!({"id": id, "method": method, "params": params});
        if let Some(session_id) = session_id {
            message["sessionId"] = json!(session_id);
        }
        self.outgoing.send(message.to_string()).map_err(|_| "CDP connection closed".to_string())?;
        result.await.map_err(|_| "CDP connection closed".to_string())?
    }

    // Subscribed before the command that triggers the event, so the event cannot be missed.
    pub fn subscribe(&self) -> broadcast::Receiver<Value> {
        self.events.subscribe()
    }
}

pub struct Page {
    connection: Arc<Connection>,
    target_id: String,
    session_id: String,
    timeout: Duration,
    pub commands: u64,
}

impl Page {
    pub async fn open(connection: Arc<Connection>, timeout_ms: u64) -> Result<Page, String> {
        let target = connection.send("Target.createTarget", json!({"url": "about:blank"}), None).await?;
        let target_id = target["targetId"].as_str().ok_or("missing targetId")?.to_string();
        let session = connection.send("Target.attachToTarget", json!({"targetId": target_id, "flatten": true}), None).await?;
        let session_id = session["sessionId"].as_str().ok_or("missing sessionId")?.to_string();
        let mut page = Page { connection, target_id, session_id, timeout: Duration::from_millis(timeout_ms), commands: 2 };
        page.send("Page.enable", json!({})).await?;
        page.send("Emulation.setDeviceMetricsOverride", json!({"width": 1280, "height": 720, "deviceScaleFactor": 1, "mobile": false})).await?;
        Ok(page)
    }

    async fn send(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.commands += 1;
        self.connection.send(method, params, Some(&self.session_id)).await
    }

    pub async fn goto(&mut self, url: &str) -> Result<(), String> {
        let mut events = self.connection.subscribe();
        let result = self.send("Page.navigate", json!({"url": url})).await?;
        if let Some(error) = result["errorText"].as_str() {
            return Err(format!("navigation failed: {error}"));
        }
        let session_id = self.session_id.clone();
        tokio::time::timeout(self.timeout, async move {
            loop {
                match events.recv().await {
                    Ok(event) if event["method"] == "Page.domContentEventFired" && event["sessionId"] == session_id.as_str() => return Ok(()),
                    Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => return Err("CDP connection closed".to_string()),
                }
            }
        })
        .await
        .map_err(|_| "Page.domContentEventFired timeout".to_string())?
    }

    pub async fn evaluate(&mut self, expression: &str) -> Result<Value, String> {
        let result = self.send("Runtime.evaluate", json!({"expression": expression, "awaitPromise": true, "returnByValue": true})).await?;
        if let Some(details) = result.get("exceptionDetails") {
            let message = details["exception"]["description"].as_str().or(details["text"].as_str()).unwrap_or("evaluation failed");
            return Err(message.to_string());
        }
        Ok(result["result"].get("value").cloned().unwrap_or(Value::Null))
    }

    pub async fn click(&mut self, selector: &str) -> Result<(), String> {
        let point = self.evaluate(&scripts::point(selector)).await?;
        let (x, y) = (point["x"].clone(), point["y"].clone());
        self.send("Input.dispatchMouseEvent", json!({"type": "mouseMoved", "x": x, "y": y})).await?;
        self.send("Input.dispatchMouseEvent", json!({"type": "mousePressed", "x": x, "y": y, "button": "left", "buttons": 1, "clickCount": 1})).await?;
        self.send("Input.dispatchMouseEvent", json!({"type": "mouseReleased", "x": x, "y": y, "button": "left", "buttons": 0, "clickCount": 1})).await?;
        Ok(())
    }

    pub async fn fill(&mut self, selector: &str, value: &str) -> Result<(), String> {
        self.evaluate(&scripts::focus(selector)).await?;
        self.send("Input.insertText", json!({"text": value})).await?;
        Ok(())
    }

    pub async fn select(&mut self, selector: &str, value: &str) -> Result<(), String> {
        self.evaluate(&scripts::select(selector, value)).await?;
        Ok(())
    }

    pub async fn close(&mut self) -> Result<(), String> {
        self.commands += 1;
        self.connection.send("Target.closeTarget", json!({"targetId": self.target_id}), None).await?;
        Ok(())
    }
}
