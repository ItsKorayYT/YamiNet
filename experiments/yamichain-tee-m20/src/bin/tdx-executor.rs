//! TDX-only disposable-funds HTTP adapter. No software fallback.
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    time::Duration,
};
use yamichain_tee_m20::{tdx_guest::*, *};
fn handle(
    guest: &mut Guest,
    generator: &QuoteGenerator,
    stream: &mut TcpStream,
) -> Result<serde_json::Value> {
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    stream.set_write_timeout(Some(Duration::from_secs(30)))?;
    let mut header = Vec::new();
    let mut byte = [0];
    while !header.ends_with(b"\r\n\r\n") {
        if header.len() >= 8192 {
            return Err("oversized HTTP header".into());
        }
        stream.read_exact(&mut byte)?;
        header.push(byte[0]);
    }
    let text = std::str::from_utf8(&header)?;
    let mut lines = text.split("\r\n");
    let parts: Vec<_> = lines
        .next()
        .ok_or("HTTP request")?
        .split_whitespace()
        .collect();
    if parts.len() != 3 || parts[2] != "HTTP/1.1" {
        return Err("HTTP/1.1 required".into());
    }
    let sizes: Vec<_> = lines
        .clone()
        .filter_map(|l| l.split_once(':'))
        .filter(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        .collect();
    if sizes.len() > 1
        || lines.any(|l| {
            l.split_once(':')
                .is_some_and(|(k, _)| k.eq_ignore_ascii_case("transfer-encoding"))
        })
    {
        return Err("ambiguous HTTP frame".into());
    }
    let size = if sizes.is_empty() {
        0
    } else {
        sizes[0].1.trim().parse::<usize>()?
    };
    if size > 32 * 1024 * 1024 {
        return Err("HTTP body limit".into());
    }
    let mut raw = vec![0; size];
    stream.read_exact(&mut raw)?;
    match (parts[0], parts[1]) {
        ("GET", "/backup") => Ok(serde_json::to_value(guest.backup()?)?),
        ("POST", "/activate") => {
            guest.activate(serde_json::from_slice(&raw)?)?;
            Ok(serde_json::json!({"activated":true,"production":false}))
        }
        ("GET", "/identity") => Ok(serde_json::to_value(guest.identity())?),
        ("GET", "/") | ("GET", "/status") => Ok(serde_json::to_value(
            guest.command(serde_json::from_str("\"Head\"")?)?,
        )?),
        ("POST", "/attest") => {
            #[derive(serde::Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Request {
                challenge: Id,
                enrollment: bool,
            }
            let req: Request = serde_json::from_slice(&raw)?;
            let quote = generator.quote(&guest.report_data(req.challenge, req.enrollment)?)?;
            // This is raw evidence, NOT a verification decision. No caller-selected REPORTDATA.
            Ok(serde_json::json!({"quote":quote}))
        }
        ("POST", "/configure") => Ok(
            serde_json::json!({"backup":guest.configure(serde_json::from_slice(&raw)?,std::path::Path::new("/run/yami/private.state"))?}),
        ),
        ("POST", "/") => Ok(serde_json::to_value(
            guest.command(serde_json::from_slice(&raw)?)?,
        )?),
        _ => Err("unsupported guest endpoint".into()),
    }
}
fn main() -> Result<()> {
    let path = std::env::args()
        .nth(1)
        .ok_or("tdx-executor IMMUTABLE_GUEST_CONFIG")?;
    let config: GuestConfig = serde_json::from_slice(&std::fs::read(path)?)?;
    let generator = QuoteGenerator::load(&config)?;
    let listener = TcpListener::bind(config.listen)?;
    let mut guest = Guest::new(config)?;
    eprintln!("TDX DEVNET ONLY: local quote generation is not wallet verification; real-money activation CLOSED");
    for stream in listener.incoming() {
        let mut stream = stream?;
        let response = handle(&mut guest, &generator, &mut stream);
        let (code, body) = match response {
            Ok(v) => (200, serde_json::to_vec(&v)?),
            Err(e) => (
                503,
                serde_json::to_vec(&serde_json::json!({"error":e.to_string()}))?,
            ),
        };
        let header=format!("HTTP/1.1 {code} Response\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",body.len());
        let _ = stream
            .write_all(header.as_bytes())
            .and_then(|_| stream.write_all(&body));
    }
    Ok(())
}
