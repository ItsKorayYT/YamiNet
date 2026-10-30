//! Same-authenticated-owner PNG loading on existing loader workers.
use crate::model::Page;
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use yami_identity::NodeId;
use yami_render::assets::{self, ImageState, MAX_PAGE_COMPRESSED, MAX_PAGE_DECODED};
#[derive(Debug, Clone)]
pub struct Origin {
    pub destination: NodeId,
    pub endpoint: std::net::SocketAddr,
    pub relay: Option<NodeId>,
    pub public_relay: bool,
}
impl Origin {
    fn connect(&self, budget: Duration) -> std::io::Result<Box<dyn ynp::Connection>> {
        if let Some(relay) = self.relay {
            yami_relay::Route {
                relay,
                endpoint: self.endpoint,
                destination: self.destination,
            }
            .connect_with_timeout(self.public_relay, budget)
        } else {
            ynp_tls::TlsTransport::new(self.destination)?
                .connect_with_timeout(self.endpoint, budget)
        }
    }
}
struct Metered<'a> {
    inner: &'a mut dyn ynp::Connection,
    limit: usize,
    read: usize,
}
impl std::io::Read for Metered<'_> {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        let remaining = self.limit.saturating_sub(self.read);
        if remaining == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "page image transfer budget exceeded",
            ));
        }
        let length = bytes.len().min(remaining);
        let read = self.inner.read(&mut bytes[..length])?;
        self.read += read;
        Ok(read)
    }
}
impl std::io::Write for Metered<'_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.inner.write(bytes)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}
impl ynp::Connection for Metered<'_> {
    fn protection(&self) -> ynp::Protection {
        self.inner.protection()
    }
}
pub(crate) fn load(
    page: &Page,
    cache: &crate::image_cache::DecodeCache,
    cancel: &AtomicBool,
    deadline: Instant,
    mut emit: impl FnMut(String, ImageState) -> bool,
) {
    let mut bytes = 0;
    let mut decoded = 0;
    for key in page.images.keys() {
        if cancel.load(Ordering::SeqCst) {
            return;
        }
        let result = (|| -> Result<Arc<assets::Image>, String> {
            let origin = page
                .origin
                .as_ref()
                .filter(|origin| page.security.destination() == Some(origin.destination))
                .ok_or("Images require an authenticated parent node")?;
            if bytes >= MAX_PAGE_COMPRESSED
                || decoded >= MAX_PAGE_DECODED
                || Instant::now() >= deadline
            {
                return Err("Page image budget exhausted".into());
            }
            let attempt_deadline = deadline.min(Instant::now() + Duration::from_secs(3));
            let mut connection = None;
            for attempt in 0..2 {
                if cancel.load(Ordering::SeqCst) {
                    return Err("Image load cancelled".into());
                }
                if attempt > 0 {
                    let mut entropy = [0; 8];
                    getrandom::fill(&mut entropy).map_err(|_| "Cannot schedule image retry")?;
                    let delay = crate::reliability::backoff(1, u64::from_ne_bytes(entropy));
                    let until = Instant::now()
                        + delay.min(attempt_deadline.saturating_duration_since(Instant::now()));
                    while Instant::now() < until && !cancel.load(Ordering::SeqCst) {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                }
                let budget = attempt_deadline.saturating_duration_since(Instant::now());
                if budget.is_zero() {
                    break;
                }
                match origin.connect(budget) {
                    Ok(stream) => {
                        connection = Some(stream);
                        break;
                    }
                    Err(error) => {
                        if error
                            .get_ref()
                            .and_then(|e| e.downcast_ref::<ynp_tls::IdentityMismatch>())
                            .is_some()
                        {
                            return Err("Image destination/relay identity mismatch".into());
                        }
                    }
                }
            }
            let mut connection = connection.ok_or("Image node unavailable")?;
            let remaining = MAX_PAGE_COMPRESSED - bytes;
            let overhead = 16 + 2 + key.len() + 1 + 4;
            let limit = remaining.saturating_sub(overhead).min(ynp::MAX_ASSET_BYTES);
            if limit == 0 {
                return Err("Page image transfer budget exhausted".into());
            }
            let mut metered = Metered {
                inner: &mut *connection,
                limit: remaining,
                read: 0,
            };
            let response = ynp::request_asset(&mut metered, key, limit);
            bytes += metered.read;
            let data = response.map_err(|error| {
                if error.kind() == std::io::ErrorKind::NotFound {
                    "Image not found"
                } else {
                    "Invalid or unavailable image response"
                }
            })?;
            if cancel.load(Ordering::SeqCst) {
                return Err("Image load cancelled".into());
            }
            let image = cache
                .decode(
                    origin.destination,
                    key,
                    &data.bytes,
                    MAX_PAGE_DECODED - decoded,
                )
                .map_err(|_| "Invalid, unsupported or oversized PNG")?;
            decoded += image.pixels.len();
            Ok(image)
        })();
        if cancel.load(Ordering::SeqCst) {
            return;
        }
        let state = match result {
            Ok(image) => ImageState::Ready(image),
            Err(error) => ImageState::Missing(error),
        };
        if !emit(key.clone(), state) {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    struct Input(std::io::Cursor<Vec<u8>>);
    impl std::io::Read for Input {
        fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
            self.0.read(b)
        }
    }
    impl std::io::Write for Input {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    impl ynp::Connection for Input {
        fn protection(&self) -> ynp::Protection {
            ynp::Protection::Tls13PinnedServer
        }
    }
    #[test]
    fn malformed_response_bytes_are_charged_and_never_exceed_remaining_budget() {
        let mut input = Input(std::io::Cursor::new(vec![0; 100]));
        let mut metered = Metered {
            inner: &mut input,
            limit: 17,
            read: 0,
        };
        let mut bytes = [0; 100];
        assert_eq!(metered.read(&mut bytes).unwrap(), 17);
        assert!(metered.read(&mut bytes).is_err());
        assert_eq!(metered.read, 17);
    }
}
