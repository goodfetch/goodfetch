//! Writes the `sample-protocol` scenario: the messages a Client sent and
//! received, one per line.

const TRANSCRIPT: &str = r#"{"id":1,"method":"sample.echo","params":{"text":"hello"}}
{"id":1,"result":{"text":"hello"}}
"#;

fn main() {
    let folder = std::path::PathBuf::from(std::env::args_os().nth(1).expect("the folder"));
    let transcripts = folder.join("transcripts");
    std::fs::create_dir_all(&transcripts).expect("creating transcripts/");
    std::fs::write(transcripts.join("echo.jsonl"), TRANSCRIPT).expect("writing echo.jsonl");
}
