use super::Failure;
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};
use yami_dht::Network;
use yami_identity::NodeId;

#[derive(Clone, Debug)]
pub struct Status {
    pub id: NodeId,
    pub network: Network,
    pub observed: u64,
    pub running: bool,
    pub known_peers: usize,
    pub recently_verified: usize,
    pub last_publication: Option<u64>,
    pub expires: u64,
    pub next_attempt: u64,
    pub last_error: Option<Failure>,
}
fn mode(network: Network) -> &'static str {
    if network == Network::Public {
        "public"
    } else {
        "local"
    }
}
pub fn status_path(directory: &Path, network: Network) -> PathBuf {
    directory.join(format!("dht-{}-maintenance.status", mode(network)))
}
pub(super) fn write_status(path: &Path, status: &Status) -> io::Result<()> {
    let error = match status.last_error {
        None => 0,
        Some(Failure::NoPeers) => 1,
        Some(Failure::Unreachable) => 2,
        Some(Failure::State) => 3,
        Some(Failure::Configuration) => 4,
        Some(Failure::Clock) => 5,
    };
    let text = format!(
        "YDM1 {} {} {} {} {} {} {} {} {} {}\n",
        mode(status.network),
        status.id,
        status.observed,
        u8::from(status.running),
        status.known_peers,
        status.recently_verified,
        status.last_publication.unwrap_or(0),
        status.expires,
        status.next_attempt,
        error
    );
    let temp = path.with_extension("status.tmp");
    // Only the holder of the DHT server lifetime lock writes this sidecar.
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options.open(&temp)?;
    file.write_all(text.as_bytes())?;
    file.sync_all()?;
    drop(file);
    fs::rename(temp, path)
}
pub fn read_status(path: &Path, network: Network) -> io::Result<Status> {
    let mut text = String::new();
    File::open(path)?.take(513).read_to_string(&mut text)?;
    let invalid = || io::Error::new(io::ErrorKind::InvalidData, "invalid DHT maintenance status");
    let fields: Vec<_> = text.split_whitespace().collect();
    if text.len() > 512 || fields.len() != 11 || fields[0] != "YDM1" || fields[1] != mode(network) {
        return Err(invalid());
    }
    let number = |index: usize| fields[index].parse::<u64>().map_err(|_| invalid());
    let running = match number(4)? {
        0 => false,
        1 => true,
        _ => return Err(invalid()),
    };
    let known_peers = usize::try_from(number(5)?).map_err(|_| invalid())?;
    let recently_verified = usize::try_from(number(6)?).map_err(|_| invalid())?;
    if known_peers > yami_dht::MAX_CACHE + yami_dht::MAX_ROUTES
        || recently_verified > yami_dht::MAX_ROUTES
    {
        return Err(invalid());
    }
    let last_error = match number(10)? {
        0 => None,
        1 => Some(Failure::NoPeers),
        2 => Some(Failure::Unreachable),
        3 => Some(Failure::State),
        4 => Some(Failure::Configuration),
        5 => Some(Failure::Clock),
        _ => return Err(invalid()),
    };
    Ok(Status {
        id: fields[2].parse().map_err(|_| invalid())?,
        network,
        observed: number(3)?,
        running,
        known_peers,
        recently_verified,
        last_publication: match number(7)? {
            0 => None,
            at => Some(at),
        },
        expires: number(8)?,
        next_attempt: number(9)?,
        last_error,
    })
}
