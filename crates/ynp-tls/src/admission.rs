//! Bounded admission state for explicitly public listeners. No bans on a
//! growing map of hostile addresses; only admitted starts and active leases.
use std::{
    collections::{HashMap, VecDeque},
    io,
    net::IpAddr,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

pub(crate) const MAX_ACTIVE: usize = 32;
pub(crate) const PER_IP_ACTIVE: usize = 2;
pub(crate) const STARTS_PER_SECOND: usize = 32;
pub(crate) const PER_IP_STARTS: usize = 8;
#[derive(Default)]
pub(crate) struct Admission {
    recent: VecDeque<(Instant, IpAddr)>,
    active: HashMap<IpAddr, usize>,
    total: usize,
}
impl Admission {
    fn admit(&mut self, ip: IpAddr, now: Instant) -> bool {
        while self
            .recent
            .front()
            .is_some_and(|(time, _)| now.duration_since(*time) >= Duration::from_secs(1))
        {
            self.recent.pop_front();
        }
        if self.total >= MAX_ACTIVE
            || self.active.get(&ip).copied().unwrap_or(0) >= PER_IP_ACTIVE
            || self.recent.len() >= STARTS_PER_SECOND
            || self.recent.iter().filter(|(_, addr)| *addr == ip).count() >= PER_IP_STARTS
        {
            return false;
        }
        self.recent.push_back((now, ip));
        *self.active.entry(ip).or_default() += 1;
        self.total += 1;
        true
    }
}
pub(crate) struct Lease {
    limits: Arc<Mutex<Admission>>,
    ip: IpAddr,
}
impl Lease {
    pub(crate) fn acquire(limits: Arc<Mutex<Admission>>, ip: IpAddr) -> io::Result<Self> {
        let allowed = limits
            .lock()
            .map_err(|_| io::Error::other("admission state poisoned"))?
            .admit(ip, Instant::now());
        if !allowed {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "public connection budget exceeded",
            ));
        }
        Ok(Self { limits, ip })
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        if let Ok(mut limits) = self.limits.lock() {
            if let Some(count) = limits.active.get_mut(&self.ip) {
                *count -= 1;
                if *count == 0 {
                    limits.active.remove(&self.ip);
                }
                limits.total -= 1;
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn concurrent_and_rate_budgets_release_and_expire() {
        let ip = "127.0.0.1".parse().unwrap();
        let limits = Arc::new(Mutex::new(Admission::default()));
        let one = Lease::acquire(Arc::clone(&limits), ip).unwrap();
        let two = Lease::acquire(Arc::clone(&limits), ip).unwrap();
        assert!(Lease::acquire(Arc::clone(&limits), ip).is_err());
        drop((one, two));
        for _ in 0..PER_IP_STARTS - 2 {
            drop(Lease::acquire(Arc::clone(&limits), ip).unwrap());
        }
        assert!(Lease::acquire(Arc::clone(&limits), ip).is_err());
        assert!(limits.lock().unwrap().active.is_empty());
        let mut state = Admission::default();
        let now = Instant::now();
        for host in 1..=32 {
            assert!(state.admit(IpAddr::from([10, 0, 0, host]), now));
        }
        assert_eq!(state.active.len(), MAX_ACTIVE);
        assert!(!state.admit(IpAddr::from([10, 0, 0, 33]), now));
        state.active.clear();
        state.total = 0;
        assert!(!state.admit(ip, now));
        assert!(state.admit(ip, now + Duration::from_secs(1)));
        assert_eq!(state.recent.len(), 1);
    }
}
