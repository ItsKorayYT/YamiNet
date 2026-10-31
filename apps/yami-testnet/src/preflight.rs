//! End-to-end preflight using the existing signed DHT, pinned TLS, relay and YNP
//! code paths. No new protocol, plaintext fallback or automatic listener.
use std::{
    collections::BTreeMap,
    io,
    net::SocketAddrV4,
    time::{Duration, Instant},
};

use serde_json::{json, Value};
use yami_config::{Config, Role};
use yami_dht::{Contact, Network, PeerRecord, PinnedTls, RelayRoute, Service};
use yami_identity::NodeId;
use ynp::{Connection, Protection};

use crate::{
    config::{describe, inspect_identity, inspect_listener, inspect_site, Loaded},
    report::{Check, Report, Status},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteMode {
    Auto,
    Direct,
    Relay,
}

impl RouteMode {
    pub fn parse(text: &str) -> Result<Self, String> {
        match text {
            "auto" => Ok(Self::Auto),
            "direct" => Ok(Self::Direct),
            "relay" => Ok(Self::Relay),
            _ => Err("route mode must be auto, direct or relay".into()),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Direct => "direct",
            Self::Relay => "relay",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Target {
    pub url: String,
    pub destination: NodeId,
    pub document: String,
    pub asset: Option<String>,
}

impl Target {
    pub fn parse(url: &str, asset: Option<&str>) -> Result<Self, String> {
        if url.len() > 512 {
            return Err("URL is too long".into());
        }
        let rest = url
            .strip_prefix("yami://")
            .ok_or_else(|| "preflight needs an exact yami://NODE-ID/document URL".to_string())?;
        let (id, document) = rest
            .split_once('/')
            .ok_or_else(|| "yami:// URL needs a document key".to_string())?;
        if id.is_empty() || document.is_empty() {
            return Err("yami:// URL needs a full node ID and a document key".into());
        }
        let destination: NodeId = id
            .parse()
            .map_err(|_| "yami:// URL contains an invalid full node ID".to_string())?;
        ynp::validate_key(document)
            .map_err(|_| "yami:// URL contains an invalid document key".to_string())?;
        if let Some(asset) = asset {
            ynp::validate_asset_key(asset)
                .map_err(|_| "asset key must be 1..64 safe ASCII characters".to_string())?;
        }
        Ok(Self {
            url: format!("yami://{destination}/{document}"),
            destination,
            document: document.into(),
            asset: asset.map(str::to_owned),
        })
    }
}

pub struct Options {
    pub primary: Loaded,
    pub client: Option<Loaded>,
    pub host: Option<Loaded>,
    pub target: Target,
    pub extra_bootstraps: Vec<Contact>,
    pub route: RouteMode,
    pub timeout: Duration,
    pub check_listeners: bool,
    pub include_endpoints: bool,
}

/// A configuration-only report used by `check-config`.
pub fn check_config(
    path: &std::path::Path,
    expected: Option<Role>,
    check_listeners: bool,
) -> Report {
    let mut report = Report::new(
        "YamiNet testnet configuration check",
        json!({"role": expected.map(|role| format!("{role:?}"))}),
    );
    match Loaded::load(path, expected) {
        Ok(loaded) => {
            report.push(Check::pass(
                "configuration",
                "Configuration validity",
                describe(&loaded.config),
            ));
            let config = loaded.config;
            let _ = local_checks(&mut report, &[("primary", &config)], check_listeners);
        }
        Err(error) => {
            report.push(Check::fail(
                "configuration",
                "Configuration validity",
                error,
            ));
        }
    }
    report
}

pub fn run(options: &Options) -> Report {
    let network = options
        .client
        .as_ref()
        .map_or(options.primary.config.network, |client| {
            client.config.network
        });
    let target = &options.target;
    let mut report = Report::preflight(json!({
        "url": target.url.clone(),
        "destination": target.destination.to_string(),
        "document": target.document.clone(),
        "asset": target.asset.clone(),
        "network": match network { Network::Public => "public", Network::Loopback => "loopback" },
        "route_mode": options.route.as_str(),
    }));

    if let Err(error) = validate_profiles(options, network) {
        report.push(Check::fail(
            "configuration",
            "Configuration validity",
            error,
        ));
        return report;
    }
    let relay_enabled = options.client.as_ref().map_or_else(
        || options.primary.config.relay_fallback || !options.primary.config.relays.is_empty(),
        |client| client.config.relay_fallback,
    );
    let relay_source = if options.client.is_some() {
        "client relay-fallback setting"
    } else if !options.primary.config.relays.is_empty() {
        "explicit host relay pin"
    } else {
        "client relay-fallback setting"
    };
    report.push(
        Check::pass(
            "configuration",
            "Configuration validity",
            format!(
                "{}; route mode {}; relay path explicitly {} ({relay_source})",
                describe(&options.primary.config),
                options.route.as_str(),
                if relay_enabled { "enabled" } else { "disabled" }
            ),
        )
        .with_data(json!({
            "primary_role": format!("{:?}", options.primary.config.role),
            "client_role": options.client.as_ref().map(|client| format!("{:?}", client.config.role)),
            "host_supplied": options.host.is_some(),
            "network": match network { Network::Public => "public", Network::Loopback => "loopback" },
        })),
    );

    let mut locals: Vec<(&str, &Config)> = vec![("primary", &options.primary.config)];
    if let Some(host) = &options.host {
        if host.path != options.primary.path {
            locals.push(("host", &host.config));
        }
    }
    let identities = local_checks(&mut report, &locals, options.check_listeners);

    let host_profile =
        options.host.as_ref().map(|host| &host.config).or_else(|| {
            (options.primary.config.role == Role::Node).then_some(&options.primary.config)
        });
    host_local_checks(
        &mut report,
        host_profile,
        &target.destination,
        &identities,
        options.include_endpoints,
    );

    let pins = match collect_bootstraps(options, network) {
        Ok(pins) => pins,
        Err(error) => {
            report.push(Check::fail(
                "bootstrap-pins",
                "Bootstrap pin validity",
                error,
            ));
            network_checks_unavailable(
                &mut report,
                target,
                options.route,
                "bootstrap pins are invalid",
            );
            return report;
        }
    };
    let state = options
        .client
        .as_ref()
        .map_or(&options.primary.config.state, |client| &client.config.state);
    let mode = if network == Network::Public {
        "public"
    } else {
        "local"
    };
    let cache = state.join(format!("dht-preflight-{mode}.cache"));
    let service = match Service::open_lookup(&cache, network) {
        Ok(service) => service,
        Err(_) => {
            report.push(Check::fail(
                "dht-state",
                "DHT state",
                "preflight DHT state is unavailable, locked or invalid; stop duplicate preflight runs and check the state directory",
            ));
            network_checks_unavailable(
                &mut report,
                target,
                options.route,
                "DHT state could not be opened",
            );
            return report;
        }
    };
    report.push(Check::pass(
        "dht-state",
        "DHT state",
        "local preflight lookup cache opened in the configured state directory; separate from service caches",
    ));

    if pins.is_empty() {
        report.push(Check::not_run(
            "bootstrap-reachability",
            "Bootstrap peer reachability",
            "no explicit bootstrap pins are configured; signed public lookup cannot be attempted",
        ));
        report.push(Check::not_run(
            "signed-dht-lookup",
            "Signed DHT lookup",
            "no bootstrap pins; no fresh signed record was requested",
        ));
        network_checks_unavailable(
            &mut report,
            target,
            options.route,
            "no explicit bootstrap pins",
        );
        return report;
    }

    let (reach, attempts) = bootstrap_checks(
        &service,
        &pins,
        target.destination,
        options.timeout,
        options.include_endpoints,
    );
    report.push(reach.with_data(json!({"attempts": attempts})));
    let lookup = yami_dht::lookup_fresh(&service, target.destination, &pins);
    drop(service);
    let record = match lookup {
        Ok(lookup) => {
            let Some(record) = lookup.record else {
                report.push(
                    Check::fail(
                        "signed-dht-lookup",
                        "Signed DHT lookup",
                        "no freshly returned signed publication was found; cached records are not accepted as this check",
                    )
                    .with_data(json!({
                        "queries": lookup.queries,
                        "failures": lookup.failures,
                        "invalid_records": lookup.invalid_records,
                        "timeouts": lookup.timeouts,
                    })),
                );
                network_checks_unavailable(
                    &mut report,
                    target,
                    options.route,
                    "no fresh signed DHT record",
                );
                return report;
            };
            if record
                .verify(network, yami_dht::now().unwrap_or(0))
                .is_err()
            {
                report.push(Check::fail(
                    "signed-dht-lookup",
                    "Signed DHT lookup",
                    "returned publication failed signature, expiry or rollback policy validation",
                ));
                network_checks_unavailable(
                    &mut report,
                    target,
                    options.route,
                    "signed record validation failed",
                );
                return report;
            }
            report.push(
                Check::pass(
                    "signed-dht-lookup",
                    "Signed DHT lookup",
                    format!(
                        "fresh signed record accepted; sequence {} expires {}; {} direct endpoint(s), {} relay route(s)",
                        record.sequence(),
                        record.expires(),
                        record.endpoints().len(),
                        record.relay_routes().len()
                    ),
                )
                .with_data(json!({
                    "sequence": record.sequence(),
                    "expires": record.expires(),
                    "queries": lookup.queries,
                    "failures": lookup.failures,
                    "invalid_records": lookup.invalid_records,
                    "timeouts": lookup.timeouts,
                    "direct_endpoints": record.endpoints().len(),
                    "relay_routes": record.relay_routes().len(),
                })),
            );
            record
        }
        Err(_) => {
            report.push(Check::fail(
                "signed-dht-lookup",
                "Signed DHT lookup",
                "fresh authenticated DHT lookup failed; peer state or transport was unavailable",
            ));
            network_checks_unavailable(&mut report, target, options.route, "DHT lookup failed");
            return report;
        }
    };

    report.push(scope_check(&record));
    let candidates = match select_routes(&record, options.route, relay_enabled) {
        Ok(candidates) => {
            report.push(
                Check::pass(
                    "route-selection",
                    "Direct versus relay route selection",
                    route_detail(options.route, relay_enabled, &candidates),
                )
                .with_data(route_data(
                    options.route,
                    relay_enabled,
                    &candidates,
                    options.include_endpoints,
                )),
            );
            candidates
        }
        Err(error) => {
            report.push(Check::fail(
                "route-selection",
                "Direct versus relay route selection",
                error,
            ));
            network_checks_unavailable(
                &mut report,
                target,
                options.route,
                "no usable authenticated route",
            );
            return report;
        }
    };

    let deadline = Instant::now() + options.timeout;
    let (mut chosen, attempts) = connect_any(&candidates, target, network, deadline);
    let mut chosen = match chosen.take() {
        Some(chosen) => chosen,
        None => {
            let identity_failure = attempts
                .iter()
                .any(|attempt| attempt["result"] == "identity-mismatch");
            report.push(
                Check::fail(
                    "destination-tls",
                    "Exact destination TLS authentication",
                    if identity_failure {
                        "TLS identity mismatch: the remote node did not prove the exact requested node ID. Nothing was requested and there is no plaintext fallback."
                    } else {
                        "no candidate completed pinned TLS 1.3 to the exact destination; there is no plaintext fallback."
                    },
                )
                .with_data(json!({"attempts": attempts})),
            );
            direct_route_check(&mut report, options.route, &candidates, false, &attempts);
            report.push(
                Check::not_run(
                    "relay-identity",
                    "Relay identity",
                    "no relay pairing and authenticated destination completed",
                )
                .optional(),
            );
            report.push(Check::fail(
                "document-retrieval",
                "YNP document retrieval",
                "not attempted because exact destination TLS authentication failed",
            ));
            png_failure_or_not_run(&mut report, target, false);
            relay_fallback_check(
                &mut report,
                options.route,
                relay_enabled,
                &record,
                &candidates,
                false,
            );
            return report;
        }
    };

    report.push(
        Check::pass(
            "destination-tls",
            "Exact destination TLS authentication",
            format!(
                "TLS 1.3 authenticated the exact destination {} over the {} path{}",
                target.destination,
                route_label(&chosen),
                if chosen.relay.is_some() {
                    "; the relay identity was authenticated by the outer TLS session"
                } else {
                    ""
                }
            ),
        )
        .with_data(route_result(&chosen, options.include_endpoints, &attempts)),
    );
    if let Some(relay) = chosen.relay {
        let check = if relay == target.destination {
            Check::fail(
                "relay-identity",
                "Relay identity",
                "the relay pin and destination pin are the same identity; use a separate relay identity",
            )
        } else {
            Check::pass(
                "relay-identity",
                "Relay identity",
                format!(
                    "outer TLS authenticated relay {relay}; the destination {0} was authenticated separately over the inner TLS session",
                    target.destination
                ),
            )
        };
        report.push(check.with_data(json!({
            "relay": relay.to_string(),
            "destination": target.destination.to_string(),
            "distinct_pins": relay != target.destination,
        })));
    } else {
        report.push(
            Check::not_run(
                "relay-identity",
                "Relay identity",
                "direct path authenticated; no relay identity was involved",
            )
            .optional(),
        );
    }
    direct_route_check(
        &mut report,
        options.route,
        &candidates,
        chosen.relay.is_none(),
        &attempts,
    );

    let mut document = None;
    match ynp::request(&mut *chosen.connection, &target.document) {
        Ok(data) if data.key == target.document => {
            let parsed = parse_document(&data.source).and_then(|document| {
                if data.style.is_empty() {
                    Ok(document)
                } else {
                    ysl::parse(&data.style).map(|_| document).map_err(|_| {
                        "authenticated stylesheet syntax validation failed".to_string()
                    })
                }
            });
            match parsed {
                Ok(parsed) => {
                    let images = yami_render::assets::image_keys(&parsed);
                    report.push(
                        Check::pass(
                            "document-retrieval",
                            "YNP document retrieval",
                            format!(
                                "authenticated YNP Get returned document '{}' ({} bytes) and a valid YLang/YSL document",
                                target.document,
                                data.source.len()
                            ),
                        )
                        .with_data(json!({
                            "document_bytes": data.source.len(),
                            "style_bytes": data.style.len(),
                            "images": images.len(),
                        })),
                    );
                    document = Some(images);
                }
                Err(error) => {
                    report.push(Check::fail(
                        "document-retrieval",
                        "YNP document retrieval",
                        error,
                    ));
                }
            }
        }
        Ok(_) => {
            report.push(Check::fail(
                "document-retrieval",
                "YNP document retrieval",
                "node returned a different document key than requested",
            ));
        }
        Err(error) => {
            report.push(Check::fail(
                "document-retrieval",
                "YNP document retrieval",
                format!(
                    "authenticated node did not return the document: {}; no plaintext fallback",
                    describe_io(&error)
                ),
            ));
        }
    }
    drop(chosen.connection);

    relay_fallback_check(
        &mut report,
        options.route,
        relay_enabled,
        &record,
        &candidates,
        true,
    );
    let asset = target
        .asset
        .clone()
        .or_else(|| document.as_ref().and_then(|images| images.first().cloned()));
    png_check(
        &mut report,
        &candidates,
        target,
        asset,
        network,
        options.timeout,
        options.include_endpoints,
    );
    report
}

fn validate_profiles(options: &Options, network: Network) -> Result<(), String> {
    if let Some(client) = &options.client {
        if client.config.role != Role::Browser {
            return Err("--client-config must use role browser".into());
        }
        if client.config.network != network {
            return Err("client and host configurations use different networks".into());
        }
    }
    if let Some(host) = &options.host {
        if host.config.role != Role::Node {
            return Err("--host-config must use role node".into());
        }
        if host.config.network != network {
            return Err("host and client configurations use different networks".into());
        }
    }
    if options.primary.config.network != network {
        return Err("primary and client configurations use different networks".into());
    }
    Ok(())
}

fn local_checks(
    report: &mut Report,
    locals: &[(&str, &Config)],
    check_listeners: bool,
) -> BTreeMap<String, NodeId> {
    let mut identities = BTreeMap::new();
    let mut identity_errors = Vec::new();
    let mut permission_results = Vec::new();
    let mut identity_data = Vec::new();
    let mut has_identity = false;
    for (label, config) in locals {
        let Some(directory) = &config.identity else {
            continue;
        };
        has_identity = true;
        match inspect_identity(directory) {
            Ok(info) => {
                identities.insert((*label).to_string(), info.id);
                identity_data.push(json!({
                    "profile": label,
                    "identity": info.id.to_string(),
                    "node_id_file": info.node_id_present,
                }));
                permission_results.push((*label, info.directory_mode_ok));
            }
            Err(error) => {
                identity_errors.push(format!("{label}: {error}"));
                identity_data.push(json!({"profile": label, "error": error}));
            }
        }
    }
    if has_identity {
        if identity_errors.is_empty() {
            report.push(
                Check::pass(
                    "identity-files",
                    "Identity files",
                    format!(
                        "{} identity directory/directories loaded; key bytes are never printed",
                        identities.len()
                    ),
                )
                .with_data(json!({"profiles": identity_data})),
            );
        } else {
            report.push(
                Check::fail(
                    "identity-files",
                    "Identity files",
                    identity_errors.join("; "),
                )
                .with_data(json!({"profiles": identity_data})),
            );
        }
    } else {
        report.push(
            Check::not_run(
                "identity-files",
                "Identity files",
                "this profile has no persistent node identity (browser/client role)",
            )
            .optional(),
        );
    }

    if has_identity {
        let denied: Vec<_> = permission_results
            .iter()
            .filter(|(_, status)| *status == Some(false))
            .map(|(label, _)| *label)
            .collect();
        let unknown: Vec<_> = permission_results
            .iter()
            .filter(|(_, status)| status.is_none())
            .map(|(label, _)| *label)
            .collect();
        if !denied.is_empty() {
            report.push(Check::fail(
                "identity-permissions",
                "Identity file permissions",
                format!(
                    "identity directory or key is accessible by group/other users for {}; use chmod 700 on the directory and chmod 600 node.key",
                    denied.join(", ")
                ),
            ));
        } else if !unknown.is_empty() {
            report.push(
                Check::not_run(
                    "identity-permissions",
                    "Identity file permissions",
                    "POSIX owner-only mode is not available on this platform, so directory/file ACLs were not inspected",
                )
                .optional(),
            );
        } else if !permission_results.is_empty() {
            report.push(Check::pass(
                "identity-permissions",
                "Identity file permissions",
                "identity directories and node.key files are owner-only",
            ));
        } else {
            report.push(
                Check::not_run(
                    "identity-permissions",
                    "Identity file permissions",
                    "identity files could not be loaded, so permissions were not assessed",
                )
                .optional(),
            );
        }
    } else {
        report.push(
            Check::not_run(
                "identity-permissions",
                "Identity file permissions",
                "browser/client profiles have no node key",
            )
            .optional(),
        );
    }

    let mut sites = Vec::new();
    for (label, config) in locals {
        if let Some(site) = &config.site {
            match inspect_site(site) {
                Ok(info) => sites.push((*label, Some(info), None)),
                Err(error) => sites.push((*label, None, Some(error))),
            }
        }
    }
    if sites.is_empty() {
        report.push(
            Check::not_run(
                "website-files",
                "Website files",
                "no website directory is configured for this profile",
            )
            .optional(),
        );
    } else {
        let errors: Vec<_> = sites
            .iter()
            .filter_map(|(label, _, error)| error.as_ref().map(|error| format!("{label}: {error}")))
            .collect();
        if errors.is_empty() {
            let pages: usize = sites
                .iter()
                .filter_map(|(_, info, _)| info.as_ref().map(|info| info.pages))
                .sum();
            let assets: usize = sites
                .iter()
                .filter_map(|(_, info, _)| info.as_ref().map(|info| info.assets))
                .sum();
            report.push(Check::pass(
                "website-files",
                "Website files",
                format!("{pages} page(s) and {assets} PNG asset(s) validated locally"),
            ));
        } else {
            report.push(Check::fail(
                "website-files",
                "Website files",
                errors.join("; "),
            ));
        }
    }

    let mut listeners = Vec::new();
    for (label, config) in locals {
        if config.listen.is_some() {
            let (status, detail) = inspect_listener(config, check_listeners);
            listeners.push((*label, status, detail, config.listen));
        }
    }
    if listeners.is_empty() {
        report.push(
            Check::not_run(
                "configured-listener",
                "Configured listener",
                "no listener is configured; outbound-only hosting or client role",
            )
            .optional(),
        );
    } else {
        let failure = listeners
            .iter()
            .find(|(_, status, _, _)| *status == Status::Fail);
        let not_run = listeners
            .iter()
            .any(|(_, status, _, _)| *status == Status::NotRun);
        let detail = listeners
            .iter()
            .map(|(label, _, detail, _)| format!("{label}: {detail}"))
            .collect::<Vec<_>>()
            .join("; ");
        let data = listeners
            .iter()
            .map(|(label, status, _, address)| {
                json!({
                    "profile": label,
                    "address": address.map(|address| address.to_string()),
                    "status": status.as_str(),
                })
            })
            .collect::<Vec<_>>();
        let check = if let Some((label, _, detail, _)) = failure {
            Check::fail(
                "configured-listener",
                "Configured listener",
                format!("{label}: {detail}"),
            )
        } else if not_run {
            Check::not_run("configured-listener", "Configured listener", detail).optional()
        } else {
            Check::pass("configured-listener", "Configured listener", detail)
        };
        report.push(check.with_data(json!({"listeners": data})));
    }

    let mut relay_status_seen = false;
    for (label, config) in locals {
        if config.role != Role::Relay {
            continue;
        }
        relay_status_seen = true;
        let expected = identities.get(*label).copied();
        let check = match yami_config::status::RelayStatus::read(&config.state) {
            Ok(status) => {
                if expected.is_some_and(|id| id != status.id) {
                    Check::fail(
                        "relay-status",
                        "Relay diagnostic snapshot",
                        format!("{label}: local relay status belongs to a different identity"),
                    )
                } else if status.running
                    && yami_dht::now()
                        .ok()
                        .is_some_and(|now| status.observed <= now && now - status.observed <= 10)
                {
                    Check::pass(
                        "relay-status",
                        "Relay diagnostic snapshot",
                        format!(
                            "{label}: relay reports running with {} connection(s), {} registration(s) and {} tunnel(s); local snapshot only, not external reachability",
                            status.connections, status.registrations, status.tunnels
                        ),
                    )
                } else {
                    Check::not_run(
                        "relay-status",
                        "Relay diagnostic snapshot",
                        format!("{label}: relay snapshot is stopped or stale; not external reachability proof"),
                    )
                }
            }
            Err(_) => Check::not_run(
                "relay-status",
                "Relay diagnostic snapshot",
                format!("{label}: no readable local relay snapshot"),
            ),
        };
        report.push(check.optional());
    }
    if !relay_status_seen {
        report.push(
            Check::not_run(
                "relay-status",
                "Relay diagnostic snapshot",
                "no relay role in the supplied profiles",
            )
            .optional(),
        );
    }
    identities
}

fn host_local_checks(
    report: &mut Report,
    host: Option<&Config>,
    destination: &NodeId,
    _identities: &BTreeMap<String, NodeId>,
    _include_endpoints: bool,
) {
    let Some(host) = host else {
        report.push(
            Check::not_run(
                "local-host-profile",
                "Local host profile",
                "host filesystem not supplied on this client; verify the website identity separately on the host",
            )
            .optional(),
        );
        report.push(
            Check::not_run(
                "local-relay-registration",
                "Local relay registration snapshot",
                "host filesystem not supplied on this client",
            )
            .optional(),
        );
        return;
    };
    let identity = host
        .identity
        .as_ref()
        .and_then(|directory| inspect_identity(directory).ok())
        .map(|info| info.id);
    let Some(identity) = identity else {
        report.push(Check::fail(
            "local-host-profile",
            "Local host profile",
            "host configuration has no readable persistent identity",
        ));
        return;
    };
    if identity != *destination {
        report.push(Check::fail(
            "local-host-profile",
            "Local host profile",
            "host configuration belongs to a different destination identity than the yami:// URL",
        ));
    } else {
        let pages = host.site.as_ref().and_then(|site| inspect_site(site).ok());
        report.push(
            Check::pass(
                "local-host-profile",
                "Local host profile",
                match pages {
                    Some(info) => format!(
                        "host identity matches the URL destination; {} page(s) and {} PNG asset(s) are present locally",
                        info.pages, info.assets
                    ),
                    None => "host identity matches the URL destination".into(),
                },
            )
            .with_data(json!({"identity": identity.to_string()})),
        );
    }
    if host.relays.is_empty() {
        report.push(
            Check::not_run(
                "local-relay-registration",
                "Local relay registration snapshot",
                "host uses a direct listener; no relay registration snapshot applies",
            )
            .optional(),
        );
        return;
    }
    match yami_node::relay_host::read_health(&host.state) {
        Ok(health) if health.id != identity => {
            report.push(Check::fail(
                "local-relay-registration",
                "Local relay registration snapshot",
                "local relay health belongs to a different destination identity",
            ));
        }
        Ok(health)
            if yami_dht::now()
                .ok()
                .is_some_and(|now| health.observed <= now && now - health.observed <= 120)
                && health.registered > 0 =>
        {
            report.push(
                Check::pass(
                    "local-relay-registration",
                    "Local relay registration snapshot",
                    format!(
                        "recent matching local snapshot with {}/{} registration slot(s); this alone is not authenticated external delivery",
                        health.registered, health.configured
                    ),
                )
                .optional(),
            );
        }
        Ok(_) => {
            report.push(
                Check::not_run(
                    "local-relay-registration",
                    "Local relay registration snapshot",
                    "local snapshot is stale or has no active registration; not external reachability proof",
                )
                .optional(),
            );
        }
        Err(_) => {
            report.push(
                Check::not_run(
                    "local-relay-registration",
                    "Local relay registration snapshot",
                    "no readable local relay health snapshot",
                )
                .optional(),
            );
        }
    }
}

fn collect_bootstraps(options: &Options, network: Network) -> Result<Vec<Contact>, String> {
    let configured = options
        .client
        .as_ref()
        .map_or(&options.primary.config.bootstrap, |client| {
            &client.config.bootstrap
        });
    let mut contacts: Vec<Contact> = configured.clone();
    for contact in &options.extra_bootstraps {
        if !contacts.iter().any(|existing| existing.id == contact.id) {
            contacts.push(*contact);
        }
    }
    for contact in &contacts {
        contact
            .validate(network)
            .map_err(|_| "a bootstrap pin is outside the selected network policy".to_string())?;
    }
    if contacts.len() > 15 {
        return Err("at most fifteen bootstrap pins can be tested".into());
    }
    Ok(contacts)
}

fn bootstrap_checks(
    service: &Service,
    pins: &[Contact],
    target: NodeId,
    timeout: Duration,
    include_endpoints: bool,
) -> (Check, Vec<Value>) {
    let deadline = Instant::now() + timeout;
    let mut attempts = Vec::new();
    let mut failures = 0;
    for (index, contact) in pins.iter().take(8).enumerate() {
        let budget = Duration::from_secs(3).min(deadline.saturating_duration_since(Instant::now()));
        if budget.is_zero() {
            break;
        }
        match yami_dht::exchange(service, *contact, None, target, budget, &PinnedTls) {
            Ok(()) => {
                let mut value = json!({
                    "peer": index + 1,
                    "id": contact.id.to_string(),
                    "result": "answered authenticated DHT query",
                });
                if include_endpoints {
                    value["endpoint"] = json!(contact.endpoint.to_string());
                }
                attempts.push(value);
            }
            Err(error) => {
                failures += 1;
                let mut value = json!({
                    "peer": index + 1,
                    "id": contact.id.to_string(),
                    "result": describe_io(&error),
                });
                if include_endpoints {
                    value["endpoint"] = json!(contact.endpoint.to_string());
                }
                attempts.push(value);
            }
        }
    }
    let attempted = attempts.len();
    let untested = pins.len().saturating_sub(attempted);
    let detail = if attempted == 0 {
        "no bootstrap peer was attempted within the bounded budget".to_string()
    } else if failures > 0 {
        format!("{failures} of {attempted} attempted bootstrap peer(s) did not answer with authenticated TLS plus a valid DHT response")
    } else if untested > 0 {
        format!("{attempted} bootstrap peer(s) answered; {untested} additional pin(s) were not tested within the bounded budget")
    } else {
        format!("all {attempted} configured bootstrap peer(s) answered with authenticated TLS and a valid DHT response")
    };
    let check = if failures > 0 {
        Check::fail(
            "bootstrap-reachability",
            "Bootstrap peer reachability",
            detail,
        )
    } else if attempted == 0 || untested > 0 {
        Check::not_run(
            "bootstrap-reachability",
            "Bootstrap peer reachability",
            detail,
        )
    } else {
        Check::pass(
            "bootstrap-reachability",
            "Bootstrap peer reachability",
            detail,
        )
    };
    (check, attempts)
}

fn network_checks_unavailable(
    report: &mut Report,
    target: &Target,
    route: RouteMode,
    reason: &'static str,
) {
    let has_bootstrap = report
        .checks()
        .iter()
        .any(|check| check.id == "bootstrap-reachability");
    let has_lookup = report
        .checks()
        .iter()
        .any(|check| check.id == "signed-dht-lookup");
    let has_route = report
        .checks()
        .iter()
        .any(|check| check.id == "route-selection");
    if !has_bootstrap {
        report.push(Check::not_run(
            "bootstrap-reachability",
            "Bootstrap peer reachability",
            format!("not attempted: {reason}"),
        ));
    }
    if !has_lookup {
        report.push(Check::not_run(
            "signed-dht-lookup",
            "Signed DHT lookup",
            format!("not attempted: {reason}"),
        ));
    }
    if !has_route {
        report.push(Check::not_run(
            "route-selection",
            "Direct versus relay route selection",
            format!("not attempted: {reason}"),
        ));
    }
    report.push(
        Check::not_run(
            "relay-identity",
            "Relay identity",
            format!("not established: {reason}"),
        )
        .optional(),
    );
    report.push(Check::not_run(
        "destination-tls",
        "Exact destination TLS authentication",
        format!("not attempted: {reason}"),
    ));
    report.push(Check::not_run(
        "document-retrieval",
        "YNP document retrieval",
        format!("not attempted: {reason}"),
    ));
    if target.asset.is_some() {
        report.push(Check::not_run(
            "png-retrieval",
            "PNG asset retrieval",
            format!("not attempted: {reason}"),
        ));
    } else {
        report.push(
            Check::not_run(
                "png-retrieval",
                "PNG asset retrieval",
                format!("not attempted: {reason}; no --asset was supplied"),
            )
            .optional(),
        );
    }
    report.push(
        Check::not_run(
            "relay-fallback",
            "Relay fallback",
            if route == RouteMode::Relay {
                format!("not attempted: {reason}")
            } else {
                format!("not exercised: {reason}")
            },
        )
        .optional(),
    );
    report.push(
        Check::not_run(
            "validation-scope",
            "Validation scope",
            "no signed record was available to classify loopback versus remote endpoints; independent machines NOT VERIFIED",
        )
        .optional(),
    );
}

fn scope_check(record: &PeerRecord) -> Check {
    let remote = record
        .endpoints()
        .iter()
        .any(|endpoint| !endpoint.ip().is_loopback())
        || record
            .relay_routes()
            .iter()
            .any(|route| !route.endpoint.ip().is_loopback());
    Check::not_run(
        "validation-scope",
        "Validation scope",
        if remote {
            "signed record uses remote numeric endpoints; independent machines and physical topology are still NOT VERIFIED"
        } else {
            "all observed endpoints are loopback; this is a simulation and independent machines are NOT VERIFIED"
        },
    )
    .optional()
    .with_data(json!({"loopback_only": !remote}))
}

#[derive(Clone, Debug)]
struct Candidate {
    relay: Option<NodeId>,
    endpoint: SocketAddrV4,
}

fn select_routes(
    record: &PeerRecord,
    mode: RouteMode,
    relay_enabled: bool,
) -> Result<Vec<Candidate>, String> {
    let direct: Vec<Candidate> = record
        .endpoints()
        .iter()
        .map(|endpoint| Candidate {
            relay: None,
            endpoint: *endpoint,
        })
        .collect();
    let relayed: Vec<Candidate> = record
        .relay_routes()
        .iter()
        .map(|RelayRoute { relay, endpoint }| Candidate {
            relay: Some(*relay),
            endpoint: *endpoint,
        })
        .collect();
    match mode {
        RouteMode::Direct => {
            if direct.is_empty() {
                return Err(
                    "the signed record has no direct endpoint; use --route relay or --route auto"
                        .into(),
                );
            }
            Ok(direct)
        }
        RouteMode::Relay => {
            if relayed.is_empty() {
                return Err("the signed record has no relay route".into());
            }
            if !relay_enabled {
                return Err(
                    "relay fallback is not explicitly enabled; set relay-fallback yes before testing a relay path"
                        .into(),
                );
            }
            Ok(relayed)
        }
        RouteMode::Auto => {
            if direct.is_empty() && relayed.is_empty() {
                return Err(
                    "the signed record has no usable direct endpoint or relay route".into(),
                );
            }
            if direct.is_empty() && !relay_enabled {
                return Err(
                    "the record is relay-only but relay fallback is not explicitly enabled; set relay-fallback yes"
                        .into(),
                );
            }
            let mut candidates = direct;
            if relay_enabled {
                candidates.extend(relayed);
            }
            Ok(candidates)
        }
    }
}

fn route_detail(mode: RouteMode, relay_enabled: bool, candidates: &[Candidate]) -> String {
    let direct = candidates.iter().filter(|c| c.relay.is_none()).count();
    let relay = candidates.len() - direct;
    format!(
        "mode {}; {direct} direct candidate(s), {relay} relay candidate(s); relay fallback explicitly {}; direct candidates are tried before enabled relay routes",
        mode.as_str(),
        if relay_enabled { "enabled" } else { "disabled" }
    )
}

fn route_data(
    mode: RouteMode,
    relay_enabled: bool,
    candidates: &[Candidate],
    include_endpoints: bool,
) -> Value {
    let list: Vec<Value> = candidates
        .iter()
        .map(|candidate| {
            let mut value = json!({
                "path": if candidate.relay.is_some() { "relay" } else { "direct" },
                "relay": candidate.relay.map(|relay| relay.to_string()),
            });
            if include_endpoints {
                value["endpoint"] = json!(candidate.endpoint.to_string());
            }
            value
        })
        .collect();
    json!({
        "mode": mode.as_str(),
        "relay_fallback_enabled": relay_enabled,
        "candidates": list,
    })
}

struct Chosen {
    connection: Box<dyn Connection>,
    relay: Option<NodeId>,
    endpoint: SocketAddrV4,
}

fn route_label(chosen: &Chosen) -> &'static str {
    if chosen.relay.is_some() {
        "relay"
    } else {
        "direct"
    }
}

fn connect_any(
    candidates: &[Candidate],
    target: &Target,
    network: Network,
    deadline: Instant,
) -> (Option<Chosen>, Vec<Value>) {
    let mut attempts = Vec::new();
    for candidate in candidates {
        let budget = Duration::from_secs(3).min(deadline.saturating_duration_since(Instant::now()));
        if budget.is_zero() {
            break;
        }
        let result = connect_candidate(candidate, &target.destination, network, budget);
        match result {
            Ok(connection) => {
                if connection.protection() != Protection::Tls13PinnedServer {
                    attempts.push(json!({
                        "path": if candidate.relay.is_some() { "relay" } else { "direct" },
                        "relay": candidate.relay.map(|relay| relay.to_string()),
                        "result": "identity-mismatch",
                    }));
                    continue;
                }
                attempts.push(json!({
                    "path": if candidate.relay.is_some() { "relay" } else { "direct" },
                    "relay": candidate.relay.map(|relay| relay.to_string()),
                    "result": "authenticated",
                }));
                return (
                    Some(Chosen {
                        connection,
                        relay: candidate.relay,
                        endpoint: candidate.endpoint,
                    }),
                    attempts,
                );
            }
            Err(error) => attempts.push(json!({
                "path": if candidate.relay.is_some() { "relay" } else { "direct" },
                "relay": candidate.relay.map(|relay| relay.to_string()),
                "result": describe_io(&error),
            })),
        }
    }
    (None, attempts)
}

fn connect_candidate(
    candidate: &Candidate,
    destination: &NodeId,
    network: Network,
    budget: Duration,
) -> io::Result<Box<dyn Connection>> {
    match candidate.relay {
        Some(relay) => yami_relay::Route {
            relay,
            endpoint: candidate.endpoint.into(),
            destination: *destination,
        }
        .connect_with_timeout(network == Network::Public, budget),
        None => ynp_tls::TlsTransport::new(*destination)?
            .connect_with_timeout(candidate.endpoint.into(), budget),
    }
}

fn route_result(chosen: &Chosen, include_endpoints: bool, attempts: &[Value]) -> Value {
    let mut data = json!({
        "authenticated": chosen.connection.protection() == Protection::Tls13PinnedServer,
        "path": route_label(chosen),
        "relay": chosen.relay.map(|relay| relay.to_string()),
        "attempts": attempts,
    });
    if include_endpoints {
        data["endpoint"] = json!(chosen.endpoint.to_string());
    }
    data
}

fn direct_route_check(
    report: &mut Report,
    mode: RouteMode,
    candidates: &[Candidate],
    chosen_direct: bool,
    attempts: &[Value],
) {
    let has_direct = candidates.iter().any(|candidate| candidate.relay.is_none());
    let check = match mode {
        RouteMode::Direct => {
            if chosen_direct {
                Check::pass(
                    "direct-route",
                    "Direct route",
                    "direct destination TLS path authenticated",
                )
            } else {
                Check::fail(
                    "direct-route",
                    "Direct route",
                    "direct mode was requested but no direct endpoint authenticated",
                )
            }
        }
        RouteMode::Relay => Check::not_run(
            "direct-route",
            "Direct route",
            "direct mode was not requested; relay path was tested explicitly",
        ),
        RouteMode::Auto if !has_direct => Check::not_run(
            "direct-route",
            "Direct route",
            "the signed record has no direct endpoint; direct failure is not applicable",
        ),
        RouteMode::Auto if chosen_direct => Check::pass(
            "direct-route",
            "Direct route",
            "direct destination TLS path authenticated; relay fallback was not needed",
        ),
        RouteMode::Auto => Check::not_run(
            "direct-route",
            "Direct route",
            "direct candidate(s) did not authenticate; an enabled relay route was selected instead",
        ),
    };
    report.push(check.optional().with_data(json!({"attempts": attempts})));
}

fn relay_fallback_check(
    report: &mut Report,
    mode: RouteMode,
    relay_enabled: bool,
    record: &PeerRecord,
    candidates: &[Candidate],
    success: bool,
) {
    let has_relay = !record.relay_routes().is_empty();
    let check = match mode {
        RouteMode::Direct => Check::not_run(
            "relay-fallback",
            "Relay fallback",
            "direct route mode was requested; relay fallback was not tested",
        ),
        RouteMode::Relay if success => Check::pass(
            "relay-fallback",
            "Relay fallback",
            "explicit relay path authenticated the destination end-to-end; direct failure was not part of this run",
        ),
        RouteMode::Relay => Check::fail(
            "relay-fallback",
            "Relay fallback",
            "explicit relay path did not authenticate the destination",
        ),
        RouteMode::Auto if !has_relay => Check::not_run(
            "relay-fallback",
            "Relay fallback",
            "the signed record advertises no relay route",
        ),
        RouteMode::Auto if !relay_enabled => Check::not_run(
            "relay-fallback",
            "Relay fallback",
            "relay routes are advertised but relay fallback is explicitly disabled; no relay was attempted",
        ),
        RouteMode::Auto if success && candidates.iter().any(|candidate| candidate.relay.is_some()) => {
            let direct_failed = candidates.iter().any(|candidate| candidate.relay.is_none());
            Check::pass(
                "relay-fallback",
                "Relay fallback",
                if direct_failed {
                    "direct candidate(s) failed; explicitly enabled relay fallback authenticated the destination"
                } else {
                    "record was relay-only; explicitly enabled relay path authenticated the destination"
                },
            )
        }
        RouteMode::Auto if has_relay && relay_enabled && !success => Check::fail(
            "relay-fallback",
            "Relay fallback",
            "explicitly enabled relay fallback did not authenticate the destination",
        ),
        RouteMode::Auto => Check::not_run(
            "relay-fallback",
            "Relay fallback",
            "direct path succeeded without relay fallback",
        ),
    };
    report.push(check.optional());
}

fn parse_document(source: &str) -> Result<ylang::Document, String> {
    // The browser accepts native yami:// links by adapting them before the
    // unchanged YLang parser. Do the same here without changing YLang.
    let mut adapted = String::with_capacity(source.len());
    for line in source.split_inclusive('\n') {
        if line.trim_start().starts_with("> yami://") {
            adapted.push_str(&line.replacen("> yami://", "> ynp://", 1));
        } else {
            adapted.push_str(line);
        }
    }
    ylang::parse(&adapted)
        .map_err(|_| "authenticated document failed YLang syntax validation".into())
}

fn png_failure_or_not_run(report: &mut Report, target: &Target, document_ok: bool) {
    if target.asset.is_some() {
        report.push(Check::fail(
            "png-retrieval",
            "PNG asset retrieval",
            "not attempted because the document/route did not complete",
        ));
    } else if document_ok {
        report.push(
            Check::not_run(
                "png-retrieval",
                "PNG asset retrieval",
                "the document references no image and no --asset was supplied",
            )
            .optional(),
        );
    } else {
        report.push(
            Check::not_run(
                "png-retrieval",
                "PNG asset retrieval",
                "document retrieval failed and no --asset was supplied",
            )
            .optional(),
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn png_check(
    report: &mut Report,
    candidates: &[Candidate],
    target: &Target,
    asset: Option<String>,
    network: Network,
    timeout: Duration,
    include_endpoints: bool,
) {
    let Some(asset) = asset else {
        report.push(
            Check::not_run(
                "png-retrieval",
                "PNG asset retrieval",
                "the document references no image and no --asset was supplied",
            )
            .optional(),
        );
        return;
    };
    if ynp::validate_asset_key(&asset).is_err() {
        report.push(Check::fail(
            "png-retrieval",
            "PNG asset retrieval",
            "asset key is invalid; expected 1..64 safe ASCII characters",
        ));
        return;
    }
    let auto_selected = target.asset.is_none();
    let deadline = Instant::now() + timeout;
    let mut attempts = Vec::new();
    let mut last_error = None;
    let mut fetched = None;
    let mut tries = 0;
    while Instant::now() < deadline && tries < 6 {
        tries += 1;
        let (candidate, mut candidate_attempts) =
            connect_any(candidates, target, network, deadline);
        attempts.append(&mut candidate_attempts);
        let Some(mut chosen) = candidate else {
            std::thread::sleep(Duration::from_millis(200));
            continue;
        };
        if chosen.connection.protection() != Protection::Tls13PinnedServer {
            last_error = Some("destination identity was not authenticated");
            break;
        }
        match ynp::request_asset(&mut *chosen.connection, &asset, ynp::MAX_ASSET_BYTES) {
            Ok(data) if data.key == asset => {
                fetched = Some((data, chosen));
                break;
            }
            Ok(_) => {
                last_error = Some("node returned a different asset key");
                break;
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                last_error = Some("asset was not found");
                break;
            }
            Err(error) => {
                last_error = Some(describe_io(&error));
                std::thread::sleep(Duration::from_millis(200));
            }
        }
    }
    let Some((data, chosen)) = fetched else {
        report.push(
            Check::fail(
                "png-retrieval",
                "PNG asset retrieval",
                format!(
                    "a second authenticated connection/request could not retrieve the PNG after {tries} attempt(s): {}",
                    last_error.unwrap_or("no candidate authenticated")
                ),
            )
            .with_data(json!({"attempts": attempts})),
        );
        return;
    };
    drop(chosen.connection);
    match yami_render::assets::decode_png(&data.bytes, yami_render::assets::MAX_DECODED) {
        Ok(image) if image.valid() => {
            let mut value = json!({
                "asset": asset,
                "width": image.width,
                "height": image.height,
                "bytes": data.bytes.len(),
                "auto_selected": auto_selected,
                "relay": chosen.relay.map(|relay| relay.to_string()),
                "attempts": attempts,
            });
            if include_endpoints {
                value["endpoint"] = json!(chosen.endpoint.to_string());
                value["path"] = json!(if chosen.relay.is_some() {
                    "relay"
                } else {
                    "direct"
                });
            }
            report.push(
                Check::pass(
                    "png-retrieval",
                    "PNG asset retrieval",
                    format!(
                        "authenticated PNG '{}' retrieved and decoded: {} x {}, {} bytes",
                        asset,
                        image.width,
                        image.height,
                        data.bytes.len()
                    ),
                )
                .with_data(value),
            );
        }
        _ => {
            report.push(Check::fail(
                "png-retrieval",
                "PNG asset retrieval",
                "returned asset is not a bounded, non-animated PNG",
            ));
        }
    }
}

fn describe_io(error: &io::Error) -> &'static str {
    if error
        .get_ref()
        .and_then(|source| source.downcast_ref::<ynp_tls::IdentityMismatch>())
        .is_some()
    {
        "identity-mismatch"
    } else {
        match error.kind() {
            io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => "timeout",
            io::ErrorKind::ConnectionRefused
            | io::ErrorKind::ConnectionReset
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::NotConnected
            | io::ErrorKind::UnexpectedEof
            | io::ErrorKind::BrokenPipe => "unreachable",
            io::ErrorKind::NotFound => "not-found",
            io::ErrorKind::PermissionDenied => "permission-denied",
            io::ErrorKind::InvalidData | io::ErrorKind::InvalidInput => "rejected",
            _ => "failed",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record() -> (PeerRecord, yami_identity::NodeIdentity) {
        let identity = yami_identity::NodeIdentity::generate().unwrap();
        let now = yami_dht::now().unwrap();
        let record = PeerRecord::sign(
            &identity,
            vec!["127.0.0.1:7443".parse().unwrap()],
            now + 60,
            1,
            Network::Loopback,
            now,
        )
        .unwrap();
        (record, identity)
    }

    #[test]
    fn target_url_requires_exact_identity_and_safe_keys() {
        let id = yami_identity::NodeIdentity::generate()
            .unwrap()
            .id()
            .to_string();
        let target = Target::parse(&format!("yami://{id}/home"), Some("cover")).unwrap();
        assert_eq!(target.document, "home");
        assert!(Target::parse("ynps://127.0.0.1:1/home", None).is_err());
        assert!(Target::parse(&format!("yami://{id}/../bad"), None).is_err());
        assert!(Target::parse(&format!("yami://{id}/home"), Some("bad key")).is_err());
        assert!(Target::parse(&format!("yami://{id}"), None).is_err());
    }

    #[test]
    fn direct_relay_and_auto_selection_are_explicit() {
        let (record, _) = record();
        let direct = select_routes(&record, RouteMode::Direct, false).unwrap();
        assert_eq!(direct.len(), 1);
        assert!(direct[0].relay.is_none());
        assert!(select_routes(&record, RouteMode::Relay, false).is_err());
        assert!(select_routes(&record, RouteMode::Relay, true).is_err());
        assert!(select_routes(&record, RouteMode::Auto, true).unwrap().len() == 1);
    }

    #[test]
    fn relay_only_records_need_explicit_fallback() {
        let identity = yami_identity::NodeIdentity::generate().unwrap();
        let relay = yami_identity::NodeIdentity::generate().unwrap();
        let now = yami_dht::now().unwrap();
        let record = PeerRecord::sign_routes(
            &identity,
            vec![],
            vec![RelayRoute {
                relay: *relay.id(),
                endpoint: "127.0.0.1:7550".parse().unwrap(),
            }],
            now + 60,
            1,
            Network::Loopback,
            now,
        )
        .unwrap();
        assert!(select_routes(&record, RouteMode::Auto, false).is_err());
        let selected = select_routes(&record, RouteMode::Auto, true).unwrap();
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].relay, Some(*relay.id()));
    }

    #[test]
    fn document_adaptation_accepts_native_yami_links() {
        let id = yami_identity::NodeIdentity::generate()
            .unwrap()
            .id()
            .to_string();
        let source = format!("@page 0.2\n  title Example\n= Hello\n> yami://{id}/about | About\n");
        parse_document(&source).unwrap();
        assert!(parse_document("@page 0.2\n= [broken\n").is_err());
    }
}
