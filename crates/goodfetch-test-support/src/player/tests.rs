use std::cell::RefCell;
use std::io::{self, BufRead, Cursor};

use serde_json::json;

use super::{PlayError, play, play_relinking};
use crate::script::Script;

/// A script whose first Client is dropped after its `hello` is answered,
/// and whose next sends `engine.info`.
fn dropping() -> Script {
    serde_json::from_value(json!({
        "description": "drops the first Client after hello",
        "steps": [
            { "expect": { "method": "hello", "params": {} } },
            { "reply": { "result": {} } },
            { "drop": {} },
            { "expect": { "method": "engine.info" } },
            { "reply": { "result": {} } },
        ],
    }))
    .expect("invariant: the test's script reads")
}

/// The Client's two links, read one after the other: each relink moves to
/// the next.
struct Links {
    links: RefCell<Vec<Cursor<Vec<u8>>>>,
}

impl io::Read for &Links {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let mut links = self.links.borrow_mut();
        links
            .first_mut()
            .map_or(Ok(0), |link| io::Read::read(link, buffer))
    }
}

#[test]
fn a_drop_plays_the_steps_after_it_to_the_next_client() {
    let links = Links {
        links: RefCell::new(vec![
            Cursor::new(
                b"{\"jsonrpc\":\"2.0\",\"id\":0,\"method\":\"hello\",\"params\":{}}\n".to_vec(),
            ),
            Cursor::new(b"{\"jsonrpc\":\"2.0\",\"id\":5,\"method\":\"engine.info\"}\n".to_vec()),
        ]),
    };
    let mut written = Vec::new();
    let mut relinked = 0;

    let ending = play_relinking(
        &dropping(),
        io::BufReader::new(&links),
        &mut written,
        io::sink(),
        &mut || {
            relinked += 1;
            links.links.borrow_mut().remove(0);
            Ok(())
        },
    );

    let answers: Vec<serde_json::Value> = written
        .lines()
        .map(|line| serde_json::from_str(&line.expect("invariant: written as text")))
        .collect::<Result<_, _>>()
        .expect("invariant: the player writes JSON lines");
    assert!(ending.is_ok(), "{ending:?}");
    assert_eq!(
        (relinked, answers),
        (
            1,
            vec![
                json!({ "jsonrpc": "2.0", "id": 0, "result": {} }),
                json!({ "jsonrpc": "2.0", "id": 5, "result": {} }),
            ]
        )
    );
}

#[test]
fn a_player_that_takes_no_other_client_refuses_a_drop() {
    let client = b"{\"jsonrpc\":\"2.0\",\"id\":0,\"method\":\"hello\",\"params\":{}}\n";

    let failed = play(&dropping(), &client[..], io::sink(), io::sink());

    assert!(
        matches!(failed, Err(PlayError::DropUnsupported { step: 2 })),
        "{failed:?}"
    );
}
