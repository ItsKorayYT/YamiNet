mod chrome;
mod desktop;
mod discovery;
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
    if args.first().is_some_and(|s| s == "--help") {
        println!("Yami Browser\n  yami-browser                         Native desktop window\n  yami-browser --desktop [YNP URL]     Desktop with initial page\n  yami-browser --terminal [file]       Terminal viewer\n  yami-browser [file | YNP URL] [--style file] [--ppm image.ppm]\nConfiguration: --config FILE | check-config FILE | --network-status\nSettings: --enable-relays | --disable-relays; --bootstrap node-ID@IPv4:port\nSidebar: relay toggle and Add bootstrap peer (paste full ID@IPv4:port, Enter).\nYNP URLs: yami://node-id/key (DHT lookup, pinned TLS 1.3)\n          ynps://node-id@IP:port/key (TLS 1.3, loopback, LAN or public IPv4)\n          ynp://loopback-IP:port/key (plaintext, this computer only)\n{}\n{}", ynp::WARNING, ynp_tls::WARNING);
        Ok(())
    } else if args.is_empty() || args[0] == "--desktop" {
        if !args.is_empty() {
            args.remove(0);
        }
        desktop::run(args, relay_enabled, resolver)
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
