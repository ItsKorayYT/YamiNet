//! User-activated, in-memory hosted delivery. Game workers share the four-session cap.
use crate::resources::Origin;
use std::{
    io::{self, Read, Write},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
#[derive(Debug, Clone)]
pub enum Source {
    Local(PathBuf),
    Hosted { origin: Origin, key: String },
}
impl Source {
    pub fn launch(&self, wake: Arc<dyn Fn() + Send + Sync>) -> Result<yami_games::Session, String> {
        match self.clone() {
            Self::Local(path) => yami_games::Session::launch(path, wake),
            Self::Hosted { origin, key } => yami_games::Session::launch_with(
                format!("Loading game: {key}"),
                move |cancel| fetch(&origin, &key, cancel),
                wake,
            ),
        }
    }
}
const TRANSFER_TIME: Duration = Duration::from_secs(8);
// Includes both frame headers and maximum keys; failed requests consume this budget too.
const TRANSFER_BYTES: usize = ynp::MAX_GAME_MANIFEST + ynp::MAX_GAME_MODULE + 2 * (16 + 2 + 64 + 5);
struct Transfer<'a> {
    inner: Box<dyn ynp::Connection>,
    cancel: &'a AtomicBool,
    until: Instant,
    remaining: &'a mut usize,
}
impl Transfer<'_> {
    fn check(&self) -> io::Result<()> {
        if self.cancel.load(Ordering::Acquire) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "Game load cancelled",
            ));
        }
        if Instant::now() >= self.until {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Game transfer deadline exceeded",
            ));
        }
        Ok(())
    }
}
impl Read for Transfer<'_> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.check()?;
        if *self.remaining == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Game transfer byte budget exhausted",
            ));
        }
        let length = bytes.len().min(*self.remaining);
        let count = self.inner.read(&mut bytes[..length])?;
        *self.remaining -= count;
        Ok(count)
    }
}
impl Write for Transfer<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.check()?;
        self.inner.write(bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.check()?;
        self.inner.flush()
    }
}
impl ynp::Connection for Transfer<'_> {
    fn protection(&self) -> ynp::Protection {
        self.inner.protection()
    }
}
pub fn fetch(
    origin: &Origin,
    key: &str,
    cancel: &AtomicBool,
) -> Result<(yami_games::Manifest, Vec<u8>), String> {
    let until = Instant::now() + TRANSFER_TIME;
    let mut remaining = TRANSFER_BYTES;
    let mut request = |key: &str, resource: ynp::GameResource| -> Result<Vec<u8>, String> {
        if cancel.load(Ordering::Acquire) {
            return Err("Game load cancelled".into());
        }
        let budget = until.saturating_duration_since(Instant::now());
        if budget.is_zero() {
            return Err("Game transfer deadline exceeded".into());
        }
        // Retain the exact parent destination and relay pins. No plaintext fallback,
        // redirects, cross-owner references or executable disk cache.
        let inner = match origin.connect(budget) {
            Ok(connection) => connection,
            // Relay registrations are consumed by one connection and renewed after
            // 250..313 ms. Permit one reconnect for a refused relay pairing only;
            // identity mismatches and failed/malformed resource transfers never retry.
            Err(error)
                if origin.relay.is_some() && error.kind() == io::ErrorKind::ConnectionRefused =>
            {
                let mut entropy = [0; 8];
                getrandom::fill(&mut entropy).map_err(|e| e.to_string())?;
                let delay = crate::reliability::backoff(1, u64::from_ne_bytes(entropy));
                let retry_at = until.min(Instant::now() + delay);
                while Instant::now() < retry_at {
                    if cancel.load(Ordering::Acquire) {
                        return Err("Game load cancelled".into());
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                let remaining = until.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Err("Game transfer deadline exceeded".into());
                }
                origin
                    .connect(remaining)
                    .map_err(|e| format!("Cannot authenticate game publisher/relay: {e}"))?
            }
            Err(error) => return Err(format!("Cannot authenticate game publisher/relay: {error}")),
        };
        let mut transfer = Transfer {
            inner,
            cancel,
            until,
            remaining: &mut remaining,
        };
        ynp::request_game(&mut transfer, key, resource)
            .map_err(|e| format!("Game resource unavailable or invalid: {e}"))
    };
    let manifest = yami_games::HostedManifest::parse(&request(key, ynp::GameResource::Manifest)?)?;
    let bytes = request(&manifest.module.key, ynp::GameResource::Module)?;
    if cancel.load(Ordering::Acquire) {
        return Err("Game load cancelled".into());
    }
    manifest.validate_module(&bytes)?;
    Ok((manifest.runtime_manifest(), bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Bytes(std::io::Cursor<Vec<u8>>);
    impl Read for Bytes {
        fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
            self.0.read(b)
        }
    }
    impl Write for Bytes {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            Ok(b.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    impl ynp::Connection for Bytes {
        fn protection(&self) -> ynp::Protection {
            ynp::Protection::Tls13PinnedServer
        }
    }
    #[test]
    fn transfer_charges_partial_reads_and_enforces_cancellation_and_deadline() {
        let cancel = AtomicBool::new(false);
        let mut remaining = 17;
        let mut transfer = Transfer {
            inner: Box::new(Bytes(std::io::Cursor::new(vec![0; 100]))),
            cancel: &cancel,
            until: Instant::now() + TRANSFER_TIME,
            remaining: &mut remaining,
        };
        let mut bytes = [0; 100];
        assert_eq!(transfer.read(&mut bytes).unwrap(), 17);
        assert_eq!(
            transfer.read(&mut bytes).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        cancel.store(true, Ordering::Release);
        assert_eq!(
            transfer.write(&bytes).unwrap_err().kind(),
            io::ErrorKind::Interrupted
        );
        cancel.store(false, Ordering::Release);
        transfer.until = Instant::now();
        assert_eq!(
            transfer.flush().unwrap_err().kind(),
            io::ErrorKind::TimedOut
        );
        assert_eq!(remaining, 0);
    }
}
