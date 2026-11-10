//! Pinned witness TLS and absolute connection deadlines. No identity/attestation fallback.
use crate::{check, Result};
use openssl::{
    ssl::{SslAcceptor, SslConnector, SslFiletype, SslMethod, SslVerifyMode, SslVersion},
    x509::{store::X509StoreBuilder, X509},
};
use serde::{Deserialize, Serialize};
use std::{
    io::{self, Read, Write},
    net::TcpStream,
    path::PathBuf,
    time::{Duration, Instant},
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientTls {
    pub certificate_pem: Vec<u8>,
    pub server_name: String,
}
impl ClientTls {
    pub fn connector(&self) -> Result<SslConnector> {
        check(
            !self.certificate_pem.is_empty()
                && self.certificate_pem.len() <= 16 * 1024
                && !self.server_name.is_empty()
                && self.server_name.len() <= 253
                && self
                    .server_name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b".-:".contains(&b)),
        )?;
        let mut trust = X509StoreBuilder::new()?;
        trust.add_cert(X509::from_pem(&self.certificate_pem)?)?;
        let mut builder = SslConnector::builder(SslMethod::tls_client())?;
        // Replace ambient system roots with the explicitly provisioned anchor.
        builder.set_cert_store(trust.build());
        builder.set_verify(SslVerifyMode::PEER);
        builder.set_min_proto_version(Some(SslVersion::TLS1_2))?;
        Ok(builder.build())
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerTls {
    pub certificate_file: PathBuf,
    pub key_file: PathBuf,
}
impl ServerTls {
    pub fn acceptor(&self) -> Result<SslAcceptor> {
        let metadata = std::fs::symlink_metadata(&self.key_file)?;
        check(metadata.is_file() && metadata.len() <= 16 * 1024)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            check(metadata.permissions().mode() & 0o077 == 0)?;
        }
        let mut builder = SslAcceptor::mozilla_intermediate_v5(SslMethod::tls_server())?;
        builder.set_min_proto_version(Some(SslVersion::TLS1_2))?;
        builder.set_certificate_chain_file(&self.certificate_file)?;
        builder.set_private_key_file(&self.key_file, SslFiletype::PEM)?;
        builder.check_private_key()?;
        Ok(builder.build())
    }
}

/// A trickle of bytes cannot extend the overall handshake/frame lifetime.
#[derive(Debug)]
pub(crate) struct DeadlineStream {
    stream: TcpStream,
    deadline: Instant,
}
impl DeadlineStream {
    pub(crate) fn new(stream: TcpStream) -> Self {
        Self {
            stream,
            deadline: Instant::now() + Duration::from_secs(5),
        }
    }
    fn remaining(&self) -> io::Result<Duration> {
        self.deadline
            .checked_duration_since(Instant::now())
            .filter(|duration| !duration.is_zero())
            .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "witness connection deadline"))
    }
}
impl Read for DeadlineStream {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.stream.set_read_timeout(Some(self.remaining()?))?;
        self.stream.read(buffer)
    }
}
impl Write for DeadlineStream {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.stream.set_write_timeout(Some(self.remaining()?))?;
        self.stream.write(buffer)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.stream.set_write_timeout(Some(self.remaining()?))?;
        self.stream.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn trickled_bytes_do_not_renew_the_absolute_deadline() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let mut sender = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let mut receiver = DeadlineStream::new(listener.accept().unwrap().0);
        receiver.deadline = Instant::now() + Duration::from_millis(150);
        let writer = std::thread::spawn(move || {
            for _ in 0..50 {
                if sender.write_all(&[1]).is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        });
        let start = Instant::now();
        let mut bytes = Vec::new();
        assert!(receiver.read_to_end(&mut bytes).is_err());
        assert!(bytes.len() > 1);
        assert!(start.elapsed() < Duration::from_secs(1));
        drop(receiver);
        writer.join().unwrap();
    }
}
