//! Operator entry point for the public-testnet deployment kit.
use std::{
    env, fmt,
    net::{Ipv4Addr, SocketAddrV4},
    path::PathBuf,
    process::ExitCode,
    time::Duration,
};

use yami_config::Role;
use yami_dht::{Contact, Network};
use yami_identity::NodeId;
use yami_testnet::{
    config::Loaded,
    preflight::{self, RouteMode, Target},
    profile::{self, InitOptions, JoinPins, Mode, Ports},
    render::{self, UnitRole, UnitSpec},
    report::{Check, Report},
};

const USAGE: &str = "YamiNet public-testnet kit

Usage:
  yami-testnet check-config <FILE> [--role node|relay|browser] [--check-listeners] [--json]
  yami-testnet preflight --config <FILE> --url <yami://NODE-ID/KEY>
      [--client-config <FILE>] [--host-config <FILE>] [--asset <KEY>]
      [--bootstrap <NODE-ID@IPv4:port>]... [--route auto|direct|relay]
      [--timeout 5..60] [--check-listeners] [--include-endpoints] [--json]
  yami-testnet init --profile <DIR> [--mode loopback|public]
      [--bind-ip <IPv4> --public-ip <IPv4>] [--relay] [--site] [--client]
      [--binary-dir <DIR>] [--user <ACCOUNT>] [--group <ACCOUNT>]
      [--bootstrap-port <PORT>] [--relay-port <PORT>] [--site-port <PORT>]
  yami-testnet init --profile <DIR> --home --site
      --bootstrap-pin <ID@PUBLIC-IP:port> --relay-pin <ID@PUBLIC-IP:port>
      [--client] [--binary-dir <DIR>] [--user <ACCOUNT>] [--group <ACCOUNT>]
  yami-testnet render-unit --role bootstrap|relay|host --config <FILE>
      [--binary <PATH>] [--user <ACCOUNT>] [--group <ACCOUNT>]
      [--output <FILE>] [--force]
  yami-testnet remove --profile <DIR> --confirm --purge-data
  yami-testnet --version | --help

Commands never start a service, install a unit, change a firewall or contact a
network during init/remove. Preflight only performs bounded checks requested on
the command line. PASS means automated local validation; independent machines
are always reported as NOT VERIFIED.";

#[derive(Debug)]
enum CliError {
    Usage(String),
    Failed(String),
}

impl fmt::Display for CliError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Usage(message) | Self::Failed(message) => formatter.write_str(message),
        }
    }
}

fn usage(message: impl Into<String>) -> CliError {
    CliError::Usage(message.into())
}

fn failed(message: impl Into<String>) -> CliError {
    CliError::Failed(message.into())
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => ExitCode::from(code as u8),
        Err(CliError::Usage(message)) => {
            eprintln!("yami-testnet: {message}\n\n{USAGE}");
            ExitCode::from(64)
        }
        Err(CliError::Failed(message)) => {
            eprintln!("yami-testnet: {message}");
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<i32, CliError> {
    let args: Vec<String> = env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None | Some("--help") | Some("-h") => {
            println!("{USAGE}");
            Ok(0)
        }
        Some("--version") => {
            println!("YamiNet Testnet {}", env!("CARGO_PKG_VERSION"));
            Ok(0)
        }
        Some("check-config") => check_config(&args[1..]),
        Some("preflight") => preflight_command(&args[1..]),
        Some("init") => init_command(&args[1..]),
        Some("render-unit") => render_command(&args[1..]),
        Some("remove") => remove_command(&args[1..]),
        Some(other) => Err(usage(format!("unknown command '{other}'"))),
    }
}

fn emit(report: &Report, json: bool) -> Result<(), CliError> {
    if json {
        let text = serde_json::to_string_pretty(&report.to_json())
            .map_err(|_| failed("report could not be serialized"))?;
        println!("{text}");
    } else {
        print!("{}", report.to_text());
    }
    Ok(())
}

fn parse_role(text: &str) -> Result<Role, CliError> {
    match text {
        "node" => Ok(Role::Node),
        "relay" => Ok(Role::Relay),
        "browser" => Ok(Role::Browser),
        _ => Err(usage("--role must be node, relay or browser")),
    }
}

fn set_once<T>(slot: &mut Option<T>, value: T, name: &str) -> Result<(), CliError> {
    if slot.is_some() {
        return Err(usage(format!("{name} was supplied more than once")));
    }
    *slot = Some(value);
    Ok(())
}

fn check_config(args: &[String]) -> Result<i32, CliError> {
    let mut file = None;
    let mut role = None;
    let mut json = false;
    let mut check_listeners = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--role" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| usage("--role needs a value"))?;
                set_once(&mut role, parse_role(value)?, "--role")?;
            }
            "--json" => json = true,
            "--check-listeners" => check_listeners = true,
            "--help" | "-h" => {
                println!("{USAGE}");
                return Ok(0);
            }
            value if value.starts_with('-') => {
                return Err(usage(format!("unknown option '{value}'")))
            }
            value => set_once(&mut file, PathBuf::from(value), "configuration file")?,
        }
        index += 1;
    }
    let file = file.ok_or_else(|| usage("check-config needs a configuration file"))?;
    let report = preflight::check_config(&file, role, check_listeners);
    emit(&report, json)?;
    Ok(report.outcome().exit_code())
}

fn preflight_command(args: &[String]) -> Result<i32, CliError> {
    let mut config = None;
    let mut client_config = None;
    let mut host_config = None;
    let mut url = None;
    let mut asset = None;
    let mut route = RouteMode::Auto;
    let mut route_set = false;
    let mut pins: Vec<(String, String)> = Vec::new();
    let mut timeout = 30u64;
    let mut timeout_set = false;
    let mut check_listeners = false;
    let mut include_endpoints = false;
    let mut json = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--config" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| usage("--config needs a file"))?;
                set_once(&mut config, PathBuf::from(value), "--config")?;
            }
            "--client-config" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| usage("--client-config needs a file"))?;
                set_once(&mut client_config, PathBuf::from(value), "--client-config")?;
            }
            "--host-config" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| usage("--host-config needs a file"))?;
                set_once(&mut host_config, PathBuf::from(value), "--host-config")?;
            }
            "--url" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| usage("--url needs a value"))?;
                set_once(&mut url, value.to_string(), "--url")?;
            }
            "--asset" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| usage("--asset needs a key"))?;
                set_once(&mut asset, value.to_string(), "--asset")?;
            }
            "--route" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| usage("--route needs a value"))?;
                if route_set {
                    return Err(usage("--route was supplied more than once"));
                }
                route = RouteMode::parse(value).map_err(usage)?;
                route_set = true;
            }
            "--bootstrap" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| usage("--bootstrap needs ID@IPv4:port"))?;
                let (id, endpoint) = value
                    .split_once('@')
                    .ok_or_else(|| usage("--bootstrap needs full-node-ID@IPv4:port"))?;
                pins.push((id.to_string(), endpoint.to_string()));
            }
            "--timeout" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| usage("--timeout needs seconds"))?;
                if timeout_set {
                    return Err(usage("--timeout was supplied more than once"));
                }
                timeout = value
                    .parse()
                    .map_err(|_| usage("--timeout must be an integer"))?;
                if !(5..=60).contains(&timeout) {
                    return Err(usage("--timeout must be 5..60 seconds"));
                }
                timeout_set = true;
            }
            "--check-listeners" => check_listeners = true,
            "--include-endpoints" => include_endpoints = true,
            "--json" => json = true,
            "--help" | "-h" => {
                println!("{USAGE}");
                return Ok(0);
            }
            value if value.starts_with('-') => {
                return Err(usage(format!("unknown option '{value}'")))
            }
            value => return Err(usage(format!("unexpected argument '{value}'"))),
        }
        index += 1;
    }
    let config = config.ok_or_else(|| usage("preflight needs --config"))?;
    let url = url.ok_or_else(|| usage("preflight needs --url yami://NODE-ID/KEY"))?;
    let target = Target::parse(&url, asset.as_deref()).map_err(usage)?;
    let primary = match Loaded::load(&config, None) {
        Ok(primary) => primary,
        Err(error) => return preflight_config_failure(&target, json, error),
    };
    let client = match client_config.as_deref() {
        None => None,
        Some(path) => match Loaded::load(path, Some(Role::Browser)) {
            Ok(client) => Some(client),
            Err(error) => return preflight_config_failure(&target, json, error),
        },
    };
    if client
        .as_ref()
        .is_some_and(|client| client.path == primary.path)
    {
        return Err(usage("--client-config must differ from --config"));
    }
    let host = match host_config.as_deref() {
        None => None,
        Some(path) => {
            if primary.config.role == Role::Node {
                return Err(usage(
                    "--config already is the host node profile; omit --host-config",
                ));
            }
            match Loaded::load(path, Some(Role::Node)) {
                Ok(host) => Some(host),
                Err(error) => return preflight_config_failure(&target, json, error),
            }
        }
    };
    let network = client
        .as_ref()
        .map_or(primary.config.network, |client| client.config.network);
    let mut extra_bootstraps = Vec::new();
    for (id, endpoint) in pins {
        let id = id
            .parse()
            .map_err(|_| usage("--bootstrap contains an invalid full node ID"))?;
        let endpoint: SocketAddrV4 = endpoint
            .parse()
            .map_err(|_| usage("--bootstrap contains an invalid IPv4:port"))?;
        let contact = Contact::new(id, endpoint, network)
            .map_err(|_| usage("--bootstrap endpoint is outside the selected network policy"))?;
        if !extra_bootstraps
            .iter()
            .any(|existing: &Contact| existing.id == contact.id)
        {
            extra_bootstraps.push(contact);
        }
    }
    let options = preflight::Options {
        primary,
        client,
        host,
        target,
        extra_bootstraps,
        route,
        timeout: Duration::from_secs(timeout),
        check_listeners,
        include_endpoints,
    };
    let report = preflight::run(&options);
    emit(&report, json)?;
    Ok(report.outcome().exit_code())
}

fn preflight_config_failure(target: &Target, json: bool, error: String) -> Result<i32, CliError> {
    let mut report = Report::preflight(serde_json::json!({
        "url": target.url.clone(),
        "destination": target.destination.to_string(),
        "document": target.document.clone(),
        "asset": target.asset.clone(),
    }));
    report.push(Check::fail(
        "configuration",
        "Configuration validity",
        error,
    ));
    emit(&report, json)?;
    Ok(1)
}

fn init_command(args: &[String]) -> Result<i32, CliError> {
    let mut profile = None;
    let mut mode_name = "loopback".to_string();
    let mut mode_set = false;
    let mut bind = None;
    let mut public = None;
    let mut relay = false;
    let mut site = false;
    let mut client = false;
    let mut home = false;
    let mut bootstrap_pin = None;
    let mut relay_pin = None;
    let mut binary_dir = None;
    let mut user = "yaminet".to_string();
    let mut group = "yaminet".to_string();
    let mut ports = Ports::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--profile" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| usage("--profile needs a directory"))?;
                set_once(&mut profile, PathBuf::from(value), "--profile")?;
            }
            "--mode" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| usage("--mode needs a value"))?;
                if mode_set {
                    return Err(usage("--mode was supplied more than once"));
                }
                if value != "loopback" && value != "public" {
                    return Err(usage("--mode must be loopback or public"));
                }
                mode_name = value.clone();
                mode_set = true;
            }
            "--bind-ip" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| usage("--bind-ip needs an IPv4 address"))?;
                let address: Ipv4Addr = value
                    .parse()
                    .map_err(|_| usage("--bind-ip must be a numeric IPv4 address"))?;
                set_once(&mut bind, address, "--bind-ip")?;
            }
            "--public-ip" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| usage("--public-ip needs an IPv4 address"))?;
                let address: Ipv4Addr = value
                    .parse()
                    .map_err(|_| usage("--public-ip must be a numeric IPv4 address"))?;
                set_once(&mut public, address, "--public-ip")?;
            }
            "--relay" => relay = true,
            "--site" => site = true,
            "--client" => client = true,
            "--home" => home = true,
            "--bootstrap-pin" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| usage("--bootstrap-pin needs ID@IPv4:port"))?;
                set_once(&mut bootstrap_pin, value.clone(), "--bootstrap-pin")?;
            }
            "--relay-pin" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| usage("--relay-pin needs ID@IPv4:port"))?;
                set_once(&mut relay_pin, value.clone(), "--relay-pin")?;
            }
            "--binary-dir" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| usage("--binary-dir needs a directory"))?;
                set_once(&mut binary_dir, PathBuf::from(value), "--binary-dir")?;
            }
            "--user" => {
                index += 1;
                user = args
                    .get(index)
                    .ok_or_else(|| usage("--user needs an account name"))?
                    .clone();
            }
            "--group" => {
                index += 1;
                group = args
                    .get(index)
                    .ok_or_else(|| usage("--group needs an account name"))?
                    .clone();
            }
            "--bootstrap-port" => {
                index += 1;
                ports.bootstrap = parse_port(args.get(index), "--bootstrap-port")?;
            }
            "--relay-port" => {
                index += 1;
                ports.relay = parse_port(args.get(index), "--relay-port")?;
            }
            "--site-port" => {
                index += 1;
                ports.site = parse_port(args.get(index), "--site-port")?;
            }
            "--help" | "-h" => {
                println!("{USAGE}");
                return Ok(0);
            }
            value if value.starts_with('-') => {
                return Err(usage(format!("unknown option '{value}'")))
            }
            value => return Err(usage(format!("unexpected argument '{value}'"))),
        }
        index += 1;
    }
    let profile = profile.ok_or_else(|| usage("init needs --profile"))?;
    let (mode, join) = if home {
        if mode_set || bind.is_some() || public.is_some() {
            return Err(usage(
                "--home does not use --mode/--bind-ip/--public-ip; it only joins an existing relay",
            ));
        }
        if relay {
            return Err(usage(
                "--home does not create a local relay; use --relay-pin for an existing relay",
            ));
        }
        if !site {
            return Err(usage("--home needs --site"));
        }
        let bootstrap_pin =
            bootstrap_pin.ok_or_else(|| usage("--home needs --bootstrap-pin ID@IPv4:port"))?;
        let relay_pin = relay_pin.ok_or_else(|| usage("--home needs --relay-pin ID@IPv4:port"))?;
        let bootstrap = parse_public_pin(&bootstrap_pin, "--bootstrap-pin")?;
        let relay = Some(parse_public_pin(&relay_pin, "--relay-pin")?);
        (Mode::Join, Some(JoinPins { bootstrap, relay }))
    } else {
        if bootstrap_pin.is_some() || relay_pin.is_some() {
            return Err(usage(
                "--bootstrap-pin/--relay-pin are only valid with --home",
            ));
        }
        let mode = match (mode_name.as_str(), bind, public) {
            ("loopback", None, None) => Mode::Loopback,
            ("loopback", _, _) => {
                return Err(usage(
                    "--bind-ip/--public-ip are only valid with --mode public",
                ))
            }
            ("public", Some(bind), Some(public)) => Mode::Public { bind, public },
            ("public", _, _) => {
                return Err(usage("--mode public needs both --bind-ip and --public-ip"))
            }
            _ => return Err(usage("--mode must be loopback or public")),
        };
        (mode, None)
    };
    let binary_dir = binary_dir.unwrap_or_else(default_binary_dir);
    let report = profile::init(&InitOptions {
        profile,
        mode,
        relay,
        site,
        client,
        binary_dir,
        user,
        group,
        ports,
        join,
    })
    .map_err(failed)?;
    println!("Initialized testnet profile: {}", report.profile.display());
    println!("Mode: {}", report.mode.as_str());
    println!(
        "Bootstrap ID: {}  endpoint: {}",
        report.bootstrap_id, report.bootstrap_endpoint
    );
    if let Some((id, endpoint)) = report.relay {
        println!("Relay ID: {id}  endpoint: {endpoint}");
    }
    if let Some(id) = report.host {
        println!("Website ID: {id}");
        if let Some(url) = &report.host_url {
            println!("Website address: {url}");
        }
    }
    if let Some(client) = &report.client_config {
        println!("Client config: {}", client.display());
    }
    println!(
        "Files created: {}; existing files kept: {}",
        report.created, report.kept
    );
    println!(
        "Nothing was started, installed, published or allowed through a firewall. Review every *.conf and units/*.service file, then follow docs/public-testnet.md."
    );
    Ok(0)
}

fn default_binary_dir() -> PathBuf {
    #[cfg(unix)]
    {
        PathBuf::from("/opt/yaminet")
    }
    #[cfg(not(unix))]
    {
        env::current_exe()
            .ok()
            .and_then(|path| path.parent().map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from("."))
    }
}

fn parse_port(value: Option<&String>, name: &str) -> Result<u16, CliError> {
    let value = value.ok_or_else(|| usage(format!("{name} needs a port")))?;
    let port: u16 = value
        .parse()
        .map_err(|_| usage(format!("{name} must be 1..65535")))?;
    if port == 0 {
        return Err(usage(format!("{name} must be 1..65535")));
    }
    Ok(port)
}

fn parse_public_pin(value: &str, name: &str) -> Result<(NodeId, SocketAddrV4), CliError> {
    let (id, endpoint) = value
        .split_once('@')
        .ok_or_else(|| usage(format!("{name} needs ID@IPv4:port")))?;
    let id: NodeId = id
        .parse()
        .map_err(|_| usage(format!("{name} contains an invalid full node ID")))?;
    let endpoint: SocketAddrV4 = endpoint
        .parse()
        .map_err(|_| usage(format!("{name} contains an invalid IPv4:port")))?;
    Contact::new(id, endpoint, Network::Public)
        .map_err(|_| usage(format!("{name} needs a public unicast IPv4 endpoint")))?;
    Ok((id, endpoint))
}

fn render_command(args: &[String]) -> Result<i32, CliError> {
    let mut role = None;
    let mut config = None;
    let mut binary = None;
    let mut user = "yaminet".to_string();
    let mut group = "yaminet".to_string();
    let mut output = None;
    let mut force = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--role" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| usage("--role needs a value"))?;
                set_once(&mut role, value.to_string(), "--role")?;
            }
            "--config" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| usage("--config needs a file"))?;
                set_once(&mut config, PathBuf::from(value), "--config")?;
            }
            "--binary" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| usage("--binary needs a path"))?;
                set_once(&mut binary, PathBuf::from(value), "--binary")?;
            }
            "--user" => {
                index += 1;
                user = args
                    .get(index)
                    .ok_or_else(|| usage("--user needs an account name"))?
                    .clone();
            }
            "--group" => {
                index += 1;
                group = args
                    .get(index)
                    .ok_or_else(|| usage("--group needs an account name"))?
                    .clone();
            }
            "--output" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| usage("--output needs a file"))?;
                set_once(&mut output, PathBuf::from(value), "--output")?;
            }
            "--force" => force = true,
            "--help" | "-h" => {
                println!("{USAGE}");
                return Ok(0);
            }
            value if value.starts_with('-') => {
                return Err(usage(format!("unknown option '{value}'")))
            }
            value => return Err(usage(format!("unexpected argument '{value}'"))),
        }
        index += 1;
    }
    let role =
        UnitRole::parse(&role.ok_or_else(|| usage("render-unit needs --role"))?).map_err(usage)?;
    let config = config.ok_or_else(|| usage("render-unit needs --config"))?;
    let mut spec = UnitSpec::new(role, config);
    if let Some(binary) = binary {
        spec.binary = binary;
    }
    spec.user = user;
    spec.group = group;
    let unit = render::render(&spec).map_err(failed)?;
    if let Some(output) = output {
        let changed =
            render::write_unit(&output, &unit, force).map_err(|error| failed(error.to_string()))?;
        println!(
            "{} {}",
            if changed { "Wrote" } else { "Unchanged" },
            output.display()
        );
    } else {
        print!("{unit}");
    }
    Ok(0)
}

fn remove_command(args: &[String]) -> Result<i32, CliError> {
    let mut profile = None;
    let mut confirm = false;
    let mut purge = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--profile" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| usage("--profile needs a directory"))?;
                set_once(&mut profile, PathBuf::from(value), "--profile")?;
            }
            "--confirm" => confirm = true,
            "--purge-data" => purge = true,
            "--help" | "-h" => {
                println!("{USAGE}");
                return Ok(0);
            }
            value if value.starts_with('-') => {
                return Err(usage(format!("unknown option '{value}'")))
            }
            value => return Err(usage(format!("unexpected argument '{value}'"))),
        }
        index += 1;
    }
    let profile = profile.ok_or_else(|| usage("remove needs --profile"))?;
    let report = profile::remove(&profile, confirm, purge).map_err(failed)?;
    println!("Removed {} generated entr(ies).", report.removed);
    if report.root_removed {
        println!("Profile directory removed.");
    } else {
        println!("Profile directory retained because it contains files not created by this kit.");
        for path in report.leftover {
            println!("  {}", path.display());
        }
    }
    println!(
        "Stop and disable installed units first with deploy/testnet/yaminet-testnet uninstall."
    );
    Ok(0)
}
