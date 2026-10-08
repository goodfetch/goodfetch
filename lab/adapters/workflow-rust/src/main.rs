mod coordinator;
mod ledger;
fn main() {
    if std::env::args().nth(1).as_deref() == Some("sink") {
        ledger::serve();
        return;
    }
    let result = (|| {
        let path = std::env::args().nth(1).ok_or("missing config path")?;
        let config = serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| e.to_string())?
            .block_on(coordinator::execute(config))
    })();
    match result {
        Ok(summary) => println!("{}", serde_json::json!({"type":"summary","result":summary})),
        Err(error) => {
            println!("{}", serde_json::json!({"type":"fatal","error":error}));
            std::process::exit(1)
        }
    }
}
