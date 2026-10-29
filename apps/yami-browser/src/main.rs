mod chrome;
mod desktop;
mod discovery;
mod model;
mod terminal;

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let result = if args.first().is_some_and(|s| s == "--help") {
        println!("Yami Browser\n  yami-browser                         Native desktop window\n  yami-browser --desktop [YNP URL]     Desktop with initial page\n  yami-browser --terminal [file]       Terminal viewer\n  yami-browser [file | YNP URL] [--style file] [--ppm image.ppm]\nYNP URLs: ynps://node-id@IP:port/key (TLS 1.3, loopback or private LAN)\n          ynp://loopback-IP:port/key (plaintext, this computer only)\n{}\n{}", ynp::WARNING, ynp_tls::WARNING);
        Ok(())
    } else if args.is_empty() || args[0] == "--desktop" {
        if !args.is_empty() {
            args.remove(0);
        }
        desktop::run(args)
    } else {
        if args[0] == "--terminal" {
            args.remove(0);
        }
        terminal::run(args)
    };
    if let Err(error) = result {
        eprintln!("yami-browser: {error}");
        std::process::exit(1);
    }
}
