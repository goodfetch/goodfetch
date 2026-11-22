#![expect(
    clippy::disallowed_types,
    reason = "the relay's tests stand in for the browser and the tool on loopback sockets"
)]

use std::io::{BufRead as _, BufReader};
use std::net::{TcpListener, TcpStream};

use super::*;

/// A client's frame: masked, as a client sends it, with `fin` and `opcode`.
fn frame(fin: bool, opcode: u8, payload: &[u8]) -> Vec<u8> {
    let mut bytes = vec![u8::from(fin) << 7 | opcode];
    match payload.len() {
        length @ 0..=125 => bytes.push(0x80 | u8::try_from(length).unwrap()),
        length @ 126..=65535 => {
            bytes.push(0x80 | 0x7e);
            bytes.extend(u16::try_from(length).unwrap().to_be_bytes());
        }
        length => {
            bytes.push(0x80 | 0x7f);
            bytes.extend((length as u64).to_be_bytes());
        }
    }
    let mask = [1, 2, 3, 4];
    bytes.extend(mask);
    bytes.extend(
        payload
            .iter()
            .enumerate()
            .map(|(index, byte)| byte ^ mask[index % 4]),
    );
    bytes
}

fn command(id: u32) -> Vec<u8> {
    frame(
        true,
        1,
        format!(r#"{{"id":{id},"method":"Runtime.evaluate"}}"#).as_bytes(),
    )
}

#[test]
fn each_whole_data_message_counts_once_whatever_its_frames_and_chunks() {
    let mut stream = Vec::new();
    stream.extend(command(1));
    // A message in three frames: one command.
    stream.extend(frame(false, 1, b"{\"id\":2,"));
    stream.extend(frame(false, 0, b"\"method\":"));
    stream.extend(frame(true, 0, b"\"Page.navigate\"}"));
    // Control frames are no command.
    stream.extend(frame(true, 9, b"ping"));
    stream.extend(frame(true, 10, b""));
    // Lengths in 16 and 64 bits.
    stream.extend(frame(true, 2, &[7; 300]));
    stream.extend(frame(true, 1, &vec![b'x'; 70_000]));
    stream.extend(frame(true, 8, b""));
    for chunk in [1, 2, 3, 7, 1000, stream.len()] {
        let mut frames = Frames::default();
        let counted: u64 = stream.chunks(chunk).map(|bytes| frames.feed(bytes)).sum();
        assert_eq!(counted, 4, "in chunks of {chunk}");
    }
}

fn get(address: &str, path: &str) -> String {
    let mut stream = TcpStream::connect(address).unwrap();
    write!(stream, "GET {path} HTTP/1.1\r\nHost: {address}\r\n\r\n").unwrap();
    let mut answer = String::new();
    stream.read_to_string(&mut answer).unwrap();
    answer
}

/// A tool's commands reach the browser unchanged, and the marks count those
/// sent between them alone; each reply the tool reads shows its command
/// was counted, since the relay counts a command before passing it on.
#[test]
fn the_relay_passes_a_tools_websocket_through_and_marks_its_commands() {
    let (port, received) = answering_browser();
    let relay = Relay::start(port).unwrap();
    let address = relay.address().to_string();
    let mut tool = BufReader::new(TcpStream::connect(&address).unwrap());
    write!(
        tool.get_mut(),
        "GET /devtools/browser/abc HTTP/1.1\r\nHost: {address}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\r\n"
    )
    .unwrap();
    let mut line = String::new();
    tool.read_line(&mut line).unwrap();
    assert!(line.starts_with("HTTP/1.1 101"), "{line}");
    while line != "\r\n" {
        line.clear();
        tool.read_line(&mut line).unwrap();
    }
    let send = |tool: &mut BufReader<TcpStream>, id: u32| {
        tool.get_mut().write_all(&command(id)).unwrap();
        let mut reply = vec![0; answer(format!(r#"{{"id":{id},"result":{{}}}}"#).as_bytes()).len()];
        tool.read_exact(&mut reply).unwrap();
    };
    send(&mut tool, 1);
    assert!(get(&address, "/start").starts_with("HTTP/1.1 204"));
    send(&mut tool, 2);
    send(&mut tool, 3);
    assert!(get(&address, "/end").starts_with("HTTP/1.1 204"));
    send(&mut tool, 4);
    drop(tool);
    let expected: Vec<u8> = (1..=4).flat_map(command).collect();
    assert_eq!(received.join().unwrap(), expected);
    let (start, end) = relay.marks();
    let (start, end) = (start.unwrap(), end.unwrap());
    assert_eq!(end.commands - start.commands, 2);
    assert!(end.at >= start.at);
}

#[test]
fn a_mark_asked_twice_is_refused_and_keeps_the_first() {
    let (port, _received) = answering_browser();
    let relay = Relay::start(port).unwrap();
    let address = relay.address().to_string();
    assert!(get(&address, "/start").starts_with("HTTP/1.1 204"));
    assert!(get(&address, "/start").starts_with("HTTP/1.1 409"));
    let (start, end) = relay.marks();
    assert!(start.is_some());
    assert!(end.is_none());
}

/// A browser's frame: unmasked, as a server sends it.
fn answer(payload: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0x81];
    match payload.len() {
        length @ 0..=125 => bytes.push(u8::try_from(length).unwrap()),
        length => {
            bytes.push(0x7e);
            bytes.extend(u16::try_from(length).unwrap().to_be_bytes());
        }
    }
    bytes.extend(payload);
    bytes
}

#[test]
fn a_browser_message_that_answers_a_command_is_a_reply_and_an_event_is_not() {
    let mut stream = Vec::new();
    stream.extend(answer(br#"{"id":1,"result":{}}"#));
    stream.extend(answer(br#"{"method":"Page.loadEventFired","params":{}}"#));
    // A reply in two frames, a ping between them.
    stream.extend([0x01, 4]);
    stream.extend(br#"{"id"#);
    stream.extend([0x89, 0]);
    stream.extend([0x80, 16]);
    stream.extend(br#"":2,"result":{}}"#);
    stream.extend(answer(&[b' '; 300]));
    for chunk in [1, 2, 5, 1000] {
        let mut frames = Frames::default();
        let read = stream.chunks(chunk).fold(Ended::default(), |sum, bytes| {
            let read = frames.read(bytes);
            Ended {
                messages: sum.messages + read.messages,
                replies: sum.replies + read.replies,
            }
        });
        assert_eq!(
            read,
            Ended {
                messages: 4,
                replies: 2
            },
            "in chunks of {chunk}"
        );
    }
    // A masked frame, as a client sends it, is read unmasked.
    let mut frames = Frames::default();
    assert_eq!(frames.read(&command(7)).replies, 1);
}

#[test]
fn a_round_starts_each_time_the_tool_sends_with_nothing_in_flight() {
    let rounds = AtomicU64::new(0);
    let awaited = Awaited::default();
    // One command, answered: one round.
    awaited.sent(1, &rounds);
    awaited.answered(1);
    // Two commands at once, then a third before either answer: one round.
    awaited.sent(2, &rounds);
    awaited.sent(1, &rounds);
    awaited.answered(2);
    awaited.answered(1);
    // A message that ends no command starts none.
    awaited.sent(0, &rounds);
    // Two commands one after another, each awaited: two rounds.
    awaited.sent(1, &rounds);
    awaited.answered(1);
    awaited.sent(1, &rounds);
    assert_eq!(rounds.load(Ordering::SeqCst), 4);
}

#[test]
fn a_relay_with_no_browser_answers_its_marks_and_nothing_else() {
    let relay = Relay::marks_only().unwrap();
    let address = relay.address().to_string();
    assert!(get(&address, "/start").starts_with("HTTP/1.1 204"));
    assert!(get(&address, "/json/version").starts_with("HTTP/1.1 404"));
    assert!(get(&address, "/end").starts_with("HTTP/1.1 204"));
    let (start, end) = relay.marks();
    assert_eq!((start.unwrap().rounds, end.unwrap().commands), (0, 0));
}

#[test]
fn each_mark_records_the_engines_counts_as_they_stand_then() {
    let commands = Arc::new(AtomicU64::new(7));
    let read = Arc::clone(&commands);
    let relay = Relay::marks_reading(Box::new(move || {
        let commands = read.load(Ordering::SeqCst);
        Some(EngineCounts {
            paused: commands / 2,
            commands,
        })
    }))
    .unwrap();
    let address = relay.address().to_string();
    assert!(get(&address, "/start").starts_with("HTTP/1.1 204"));
    commands.store(30, Ordering::SeqCst);
    assert!(get(&address, "/end").starts_with("HTTP/1.1 204"));
    let (start, end) = relay.marks();
    assert_eq!(
        (start.unwrap().engine, end.unwrap().engine),
        (
            Some(EngineCounts {
                paused: 3,
                commands: 7
            }),
            Some(EngineCounts {
                paused: 15,
                commands: 30
            })
        )
    );
}

/// A browser's endpoint that accepts one WebSocket, answers its handshake
/// as Chrome does, then answers each command it receives once whole, and
/// keeps the commands it received.
fn answering_browser() -> (u16, std::thread::JoinHandle<Vec<u8>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let thread = std::thread::spawn(move || {
        let mut received = Vec::new();
        let Ok((stream, _)) = listener.accept() else {
            return received;
        };
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        loop {
            line.clear();
            if reader.read_line(&mut line).unwrap_or(0) == 0 {
                return received;
            }
            if line == "\r\n" {
                break;
            }
        }
        reader
            .get_mut()
            .write_all(
                b"HTTP/1.1 101 WebSocket Protocol Handshake\r\nUpgrade: WebSocket\r\n\
                  Connection: Upgrade\r\nSec-WebSocket-Accept: s3pPLMBiTxaQ9kYGzzhZRbK+xOo=\r\n\r\n",
            )
            .unwrap();
        let mut command = vec![0; command(1).len()];
        for id in 1.. {
            if reader.read_exact(&mut command).is_err() {
                break;
            }
            received.extend_from_slice(&command);
            let reply = answer(format!(r#"{{"id":{id},"result":{{}}}}"#).as_bytes());
            if reader.get_mut().write_all(&reply).is_err() {
                break;
            }
        }
        received
    });
    (port, thread)
}

#[test]
fn the_relay_counts_one_round_per_command_awaited_after_the_browsers_handshake() {
    let relay = Relay::start(answering_browser().0).unwrap();
    let address = relay.address().to_string();
    let mut tool = BufReader::new(TcpStream::connect(&address).unwrap());
    write!(
        tool.get_mut(),
        "GET /devtools/browser/abc HTTP/1.1\r\nHost: {address}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\r\n"
    )
    .unwrap();
    let mut line = String::new();
    while line != "\r\n" {
        line.clear();
        tool.read_line(&mut line).unwrap();
    }
    let replies = |tool: &mut BufReader<TcpStream>, count: usize| {
        let mut reply = vec![0; answer(br#"{"id":1,"result":{}}"#).len() * count];
        tool.read_exact(&mut reply).unwrap();
    };
    assert!(get(&address, "/start").starts_with("HTTP/1.1 204"));
    // Two commands, each awaited before the next: two rounds.
    tool.get_mut().write_all(&command(1)).unwrap();
    replies(&mut tool, 1);
    tool.get_mut().write_all(&command(2)).unwrap();
    replies(&mut tool, 1);
    // Two sent together: one round.
    tool.get_mut()
        .write_all(&[command(3), command(4)].concat())
        .unwrap();
    replies(&mut tool, 2);
    assert!(get(&address, "/end").starts_with("HTTP/1.1 204"));
    let (start, end) = relay.marks();
    let (start, end) = (start.unwrap(), end.unwrap());
    assert_eq!(
        (end.commands - start.commands, end.rounds - start.rounds),
        (4, 3)
    );
}

#[test]
fn the_browsers_handshake_answer_is_skipped_whatever_its_chunks() {
    let head: &[u8] = b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\n\r\r\n\r\n";
    let stream = [head, &answer(br#"{"id":1,"result":{}}"#)].concat();
    for chunk in [1, 2, 3, 5, stream.len()] {
        let (mut end, mut frames) = (HeadEnd::default(), Frames::default());
        let replies: u64 = stream
            .chunks(chunk)
            .map(|bytes| frames.read(end.past(bytes)).replies)
            .sum();
        assert_eq!(replies, 1, "in chunks of {chunk}");
    }
}
