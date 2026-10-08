// Port of runBrowserTask (adapters/node/browser-tasks.ts): same steps, same scripts, same metrics.
use crate::cdp::Page;
use crate::scripts;
use serde_json::{Map, Value, json};
use std::time::Instant;

fn ms(since: Instant) -> f64 {
    since.elapsed().as_secs_f64() * 1000.0
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
        let attempt = async {
            if guarded_policy {
                page.evaluate(&scripts::guard("#target", timeout)).await?;
            }
            page.click("#target").await
        }
        .await;
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
            if guarded_policy {
                page.evaluate(&scripts::guard("#counter", timeout)).await?;
                operations += 1;
            }
            page.click("#counter").await?;
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
            if guarded_policy {
                page.evaluate(&scripts::guard(selector, timeout)).await?;
                operations += 1;
            }
            match step {
                "fill" => page.fill(selector, "Automan").await?,
                "select" => page.select(selector, "eu").await?,
                _ => page.click(selector).await?,
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
