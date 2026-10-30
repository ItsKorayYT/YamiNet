//! Bounded outbound relay slots. DHT publication uses the existing maintainer.
use crate::{shutdown::Shutdown, Site};
use std::{
    io,
    sync::{Arc, Mutex},
    thread,
    time::Instant,
};
use yami_dht::{Network, RelayRoute, Service};
use yami_identity::NodeIdentity;
use yami_relay::{HostEvent, Route};

#[derive(Clone, Default)]
pub struct ActiveRoutes(Arc<Mutex<Vec<(RelayRoute, Instant)>>>);
impl ActiveRoutes {
    pub fn current(&self) -> io::Result<Vec<RelayRoute>> {
        let routes = self
            .0
            .lock()
            .map_err(|_| io::Error::other("relay status poisoned"))?;
        Ok(routes
            .iter()
            .filter(|(_, until)| *until > Instant::now())
            .map(|(r, _)| *r)
            .collect())
    }
    fn update(&self, route: RelayRoute, active: bool) {
        if let Ok(mut routes) = self.0.lock() {
            routes.retain(|(r, _)| *r != route);
            if active && routes.len() < 2 {
                routes.push((route, Instant::now() + yami_relay::REGISTRATION_LEASE));
            }
        }
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
                                    active.update(relay, matches!(event, HostEvent::Registered));
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
                            active.update(relay, false);
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
