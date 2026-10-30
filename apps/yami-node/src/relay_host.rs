//! Bounded outbound relay slots. DHT publication uses the existing maintainer.
use crate::{shutdown::Shutdown, Site};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::Path,
    sync::{Arc, Mutex},
    thread,
    time::Instant,
};
use yami_dht::{Network, RelayRoute, Service};
use yami_identity::NodeIdentity;
use yami_relay::{HostEvent, Route};

#[derive(Clone, Default)]
pub struct ActiveRoutes(Arc<Mutex<Health>>);
#[derive(Default)]
struct Health {
    routes: Vec<(RelayRoute, Instant)>,
    configured: usize,
    failures: u64,
    last_success: u64,
}
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
        let routes = self
            .0
            .lock()
            .map_err(|_| io::Error::other("relay status poisoned"))?;
        Ok(routes
            .routes
            .iter()
            .filter(|(_, until)| *until > Instant::now())
            .map(|(r, _)| *r)
            .collect())
    }
    fn update(&self, route: RelayRoute, event: HostEvent) {
        if let Ok(mut health) = self.0.lock() {
            health.routes.retain(|(r, _)| *r != route);
            if matches!(event, HostEvent::Registered) && health.routes.len() < 2 {
                health
                    .routes
                    .push((route, Instant::now() + yami_relay::REGISTRATION_LEASE));
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
                .filter(|(_, until)| *until > Instant::now())
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
    #[test]
    fn expired_registration_is_not_published_and_health_counts_failures() {
        let active = ActiveRoutes::default();
        let route = RelayRoute {
            relay: *NodeIdentity::generate().unwrap().id(),
            endpoint: "127.0.0.1:7443".parse().unwrap(),
        };
        active.update(route, HostEvent::Registered);
        assert_eq!(active.current().unwrap(), vec![route]);
        active.0.lock().unwrap().routes[0].1 = Instant::now();
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
