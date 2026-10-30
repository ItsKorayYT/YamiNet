mod chrome;
mod desktop;
mod discovery;
mod terminal;
use yami_browser::model;

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
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
                yami_browser::native::add_bootstrap_from_environment(&args.remove(index))?;
            }
            _ => index += 1,
        }
    }
    if args.first().is_some_and(|s| s == "--help") {
        println!("Yami Browser\n  yami-browser                         Native desktop window\n  yami-browser --desktop [YNP URL]     Desktop with initial page\n  yami-browser --terminal [file]       Terminal viewer\n  yami-browser [file | YNP URL] [--style file] [--ppm image.ppm]\nSettings: --enable-relays | --disable-relays; --bootstrap node-ID@IPv4:port\nSidebar: relay toggle and Add bootstrap peer (paste full ID@IPv4:port, Enter).\nYNP URLs: yami://node-id/key (DHT lookup, pinned TLS 1.3)\n          ynps://node-id@IP:port/key (TLS 1.3, loopback, LAN or public IPv4)\n          ynp://loopback-IP:port/key (plaintext, this computer only)\n{}\n{}", ynp::WARNING, ynp_tls::WARNING);
        Ok(())
    } else if args.is_empty() || args[0] == "--desktop" {
        if !args.is_empty() {
            args.remove(0);
        }
        desktop::run(args, relay_enabled)
    } else {
        if args[0] == "--terminal" {
            args.remove(0);
        }
        terminal::run(args, relay_enabled)
    }
}
fn main() {
    if let Err(error) = run() {
        eprintln!("yami-browser: {error}");
        std::process::exit(1);
    }
}
