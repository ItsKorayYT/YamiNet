//! Deterministic stream fragmentation, latency and connection loss. This is not
//! IP packet loss or a physical NAT test: TCP handles packet retransmission.
use std::{
    io::{self, Read, Write},
    net::{Shutdown, TcpListener, TcpStream},
    sync::Arc,
    thread,
    time::{Duration, Instant},
};
use yami_identity::NodeIdentity;
use ynp_tls::{ByteStream, TlsAcceptor, TlsTransport};
struct Impaired {
    socket: TcpStream,
    fragment: usize,
    delay: Duration,
    remaining: Option<usize>,
    deadline: Option<Instant>,
}
impl Impaired {
    fn ready(&self) -> io::Result<()> {
        if self.remaining == Some(0) {
            return Err(io::Error::new(
                io::ErrorKind::ConnectionReset,
                "injected connection loss",
            ));
        }
        if self
            .deadline
            .is_some_and(|d| Instant::now() + self.delay >= d)
        {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "injected latency exhausted deadline",
            ));
        }
        thread::sleep(self.delay);
        Ok(())
    }
    fn count(&mut self, n: usize) {
        if let Some(remaining) = &mut self.remaining {
            *remaining = remaining.saturating_sub(n);
        }
    }
}
impl Read for Impaired {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.ready()?;
        let length = bytes
            .len()
            .min(self.fragment)
            .min(self.remaining.unwrap_or(usize::MAX));
        let n = self.socket.read(&mut bytes[..length])?;
        self.count(n);
        Ok(n)
    }
}
impl Write for Impaired {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.ready()?;
        let length = bytes
            .len()
            .min(self.fragment)
            .min(self.remaining.unwrap_or(usize::MAX));
        let n = self.socket.write(&bytes[..length])?;
        self.count(n);
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.socket.flush()
    }
}
impl ByteStream for Impaired {
    fn read_timeout(&mut self, timeout: Duration) -> io::Result<()> {
        self.socket.set_read_timeout(Some(timeout))
    }
    fn write_timeout(&mut self, timeout: Duration) -> io::Result<()> {
        self.socket.set_write_timeout(Some(timeout))
    }
    fn close(&mut self) -> io::Result<()> {
        self.socket.shutdown(Shutdown::Both)
    }
    fn limit_deadline(&mut self, deadline: Instant) -> io::Result<()> {
        self.deadline = Some(self.deadline.map_or(deadline, |old| old.min(deadline)));
        Ok(())
    }
}
#[test]
fn tls_and_ynp_survive_fragmentation_and_latency_but_fail_closed_on_connection_loss() {
    for reset in [false, true] {
        let identity = Arc::new(NodeIdentity::generate().unwrap());
        let id = *identity.id();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (socket, _) = listener.accept().unwrap();
            let acceptor = TlsAcceptor::new(identity).unwrap();
            let Ok(mut connection) = acceptor.accept_over(Box::new(socket), Duration::from_secs(3))
            else {
                return false;
            };
            let Ok(request) = ynp::read_frame(&mut *connection) else {
                return false;
            };
            assert!(matches!(request.message, ynp::Message::Get { .. }));
            ynp::write_frame(
                &mut *connection,
                &ynp::Frame {
                    request_id: request.request_id,
                    message: ynp::Message::Document(ynp::DocumentData {
                        key: "home".into(),
                        source: "@page 0.1\n  title Delayed TLS\n= Still authenticated\n".into(),
                        style: String::new(),
                    }),
                },
            )
            .is_ok()
        });
        let stream = Impaired {
            socket: TcpStream::connect(address).unwrap(),
            fragment: 17,
            delay: Duration::from_millis(1),
            remaining: reset.then_some(128),
            deadline: None,
        };
        let result = TlsTransport::new(id)
            .unwrap()
            .connect_over(Box::new(stream), Duration::from_secs(3))
            .and_then(|mut connection| ynp::request(&mut *connection, "home"));
        if reset {
            assert!(result.is_err());
        } else {
            assert!(result.unwrap().source.contains("Still authenticated"));
        }
        assert_eq!(server.join().unwrap(), !reset);
    }
}
#[test]
fn latency_does_not_extend_a_short_tls_deadline() {
    let identity = Arc::new(NodeIdentity::generate().unwrap());
    let id = *identity.id();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (socket, _) = listener.accept().unwrap();
        let _ = TlsAcceptor::new(identity)
            .unwrap()
            .accept_over(Box::new(socket), Duration::from_secs(1));
    });
    let stream = Impaired {
        socket: TcpStream::connect(address).unwrap(),
        fragment: 1,
        delay: Duration::from_millis(5),
        remaining: None,
        deadline: None,
    };
    let started = Instant::now();
    assert!(TlsTransport::new(id)
        .unwrap()
        .connect_over(Box::new(stream), Duration::from_millis(80))
        .is_err());
    assert!(started.elapsed() < Duration::from_millis(400));
    server.join().unwrap();
}
