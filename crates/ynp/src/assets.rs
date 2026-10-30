//! Explicit YNP wire v2: PNG assets only. Document frames remain wire v1.
use super::*;
pub const MAX_ASSET_KEY: usize = 64;
pub const MAX_ASSET_BYTES: usize = 256 * 1024;
pub const MAX_ASSET_PAYLOAD: usize = 2 + MAX_ASSET_KEY + 1 + 4 + MAX_ASSET_BYTES;
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetData {
    pub key: String,
    pub bytes: Vec<u8>,
}
pub fn validate_asset_key(key: &str) -> io::Result<()> {
    validate_key(key)?;
    let folded = key.to_ascii_lowercase();
    if key.len() > MAX_ASSET_KEY
        || ["con", "prn", "aux", "nul"].contains(&folded.as_str())
        || (folded.len() == 4
            && (folded.starts_with("com") || folded.starts_with("lpt"))
            && folded.as_bytes()[3].is_ascii_digit())
    {
        return Err(invalid("asset key must be 1..64 safe ASCII letters, digits, underscores or hyphens; device names are forbidden"));
    }
    Ok(())
}
pub(super) fn write_asset(writer: &mut (impl Write + ?Sized), frame: &Frame) -> io::Result<()> {
    let (key, kind) = match &frame.message {
        Message::GetAsset { key } => (key, 1),
        Message::Asset(asset) => (&asset.key, 2),
        Message::AssetNotFound { key } => (key, 3),
        _ => return Err(invalid("not an asset frame")),
    };
    validate_asset_key(key)?;
    let mut body = Vec::new();
    put_key(&mut body, key)?;
    if let Message::Asset(asset) = &frame.message {
        if asset.bytes.len() > MAX_ASSET_BYTES || asset.bytes.is_empty() {
            return Err(invalid("PNG asset exceeds byte bounds"));
        }
        body.push(1);
        body.extend((asset.bytes.len() as u32).to_be_bytes());
        body.extend(&asset.bytes);
    }
    let mut header = [0; HEADER];
    header[..4].copy_from_slice(b"YNP\0");
    header[4] = 2;
    header[5] = kind;
    header[8..12].copy_from_slice(&(body.len() as u32).to_be_bytes());
    header[12..].copy_from_slice(&frame.request_id.to_be_bytes());
    writer.write_all(&header)?;
    writer.write_all(&body)?;
    writer.flush()
}
pub(super) fn read_asset(
    reader: &mut (impl Read + ?Sized),
    header: [u8; HEADER],
    limit: usize,
) -> io::Result<Frame> {
    let kind = header[5];
    if !(1..=3).contains(&kind) {
        return Err(invalid("unsupported asset message kind"));
    }
    let size = u32::from_be_bytes(header[8..12].try_into().unwrap()) as usize;
    let max = if kind == 2 {
        2 + MAX_ASSET_KEY + 5 + limit.min(MAX_ASSET_BYTES)
    } else {
        2 + MAX_ASSET_KEY
    };
    if size < 3 || size > max {
        return Err(invalid("asset frame exceeds resource budget"));
    }
    let mut payload = vec![0; size];
    reader.read_exact(&mut payload)?;
    let mut cursor = Cursor {
        bytes: &payload,
        offset: 0,
    };
    let key = cursor.key()?;
    validate_asset_key(&key)?;
    let message = match kind {
        1 => Message::GetAsset { key },
        3 => Message::AssetNotFound { key },
        _ => {
            if cursor.take(1)?[0] != 1 {
                return Err(invalid("only PNG assets are supported"));
            }
            let size = u32::from_be_bytes(cursor.take(4)?.try_into().unwrap()) as usize;
            if size == 0 || size > limit.min(MAX_ASSET_BYTES) {
                return Err(invalid("asset byte limit exceeded"));
            }
            Message::Asset(AssetData {
                key,
                bytes: cursor.take(size)?.to_vec(),
            })
        }
    };
    if cursor.offset != payload.len() {
        return Err(invalid("trailing asset payload bytes"));
    }
    Ok(Frame {
        request_id: u32::from_be_bytes(header[12..].try_into().unwrap()),
        message,
    })
}
pub fn request_asset(
    connection: &mut dyn Connection,
    key: &str,
    limit: usize,
) -> io::Result<AssetData> {
    validate_asset_key(key)?;
    if connection.protection() != Protection::Tls13PinnedServer {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "PNG assets require the authenticated parent identity over TLS 1.3",
        ));
    }
    if limit == 0 || limit > MAX_ASSET_BYTES {
        return Err(invalid("invalid per-asset budget"));
    }
    write_frame(
        connection,
        &Frame {
            request_id: 2,
            message: Message::GetAsset { key: key.into() },
        },
    )?;
    let response = read_frame_limited(connection, limit, true)?;
    if response.request_id != 2 {
        return Err(invalid("asset request ID mismatch"));
    }
    match response.message {
        Message::Asset(asset) if asset.key == key => Ok(asset),
        Message::AssetNotFound { key: returned } if returned == key => Err(io::Error::new(
            io::ErrorKind::NotFound,
            "image asset not found",
        )),
        _ => Err(invalid("asset response identity/key/type mismatch")),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn asset_frames_are_versioned_and_strictly_bounded() {
        for message in [
            Message::GetAsset {
                key: "cover".into(),
            },
            Message::Asset(AssetData {
                key: "cover".into(),
                bytes: vec![1, 2, 3],
            }),
            Message::AssetNotFound {
                key: "cover".into(),
            },
        ] {
            let frame = Frame {
                request_id: 2,
                message,
            };
            let mut data = Vec::new();
            write_frame(&mut data, &frame).unwrap();
            assert_eq!(data[4], 2);
            assert_eq!(read_frame(&mut data.as_slice()).unwrap(), frame);
            for end in 0..data.len() {
                assert!(read_frame(&mut &data[..end]).is_err());
            }
            let mut huge = data.clone();
            huge[8..12].copy_from_slice(&u32::MAX.to_be_bytes());
            assert!(read_frame(&mut huge.as_slice()).is_err());
        }
        for key in [
            "../secret",
            "cover.png",
            "CON",
            "aux",
            "lpt1",
            "a/b",
            "yn1-node@host",
            "",
            &"x".repeat(65),
        ] {
            assert!(validate_asset_key(key).is_err());
        }
        let mut bad = Vec::new();
        write_frame(
            &mut bad,
            &Frame {
                request_id: 2,
                message: Message::Asset(AssetData {
                    key: "a".into(),
                    bytes: vec![1; 10],
                }),
            },
        )
        .unwrap();
        bad[19] = 2;
        assert!(read_frame(&mut bad.as_slice()).is_err());
        let mut too_small = Vec::new();
        write_frame(
            &mut too_small,
            &Frame {
                request_id: 2,
                message: Message::Asset(AssetData {
                    key: "a".into(),
                    bytes: vec![1; 10],
                }),
            },
        )
        .unwrap();
        assert!(read_frame_limited(&mut too_small.as_slice(), 5, true).is_err());
    }
    #[test]
    fn plaintext_asset_requests_are_rejected_before_writing() {
        struct Plain(Vec<u8>);
        impl Read for Plain {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Ok(0)
            }
        }
        impl Write for Plain {
            fn write(&mut self, data: &[u8]) -> io::Result<usize> {
                self.0.extend(data);
                Ok(data.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        impl Connection for Plain {
            fn protection(&self) -> Protection {
                Protection::ExperimentalPlaintext
            }
        }
        let mut plain = Plain(Vec::new());
        assert!(request_asset(&mut plain, "cover", 100).is_err());
        assert!(plain.0.is_empty());
    }
}
