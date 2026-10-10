//! Hello-world sidecar: newline-delimited JSON-RPC over stdio. No dependencies, so
//! the build stays fast; install friction does not depend on engine logic.
use std::io::{BufRead, Write};

fn main() {
    let target = env!("BUILD_TARGET");
    if std::env::args().any(|a| a == "--version") {
        println!("engined 0.1.0 ({target})");
        return;
    }
    let stdin = std::io::stdin();
    let mut out = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        // Crude id extraction keeps the binary dependency-free.
        let id = line
            .split("\"id\":")
            .nth(1)
            .and_then(|s| s.trim_start().split([',', '}']).next())
            .map(str::trim)
            .unwrap_or("null");
        let _ = writeln!(
            out,
            "{{\"jsonrpc\":\"2.0\",\"id\":{id},\"result\":{{\"pong\":true,\"target\":\"{target}\",\"pid\":{}}}}}",
            std::process::id()
        );
        let _ = out.flush();
    }
}
