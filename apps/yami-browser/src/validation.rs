//! Explicit document/PNG validation using the browser's authenticated transport.
//! Reports no page contents, keys or private filesystem paths.
use crate::{
    model::{self, Location},
    native::Resolver,
    resources::Origin,
};
use std::time::{Duration, Instant};

pub fn probe(url: &str, asset: Option<&str>, resolver: &Resolver) -> Result<String, String> {
    let location = Location::parse(url)?;
    if let Some(key) = asset {
        ynp::validate_asset_key(key).map_err(|e| e.to_string())?;
    }
    let started = Instant::now();
    let (data, origin) = match location {
        Location::Yami(address) => {
            let (data, info) = resolver.fetch_verified(&address)?;
            (data, Origin { destination: address.id, endpoint: info.endpoint, relay: info.relay, public_relay: resolver.public_network() })
        }
        Location::Ynps(ref address) => {
            let (data, id) = model::fetch_verified(address)?;
            let (_, endpoint, _) = ynp_tls::parse_url(address).map_err(|e| e.to_string())?;
            (data, Origin { destination: id, endpoint, relay: None, public_relay: false })
        }
        _ => return Err("Probe requires yami:// or ynps:// with an exact node identity; plaintext and built-in pages are not cross-network checks".into()),
    };
    ylang::parse(&data.source).map_err(|e| format!("Authenticated document syntax failed: {e}"))?;
    if !data.style.is_empty() {
        ysl::parse(&data.style)
            .map_err(|e| format!("Authenticated stylesheet syntax failed: {e}"))?;
    }
    let mut report = format!("YAMINET-VALIDATION 1\nversion {}\nscope {}\ndestination {}\nrelay {}\nendpoint {}\ndocument PASS (pinned TLS 1.3; YLang/YSL parsed)\n", env!("CARGO_PKG_VERSION"), if origin.endpoint.ip().is_loopback() { "loopback simulation" } else { "explicit remote endpoint; physical network independence NOT VERIFIED" }, origin.destination, origin.relay.map_or_else(|| "direct".into(), |id| id.to_string()), origin.endpoint);
    if let Some(key) = asset {
        let mut connection = origin
            .connect(Duration::from_secs(5))
            .map_err(|e| format!("PNG authenticated connection failed: {e}"))?;
        let bytes = ynp::request_asset(&mut *connection, key, ynp::MAX_ASSET_BYTES)
            .map_err(|e| format!("PNG fetch failed: {e}"))?;
        let image = yami_render::assets::decode_png(&bytes.bytes, yami_render::assets::MAX_DECODED)
            .map_err(|e| format!("PNG validation failed: {e}"))?;
        report.push_str(&format!(
            "png PASS ({key}; {} x {})\n",
            image.width, image.height
        ));
    } else {
        report.push_str("png NOT RUN (pass --asset KEY)\n");
    }
    report.push_str(&format!("elapsed_ms {}\nrendering NOT RUN (native desktop checklist required)\nindependent_networks NOT VERIFIED\n", started.elapsed().as_millis()));
    Ok(report)
}
