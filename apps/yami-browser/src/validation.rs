//! Explicit document/PNG validation using the browser's authenticated transport.
//! Reports no page contents, keys or private filesystem paths.
use crate::{
    model::{self, Location},
    native::Resolver,
    resources::Origin,
};
use std::time::{Duration, Instant};

pub struct Observation {
    pub destination: yami_identity::NodeId,
    pub origin: Origin,
    pub png: Option<(u32, u32)>,
    pub elapsed_ms: u128,
}
pub fn inspect(url: &str, asset: Option<&str>, resolver: &Resolver) -> Result<Observation, String> {
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
    let document = crate::native::parse_document(&data.source)
        .map_err(|e| format!("Authenticated document syntax failed: {e}"))?;
    let style = if data.style.is_empty() {
        ysl::Stylesheet::default()
    } else {
        ysl::parse(&data.style)
            .map_err(|e| format!("Authenticated stylesheet syntax failed: {e}"))?
    };
    let png = if let Some(key) = asset {
        use yami_render::assets::{ImageState, Images};
        let security =
            origin
                .relay
                .map_or(model::Security::Verified(origin.destination), |relay| {
                    model::Security::Relayed {
                        destination: origin.destination,
                        relay,
                    }
                });
        let mut images = Images::new();
        images.insert(key.into(), ImageState::Pending);
        let page = model::Page {
            document,
            style,
            security,
            endpoint: Some(origin.endpoint),
            images,
            revision: 0,
            origin: Some(origin.clone()),
        };
        let cache = crate::image_cache::DecodeCache::default();
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let mut result = None;
        crate::resources::load(
            &page,
            &cache,
            &cancel,
            Instant::now() + Duration::from_secs(5),
            |_, state| {
                result = Some(state);
                true
            },
        );
        let image = match result {
            Some(ImageState::Ready(image)) => image,
            Some(ImageState::Missing(error)) => {
                return Err(format!("PNG validation failed: {error}"))
            }
            _ => return Err("PNG validation did not complete".into()),
        };
        Some((image.width, image.height))
    } else {
        None
    };
    Ok(Observation {
        destination: origin.destination,
        origin,
        png,
        elapsed_ms: started.elapsed().as_millis(),
    })
}
pub fn probe(url: &str, asset: Option<&str>, resolver: &Resolver) -> Result<String, String> {
    let o = inspect(url, asset, resolver)?;
    let mut report = format!("YAMINET-VALIDATION 1\nversion {}\nscope {}\ndestination {}\nrelay {}\nendpoint {}\ndocument PASS (pinned TLS 1.3; YLang/YSL parsed)\n", env!("CARGO_PKG_VERSION"), if o.origin.endpoint.ip().is_loopback() { "loopback simulation" } else { "explicit remote endpoint; physical network independence NOT VERIFIED" }, o.destination, o.origin.relay.map_or_else(|| "direct".into(), |id| id.to_string()), o.origin.endpoint);
    if let Some((w, h)) = o.png {
        report.push_str(&format!(
            "png PASS ({}; {w} x {h})\n",
            asset.unwrap_or_default()
        ));
    } else {
        report.push_str("png NOT RUN (pass --asset KEY)\n");
    }
    report.push_str(&format!("elapsed_ms {}\nrendering NOT RUN (native desktop checklist required)\nindependent_networks NOT VERIFIED\n", o.elapsed_ms));
    Ok(report)
}
