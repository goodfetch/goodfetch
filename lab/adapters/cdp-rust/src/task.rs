// Port of runBrowserTask (adapters/node/browser-tasks.ts): same steps, same scripts, same metrics.
use crate::cdp::{Frame, Page, upload_file};
use crate::scripts;
use serde_json::{Map, Value, json};
use std::time::{Duration, Instant};

#[derive(Clone, Copy)]
enum Gesture<'a> {
    Click,
    DblClick,
    RightClick,
    Drag(&'a str),
    Wheel(i64),
}

async fn perform(page: &mut Page, kind: Gesture<'_>, selector: &str) -> Result<(), String> {
    match kind {
        Gesture::Click => page.click(selector).await,
        Gesture::DblClick => page.dblclick(selector).await,
        Gesture::RightClick => page.right_click(selector).await,
        Gesture::Drag(to) => page.drag(selector, to).await,
        Gesture::Wheel(delta) => page.wheel(selector, delta).await,
    }
}

// Same loop as the Node gesture helper: guard, arm the capture listener, act, disarm, retry when intercepted.
async fn gesture(page: &mut Page, selector: &str, kind: Gesture<'_>, guarded: bool, timeout: u64, operations: &mut u64) -> Result<(), String> {
    if !guarded {
        return perform(page, kind, selector).await;
    }
    let events = match kind {
        Gesture::Click => scripts::CLICK_EVENTS,
        Gesture::DblClick => scripts::DBLCLICK_EVENTS,
        Gesture::RightClick => scripts::CONTEXTMENU_EVENTS,
        Gesture::Drag(_) => scripts::DRAG_EVENTS,
        Gesture::Wheel(_) => scripts::WHEEL_EVENTS,
    };
    let deadline = Instant::now() + Duration::from_millis(timeout);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now()).as_secs_f64() * 1000.0;
        page.evaluate(&scripts::guard(selector, (remaining.round() as u64).max(50))).await?;
        *operations += 1;
        page.evaluate(&scripts::arm(selector, events)).await?;
        perform(page, kind, selector).await?;
        let intercepted = page.evaluate(scripts::DISARM.trim()).await?;
        *operations += 2;
        if intercepted.is_null() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!("gesture intercepted: {}", intercepted.as_str().unwrap_or("?")));
        }
    }
}

// The drivers' own copy of the unicode variant text (the fixture keeps the oracle's copy).
const UNICODE_TEXT: &str = "Ünïcødé — 漢字 😀";
const UPLOAD_TEXT: &str = "automan-upload\n";

fn ms(since: Instant) -> f64 {
    since.elapsed().as_secs_f64() * 1000.0
}

// Inside a popup: same readiness, guarded click and postcondition as on the main page.
async fn confirm_page(page: &mut Page, guarded: bool, timeout: u64, operations: &mut u64) -> Result<(), String> {
    page.evaluate(&scripts::wait(&scripts::condition("body-ready", &[]), timeout)).await?;
    gesture(page, "#confirm", Gesture::Click, guarded, timeout, operations).await?;
    page.evaluate(&scripts::wait(&scripts::condition("done", &[]), timeout)).await?;
    Ok(())
}

// Inside a frame: the gesture loop runs in the frame's context, input goes through the page.
async fn confirm_frame(page: &mut Page, frame: &Frame, guarded: bool, timeout: u64, operations: &mut u64) -> Result<(), String> {
    page.frame_evaluate(frame, &scripts::wait(&scripts::condition("body-ready", &[]), timeout)).await?;
    if guarded {
        let deadline = Instant::now() + Duration::from_millis(timeout);
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now()).as_secs_f64() * 1000.0;
            page.frame_evaluate(frame, &scripts::guard("#confirm", (remaining.round() as u64).max(50))).await?;
            *operations += 1;
            page.frame_evaluate(frame, &scripts::arm("#confirm", scripts::CLICK_EVENTS)).await?;
            page.frame_click(frame, "#confirm").await?;
            let intercepted = page.frame_evaluate(frame, scripts::DISARM.trim()).await?;
            *operations += 2;
            if intercepted.is_null() {
                break;
            }
            if Instant::now() >= deadline {
                return Err(format!("gesture intercepted: {}", intercepted.as_str().unwrap_or("?")));
            }
        }
    } else {
        page.frame_click(frame, "#confirm").await?;
    }
    page.frame_evaluate(frame, &scripts::wait(&scripts::condition("done", &[]), timeout)).await?;
    Ok(())
}

pub async fn run(page: &mut Page, task: &Value) -> Result<Value, String> {
    let start = Instant::now();
    let scenario = task["scenario"].as_str().unwrap_or("");
    let url = task["url"].as_str().ok_or("missing url")?.to_string();
    let timeout = task["timeoutMs"].as_u64().ok_or("missing timeoutMs")?;
    let pages = task["pages"].as_u64().unwrap_or(1);
    let actions = task["actions"].as_u64().unwrap_or(1);
    let variant = task["variant"].as_str().unwrap_or("normal");
    let guarded_policy = task["actionPolicy"] == "guarded";
    let mut operations: u64 = 0;
    let mut metrics = Map::new();

    page.goto(&url).await?;
    operations += 1;
    if scenario.ends_with("catalog") {
        let mut records: Vec<Value> = Vec::new();
        let mut next = Some(url);
        let mut visited = 0;
        while next.is_some() {
            if visited > pages {
                return Err("pagination exceeded contract".into());
            }
            visited += 1;
            page.evaluate(&scripts::wait(&scripts::condition("catalog-ready", &[]), timeout)).await?;
            operations += 1;
            if let Value::Array(found) = page.evaluate(scripts::RECORDS.trim()).await? {
                records.extend(found);
            }
            operations += 1;
            next = page.evaluate(scripts::NEXT.trim()).await?.as_str().map(str::to_string);
            operations += 1;
            if let Some(link) = &next {
                page.goto(link).await?;
                operations += 1;
            }
        }
        return Ok(json!({"status": "ok", "records": records, "metrics": {"operationCount": operations, "taskMs": ms(start)}}));
    }
    if scenario == "primitives" {
        page.evaluate(&scripts::wait(&scripts::condition("body-ready", &[]), timeout)).await?;
        let negative = variant == "retarget";
        let began = Instant::now();
        let mut native_rejected = 0;
        let attempt = async {
            let target = match variant {
                "drag" => "#source",
                "wheel" => "#scroller",
                "keyboard" | "unicode" => "#field",
                "contenteditable" => "#editor",
                "hover" | "dblclick" | "contextmenu" | "moved" | "retarget" => "#target",
                other => return Err(format!("unknown primitive {other}")),
            };
            let kind = match variant {
                "dblclick" => Some(Gesture::DblClick),
                "contextmenu" => Some(Gesture::RightClick),
                "drag" => Some(Gesture::Drag("#dropzone")),
                "wheel" => Some(Gesture::Wheel(600)),
                "moved" | "retarget" => Some(Gesture::Click),
                _ => None,
            };
            if let Some(kind) = kind {
                return gesture(page, target, kind, guarded_policy, timeout, &mut operations).await;
            }
            if guarded_policy {
                page.evaluate(&scripts::guard(target, timeout)).await?;
            }
            match variant {
                "hover" => page.hover(target).await,
                "keyboard" => page.press_enter(target).await,
                "contenteditable" => page.fill(target, "Automan").await,
                _ => page.fill(target, UNICODE_TEXT).await,
            }
        }
        .await;
        if let Err(error) = attempt {
            native_rejected = 1;
            if !negative {
                return Err(error);
            }
        }
        let action_ms = ms(began);
        if !negative {
            page.evaluate(&scripts::wait(&scripts::condition("done", &[]), timeout)).await?;
        }
        return Ok(json!({"status": "ok", "metrics": {"nativeRejected": native_rejected, "actionMs": action_ms, "taskMs": ms(start)}}));
    }
    if scenario == "navigation" {
        page.evaluate(&scripts::wait(&scripts::condition("body-ready", &[]), timeout)).await?;
        let wait_done = |target: &str| scripts::wait(&scripts::condition(target, &[]), timeout);
        let mut file: Option<String> = None;
        match variant {
            "spa" => {
                gesture(page, "#go", Gesture::Click, guarded_policy, timeout, &mut operations).await?;
                page.evaluate(&wait_done("done")).await?;
            }
            "history" => {
                page.goto(&url.replace("/ui", "/page-b")).await?;
                page.evaluate(&wait_done("page-b")).await?;
                page.go_back().await?;
                page.evaluate(&wait_done("done")).await?;
            }
            "popup" => {
                if guarded_policy {
                    page.evaluate(&scripts::guard("#open", timeout)).await?;
                }
                let mut popup = page.open_popup("#open").await?;
                let confirmed = confirm_page(&mut popup, guarded_policy, timeout, &mut operations).await;
                let closed = page.close_popup(popup).await;
                confirmed?;
                closed?;
            }
            "frame" | "cross-frame" => {
                page.evaluate(&wait_done("frame-loaded")).await?;
                let frame = page.frame("#frame").await?;
                confirm_frame(page, &frame, guarded_policy, timeout, &mut operations).await?;
            }
            "upload" => {
                let path = upload_file(UPLOAD_TEXT)?;
                if guarded_policy {
                    page.evaluate(&scripts::guard("#file", timeout)).await?;
                }
                page.upload("#file", &path).await?;
                page.evaluate(&wait_done("done")).await?;
            }
            "download" => {
                if guarded_policy {
                    page.evaluate(&scripts::guard("#file", timeout)).await?;
                }
                file = Some(page.download("#file").await?);
            }
            other => return Err(format!("unknown navigation variant {other}")),
        }
        let mut output = json!({"status": "ok", "metrics": {"taskMs": ms(start)}});
        if let Some(text) = file {
            output["file"] = json!(text);
        }
        return Ok(output);
    }
    if scenario == "download" {
        return Ok(json!({"status": "unsupported", "reason": "download events not yet qualified on this adapter"}));
    }
    if scenario == "iframe" {
        return Ok(json!({"status": "unsupported", "reason": "frame-specific adapter contract not qualified"}));
    }
    if scenario == "actionability" {
        page.evaluate(&scripts::wait(&scripts::condition("body-ready", &[]), timeout)).await?;
        let negative = ["never-enabled", "never-uncovered", "ambiguous"].contains(&variant);
        let began = Instant::now();
        let mut native_rejected = 0;
        let attempt = gesture(page, "#target", Gesture::Click, guarded_policy, timeout, &mut operations).await;
        if let Err(error) = attempt {
            native_rejected = 1;
            if !negative {
                return Err(error);
            }
        }
        let native_click_ms = ms(began);
        if !negative {
            page.evaluate(&scripts::wait(&scripts::condition("action-saved", &[]), timeout)).await?;
        }
        return Ok(json!({"status": "ok", "metrics": {"nativeRejected": native_rejected, "nativeClickMs": native_click_ms, "taskMs": ms(start)}}));
    }
    page.evaluate(&scripts::wait(&scripts::condition("ui-ready", &[]), timeout)).await?;
    operations += 1;
    let snapshot_start = Instant::now();
    let snapshot = page.evaluate(scripts::SNAPSHOT.trim()).await?;
    if snapshot["width"] != 1280 || snapshot["height"] != 720 {
        return Err(format!("viewport contract: {}x{}", snapshot["width"], snapshot["height"]));
    }
    metrics.insert("domNodes".into(), snapshot["nodes"].clone());
    metrics.insert("viewportWidth".into(), snapshot["width"].clone());
    metrics.insert("viewportHeight".into(), snapshot["height"].clone());
    metrics.insert("domCountMs".into(), json!(ms(snapshot_start)));
    operations += 1;
    if scenario == "clicks" {
        let mut click_total = 0.0;
        for i in 1..=actions {
            let click_start = Instant::now();
            gesture(page, "#counter", Gesture::Click, guarded_policy, timeout, &mut operations).await?;
            page.evaluate(&scripts::wait(&scripts::condition("count", &[("N", &i.to_string())]), timeout)).await?;
            click_total += ms(click_start);
            operations += 2;
        }
        metrics.insert("clickTotalMs".into(), json!(click_total));
    } else {
        let root = if scenario == "shadow" { "document.querySelector(\"#shadow-host\").shadowRoot" } else { "document" };
        page.evaluate(&scripts::wait(&scripts::condition("submit-enabled", &[("ROOT", root)]), timeout)).await?;
        operations += 1;
        for (selector, step) in [("#name", "fill"), ("#region", "select"), ("#consent", "click"), ("#submit", "click")] {
            if step == "click" {
                gesture(page, selector, Gesture::Click, guarded_policy, timeout, &mut operations).await?;
                continue;
            }
            if guarded_policy {
                page.evaluate(&scripts::guard(selector, timeout)).await?;
                operations += 1;
            }
            match step {
                "fill" => page.fill(selector, "Automan").await?,
                _ => page.select(selector, "eu").await?,
            }
        }
        operations += 4;
        page.evaluate(&scripts::wait(&scripts::condition("saved", &[]), timeout)).await?;
        operations += 1;
    }
    metrics.insert("operationCount".into(), json!(operations));
    metrics.insert("taskMs".into(), json!(ms(start)));
    Ok(json!({"status": "ok", "metrics": metrics}))
}
