//! Independently deployable authenticated authority. Use a separate administrative host/storage.
use yamichain_tee_m20::{witness::*, Result};
fn main() -> Result<()> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: witness-node PRIVATE_CONFIG")?;
    let config: ServiceConfig = serde_json::from_slice(&std::fs::read(path)?)?;
    Service::open(config)?.serve()
}
