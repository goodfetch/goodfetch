// Chrome DevTools Protocol connection and page. The command sequence per page operation is the
// contract shared with the Node reference driver (adapters/cdp-node/cdp.ts).
use crate::scripts;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
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

// Waits on a subscription taken before the triggering command, so the event cannot be missed.
async fn wait_event(mut events: broadcast::Receiver<Value>, timeout: Duration, label: &str, matches: impl Fn(&Value) -> bool) -> Result<Value, String> {
    tokio::time::timeout(timeout, async move {
        loop {
            match events.recv().await {
                Ok(event) if matches(&event) => return Ok(event),
                Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => return Err("CDP connection closed".to_string()),
            }
        }
    })
    .await
    .map_err(|_| format!("{label} timeout"))?
}

fn scratch_dir(prefix: &str) -> Result<std::path::PathBuf, String> {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let directory = std::env::temp_dir().join(format!("{prefix}-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    Ok(directory)
}

pub fn upload_file(text: &str) -> Result<String, String> {
    let path = scratch_dir("automan-upload")?.join("upload.txt");
    std::fs::write(&path, text).map_err(|error| error.to_string())?;
    Ok(path.display().to_string())
}

// A frame of a page: scripts run in its own context, input goes through the page in page coordinates.
pub struct Frame {
    session_id: String,
    context_id: Option<i64>,
    offset: (f64, f64),
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

    // Every command for this page counts, including those on its browser connection and on frame sessions.
    async fn send_on(&mut self, method: &str, params: Value, session_id: Option<&str>) -> Result<Value, String> {
        self.commands += 1;
        self.connection.send(method, params, session_id).await
    }

    pub async fn go_back(&mut self) -> Result<(), String> {
        let history = self.send("Page.getNavigationHistory", json!({})).await?;
        let index = history["currentIndex"].as_u64().ok_or("missing currentIndex")? as usize;
        let entry = index.checked_sub(1).and_then(|i| history["entries"].get(i)).ok_or("no previous history entry")?["id"].clone();
        // A back-forward cache restore fires frameNavigated without DOMContentLoaded, so wait for the main frame commit.
        let events = self.connection.subscribe();
        self.send("Page.navigateToHistoryEntry", json!({"entryId": entry})).await?;
        let session_id = self.session_id.clone();
        wait_event(events, self.timeout, "Page.frameNavigated", |event| {
            event["method"] == "Page.frameNavigated" && event["sessionId"] == session_id.as_str() && event["params"]["frame"].get("parentId").is_none()
        })
        .await?;
        Ok(())
    }

    pub async fn open_popup(&mut self, trigger: &str) -> Result<Page, String> {
        self.send_on("Target.setDiscoverTargets", json!({"discover": true}), None).await?;
        let events = self.connection.subscribe();
        self.click(trigger).await?;
        let opener = self.target_id.clone();
        let created = wait_event(events, self.timeout, "popup", |event| {
            event["method"] == "Target.targetCreated" && event["params"]["targetInfo"]["openerId"] == opener.as_str() && event["params"]["targetInfo"]["type"] == "page"
        })
        .await?;
        let target_id = created["params"]["targetInfo"]["targetId"].as_str().ok_or("missing popup targetId")?.to_string();
        let attached = self.send_on("Target.attachToTarget", json!({"targetId": target_id, "flatten": true}), None).await?;
        let session_id = attached["sessionId"].as_str().ok_or("missing popup sessionId")?.to_string();
        Ok(Page { connection: self.connection.clone(), target_id, session_id, timeout: self.timeout, commands: 0 })
    }

    pub async fn close_popup(&mut self, popup: Page) -> Result<(), String> {
        self.commands += popup.commands + 1;
        self.connection.send("Target.closeTarget", json!({"targetId": popup.target_id}), None).await?;
        Ok(())
    }

    pub async fn frame(&mut self, selector: &str) -> Result<Frame, String> {
        let offset = self.evaluate(&scripts::frame_offset(selector)).await?;
        let offset = (offset["x"].as_f64().unwrap_or(0.0), offset["y"].as_f64().unwrap_or(0.0));
        let document = self.send("DOM.getDocument", json!({"depth": 0})).await?;
        let node = self.send("DOM.querySelector", json!({"nodeId": document["root"]["nodeId"], "selector": selector})).await?;
        let described = self.send("DOM.describeNode", json!({"nodeId": node["nodeId"]})).await?;
        let frame_id = described["node"]["frameId"].as_str().ok_or_else(|| format!("no frame for {selector}"))?.to_string();
        // An in-process frame gets an isolated world; an out-of-process frame is its own target, reached by auto-attach.
        if let Ok(world) = self.send("Page.createIsolatedWorld", json!({"frameId": frame_id, "worldName": "automan"})).await {
            return Ok(Frame { session_id: self.session_id.clone(), context_id: world["executionContextId"].as_i64(), offset });
        }
        let events = self.connection.subscribe();
        self.send("Target.setAutoAttach", json!({"autoAttach": true, "waitForDebuggerOnStart": false, "flatten": true})).await?;
        let attached = wait_event(events, self.timeout, "frame attach", |event| {
            event["method"] == "Target.attachedToTarget" && event["params"]["targetInfo"]["targetId"] == frame_id.as_str()
        })
        .await?;
        let session_id = attached["params"]["sessionId"].as_str().ok_or("missing frame sessionId")?.to_string();
        Ok(Frame { session_id, context_id: None, offset })
    }

    pub async fn frame_evaluate(&mut self, frame: &Frame, expression: &str) -> Result<Value, String> {
        let mut params = json!({"expression": expression, "awaitPromise": true, "returnByValue": true});
        if let Some(context) = frame.context_id {
            params["contextId"] = json!(context);
        }
        let result = self.send_on("Runtime.evaluate", params, Some(&frame.session_id.clone())).await?;
        if let Some(details) = result.get("exceptionDetails") {
            let message = details["exception"]["description"].as_str().or(details["text"].as_str()).unwrap_or("evaluation failed");
            return Err(message.to_string());
        }
        Ok(result["result"].get("value").cloned().unwrap_or(Value::Null))
    }

    // An out-of-process frame takes input on its own session in frame coordinates: routed through the page, input to a
    // freshly created frame process can land on the parent's <iframe> element instead, with any driver.
    pub async fn frame_click(&mut self, frame: &Frame, selector: &str) -> Result<(), String> {
        let point = self.frame_evaluate(frame, &scripts::point(selector)).await?;
        let own = frame.context_id.is_none();
        let (dx, dy) = if own { (0.0, 0.0) } else { frame.offset };
        let x = json!(point["x"].as_f64().unwrap_or(0.0) + dx);
        let y = json!(point["y"].as_f64().unwrap_or(0.0) + dy);
        let session = if own { Some(frame.session_id.clone()) } else { None };
        for (kind, extra) in [
            ("mouseMoved", json!({})),
            ("mousePressed", json!({"button": "left", "buttons": 1, "clickCount": 1})),
            ("mouseReleased", json!({"button": "left", "buttons": 0, "clickCount": 1})),
        ] {
            let mut params = json!({"type": kind, "x": x, "y": y});
            if let (Some(target), Value::Object(fields)) = (params.as_object_mut(), extra) {
                target.extend(fields);
            }
            match &session {
                Some(id) => self.send_on("Input.dispatchMouseEvent", params, Some(id)).await?,
                None => self.send("Input.dispatchMouseEvent", params).await?,
            };
        }
        Ok(())
    }

    pub async fn upload(&mut self, selector: &str, path: &str) -> Result<(), String> {
        let document = self.send("DOM.getDocument", json!({"depth": 0})).await?;
        let node = self.send("DOM.querySelector", json!({"nodeId": document["root"]["nodeId"], "selector": selector})).await?;
        self.send("DOM.setFileInputFiles", json!({"files": [path], "nodeId": node["nodeId"]})).await?;
        Ok(())
    }

    pub async fn download(&mut self, selector: &str) -> Result<String, String> {
        let directory = scratch_dir("automan-download")?;
        self.send_on("Browser.setDownloadBehavior", json!({"behavior": "allowAndName", "downloadPath": directory.display().to_string(), "eventsEnabled": true}), None).await?;
        let events = self.connection.subscribe();
        self.click(selector).await?;
        let done = wait_event(events, self.timeout, "download", |event| event["method"] == "Browser.downloadProgress" && event["params"]["state"] == "completed").await?;
        let guid = done["params"]["guid"].as_str().ok_or("missing download guid")?;
        std::fs::read_to_string(directory.join(guid)).map_err(|error| error.to_string())
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

    async fn point(&mut self, selector: &str) -> Result<(Value, Value), String> {
        let point = self.evaluate(&scripts::point(selector)).await?;
        Ok((point["x"].clone(), point["y"].clone()))
    }

    async fn mouse(&mut self, kind: &str, x: &Value, y: &Value, extra: Value) -> Result<(), String> {
        let mut params = json!({"type": kind, "x": x, "y": y});
        if let (Some(target), Value::Object(fields)) = (params.as_object_mut(), extra) {
            target.extend(fields);
        }
        self.send("Input.dispatchMouseEvent", params).await?;
        Ok(())
    }

    pub async fn hover(&mut self, selector: &str) -> Result<(), String> {
        let (x, y) = self.point(selector).await?;
        self.mouse("mouseMoved", &x, &y, json!({})).await
    }

    pub async fn dblclick(&mut self, selector: &str) -> Result<(), String> {
        let (x, y) = self.point(selector).await?;
        self.mouse("mouseMoved", &x, &y, json!({})).await?;
        for click_count in [1, 2] {
            self.mouse("mousePressed", &x, &y, json!({"button": "left", "buttons": 1, "clickCount": click_count})).await?;
            self.mouse("mouseReleased", &x, &y, json!({"button": "left", "buttons": 0, "clickCount": click_count})).await?;
        }
        Ok(())
    }

    pub async fn right_click(&mut self, selector: &str) -> Result<(), String> {
        let (x, y) = self.point(selector).await?;
        self.mouse("mouseMoved", &x, &y, json!({})).await?;
        self.mouse("mousePressed", &x, &y, json!({"button": "right", "buttons": 2, "clickCount": 1})).await?;
        self.mouse("mouseReleased", &x, &y, json!({"button": "right", "buttons": 0, "clickCount": 1})).await
    }

    pub async fn drag(&mut self, from: &str, to: &str) -> Result<(), String> {
        let (sx, sy) = self.point(from).await?;
        let (ex, ey) = self.point(to).await?;
        self.mouse("mouseMoved", &sx, &sy, json!({})).await?;
        self.mouse("mousePressed", &sx, &sy, json!({"button": "left", "buttons": 1, "clickCount": 1})).await?;
        self.mouse("mouseMoved", &ex, &ey, json!({"buttons": 1})).await?;
        self.mouse("mouseReleased", &ex, &ey, json!({"button": "left", "buttons": 0, "clickCount": 1})).await
    }

    pub async fn wheel(&mut self, selector: &str, delta_y: i64) -> Result<(), String> {
        let (x, y) = self.point(selector).await?;
        self.mouse("mouseMoved", &x, &y, json!({})).await?;
        self.mouse("mouseWheel", &x, &y, json!({"deltaX": 0, "deltaY": delta_y})).await
    }

    pub async fn press_enter(&mut self, selector: &str) -> Result<(), String> {
        self.evaluate(&scripts::focus(selector)).await?;
        self.send("Input.dispatchKeyEvent", json!({"type": "keyDown", "key": "Enter", "code": "Enter", "windowsVirtualKeyCode": 13, "text": "\r"})).await?;
        self.send("Input.dispatchKeyEvent", json!({"type": "keyUp", "key": "Enter", "code": "Enter", "windowsVirtualKeyCode": 13})).await?;
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
