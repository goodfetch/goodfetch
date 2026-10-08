use serde_json::{Value, json};
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::io::{self, BufRead, Write};
use std::time::Instant;
unsafe extern "C" {
    fn lab_open(path: *const c_char, format: *const c_char, busy: c_int) -> *mut c_void;
    fn lab_begin(db: *mut c_void) -> c_int;
    fn lab_insert(db: *mut c_void, id: *const c_char, value: i64, payload: *const c_char) -> c_int;
    fn lab_commit(db: *mut c_void) -> c_int;
    fn lab_rollback(db: *mut c_void) -> c_int;
    fn lab_close(db: *mut c_void) -> c_int;
    fn lab_error(db: *mut c_void) -> *const c_char;
    fn lab_info(db: *mut c_void) -> *mut c_char;
    fn lab_free(ptr: *mut c_void);
}
fn text(v: &Value, key: &str) -> Result<CString, String> {
    CString::new(v[key].as_str().ok_or(format!("missing {key}"))?).map_err(|e| e.to_string())
}
fn error(db: *mut c_void) -> String {
    unsafe { CStr::from_ptr(lab_error(db)).to_string_lossy().into_owned() }
}
fn info(db: *mut c_void) -> Result<Value, String> {
    unsafe {
        let p = lab_info(db);
        if p.is_null() {
            return Err("SQLite metadata failed".into());
        }
        let result = serde_json::from_slice::<Value>(CStr::from_ptr(p).to_bytes())
            .map_err(|e| e.to_string());
        lab_free(p.cast());
        let mut value = result?;
        value["backend"] = json!("storage-rust-sync");
        value["binding"] = json!("synchronous Rust FFI; one common C call per inserted row");
        Ok(value)
    }
}
fn batch(db: *mut c_void, b: &Value) -> Result<Value, String> {
    if db.is_null() {
        return Err("not prepared".into());
    }
    let began = Instant::now();
    let id = b["batchId"].as_str().ok_or("missing batch ID")?;
    let count = b["rows"]
        .as_u64()
        .filter(|n| *n <= 10000)
        .ok_or("invalid rows")?;
    let seed = b["seed"].as_i64().ok_or("invalid seed")?;
    if id.contains('\0') || count == 0 {
        return Err("invalid batch".into());
    }
    let payload = "x".repeat(
        b["payloadBytes"]
            .as_u64()
            .filter(|n| *n <= 65536)
            .ok_or("invalid payload")? as usize,
    );
    let rows: Vec<Value> = (0..count)
        .map(|i| json!({"id":format!("{id}:{i}"),"value":seed+i as i64,"payload":payload}))
        .collect();
    let mut encoded = rows
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    encoded.push('\n');
    let serialization_ms = began.elapsed().as_secs_f64() * 1000.0;
    let payload_c = CString::new(payload).map_err(|e| e.to_string())?;
    unsafe {
        if lab_begin(db) != 0 {
            return Err(error(db));
        }
        for r in &rows {
            let id_c = CString::new(r["id"].as_str().unwrap()).map_err(|e| e.to_string())?;
            if lab_insert(
                db,
                id_c.as_ptr(),
                r["value"].as_i64().unwrap(),
                payload_c.as_ptr(),
            ) != 0
            {
                let message = error(db);
                lab_rollback(db);
                return Err(message);
            }
        }
        if lab_commit(db) != 0 {
            let message = error(db);
            lab_rollback(db);
            return Err(message);
        }
    }
    Ok(
        json!({"status":"ok","rows":count,"metrics":{"serializationMs":serialization_ms,"writerAckMs":began.elapsed().as_secs_f64()*1000.0,"encodedBytes":encoded.len()}}),
    )
}
fn main() {
    let mut db = std::ptr::null_mut();
    for line in io::stdin().lock().lines() {
        let line = match line {
            Ok(l) if l.len() < 128000 => l,
            _ => break,
        };
        let m: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => break,
        };
        if m["version"] != 1 || !m["id"].is_u64() {
            break;
        }
        let p = &m["params"];
        let method = m["method"].as_str().unwrap_or("");
        let result = match method {
            "hello" => info(db),
            "prepare" => {
                if !db.is_null() {
                    Err("already prepared".into())
                } else {
                    (move || {
                        let path = text(p, "path")?;
                        let format = text(p, "format")?;
                        let handle = unsafe {
                            lab_open(
                                path.as_ptr(),
                                format.as_ptr(),
                                p["busyTimeoutMs"]
                                    .as_i64()
                                    .filter(|n| *n >= 0 && *n <= 60000)
                                    .ok_or("invalid busy timeout")?
                                    as c_int,
                            )
                        };
                        if handle.is_null() {
                            Err(error(handle))
                        } else {
                            Ok(handle)
                        }
                    })()
                    .map(|handle| {
                        db = handle;
                        json!({"ready":true})
                    })
                }
            }
            "run" => batch(db, p),
            "close" => {
                if !db.is_null() {
                    unsafe {
                        lab_close(db);
                    }
                    db = std::ptr::null_mut()
                }
                Ok(json!({"closed":true}))
            }
            _ => Err("unknown method".into()),
        };
        let response = match result {
            Ok(v) => json!({"version":1,"id":m["id"],"result":v}),
            Err(e) => json!({"version":1,"id":m["id"],"error":e}),
        };
        let mut stdout = io::stdout().lock();
        writeln!(stdout, "{response}").ok();
        stdout.flush().ok();
        if method == "close" {
            break;
        }
    }
    if !db.is_null() {
        unsafe {
            lab_close(db);
        }
    }
}
