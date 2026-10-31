//! YNP wire v3: same-owner, explicitly requested game resources. No paths or URLs.
use super::*;
pub const MAX_GAME_MANIFEST: usize = 4096;
pub const MAX_GAME_MODULE: usize = 1024 * 1024;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameResource {
    Manifest = 1,
    Module = 2,
}
impl GameResource {
    pub fn limit(self) -> usize {
        match self {
            Self::Manifest => MAX_GAME_MANIFEST,
            Self::Module => MAX_GAME_MODULE,
        }
    }
    fn decode(value: u8) -> io::Result<Self> {
        match value {
            1 => Ok(Self::Manifest),
            2 => Ok(Self::Module),
            _ => Err(invalid("unsupported game resource type")),
        }
    }
}
pub(super) fn write_game(writer: &mut (impl Write + ?Sized), frame: &Frame) -> io::Result<()> {
    let (key, resource, kind) = match &frame.message {
        Message::GetGame { key, resource } => (key, *resource, 1),
        Message::Game { key, resource, .. } => (key, *resource, 2),
        Message::GameNotFound { key, resource } => (key, *resource, 3),
        _ => return Err(invalid("not a game frame")),
    };
    validate_asset_key(key)?;
    let mut body = Vec::new();
    put_key(&mut body, key)?;
    body.push(resource as u8);
    if let Message::Game { bytes, .. } = &frame.message {
        if bytes.is_empty() || bytes.len() > resource.limit() {
            return Err(invalid("game resource exceeds byte bounds"));
        }
        body.extend((bytes.len() as u32).to_be_bytes());
        body.extend(bytes);
    }
    let mut header = [0; HEADER];
    header[..4].copy_from_slice(b"YNP\0");
    header[4] = 3;
    header[5] = kind;
    header[8..12].copy_from_slice(&(body.len() as u32).to_be_bytes());
    header[12..].copy_from_slice(&frame.request_id.to_be_bytes());
    writer.write_all(&header)?;
    writer.write_all(&body)?;
    writer.flush()
}
pub(super) fn read_game(
    reader: &mut (impl Read + ?Sized),
    header: [u8; HEADER],
    limit: usize,
) -> io::Result<Frame> {
    let kind = header[5];
    if !(1..=3).contains(&kind) {
        return Err(invalid("unsupported game message kind"));
    }
    let size = u32::from_be_bytes(header[8..12].try_into().unwrap()) as usize;
    // Read the small prefix before allocating the resource body. Type-specific bounds
    // apply even to unsolicited responses received by the node dispatcher.
    let mut key_size = [0; 2];
    if size < 4 || size > 2 + MAX_ASSET_KEY + 5 + limit.min(MAX_GAME_MODULE) {
        return Err(invalid("game frame exceeds resource budget"));
    }
    reader.read_exact(&mut key_size)?;
    let length = u16::from_be_bytes(key_size) as usize;
    if length == 0 || length > MAX_ASSET_KEY || size < 3 + length {
        return Err(invalid("invalid game key length"));
    }
    let mut key = vec![0; length];
    reader.read_exact(&mut key)?;
    let key = String::from_utf8(key).map_err(|_| invalid("invalid game key encoding"))?;
    validate_asset_key(&key)?;
    let mut tag = [0];
    reader.read_exact(&mut tag)?;
    let resource = GameResource::decode(tag[0])?;
    let message = if kind == 2 {
        let mut count = [0; 4];
        if size < 7 + length {
            return Err(invalid("truncated game resource"));
        }
        reader.read_exact(&mut count)?;
        let count = u32::from_be_bytes(count) as usize;
        if count == 0 || count > resource.limit().min(limit) || size != 7 + length + count {
            return Err(invalid("game resource size or trailing bytes invalid"));
        }
        let mut bytes = vec![0; count];
        reader.read_exact(&mut bytes)?;
        Message::Game {
            key,
            resource,
            bytes,
        }
    } else {
        if size != 3 + length {
            return Err(invalid("trailing game request bytes"));
        }
        if kind == 1 {
            Message::GetGame { key, resource }
        } else {
            Message::GameNotFound { key, resource }
        }
    };
    Ok(Frame {
        request_id: u32::from_be_bytes(header[12..].try_into().unwrap()),
        message,
    })
}
pub fn request_game(
    connection: &mut dyn Connection,
    key: &str,
    resource: GameResource,
) -> io::Result<Vec<u8>> {
    validate_asset_key(key)?;
    if connection.protection() != Protection::Tls13PinnedServer {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "games require pinned TLS 1.3",
        ));
    }
    write_frame(
        connection,
        &Frame {
            request_id: 3,
            message: Message::GetGame {
                key: key.into(),
                resource,
            },
        },
    )?;
    let mut header = [0; HEADER];
    connection.read_exact(&mut header)?;
    if &header[..4] != b"YNP\0"
        || header[4] != 3
        || header[6..8] != [0, 0]
        || header[12..] != 3u32.to_be_bytes()
    {
        return Err(invalid(
            "wrong game response version, reserved bits or request ID",
        ));
    }
    match read_game(connection, header, resource.limit())?.message {
        Message::Game {
            key: returned,
            resource: returned_type,
            bytes,
        } if returned == key && returned_type == resource => Ok(bytes),
        Message::GameNotFound {
            key: returned,
            resource: returned_type,
        } if returned == key && returned_type == resource => Err(io::Error::new(
            io::ErrorKind::NotFound,
            "hosted game resource not found",
        )),
        _ => Err(invalid("game response key/type mismatch")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Stream {
        input: io::Cursor<Vec<u8>>,
        output: Vec<u8>,
        protection: Protection,
    }
    impl Read for Stream {
        fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
            self.input.read(bytes)
        }
    }
    impl Write for Stream {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.output.extend(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    impl Connection for Stream {
        fn protection(&self) -> Protection {
            self.protection
        }
    }
    fn encoded(message: Message) -> Vec<u8> {
        let mut bytes = Vec::new();
        write_frame(
            &mut bytes,
            &Frame {
                request_id: 3,
                message,
            },
        )
        .unwrap();
        bytes
    }
    fn response() -> Vec<u8> {
        encoded(Message::Game {
            key: "prism".into(),
            resource: GameResource::Manifest,
            bytes: vec![b'x'; 20],
        })
    }
    fn request(bytes: Vec<u8>) -> io::Result<Vec<u8>> {
        request_game(
            &mut Stream {
                input: io::Cursor::new(bytes),
                output: Vec::new(),
                protection: Protection::Tls13PinnedServer,
            },
            "prism",
            GameResource::Manifest,
        )
    }
    #[test]
    fn v3_frames_roundtrip_and_reject_truncation_versions_types_lengths_and_trailing_data() {
        for resource in [GameResource::Manifest, GameResource::Module] {
            for message in [
                Message::GetGame {
                    key: "prism".into(),
                    resource,
                },
                Message::Game {
                    key: "prism".into(),
                    resource,
                    bytes: vec![0; resource.limit()],
                },
                Message::GameNotFound {
                    key: "prism".into(),
                    resource,
                },
            ] {
                let frame = Frame {
                    request_id: 3,
                    message,
                };
                let bytes = encoded(frame.message.clone());
                assert_eq!(bytes[4], 3);
                assert_eq!(read_frame(&mut bytes.as_slice()).unwrap(), frame);
                for end in [0, 4, 16, bytes.len() - 1] {
                    assert!(read_frame(&mut &bytes[..end]).is_err());
                }
            }
        }
        assert_eq!(request(response()).unwrap(), vec![b'x'; 20]);
        for (offset, value) in [
            (4, 1),
            (4, 2),
            (5, 0),
            (6, 1),
            (15, 4),
            (23, 9),
            (27, 21),
            (18, b'z'),
        ] {
            let mut bytes = response();
            bytes[offset] = value;
            assert!(request(bytes).is_err(), "offset {offset}");
        }
        let mut bytes = response();
        bytes[8..12].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(request(bytes).is_err());
        let mut bytes = response();
        bytes.push(0);
        let size = (bytes.len() - 16) as u32;
        bytes[8..12].copy_from_slice(&size.to_be_bytes());
        assert!(request(bytes).is_err());
        assert!(request(encoded(Message::Game {
            key: "prism".into(),
            resource: GameResource::Module,
            bytes: vec![1; 20]
        }))
        .is_err());
        assert_eq!(
            request(encoded(Message::GameNotFound {
                key: "prism".into(),
                resource: GameResource::Manifest
            }))
            .unwrap_err()
            .kind(),
            io::ErrorKind::NotFound
        );
        assert!(request(encoded(Message::Game {
            key: "prism".into(),
            resource: GameResource::Module,
            bytes: vec![1; MAX_GAME_MANIFEST + 1]
        }))
        .is_err());
        assert!(request(encoded(Message::Asset(AssetData {
            key: "prism".into(),
            bytes: vec![1]
        })))
        .is_err());
    }
    #[test]
    fn plaintext_is_rejected_before_any_request_bytes() {
        let mut stream = Stream {
            input: io::Cursor::new(response()),
            output: Vec::new(),
            protection: Protection::ExperimentalPlaintext,
        };
        assert_eq!(
            request_game(&mut stream, "prism", GameResource::Manifest)
                .unwrap_err()
                .kind(),
            io::ErrorKind::PermissionDenied
        );
        assert!(stream.output.is_empty());
    }
}
