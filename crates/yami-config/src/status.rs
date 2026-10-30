//! Bounded local diagnostic snapshot, never a remote authentication proof.
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};
use yami_identity::NodeId;
#[derive(Debug)]
pub struct RelayStatus {
    pub id: NodeId,
    pub observed: u64,
    pub running: bool,
    pub connections: usize,
    pub registrations: usize,
    pub tunnels: usize,
    pub forwarded: u64,
}
impl RelayStatus {
    pub fn read(directory: &Path) -> io::Result<Self> {
        let mut text = String::new();
        File::open(directory.join("relay.status"))?
            .take(513)
            .read_to_string(&mut text)?;
        let values: Vec<_> = text.split_whitespace().collect();
        let invalid =
            || io::Error::new(io::ErrorKind::InvalidData, "invalid relay status snapshot");
        if text.len() > 512 || values.len() != 8 || values[0] != "YRS1" {
            return Err(invalid());
        }
        let number = |i: usize| values[i].parse::<u64>().map_err(|_| invalid());
        let status = Self {
            id: values[1].parse().map_err(|_| invalid())?,
            observed: number(2)?,
            running: match number(3)? {
                0 => false,
                1 => true,
                _ => return Err(invalid()),
            },
            connections: usize::try_from(number(4)?).map_err(|_| invalid())?,
            registrations: usize::try_from(number(5)?).map_err(|_| invalid())?,
            tunnels: usize::try_from(number(6)?).map_err(|_| invalid())?,
            forwarded: number(7)?,
        };
        if status.connections > 32 || status.registrations > 8 || status.tunnels > 4 {
            return Err(invalid());
        }
        Ok(status)
    }
}
pub struct StatusWriter {
    directory: PathBuf,
    _lock: File,
}
impl StatusWriter {
    pub fn open(directory: &Path) -> io::Result<Self> {
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        builder.create(directory)?;
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let lock = options.open(directory.join("relay-status.lock"))?;
        lock.try_lock()
            .map_err(|_| io::Error::other("relay status directory is locked by another process"))?;
        match fs::remove_file(directory.join("relay.status.tmp")) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        Ok(Self {
            directory: directory.into(),
            _lock: lock,
        })
    }
    pub fn write(&self, status: &RelayStatus) -> io::Result<()> {
        let temp = self.directory.join("relay.status.tmp");
        let path = self.directory.join("relay.status");
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let text = format!(
            "YRS1 {} {} {} {} {} {} {}\n",
            status.id,
            status.observed,
            u8::from(status.running),
            status.connections,
            status.registrations,
            status.tunnels,
            status.forwarded
        );
        let result = (|| {
            let mut file = options.open(&temp)?;
            file.write_all(text.as_bytes())?;
            file.sync_all()?;
            drop(file);
            fs::rename(&temp, path)
        })();
        if result.is_err() {
            let _ = fs::remove_file(temp);
        }
        result
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn relay_status_is_bounded_locked_and_marks_stopped() {
        let dir = std::env::temp_dir().join(format!("yami-status-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let writer = StatusWriter::open(&dir).unwrap();
        assert!(StatusWriter::open(&dir).is_err());
        let status = RelayStatus {
            id: *yami_identity::NodeIdentity::generate().unwrap().id(),
            observed: 100,
            running: false,
            connections: 0,
            registrations: 0,
            tunnels: 0,
            forwarded: 3,
        };
        writer.write(&status).unwrap();
        assert!(!RelayStatus::read(&dir).unwrap().running);
        fs::write(dir.join("relay.status"), "x".repeat(513)).unwrap();
        assert!(RelayStatus::read(&dir).is_err());
        drop(writer);
        let _ = fs::remove_dir_all(dir);
    }
}
