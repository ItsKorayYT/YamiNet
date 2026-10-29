use crate::{invalid, PeerRecord, MAX_FRAME, MAX_REPLY_RECORDS};
use std::io::{self, Read, Write};
use yami_identity::NodeId;
pub const MAGIC: [u8; 4] = *b"YDH\0";
pub(crate) type Nonce = [u8; 16];
#[derive(Clone, Debug)]
pub(crate) enum Message {
    Find(NodeId),
    Store(PeerRecord),
    Peers(Vec<PeerRecord>),
    Stored,
}
fn record(bytes: &mut Vec<u8>, value: &PeerRecord) {
    let value = value.encode();
    bytes.extend_from_slice(&(value.len() as u16).to_be_bytes());
    bytes.extend_from_slice(&value);
}
pub(crate) fn encode(nonce: Nonce, message: &Message) -> io::Result<[u8; MAX_FRAME]> {
    let mut body = Vec::new();
    let kind = match message {
        Message::Find(id) => {
            body.extend_from_slice(&id.public_key());
            1
        }
        Message::Store(value) => {
            record(&mut body, value);
            2
        }
        Message::Peers(records) => {
            if records.len() > MAX_REPLY_RECORDS {
                return Err(invalid("too many DHT response records"));
            }
            body.push(records.len() as u8);
            for value in records {
                record(&mut body, value);
            }
            3
        }
        Message::Stored => 4,
    };
    if body.len() > MAX_FRAME - 24 {
        return Err(invalid("DHT body is too large"));
    }
    let mut packet = [0; MAX_FRAME];
    packet[..4].copy_from_slice(&MAGIC);
    packet[4] = 1;
    packet[5] = kind;
    packet[6..22].copy_from_slice(&nonce);
    packet[22..24].copy_from_slice(&(body.len() as u16).to_be_bytes());
    packet[24..24 + body.len()].copy_from_slice(&body);
    Ok(packet)
}
struct Cursor<'a>(&'a [u8]);
impl<'a> Cursor<'a> {
    fn take(&mut self, count: usize) -> io::Result<&'a [u8]> {
        let value = self
            .0
            .get(..count)
            .ok_or_else(|| invalid("truncated DHT field"))?;
        self.0 = &self.0[count..];
        Ok(value)
    }
    fn record(&mut self) -> io::Result<PeerRecord> {
        let len = u16::from_be_bytes(
            self.take(2)?
                .try_into()
                .map_err(|_| invalid("record length"))?,
        ) as usize;
        PeerRecord::decode(self.take(len)?)
    }
}
pub(crate) fn decode(packet: &[u8]) -> io::Result<(Nonce, Message)> {
    if packet.len() != MAX_FRAME || packet[..4] != MAGIC || packet[4] != 1 {
        return Err(invalid("not a bounded version 1 DHT frame"));
    }
    let len = u16::from_be_bytes([packet[22], packet[23]]) as usize;
    if len > MAX_FRAME - 24 || packet[24 + len..].iter().any(|b| *b != 0) {
        return Err(invalid("DHT body length/padding"));
    }
    let mut cursor = Cursor(&packet[24..24 + len]);
    let message = match packet[5] {
        1 => Message::Find(
            NodeId::from_public_key(cursor.take(32)?.try_into().map_err(|_| invalid("target"))?)
                .map_err(io::Error::other)?,
        ),
        2 => Message::Store(cursor.record()?),
        3 => {
            let count = cursor.take(1)?[0] as usize;
            if count > MAX_REPLY_RECORDS {
                return Err(invalid("DHT response count exceeds budget"));
            }
            let mut records: Vec<PeerRecord> = Vec::with_capacity(count);
            for _ in 0..count {
                let value = cursor.record()?;
                if records.iter().any(|r| r.id() == value.id()) {
                    return Err(invalid("duplicate ID in DHT response"));
                }
                records.push(value);
            }
            Message::Peers(records)
        }
        4 => Message::Stored,
        _ => return Err(invalid("unknown DHT message type")),
    };
    if !cursor.0.is_empty() {
        return Err(invalid("trailing DHT body fields"));
    }
    Ok((
        packet[6..22]
            .try_into()
            .map_err(|_| invalid("request nonce"))?,
        message,
    ))
}
pub(crate) fn write(
    connection: &mut (impl Write + ?Sized),
    nonce: Nonce,
    message: &Message,
) -> io::Result<()> {
    connection.write_all(&encode(nonce, message)?)?;
    connection.flush()
}
pub(crate) fn read(connection: &mut (impl Read + ?Sized)) -> io::Result<(Nonce, Message)> {
    let mut packet = [0; MAX_FRAME];
    connection.read_exact(&mut packet)?;
    decode(&packet)
}
pub(crate) fn read_prefixed(
    connection: &mut (impl Read + ?Sized),
    prefix: [u8; 4],
) -> io::Result<(Nonce, Message)> {
    let mut packet = [0; MAX_FRAME];
    packet[..4].copy_from_slice(&prefix);
    connection.read_exact(&mut packet[4..])?;
    decode(&packet)
}
