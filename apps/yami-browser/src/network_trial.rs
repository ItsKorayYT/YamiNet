//! Operator-invoked checks of existing TLS/DHT/relay/PNG protocols. No scans or infrastructure changes.
use crate::{
    native::{Address, Resolver},
    validation,
};
use serde_json::{json, Value};
use std::{
    io,
    time::{Duration, Instant},
};
use yami_config::{Config, Role};

fn step(steps: &mut Vec<Value>, name: &str, status: &str, evidence: &str) {
    steps.push(json!({"name":name,"status":status,"evidence":evidence}));
}
fn observation(o: &validation::Observation, endpoints: bool) -> Value {
    json!({"destination":o.destination.to_string(),"route":if o.origin.relay.is_some(){"relay"}else{"direct"},"relay":o.origin.relay.map(|id|id.to_string()),"endpoint":if endpoints {Some(o.origin.endpoint.to_string())}else{None},"png":o.png,"elapsed_ms":o.elapsed_ms})
}
/// Optional checks are explicit operator requests. A game is fetched and run
/// only when --game KEY is supplied, and only when advertised by the page.
#[derive(Default, Clone, Copy)]
pub struct Checks<'a> {
    pub name: Option<&'a str>,
    pub navigation: Option<&'a str>,
    pub game: Option<&'a str>,
}
pub fn run(
    config: &Config,
    url: &str,
    asset: &str,
    host: Option<&Config>,
    include_endpoints: bool,
    cycle: bool,
) -> (Value, bool) {
    run_with_checks(
        config,
        url,
        asset,
        host,
        include_endpoints,
        cycle,
        Checks::default(),
    )
}
pub fn run_with_checks(
    config: &Config,
    url: &str,
    asset: &str,
    host: Option<&Config>,
    include_endpoints: bool,
    cycle: bool,
    checks: Checks<'_>,
) -> (Value, bool) {
    let mut steps = Vec::new();
    let mut phase = "configuration";
    let outcome = (|| -> Result<(), String> {
        config
            .expect_role(Role::Browser)
            .map_err(|_| "browser-role configuration required")?;
        let address = Address::parse(url)?;
        ynp::validate_asset_key(asset).map_err(|_| "invalid asset key")?;
        step(
            &mut steps,
            "configuration",
            "PASS",
            "strict browser profile and full destination identity validated",
        );
        if config.bootstrap.is_empty() || config.bootstrap.len() > 8 {
            return Err("trial requires one to eight explicitly pinned bootstrap peers".into());
        }
        phase = "bootstrap-dht";
        let resolver = Resolver::from_config(config)?;
        let service = yami_dht::Service::open_lookup(resolver.state_path(), config.network)
            .map_err(|_| "DHT state unavailable or locked")?;
        let deadline = Instant::now() + Duration::from_secs(15);
        for contact in &config.bootstrap {
            let budget =
                Duration::from_secs(3).min(deadline.saturating_duration_since(Instant::now()));
            let passed = !budget.is_zero()
                && yami_dht::exchange(
                    &service,
                    *contact,
                    None,
                    address.id,
                    budget,
                    &yami_dht::PinnedTls,
                )
                .is_ok();
            let mut record = json!({"name":"bootstrap-pin", "status":if passed {"PASS"} else {"FAIL"}, "id":contact.id.to_string(), "evidence":"bounded exact-identity TLS plus existing DHT FIND response; no plaintext fallback"});
            if include_endpoints {
                record["endpoint"] = json!(contact.endpoint.to_string());
            }
            steps.push(record);
        }
        phase = "signed-publication-resolution";
        let lookup = yami_dht::lookup_fresh(&service, address.id, &config.bootstrap)
            .map_err(|_| "fresh authenticated DHT lookup failed")?;
        let record = lookup.record.ok_or("no currently returned valid signed publication (cached records do not pass this check)")?;
        record
            .verify(
                config.network,
                yami_dht::now().map_err(|_| "clock unavailable")?,
            )
            .map_err(|_| "signed publication failed freshness/policy verification")?;
        steps.push(json!({"name":"signed-publication-resolution","status":"PASS","sequence":record.sequence(),"expires":record.expires(),"queries":lookup.queries,"failures":lookup.failures,"evidence":"record returned by authenticated FIND and accepted by existing expiry/rollback checks"}));
        drop(service);
        phase = "local-host-profile";
        if let Some(host) = host {
            host.expect_role(Role::Node)
                .map_err(|_| "host configuration has wrong role")?;
            let id = yami_identity::NodeIdentity::load(
                host.identity.as_ref().ok_or("host identity missing")?,
            )
            .map_err(|_| "host identity unreadable")?;
            if *id.id() != address.id {
                return Err("host profile belongs to a different destination identity".into());
            }
            let report =
                yami_node::authoring::check(host.site.as_ref().ok_or("host site missing")?)
                    .map_err(|_| "host site validation failed")?;
            steps.push(json!({"name":"local-host-profile","status":"PASS","pages":report.pages.len(),"assets":report.assets.len()}));
            let now = yami_dht::now().map_err(|_| "clock unavailable")?;
            if !host.relays.is_empty() {
                match yami_node::relay_host::read_health(&host.state) {
                    Ok(h) if h.id == address.id && h.observed <= now && now-h.observed <= 120 && h.registered > 0 => step(&mut steps,"local-registration-snapshot","PASS","recent matching local snapshot; this alone is not authenticated external delivery"),
                    _ => step(&mut steps,"local-registration-snapshot","FAIL","missing, stale, wrong-owner or unregistered snapshot"),
                }
            }
        } else {
            step(
                &mut steps,
                "local-host-profile",
                "NOT RUN",
                "host filesystem not supplied on independent client; verify separately on host",
            );
        }
        if let Some(name) = checks.name {
            phase = "yns-resolution";
            let name = yami_yns::Name::parse(name).map_err(|_| "invalid readable name")?;
            if matches!(name, yami_yns::Name::Alias(_)) {
                return Err("trial requires a published name, not a local alias".into());
            }
            let id = resolver.resolve_name_until(
                &name,
                Instant::now() + yami_yns::network::DISCOVERY_BUDGET,
                true,
            )?;
            if id != address.id {
                return Err("signed name targets a different identity than the explicitly pinned trial destination".into());
            }
            step(&mut steps,"yns-resolution","PASS","forced authenticated provider retrieval and validated cache; signed name matches the independently supplied full destination identity");
        } else {
            step(
                &mut steps,
                "yns-resolution",
                "NOT RUN",
                "pass --name CANONICAL-NAME to test a network YNS refresh",
            );
        }
        phase = "document-and-png";
        let baseline = validation::inspect(url, Some(asset), &resolver)?;
        steps.push(json!({"name":"document-and-png","status":"PASS","observation":observation(&baseline,include_endpoints)}));
        step(&mut steps,"authenticated-relay-registration",if baseline.origin.relay.is_some(){"PASS"}else{"NOT RUN"},"successful nested authenticated relay tunnel proves a live destination route at fetch time; direct routes do not test relays");
        if let Some(key) = checks.navigation {
            phase = "page-navigation";
            ynp::validate_key(key).map_err(|_| "invalid navigation key")?;
            if !baseline.link_targets.contains(&format!("/{key}"))
                && !baseline
                    .link_targets
                    .contains(&format!("yami://{}/{key}", address.id))
            {
                return Err("requested navigation is not advertised as a same-node link".into());
            }
            let linked =
                validation::inspect(&format!("yami://{}/{key}", address.id), None, &resolver)?;
            if linked.destination != address.id {
                return Err("navigation changed destination".into());
            }
            steps.push(json!({"name":"page-navigation","status":"PASS","observation":observation(&linked,include_endpoints),"evidence":"advertised same-node link fetched and parsed with the original destination pin"}));
        } else {
            step(
                &mut steps,
                "page-navigation",
                "NOT RUN",
                "pass --navigate PAGE-KEY; GUI click/back controls are checked separately",
            );
        }
        if let Some(key) = checks.game {
            phase = "hosted-game";
            steps.push(game_check(&baseline, key)?);
        } else {
            step(&mut steps,"hosted-game","NOT RUN","game execution requires explicit --game GAME-KEY consent; pages alone never start a VM");
        }
        if cycle {
            phase = "operator-relay-cycle";
            let relay = baseline
                .origin
                .relay
                .ok_or("operator relay cycle requires a relayed baseline")?;
            eprintln!("Stop the selected relay {relay} through your normal operator controls. Type stopped, then Enter. No infrastructure will be changed by this runner.");
            confirmation("stopped")?;
            let outage = validation::inspect(url, Some(asset), &resolver);
            let detected = outage
                .as_ref()
                .map_or(true, |o| o.origin.relay != Some(relay));
            step(
                &mut steps,
                "operator-relay-outage",
                if detected { "PASS" } else { "FAIL" },
                "expected failure or authenticated alternate route after operator action",
            );
            eprintln!("Restore that relay through your normal operator controls. Type restarted, then Enter.");
            confirmation("restarted")?;
            let deadline = Instant::now() + Duration::from_secs(30);
            let mut recovered = None;
            while Instant::now() < deadline {
                if let Ok(o) = validation::inspect(url, Some(asset), &resolver) {
                    recovered = Some(o);
                    break;
                }
                std::thread::sleep(Duration::from_millis(500));
            }
            if let Some(o) = recovered {
                steps.push(json!({"name":"operator-relay-recovery","status":"PASS","observation":observation(&o,include_endpoints)}));
                if let Some(key) = checks.game {
                    phase = "hosted-game-after-recovery";
                    let mut game = game_check(&o, key)?;
                    game["name"] = json!("hosted-game-after-recovery");
                    steps.push(game);
                }
            } else {
                step(
                    &mut steps,
                    "operator-relay-recovery",
                    "FAIL",
                    "no authenticated document/PNG delivery within recovery window",
                );
            }
        } else {
            step(
                &mut steps,
                "operator-relay-cycle",
                "NOT RUN",
                "requires explicit --operator-relay-cycle and operator confirmations",
            );
        }
        Ok(())
    })();
    if outcome.is_err() {
        step(&mut steps,phase,"FAIL","an explicitly requested configuration, identity, DHT, YNS, transport, navigation, PNG or sandboxed game check failed; raw errors intentionally omitted");
    }
    let success = outcome.is_ok() && !steps.iter().any(|s| s["status"] == "FAIL");
    (
        json!({"schema":"yaminet-network-trial-1","version":env!("CARGO_PKG_VERSION"),"result":if success{"PASS"}else{"FAIL"},"physical_topology":"NOT VERIFIED; operator must record independent machines and connection types", "gui_rendering":"NOT RUN", "endpoints_included":include_endpoints,"steps":steps}),
        success,
    )
}
fn game_check(observation: &validation::Observation, key: &str) -> Result<Value, String> {
    use std::sync::Arc;
    ynp::validate_asset_key(key).map_err(|_| "invalid game key")?;
    if !observation.link_targets.contains(&format!("game:{key}")) {
        return Err("requested game is not advertised by the authenticated page".into());
    }
    let session = crate::hosted_games::Source::Hosted {
        origin: observation.origin.clone(),
        key: key.into(),
    }
    .launch(Arc::new(|| {}))?;
    let deadline = Instant::now() + Duration::from_secs(12);
    let mut pressed_at = None;
    while Instant::now() < deadline {
        let snapshot = session.snapshot();
        if let yami_games::Status::Failed(_) = snapshot.status {
            session.stop();
            return Err("hosted transfer, integrity, ABI or sandbox execution failed".into());
        }
        if snapshot.status == yami_games::Status::Running
            && snapshot.frames > 0
            && pressed_at.is_none()
        {
            session.key(yami_games::START, true);
            pressed_at = Some(snapshot.frames);
        }
        if pressed_at.is_some_and(|frame| snapshot.frames >= frame + 3)
            && snapshot
                .frame
                .as_ref()
                .is_some_and(|f| !f.commands.is_empty())
        {
            session.stop();
            return Ok(
                json!({"name":"hosted-game","status":"PASS","frames":snapshot.frames,"draw_commands":snapshot.frame.as_ref().map(|f|f.commands.len()),"evidence":"explicit opt-in; manifest and Wasm fetched through original destination/relay pins; digest and ABI checked; existing bounded no-WASI sandbox produced draw commands after synthetic START input","gui_rendering":"NOT RUN"}),
            );
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    session.stop();
    Err("hosted game did not produce bounded frames before timeout".into())
}
fn confirmation(expected: &str) -> Result<(), String> {
    use std::io::{BufRead, Read};
    let mut input = String::new();
    io::stdin()
        .lock()
        .take(65)
        .read_line(&mut input)
        .map_err(|_| "operator input failed")?;
    if input.len() > 64 {
        return Err("operator confirmation too long".into());
    }
    if input.lines().next() != Some(expected) {
        return Err("operator confirmation missing".into());
    }
    Ok(())
}
