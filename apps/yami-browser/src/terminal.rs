use std::{
    error::Error,
    fs::File,
    io::{self, Read},
};
use ynp::LoopbackTcp;

const USAGE: &str = "yami-browser [document.ylang | yami://node-id/key | ynp://loopback-IP:port/key | ynps://node-id@IP:port/key] [--style theme.ysl] [--ppm output.ppm]\nWithout a source, opens the built-in sample. Links are displayed, never automatically fetched.\nEXPERIMENTAL: ynp:// is plaintext and unauthenticated; yami:// and ynps:// use TLS 1.3 to the node ID in the address. Neither is reviewed or anonymous.";
fn read_file(path: &str, limit: usize) -> io::Result<String> {
    let mut text = String::new();
    File::open(path)?
        .take((limit + 1) as u64)
        .read_to_string(&mut text)?;
    if text.len() > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "file exceeds parser limit",
        ));
    }
    Ok(text)
}
pub fn run(
    arguments: Vec<String>,
    relay_enabled: Option<bool>,
    resolver: Option<std::sync::Arc<yami_browser::native::Resolver>>,
) -> Result<(), Box<dyn Error>> {
    let mut args = arguments.into_iter();
    let mut source = None;
    let mut style_path = None;
    let mut ppm_path = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" => {
                println!("{USAGE}");
                return Ok(());
            }
            "--style" if style_path.is_none() => {
                style_path = Some(args.next().ok_or("--style requires a path")?)
            }
            "--ppm" if ppm_path.is_none() => {
                ppm_path = Some(args.next().ok_or("--ppm requires a path")?)
            }
            _ if !arg.starts_with('-') && source.is_none() => source = Some(arg),
            _ => return Err(USAGE.into()),
        }
    }
    let verified = source.as_deref().is_some_and(|url| {
        url.starts_with(ynp_tls::SCHEME) || url.starts_with(yami_browser::native::SCHEME)
    });
    eprintln!(
        "{}",
        if verified {
            ynp_tls::WARNING
        } else {
            ynp::WARNING
        }
    );
    let (text, remote_style) = match source.as_deref() {
        Some(url) if url.starts_with(yami_browser::native::SCHEME) => {
            let address = yami_browser::native::Address::parse(url)?;
            let resolver = match resolver {
                Some(resolver) => resolver,
                None => std::sync::Arc::new(yami_browser::native::Resolver::from_environment()?),
            };
            if let Some(enabled) = relay_enabled {
                resolver.set_relay_enabled(enabled);
            }
            let (data, info) = resolver.fetch_verified(&address)?;
            if let Some(relay) = info.relay {
                eprintln!(
                    "Verified node · TLS 1.3 · Relayed; destination {}; relay {relay}",
                    address.id
                );
            }
            eprintln!(
                "Verified node {} over TLS 1.3 after distributed lookup",
                address.id
            );
            (data.source, Some(data.style))
        }
        Some(url) if url.starts_with("ynp://") => {
            let (address, key) = ynp::parse_url(url)?;
            let data = ynp::fetch(&LoopbackTcp, address, &key)?;
            (data.source, Some(data.style))
        }
        Some(url) if verified => {
            let (data, id) = crate::model::fetch_verified(url)?;
            eprintln!("Verified node {id} over TLS 1.3");
            (data.source, Some(data.style))
        }
        Some(path) if path.contains("://") => return Err("only YNP URLs are supported".into()),
        Some(path) => (read_file(path, ylang::MAX_SOURCE)?, None),
        None => (include_str!("../../../examples/welcome.ylang").into(), None),
    };
    let style = match style_path {
        Some(path) => ysl::parse(&read_file(&path, ysl::MAX_SOURCE)?)?,
        None => match remote_style {
            Some(text) if !text.is_empty() => ysl::parse(&text)?,
            _ => ysl::Stylesheet::default(),
        },
    };
    let document = yami_browser::native::parse_document(&text)?;
    let layout = yami_render::layout(&document, &style)?;
    println!("YamiNet — {}\n", document.metadata.title);
    print!("{}", layout.terminal());
    if let Some(path) = ppm_path {
        layout.write_ppm(File::create(&path)?)?;
        eprintln!("Wrote software-rendered bitmap to {path}");
    }
    Ok(())
}
