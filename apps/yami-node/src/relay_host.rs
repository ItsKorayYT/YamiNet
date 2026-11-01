//! Bounded outbound relay slots. DHT publication uses the existing maintainer.
use crate::{shutdown::Shutdown, Site};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::Path,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};
use yami_dht::{Network, RelayRoute, Service};
use yami_identity::NodeIdentity;
use yami_relay::{HostEvent, Route};

#[derive(Clone, Default)]
pub struct ActiveRoutes(Arc<Mutex<Health>>);
#[derive(Default)]
struct Health {
    routes: Vec<LiveRoute>,
    configured: usize,
    failures: u64,
    last_success: u64,
}
struct LiveRoute {
    route: RelayRoute,
    until: Instant,
    rotating: bool,
}
// A successfully served single-use slot immediately re-registers. Retain only
// its last authenticated location claim briefly, never past its original lease.
const ROTATION_GRACE: Duration = Duration::from_secs(2);
#[derive(Debug)]
pub struct RelayHealth {
    pub id: yami_identity::NodeId,
    pub observed: u64,
    pub configured: usize,
    pub registered: usize,
    pub failures: u64,
    pub last_success: u64,
}
pub fn read_health(directory: &Path) -> io::Result<RelayHealth> {
    let mut text = String::new();
    File::open(directory.join("relay-health.status"))?
        .take(513)
        .read_to_string(&mut text)?;
    let fields: Vec<_> = text.split_whitespace().collect();
    let invalid = || io::Error::new(io::ErrorKind::InvalidData, "invalid relay health status");
    if text.len() > 512 || fields.len() != 7 || fields[0] != "YRH1" {
        return Err(invalid());
    }
    let number = |n: usize| fields[n].parse::<u64>().map_err(|_| invalid());
    let status = RelayHealth {
        id: fields[1].parse().map_err(|_| invalid())?,
        observed: number(2)?,
        configured: usize::try_from(number(3)?).map_err(|_| invalid())?,
        registered: usize::try_from(number(4)?).map_err(|_| invalid())?,
        failures: number(5)?,
        last_success: number(6)?,
    };
    if status.configured > 2 || status.registered > status.configured {
        return Err(invalid());
    }
    Ok(status)
}
impl ActiveRoutes {
    pub fn current(&self) -> io::Result<Vec<RelayRoute>> {
        self.current_at(Instant::now())
    }
    fn current_at(&self, at: Instant) -> io::Result<Vec<RelayRoute>> {
        let routes = self
            .0
            .lock()
            .map_err(|_| io::Error::other("relay status poisoned"))?;
        Ok(routes
            .routes
            .iter()
            .filter(|live| live.until > at)
            .map(|live| live.route)
            .collect())
    }
    fn update(&self, route: RelayRoute, event: HostEvent) {
        self.update_at(route, event, Instant::now());
    }
    fn update_at(&self, route: RelayRoute, event: HostEvent, at: Instant) {
        if let Ok(mut health) = self.0.lock() {
            if matches!(event, HostEvent::Served) {
                if let Some(live) = health.routes.iter_mut().find(|live| live.route == route) {
                    live.until = live.until.min(at + ROTATION_GRACE);
                    live.rotating = true;
                }
                return;
            }
            health.routes.retain(|live| {
                live.route != route
                    || (matches!(event, HostEvent::Unregistered)
                        && live.rotating
                        && live.until > at)
            });
            if matches!(event, HostEvent::Registered) && health.routes.len() < 2 {
                health.routes.push(LiveRoute {
                    route,
                    until: at + yami_relay::REGISTRATION_LEASE,
                    rotating: false,
                });
                health.last_success = yami_dht::now().unwrap_or(0);
            }
            if matches!(event, HostEvent::Retrying) {
                health.failures = health.failures.saturating_add(1);
            }
        }
    }
    pub fn write_health(
        &self,
        directory: &Path,
        id: yami_identity::NodeId,
        running: bool,
    ) -> io::Result<()> {
        let health = self
            .0
            .lock()
            .map_err(|_| io::Error::other("relay health poisoned"))?;
        let active = if running {
            health
                .routes
                .iter()
                .filter(|live| !live.rotating && live.until > Instant::now())
                .count()
        } else {
            0
        };
        let text = format!(
            "YRH1 {id} {} {} {} {} {}\n",
            yami_dht::now()?,
            health.configured,
            active,
            health.failures,
            health.last_success
        );
        let path = directory.join("relay-health.status");
        let temp = directory.join(format!("relay-health.{}.tmp", std::process::id()));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let result = (|| {
            let mut file = options.open(&temp)?;
            file.write_all(text.as_bytes())?;
            file.sync_all()?;
            drop(file);
            fs::rename(&temp, path)
        })();
        if result.is_err() {
            let _ = fs::remove_file(temp);
        }
        result
    }
}
pub struct Hosts {
    shutdown: Shutdown,
    tasks: Vec<thread::JoinHandle<()>>,
}
impl Hosts {
    pub fn start(
        routes: Vec<RelayRoute>,
        identity: Arc<NodeIdentity>,
        site: Arc<Site>,
        service: Service,
        shutdown: Shutdown,
        active: ActiveRoutes,
    ) -> io::Result<Self> {
        let network = service.snapshot()?.network;
        if routes.is_empty() || routes.len() > 2 || routes.windows(2).any(|p| p[0] >= p[1]) {
            return Err(io::Error::other(
                "configure one or two unique sorted relay routes",
            ));
        }
        for route in &routes {
            network.check(route.endpoint)?;
            if route.relay == *identity.id() {
                return Err(io::Error::other("host cannot relay through itself"));
            }
        }
        active
            .0
            .lock()
            .map_err(|_| io::Error::other("relay health poisoned"))?
            .configured = routes.len();
        let mut hosts = Self {
            shutdown,
            tasks: Vec::new(),
        };
        for relay in routes {
            let route = Route {
                relay: relay.relay,
                endpoint: relay.endpoint.into(),
                destination: *identity.id(),
            };
            let (identity, site, service, signal, active) = (
                identity.clone(),
                site.clone(),
                service.clone(),
                hosts.shutdown.clone(),
                active.clone(),
            );
            hosts
                .tasks
                .push(
                    thread::Builder::new()
                        .name("relay-host".into())
                        .spawn(move || {
                            let run = if network == Network::Public {
                                yami_relay::run_host_public
                            } else {
                                yami_relay::run_host
                            };
                            let result = run(
                                route,
                                identity,
                                signal.flag(),
                                &|connection| {
                                    crate::serve_connection_with_dht(connection, &*site, &service)
                                        .map(|_| ())
                                },
                                &|event| {
                                    active.update(relay, event);
                                    match event {
                                        HostEvent::Registered => eprintln!(
                                            "Registered outbound host {} at relay {}",
                                            route.destination, route.relay
                                        ),
                                        HostEvent::Retrying => eprintln!(
                                            "Relay {} unavailable; bounded reconnect backoff",
                                            route.relay
                                        ),
                                        _ => {}
                                    }
                                },
                            );
                            active.update(relay, HostEvent::Unregistered);
                            if result.is_err() {
                                eprintln!("Relay worker stopped after local transport failure");
                            }
                        })?,
                );
        }
        Ok(hosts)
    }
}
impl Drop for Hosts {
    fn drop(&mut self) {
        self.shutdown.stop();
        for task in self.tasks.drain(..) {
            let _ = task.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn route() -> RelayRoute {
        RelayRoute {
            relay: *NodeIdentity::generate().unwrap().id(),
            endpoint: "127.0.0.1:7443".parse().unwrap(),
        }
    }
    #[test]
    fn served_slot_rotation_preserves_a_bounded_location_claim_but_failures_withdraw() {
        let active = ActiveRoutes::default();
        let route = route();
        let at = Instant::now();
        active.update_at(route, HostEvent::Registered, at);
        active.update_at(route, HostEvent::Served, at);
        active.update_at(route, HostEvent::Unregistered, at);
        assert_eq!(
            active.current_at(at + Duration::from_secs(1)).unwrap(),
            vec![route]
        );
        assert!(active.0.lock().unwrap().routes[0].rotating);
        // Repeated notifications cannot extend a verified claim indefinitely.
        active.update_at(route, HostEvent::Served, at + Duration::from_secs(1));
        assert!(active.current_at(at + ROTATION_GRACE).unwrap().is_empty());
        active.update_at(route, HostEvent::Retrying, at);
        assert!(active.current_at(at).unwrap().is_empty());
        active.update_at(route, HostEvent::Registered, at);
        active.update_at(route, HostEvent::Unregistered, at);
        assert!(active.current_at(at).unwrap().is_empty());
    }
    #[test]
    fn rotation_never_invents_a_route_or_extends_the_original_authenticated_lease() {
        let active = ActiveRoutes::default();
        let route = route();
        let at = Instant::now();
        active.update_at(route, HostEvent::Served, at);
        active.update_at(route, HostEvent::Unregistered, at);
        assert!(active.current_at(at).unwrap().is_empty());
        active.update_at(route, HostEvent::Registered, at);
        let expires = at + yami_relay::REGISTRATION_LEASE;
        active.update_at(
            route,
            HostEvent::Served,
            expires - Duration::from_millis(100),
        );
        active.update_at(
            route,
            HostEvent::Unregistered,
            expires - Duration::from_millis(100),
        );
        assert_eq!(
            active
                .current_at(expires - Duration::from_millis(50))
                .unwrap(),
            vec![route]
        );
        assert!(active.current_at(expires).unwrap().is_empty());
    }
    #[test]
    fn expired_registration_is_not_published_and_health_counts_failures() {
        let active = ActiveRoutes::default();
        let route = RelayRoute {
            relay: *NodeIdentity::generate().unwrap().id(),
            endpoint: "127.0.0.1:7443".parse().unwrap(),
        };
        active.update(route, HostEvent::Registered);
        assert_eq!(active.current().unwrap(), vec![route]);
        active.0.lock().unwrap().routes[0].until = Instant::now();
        assert!(active.current().unwrap().is_empty());
        active.update(route, HostEvent::Retrying);
        assert!(active.current().unwrap().is_empty());
        assert_eq!(active.0.lock().unwrap().failures, 1);
        active.update(route, HostEvent::Registered);
        assert_eq!(active.current().unwrap(), vec![route]);
        active.update(route, HostEvent::Unregistered);
        assert!(active.current().unwrap().is_empty());
    }
}
