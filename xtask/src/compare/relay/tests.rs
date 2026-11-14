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

/// A browser's endpoint that accepts one WebSocket and keeps what it
/// received.
fn browser() -> (u16, std::thread::JoinHandle<Vec<u8>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let thread = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        loop {
            line.clear();
            reader.read_line(&mut line).unwrap();
            if line == "\r\n" {
                break;
            }
        }
        reader
            .get_mut()
            .write_all(b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\r\n")
            .unwrap();
        let mut received = Vec::new();
        reader.read_to_end(&mut received).unwrap();
        received
    });
    (port, thread)
}

fn get(address: &str, path: &str) -> String {
    let mut stream = TcpStream::connect(address).unwrap();
    write!(stream, "GET {path} HTTP/1.1\r\nHost: {address}\r\n\r\n").unwrap();
    let mut answer = String::new();
    stream.read_to_string(&mut answer).unwrap();
    answer
}

#[test]
fn the_relay_passes_a_tools_websocket_through_and_marks_its_commands() {
    let (port, received) = browser();
    let relay = Relay::start(port).unwrap();
    let address = relay.address().to_string();
    let mut tool = TcpStream::connect(&address).unwrap();
    write!(
        tool,
        "GET /devtools/browser/abc HTTP/1.1\r\nHost: {address}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\r\n"
    )
    .unwrap();
    let mut head = [0; 12];
    tool.read_exact(&mut head).unwrap();
    assert_eq!(&head, b"HTTP/1.1 101");
    tool.write_all(&command(1)).unwrap();
    // The commands reach the relay's count before the mark asks for it.
    let wait_for = |count: u64| {
        for _ in 0..500 {
            if relay.commands() == count {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("the relay counted {}, not {count}", relay.commands());
    };
    wait_for(1);
    assert!(get(&address, "/start").starts_with("HTTP/1.1 204"));
    tool.write_all(&command(2)).unwrap();
    tool.write_all(&command(3)).unwrap();
    wait_for(3);
    assert!(get(&address, "/end").starts_with("HTTP/1.1 204"));
    tool.write_all(&command(4)).unwrap();
    wait_for(4);
    drop(tool);
    let mut expected = command(1);
    for id in 2..=4 {
        expected.extend(command(id));
    }
    assert_eq!(received.join().unwrap(), expected);
    let (start, end) = relay.marks();
    let (start, end) = (start.unwrap(), end.unwrap());
    assert_eq!(end.commands - start.commands, 2);
    assert!(end.at >= start.at);
}

#[test]
fn a_mark_asked_twice_is_refused_and_keeps_the_first() {
    let (port, _received) = browser();
    let relay = Relay::start(port).unwrap();
    let address = relay.address().to_string();
    assert!(get(&address, "/start").starts_with("HTTP/1.1 204"));
    assert!(get(&address, "/start").starts_with("HTTP/1.1 409"));
    let (start, end) = relay.marks();
    assert!(start.is_some());
    assert!(end.is_none());
}
