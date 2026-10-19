//! DNS messages as the DNS fixture reads and writes them (RFC 1035): a query
//! of one question, and an answer of addresses with a TTL of 0, its name a
//! pointer to the question's. A message that is itself an answer, or too
//! short to carry an ID, gets no reply.

use std::net::IpAddr;

/// The record type of an IPv4 address.
pub(crate) const A: u16 = 1;
/// The record type of an IPv6 address.
pub(crate) const AAAA: u16 = 28;
/// The Internet class.
const CLASS_IN: u16 = 1;

/// The header's length.
const HEADER: usize = 12;
/// The longest name in its wire form (RFC 1035 section 2.3.4).
const NAME_LIMIT: usize = 255;
/// The longest label.
const LABEL_LIMIT: usize = 63;
/// A pointer to the question's name, at the offset just past the header.
const QUESTION_NAME: [u8; 2] = [0xc0, 0x0c];

/// A response code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Rcode {
    NoError = 0,
    FormatError = 1,
    NameError = 3,
    NotImplemented = 4,
}

/// A query of one question, as read.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Query {
    id: u16,
    recursion_desired: bool,
    /// The name asked for, in lowercase, without a trailing dot.
    pub(crate) name: String,
    pub(crate) record_type: u16,
    /// The question as sent, which the answer repeats.
    question: Vec<u8>,
}

/// Why a message is not a query the fixture answers.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// It cannot be answered at all: too short, or itself an answer.
    Ignored,
    /// It can be answered with this code and no question.
    Answered { id: u16, rcode: Rcode },
}

/// Reads a query.
pub(crate) fn parse(message: &[u8]) -> Result<Query, Refusal> {
    let Some(header) = message.get(..HEADER) else {
        return Err(Refusal::Ignored);
    };
    let id = u16::from_be_bytes([header[0], header[1]]);
    if header[2] & 0x80 != 0 {
        return Err(Refusal::Ignored);
    }
    let refuse = |rcode| Err(Refusal::Answered { id, rcode });
    if (header[2] >> 3) & 0x0f != 0 {
        return refuse(Rcode::NotImplemented);
    }
    if u16::from_be_bytes([header[4], header[5]]) != 1 {
        return refuse(Rcode::FormatError);
    }
    let Some((name, end)) = name(message, HEADER) else {
        return refuse(Rcode::FormatError);
    };
    let Some(fields) = message.get(end..end + 4) else {
        return refuse(Rcode::FormatError);
    };
    if u16::from_be_bytes([fields[2], fields[3]]) != CLASS_IN {
        return refuse(Rcode::NotImplemented);
    }
    Ok(Query {
        id,
        recursion_desired: header[2] & 0x01 != 0,
        name,
        record_type: u16::from_be_bytes([fields[0], fields[1]]),
        question: message[HEADER..end + 4].to_vec(),
    })
}

/// Reads the uncompressed name at `at`: the name in lowercase, and the
/// offset just past it.
fn name(message: &[u8], mut at: usize) -> Option<(String, usize)> {
    let mut labels = Vec::new();
    let start = at;
    loop {
        let length = usize::from(*message.get(at)?);
        if length == 0 {
            break;
        }
        if length > LABEL_LIMIT {
            return None;
        }
        let label = message.get(at + 1..at + 1 + length)?;
        labels.push(std::str::from_utf8(label).ok()?.to_ascii_lowercase());
        at += 1 + length;
        if at - start >= NAME_LIMIT {
            return None;
        }
    }
    Some((labels.join("."), at + 1))
}

/// The answer to `query`: `rcode`, and each address of the asked type with a
/// TTL of 0.
pub(crate) fn answer(query: &Query, rcode: Rcode, addresses: &[IpAddr]) -> Vec<u8> {
    let count = u16::try_from(addresses.len()).expect("invariant: a name holds few addresses");
    let mut message = header(query.id, query.recursion_desired, rcode, 1, count);
    message.extend_from_slice(&query.question);
    for address in addresses {
        message.extend_from_slice(&QUESTION_NAME);
        let (record_type, data) = match address {
            IpAddr::V4(address) => (A, address.octets().to_vec()),
            IpAddr::V6(address) => (AAAA, address.octets().to_vec()),
        };
        message.extend_from_slice(&record_type.to_be_bytes());
        message.extend_from_slice(&CLASS_IN.to_be_bytes());
        message.extend_from_slice(&0u32.to_be_bytes());
        let length = u16::try_from(data.len()).expect("invariant: an address is short");
        message.extend_from_slice(&length.to_be_bytes());
        message.extend_from_slice(&data);
    }
    message
}

/// An answer that carries only `rcode`.
pub(crate) fn refusal(id: u16, rcode: Rcode) -> Vec<u8> {
    header(id, false, rcode, 0, 0)
}

/// A response's header: authoritative, recursion available.
fn header(id: u16, recursion_desired: bool, rcode: Rcode, questions: u16, answers: u16) -> Vec<u8> {
    let mut header = Vec::with_capacity(HEADER);
    header.extend_from_slice(&id.to_be_bytes());
    header.push(0x80 | 0x04 | u8::from(recursion_desired));
    header.push(0x80 | rcode as u8);
    header.extend_from_slice(&questions.to_be_bytes());
    header.extend_from_slice(&answers.to_be_bytes());
    header.extend_from_slice(&[0, 0, 0, 0]);
    header
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    /// A query for `catalog.test`'s A records, ID 0xbeef, recursion
    /// desired, as `dig` sends it without EDNS.
    const QUERY: &[u8] =
        b"\xbe\xef\x01\x00\x00\x01\x00\x00\x00\x00\x00\x00\x07Catalog\x04test\x00\x00\x01\x00\x01";

    #[test]
    fn a_query_is_read_with_its_name_in_lowercase() {
        let query = parse(QUERY).unwrap();

        assert_eq!(
            (query.id, query.name.as_str(), query.record_type),
            (0xbeef, "catalog.test", A)
        );
    }

    #[test]
    fn an_answer_repeats_the_question_and_points_each_record_at_its_name() {
        let query = parse(QUERY).unwrap();

        let answer = answer(&query, Rcode::NoError, &["127.0.0.1".parse().unwrap()]);

        assert_eq!(
            answer,
            [
                b"\xbe\xef\x85\x80\x00\x01\x00\x01\x00\x00\x00\x00".as_slice(),
                &QUERY[12..],
                b"\xc0\x0c\x00\x01\x00\x01\x00\x00\x00\x00\x00\x04\x7f\x00\x00\x01",
            ]
            .concat()
        );
    }

    #[test]
    fn a_message_that_is_an_answer_gets_no_reply() {
        let mut message = QUERY.to_vec();
        message[2] |= 0x80;

        assert_eq!(parse(&message), Err(Refusal::Ignored));
    }

    #[test]
    fn a_compressed_question_is_a_format_error() {
        let message = b"\xbe\xef\x01\x00\x00\x01\x00\x00\x00\x00\x00\x00\xc0\x0c\x00\x01\x00\x01";

        assert_eq!(
            parse(message),
            Err(Refusal::Answered {
                id: 0xbeef,
                rcode: Rcode::FormatError
            })
        );
    }

    proptest! {
        #[test]
        fn any_bytes_are_read_without_panicking(message in proptest::collection::vec(any::<u8>(), 0..300)) {
            let _ = parse(&message);
        }
    }
}
