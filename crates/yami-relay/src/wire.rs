//! Strict bounded relay control frames, followed by raw inner TLS bytes.
use crate::invalid;
use std::io::{self, Read, Write};
pub const HEADER: usize = 8;
pub const MAX_BODY: usize = 64;
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame {
    Register([u8; 32]),
    Challenge([u8; 32], u64),
    Proof([u8; 64]),
    Registered(u64),
    Connect([u8; 32]),
    Go([u8; 32]),
    Paired([u8; 32]),
    Error(u8),
}
impl Frame {
    fn body(&self) -> (u8, Vec<u8>) {
        match self {
            Self::Register(id) => (1, id.to_vec()),
            Self::Challenge(nonce, expires) => {
                (2, [nonce.as_slice(), &expires.to_be_bytes()].concat())
            }
            Self::Proof(signature) => (3, signature.to_vec()),
            Self::Registered(expires) => (4, expires.to_be_bytes().to_vec()),
            Self::Connect(id) => (5, id.to_vec()),
            Self::Go(id) => (6, id.to_vec()),
            Self::Paired(id) => (7, id.to_vec()),
            Self::Error(code) => (255, vec![*code]),
        }
    }
}
pub fn write(stream: &mut (impl Write + ?Sized), frame: &Frame) -> io::Result<()> {
    let (kind, body) = frame.body();
    let mut header = [0; HEADER];
    header[..4].copy_from_slice(b"YRL\0");
    header[4] = 1;
    header[5] = kind;
    header[6..].copy_from_slice(&(body.len() as u16).to_be_bytes());
    stream.write_all(&header)?;
    stream.write_all(&body)?;
    stream.flush()
}
pub fn read(stream: &mut (impl Read + ?Sized)) -> io::Result<Frame> {
    let mut first = [0];
    stream.read_exact(&mut first)?;
    read_prefixed(stream, first[0])
}
pub fn read_prefixed(stream: &mut (impl Read + ?Sized), first: u8) -> io::Result<Frame> {
    let mut header = [0; HEADER];
    header[0] = first;
    stream.read_exact(&mut header[1..])?;
    if &header[..4] != b"YRL\0" || header[4] != 1 {
        return Err(invalid("unknown relay protocol/version"));
    }
    let length = u16::from_be_bytes([header[6], header[7]]) as usize;
    let expected = match header[5] {
        1 | 5 | 6 | 7 => 32,
        2 => 40,
        3 => 64,
        4 => 8,
        255 => 1,
        _ => return Err(invalid("unknown relay message")),
    };
    if length != expected || length > MAX_BODY {
        return Err(invalid("invalid relay frame length"));
    }
    let mut body = [0; MAX_BODY];
    stream.read_exact(&mut body[..length])?;
    Ok(match header[5] {
        1 => Frame::Register(body[..32].try_into().map_err(|_| invalid("host ID"))?),
        2 => Frame::Challenge(
            body[..32].try_into().map_err(|_| invalid("nonce"))?,
            u64::from_be_bytes(
                body[32..40]
                    .try_into()
                    .map_err(|_| invalid("challenge expiry"))?,
            ),
        ),
        3 => Frame::Proof(body),
        4 => Frame::Registered(u64::from_be_bytes(
            body[..8].try_into().map_err(|_| invalid("lease expiry"))?,
        )),
        5 => Frame::Connect(body[..32].try_into().map_err(|_| invalid("destination"))?),
        6 => Frame::Go(body[..32].try_into().map_err(|_| invalid("destination"))?),
        7 => Frame::Paired(body[..32].try_into().map_err(|_| invalid("destination"))?),
        255 if (1..=6).contains(&body[0]) => Frame::Error(body[0]),
        _ => return Err(invalid("invalid relay error code")),
    })
}
pub fn expect(frame: Frame) -> io::Result<Frame> {
    if let Frame::Error(code) = frame {
        return Err(io::Error::new(
            io::ErrorKind::ConnectionRefused,
            match code {
                1 => "relay registration is busy/conflicting",
                2 => "relay has no live registration for this node",
                3 => "relay registration authentication failed",
                4 => "malformed relay request",
                5 => "relay resource limit reached",
                _ => "relay registration expired",
            },
        ));
    }
    Ok(frame)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn frames_reject_unknown_versions_lengths_truncation_and_kinds() {
        for frame in [
            Frame::Register([1; 32]),
            Frame::Challenge([2; 32], 55),
            Frame::Proof([3; 64]),
            Frame::Registered(77),
            Frame::Connect([4; 32]),
            Frame::Go([5; 32]),
            Frame::Paired([6; 32]),
            Frame::Error(5),
        ] {
            let mut bytes = Vec::new();
            write(&mut bytes, &frame).unwrap();
            assert_eq!(read(&mut bytes.as_slice()).unwrap(), frame);
            for size in 0..bytes.len() {
                assert!(read(&mut &bytes[..size]).is_err());
            }
            for (offset, value) in [(0, 0), (4, 2), (5, 0), (6, 255), (7, 255)] {
                let mut bad = bytes.clone();
                bad[offset] = value;
                assert!(read(&mut bad.as_slice()).is_err());
            }
        }
    }
}
