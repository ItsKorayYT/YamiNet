//! YNP wire v4: a bounded opaque YNS v1 envelope. Other wire versions are unchanged.
use super::*;
pub const MAX_NAMING_BYTES: usize = 36 * 1024;
pub(super) fn write_naming(writer: &mut (impl Write + ?Sized), frame: &Frame) -> io::Result<()> {
    let (kind, bytes) = match &frame.message {
        Message::NamingRequest { bytes } => (1, bytes),
        Message::NamingResponse { bytes } => (2, bytes),
        _ => return Err(invalid("not a naming frame")),
    };
    if bytes.is_empty() || bytes.len() > MAX_NAMING_BYTES {
        return Err(invalid("naming payload exceeds budget"));
    }
    let mut header = [0; HEADER];
    header[..4].copy_from_slice(b"YNP\0");
    header[4] = 4;
    header[5] = kind;
    header[8..12].copy_from_slice(&(bytes.len() as u32).to_be_bytes());
    header[12..].copy_from_slice(&frame.request_id.to_be_bytes());
    writer.write_all(&header)?;
    writer.write_all(bytes)?;
    writer.flush()
}
pub(super) fn read_naming(
    reader: &mut (impl Read + ?Sized),
    header: [u8; HEADER],
) -> io::Result<Frame> {
    let size = u32::from_be_bytes(header[8..12].try_into().unwrap()) as usize;
    if !(1..=2).contains(&header[5]) || !(1..=MAX_NAMING_BYTES).contains(&size) {
        return Err(invalid("invalid naming frame bounds or kind"));
    }
    let mut bytes = vec![0; size];
    reader.read_exact(&mut bytes)?;
    Ok(Frame {
        request_id: u32::from_be_bytes(header[12..].try_into().unwrap()),
        message: if header[5] == 1 {
            Message::NamingRequest { bytes }
        } else {
            Message::NamingResponse { bytes }
        },
    })
}
/// Exactly one exchange on an already destination-pinned connection.
pub fn request_naming(connection: &mut dyn Connection, bytes: Vec<u8>) -> io::Result<Vec<u8>> {
    if connection.protection() != Protection::Tls13PinnedServer {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "naming requires pinned TLS",
        ));
    }
    write_frame(
        connection,
        &Frame {
            request_id: 4,
            message: Message::NamingRequest { bytes },
        },
    )?;
    // Do not let an adversarial mirror select a larger document/game codec.
    let mut header = [0; HEADER];
    connection.read_exact(&mut header)?;
    if header[..4] != *b"YNP\0"
        || header[4] != 4
        || header[5] != 2
        || header[6..8] != [0, 0]
        || u32::from_be_bytes(header[12..].try_into().unwrap()) != 4
    {
        return Err(invalid("unexpected naming response header"));
    }
    let response = read_naming(connection, header)?;
    match response {
        Frame {
            request_id: 4,
            message: Message::NamingResponse { bytes },
        } => Ok(bytes),
        _ => Err(invalid("unexpected naming reply or request ID")),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounds_are_checked_before_reading_the_body() {
        let mut header = [0; HEADER];
        header[..4].copy_from_slice(b"YNP\0");
        header[4] = 4;
        header[5] = 1;
        for size in [0, MAX_NAMING_BYTES + 1, usize::MAX.min(u32::MAX as usize)] {
            header[8..12].copy_from_slice(&(size as u32).to_be_bytes());
            assert_eq!(
                read_frame(&mut &header[..]).unwrap_err().kind(),
                io::ErrorKind::InvalidData
            );
        }
        let frame = Frame {
            request_id: 4,
            message: Message::NamingResponse {
                bytes: vec![b'x'; MAX_NAMING_BYTES],
            },
        };
        let mut bytes = Vec::new();
        write_frame(&mut bytes, &frame).unwrap();
        assert_eq!(read_frame(&mut &bytes[..]).unwrap(), frame);
    }
}

#[cfg(test)]
mod exchange_tests {
    use super::*;
    struct Memory {
        incoming: io::Cursor<Vec<u8>>,
        written: Vec<u8>,
        protected: bool,
    }
    impl Read for Memory {
        fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
            self.incoming.read(b)
        }
    }
    impl Write for Memory {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            self.written.extend(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    impl Connection for Memory {
        fn protection(&self) -> Protection {
            if self.protected {
                Protection::Tls13PinnedServer
            } else {
                Protection::ExperimentalPlaintext
            }
        }
    }
    #[test]
    fn plaintext_and_uncorrelated_or_reflected_replies_are_rejected() {
        let mut c = Memory {
            incoming: io::Cursor::new(Vec::new()),
            written: Vec::new(),
            protected: false,
        };
        assert!(request_naming(&mut c, b"{}".to_vec()).is_err());
        assert!(c.written.is_empty());
        let mut game_header = [0; HEADER];
        game_header[..4].copy_from_slice(b"YNP\0");
        game_header[4] = 3;
        game_header[5] = 2;
        game_header[8..12].copy_from_slice(&(8_u32 * 1024 * 1024).to_be_bytes());
        game_header[12..].copy_from_slice(&4_u32.to_be_bytes());
        c.incoming = io::Cursor::new(game_header.to_vec());
        c.protected = true;
        assert_eq!(
            request_naming(&mut c, b"{}".to_vec()).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        for frame in [
            Frame {
                request_id: 3,
                message: Message::NamingResponse {
                    bytes: b"{}".to_vec(),
                },
            },
            Frame {
                request_id: 4,
                message: Message::NamingRequest {
                    bytes: b"{}".to_vec(),
                },
            },
        ] {
            let mut bytes = Vec::new();
            write_frame(&mut bytes, &frame).unwrap();
            c.incoming = io::Cursor::new(bytes);
            c.protected = true;
            assert!(request_naming(&mut c, b"{}".to_vec()).is_err());
        }
    }
}
