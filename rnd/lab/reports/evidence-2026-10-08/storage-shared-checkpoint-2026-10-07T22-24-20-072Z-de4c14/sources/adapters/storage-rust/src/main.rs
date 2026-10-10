use serde_json::{Value, json};
use sqlx::{
    Row, SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
};
use std::{
    io::{self, BufRead, Write},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{
    fs::{File, OpenOptions},
    io::AsyncWriteExt,
    sync::Mutex,
};

enum Sink {
    Lines(Mutex<File>, bool),
    Snapshot(String, Mutex<Vec<Value>>),
    Database(SqlitePool),
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
#[tokio::main]
async fn main() {
    let mut sink: Option<Arc<Sink>> = None;
    let mut tasks = tokio::task::JoinSet::new();
    for line in io::stdin().lock().lines() {
        let line = match line {
            Ok(line) if line.len() < 128_000 => line,
            _ => break,
        };
        let m: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(_) => break,
        };
        let id = m["id"].clone();
        if m["version"] != 1 || !id.is_u64() {
            break;
        }
        let method = m["method"].as_str().unwrap_or("");
        if method == "run" {
            while tasks.try_join_next().is_some() {}
            if tasks.len() >= 128 {
                respond(id, Err("outstanding request limit".into()));
                continue;
            }
            if let Some(state) = sink.clone() {
                let batch = m["params"].clone();
                tasks.spawn(async move {
                    respond(id, write_batch(state, batch).await);
                });
            } else {
                respond(id, Err("not prepared".into()));
            }
            continue;
        }
        let result = match method {
            "hello" => info(sink.as_deref()).await,
            "prepare" => match prepare(&m["params"]).await {
                Ok(state) => {
                    sink = Some(Arc::new(state));
                    Ok(json!({"ready":true}))
                }
                Err(error) => Err(error),
            },
            "close" => {
                while tasks.join_next().await.is_some() {}
                if let Some(Sink::Database(pool)) = sink.as_deref() {
                    pool.close().await;
                }
                sink.take();
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
async fn info(sink: Option<&Sink>) -> Result<Value, String> {
    let mut result = json!({"backend":"storage-rust","sqlx":"0.8.6","execution":"Tokio; one SQLx connection per worker process"});
    if let Some(Sink::Database(pool)) = sink {
        let row = sqlx::query("SELECT sqlite_version()")
            .fetch_one(pool)
            .await
            .map_err(|e| e.to_string())?;
        result["sqlite"] = json!(row.get::<String, _>(0));
        for key in [
            "journal_mode",
            "synchronous",
            "busy_timeout",
            "wal_autocheckpoint",
        ] {
            let row = sqlx::query(&format!("PRAGMA {key}"))
                .fetch_one(pool)
                .await
                .map_err(|e| e.to_string())?;
            result[key] = if key == "journal_mode" {
                json!(row.get::<String, _>(0))
            } else {
                json!(row.get::<i64, _>(0))
            };
        }
        let rows = sqlx::query("PRAGMA compile_options")
            .fetch_all(pool)
            .await
            .map_err(|e| e.to_string())?;
        result["compileOptions"] = json!(
            rows.into_iter()
                .map(|r| r.get::<String, _>(0))
                .collect::<Vec<_>>()
        );
    }
    Ok(result)
}
async fn prepare(p: &Value) -> Result<Sink, String> {
    let path = p["path"].as_str().ok_or("missing path")?;
    let format = p["format"].as_str().ok_or("missing format")?;
    if format.starts_with("sqlite") {
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .journal_mode(if format.contains("wal") {
                SqliteJournalMode::Wal
            } else {
                SqliteJournalMode::Delete
            })
            .synchronous(if format.ends_with("normal") {
                SqliteSynchronous::Normal
            } else {
                SqliteSynchronous::Full
            })
            .busy_timeout(Duration::from_millis(
                p["busyTimeoutMs"].as_u64().unwrap_or(5000),
            ))
            .pragma("wal_autocheckpoint", "1000");
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .map_err(|e| e.to_string())?;
        sqlx::query("CREATE TABLE IF NOT EXISTS records(id TEXT PRIMARY KEY, value INTEGER NOT NULL, payload TEXT NOT NULL)").execute(&pool).await.map_err(|e|e.to_string())?;
        Ok(Sink::Database(pool))
    } else if format.starts_with("jsonl") {
        Ok(Sink::Lines(
            Mutex::new(
                OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                    .await
                    .map_err(|e| e.to_string())?,
            ),
            format == "jsonl-durable",
        ))
    } else if format == "json-snapshot-durable" {
        let stored = match tokio::fs::read(path).await {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| e.to_string())?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e.to_string()),
        };
        Ok(Sink::Snapshot(path.into(), Mutex::new(stored)))
    } else {
        Err("unsupported storage format".into())
    }
}
async fn write_batch(sink: Arc<Sink>, b: Value) -> Result<Value, String> {
    let start = Instant::now();
    let id = b["batchId"].as_str().ok_or("missing batch ID")?;
    let count = b["rows"].as_u64().ok_or("missing rows")?;
    let seed = b["seed"].as_i64().ok_or("missing seed")?;
    let payload = "x".repeat(b["payloadBytes"].as_u64().unwrap_or(0) as usize);
    let rows: Vec<Value> = (0..count)
        .map(|i| json!({"id":format!("{id}:{i}"),"value":seed+i as i64,"payload":payload}))
        .collect();
    let mut encoded = rows
        .iter()
        .map(|r| r.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    encoded.push('\n');
    let serialization_ms = start.elapsed().as_secs_f64() * 1000.0;
    match sink.as_ref() {
        Sink::Lines(file, durable) => {
            let mut file = file.lock().await;
            file.write_all(encoded.as_bytes())
                .await
                .map_err(|e| e.to_string())?;
            file.flush().await.map_err(|e| e.to_string())?;
            if *durable {
                file.sync_all().await.map_err(|e| e.to_string())?;
            }
        }
        Sink::Snapshot(path, stored) => {
            let mut stored = stored.lock().await;
            stored.extend(rows.clone());
            let temporary = format!("{path}.{}.{id}.tmp", std::process::id());
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temporary)
                .await
                .map_err(|e| e.to_string())?;
            file.write_all(
                serde_json::to_string(&*stored)
                    .map_err(|e| e.to_string())?
                    .as_bytes(),
            )
            .await
            .map_err(|e| e.to_string())?;
            file.flush().await.map_err(|e| e.to_string())?;
            file.sync_all().await.map_err(|e| e.to_string())?;
            drop(file);
            tokio::fs::rename(&temporary, path)
                .await
                .map_err(|e| e.to_string())?;
            File::open(
                std::path::Path::new(path)
                    .parent()
                    .ok_or("missing parent")?,
            )
            .await
            .map_err(|e| e.to_string())?
            .sync_all()
            .await
            .map_err(|e| e.to_string())?;
        }
        Sink::Database(pool) => {
            let mut transaction = pool
                .begin_with("BEGIN IMMEDIATE")
                .await
                .map_err(|e| e.to_string())?;
            for row in &rows {
                sqlx::query("INSERT INTO records(id,value,payload) VALUES(?,?,?)")
                    .bind(row["id"].as_str().unwrap())
                    .bind(row["value"].as_i64().unwrap())
                    .bind(row["payload"].as_str().unwrap())
                    .execute(&mut *transaction)
                    .await
                    .map_err(|e| e.to_string())?;
            }
            transaction.commit().await.map_err(|e| e.to_string())?;
        }
    }
    Ok(
        json!({"status":"ok","rows":count,"metrics":{"serializationMs":serialization_ms,"writerAckMs":start.elapsed().as_secs_f64()*1000.0,"encodedBytes":encoded.len()}}),
    )
}
