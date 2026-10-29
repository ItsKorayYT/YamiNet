//! Experimental YNP 0.1: the frame codec, transport traits and the legacy
//! plaintext, unauthenticated, loopback-only TCP transport.
//! This crate provides no identity, encryption or anonymity. The `ynp-tls`
//! crate carries the same frames over TLS 1.3 to a pinned node identity.
use std::{
    io::{self, Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    time::Duration,
};

pub const WARNING: &str =
    "EXPERIMENTAL: plaintext, unauthenticated localhost networking; not secure for sensitive use.";
pub const MAX_KEY: usize = 128;
pub const MAX_DOCUMENT: usize = 256 * 1024;
pub const MAX_STYLE: usize = 16 * 1024;
pub const MAX_PAYLOAD: usize = 2 + MAX_KEY + 4 + MAX_DOCUMENT + 4 + MAX_STYLE;
pub const TIMEOUT: Duration = Duration::from_secs(3);
const HEADER: usize = 16;

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentData {
    pub key: String,
    pub source: String,
    pub style: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    Get { key: String },
    Document(DocumentData),
    NotFound { key: String },
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub request_id: u32,
    pub message: Message,
}

pub fn validate_key(key: &str) -> io::Result<()> {
    if key.is_empty()
        || key.len() > MAX_KEY
        || !key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err(invalid(
            "key must be 1..128 ASCII letters, digits, underscores or hyphens",
        ));
    }
    Ok(())
}
fn put_key(output: &mut Vec<u8>, key: &str) -> io::Result<()> {
    validate_key(key)?;
    output.extend_from_slice(&(key.len() as u16).to_be_bytes());
    output.extend_from_slice(key.as_bytes());
    Ok(())
}
fn put_text(output: &mut Vec<u8>, text: &str, max: usize) -> io::Result<()> {
    if text.len() > max {
        return Err(invalid("text exceeds protocol limit"));
    }
    output.extend_from_slice(&(text.len() as u32).to_be_bytes());
    output.extend_from_slice(text.as_bytes());
    Ok(())
}

pub fn write_frame(writer: &mut (impl Write + ?Sized), frame: &Frame) -> io::Result<()> {
    let mut payload = Vec::new();
    let kind = match &frame.message {
        Message::Get { key } => {
            put_key(&mut payload, key)?;
            1
        }
        Message::Document(doc) => {
            put_key(&mut payload, &doc.key)?;
            put_text(&mut payload, &doc.source, MAX_DOCUMENT)?;
            put_text(&mut payload, &doc.style, MAX_STYLE)?;
            2
        }
        Message::NotFound { key } => {
            put_key(&mut payload, key)?;
            3
        }
    };
    let mut header = [0u8; HEADER];
    header[..4].copy_from_slice(b"YNP\0");
    header[4] = 1;
    header[5] = kind;
    header[8..12].copy_from_slice(&(payload.len() as u32).to_be_bytes());
    header[12..16].copy_from_slice(&frame.request_id.to_be_bytes());
    writer.write_all(&header)?;
    writer.write_all(&payload)?;
    writer.flush()
}

struct Cursor<'a> {
    bytes: &'a [u8],
    offset: usize,
}
impl<'a> Cursor<'a> {
    fn take(&mut self, size: usize) -> io::Result<&'a [u8]> {
        let end = self
            .offset
            .checked_add(size)
            .ok_or_else(|| invalid("length overflow"))?;
        let value = self
            .bytes
            .get(self.offset..end)
            .ok_or_else(|| invalid("field length exceeds frame payload"))?;
        self.offset = end;
        Ok(value)
    }
    fn text(&mut self, size: usize) -> io::Result<String> {
        let bytes = self.take(size)?;
        String::from_utf8(bytes.to_vec()).map_err(|_| invalid("text is not UTF-8"))
    }
    fn key(&mut self) -> io::Result<String> {
        let bytes = self.take(2)?;
        let size = u16::from_be_bytes([bytes[0], bytes[1]]) as usize;
        if size > MAX_KEY {
            return Err(invalid("key exceeds limit"));
        }
        let key = self.text(size)?;
        validate_key(&key)?;
        Ok(key)
    }
    fn long_text(&mut self, max: usize) -> io::Result<String> {
        let bytes = self.take(4)?;
        let size = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
        if size > max {
            return Err(invalid("text exceeds protocol limit"));
        }
        self.text(size)
    }
}

pub fn read_frame(reader: &mut (impl Read + ?Sized)) -> io::Result<Frame> {
    let mut header = [0u8; HEADER];
    reader.read_exact(&mut header)?;
    if &header[..4] != b"YNP\0" {
        return Err(invalid("invalid YNP magic"));
    }
    if header[4] != 1 {
        return Err(invalid("unsupported YNP wire version"));
    }
    if header[6..8] != [0, 0] {
        return Err(invalid("reserved header bits must be zero"));
    }
    let kind = header[5];
    if !(1..=3).contains(&kind) {
        return Err(invalid("unknown message kind"));
    }
    let size = u32::from_be_bytes(header[8..12].try_into().expect("fixed header")) as usize;
    let max = if kind == 2 { MAX_PAYLOAD } else { 2 + MAX_KEY };
    let min = if kind == 2 { 11 } else { 3 };
    if !(min..=max).contains(&size) {
        return Err(invalid("invalid payload length"));
    }
    let request_id = u32::from_be_bytes(header[12..16].try_into().expect("fixed header"));
    let mut payload = vec![0; size];
    reader.read_exact(&mut payload)?;
    let mut cursor = Cursor {
        bytes: &payload,
        offset: 0,
    };
    let key = cursor.key()?;
    let message = match kind {
        1 => Message::Get { key },
        2 => Message::Document(DocumentData {
            key,
            source: cursor.long_text(MAX_DOCUMENT)?,
            style: cursor.long_text(MAX_STYLE)?,
        }),
        _ => Message::NotFound { key },
    };
    if cursor.offset != payload.len() {
        return Err(invalid("trailing bytes inside payload"));
    }
    Ok(Frame {
        request_id,
        message,
    })
}

/// Report observed protection, rather than implying a secure channel exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protection {
    /// Legacy loopback TCP: anyone on the machine can read, alter or
    /// impersonate either side.
    ExperimentalPlaintext,
    /// TLS 1.3 in which the server proved the key of a node ID the client
    /// trusted before connecting (`ynp-tls`). The client is not authenticated.
    Tls13PinnedServer,
}
/// Transports own identity handshakes and channel protection. Each protection
/// state has its own specification and threat model.
pub trait Connection: Read + Write + Send {
    fn protection(&self) -> Protection;
}
impl Connection for TcpStream {
    fn protection(&self) -> Protection {
        Protection::ExperimentalPlaintext
    }
}
pub trait Transport {
    fn connect(&self, address: SocketAddr) -> io::Result<Box<dyn Connection>>;
}
pub trait Incoming {
    fn accept(&self) -> io::Result<Box<dyn Connection>>;
}
/// LEGACY, EXPERIMENTAL AND INSECURE: plaintext and unauthenticated. Kept for
/// local tests and the original demos; use `ynp-tls` for authenticated,
/// encrypted connections.
#[derive(Debug, Default)]
pub struct LoopbackTcp;
/// LEGACY, EXPERIMENTAL AND INSECURE plaintext listener; see [`LoopbackTcp`].
pub struct LocalListener(TcpListener);
/// The localhost-only policy shared by every transport in this release.
pub fn require_loopback(address: SocketAddr) -> io::Result<()> {
    if !address.ip().is_loopback() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "only loopback addresses are allowed",
        ));
    }
    Ok(())
}
/// Per-operation timeouts and no Nagle delay, for every YNP TCP stream.
pub fn configure_stream(stream: &TcpStream) -> io::Result<()> {
    stream.set_read_timeout(Some(TIMEOUT))?;
    stream.set_write_timeout(Some(TIMEOUT))?;
    stream.set_nodelay(true)
}
impl Transport for LoopbackTcp {
    fn connect(&self, address: SocketAddr) -> io::Result<Box<dyn Connection>> {
        require_loopback(address)?;
        let stream = TcpStream::connect_timeout(&address, TIMEOUT)?;
        configure_stream(&stream)?;
        Ok(Box::new(stream))
    }
}
impl LocalListener {
    pub fn bind(address: SocketAddr) -> io::Result<Self> {
        require_loopback(address)?;
        Ok(Self(TcpListener::bind(address)?))
    }
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.0.local_addr()
    }
}
impl Incoming for LocalListener {
    fn accept(&self) -> io::Result<Box<dyn Connection>> {
        let (stream, peer) = self.0.accept()?;
        require_loopback(peer)?;
        configure_stream(&stream)?;
        Ok(Box::new(stream))
    }
}
/// Numeric loopback addresses only; never invokes DNS or follows redirects.
pub fn parse_url(url: &str) -> io::Result<(SocketAddr, String)> {
    let rest = url
        .strip_prefix("ynp://")
        .ok_or_else(|| invalid("expected ynp://numeric-loopback:port/key"))?;
    let (address, key) = rest
        .split_once('/')
        .ok_or_else(|| invalid("missing document key"))?;
    validate_key(key)?;
    let address = address
        .parse::<SocketAddr>()
        .map_err(|_| invalid("expected numeric IP and port"))?;
    require_loopback(address)?;
    if address.port() == 0 {
        return Err(invalid("destination port must be nonzero"));
    }
    Ok((address, key.into()))
}

/// Exactly one request/response per connection in v0.1. No automatic fetching.
pub fn fetch(
    transport: &impl Transport,
    address: SocketAddr,
    key: &str,
) -> io::Result<DocumentData> {
    validate_key(key)?;
    let mut connection = transport.connect(address)?;
    write_frame(
        &mut *connection,
        &Frame {
            request_id: 1,
            message: Message::Get { key: key.into() },
        },
    )?;
    let response = read_frame(&mut *connection)?;
    if response.request_id != 1 {
        return Err(invalid("response request ID mismatch"));
    }
    match response.message {
        Message::Document(doc) if doc.key == key => Ok(doc),
        Message::NotFound { key: returned } if returned == key => Err(io::Error::new(
            io::ErrorKind::NotFound,
            "document not found",
        )),
        _ => Err(invalid("unexpected response kind or document key")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fragmented<T>(T);
    impl<T: Read> Read for Fragmented<T> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let count = buf.len().min(1);
            self.0.read(&mut buf[..count])
        }
    }
    impl<T: Write> Write for Fragmented<T> {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.write(&buf[..buf.len().min(1)])
        }
        fn flush(&mut self) -> io::Result<()> {
            self.0.flush()
        }
    }

    #[test]
    fn fragmented_io_preserves_frame_boundaries() {
        let frame = Frame {
            request_id: 7,
            message: Message::Document(DocumentData {
                key: "fragmented".into(),
                source: "Unicode: café".into(),
                style: "@style 0.1".into(),
            }),
        };
        let mut output = Fragmented(Vec::new());
        write_frame(&mut output, &frame).unwrap();
        write_frame(&mut output, &frame).unwrap();
        let mut input = Fragmented(output.0.as_slice());
        assert_eq!(read_frame(&mut input).unwrap(), frame);
        assert_eq!(read_frame(&mut input).unwrap(), frame);
        assert_eq!(
            read_frame(&mut input).unwrap_err().kind(),
            io::ErrorKind::UnexpectedEof
        );
    }

    #[test]
    fn rejects_style_overflow_and_invalid_document_utf8() {
        let oversized = Frame {
            request_id: 1,
            message: Message::Document(DocumentData {
                key: "a".into(),
                source: String::new(),
                style: "x".repeat(MAX_STYLE + 1),
            }),
        };
        let mut output = Vec::new();
        assert!(write_frame(&mut output, &oversized).is_err());
        assert!(output.is_empty());
        let doc = DocumentData {
            key: "a".into(),
            source: "x".into(),
            style: "y".into(),
        };
        let mut bytes = encoded(Message::Document(doc));
        bytes[23] = 255; // source byte after header, key and source length
        assert!(read_frame(&mut bytes.as_slice()).is_err());
        bytes[23] = b'x';
        bytes[24..28].copy_from_slice(&((MAX_STYLE + 1) as u32).to_be_bytes());
        assert!(read_frame(&mut bytes.as_slice()).is_err());
    }
    fn encoded(message: Message) -> Vec<u8> {
        let mut bytes = Vec::new();
        write_frame(
            &mut bytes,
            &Frame {
                request_id: 0x01020304,
                message,
            },
        )
        .unwrap();
        bytes
    }
    #[test]
    fn golden_get_and_roundtrips() {
        let get = encoded(Message::Get { key: "a".into() });
        assert_eq!(
            get,
            [b'Y', b'N', b'P', 0, 1, 1, 0, 0, 0, 0, 0, 3, 1, 2, 3, 4, 0, 1, b'a']
        );
        for message in [
            Message::Get {
                key: "welcome".into(),
            },
            Message::NotFound {
                key: "absent".into(),
            },
            Message::Document(DocumentData {
                key: "a".into(),
                source: "héllo".into(),
                style: "@style 0.1".into(),
            }),
        ] {
            let bytes = encoded(message.clone());
            assert_eq!(read_frame(&mut bytes.as_slice()).unwrap().message, message);
            for n in 0..bytes.len() {
                assert!(read_frame(&mut &bytes[..n]).is_err(), "truncated at {n}");
            }
        }
    }
    #[test]
    fn rejects_headers_lengths_utf8_and_trailing_data() {
        let good = encoded(Message::Get { key: "a".into() });
        for (offset, value) in [
            (0, b'X'),
            (4, 9),
            (5, 4),
            (6, 1),
            (7, 1),
            (8, 255),
            (11, 0),
            (16, 255),
            (18, 255),
        ] {
            let mut bytes = good.clone();
            bytes[offset] = value;
            assert!(
                read_frame(&mut bytes.as_slice()).is_err(),
                "offset {offset}"
            );
        }
        let mut trailing = good.clone();
        trailing[11] = 4;
        trailing.push(0);
        assert!(read_frame(&mut trailing.as_slice()).is_err());
        let mut doc = encoded(Message::Document(DocumentData {
            key: "a".into(),
            source: "text".into(),
            style: "".into(),
        }));
        doc[19..23].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(read_frame(&mut doc.as_slice()).is_err());
        for key in ["", "../escape", "é", "a/b", "with space"] {
            assert!(validate_key(key).is_err());
        }
    }
    #[test]
    fn size_boundaries() {
        let mut doc = DocumentData {
            key: "k".repeat(MAX_KEY),
            source: "x".repeat(MAX_DOCUMENT),
            style: "y".repeat(MAX_STYLE),
        };
        let bytes = encoded(Message::Document(doc.clone()));
        assert_eq!(bytes.len(), HEADER + MAX_PAYLOAD);
        assert_eq!(
            read_frame(&mut bytes.as_slice()).unwrap().message,
            Message::Document(doc.clone())
        );
        doc.source.push('x');
        assert!(write_frame(
            &mut Vec::new(),
            &Frame {
                request_id: 1,
                message: Message::Document(doc)
            }
        )
        .is_err());
    }
    #[test]
    fn no_remote_or_ambiguous_addresses() {
        for url in [
            "http://127.0.0.1:9/a",
            "ynp://example.com:9/a",
            "ynp://192.0.2.1:9/a",
            "ynp://0.0.0.0:9/a",
            "ynp://127.0.0.1:0/a",
            "ynp://127.0.0.1:9/../a",
        ] {
            assert!(parse_url(url).is_err(), "{url}");
        }
        assert!(parse_url("ynp://[::1]:7301/a").is_ok());
        assert!(LoopbackTcp.connect("192.0.2.1:9".parse().unwrap()).is_err());
        assert!(LocalListener::bind("0.0.0.0:0".parse().unwrap()).is_err());
    }
    struct MockTransport(Vec<u8>);
    struct MockConnection {
        input: io::Cursor<Vec<u8>>,
        output: Vec<u8>,
    }
    impl Read for MockConnection {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.input.read(buf)
        }
    }
    impl Write for MockConnection {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.output.write(buf)
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    impl Connection for MockConnection {
        fn protection(&self) -> Protection {
            Protection::ExperimentalPlaintext
        }
    }
    impl Transport for MockTransport {
        fn connect(&self, _: SocketAddr) -> io::Result<Box<dyn Connection>> {
            Ok(Box::new(MockConnection {
                input: io::Cursor::new(self.0.clone()),
                output: Vec::new(),
            }))
        }
    }
    #[test]
    fn correlation_is_checked_with_alternate_transport() {
        let address = "127.0.0.1:1".parse().unwrap();
        let mut bytes = Vec::new();
        write_frame(
            &mut bytes,
            &Frame {
                request_id: 2,
                message: Message::NotFound { key: "a".into() },
            },
        )
        .unwrap();
        assert_eq!(
            fetch(&MockTransport(bytes), address, "a")
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
        let mut bytes = Vec::new();
        write_frame(
            &mut bytes,
            &Frame {
                request_id: 1,
                message: Message::Document(DocumentData {
                    key: "wrong".into(),
                    source: "".into(),
                    style: "".into(),
                }),
            },
        )
        .unwrap();
        assert_eq!(
            fetch(&MockTransport(bytes), address, "a")
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
    }
}
