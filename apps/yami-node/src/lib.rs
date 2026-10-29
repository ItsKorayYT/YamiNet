//! Small document nodes: an in-memory store and a directory-backed site.
//! Optional LAN discovery is provided separately by `ynp-discovery`.
pub mod peers;
mod signing;
mod site;

use std::{
    collections::BTreeMap,
    io::{self, Read, Write},
    thread,
    time::{Duration, Instant},
};
use ynp::{Connection, DocumentData, Frame, Incoming, Message};

pub use signing::{sign_site, site_manifest, verify_site, SIGNATURE_FILE};
pub use site::{Site, PAGE_EXTENSION, STYLE_FILE};

/// Total time one connection may take, so a slow client cannot hold a worker
/// indefinitely by trickling bytes inside the per-operation socket timeouts.
pub const CONNECTION_DEADLINE: Duration = Duration::from_secs(10);
/// Upper bound on concurrently served connections for `serve_concurrently`.
pub const MAX_WORKERS: usize = 32;

/// Anything that can answer a YNP Get. `Ok(None)` becomes a NotFound reply;
/// an error closes the connection without a reply.
pub trait Documents: Sync {
    fn lookup(&self, key: &str) -> io::Result<Option<DocumentData>>;
}
impl Documents for Site {
    fn lookup(&self, key: &str) -> io::Result<Option<DocumentData>> {
        Site::lookup(self, key)
    }
}

#[derive(Default)]
pub struct Store(BTreeMap<String, DocumentData>);
impl Store {
    pub fn insert(&mut self, document: DocumentData) -> io::Result<()> {
        // Validate all wire bounds before serving, without buffering a frame.
        ynp::write_frame(
            &mut io::sink(),
            &Frame {
                request_id: 0,
                message: Message::Document(document.clone()),
            },
        )?;
        self.0.insert(document.key.clone(), document);
        Ok(())
    }
}
impl Documents for Store {
    fn lookup(&self, key: &str) -> io::Result<Option<DocumentData>> {
        Ok(self.0.get(key).cloned())
    }
}

/// What a node sent back for one request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    Document(String),
    NotFound(String),
    Dht,
}

/// Opt-in protocol dispatch AFTER the existing TLS handshake. YNP bytes are
/// replayed unchanged into the original codec; DHT has its own bounded codec.
pub fn serve_connection_with_dht(
    connection: &mut dyn Connection,
    documents: &impl Documents,
    dht: &yami_dht::Service,
) -> io::Result<Reply> {
    let mut prefix = [0; 4];
    connection.read_exact(&mut prefix)?;
    if prefix == yami_dht::MAGIC {
        dht.handle(connection, prefix)?;
        return Ok(Reply::Dht);
    }
    if prefix != *b"YNP\0" {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unknown encrypted protocol prefix",
        ));
    }
    let mut replay = Prefixed {
        prefix: io::Cursor::new(prefix),
        inner: connection,
    };
    serve_connection(&mut replay, documents)
}
struct Prefixed<'a> {
    prefix: io::Cursor<[u8; 4]>,
    inner: &'a mut dyn Connection,
}
impl Read for Prefixed<'_> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        if self.prefix.position() < 4 {
            self.prefix.read(bytes)
        } else {
            self.inner.read(bytes)
        }
    }
}
impl Write for Prefixed<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.inner.write(bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}
impl Connection for Prefixed<'_> {
    fn protection(&self) -> ynp::Protection {
        self.inner.protection()
    }
}

/// One bounded request per connection. A bad client affects only its connection.
pub fn serve_once(listener: &impl Incoming, documents: &impl Documents) -> io::Result<Reply> {
    let mut connection = listener.accept()?;
    serve_connection(&mut *connection, documents)
}
pub fn serve_connection(
    connection: &mut dyn Connection,
    documents: &impl Documents,
) -> io::Result<Reply> {
    let mut connection = Deadline {
        inner: connection,
        until: Instant::now() + CONNECTION_DEADLINE,
    };
    let request = ynp::read_frame(&mut connection)?;
    let Message::Get { key } = request.message else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "node accepts only Get requests",
        ));
    };
    let (message, reply) = match documents.lookup(&key)? {
        Some(doc) if doc.key == key => (Message::Document(doc), Reply::Document(key)),
        Some(_) => return Err(io::Error::other("document source returned another key")),
        None => (Message::NotFound { key: key.clone() }, Reply::NotFound(key)),
    };
    ynp::write_frame(
        &mut connection,
        &Frame {
            request_id: request.request_id,
            message,
        },
    )?;
    Ok(reply)
}

/// Serve forever with a fixed pool of `workers` threads sharing one listener.
/// At most `workers` connections are handled at once, each holding at most one
/// maximum-size frame, so memory and threads stay bounded. Further clients wait
/// in the operating system's accept backlog. `report` sees every outcome.
pub fn serve_concurrently<L, D>(
    listener: &L,
    documents: &D,
    workers: usize,
    report: &(dyn Fn(io::Result<Reply>) + Sync),
) -> io::Result<()>
where
    L: Incoming + Sync,
    D: Documents,
{
    serve_with_handler(
        listener,
        workers,
        &|connection| serve_connection(connection, documents),
        report,
    )
}
pub fn serve_with_handler<L: Incoming + Sync>(
    listener: &L,
    workers: usize,
    handler: &(dyn Fn(&mut dyn Connection) -> io::Result<Reply> + Sync),
    report: &(dyn Fn(io::Result<Reply>) + Sync),
) -> io::Result<()> {
    if !(1..=MAX_WORKERS).contains(&workers) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "worker count must be 1..32",
        ));
    }
    // Workers never exit, so this returns only when no worker could start.
    thread::scope(|scope| {
        let mut started = 0;
        for index in 0..workers {
            let worker = thread::Builder::new()
                .name(format!("yami-node-{index}"))
                .spawn_scoped(scope, move || loop {
                    match listener.accept() {
                        Ok(mut connection) => report(handler(&mut *connection)),
                        Err(error) => {
                            report(Err(error));
                            // Avoid spinning if accepting fails persistently,
                            // for example when file descriptors run out.
                            thread::sleep(Duration::from_millis(50));
                        }
                    }
                });
            match worker {
                Ok(_) => started += 1,
                Err(error) => report(Err(error)),
            }
        }
        if started == 0 {
            return Err(io::Error::other("no node worker could start"));
        }
        Ok(())
    })
}

struct Deadline<'a> {
    inner: &'a mut dyn Connection,
    until: Instant,
}
impl Deadline<'_> {
    fn check(&self) -> io::Result<()> {
        if Instant::now() > self.until {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "connection deadline exceeded",
            ));
        }
        Ok(())
    }
}
impl Read for Deadline<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.check()?;
        self.inner.read(buf)
    }
}
impl Write for Deadline<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.check()?;
        self.inner.write(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn store_rejects_unservable_documents() {
        assert!(Store::default()
            .insert(DocumentData {
                key: "../bad".into(),
                source: String::new(),
                style: String::new()
            })
            .is_err());
    }
}
