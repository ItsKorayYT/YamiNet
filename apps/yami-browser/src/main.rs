mod chrome;
mod desktop;
mod discovery;
mod game_view;
mod terminal;
use yami_browser::model;

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() == 2 && args[0] == "check-config" {
        let config = yami_config::Config::load(&args[1])?;
        config.expect_role(yami_config::Role::Browser)?;
        println!("Valid browser configuration; network {:?}; relay fallback {}; {} bootstrap(s); state {}",config.network,config.relay_fallback,config.bootstrap.len(),config.state.display());
        return Ok(());
    }
    if args == ["--version"] {
        println!("Yami Browser {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if let [command, action, dir] = args.as_slice() {
        if command == "diagnostics" {
            let log = yami_browser::diagnostics::Log::open(std::path::Path::new(dir))?;
            match action.as_str() {
                "report" => println!("{}", log.report()?),
                "clear" => {
                    log.clear()?;
                    println!("Local diagnostic logs cleared");
                }
                _ => return Err("Use diagnostics report|clear ABSOLUTE-DIRECTORY".into()),
            }
            return Ok(());
        }
    }
    if args.first().is_some_and(|arg| arg == "--network-trial") {
        if args.len() < 6 || args[2] != "--config" || args[4] != "--asset" {
            return Err("Usage: --network-trial yami://ID/KEY --config BROWSER-CONFIG --asset PNG-KEY [--host-config FILE] [--name NAME] [--navigate PAGE] [--game GAME-KEY] [--include-endpoints] [--operator-relay-cycle]".into());
        }
        let (mut host_path, mut endpoints, mut cycle) = (None, false, false);
        let mut checks = yami_browser::network_trial::Checks::default();
        let mut rest = args[6..].iter();
        while let Some(flag) = rest.next() {
            match flag.as_str() {
                "--host-config" if host_path.is_none() => {
                    host_path = Some(rest.next().ok_or("host config file required")?)
                }
                "--name" if checks.name.is_none() => {
                    checks.name = Some(rest.next().ok_or("canonical name required")?)
                }
                "--navigate" if checks.navigation.is_none() => {
                    checks.navigation = Some(rest.next().ok_or("navigation page key required")?)
                }
                "--game" if checks.game.is_none() => {
                    checks.game = Some(rest.next().ok_or("explicit hosted game key required")?)
                }
                "--include-endpoints" if !endpoints => endpoints = true,
                "--operator-relay-cycle" if !cycle => cycle = true,
                _ => return Err("unknown or duplicate network trial option".into()),
            }
        }
        let loaded = yami_config::Config::load(&args[3]).and_then(|config| {
            host_path
                .map(yami_config::Config::load)
                .transpose()
                .map(|host| (config, host))
        });
        let (config, host) = match loaded {
            Ok(value) => value,
            Err(_) => {
                println!(
                    "{}",
                    serde_json::json!({"schema":"yaminet-network-trial-1","result":"FAIL","steps":[{"name":"configuration","status":"FAIL","evidence":"supplied configuration is missing or invalid; paths and raw errors omitted"}],"physical_topology":"NOT VERIFIED"})
                );
                return Err("network trial configuration failed".into());
            }
        };
        let (report, success) = yami_browser::network_trial::run_with_checks(
            &config,
            &args[1],
            &args[5],
            host.as_ref(),
            endpoints,
            cycle,
            checks,
        );
        println!("{}", serde_json::to_string_pretty(&report)?);
        if !success {
            return Err("network trial failed; see structured report".into());
        }
        return Ok(());
    }
    let mut game_path = None;
    if let Some(index) = args.iter().position(|arg| arg == "--game") {
        args.remove(index);
        if index >= args.len() {
            return Err("--game requires a local manifest path".into());
        }
        game_path = Some(std::path::PathBuf::from(args.remove(index)));
    }
    let mut diagnostic_dir = None;
    let mut smoke_output = None;
    for flag in ["--diagnostic-log", "--desktop-smoke"] {
        if let Some(index) = args.iter().position(|arg| arg == flag) {
            args.remove(index);
            if index >= args.len() {
                return Err("diagnostic/smoke flag requires an absolute path".into());
            }
            let path = std::path::PathBuf::from(args.remove(index));
            if !path.is_absolute() {
                return Err("diagnostic/smoke path must be absolute".into());
            }
            if flag == "--diagnostic-log" {
                diagnostic_dir = Some(path);
            } else {
                smoke_output = Some(path);
            }
        }
    }
    if smoke_output.is_some()
        && (std::env::var_os("YAMINET_BROWSER_DATA_DIR").is_none()
            || !args.is_empty()
            || game_path.is_some())
    {
        return Err("Desktop smoke requires an explicit isolated YAMINET_BROWSER_DATA_DIR and accepts no navigation/config arguments".into());
    }
    let mut resolver = None;
    if let Some(index) = args.iter().position(|arg| arg == "--config") {
        args.remove(index);
        if index >= args.len() {
            return Err("--config requires a file".into());
        }
        let config = yami_config::Config::load(args.remove(index))?;
        resolver = Some(std::sync::Arc::new(
            yami_browser::native::Resolver::from_config(&config)?,
        ));
    }
    if args.first().is_some_and(|s| s == "yns") {
        let resolver = match resolver {
            Some(resolver) => resolver,
            None => std::sync::Arc::new(yami_browser::native::Resolver::from_environment()?),
        };
        println!("{}", yami_browser::naming::command(&args[1..], &resolver)?);
        return Ok(());
    }
    if args == ["--network-status"] {
        let resolver = match resolver {
            Some(resolver) => resolver,
            None => std::sync::Arc::new(yami_browser::native::Resolver::from_environment()?),
        };
        println!("{}", resolver.diagnostics()?);
        return Ok(());
    }
    let mut relay_enabled = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--enable-relays" | "--disable-relays" => {
                relay_enabled = Some(args.remove(index) == "--enable-relays");
            }
            "--bootstrap" => {
                args.remove(index);
                if index >= args.len() {
                    return Err("--bootstrap requires node-ID@IPv4:port".into());
                }
                let pin = args.remove(index);
                if let Some(resolver) = &resolver {
                    resolver.add_bootstrap(&pin)?;
                } else {
                    yami_browser::native::add_bootstrap_from_environment(&pin)?;
                }
            }
            _ => index += 1,
        }
    }
    if args.first().is_some_and(|s| s == "--probe") {
        let (url, asset) =
            match args.as_slice() {
                [_, url] => (url.as_str(), None),
                [_, url, flag, key] if flag == "--asset" => (url.as_str(), Some(key.as_str())),
                _ => return Err(
                    "Usage: yami-browser [--config FILE] --probe YAMI-OR-YNPS-URL [--asset KEY]"
                        .into(),
                ),
            };
        let resolver = match resolver {
            Some(resolver) => resolver,
            None => std::sync::Arc::new(yami_browser::native::Resolver::from_environment()?),
        };
        match yami_browser::validation::probe(url, asset, &resolver) {
            Ok(report) => println!("{report}"),
            Err(error) => {
                println!("YAMINET-VALIDATION 1\nresult FAIL\nreason {}\nindependent_networks NOT VERIFIED", error.escape_debug());
                return Err(error.into());
            }
        }
        return Ok(());
    }
    if args.first().is_some_and(|s| s == "--help") {
        println!("Yami Browser\n  --network-trial URL --config FILE --asset KEY [--host-config FILE] [--name NAME] [--navigate PAGE] [--game GAME-KEY] [--include-endpoints] [--operator-relay-cycle]\n  --diagnostic-log ABS-DIR | diagnostics report|clear ABS-DIR\n  --desktop-smoke ABS-REPORT (isolated test profile required)\n  --game LOCAL-MANIFEST                 Play a local WebAssembly game\n  --version | --probe URL [--asset KEY]  Explicit authenticated diagnostics\n  yami-browser                         Native desktop window\n  yami-browser --desktop [YNP URL]     Desktop with initial page\n  yami-browser --terminal [file]       Terminal viewer\n  yami-browser [file | YNP URL] [--style file] [--ppm image.ppm]\nConfiguration: --config FILE | check-config FILE | --network-status\nSettings: --enable-relays | --disable-relays; --bootstrap node-ID@IPv4:port\nSidebar: relay toggle and Add bootstrap peer (paste full ID@IPv4:port, Enter).\nNaming: yns (see docs/yns.md); yami://@alias/key; yami://label.full-owner-ID.yns/key\nYNP URLs: yami://node-id/key (DHT lookup, pinned TLS 1.3)\n          ynps://node-id@IP:port/key (TLS 1.3, loopback, LAN or public IPv4)\n          ynp://loopback-IP:port/key (plaintext, this computer only)\n{}\n{}", ynp::WARNING, ynp_tls::WARNING);
        Ok(())
    } else if game_path.is_some() || args.is_empty() || args[0] == "--desktop" {
        if args.first().is_some_and(|arg| arg == "--desktop") {
            args.remove(0);
        }
        desktop::run(
            args,
            relay_enabled,
            resolver,
            diagnostic_dir,
            smoke_output,
            game_path,
        )
    } else {
        if args[0] == "--terminal" {
            args.remove(0);
        }
        terminal::run(args, relay_enabled, resolver)
    }
}
fn main() {
    if let Err(error) = run() {
        eprintln!("yami-browser: {error}");
        std::process::exit(1);
    }
}
