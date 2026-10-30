//! Bounded shared discovery and endpoint failure memory. No document/session cache.
use std::{
    collections::VecDeque,
    io,
    net::SocketAddr,
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant},
};
use yami_dht::Lookup;
use yami_identity::NodeId;
pub const MAX_FAILURES: usize = 128;
pub const MAX_LOOKUPS: usize = 64;
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Endpoint {
    pub destination: NodeId,
    pub relay: Option<NodeId>,
    pub address: SocketAddr,
}
struct Failure {
    endpoint: Endpoint,
    count: u32,
    identity: bool,
    until: Instant,
}
#[derive(Default)]
pub struct Failures(Mutex<VecDeque<Failure>>);
pub fn backoff(count: u32, entropy: u64) -> Duration {
    let base = 400u64
        .saturating_mul(1u64 << count.saturating_sub(1).min(7))
        .min(24_000);
    Duration::from_millis(base + entropy % (base / 4 + 1))
}
impl Failures {
    pub fn eligible(&self, endpoint: Endpoint, at: Instant) -> bool {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .find(|f| f.endpoint == endpoint)
            .is_none_or(|f| at >= f.until)
    }
    pub fn failed(&self, endpoint: Endpoint, identity: bool) -> io::Result<Duration> {
        let mut bytes = [0; 8];
        getrandom::fill(&mut bytes).map_err(io::Error::other)?;
        Ok(self.failed_at(
            endpoint,
            Instant::now(),
            u64::from_ne_bytes(bytes),
            identity,
        ))
    }
    fn failed_at(&self, endpoint: Endpoint, at: Instant, entropy: u64, identity: bool) -> Duration {
        let mut failures = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let count = failures
            .iter()
            .position(|f| f.endpoint == endpoint)
            .and_then(|i| failures.remove(i))
            .map_or(1, |f| f.count.saturating_add(1));
        let delay = backoff(count, entropy);
        if failures.len() == MAX_FAILURES {
            failures.pop_front();
        }
        failures.push_back(Failure {
            endpoint,
            count,
            identity,
            until: at + delay,
        });
        delay
    }
    pub fn identity_failure(&self, endpoint: Endpoint) -> bool {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .any(|f| f.endpoint == endpoint && f.identity)
    }
    pub fn succeeded(&self, endpoint: Endpoint) {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|f| f.endpoint != endpoint);
    }
    pub fn len(&self) -> usize {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).len()
    }
}
#[derive(Default)]
struct Query {
    running: bool,
    result: Option<(Instant, Result<Lookup, String>)>,
}
#[derive(Default)]
struct Flight {
    state: Mutex<Query>,
    changed: Condvar,
}
type LookupEntry = ((NodeId, Option<u64>), Arc<Flight>);
#[derive(Default)]
pub struct Lookups(Mutex<VecDeque<LookupEntry>>);
struct Running(Arc<Flight>);
impl Drop for Running {
    fn drop(&mut self) {
        let mut state = self.0.state.lock().unwrap_or_else(|e| e.into_inner());
        state.running = false;
        self.0.changed.notify_all();
    }
}
impl Lookups {
    pub fn run(
        &self,
        target: NodeId,
        after: Option<u64>,
        deadline: Instant,
        query: impl FnOnce() -> Result<Lookup, String>,
    ) -> Result<Lookup, String> {
        let arrived = Instant::now();
        let key = (target, after);
        let flight = {
            let mut entries = self.0.lock().unwrap_or_else(|e| e.into_inner());
            if let Some((_, flight)) = entries.iter().find(|(id, _)| *id == key) {
                flight.clone()
            } else {
                if entries.len() == MAX_LOOKUPS {
                    let index = entries
                        .iter()
                        .position(|(_, f)| {
                            Arc::strong_count(f) == 1
                                && !f.state.lock().unwrap_or_else(|e| e.into_inner()).running
                        })
                        .ok_or("DHT discovery is busy; retry shortly.")?;
                    entries.remove(index);
                }
                let flight = Arc::new(Flight::default());
                entries.push_back((key, flight.clone()));
                flight
            }
        };
        let mut state = flight.state.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err("DHT lookup timeout: navigation deadline exceeded.".into());
            }
            if let Some((at, result)) = &state.result {
                if *at >= arrived {
                    return result.clone();
                }
                let cooldown = Duration::from_secs(1).saturating_sub(at.elapsed());
                if !cooldown.is_zero() {
                    state = flight
                        .changed
                        .wait_timeout(state, remaining.min(cooldown))
                        .unwrap_or_else(|e| e.into_inner())
                        .0;
                    continue;
                }
            }
            if !state.running {
                break;
            }
            state = flight
                .changed
                .wait_timeout(state, remaining)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
        state.running = true;
        drop(state);
        let _running = Running(flight.clone()); // Also wakes waiters after unwind.
        let result = query();
        flight
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .result = Some((Instant::now(), result.clone()));
        result
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::atomic::{AtomicUsize, Ordering},
        thread,
    };
    fn endpoint(port: u16) -> Endpoint {
        Endpoint {
            destination: *yami_identity::NodeIdentity::generate().unwrap().id(),
            relay: None,
            address: ([127, 0, 0, 1], port).into(),
        }
    }
    #[test]
    fn failure_cooldowns_are_jittered_exponential_bounded_and_recover() {
        let failures = Failures::default();
        let endpoint = endpoint(1);
        let at = Instant::now();
        assert!(failures.eligible(endpoint, at));
        let first = failures.failed_at(endpoint, at, 0, false);
        let second = failures.failed_at(endpoint, at, 1, false);
        assert!(second > first);
        assert!(!failures.eligible(endpoint, at));
        assert!(failures.eligible(endpoint, at + second));
        assert!(backoff(100, u64::MAX) <= Duration::from_secs(30));
        assert_ne!(backoff(2, 0), backoff(2, 20));
        failures.succeeded(endpoint);
        assert_eq!(failures.len(), 0);
        for port in 1..=200 {
            failures.failed_at(super::tests::endpoint(port), at, 0, false);
        }
        assert_eq!(failures.len(), MAX_FAILURES);
    }
    #[test]
    fn concurrent_tabs_share_discovery_and_waiters_honor_their_deadline() {
        let lookups = Arc::new(Lookups::default());
        let calls = Arc::new(AtomicUsize::new(0));
        let id = *yami_identity::NodeIdentity::generate().unwrap().id();
        thread::scope(|scope| {
            for _ in 0..12 {
                let lookups = lookups.clone();
                let calls = calls.clone();
                scope.spawn(move || {
                    lookups
                        .run(id, None, Instant::now() + Duration::from_secs(2), || {
                            calls.fetch_add(1, Ordering::SeqCst);
                            thread::sleep(Duration::from_millis(80));
                            Ok(Lookup {
                                record: None,
                                queries: 1,
                                failures: 0,
                                invalid_records: 0,
                                timeouts: 0,
                            })
                        })
                        .unwrap();
                });
            }
        });
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(lookups
            .run(id, None, Instant::now(), || unreachable!())
            .is_err());
    }
}

#[derive(Default)]
struct Transfer {
    busy: Mutex<bool>,
    changed: Condvar,
}
type TransferEntry = (NodeId, Arc<Transfer>);
#[derive(Default)]
pub struct Transfers(Mutex<VecDeque<TransferEntry>>);
pub struct TransferPermit(Arc<Transfer>);
impl Drop for TransferPermit {
    fn drop(&mut self) {
        *self.0.busy.lock().unwrap_or_else(|e| e.into_inner()) = false;
        self.0.changed.notify_all();
    }
}
impl Transfers {
    /// The relay protocol has one serving slot per host. Serialize this browser's
    /// same-owner document attempts without sharing sessions or document data.
    pub fn acquire(&self, id: NodeId, deadline: Instant) -> Result<TransferPermit, String> {
        let transfer = {
            let mut entries = self.0.lock().unwrap_or_else(|e| e.into_inner());
            if let Some((_, transfer)) = entries.iter().find(|(owner, _)| *owner == id) {
                transfer.clone()
            } else {
                if entries.len() == MAX_LOOKUPS {
                    let index = entries
                        .iter()
                        .position(|(_, t)| Arc::strong_count(t) == 1)
                        .ok_or("Document connections are busy")?;
                    entries.remove(index);
                }
                let transfer = Arc::new(Transfer::default());
                entries.push_back((id, transfer.clone()));
                transfer
            }
        };
        let mut busy = transfer.busy.lock().unwrap_or_else(|e| e.into_inner());
        while *busy {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err("Navigation deadline exceeded waiting for destination slot".into());
            }
            busy = transfer
                .changed
                .wait_timeout(busy, remaining)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
        if Instant::now() >= deadline {
            return Err("Navigation deadline exceeded".into());
        }
        *busy = true;
        drop(busy);
        Ok(TransferPermit(transfer))
    }
}
#[cfg(test)]
mod transfer_tests {
    use super::*;
    #[test]
    fn same_owner_connections_wait_with_deadline_and_cleanup_releases_slot() {
        let transfers = Transfers::default();
        let owner = *yami_identity::NodeIdentity::generate().unwrap().id();
        let held = transfers
            .acquire(owner, Instant::now() + Duration::from_secs(1))
            .unwrap();
        assert!(transfers
            .acquire(owner, Instant::now() + Duration::from_millis(10))
            .is_err());
        let other = *yami_identity::NodeIdentity::generate().unwrap().id();
        assert!(transfers
            .acquire(other, Instant::now() + Duration::from_secs(1))
            .is_ok());
        drop(held);
        assert!(transfers
            .acquire(owner, Instant::now() + Duration::from_secs(1))
            .is_ok());
    }
}
