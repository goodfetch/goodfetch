use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::io::{self, BufRead, Write};
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

unsafe extern "C" {
    fn lab_open(path: *const c_char, format: *const c_char, busy: c_int) -> *mut c_void;
    fn lab_exec(db: *mut c_void, sql: *const c_char) -> c_int;
    fn lab_query(db: *mut c_void, sql: *const c_char, params: *const c_char) -> *mut c_char;
    fn lab_info(db: *mut c_void) -> *mut c_char;
    fn lab_error(db: *mut c_void) -> *const c_char;
    fn lab_free(p: *mut c_void);
    fn lab_close(db: *mut c_void) -> c_int;
}
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}
fn string(v: &Value, key: &str) -> Result<String, String> {
    v[key]
        .as_str()
        .map(str::to_owned)
        .ok_or(format!("missing {key}"))
}
struct Database(*mut c_void);
impl Drop for Database {
    fn drop(&mut self) {
        unsafe {
            lab_close(self.0);
        }
    }
}
impl Database {
    fn error(&self) -> String {
        unsafe {
            CStr::from_ptr(lab_error(self.0))
                .to_string_lossy()
                .into_owned()
        }
    }
    fn exec(&self, sql: &str) -> Result<(), String> {
        let sql = CString::new(sql).map_err(|e| e.to_string())?;
        if unsafe { lab_exec(self.0, sql.as_ptr()) } != 0 {
            Err(self.error())
        } else {
            Ok(())
        }
    }
    fn query(&self, sql: &str, params: Value) -> Result<Value, String> {
        let sql = CString::new(sql).map_err(|e| e.to_string())?;
        let params = CString::new(params.to_string()).map_err(|e| e.to_string())?;
        self.parse(unsafe { lab_query(self.0, sql.as_ptr(), params.as_ptr()) })
    }
    fn parse(&self, p: *mut c_char) -> Result<Value, String> {
        if p.is_null() {
            return Err(self.error());
        }
        let result = unsafe {
            serde_json::from_slice(CStr::from_ptr(p).to_bytes()).map_err(|e| e.to_string())
        };
        unsafe { lab_free(p.cast()) };
        result
    }
    fn tx<T>(&self, work: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
        self.exec("BEGIN IMMEDIATE")?;
        match work() {
            Ok(v) => {
                if let Err(e) = self.exec("COMMIT") {
                    self.exec("ROLLBACK").ok();
                    return Err(e);
                }
                Ok(v)
            }
            Err(e) => {
                self.exec("ROLLBACK").ok();
                Err(e)
            }
        }
    }
    fn job(&self, id: &str) -> Result<Value, String> {
        Ok(self.query("SELECT * FROM jobs WHERE jobId=?", json!([id]))?[0].clone())
    }
    fn snapshot(&self) -> Result<Value, String> {
        let mut jobs = self.query("SELECT * FROM jobs ORDER BY rowid", json!([]))?;
        for job in jobs.as_array_mut().unwrap() {
            job["records"] = self.query(
                "SELECT id,price FROM results WHERE jobId=? ORDER BY id",
                json!([job["jobId"]]),
            )?;
            job["task"] = serde_json::from_str(job["task"].as_str().ok_or("invalid task")?)
                .map_err(|e| e.to_string())?;
        }
        Ok(
            json!({"jobs":jobs,"integrity":self.query("PRAGMA integrity_check",json!([]))?.as_array().unwrap().iter().map(|r|r["integrity_check"].clone()).collect::<Vec<_>>(),"foreignKeys":self.query("PRAGMA foreign_key_check",json!([]))?}),
        )
    }
}
fn canonical(v: &Value) -> Result<(Value, String), String> {
    let mut records = v
        .as_array()
        .filter(|r| !r.is_empty() && r.len() <= 10000)
        .ok_or("invalid records")?
        .clone();
    for record in &mut records {
        let id = string(record, "id")?;
        let price = record["price"]
            .as_i64()
            .filter(|p| p.unsigned_abs() <= 9_007_199_254_740_991)
            .ok_or("invalid price")?;
        if id.is_empty() || id.encode_utf16().count() > 128 || id.contains('\0') {
            return Err("invalid record ID".into());
        }
        *record = json!({"id":id,"price":price});
    }
    records.sort_by(|a, b| {
        a["id"]
            .as_str()
            .unwrap()
            .encode_utf16()
            .cmp(b["id"].as_str().unwrap().encode_utf16())
    });
    if records.windows(2).any(|r| r[0]["id"] == r[1]["id"]) {
        return Err("duplicate record ID".into());
    }
    let normalized = json!(records);
    let digest = format!("{:x}", Sha256::digest(normalized.to_string().as_bytes()));
    Ok((normalized, digest))
}
fn fault(name: &str, requested: &Value) {
    if requested.as_str() == Some(name) {
        unsafe {
            libc::kill(libc::getpid(), libc::SIGKILL);
        }
    }
}
fn run(db: &Database, p: &Value, lease: u64) -> Result<Value, String> {
    match p["op"].as_str().unwrap_or("") {
        "submit"=>db.tx(||{
            let jobs=p["jobs"].as_array().filter(|j|!j.is_empty()&&j.len()<=10000).ok_or("invalid jobs")?;
            for job in jobs {
                let id=string(job,"jobId")?;if id.is_empty()||id.len()>128||!id.bytes().all(|c|c.is_ascii_alphanumeric()||c==b'_'||c==b'-')||job["task"]["scenario"]!="http-catalog"||!job["task"]["url"].is_string(){return Err("invalid job".into())}
                let task=job["task"].to_string();let old=db.job(&id)?;
                if !old.is_null(){let old_task:Value=serde_json::from_str(old["task"].as_str().unwrap()).map_err(|e|e.to_string())?;if old_task!=job["task"]{return Err("job ID conflicts with a different task".into())}}
                else {db.query("INSERT INTO jobs(jobId,task,state) VALUES(?,?,'pending')",json!([id,task]))?;}
            }Ok(json!({"submitted":jobs.len()}))
        }),
        "claim"=>db.tx(||{
            let jobs=db.query("SELECT * FROM jobs WHERE state='pending' OR (state='leased' AND expires<=?) ORDER BY rowid LIMIT 1",json!([now()]))?;
            if jobs.as_array().unwrap().is_empty(){return Ok(json!({"job":null}))}
            let job=&jobs[0];let token=Uuid::new_v4().to_string();let expires=now()+lease;
            db.query("UPDATE jobs SET state='leased',token=?,expires=?,attempts=attempts+1 WHERE jobId=?",json!([token,expires,job["jobId"]]))?;
            Ok(json!({"jobId":job["jobId"],"task":serde_json::from_str::<Value>(job["task"].as_str().unwrap()).map_err(|e|e.to_string())?,"token":token,"expires":expires}))
        }),
        "commit"=>{
            let entries=p["entries"].as_array().filter(|e|!e.is_empty()&&e.len()<=128).ok_or("invalid commit batch")?;
            let mut seen=std::collections::HashSet::new();let mut prepared=Vec::new();
            for entry in entries {let id=string(entry,"jobId")?;if !seen.insert(id.clone()){return Err("invalid commit batch".into())}let (records,digest)=canonical(&entry["records"])?;prepared.push((id,string(entry,"token")?,records,digest));}
            let receipts=db.tx(||{
                let mut receipts=Vec::new();
                for (id,token,records,digest) in &prepared {
                    let job=db.job(id)?;if job.is_null()||job["token"]!=*token{return Err("stale lease token".into())}
                    if job["state"]=="committed"||job["state"]=="acked" {if job["digest"]!=*digest{return Err("replay conflicts with different records".into())}receipts.push(json!({"jobId":id,"receipt":job["receipt"]}));continue;}
                    if job["state"]!="leased"||job["expires"].as_u64().unwrap_or(0)<=now(){return Err("expired lease".into())}
                    for record in records.as_array().unwrap(){db.query("INSERT INTO results(jobId,id,price) VALUES(?,?,?)",json!([id,record["id"],record["price"]]))?;}
                    let receipt=Uuid::new_v4().to_string();db.query("UPDATE jobs SET state='committed',receipt=?,digest=? WHERE jobId=?",json!([receipt,digest,id]))?;
                    fault("before-commit",&p["fault"]);receipts.push(json!({"jobId":id,"receipt":receipt}));
                }Ok(receipts)
            })?;fault("after-commit",&p["fault"]);Ok(json!({"receipts":receipts}))
        },
        "ack"=>{
            let receipts=p["receipts"].as_array().filter(|r|!r.is_empty()&&r.len()<=128).ok_or("invalid acknowledgements")?;
            let result=db.tx(||{for receipt in receipts {let id=string(receipt,"jobId")?;let job=db.job(&id)?;if job.is_null()||job["receipt"]!=receipt["receipt"]||(job["state"]!="committed"&&job["state"]!="acked"){return Err("unknown committed receipt".into())}
                db.query("UPDATE jobs SET state='acked' WHERE jobId=?",json!([id]))?;fault("during-ack",&p["fault"]);
            }Ok(json!({"acknowledged":receipts.len()}))})?;fault("after-ack",&p["fault"]);Ok(result)
        },
        "reject"=>db.tx(||{let id=string(p,"jobId")?;let job=db.job(&id)?;if job.is_null()||job["state"]!="leased"||job["token"]!=p["token"]{return Err("stale lease token".into())}let reason=string(p,"reason")?.chars().take(2000).collect::<String>();db.query("UPDATE jobs SET state='failed',error=? WHERE jobId=?",json!([reason,id]))?;Ok(json!({"rejected":id}))}),
        "snapshot"=>db.snapshot(),
        _=>Err("unknown workflow operation".into())
    }
}
pub fn serve() {
    let mut database: Option<Database> = None;
    let mut lease = 1000;
    for line in io::stdin().lock().lines() {
        let line = match line {
            Ok(l) if l.len() < 128000 => l,
            _ => break,
        };
        let m: Value = match serde_json::from_str(&line) {
            Ok(m) => m,
            Err(_) => break,
        };
        if m["version"] != 1 || !m["id"].is_u64() {
            break;
        }
        let method = m["method"].as_str().unwrap_or("");
        let p = &m["params"];
        let result = (|| -> Result<Value, String> {
            match method {
                "prepare" => {
                    if database.is_some() {
                        return Err("already prepared".into());
                    }
                    lease = p["leaseMs"]
                        .as_u64()
                        .filter(|v| *v >= 10 && *v <= 120000)
                        .ok_or("invalid lease duration")?;
                    let path = CString::new(string(p, "path")?).map_err(|e| e.to_string())?;
                    let format = CString::new("sqlite-wal-full").unwrap();
                    let handle = unsafe { lab_open(path.as_ptr(), format.as_ptr(), 5000) };
                    if handle.is_null() {
                        return Err("database open failed".into());
                    }
                    let db = Database(handle);
                    db.exec("PRAGMA foreign_keys=ON;CREATE TABLE IF NOT EXISTS jobs(jobId TEXT PRIMARY KEY,task TEXT NOT NULL,state TEXT NOT NULL CHECK(state IN ('pending','leased','committed','acked','failed')),token TEXT,expires INTEGER NOT NULL DEFAULT 0,attempts INTEGER NOT NULL DEFAULT 0,receipt TEXT,digest TEXT,error TEXT);CREATE TABLE IF NOT EXISTS results(jobId TEXT NOT NULL REFERENCES jobs(jobId),id TEXT NOT NULL,price INTEGER NOT NULL,PRIMARY KEY(jobId,id));")?;
                    database = Some(db);
                    Ok(json!({"ready":true}))
                }
                "hello" => {
                    let db = database.as_ref().ok_or("not prepared")?;
                    let mut info = db.parse(unsafe { lab_info(db.0) })?;
                    info["backend"] = json!("workflow-rust");
                    info["protocol"] = json!(1);
                    info["foreign_keys"] = json!(1);
                    Ok(info)
                }
                "run" => run(database.as_ref().ok_or("not prepared")?, p, lease),
                "close" => {
                    database = None;
                    Ok(json!({"closed":true}))
                }
                _ => Err("unknown method".into()),
            }
        })();
        let response = match result {
            Ok(v) => json!({"version":1,"id":m["id"],"result":v}),
            Err(e) => json!({"version":1,"id":m["id"],"error":e}),
        };
        let mut output = io::stdout().lock();
        writeln!(output, "{response}").ok();
        output.flush().ok();
        if method == "close" {
            break;
        }
    }
}
